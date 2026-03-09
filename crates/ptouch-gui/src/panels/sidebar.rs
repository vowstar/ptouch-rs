// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Left sidebar panel: printer info, tape selection, and element list.

use log::{error, info};

use ptouch_core::tape;

use crate::state::AppState;

/// Render the left sidebar.
pub fn show_sidebar(ui: &mut egui::Ui, state: &mut AppState) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        show_printer_section(ui, state);
        ui.separator();
        show_tape_section(ui, state);
        ui.separator();
        show_elements_section(ui, state);
    });
}

/// Printer connection section.
fn show_printer_section(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading("Printer");
    ui.add_space(4.0);

    let model_text = state.printer_model.as_deref().unwrap_or("Not connected");
    ui.label(format!("Model: {}", model_text));

    let status_text = state.printer_status.as_deref().unwrap_or("--");
    ui.label(format!("Status: {}", status_text));

    ui.add_space(4.0);
    if ui.button("Connect").clicked() {
        connect_printer(state);
    }
}

/// Try to open a printer and read its status.
fn connect_printer(state: &mut AppState) {
    use ptouch_core::transport::PtouchDevice;

    state.status_message = "Connecting...".to_string();

    match PtouchDevice::open_first() {
        Ok(mut dev) => {
            if let Err(e) = dev.init() {
                state.printer_status = Some(format!("Init error: {}", e));
                state.status_message = format!("Init error: {}", e);
                return;
            }
            match dev.get_status() {
                Ok(status) => {
                    let width_mm = status.media_width;
                    state.printer_status = Some("Connected".to_string());
                    state.printer_model =
                        Some(format!("{} mm {}", width_mm, status.media_type_name()));

                    // Update tape width from printer
                    if width_mm > 0 {
                        state.tape_width_mm = width_mm;
                        state.update_tape_pixels();
                        state.mark_dirty();
                    }

                    state.status_message = "Printer connected".to_string();
                    info!("Printer connected: {} mm tape", width_mm);
                }
                Err(e) => {
                    state.printer_status = Some(format!("Error: {}", e));
                    state.status_message = format!("Status error: {}", e);
                    error!("Status error: {}", e);
                }
            }
            let _ = dev.close();
        }
        Err(e) => {
            state.printer_status = Some("Not found".to_string());
            state.status_message = format!("Connect error: {}", e);
            error!("Connect error: {}", e);
        }
    }
}

/// Tape width selection section.
fn show_tape_section(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading("Tape");
    ui.add_space(4.0);

    let tapes = tape::supported_tapes();
    let current_label = format!("{} mm ({} px)", state.tape_width_mm, state.tape_width_px);

    egui::ComboBox::from_label("Width")
        .selected_text(&current_label)
        .show_ui(ui, |ui| {
            for t in tapes {
                let label = format!("{} mm ({} px)", t.width_mm, t.pixels);
                if ui
                    .selectable_value(&mut state.tape_width_mm, t.width_mm, &label)
                    .clicked()
                {
                    state.tape_width_px = u32::from(t.pixels);
                    state.mark_dirty();
                    info!("Tape changed: {} mm", t.width_mm);
                }
            }
        });
}

/// Element list section with reorder and delete controls.
fn show_elements_section(ui: &mut egui::Ui, state: &mut AppState) {
    ui.heading("Elements");
    ui.add_space(4.0);

    if state.elements.is_empty() {
        ui.label("(no elements)");
        return;
    }

    let mut action: Option<ElementAction> = None;

    for (idx, element) in state.elements.iter().enumerate() {
        let is_selected = state.selected_element == Some(idx);
        let label = format!("{}. {}", idx + 1, element.display_name());

        ui.horizontal(|ui| {
            if ui.selectable_label(is_selected, &label).clicked() {
                state.selected_element = Some(idx);
            }
        });
    }

    ui.add_space(4.0);

    ui.horizontal(|ui| {
        let has_selection = state.selected_element.is_some();

        if ui
            .add_enabled(
                has_selection && state.selected_element.unwrap_or(0) > 0,
                egui::Button::new("Up"),
            )
            .clicked()
        {
            if let Some(idx) = state.selected_element {
                action = Some(ElementAction::MoveUp(idx));
            }
        }

        if ui
            .add_enabled(
                has_selection
                    && state
                        .selected_element
                        .map(|i| i + 1 < state.elements.len())
                        .unwrap_or(false),
                egui::Button::new("Down"),
            )
            .clicked()
        {
            if let Some(idx) = state.selected_element {
                action = Some(ElementAction::MoveDown(idx));
            }
        }

        if ui
            .add_enabled(has_selection, egui::Button::new("Delete"))
            .clicked()
        {
            if let Some(idx) = state.selected_element {
                action = Some(ElementAction::Delete(idx));
            }
        }
    });

    // Apply deferred action
    if let Some(act) = action {
        match act {
            ElementAction::MoveUp(idx) => {
                if idx > 0 {
                    state.elements.swap(idx, idx - 1);
                    state.selected_element = Some(idx - 1);
                    state.mark_dirty();
                }
            }
            ElementAction::MoveDown(idx) => {
                if idx + 1 < state.elements.len() {
                    state.elements.swap(idx, idx + 1);
                    state.selected_element = Some(idx + 1);
                    state.mark_dirty();
                }
            }
            ElementAction::Delete(idx) => {
                state.elements.remove(idx);
                state.validate_selection();
                state.mark_dirty();
            }
        }
    }
}

/// Deferred actions for element list manipulation.
enum ElementAction {
    MoveUp(usize),
    MoveDown(usize),
    Delete(usize),
}
