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

fn main() -> eframe::Result<()> {
    env_logger::init();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 700.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "P-Touch Label Printer",
        options,
        Box::new(|cc| Ok(Box::new(app::PtouchApp::new(cc)))),
    )
}
