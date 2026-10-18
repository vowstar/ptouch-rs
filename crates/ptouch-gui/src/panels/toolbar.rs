// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Top toolbar panel with element addition and action buttons.

use std::path::PathBuf;

use log::{error, info};

use ptouch_render::document::LabelDocument;
use ptouch_render::raster;

use crate::state::{AppState, LabelElement, PrinterCommand};

/// Toolbar actions run after properties commit pending keyboard edits.
#[derive(Clone, Copy)]
pub enum ToolbarAction {
    Undo,
    Redo,
    Convert,
    AddText,
    AddImage,
    AddLine,
    CutMark,
    Padding,
    Print,
    Feed,
    Cancel,
    Export,
    Save,
    Open,
}

/// Render the toolbar without consuming a document that still has pending edits.
pub fn show_toolbar(ui: &mut egui::Ui, state: &AppState) -> Option<ToolbarAction> {
    let mut action = None;
    if !state.is_free_layout() {
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("Legacy objects (format v{})", state.document_version));
            if ui.button("Convert to free layout")
                .on_hover_text("Fix every object's current position in millimeters. Natural font sizing and rotated image fitting may change; review the preview before saving.")
                .clicked() { action = Some(ToolbarAction::Convert); }
            ui.small("Legacy spacing stays unchanged until converted.");
        });
    }
    ui.horizontal(|ui| {
        if ui
            .add_enabled(state.history.can_undo(), egui::Button::new("Undo"))
            .clicked()
        {
            action = Some(ToolbarAction::Undo);
        }
        if ui
            .add_enabled(state.history.can_redo(), egui::Button::new("Redo"))
            .clicked()
        {
            action = Some(ToolbarAction::Redo);
        }
        ui.separator();
        for (label, command) in [
            ("Add Text", ToolbarAction::AddText),
            ("Add Image", ToolbarAction::AddImage),
            ("Cut Mark", ToolbarAction::CutMark),
        ] {
            if ui.button(label).clicked() {
                action = Some(command);
            }
        }
        if ui
            .add_enabled(state.is_free_layout(), egui::Button::new("Add Line"))
            .on_hover_text("Add a printed line. Convert legacy layouts to free layout first.")
            .clicked()
        {
            action = Some(ToolbarAction::AddLine);
        }
        if !state.is_free_layout() && ui.button("Padding").clicked() {
            action = Some(ToolbarAction::Padding);
        }
        ui.separator();
        let ready = state.printer_connected && !state.is_printer_busy();
        let bitmap = state.preview_bitmap.is_some() && !state.needs_rerender;
        if ui
            .add_enabled(ready && bitmap, egui::Button::new("Print"))
            .clicked()
        {
            action = Some(ToolbarAction::Print);
        }
        if ui
            .add_enabled(
                ready && !state.printer_target.is_bluetooth(),
                egui::Button::new("Feed & Cut"),
            )
            .clicked()
        {
            action = Some(ToolbarAction::Feed);
        }
        if state.operation_in_progress
            && !state.printer_target.is_bluetooth()
            && ui.button("Cancel").clicked()
        {
            action = Some(ToolbarAction::Cancel);
        }
        if ui
            .add_enabled(bitmap, egui::Button::new("Export Image"))
            .clicked()
        {
            action = Some(ToolbarAction::Export);
        }
        ui.separator();
        if ui.button("Save Layout").clicked() {
            action = Some(ToolbarAction::Save);
        }
        if ui.button("Open Layout").clicked() {
            action = Some(ToolbarAction::Open);
        }
    });
    action
}

