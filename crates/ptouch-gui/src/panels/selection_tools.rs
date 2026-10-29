// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Edit only the submitted property across the current selection.

use crate::{batch_edit::SharedEdit, state::AppState};
use ptouch_render::{
    document::ObjectContent,
    object_layout::Placement,
    styled_text::{StylePatch, style_regions},
};

#[derive(Clone)]
struct NumberDraft {
    text: String,
    changed: bool,
    source: Option<f64>,
    invalid: bool,
}

fn common(values: &[f64]) -> Option<f64> {
    let first = *values.first()?;
    values
        .iter()
        .all(|value| (*value - first).abs() < 1e-9)
        .then_some(first)
}

fn number(
    ui: &mut egui::Ui,
    revision: u64,
    label: &str,
    values: &[f64],
    placeholder: &str,
) -> Option<f64> {
    let value = common(values);
    let id = ui.id().with(("shared_number", revision, label));
    let mut draft = ui
        .ctx()
        .data_mut(|data| data.get_temp::<NumberDraft>(id))
        .unwrap_or(NumberDraft {
            text: value.map_or(String::new(), |value| format!("{value:.3}")),
            changed: false,
            source: value,
            invalid: false,
        });
    if !draft.changed && (draft.source != value || !ui.memory(|memory| memory.has_focus(id))) {
        draft.text = value.map_or(String::new(), |value| format!("{value:.3}"));
        draft.source = value;
    }
    let response = ui
        .horizontal(|ui| {
            ui.label(label);
            ui.add(
                egui::TextEdit::singleline(&mut draft.text)
                    .id(id)
                    .desired_width(110.0)
                    .hint_text(placeholder),
            )
        })
        .inner;
    if response.changed() {
        draft.invalid = false;
    }
    draft.changed |= response.changed();
    let commit = draft.changed
        && (response.lost_focus()
            || response.has_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)));
    let result = if commit {
        draft.changed = false;
        let result = crate::widgets::parse_decimal(&draft.text).filter(|value| value.is_finite());
        draft.invalid = result.is_none();
        draft.text = value.map_or(String::new(), |value| format!("{value:.3}"));
        result
    } else {
        None
    };
    if draft.invalid {
        ui.colored_label(ui.visuals().error_fg_color, "Enter a number.");
    }
    ui.ctx().data_mut(|data| data.insert_temp(id, draft));
    result
}

