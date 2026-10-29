// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Output selection is task state. It never changes or enters a layout file.

use std::{borrow::Cow, ops::Range};

use ptouch_render::{
    bitmap::LabelBitmap,
    calibration::apply_calibration,
    output_range::{OutputRange, apply_output_range, output_bounds},
};

use crate::state::AppState;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutputSelection {
    pub enabled: bool,
    pub start_mm: f64,
    pub end_mm: f64,
    initialized: bool,
}

impl OutputSelection {
    pub fn range(&self) -> ptouch_render::Result<Option<OutputRange>> {
        self.enabled
            .then(|| OutputRange::new(self.start_mm, self.end_mm))
            .transpose()
    }

    fn enable(&mut self, bitmap: Option<&LabelBitmap>, dpi: u16) {
        if !self.initialized {
            self.start_mm = 0.0;
            self.end_mm = bitmap.map_or(0.0, |bitmap| {
                f64::from(bitmap.width()) * 25.4 / f64::from(dpi)
            });
            self.initialized = true;
        }
        self.enabled = true;
    }

    pub fn pixel_bounds(
        &self,
        bitmap: &LabelBitmap,
        dpi: u16,
    ) -> ptouch_render::Result<Range<u32>> {
        output_bounds(bitmap, dpi, self.range()?)
    }
}

pub fn reset(state: &mut AppState) {
    state.output_range = OutputSelection::default();
}

/// A range field commits on focus loss. Keep the output action available so
/// that its click can commit the field before `prepare` checks the selection.
pub fn preview_ready(state: &AppState) -> bool {
    !state.needs_rerender && state.preview_bitmap.is_some()
}

/// Output coordinates refer to the final, already mirrored preview. Printer
/// placement correction is applied to the cropped label, never before it.
pub fn prepare(state: &AppState, calibrated: bool) -> Result<Cow<'_, LabelBitmap>, String> {
    if state.needs_rerender {
        return Err("The preview is not ready.".into());
    }
    let bitmap = state
        .preview_bitmap
        .as_ref()
        .ok_or("The label has no printable pixels.")?;
    let range = state.output_range.range().map_err(|e| e.to_string())?;
    let cropped =
        apply_output_range(bitmap, state.target_dpi(), range).map_err(|e| e.to_string())?;
    if calibrated {
        let offset = state
            .calibration_enabled
            .then_some(state.calibration.offset_mm);
        apply_calibration(&cropped, state.target_dpi(), offset)
            .map(Cow::Owned)
            .map_err(|e| e.to_string())
    } else {
        Ok(cropped)
    }
}

pub fn show_controls(ui: &mut egui::Ui, state: &mut AppState) {
    ui.add_space(4.0);
    ui.label("Output");
    ui.scope(|ui| {
        let mut enabled = state.output_range.enabled;
        ui.horizontal(|ui| {
            ui.selectable_value(&mut enabled, false, "Entire label");
            if ui
                .add_enabled(
                    preview_ready(state),
                    egui::Button::selectable(state.output_range.enabled, "Range"),
                )
                .clicked()
            {
                enabled = true;
            }
        });
        // Commit the old interval's focused field before collapsing it.
        if state.output_range.enabled {
            egui::Grid::new(("output_range_values", state.editor_generation))
                .num_columns(2)
                .show(ui, |ui| {
                    for (name, value) in [
                        ("Start", &mut state.output_range.start_mm),
                        ("End", &mut state.output_range.end_mm),
                    ] {
                        ui.label(name);
                        ui.add(crate::widgets::decimal_drag(value).speed(0.1).suffix(" mm"));
                        ui.end_row();
                    }
                });
            if let Some(bitmap) = &state.preview_bitmap {
                match state.output_range.pixel_bounds(bitmap, state.target_dpi()) {
                    Ok(bounds) => {
                        let length = f64::from(bounds.end - bounds.start) * 25.4
                            / f64::from(state.target_dpi());
                        ui.small(format!("Range length: {length:.3} mm"));
                    }
                    Err(error) => {
                        ui.colored_label(ui.visuals().error_fg_color, error.to_string());
                    }
                }
            } else {
                ui.small("Add content to set a print range.");
            }
            ui.small("Print and image exports use this range.");
        }
        if enabled != state.output_range.enabled {
            if enabled {
                let dpi = state.target_dpi();
                state
                    .output_range
                    .enable(state.preview_bitmap.as_ref(), dpi);
            } else {
                state.output_range.enabled = false;
            }
        }
    });
}

