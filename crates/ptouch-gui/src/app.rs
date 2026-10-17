// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Main application struct and eframe::App implementation.

use std::sync::mpsc;

use log::{error, info};

use ptouch_render::text::TextRenderer;

use crate::panels;
use crate::printer_worker;
use crate::state::{AppState, PrinterEvent, PrinterResponse};

/// The main P-Touch GUI application.
pub struct PtouchApp {
    /// Application state shared across all panels.
    pub state: AppState,
    /// Text renderer instance for generating label bitmaps.
    renderer: TextRenderer,
    /// Receiver for responses from the printer worker thread.
    resp_rx: mpsc::Receiver<PrinterEvent>,
    smoke_frames: Option<u8>,
}

impl PtouchApp {
    /// Create a new application instance.
    pub fn new(cc: &eframe::CreationContext<'_>, smoke_test: bool, offline: bool) -> Self {
        setup_fallback_fonts(&cc.egui_ctx);

        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (resp_tx, resp_rx) = mpsc::channel();

        let enable_printer = !smoke_test && !offline;
        if enable_printer {
            let ctx = cc.egui_ctx.clone();
            std::thread::Builder::new()
                .name("printer-worker".to_string())
                .spawn(move || {
                    printer_worker::printer_worker(cmd_rx, resp_tx, ctx);
                })
                .expect("failed to spawn printer worker thread");
        }

        Self {
            state: AppState {
                calibration_profiles: cc
                    .storage
                    .and_then(|storage| eframe::get_value(storage, "calibration_profiles"))
                    .unwrap_or_default(),
                available_fonts: ptouch_render::font::list_fonts(),
                printer_cmd_tx: enable_printer.then_some(cmd_tx),
                printer_status: (!enable_printer).then(|| "Offline mode".to_string()),
                ..AppState::default()
            },
            renderer: TextRenderer::new(),
            resp_rx,
            smoke_frames: smoke_test.then_some(0),
        }
    }

    /// Re-render the preview bitmap from the current element list.
    pub fn update_preview(&mut self, ctx: &egui::Context) {
        self.state.needs_rerender = false;

        if self.state.elements.is_empty() {
            self.state.preview_bitmap = None;
            self.state.preview_texture = None;
            self.state.preview_bounds.clear();
            self.state.preview_content_bounds.clear();
            self.state.preview_diagnostics.clear();
            return;
        }

        let result = match ptouch_render::document::render_document(
            &self.state.document(),
            self.state.tape_width_px,
            self.state.target_dpi(),
            &mut self.renderer,
        ) {
            Ok(result) => {
                if self.state.status_message.starts_with("Render error:") {
                    self.state.status_message = "Preview updated".to_string();
                }
                self.state.preview_bounds = result.bounds;
                self.state.preview_content_bounds = result.content_bounds;
                self.state.preview_diagnostics = result.diagnostics;
                result.bitmap
            }
            Err(e) => {
                error!("Render failed: {}", e);
                self.state.status_message = format!("Render error: {}", e);
                self.state.preview_bounds.clear();
                self.state.preview_content_bounds.clear();
                self.state.preview_diagnostics = vec![e.to_string()];
                None
            }
        };

        if let Some(ref bitmap) = result {
            let rgba = bitmap.to_rgba_image();
            let max_side = ctx.input(|i| i.max_texture_side);

            let rgba = if rgba.width() as usize > max_side || rgba.height() as usize > max_side {
                let scale = max_side as f32 / rgba.width().max(rgba.height()) as f32;
                let new_w = (rgba.width() as f32 * scale).floor() as u32;
                let new_h = (rgba.height() as f32 * scale).floor() as u32;
                image::imageops::resize(
                    &rgba,
                    new_w.max(1),
                    new_h.max(1),
                    image::imageops::FilterType::Nearest,
                )
            } else {
                rgba
            };

            let size = [rgba.width() as usize, rgba.height() as usize];
            let pixels = rgba.into_raw();
            let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);

            let texture =
                ctx.load_texture("label_preview", color_image, egui::TextureOptions::NEAREST);
            self.state.preview_texture = Some(texture);
        } else {
            self.state.preview_texture = None;
        }