pub fn execute_action(state: &mut AppState, action: ToolbarAction) {
    match action {
        ToolbarAction::Undo => state.history_step(false),
        ToolbarAction::Redo => state.history_step(true),
        ToolbarAction::Convert => match state.convert_to_free_layout() {
            Ok(()) => {
                state.status_message =
                    "Converted to free layout. Review text sizing and rotated images before saving."
                        .into()
            }
            Err(error) => state.status_message = format!("Conversion error: {error}"),
        },
        ToolbarAction::AddText => state.add_text(),
        ToolbarAction::AddLine => state.add_line(),
        ToolbarAction::AddImage => {
            if let Some(path) = crate::widgets::image_file_dialog().pick_file() {
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        if state.add_image_bytes(Some(path.clone()), bytes) {
                            info!("Loaded image: {}", path.display());
                        } else {
                            state.status_message = format!("Image load error: {}", path.display());
                        }
                    }
                    Err(error) => state.status_message = format!("Image read error: {error}"),
                }
            }
        }
        ToolbarAction::CutMark => state.add_cut_mark(),
        ToolbarAction::Padding => {
            state.elements.push(LabelElement::Padding { pixels: 20 });
            state.selected_element = Some(state.elements.len() - 1);
            state.mark_dirty();
        }
        ToolbarAction::Print => {
            if !state.printer_connected || state.is_printer_busy() || state.needs_rerender {
                return;
            }
            if let Some(bitmap) = &state.preview_bitmap
                && let Some(tx) = &state.printer_cmd_tx
            {
                let corrected = match ptouch_render::calibration::apply_calibration(
                    bitmap,
                    state.target_dpi(),
                    state
                        .calibration_enabled
                        .then_some(state.calibration.offset_mm),
                ) {
                    Ok(bitmap) => bitmap,
                    Err(error) => {
                        state.status_message = error.to_string();
                        return;
                    }
                };
                let cancellation = ptouch_core::CancellationToken::default();
                state.cancellation = Some(cancellation.clone());
                let _ = tx.send(PrinterCommand::Print {
                    raster_lines: raster::bitmap_to_raster_lines(&corrected, state.printer_max_px),
                    chain_print: !state.auto_cut,
                    auto_cut: state.auto_cut,
                    quality: state.print_quality,
                    target: state.printer_target.clone(),
                    cancellation,
                });
                state.operation_in_progress = true;
                state.status_message = "Printing...".into();
            }
        }
        ToolbarAction::Feed => {
            if state.printer_connected
                && !state.is_printer_busy()
                && !state.printer_target.is_bluetooth()
                && let Some(tx) = &state.printer_cmd_tx
            {
                let cancellation = ptouch_core::CancellationToken::default();
                state.cancellation = Some(cancellation.clone());
                let _ = tx.send(PrinterCommand::FeedAndCut {
                    target: state.printer_target.clone(),
                    cancellation,
                });
                state.operation_in_progress = true;
                state.status_message = "Feeding & cutting...".into();
            }
        }
        ToolbarAction::Cancel => {
            if let Some(token) = &state.cancellation {
                token.cancel();
            }
            state.status_message = "Cancelling; sent data may still print".into();
        }
        ToolbarAction::Export => do_export_image(state),
        ToolbarAction::Save => do_save_layout(state),
        ToolbarAction::Open => do_open_layout(state),
    }
}

/// Save the current design to a `.ptl` layout file (TOML with embedded images).
fn do_save_layout(state: &mut AppState) {
    if state.elements.is_empty() {
        state.status_message = "Nothing to save".to_string();
        return;
    }

    let document = state.document();

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

    match LabelDocument::from_toml_str(&text)
        .map_err(|error| error.to_string())
        .and_then(|document| apply_layout(state, document))
    {
        Ok(()) => {
            state.status_message = format!("Opened {}", path.display());
            info!("Opened layout: {}", path.display());
        }
        Err(e) => {
            state.status_message = format!("Open error: {}", e);
            error!("Layout parse error: {}", e);
        }
    }
}