/// Shade the omitted tape without changing the document or the object hit map.
pub fn paint_range(painter: &egui::Painter, label_rect: egui::Rect, state: &AppState) {
    if !state.output_range.enabled {
        return;
    }
    let Some(bitmap) = &state.preview_bitmap else {
        return;
    };
    let Ok(bounds) = state.output_range.pixel_bounds(bitmap, state.target_dpi()) else {
        painter.rect_filled(
            label_rect,
            0.0,
            egui::Color32::from_rgba_unmultiplied(170, 30, 30, 55),
        );
        return;
    };
    let left = label_rect.left() + bounds.start as f32 * state.zoom;
    let right = label_rect.left() + bounds.end as f32 * state.zoom;
    let shade = egui::Color32::from_rgba_unmultiplied(65, 70, 80, 145);
    for rect in [
        egui::Rect::from_min_max(label_rect.min, egui::pos2(left, label_rect.bottom())),
        egui::Rect::from_min_max(egui::pos2(right, label_rect.top()), label_rect.max),
    ] {
        if rect.width() > 0.0 {
            painter.rect_filled(rect, 0.0, shade);
        }
    }
    let outline = egui::Stroke::new(1.5, egui::Color32::from_rgb(20, 130, 80));
    for x in [left, right] {
        painter.line_segment(
            [
                egui::pos2(x, label_rect.top()),
                egui::pos2(x, label_rect.bottom()),
            ],
            outline,
        );
    }
}

