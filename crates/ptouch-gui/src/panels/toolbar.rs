// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Top toolbar panel with element addition and action buttons.

use std::path::PathBuf;

use log::{error, info};

use ptouch_render::image_loader;
use ptouch_render::text::TextAlign;

use crate::state::{AppState, LabelElement};

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
        if ui.button("Print").clicked() {
            do_print(state);
        }

        if ui.button("Feed & Cut").clicked() {
            do_feed_and_cut(state);
        }

        if ui.button("Export Image").clicked() {
            do_export_image(state);
        }
    });
}

/// Attempt to print the current label to a connected printer.
fn do_print(state: &mut AppState) {
    use ptouch_core::transport::PtouchDevice;
    use ptouch_render::raster;

    let bitmap = match state.preview_bitmap {
        Some(ref bmp) => bmp,
        None => {
            state.status_message = "Nothing to print".to_string();
            return;
        }
    };

    state.status_message = "Connecting to printer...".to_string();

    match PtouchDevice::open_first() {
        Ok(mut dev) => {
            if let Err(e) = dev.init() {
                state.status_message = format!("Init error: {}", e);
                let _ = dev.close();
                return;
            }
            // init() already called get_status() internally
            let max_px = dev.max_px();
            let raster_lines = raster::bitmap_to_raster_lines(bitmap, max_px);
            let chain_print = !state.auto_cut;
            match dev.print_raster(&raster_lines, chain_print, state.auto_cut) {
                Ok(()) => {
                    state.status_message = "Print complete".to_string();
                    info!("Print successful");
                }
                Err(e) => {
                    state.status_message = format!("Print error: {}", e);
                    error!("Print error: {}", e);
                }
            }
            let _ = dev.close();
        }
        Err(e) => {
            state.status_message = format!("Connect error: {}", e);
            error!("Failed to open printer: {}", e);
        }
    }
}

/// Feed tape forward and cut without printing.
fn do_feed_and_cut(state: &mut AppState) {
    use ptouch_core::transport::PtouchDevice;

    state.status_message = "Connecting to printer...".to_string();

    match PtouchDevice::open_first() {
        Ok(mut dev) => {
            if let Err(e) = dev.init() {
                state.status_message = format!("Init error: {}", e);
                let _ = dev.close();
                return;
            }
            match dev.feed_and_cut() {
                Ok(()) => {
                    state.status_message = "Feed & cut done".to_string();
                    info!("Feed and cut successful");
                }
                Err(e) => {
                    state.status_message = format!("Feed & cut error: {}", e);
                    error!("Feed & cut error: {}", e);
                }
            }
            let _ = dev.close();
        }
        Err(e) => {
            state.status_message = format!("Connect error: {}", e);
            error!("Failed to open printer: {}", e);
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