        self.state.preview_bitmap = result;
        info!("Preview updated");
    }

    fn drain_printer_responses(&mut self) {
        // Drain all pending responses from the printer worker
        while let Ok(event) = self.resp_rx.try_recv() {
            if event
                .target
                .as_ref()
                .is_some_and(|target| target != &self.state.printer_target)
            {
                continue;
            }
            match event.response {
                PrinterResponse::UsbDevices(devices) => {
                    self.state.usb_targets = devices;
                }
                PrinterResponse::BluetoothDevices(devices) => {
                    self.state.bluetooth_targets = devices;
                }
                PrinterResponse::NetworkDevices(devices) => {
                    self.state.network_targets = devices;
                }
                PrinterResponse::Connected {
                    model_name,
                    media_width,
                    media_type,
                    media_status_available,
                    media_detected,
                    max_px,
                    dpi,
                    quality_modes,
                    draft_quality_modes,
                    tape_width_px,
                } => {
                    let old_dpi = self.state.target_dpi();
                    self.state.printer_connected = true;
                    self.state.preview_uses_printer = true;
                    self.state.connecting = false;
                    self.state.printer_max_px = max_px;
                    self.state.printer_dpi = dpi;
                    self.state.printer_media_detected = media_detected;
                    self.state.printer_media_status_available = media_status_available;
                    self.state.printer_media_width_mm = media_detected.then_some(media_width);
                    if media_status_available {
                        self.state.manual_tcp_tape_width_mm = None;
                    }
                    self.state.printer_quality_modes = quality_modes;
                    self.state.printer_draft_quality_modes = draft_quality_modes;
                    if !quality_modes
                        || (!draft_quality_modes
                            && self.state.print_quality
                                == ptouch_core::protocol::PrintQuality::Draft)
                    {
                        self.state.print_quality = ptouch_core::protocol::PrintQuality::Standard;
                    }
                    self.state.printer_model = Some(if media_detected {
                        format!("{}: {} mm {}", model_name, media_width, media_type)
                    } else {
                        format!("{}: tape not detected", model_name)
                    });
                    self.state.printer_status = Some(if media_detected {
                        "Connected".to_string()
                    } else if media_status_available {
                        "No tape detected. Load media and refresh.".to_string()
                    } else {
                        "Select tape width manually".to_string()
                    });
                    if media_detected && media_width > 0 {
                        self.state.tape_width_mm = media_width;
                    }
                    // Use the printer's status-derived printable width. This
                    // differs from the transfer width on the PT-P300BT.
                    let old_px = self.state.tape_width_px;
                    self.state.tape_width_px = u32::from(tape_width_px);
                    if self.state.tape_width_px != old_px || self.state.target_dpi() != old_dpi {
                        self.state.mark_dirty();
                    }
                }
                PrinterResponse::ConnectionError(message) => {
                    self.state.printer_status = Some(message);
                    self.state.printer_model = None;
                    self.state.printer_connected = false;
                    self.state.printer_media_detected = false;
                    self.state.printer_media_status_available = false;
                    self.state.printer_media_width_mm = None;
                    self.state.connecting = false;
                    // Keep printer_max_px, printer_dpi, and quality state:
                    // the canvas must not resize on a transient disconnect,
                    // and printing is gated on printer_connected anyway.
                }
                PrinterResponse::PrintDone => {
                    self.state.operation_in_progress = false;
                    self.state.cancellation = None;
                    self.state.status_message = "Print complete".to_string();
                }
                PrinterResponse::FeedAndCutDone => {
                    self.state.operation_in_progress = false;
                    self.state.cancellation = None;
                    self.state.status_message = "Feed & cut done".to_string();
                }
                PrinterResponse::Error(msg) => {
                    self.state.operation_in_progress = false;
                    self.state.cancellation = None;
                    self.state.status_message = msg;
                }
            }
        }
    }
}