pub fn export_corrected(state: &mut AppState) {
    let output = match prepare(state, true) {
        Ok(output) => output,
        Err(error) => {
            state.status_message = error;
            return;
        }
    };
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("PNG", &["png"])
        .set_file_name("label-corrected.png")
        .save_file()
    {
        let result = output.save(&path);
        state.status_message = match result {
            Ok(()) => "Corrected image exported".into(),
            Err(error) => error.to_string(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bitmap() -> LabelBitmap {
        let mut bitmap = LabelBitmap::new(300, 12);
        for x in 0..300 {
            for y in 0..12 {
                bitmap.set_pixel(x, y, (x * 3 + y) % 17 < 4);
            }
        }
        bitmap
    }

    fn state(dpi: u16) -> AppState {
        let mut state = AppState {
            design_dpi: dpi,
            printer_dpi: dpi,
            preview_bitmap: Some(bitmap()),
            needs_rerender: false,
            ..Default::default()
        };
        state
            .output_range
            .enable(state.preview_bitmap.as_ref(), dpi);
        state.output_range.start_mm = 10.0;
        state.output_range.end_mm = 20.0;
        state
    }

    #[test]
    fn output_range_uses_the_final_preview_at_each_dpi_and_mirror() {
        for dpi in [180, 360] {
            for horizontal in [false, true] {
                for vertical in [false, true] {
                    let mut state = state(dpi);
                    let source = bitmap().mirrored(horizontal, vertical);
                    state.preview_bitmap = Some(source.clone());
                    let selected = prepare(&state, false).unwrap();
                    let start = (10.0 * f64::from(dpi) / 25.4).round() as u32;
                    let end = (20.0 * f64::from(dpi) / 25.4).round() as u32;
                    assert_eq!(selected.width(), end - start);
                    for x in 0..selected.width() {
                        for y in 0..selected.height() {
                            assert_eq!(selected.get_pixel(x, y), source.get_pixel(x + start, y));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn normal_export_ignores_calibration_and_corrected_output_applies_it_after_crop() {
        for dpi in [180, 360] {
            let mut state = state(dpi);
            state.calibration_enabled = true;
            state.calibration.offset_mm = 1.0;
            let normal = prepare(&state, false).unwrap();
            let expected = apply_calibration(&normal, dpi, Some(1.0)).unwrap();
            assert_eq!(*prepare(&state, true).unwrap(), expected);
            state.calibration.offset_mm = -1.0;
            let normal = prepare(&state, false).unwrap();
            assert!(normal.width() > 0);
            assert!(prepare(&state, true).is_err());
        }
    }

    #[test]
    fn range_and_calibration_are_task_state_and_reset_is_not_an_edit() {
        let mut state = state(180);
        let before = state.document();
        let revision = state.edit_revision;
        let _ = prepare(&state, false).unwrap();
        let _ = prepare(&state, true).unwrap();
        assert_eq!(state.document(), before);
        assert_eq!(state.edit_revision, revision);
        reset(&mut state);
        assert_eq!(state.output_range, OutputSelection::default());
        assert_eq!(state.document(), before);
        assert_eq!(state.edit_revision, revision);
        assert!(matches!(prepare(&state, false).unwrap(), Cow::Borrowed(_)));
    }

    #[test]
    fn changed_dpi_length_and_pending_preview_are_rechecked_without_clamping_values() {
        let mut state = state(180);
        state.output_range.end_mm = 30.0;
        assert!(prepare(&state, false).is_ok());
        let requested = state.output_range.clone();
        state.design_dpi = 360;
        assert!(prepare(&state, false).is_err());
        assert_eq!(state.output_range, requested);
        state.preview_bitmap = Some(LabelBitmap::new(600, 12));
        assert!(prepare(&state, false).is_ok());
        state.preview_bitmap = Some(LabelBitmap::new(200, 12));
        assert!(prepare(&state, false).is_err());
        assert_eq!(state.output_range, requested);
        state.output_range.enabled = false;
        state.needs_rerender = true;
        assert!(!preview_ready(&state));
        assert!(prepare(&state, false).is_err());
    }

    #[test]
    fn range_controls_accept_a_pointer_selection_without_editing_the_document() {
        let mut state = state(180);
        reset(&mut state);
        let original = state.document();
        let context = egui::Context::default();
        let mut range = None;
        let mut frame = |state: &mut AppState, events| {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(260.0, 500.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| show_controls(ui, state),
            );
            output.textures_delta.clear();
            for shape in output.shapes {
                if let egui::Shape::Text(text) = shape.shape
                    && text.galley.text() == "Range"
                {
                    range = Some(text.pos + text.galley.size() / 2.0);
                }
            }
        };
        frame(&mut state, vec![]);
        let pos = range.unwrap();
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(260.0, 500.0),
                )),
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
            |ui| show_controls(ui, &mut state),
        );
        output.textures_delta.clear();
        assert!(state.output_range.enabled);
        assert_eq!(state.output_range.start_mm, 0.0);
        assert_eq!(state.output_range.end_mm, 300.0 * 25.4 / 180.0);
        assert_eq!(state.document(), original);
        assert!(!state.history.can_undo());
    }

    #[test]
    fn print_click_commits_a_valid_end_value_before_range_validation() {
        use crate::panels::toolbar::{ToolbarAction, execute_action, show_toolbar};
        let context = egui::Context::default();
        let mut state = state(180);
        state.output_range.end_mm = 5.0;
        state.printer_connected = true;
        state.printer_max_px = 128;
        let (tx, rx) = std::sync::mpsc::channel();
        state.printer_cmd_tx = Some(tx);
        let frame = |state: &mut AppState, events| {
            let mut action = None;
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1400.0, 700.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    action = show_toolbar(ui, state);
                    show_controls(ui, state);
                },
            );
            output.textures_delta.clear();
            let text: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| {
                    if let egui::Shape::Text(text) = &shape.shape {
                        Some((
                            text.galley.text().to_owned(),
                            text.galley.rect.translate(text.pos.to_vec2()),
                        ))
                    } else {
                        None
                    }
                })
                .collect();
            (action, text)
        };
        let click = |pos| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ]
        };
        let (_, shapes) = frame(&mut state, vec![]);
        let end = shapes.iter().find(|(text, _)| text == "End").unwrap().1;
        let end_value = shapes
            .iter()
            .find(|(text, rect)| {
                text.ends_with(" mm")
                    && rect.left() > end.right()
                    && (rect.center().y - end.center().y).abs() < 3.0
            })
            .unwrap()
            .1;
        let print = shapes.iter().find(|(text, _)| text == "Print").unwrap().1;
        frame(&mut state, click(end_value.center()));
        frame(&mut state, vec![]);
        frame(
            &mut state,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::COMMAND,
                },
                egui::Event::Text("30".into()),
            ],
        );
        assert_eq!(state.output_range.end_mm, 5.0);
        let (action, _) = frame(&mut state, click(print.center()));
        assert_eq!(state.output_range.end_mm, 30.0);
        assert!(matches!(action, Some(ToolbarAction::Print)));
        execute_action(&mut state, action.unwrap());
        let crate::state::PrinterCommand::Print { raster_lines, .. } = rx.try_recv().unwrap()
        else {
            panic!("expected a cropped print command");
        };
        assert_eq!(raster_lines.len(), 142);
    }

    #[test]
    fn range_remains_editable_across_busy_states_and_window_focus_changes() {
        for busy in 0..3 {
            let mut state = state(180);
            state.operation_in_progress = busy == 0;
            state.connecting = busy == 1;
            state.bluetooth_discovering = busy == 2;
            let original = state.document();
            let ctx = egui::Context::default();
            let frame = |state: &mut AppState, events| {
                let mut output = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(300.0, 500.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| show_controls(ui, state),
                );
                output.textures_delta.clear();
                output
                    .shapes
                    .into_iter()
                    .filter_map(|shape| match shape.shape {
                        egui::Shape::Text(text) => Some((
                            text.galley.text().to_owned(),
                            text.galley.rect.translate(text.pos.to_vec2()),
                        )),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            };
            frame(&mut state, vec![]);
            frame(&mut state, vec![egui::Event::WindowFocused(false)]);
            let text = frame(&mut state, vec![egui::Event::WindowFocused(true)]);
            let label = text.iter().find(|(text, _)| text == "End").unwrap().1;
            let pos = text
                .iter()
                .find(|(text, rect)| {
                    text.ends_with(" mm")
                        && rect.left() > label.right()
                        && (rect.center().y - label.center().y).abs() < 3.0
                })
                .unwrap()
                .1
                .center();
            frame(
                &mut state,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    },
                ],
            );
            frame(
                &mut state,
                vec![
                    egui::Event::Key {
                        key: egui::Key::A,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    },
                    egui::Event::Text("25,5".into()),
                ],
            );
            frame(
                &mut state,
                vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
            );
            assert_eq!(state.output_range.end_mm, 25.5, "busy state {busy}");
            assert!(state.is_printer_busy());
            assert_eq!(state.document(), original);
        }
    }

    #[test]
    fn editing_next_range_cannot_change_a_queued_print_or_submit_another() {
        use crate::panels::toolbar::{ToolbarAction, execute_action};
        let mut state = state(180);
        state.printer_connected = true;
        state.printer_max_px = 128;
        let (tx, rx) = std::sync::mpsc::channel();
        state.printer_cmd_tx = Some(tx);
        execute_action(&mut state, ToolbarAction::Print);
        assert!(state.operation_in_progress);
        state.output_range.end_mm = 30.0;
        assert_eq!(prepare(&state, false).unwrap().width(), 142);
        execute_action(&mut state, ToolbarAction::Print);
        let crate::state::PrinterCommand::Print { raster_lines, .. } = rx.try_recv().unwrap()
        else {
            panic!("expected print")
        };
        assert_eq!(raster_lines.len(), 71);
        assert!(rx.try_recv().is_err());
    }
}
