// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! A view of resolved millimeter positions. Viewing does not edit the document.

use crate::state::AppState;
use egui_extras::{Column, TableBuilder};

fn sequence_labels(state: &AppState) -> Vec<Option<usize>> {
    let mut labels = vec![None; state.elements.len()];
    let mut explicit = std::collections::BTreeMap::new();
    let mut previous: Option<usize> = None;
    let mut count = 0;
    for (index, element) in state.elements.iter().enumerate() {
        let Some(object) = element.as_object() else {
            previous = None;
            continue;
        };
        if object
            .scene
            .as_ref()
            .is_some_and(|scene| scene.anchor.is_some())
        {
            continue;
        }
        if let Some(sequence) = object.sequence {
            labels[index] = Some(*explicit.entry(sequence).or_insert_with(|| {
                count += 1;
                count
            }));
            previous = None;
        } else {
            if object.frame.position == ptouch_render::document::PositionMode::Relative
                && let Some(prior) = previous
            {
                let group = labels[prior].unwrap_or_else(|| {
                    count += 1;
                    count
                });
                labels[prior] = Some(group);
                labels[index] = Some(group);
            } else if object.layout.as_ref().is_some_and(|layout| {
                layout.placement == ptouch_render::object_layout::Placement::Flow
            }) {
                count += 1;
                labels[index] = Some(count);
            }
            previous = Some(index);
        }
    }
    labels
}

pub fn show(ctx: &egui::Context, state: &mut AppState) {
    if !state.positions_open {
        return;
    }
    let frames = state.position_frames();
    let order = state.layout_order();
    let groups = sequence_labels(state);
    let mut open = true;
    let mut selected = None;
    egui::Window::new("Positions")
        .open(&mut open)
        .default_width(760.0)
        .default_height(360.0)
        .show(ctx, |ui| {
            ui.label("Values are in millimeters. Positions start at the upper left corner of the printable area.");
            egui::ScrollArea::horizontal().show(ui, |ui| {
                TableBuilder::new(ui)
                    .striped(true)
                    .resizable(true)
                    .column(Column::initial(180.0).at_least(80.0).clip(true))
                    .columns(Column::initial(85.0).at_least(60.0), 6)
                    .columns(Column::initial(100.0).at_least(70.0), 2)
                    .header(22.0, |mut header| {
                        for label in ["Object", "X", "Y", "Layout width", "Layout height", "Right edge", "Gap before", "Position mode", "Sequence"] {
                            header.col(|ui| { ui.strong(label); });
                        }
                    })
                    .body(|body| {
                        body.rows(22.0, order.len(), |mut row| {
                            let number = row.index();
                            let index = order[number];
                            let element = &state.elements[index];
                            row.col(|ui| {
                                if ui.selectable_label(state.selected_element == Some(index), format!("{}. {}", number + 1, element.display_name())).clicked() {
                                    selected = Some(index);
                                }
                            });
                            for value in frames[index].as_ref().map(|frame| [frame.x_mm, frame.y_mm, frame.width_mm, frame.height_mm, frame.x_mm + frame.width_mm]).unwrap_or([f64::NAN; 5]) {
                                row.col(|ui| {
                                    if value.is_finite() { ui.label(format!("{value:.3}")); } else { ui.label("Unknown"); }
                                });
                            }
                            row.col(|ui| {
                                if let Some(object) = element.as_object()
                                    && object.frame.position == ptouch_render::document::PositionMode::Relative
                                {
                                    ui.label(format!("{:.3}", object.frame.gap_mm));
                                }
                            });
                            row.col(|ui| {
                                let mode=element.as_object().map(|object| {
                                    if object.scene.as_ref().is_some_and(|scene|scene.anchor.is_some()){ "Attached" }
                                    else if object.layout.as_ref().is_some_and(|layout|layout.placement==ptouch_render::object_layout::Placement::Free){"Free"}
                                    else if object.frame.position==ptouch_render::document::PositionMode::Relative {"After previous"}else{"Absolute"}
                                }).unwrap_or("Legacy");
                                ui.label(mode);
                            });
                            row.col(|ui| {
                                if let Some(group)=groups[index] {ui.label(group.to_string());}
                            });
                        });
                    });
            });
        });
    state.positions_open = open;
    if let Some(element) = selected {
        crate::editor_navigation::set_focus(ctx, crate::editor_navigation::EditorFocus::Elements);
        crate::editor_navigation::request_canvas_reveal(ctx, element);
        state.set_selected_indices(&[element], Some(element));
    }
}