impl PtouchApp {
    fn show_editor(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.drain_printer_responses();

        // Top toolbar
        let toolbar_action = egui::Panel::top("toolbar")
            .show(ui, |ui| panels::toolbar::show_toolbar(ui, &self.state))
            .inner;

        // Bottom status bar
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            panels::status_bar::show_status_bar(ui, &self.state);
        });

        if self.state.needs_rerender {
            self.update_preview(&ctx);
        }

        // Right properties panel
        egui::Panel::right("properties")
            .default_size(250.0)
            .resizable(true)
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    panels::properties::show_properties(ui, &mut self.state);
                });
            });

        duplicate_shortcut(&ctx, &mut self.state);

        // Commit the old selection's numeric input before selecting/reordering
        // another element. The panels keep their usual visual positions.
        egui::Panel::left("sidebar")
            .default_size(200.0)
            .resizable(true)
            .show(ui, |ui| {
                panels::sidebar::show_sidebar(ui, &mut self.state);
            });

        if self.state.needs_rerender {
            self.update_preview(&ctx);
        }
        if let Some(action) = toolbar_action {
            panels::toolbar::execute_action(&mut self.state, action);
        }

        // Numeric edits and pointer gestures can arrive in the same frame.
        if self.state.needs_rerender {
            self.update_preview(&ctx);
        }

        // Central canvas
        egui::CentralPanel::default().show(ui, |ui| {
            panels::canvas::show_canvas(ui, &mut self.state);
        });

        // Re-render preview if dirty
        if self.state.needs_rerender {
            self.update_preview(&ctx);
        }

        // Exit only after multiple frames exercised the actual window renderer.
        if let Some(frames) = &mut self.smoke_frames {
            *frames += 1;
            if *frames >= 3 {
                println!("PTOUCH_GUI_SMOKE_OK");
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                self.smoke_frames = None;
            }
            ctx.request_repaint();
        }

        // Periodic repaint so we pick up worker responses even when idle
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
    }
}

impl eframe::App for PtouchApp {
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(
            storage,
            "calibration_profiles",
            &self.state.calibration_profiles,
        );
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show_editor(ui);
    }
}

fn duplicate_shortcut(ctx: &egui::Context, state: &mut AppState) {
    if !ctx.text_edit_focused()
        && !ctx.input(|input| input.pointer.any_down())
        && ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::D))
    {
        state.duplicate_selected();
    }
}

