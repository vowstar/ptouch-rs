// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Bottom status bar panel.

use crate::state::AppState;

/// Render the bottom status bar.
pub fn show_status_bar(ui: &mut egui::Ui, state: &mut AppState) {
    ui.horizontal(|ui| {
        // Printer status
        let printer = state.printer_model.as_deref().unwrap_or("--");
        ui.label(format!("Printer: {}", printer));

        ui.separator();

        // Tape info
        ui.label(format!("Tape: {} mm", state.tape_width_mm));

        ui.separator();

        // Label dimensions
        if let Some(ref bmp) = state.preview_bitmap {
            ui.label(format!(
                "{}x{} px | {:.1} mm long",
                bmp.width(),
                bmp.height(),
                f64::from(bmp.width()) * 25.4 / f64::from(state.target_dpi())
            ));
        } else {
            ui.label("-- x -- px");
        }

        ui.separator();

        // Zoom level
        ui.label(format!("Zoom: {:.0}%", state.zoom * 100.0));

        ui.separator();

        // Status message (right-aligned)
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(&state.status_message).truncate())
                .on_hover_text(
                    state
                        .status_tooltip
                        .as_ref()
                        .filter(|(message, _)| message == &state.status_message)
                        .map_or(state.status_message.as_str(), |(_, detail)| detail.as_str()),
                );
        });
    });
    // Keep a separate, stable-height row so hover feedback never replaces an
    // error/save message or changes the canvas origin during an active drag.
    let hint = state.canvas_hint().unwrap_or_else(|| {
        "Select an object. Use Adjust layout for positioning and spacing.".into()
    });
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui
            .small_button(concat!("v", env!("CARGO_PKG_VERSION")))
            .on_hover_text("Application version · click for Info")
            .clicked()
        {
            state.info_open = true;
        }
        ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(egui::RichText::new(&hint).small()).truncate())
                .on_hover_text(hint);
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{CanvasEditTarget, CanvasTarget};
    use ptouch_render::document::{LabelElement, LabelObject, TextBox};

    #[test]
    fn version_is_visible_and_opens_info_at_narrow_width_without_editing() {
        for width in [800.0, 1200.0, 1800.0] {
            let mut state = AppState {
                status_message: "Render error: ".repeat(100),
                ..Default::default()
            };
            let before = state.document();
            let context = egui::Context::default();
            let input = |events| egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 100.0),
                )),
                events,
                ..Default::default()
            };
            let mut output = context.run_ui(input(vec![]), |ui| show_status_bar(ui, &mut state));
            let text = output
                .shapes
                .iter()
                .find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text)
                        if text.galley.text() == concat!("v", env!("CARGO_PKG_VERSION")) =>
                    {
                        Some(text)
                    }
                    _ => None,
                })
                .unwrap();
            let rect = egui::Rect::from_min_size(text.pos, text.galley.size());
            assert!(rect.right() <= width && rect.left() >= 0.0);
            let pos = rect.center();
            output.textures_delta.clear();
            for pressed in [true, false] {
                let mut output = context.run_ui(
                    input(vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ]),
                    |ui| show_status_bar(ui, &mut state),
                );
                output.textures_delta.clear();
            }
            assert!(state.info_open);
            assert_eq!(state.document(), before);
        }
    }

    #[test]
    fn interaction_hint_and_operation_error_are_visible_together() {
        let mut state = AppState {
            elements: vec![LabelElement::Object(LabelObject::text(
                "PORT1",
                TextBox::default(),
            ))],
            status_message: "Render error: invalid dimensions".into(),
            ..AppState::default()
        };
        state.canvas_hover_target = Some(CanvasTarget {
            element: 0,
            edit: CanvasEditTarget::Slot,
        });
        let context = egui::Context::default();
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 100.0),
                )),
                ..Default::default()
            },
            |ui| show_status_bar(ui, &mut state),
        );
        output.textures_delta.clear();
        let labels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(shape) => Some(shape.galley.text()),
                _ => None,
            })
            .collect();
        assert!(labels.contains(&"Render error: invalid dimensions"));
        assert!(
            labels
                .iter()
                .any(|text| text.contains("Candidate object 1 · Layout:")
                    && text.contains("0 following objects"))
        );
        assert_eq!(state.status_message, "Render error: invalid dimensions");
    }
}
