// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! A view of resolved millimeter positions. Viewing does not edit the document.

use crate::state::{AppState, CanvasEditTarget, CanvasTarget};
use egui_extras::{Column, TableBuilder};

pub fn show(ctx: &egui::Context, state: &mut AppState) {
    if !state.positions_open {
        return;
    }
    let frames = state.position_frames();
    let order = state.layout_order();
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
                    .header(22.0, |mut header| {
                        for label in ["Object", "X", "Y", "Layout width", "Layout height", "Right edge", "Gap before"] {
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
                        });
                    });
            });
        });
    state.positions_open = open;
    if let Some(element) = selected {
        state.select_canvas_target(Some(CanvasTarget {
            element,
            edit: CanvasEditTarget::Content,
        }));
    }
}