/// Register fallback fonts for CJK text and emoji rendering.
fn setup_fallback_fonts(ctx: &egui::Context) {
    use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};

    let lowest_both = vec![
        InsertFontFamily {
            family: egui::FontFamily::Proportional,
            priority: FontPriority::Lowest,
        },
        InsertFontFamily {
            family: egui::FontFamily::Monospace,
            priority: FontPriority::Lowest,
        },
    ];

    // CJK fallback (DroidSansFallback, Apache-2.0)
    ctx.add_font(FontInsert {
        name: "cjk_fallback".into(),
        data: egui::FontData::from_static(include_bytes!("../assets/fonts/DroidSansFallback.ttf")),
        families: lowest_both.clone(),
    });

    // Emoji fallback (NotoEmoji, OFL-1.1)
    ctx.add_font(FontInsert {
        name: "emoji_fallback".into(),
        data: egui::FontData::from_static(include_bytes!("../assets/fonts/NotoEmoji.ttf")),
        families: lowest_both,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PrinterTarget;

    fn app(state: AppState) -> (PtouchApp, mpsc::Sender<PrinterEvent>) {
        let (tx, rx) = mpsc::channel();
        (
            PtouchApp {
                state,
                renderer: TextRenderer::new(),
                resp_rx: rx,
                smoke_frames: None,
            },
            tx,
        )
    }

    fn usb_status() -> PrinterEvent {
        PrinterEvent {
            target: Some(PrinterTarget::Usb),
            response: PrinterResponse::Connected {
                model_name: "PT-P700".into(),
                media_width: 24,
                media_type: "Laminated tape".into(),
                media_status_available: true,
                media_detected: true,
                max_px: 128,
                dpi: 180,
                quality_modes: false,
                draft_quality_modes: false,
                tape_width_px: 128,
            },
        }
    }

    #[test]
    fn tcp_model_capabilities_keep_high_resolution_and_reject_stale_draft() {
        let target = PrinterTarget::Tcp {
            address: "127.0.0.1:9100".parse().unwrap(),
            pid: 0x2085,
        };
        let (mut app, tx) = app(AppState {
            printer_target: target.clone(),
            print_quality: ptouch_core::protocol::PrintQuality::Draft,
            ..AppState::default()
        });
        tx.send(PrinterEvent {
            target: Some(target),
            response: PrinterResponse::Connected {
                model_name: "PT-P900W".into(),
                media_width: 24,
                media_type: "Laminated tape".into(),
                media_status_available: true,
                media_detected: true,
                max_px: 560,
                dpi: 360,
                quality_modes: true,
                draft_quality_modes: false,
                tape_width_px: 340,
            },
        })
        .unwrap();

        app.drain_printer_responses();

        assert!(app.state.printer_quality_modes);
        assert!(!app.state.printer_draft_quality_modes);
        assert_eq!(
            app.state.print_quality,
            ptouch_core::protocol::PrintQuality::Standard
        );
    }

    #[test]
    fn p900w_status_without_media_does_not_enable_manual_media_fallback() {
        let target = PrinterTarget::Tcp {
            address: "127.0.0.1:9100".parse().unwrap(),
            pid: 0x2085,
        };
        let (mut app, tx) = app(AppState {
            printer_target: target.clone(),
            manual_tcp_tape_width_mm: Some(12),
            ..AppState::default()
        });
        tx.send(PrinterEvent {
            target: Some(target),
            response: PrinterResponse::Connected {
                model_name: "PT-P900W".into(),
                media_width: 0,
                media_type: "No media".into(),
                media_status_available: true,
                media_detected: false,
                max_px: 560,
                dpi: 360,
                quality_modes: false,
                draft_quality_modes: false,
                tape_width_px: 560,
            },
        })
        .unwrap();

        app.drain_printer_responses();

        assert!(app.state.printer_media_status_available);
        assert!(!app.state.printer_media_detected);
        assert_eq!(app.state.manual_tcp_tape_width_mm, None);
        assert!(!app.state.can_print_selected_media());
    }

    #[test]
    fn duplicate_shortcut_respects_text_editing_focus() {
        for editing in [false, true] {
            let ctx = egui::Context::default();
            let mut state = AppState::default();
            state.add_text();
            let mut text = "PORT".to_string();
            for step in 0..3 {
                let events = if step == 2 {
                    vec![egui::Event::Key {
                        key: egui::Key::D,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers {
                            ctrl: true,
                            command: true,
                            ..Default::default()
                        },
                    }]
                } else {
                    vec![]
                };
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        if editing {
                            ui.text_edit_singleline(&mut text).request_focus();
                        } else {
                            ui.button("Selected object").request_focus();
                        }
                        duplicate_shortcut(&ctx, &mut state);
                    },
                );
                output.textures_delta.clear();
            }
            assert_eq!(state.elements.len(), if editing { 1 } else { 2 });
        }
    }

    #[test]
    fn pending_numeric_edits_do_not_leak_through_toolbar_or_list_selection() {
        use ptouch_render::document::{LabelElement, LabelObject, TextBox};
        for add in [false, true] {
            let mut state = AppState {
                selected_element: Some(0),
                canvas_edit_target: crate::state::CanvasEditTarget::Slot,
                ..Default::default()
            };
            state.elements.push(LabelElement::Object(LabelObject::text(
                "PORT",
                TextBox {
                    width_mm: 14.5,
                    ..Default::default()
                },
            )));
            state.elements.push(LabelElement::Object(LabelObject::text(
                "NEXT",
                TextBox {
                    x_mm: 30.0,
                    width_mm: 20.0,
                    ..Default::default()
                },
            )));
            let (mut app, _tx) = app(state);
            let ctx = egui::Context::default();
            let mut frame = |events| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 680.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| app.show_editor(ui),
                );
                output.textures_delta.clear();
                output
            };
            let center = |output: &egui::FullOutput, text: &str| {
                output
                    .shapes
                    .iter()
                    .find_map(|s| {
                        if let egui::Shape::Text(t) = &s.shape
                            && t.galley.job.text == text
                        {
                            Some(t.pos + t.galley.size() / 2.0)
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "missing text: {text}; visible: {:?}",
                            output
                                .shapes
                                .iter()
                                .filter_map(|s| if let egui::Shape::Text(t) = &s.shape {
                                    Some(&t.galley.job.text)
                                } else {
                                    None
                                })
                                .collect::<Vec<_>>()
                        )
                    })
            };
            let click = |pos| {
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ]
            };
            frame(vec![]);
            let output = frame(vec![]);
            let input = center(&output, "14.5");
            let action = if add {
                center(&output, "Add Text")
            } else {
                output
                    .shapes
                    .iter()
                    .filter_map(|s| {
                        if let egui::Shape::Text(t) = &s.shape
                            && t.galley.job.text == "Slot"
                        {
                            Some(t.pos + t.galley.size() / 2.0)
                        } else {
                            None
                        }
                    })
                    .nth(1)
                    .unwrap()
            };
            frame(click(input));
            frame(vec![]);
            frame(vec![egui::Event::Text("14,6".into())]);
            frame(click(action));
            frame(vec![]);
            assert_eq!(
                app.state.elements[0].as_object().unwrap().frame.width_mm,
                14.6
            );
            assert_eq!(app.state.elements.len(), if add { 3 } else { 2 });
            assert_eq!(app.state.selected_element, Some(if add { 2 } else { 1 }));
            assert_eq!(
                app.state.elements[1].as_object().unwrap().frame.width_mm,
                20.0
            );
        }
    }

    #[test]
    fn late_usb_poll_does_not_connect_selected_bluetooth_printer() {
        let (mut app, tx) = app(AppState {
            printer_target: PrinterTarget::Bluetooth {
                name: "PT-P300BT".into(),
                address: "AA:BB:CC:DD:EE:FF".into(),
            },
            connecting: true,
            ..AppState::default()
        });
        tx.send(usb_status()).unwrap();
        app.drain_printer_responses();
        assert!(!app.state.printer_connected);
        assert!(app.state.connecting);
        assert!(app.state.printer_model.is_none());
        tx.send(PrinterEvent {
            target: Some(PrinterTarget::Usb),
            response: PrinterResponse::ConnectionError("Device not found".into()),
        })
        .unwrap();
        app.drain_printer_responses();
        assert!(app.state.connecting);

        tx.send(PrinterEvent {
            target: Some(app.state.printer_target.clone()),
            response: PrinterResponse::Connected {
                model_name: "PT-P300BT".into(),
                media_width: 12,
                media_type: "Laminated tape".into(),
                media_status_available: true,
                max_px: 128,
                dpi: 180,
                quality_modes: false,
                draft_quality_modes: false,
                tape_width_px: 64,
                media_detected: true,
            },
        })
        .unwrap();
        app.drain_printer_responses();
        assert!(app.state.printer_connected);
        assert!(!app.state.is_printer_busy());
        assert_eq!(app.state.tape_width_px, 64);
    }

    #[test]
    fn status_reply_does_not_finish_pending_print() {
        let (mut app, tx) = app(AppState {
            printer_connected: true,
            operation_in_progress: true,
            ..AppState::default()
        });
        tx.send(usb_status()).unwrap();
        app.drain_printer_responses();
        assert!(app.state.operation_in_progress);
        assert!(
            app.state
                .printer_model
                .as_ref()
                .unwrap()
                .starts_with("PT-P700")
        );
        tx.send(PrinterEvent {
            target: Some(PrinterTarget::Usb),
            response: PrinterResponse::ConnectionError("Device not found".into()),
        })
        .unwrap();
        app.drain_printer_responses();
        assert!(!app.state.printer_connected);
        assert_eq!(
            app.state.printer_status.as_deref(),
            Some("Device not found")
        );
        assert!(app.state.operation_in_progress);
        tx.send(PrinterEvent {
            target: Some(PrinterTarget::Usb),
            response: PrinterResponse::PrintDone,
        })
        .unwrap();
        app.drain_printer_responses();
        assert!(!app.state.operation_in_progress);
    }

    #[test]
    fn usb_selection_ignores_another_devices_poll_and_keeps_error_detail() {
        let target =
            PrinterTarget::UsbAt(ptouch_core::transport::UsbLocation { bus: 1, address: 5 });
        let (mut app, tx) = app(AppState {
            printer_target: target.clone(),
            ..AppState::default()
        });
        tx.send(usb_status()).unwrap();
        app.drain_printer_responses();
        assert!(!app.state.printer_connected);
        tx.send(PrinterEvent {
            target: Some(target),
            response: PrinterResponse::ConnectionError("USB open: Access denied".into()),
        })
        .unwrap();
        app.drain_printer_responses();
        assert_eq!(
            app.state.printer_status.as_deref(),
            Some("USB open: Access denied")
        );
    }

    #[test]
    fn changing_target_dpi_rerenders_even_when_printable_pixels_match() {
        let (mut app, tx) = app(AppState {
            design_dpi: 360,
            tape_width_px: 128,
            needs_rerender: false,
            ..AppState::default()
        });
        tx.send(usb_status()).unwrap();
        app.drain_printer_responses();
        assert_eq!(app.state.target_dpi(), 180);
        assert_eq!(app.state.design_dpi, 360);
        assert!(app.state.needs_rerender);
        app.state.needs_rerender = false;
        tx.send(PrinterEvent {
            target: Some(PrinterTarget::Usb),
            response: PrinterResponse::ConnectionError("Disconnected".into()),
        })
        .unwrap();
        app.drain_printer_responses();
        assert_eq!(app.state.target_dpi(), 180);
        assert!(!app.state.needs_rerender);
        assert!(app.state.preview_uses_printer);
    }

    #[test]
    fn preview_matches_shared_render_and_preserves_original_bitmap_after_downsampling() {
        use ptouch_render::document::{LabelElement, TextBox, render_document};
        use ptouch_render::text::TextAlign;
        let (mut app, _tx) = app(AppState {
            overall_flip_h: true,
            elements: vec![LabelElement::Text {
                content: "Port 1".into(),
                font_size: None,
                align: TextAlign::Center,
                rotation: 0.0,
                flip_h: false,
                flip_v: false,
                text_box: Some(TextBox {
                    x_mm: 5.0,
                    width_mm: 300.0,
                    height_mm: 8.0,
                    ..TextBox::default()
                }),
            }],
            ..AppState::default()
        });
        let ctx = egui::Context::default();
        ctx.begin_pass(egui::RawInput {
            max_texture_side: Some(1024),
            ..Default::default()
        });
        app.update_preview(&ctx);
        let mut output = ctx.end_pass();
        output.textures_delta.clear();
        let expected = render_document(
            &app.state.document(),
            app.state.tape_width_px,
            app.state.target_dpi(),
            &mut TextRenderer::new(),
        )
        .unwrap();
        let bitmap = app.state.preview_bitmap.as_ref().unwrap();
        assert_eq!(
            bitmap.to_rgba_image(),
            expected.bitmap.unwrap().to_rgba_image()
        );
        assert!(bitmap.width() > 1024);
        assert_eq!(app.state.preview_texture.as_ref().unwrap().size()[0], 1024);
        let bound = &app.state.preview_bounds[0];
        assert_eq!(
            (bound.x, bound.width),
            (expected.bounds[0].x, expected.bounds[0].width)
        );
        assert!(bound.width > 1024);
    }

    #[test]
    fn reordered_auto_fit_object_uses_fresh_geometry_before_first_keyboard_edit() {
        use eframe::App;
        use ptouch_render::document::{LabelElement, LabelObject, TextBox};

        let first_slot = TextBox {
            x_mm: 5.0,
            y_mm: 1.0,
            width_mm: 6.0,
            height_mm: 4.0,
            ..TextBox::default()
        };
        let (mut app, _tx) = app(AppState {
            document_version: 3,
            selected_element: Some(0),
            elements: vec![
                LabelElement::Object(LabelObject::text("A", first_slot.clone())),
                LabelElement::Object(LabelObject::text(
                    "BBBBBBBBBBBB",
                    TextBox {
                        x_mm: 20.0,
                        width_mm: 18.0,
                        ..first_slot.clone()
                    },
                )),
            ],
            ..AppState::default()
        });
        let context = egui::Context::default();
        let mut frame = eframe::Frame::_new_kittest();
        let mut run_frame = |app: &mut PtouchApp, events| {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400.0, 1000.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| app.ui(ui, &mut frame),
            );
            output.textures_delta.clear();
        };
        run_frame(&mut app, vec![]);
        let expected = app.state.resolved_content_box(0).unwrap();
        let stale_index_geometry = app.state.resolved_content_box(1).unwrap();
        assert!((expected.width_mm - stale_index_geometry.width_mm).abs() > 1.0);
        assert_eq!(app.state.document_version, 3);

        // This is the model mutation made by a sidebar reorder. The cached
        // preview still describes the previous order when this UI frame begins.
        app.state.elements.swap(0, 1);
        app.state.selected_element = Some(1);
        app.state.mark_dirty();
        run_frame(
            &mut app,
            vec![egui::Event::Key {
                key: egui::Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        let edited = app.state.elements[1].as_object().unwrap();
        let pinned = edited.content_box.as_ref().unwrap();
        assert_eq!(
            (pinned.width_mm, pinned.height_mm),
            (expected.width_mm, expected.height_mm)
        );
        assert!((pinned.offset_x_mm - expected.offset_x_mm - 0.1).abs() < 1e-6);
        assert_eq!(pinned.offset_y_mm, expected.offset_y_mm);
        assert_eq!(edited.frame, first_slot);
        assert!(
            app.state.elements[0]
                .as_object()
                .unwrap()
                .content_box
                .is_none()
        );
        assert_eq!(app.state.document_version, 4);
    }
}