fn apply_layout(state: &mut AppState, document: LabelDocument) -> Result<(), String> {
    // Reject before touching the current design: an unknown width must not
    // inherit the previous document's raster height or differ from CLI export.
    if ptouch_core::tape::find_tape(document.tape_width_mm, document.dpi).is_none() {
        return Err(format!(
            "Unknown saved tape width: {} mm",
            document.tape_width_mm
        ));
    }
    state.document_version = document.version;
    state.design_dpi = document.dpi;
    if !state.printer_connected {
        state.preview_uses_printer = false;
    }
    if !state.printer_target.is_bluetooth() || !state.printer_connected {
        state.tape_width_mm = document.tape_width_mm;
        state.update_tape_pixels();
    }
    state.font_name = document.font_name;
    state.font_margin = document.font_margin;
    state.overall_flip_h = document.flip_h;
    state.overall_flip_v = document.flip_v;
    state.editor_generation = state.editor_generation.wrapping_add(1);
    state.elements = document.elements;
    state.select_canvas_target(None);
    state.selection_revision = state.selection_revision.wrapping_add(1);
    state.mark_dirty();
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PrinterTarget;
    use ptouch_render::text::TextRenderer;

    #[test]
    fn print_calibration_matches_shared_output_without_editing_the_layout() {
        for offset in [None, Some(1.0), Some(-0.4), Some(-10.0)] {
            let (tx, rx) = std::sync::mpsc::channel();
            let mut bitmap = ptouch_render::bitmap::LabelBitmap::new(80, 32);
            bitmap.set_pixel(10, 3, true);
            let mut state = AppState {
                printer_connected: true,
                printer_max_px: 128,
                preview_bitmap: Some(bitmap.clone()),
                needs_rerender: false,
                calibration_enabled: offset.is_some(),
                printer_cmd_tx: Some(tx),
                ..Default::default()
            };
            state.add_text();
            state.needs_rerender = false;
            state.calibration.offset_mm = offset.unwrap_or(20.0);
            let before = state.document().to_toml_string().unwrap();
            execute_action(&mut state, ToolbarAction::Print);
            let expected = ptouch_render::calibration::apply_calibration(&bitmap, 180, offset);
            if let Ok(expected) = expected {
                let PrinterCommand::Print { raster_lines, .. } = rx.try_recv().unwrap() else {
                    panic!("expected print");
                };
                assert_eq!(raster_lines, raster::bitmap_to_raster_lines(&expected, 128));
                assert!(state.operation_in_progress);
            } else {
                assert!(rx.try_recv().is_err());
                assert!(!state.operation_in_progress);
                assert!(state.status_message.contains("calibration"));
            }
            assert_eq!(state.document().to_toml_string().unwrap(), before);
        }
    }

    fn layout(tape_width_mm: u8) -> LabelDocument {
        let document = LabelDocument {
            version: ptouch_render::document::DOCUMENT_VERSION,
            tape_width_mm,
            dpi: 180,
            font_name: "DejaVuSans".into(),
            font_margin: 0,
            flip_h: false,
            flip_v: false,
            elements: vec![LabelElement::CutMark],
        };
        LabelDocument::from_toml_str(&document.to_toml_string().unwrap()).unwrap()
    }

    #[test]
    fn opening_unknown_tape_width_rejects_without_changing_the_current_design() {
        let mut state = AppState::default();
        state.add_text();
        state.needs_rerender = false;
        let before = state.document().to_toml_string().unwrap();
        let mut document = layout(15);
        document.dpi = 360;
        assert_eq!(
            apply_layout(&mut state, document).unwrap_err(),
            "Unknown saved tape width: 15 mm"
        );
        assert_eq!(state.document().to_toml_string().unwrap(), before);
        assert_eq!(state.tape_width_px, 76);
        assert_eq!(state.selected_element, Some(0));
        assert!(!state.needs_rerender);
    }

    #[test]
    fn opening_and_saving_legacy_layout_keeps_version_and_flow() {
        for version in [1, 2] {
            let mut document = layout(12);
            document.version = version;
            document.elements = vec![LabelElement::Padding { pixels: 20 }];
            let mut state = AppState::default();
            apply_layout(&mut state, document).unwrap();
            state.add_text();
            assert!(!state.is_free_layout());
            let saved = state.document();
            assert_eq!(saved.version, version);
            assert!(matches!(
                saved.elements[1],
                LabelElement::Text { text_box: None, .. }
            ));
            let reopened = LabelDocument::from_toml_str(&saved.to_toml_string().unwrap()).unwrap();
            assert_eq!(reopened.version, version);
        }
    }

    #[test]
    fn explicit_conversion_is_saved_and_reopened_as_free_layout() {
        let mut document = layout(12);
        document.version = 1;
        document.elements = vec![LabelElement::Padding { pixels: 20 }, LabelElement::CutMark];
        let mut state = AppState::default();
        apply_layout(&mut state, document).unwrap();
        state.convert_to_free_layout().unwrap();
        let saved = state.document().to_toml_string().unwrap();
        let mut reopened = AppState::default();
        apply_layout(&mut reopened, LabelDocument::from_toml_str(&saved).unwrap()).unwrap();
        assert!(reopened.is_free_layout());
        assert!(
            reopened
                .elements
                .iter()
                .all(|element| element.as_object().is_some())
        );
        assert_eq!(reopened.elements.len(), 2);
    }

    #[test]
    fn opening_layout_keeps_bluetooth_tape_geometry() {
        for width in [12, 24] {
            let mut state = AppState {
                printer_target: PrinterTarget::Bluetooth {
                    name: "PT-P300BT".into(),
                    address: "AA:BB:CC:DD:EE:FF".into(),
                },
                printer_connected: true,
                printer_max_px: 128,
                tape_width_mm: 12,
                tape_width_px: 64,
                ..AppState::default()
            };
            apply_layout(&mut state, layout(width)).unwrap();
            assert_eq!((state.tape_width_mm, state.tape_width_px), (12, 64));
            let bitmap = ptouch_render::document::render_elements(
                &state.elements,
                state.tape_width_px,
                &state.font_name,
                state.font_margin,
                &mut TextRenderer::new(),
            )
            .unwrap()
            .unwrap();
            let lines = raster::bitmap_to_raster_lines(&bitmap, state.printer_max_px);
            assert!(
                lines
                    .iter()
                    .all(|line| line[..4].iter().chain(&line[12..]).all(|&byte| byte == 0))
            );
        }
    }

    #[test]
    fn opening_usb_layout_uses_saved_tape_width() {
        let mut state = AppState {
            tape_width_mm: 24,
            tape_width_px: 128,
            ..AppState::default()
        };
        apply_layout(&mut state, layout(12)).unwrap();
        assert_eq!((state.tape_width_mm, state.tape_width_px), (12, 76));
    }

    #[test]
    fn opening_offline_layout_restores_design_dpi_after_printer_disconnect() {
        let mut state = AppState {
            printer_dpi: 180,
            printer_max_px: 128,
            preview_uses_printer: true,
            ..AppState::default()
        };
        let mut document = layout(24);
        document.dpi = 360;
        apply_layout(&mut state, document).unwrap();
        assert_eq!(state.design_dpi, 360);
        assert_eq!(state.target_dpi(), 360);
        assert_eq!(state.tape_width_px, 320);
        assert_eq!(state.document().dpi, 360);
    }

    #[test]
    fn connected_printer_overrides_preview_dpi_without_changing_saved_design() {
        let mut state = AppState {
            printer_connected: true,
            printer_dpi: 180,
            printer_max_px: 128,
            ..AppState::default()
        };
        let mut document = layout(24);
        document.dpi = 360;
        apply_layout(&mut state, document).unwrap();
        assert_eq!(state.target_dpi(), 180);
        assert_eq!(state.tape_width_px, 128);
        let saved = state.document();
        assert_eq!(saved.dpi, 360);
        let reopened = LabelDocument::from_toml_str(&saved.to_toml_string().unwrap()).unwrap();
        let mut offline = AppState::default();
        apply_layout(&mut offline, reopened).unwrap();
        assert_eq!(offline.target_dpi(), 360);
        assert_eq!(offline.tape_width_px, 320);
    }

    #[test]
    fn opening_layout_offline_releases_previous_bluetooth_geometry() {
        let mut state = AppState {
            printer_target: PrinterTarget::Bluetooth {
                name: "PT-P300BT".into(),
                address: "AA:BB:CC:DD:EE:FF".into(),
            },
            printer_max_px: 128,
            preview_uses_printer: true,
            tape_width_px: 64,
            ..AppState::default()
        };
        let mut document = layout(24);
        document.dpi = 360;
        apply_layout(&mut state, document).unwrap();
        assert_eq!(state.target_dpi(), 360);
        assert_eq!((state.tape_width_mm, state.tape_width_px), (24, 320));
    }
}
