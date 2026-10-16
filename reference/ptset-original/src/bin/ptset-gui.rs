#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use ptset::settings::{SETTINGS, Setting};
use ptset::status::Status;
use ptset::{device, usb};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ptset - Brother PT-P900 Settings")
            .with_inner_size([620.0, 560.0])
            .with_min_inner_size([480.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native("ptset", options, Box::new(|cc| Ok(Box::new(App::new(&cc.egui_ctx)))))
}

/// Results sent from the USB worker thread back to the UI.
enum Event {
    Refreshed { path: String, status: Status, values: Vec<Result<u8, String>> },
    Written { key: &'static str, value: u8 },
    Failed(String),
}

struct PendingWrite {
    setting: &'static Setting,
    raw: u8,
}

struct App {
    tx: Sender<Event>,
    rx: Receiver<Event>,
    busy: bool,
    device_path: Option<String>,
    status: Option<Status>,
    /// Current value per entry in SETTINGS, as last read from the printer.
    values: Vec<Option<Result<u8, String>>>,
    /// Value chosen in the editor per entry in SETTINGS.
    edits: Vec<Option<u8>>,
    pending: Option<PendingWrite>,
    log: Vec<String>,
}

impl App {
    fn new(ctx: &egui::Context) -> Self {
        let (tx, rx) = channel();
        let mut app = Self {
            tx,
            rx,
            busy: false,
            device_path: None,
            status: None,
            values: vec![None; SETTINGS.len()],
            edits: vec![None; SETTINGS.len()],
            pending: None,
            log: Vec::new(),
        };
        app.refresh(ctx);
        app
    }

    fn refresh(&mut self, ctx: &egui::Context) {
        self.run(ctx, "Reading printer status and settings...", |tx| {
            let printer = usb::Printer::open(None)?;
            device::initialize(&printer)?;
            let status = device::read_status(&printer)?;
            let values = SETTINGS
                .iter()
                .map(|s| device::read_setting(&printer, s).map(|r| r.value).map_err(|e| e.to_string()))
                .collect();
            let _ = tx.send(Event::Refreshed { path: printer.path.clone(), status, values });
            Ok(())
        });
    }

    fn write(&mut self, ctx: &egui::Context, setting: &'static Setting, raw: u8) {
        self.run(ctx, &format!("Writing {} = {}...", setting.label, setting.describe(raw)), move |tx| {
            let printer = usb::Printer::open(None)?;
            device::initialize(&printer)?;
            let reading = device::write_setting(&printer, setting, raw)?;
            let _ = tx.send(Event::Written { key: setting.key, value: reading.value });
            Ok(())
        });
    }

    /// Run a printer operation on a worker thread; errors come back as Event::Failed.
    fn run(
        &mut self,
        ctx: &egui::Context,
        what: &str,
        job: impl FnOnce(&Sender<Event>) -> anyhow::Result<()> + Send + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.log.push(what.to_owned());
        let tx = self.tx.clone();
        let ctx = ctx.clone();
        thread::spawn(move || {
            if let Err(e) = job(&tx) {
                let _ = tx.send(Event::Failed(format!("{e:#}")));
            }
            ctx.request_repaint();
        });
    }

    fn handle_events(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            self.busy = false;
            match event {
                Event::Refreshed { path, status, values } => {
                    self.log.push(format!("Connected: {} with {} mm tape", status.model_name(), status.media_width_mm()));
                    self.device_path = Some(path);
                    self.status = Some(status);
                    self.edits = values.iter().map(|v| v.as_ref().ok().copied()).collect();
                    self.values = values.into_iter().map(Some).collect();
                }
                Event::Written { key, value } => {
                    if let Some(i) = SETTINGS.iter().position(|s| s.key == key) {
                        self.values[i] = Some(Ok(value));
                        self.edits[i] = Some(value);
                        self.log.push(format!("Saved and verified: {} = {}", SETTINGS[i].label, SETTINGS[i].describe(value)));
                    }
                }
                Event::Failed(message) => self.log.push(format!("Error: {message}")),
            }
        }
    }

    fn status_panel(&self, ui: &mut egui::Ui) {
        ui.heading("Printer");
        let Some(status) = &self.status else {
            ui.label(if self.busy { "Connecting..." } else { "No printer connected. Turn it on, connect USB, then Refresh." });
            return;
        };
        egui::Grid::new("status").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
            ui.label("Model");
            ui.label(format!("{} (series 0x{:02X}, model 0x{:02X})", status.model_name(), status.series_code(), status.model_code()));
            ui.end_row();
            ui.label("Tape");
            ui.label(format!("{} mm", status.media_width_mm()));
            ui.end_row();
            ui.label("Errors");
            let errors = status.errors();
            if errors.is_empty() {
                ui.colored_label(egui::Color32::from_rgb(0x2e, 0x9e, 0x5b), "None");
            } else {
                ui.colored_label(ui.visuals().error_fg_color, errors.join(", "));
            }
            ui.end_row();
            if let Some(path) = &self.device_path {
                ui.label("USB");
                ui.label(egui::RichText::new(path).small().weak());
                ui.end_row();
            }
        });
    }

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Device settings");
        egui::Grid::new("settings").num_columns(3).striped(true).spacing([16.0, 8.0]).show(ui, |ui| {
            ui.strong("Setting");
            ui.strong("Current");
            ui.strong("Change");
            ui.end_row();
            for (i, setting) in SETTINGS.iter().enumerate() {
                ui.label(setting.label).on_hover_text(format!(
                    "{} - GET {:02X?}\nfrom {} (vtable 0x{:X})",
                    setting.key, setting.get, setting.class, setting.vtable
                ));
                match &self.values[i] {
                    None => ui.weak("-"),
                    Some(Ok(v)) => ui.label(setting.describe(*v)),
                    Some(Err(e)) => ui.colored_label(ui.visuals().error_fg_color, "read failed").on_hover_text(e),
                };
                if setting.writable() {
                    let current = self.values[i].as_ref().and_then(|v| v.as_ref().ok()).copied();
                    ui.horizontal(|ui| {
                        let edit = &mut self.edits[i];
                        let shown = edit.map(|v| setting.describe(v)).unwrap_or_else(|| "-".into());
                        egui::ComboBox::from_id_salt(setting.key).selected_text(shown).show_ui(ui, |ui| {
                            for (raw, name) in setting.values {
                                ui.selectable_value(edit, Some(*raw), *name);
                            }
                        });
                        let changed = edit.is_some() && *edit != current;
                        if ui.add_enabled(changed && !self.busy, egui::Button::new("Apply...")).clicked() {
                            self.pending = Some(PendingWrite { setting, raw: edit.unwrap() });
                        }
                    });
                } else {
                    ui.weak("read-only").on_hover_text(
                        "The meaning of this setting's raw values hasn't been verified yet, so writing is disabled.",
                    );
                }
                ui.end_row();
            }
        });
    }

    fn confirm_dialog(&mut self, ctx: &egui::Context) {
        let Some(pending) = &self.pending else { return };
        let (setting, raw) = (pending.setting, pending.raw);
        let mut decision = None;
        egui::Window::new("Write setting to printer?")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(format!("{} -> {}", setting.label, setting.describe(raw)));
                ui.add_space(6.0);
                ui.label("Bytes that will be sent:");
                ui.monospace(format!("{:02X?}", setting.set_command(raw)));
                ui.add_space(6.0);
                ui.label("The setting is read back afterwards to confirm the printer saved it.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Write").clicked() {
                        decision = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decision = Some(false);
                    }
                });
            });
        match decision {
            Some(true) => {
                self.pending = None;
                self.write(ctx, setting, raw);
            }
            Some(false) => self.pending = None,
            None => {}
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = &ui.ctx().clone();
        self.handle_events();

        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("ptset");
                ui.separator();
                if ui.add_enabled(!self.busy, egui::Button::new("Refresh")).clicked() {
                    self.refresh(ctx);
                }
                if self.busy {
                    ui.spinner();
                }
            });
        });

        egui::Panel::bottom("log").resizable(true).default_size(110.0).show(ui, |ui| {
            ui.strong("Activity");
            egui::ScrollArea::vertical().stick_to_bottom(true).auto_shrink([false, false]).show(ui, |ui| {
                for line in &self.log {
                    if line.starts_with("Error") {
                        ui.colored_label(ui.visuals().error_fg_color, line);
                    } else {
                        ui.monospace(line);
                    }
                }
            });
        });

        egui::CentralPanel::default_margins().show(ui, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                self.status_panel(ui);
                ui.add_space(12.0);
                ui.separator();
                ui.add_space(6.0);
                self.settings_panel(ui);
            });
        });

        self.confirm_dialog(ctx);
    }
}
