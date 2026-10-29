// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! GUI application for Brother P-Touch label printers.
//!
//! Provides a graphical interface for composing and printing labels
//! using egui/eframe.

// Use the GUI subsystem in both debug and release builds. Worker mode receives
// explicit stdin/stdout pipes from its parent and does not need a console.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod alignment;
mod app;
mod batch_edit;
mod canvas_zoom;
mod document_session;
mod editor_navigation;
mod file_workflow;
mod graphics;
mod history;
mod output_tools;
mod panels;
mod positioning;
#[cfg(test)]
mod positioning_tests;
mod printer_worker;
#[cfg(any(target_os = "macos", test))]
mod query_process;
mod repeat_edit;
mod rich_text_editor;
mod scene_edit;
mod selection;
mod selection_canvas;
#[cfg(test)]
mod selection_tests;
mod startup;
mod state;
mod widgets;
mod window_state;

/// Decode the embedded application icon for the running window.
///
/// The PNG is generated from the shared SVG by `scripts/gen-icons.sh`. Returns
/// `None` if it somehow fails to decode, so the app still starts without an icon.
fn load_window_icon() -> Option<egui::IconData> {
    let image = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .ok()?
        .into_rgba8();
    let (width, height) = image.dimensions();
    Some(egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    })
}

fn main() -> std::process::ExitCode {
    #[cfg(windows)]
    {
        if let Some(code) = ptouch_core::usbprint::run_worker_from_args() {
            std::process::exit(code);
        }
    }
    if let Some(exit_code) = printer_worker::run_bluetooth_helper_from_args() {
        std::process::exit(exit_code);
    }
    env_logger::init();
    let smoke_test = std::env::args().any(|arg| arg == "--smoke-test");
    let report = startup::StartupReport::new(!smoke_test);
    report.install_panic_hook();
    #[cfg(windows)]
    {
        if let Err(error) = std::env::current_exe()
            .map_err(|e| e.to_string())
            .and_then(|path| {
                ptouch_core::usbprint::configure_worker(path).map_err(|e| e.to_string())
            })
        {
            report.error(&format!("Cannot configure USBPRINT: {error}"));
            return std::process::ExitCode::FAILURE;
        }
    }
    #[cfg(windows)]
    let result = report.run_with_renderer_fallback(
        |software| run_gui(smoke_test, &report, software),
        |error| matches!(error, eframe::Error::Wgpu(_)),
    );
    #[cfg(not(windows))]
    let result = run_gui(smoke_test, &report, false);
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            report.error(&error.to_string());
            std::process::ExitCode::FAILURE
        }
    }
}

fn run_gui(
    smoke_test: bool,
    report: &startup::StartupReport,
    software: bool,
) -> eframe::Result<()> {
    // Offline mode leaves USB and Bluetooth untouched while editing/exporting.
    let offline = std::env::args().any(|arg| arg == "--offline");

    // `with_app_id` sets the Wayland app_id / X11 WM_CLASS so the desktop entry
    // (StartupWMClass) associates its icon with the window.
    let mut viewport = egui::ViewportBuilder::default()
        .with_app_id("io.github.vowstar.ptouch-gui")
        .with_inner_size([1200.0, 700.0])
        .with_min_inner_size([800.0, 500.0]);
    #[cfg(windows)]
    {
        viewport = viewport.with_visible(false);
    }
    if let Some(icon) = load_window_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let options = graphics::native_options(viewport, software);
    report.record("Creating renderer and window");
    let report = report.clone();
    eframe::run_native(
        "P-Touch Label Printer",
        options,
        Box::new(move |cc| {
            #[cfg(windows)]
            if let Some(state) = &cc.wgpu_render_state {
                let adapter = state.adapter.get_info();
                report.record(&format!(
                    "Renderer: {:?}\nAdapter: {}\nAdapter type: {:?}",
                    adapter.backend, adapter.name, adapter.device_type
                ));
            }
            report.record("Initializing editor");
            let app = app::PtouchApp::new(cc, smoke_test, offline);
            report.editor_ready();
            #[cfg(windows)]
            {
                cc.egui_ctx
                    .send_viewport_cmd(egui::ViewportCommand::Visible(true));
                cc.egui_ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            Ok(Box::new(app))
        }),
    )
}
