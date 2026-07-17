// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Top toolbar panel with element addition and action buttons.

use std::path::PathBuf;

use log::{error, info};

use ptouch_render::document::LabelDocument;
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
                flip_h: false,
                flip_v: false,
            });
            state.selected_element = Some(state.elements.len() - 1);
            state.mark_dirty();
            info!("Added text element");
        }

        if ui.button("Add Image").clicked()
            && let Some(path) = crate::widgets::image_file_dialog().pick_file()
        {
            // Read the original source bytes so the image is embedded in the
            // label and stays self-contained when saved to a layout file.
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let element = LabelElement::image_from_bytes(Some(path.clone()), bytes);
                    // Reject files that do not decode, so a saved layout can
                    // always be reopened.
                    if matches!(element, LabelElement::Image { bitmap: None, .. }) {
                        error!("Unsupported or corrupt image: {}", path.display());
                        state.status_message = format!("Image load error: {}", path.display());
                    } else {
                        info!("Loaded image: {}", path.display());
                        state.elements.push(element);
                        state.selected_element = Some(state.elements.len() - 1);
                        state.mark_dirty();
                    }
                }
                Err(e) => {
                    error!("Failed to read image: {}", e);
                    state.status_message = format!("Image read error: {}", e);
                }
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
            && let Some(ref bitmap) = state.preview_bitmap
        {
            let raster_lines = raster::bitmap_to_raster_lines(bitmap, state.printer_max_px);
            let chain_print = !state.auto_cut;
            let auto_cut = state.auto_cut;
            if let Some(ref tx) = state.printer_cmd_tx {
                let _ = tx.send(PrinterCommand::Print {
                    raster_lines,
                    chain_print,
                    auto_cut,
                    quality: state.print_quality,
                });
                state.operation_in_progress = true;
                state.status_message = "Printing...".to_string();
            }
        }

        if ui
            .add_enabled(connected && !busy, egui::Button::new("Feed & Cut"))
            .clicked()
            && let Some(ref tx) = state.printer_cmd_tx
        {
            let _ = tx.send(PrinterCommand::FeedAndCut);
            state.operation_in_progress = true;
            state.status_message = "Feeding & cutting...".to_string();
        }

        if ui.button("Export Image").clicked() {
            do_export_image(state);
        }

        ui.separator();

        if ui.button("Save Layout").clicked() {
            do_save_layout(state);
        }

        if ui.button("Open Layout").clicked() {
            do_open_layout(state);
        }
    });
}

/// Save the current design to a `.ptl` layout file (TOML with embedded images).
fn do_save_layout(state: &mut AppState) {
    if state.elements.is_empty() {
        state.status_message = "Nothing to save".to_string();
        return;
    }

    let document = LabelDocument {
        version: ptouch_render::document::DOCUMENT_VERSION,
        tape_width_mm: state.tape_width_mm,
        font_name: state.font_name.clone(),
        font_margin: state.font_margin,
        flip_h: state.overall_flip_h,
        flip_v: state.overall_flip_v,
        elements: state.elements.clone(),
    };

    let text = match document.to_toml_string() {
        Ok(text) => text,
        Err(e) => {
            state.status_message = format!("Save error: {}", e);
            error!("Layout serialize error: {}", e);
            return;
        }
    };

    if let Some(path) = crate::widgets::layout_file_dialog()
        .set_file_name("label.ptl")
        .save_file()
    {
        let save_path: PathBuf = if path.extension().is_none() {
            path.with_extension("ptl")
        } else {
            path
        };
        match std::fs::write(&save_path, text) {
            Ok(()) => {
                state.status_message = format!("Saved to {}", save_path.display());
                info!("Saved layout: {}", save_path.display());
            }
            Err(e) => {
                state.status_message = format!("Save error: {}", e);
                error!("Layout write error: {}", e);
            }
        }
    }
}

/// Open a `.ptl` layout file, replacing the current design.
fn do_open_layout(state: &mut AppState) {
    let Some(path) = crate::widgets::layout_file_dialog().pick_file() else {
        return;
    };

    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) => {
            state.status_message = format!("Open error: {}", e);
            error!("Layout read error: {}", e);
            return;
        }
    };

    match LabelDocument::from_toml_str(&text) {
        Ok(document) => {
            state.tape_width_mm = document.tape_width_mm;
            state.update_tape_pixels();
            state.font_name = document.font_name;
            state.font_margin = document.font_margin;
            state.overall_flip_h = document.flip_h;
            state.overall_flip_v = document.flip_v;
            state.elements = document.elements;
            state.selected_element = None;
            state.mark_dirty();
            state.status_message = format!("Opened {}", path.display());
            info!("Opened layout: {}", path.display());
        }
        Err(e) => {
            state.status_message = format!("Open error: {}", e);
            error!("Layout parse error: {}", e);
        }
    }
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
