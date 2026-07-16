// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! GUI application for Brother P-Touch label printers.
//!
//! Provides a graphical interface for composing and printing labels
//! using egui/eframe.

mod app;
mod panels;
mod printer_worker;
mod state;
mod widgets;

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

fn main() -> eframe::Result<()> {
    env_logger::init();

    // `with_app_id` sets the Wayland app_id / X11 WM_CLASS so the desktop entry
    // (StartupWMClass) associates its icon with the window.
    let mut viewport = egui::ViewportBuilder::default()
        .with_app_id("io.github.vowstar.ptouch-gui")
        .with_inner_size([1200.0, 700.0])
        .with_min_inner_size([800.0, 500.0]);
    if let Some(icon) = load_window_icon() {
        viewport = viewport.with_icon(std::sync::Arc::new(icon));
    }

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "P-Touch Label Printer",
        options,
        Box::new(|cc| Ok(Box::new(app::PtouchApp::new(cc)))),
    )
}