pub fn show(ui: &mut egui::Ui, state: &mut AppState) {
    let indices = state.selected_indices();
    ui.heading(format!("{} objects selected", indices.len()));
    let mut affected: std::collections::BTreeSet<_> = indices.iter().copied().collect();
    for index in &indices {
        affected.extend(ptouch_render::scene::children(&state.elements, *index));
    }
    let extra = affected.len() - indices.len();
    if extra > 0 {
        ui.small(format!("Copy and Delete include {extra} attached objects."));
    }
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        if ui.button("Text only").clicked() {
            action = Some(0);
        }
        if ui
            .button("Duplicate")
            .on_hover_text("This action includes attached objects.")
            .clicked()
        {
            action = Some(1);
        }
        if ui
            .button("Delete")
            .on_hover_text("Delete selected objects and their attached objects.")
            .clicked()
        {
            action = Some(2);
        }
    });
    let objects: Vec<_> = indices
        .iter()
        .filter_map(|index| state.elements[*index].as_object())
        .collect();
    if objects.len() != indices.len() {
        ui.label("Convert the layout to edit multiple objects.");
        apply_action(state, action);
        return;
    }
    let all_text = objects
        .iter()
        .all(|object| matches!(object.content, ObjectContent::Text { .. }));
    let same_type = objects.iter().all(|object| {
        std::mem::discriminant(&object.content) == std::mem::discriminant(&objects[0].content)
    });
    let hosts: Vec<_> = objects
        .iter()
        .filter(|object| {
            object
                .scene
                .as_ref()
                .is_none_or(|scene| scene.anchor.is_none())
        })
        .collect();
    let all_flow = !hosts.is_empty()
        && hosts.iter().all(|object| {
            object
                .layout
                .as_ref()
                .is_some_and(|layout| layout.placement == Placement::Flow)
        });
    let mut changes = Vec::new();
    let revision = state.selection_revision;
    let feedback_id = ui
        .id()
        .with(("shared_edit_error", revision, state.editor_generation));
    if all_flow {
        ui.label("Sequence");
        if let Some(value) = number(
            ui,
            revision,
            "Item width",
            &hosts
                .iter()
                .map(|object| object.frame.width_mm)
                .collect::<Vec<_>>(),
            "Different values",
        ) {
            changes.push(SharedEdit::ItemWidth(value));
        }
        if let Some(value) = number(
            ui,
            revision,
            "Gap",
            &hosts
                .iter()
                .map(|object| object.frame.gap_mm)
                .collect::<Vec<_>>(),
            "Different values",
        ) {
            changes.push(SharedEdit::Gap(value));
        }
    }
    if all_text {
        ui.separator();
        ui.label("Text format");
        let sizes: Vec<_> = objects
            .iter()
            .filter_map(|object| {
                if let ObjectContent::Text { font_size, .. } = &object.content {
                    font_size.map(f64::from)
                } else {
                    None
                }
            })
            .collect();
        let size_values = if sizes.len() == objects.len() {
            sizes
        } else {
            vec![f64::NAN]
        };
        if let Some(value) = number(
            ui,
            revision,
            "Font size",
            &size_values,
            if objects.iter().all(|object| {
                matches!(
                    &object.content,
                    ObjectContent::Text {
                        font_size: None,
                        ..
                    }
                )
            }) {
                "Automatic"
            } else {
                "Different values"
            },
        ) {
            changes.push(SharedEdit::FontSize(value as f32));
        }
        ui.horizontal(|ui| {
            for (label, field) in [("B", 0), ("I", 1), ("U", 2), ("S", 3)] {
                let values: Vec<_> = objects
                    .iter()
                    .flat_map(|object| {
                        let ObjectContent::Text {
                            content, styles, ..
                        } = &object.content
                        else {
                            return Vec::new();
                        };
                        style_regions(content, styles)
                            .into_iter()
                            .map(|(_, style)| match field {
                                0 => style.bold,
                                1 => style.italic,
                                2 => style.underline,
                                _ => style.strikethrough,
                            })
                            .collect::<Vec<_>>()
                    })
                    .collect();
                let enabled = !values.is_empty() && values.iter().all(|value| *value);
                let mixed = values.iter().any(|value| *value) && !enabled;
                let mut next = enabled;
                if ui
                    .add(egui::Checkbox::new(&mut next, label).indeterminate(mixed))
                    .changed()
                {
                    let mut patch = StylePatch::default();
                    match field {
                        0 => patch.bold = Some(!enabled),
                        1 => patch.italic = Some(!enabled),
                        2 => patch.underline = Some(!enabled),
                        _ => patch.strikethrough = Some(!enabled),
                    }
                    changes.push(SharedEdit::Style(patch));
                }
            }
        });
        let families: Vec<_> = objects
            .iter()
            .flat_map(|object| {
                let ObjectContent::Text {
                    content, styles, ..
                } = &object.content
                else {
                    return Vec::new();
                };
                style_regions(content, styles)
                    .into_iter()
                    .map(|(_, style)| style.font_name.unwrap_or_else(|| state.font_name.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        let family = families
            .first()
            .filter(|first| families.iter().all(|family| family == *first));
        egui::ComboBox::from_label("Font")
            .selected_text(family.map_or("Different values", String::as_str))
            .show_ui(ui, |ui| {
                for name in &state.available_fonts {
                    if ui.selectable_label(family == Some(name), name).clicked() {
                        changes.push(SharedEdit::Style(StylePatch {
                            font_name: Some(Some(name.clone())),
                            ..Default::default()
                        }));
                    }
                }
            });
    }
    ui.separator();
    if let Some(value) = number(
        ui,
        revision,
        "Rotation",
        &objects
            .iter()
            .map(|object| f64::from(object.rotation))
            .collect::<Vec<_>>(),
        "Different values",
    ) {
        changes.push(SharedEdit::Rotation(value as f32));
    }
    if same_type {
        ui.collapsing("Content dimensions", |ui| {
            let proportions_id = ui.id().with("keep_proportions");
            let mut proportional = ui
                .data(|data| data.get_temp::<bool>(proportions_id))
                .unwrap_or(false);
            ui.checkbox(&mut proportional, "Keep proportions");
            ui.data_mut(|data| data.insert_temp(proportions_id, proportional));
            let dimensions: Vec<_> = indices
                .iter()
                .filter_map(|index| state.resolved_content_box(*index))
                .collect();
            if dimensions.len() == indices.len() {
                if let Some(value) = number(
                    ui,
                    revision,
                    "Width",
                    &dimensions
                        .iter()
                        .map(|content| content.width_mm)
                        .collect::<Vec<_>>(),
                    "Different values",
                ) {
                    changes.push(SharedEdit::Width(value, proportional));
                }
                if let Some(value) = number(
                    ui,
                    revision,
                    "Height",
                    &dimensions
                        .iter()
                        .map(|content| content.height_mm)
                        .collect::<Vec<_>>(),
                    "Different values",
                ) {
                    changes.push(SharedEdit::Height(value, proportional));
                }
            }
        });
    }
    if objects.iter().all(|object| object.layout.is_some()) {
        let all_enabled = objects
            .iter()
            .all(|object| object.layout.as_ref().unwrap().constrain);
        let mut enabled = all_enabled;
        let mixed = objects
            .iter()
            .any(|object| object.layout.as_ref().unwrap().constrain)
            && !all_enabled;
        if ui
            .add(egui::Checkbox::new(&mut enabled, "Keep content inside").indeterminate(mixed))
            .changed()
        {
            changes.push(SharedEdit::Containment(enabled));
        }
    }
    for change in changes {
        let error = (!state.apply_shared_edit(change)).then(|| state.status_message.clone());
        ui.data_mut(|data| data.insert_temp(feedback_id, error));
    }
    if let Some(Some(message)) = ui.data(|data| data.get_temp::<Option<String>>(feedback_id)) {
        ui.colored_label(ui.visuals().error_fg_color, message);
        ui.small("No objects changed.");
    }
    apply_action(state, action);
}

fn apply_action(state: &mut AppState, action: Option<u8>) {
    match action {
        Some(0) => {
            let text: Vec<_> = state
                .selected_indices()
                .into_iter()
                .filter(|index| {
                    matches!(
                        &state.elements[*index],
                        ptouch_render::document::LabelElement::Text { .. }
                    ) || state.elements[*index]
                        .as_object()
                        .is_some_and(|object| matches!(object.content, ObjectContent::Text { .. }))
                })
                .collect();
            state.set_selected_indices(&text, text.first().copied());
        }
        Some(1) => {
            state.copy_selection();
        }
        Some(2) => {
            state.delete_selection();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(
        ctx: &egui::Context,
        values: &[f64],
        events: Vec<egui::Event>,
    ) -> (Option<f64>, Vec<(String, egui::Rect)>) {
        let mut submitted = None;
        let mut output = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(350.0, 200.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                submitted = number(ui, 1, "Width", values, "Different values");
            },
        );
        output.textures_delta.clear();
        let text = output
            .shapes
            .into_iter()
            .filter_map(|shape| match shape.shape {
                egui::Shape::Text(text) => Some((
                    text.galley.text().to_owned(),
                    text.galley.rect.translate(text.pos.to_vec2()),
                )),
                _ => None,
            })
            .collect();
        (submitted, text)
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    #[test]
    fn idle_fields_follow_linked_dimension_changes_and_undo_without_reselection() {
        let ctx = egui::Context::default();
        for value in [10.0, 8.0, 10.0, 12.0] {
            let (submitted, text) = frame(&ctx, &[value, value], Vec::new());
            assert!(submitted.is_none());
            assert!(
                text.iter().any(|(text, _)| text == &format!("{value:.3}")),
                "{text:?}"
            );
        }
        let (_, text) = frame(&ctx, &[8.0, 12.0], Vec::new());
        assert!(text.iter().any(|(text, _)| text == "Different values"));
    }

    #[test]
    fn invalid_shared_number_is_rejected_and_the_field_returns_to_the_model_value() {
        let ctx = egui::Context::default();
        let (_, text) = frame(&ctx, &[10.0], Vec::new());
        let pos = text
            .iter()
            .find(|(text, _)| text == "10.000")
            .unwrap()
            .1
            .center();
        frame(
            &ctx,
            &[10.0],
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
            &ctx,
            &[10.0],
            vec![
                key(egui::Key::A, egui::Modifiers::COMMAND),
                egui::Event::Text("not a number".into()),
            ],
        );
        let (submitted, text) = frame(
            &ctx,
            &[10.0],
            vec![key(egui::Key::Enter, egui::Modifiers::NONE)],
        );
        assert!(submitted.is_none());
        assert!(text.iter().any(|(text, _)| text == "Enter a number."));
        let (_, text) = frame(&ctx, &[10.0], Vec::new());
        assert!(text.iter().any(|(text, _)| text == "10.000"));
    }
}
