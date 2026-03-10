// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Top toolbar panel with element addition and action buttons.

use std::path::PathBuf;

use log::{error, info};

use ptouch_render::image_loader;
use ptouch_render::raster;
use ptouch_render::text::TextAlign;

use crate::state::{AppState, LabelElement, PrinterCommand};

/// Render the top toolbar.
pub fn show_toolbar(ui: &mut egui::Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        // -- Element addition buttons --
        if ui.button("Add Text").clicked() {
            state.elements.push(LabelElement::Text {
                content: "Label".to_string(),
                font_size: None,
                align: TextAlign::Left,
                rotation: 0.0,
            });
            state.selected_element = Some(state.elements.len() - 1);
            state.mark_dirty();
            info!("Added text element");
        }

        if ui.button("Add Image").clicked() {
            // Open a file dialog for image files
            if let Some(path) = crate::widgets::image_file_dialog().pick_file() {
                let bitmap = match image_loader::load_image(
                    &path,
                    &image_loader::ImageLoadOptions::default(),
                ) {
                    Ok(bmp) => {
                        info!("Loaded image: {}", path.display());
                        Some(bmp)
                    }
                    Err(e) => {
                        error!("Failed to load image: {}", e);
                        state.status_message = format!("Image load error: {}", e);
                        None
                    }
                };
                state.elements.push(LabelElement::Image {
                    path,
                    bitmap,
                    rotation: 0.0,
                    target_height: None,
                });
                state.selected_element = Some(state.elements.len() - 1);
                state.mark_dirty();
            }
        }

        if ui.button("Cut Mark").clicked() {
            state.elements.push(LabelElement::CutMark);
            state.selected_element = Some(state.elements.len() - 1);
            state.mark_dirty();
            info!("Added cut mark");
        }

        if ui.button("Padding").clicked() {
            state.elements.push(LabelElement::Padding { pixels: 20 });
            state.selected_element = Some(state.elements.len() - 1);
            state.mark_dirty();
            info!("Added padding element");
        }

        ui.separator();

        // -- Action buttons --
        let connected = state.printer_connected;
        let busy = state.operation_in_progress;
        let has_bitmap = state.preview_bitmap.is_some();

        if ui
            .add_enabled(connected && !busy && has_bitmap, egui::Button::new("Print"))
            .clicked()
        {
            if let Some(ref bitmap) = state.preview_bitmap {
                let raster_lines = raster::bitmap_to_raster_lines(bitmap, state.printer_max_px);
                let chain_print = !state.auto_cut;
                let auto_cut = state.auto_cut;
                if let Some(ref tx) = state.printer_cmd_tx {
                    let _ = tx.send(PrinterCommand::Print {
                        raster_lines,
                        chain_print,
                        auto_cut,
                    });
                    state.operation_in_progress = true;
                    state.status_message = "Printing...".to_string();
                }
            }
        }

        if ui
            .add_enabled(connected && !busy, egui::Button::new("Feed & Cut"))
            .clicked()
        {
            if let Some(ref tx) = state.printer_cmd_tx {
                let _ = tx.send(PrinterCommand::FeedAndCut);
                state.operation_in_progress = true;
                state.status_message = "Feeding & cutting...".to_string();
            }
        }

        if ui.button("Export Image").clicked() {
            do_export_image(state);
        }
    });
}

/// Export the current label preview as an image file.
fn do_export_image(state: &mut AppState) {
    let bitmap = match state.preview_bitmap {
        Some(ref bmp) => bmp,
        None => {
            state.status_message = "Nothing to export".to_string();
            return;
        }
    };

    if let Some(path) = rfd::FileDialog::new()
        .add_filter("PNG", &["png"])
        .add_filter("JPEG", &["jpg", "jpeg"])
        .add_filter("BMP", &["bmp"])
        .add_filter("GIF", &["gif"])
        .add_filter("TIFF", &["tiff", "tif"])
        .add_filter("WebP", &["webp"])
        .set_file_name("label.png")
        .save_file()
    {
        let save_path: PathBuf = if path.extension().is_none() {
            path.with_extension("png")
        } else {
            path
        };
        match bitmap.save(&save_path) {
            Ok(()) => {
                state.status_message = format!("Saved to {}", save_path.display());
                info!("Exported image: {}", save_path.display());
            }
            Err(e) => {
                state.status_message = format!("Save error: {}", e);
                error!("Image save error: {}", e);
            }
        }
    }
}
