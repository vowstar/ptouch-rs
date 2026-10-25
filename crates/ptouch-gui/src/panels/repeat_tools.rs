// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Explicit repetition and arrangement, with a preview of the item positions.

use crate::{repeat_edit::RepeatOptions, state::AppState};
use ptouch_render::document::ObjectContent;

#[derive(Clone, Default)]
struct Request {
    arrange: bool,
    selection: Vec<usize>,
}
#[derive(Clone)]
struct Form {
    arrange: bool,
    options: RepeatOptions,
    selection: Vec<usize>,
    generation: u64,
    error: Option<String>,
}
fn request_id() -> egui::Id {
    egui::Id::new("repeat_items_request")
}
fn form_id() -> egui::Id {
    egui::Id::new("repeat_items_form")
}

pub fn is_open(ctx: &egui::Context) -> bool {
    ctx.data_mut(|data| data.get_temp::<Form>(form_id()))
        .is_some()
}

pub fn buttons(ui: &mut egui::Ui, state: &AppState) {
    let selected = state.selected_indices();
    let hosts = selected
        .iter()
        .filter(|index| {
            state.elements[**index].as_object().is_some_and(|object| {
                object
                    .scene
                    .as_ref()
                    .is_none_or(|scene| scene.anchor.is_none())
            })
        })
        .count();
    if !state.is_free_layout() || selected.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        if ui
            .add_enabled(
                hosts == 1 && selected.len() == 1,
                egui::Button::new("Repeat"),
            )
            .on_hover_text("Create more items from the selected object.")
            .clicked()
        {
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    request_id(),
                    Request {
                        arrange: false,
                        selection: selected.clone(),
                    },
                )
            });
        }
        if ui
            .add_enabled(hosts > 0, egui::Button::new("Arrange selected"))
            .on_hover_text("Set item positions and gaps.")
            .clicked()
        {
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    request_id(),
                    Request {
                        arrange: true,
                        selection: selected.clone(),
                    },
                )
            });
        }
    });
}

pub fn show(ctx: &egui::Context, state: &mut AppState) {
    if let Some(request) = ctx.data_mut(|data| data.remove_temp::<Request>(request_id()))
        && request.selection == state.selected_indices()
    {
        let first = state.layout_order().into_iter().find(|index| {
            request.selection.contains(index)
                && state.elements[*index].as_object().is_some_and(|object| {
                    object
                        .scene
                        .as_ref()
                        .is_none_or(|scene| scene.anchor.is_none())
                })
        });
        if let Some(first) = first {
            let frames = state.position_frames();
            let mut options = RepeatOptions::default();
            if let Some(frame) = &frames[first] {
                options.start_x_mm = frame.x_mm;
                options.item_width_mm = frame.width_mm;
            }
            if request.arrange {
                options.total_items = request
                    .selection
                    .iter()
                    .filter(|index| {
                        state.elements[**index].as_object().is_some_and(|object| {
                            object
                                .scene
                                .as_ref()
                                .is_none_or(|scene| scene.anchor.is_none())
                        })
                    })
                    .count();
            }
            ctx.data_mut(|data| {
                data.insert_temp(
                    form_id(),
                    Form {
                        arrange: request.arrange,
                        options,
                        selection: request.selection,
                        generation: state.editor_generation,
                        error: None,
                    },
                )
            });
        }
    }
    let Some(mut form) = ctx.data_mut(|data| data.get_temp::<Form>(form_id())) else {
        return;
    };
    let mut apply = false;
    let mut close = false;
    let response = egui::Modal::new(form_id()).show(ctx, |ui| {
        ui.set_max_width(460.0);
        ui.heading(if form.arrange {
            "Arrange selected"
        } else {
            "Repeat items"
        });
        egui::ScrollArea::vertical()
            .max_height((ctx.content_rect().height() - 150.0).max(180.0))
            .show(ui, |ui| {
                if !form.arrange {
                    ui.horizontal(|ui| {
                        ui.label("Total items");
                        ui.add(egui::DragValue::new(&mut form.options.total_items).range(1..=4096));
                    });
                    ui.small("The total includes the selected item.");
                } else {
                    ui.label(format!("{} selected items", form.options.total_items));
                }
                for (label, value) in [
                    ("Start X", &mut form.options.start_x_mm),
                    ("Item width", &mut form.options.item_width_mm),
                    ("Gap", &mut form.options.gap_mm),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(label);
                        ui.add(
                            crate::widgets::decimal_drag(value)
                                .range(0.0..=10_000.0)
                                .speed(0.1)
                                .suffix(" mm"),
                        );
                    });
                }
                if !form.arrange {
                    ui.checkbox(&mut form.options.number_text, "Replace text with numbers");
                    if form.options.number_text {
                        ui.small("The selected item becomes 1.");
                    }
                }
                ui.checkbox(&mut form.options.cut_lines, "Print cut lines between items");
                if form.options.cut_lines {
                    let first = state.layout_order().into_iter().find(|index| {
                        form.selection.contains(index)
                            && state.elements[*index].as_object().is_some_and(|object| {
                                object
                                    .scene
                                    .as_ref()
                                    .is_none_or(|scene| scene.anchor.is_none())
                            })
                    });
                    let lines: Vec<_> = first
                        .map(|host| ptouch_render::scene::children(&state.elements, host))
                        .unwrap_or_default()
                        .into_iter()
                        .filter_map(|index| {
                            let object = state.elements[index].as_object()?;
                            matches!(object.content, ObjectContent::Line { .. }).then_some((
                                object.scene.as_ref()?.id,
                                state.elements[index].display_name(),
                            ))
                        })
                        .collect();
                    let current = form
                        .options
                        .existing_line
                        .and_then(|id| lines.iter().find(|(line, _)| *line == id))
                        .map_or("New dotted line", |(_, name)| name.as_str());
                    egui::ComboBox::from_label("Cut line")
                        .selected_text(current)
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut form.options.existing_line,
                                None,
                                "New dotted line",
                            );
                            for (id, name) in &lines {
                                ui.selectable_value(
                                    &mut form.options.existing_line,
                                    Some(*id),
                                    format!("Use {name}"),
                                );
                            }
                        });
                    ui.checkbox(&mut form.options.end_line, "End line")
                        .on_hover_text("Add a cut line after the last item.");
                }
                ui.separator();
                let count = form.options.total_items as f64;
                let last = form.options.start_x_mm
                    + (count - 1.0) * (form.options.item_width_mm + form.options.gap_mm);
                ui.label(format!("Last item X: {last:.3} mm"));
                ui.label(format!(
                    "Item span: {:.3} mm",
                    count * form.options.item_width_mm + (count - 1.0) * form.options.gap_mm
                ));
                if let Some(error) = &form.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            });
        ui.horizontal(|ui| {
            apply = ui.button("Apply").clicked();
            close = ui.button("Cancel").clicked();
        });
    });
    if apply {
        if state.editor_generation != form.generation || state.selected_indices() != form.selection
        {
            form.error = Some("Select the objects again before you apply this layout.".into());
        } else {
            let result = if form.arrange {
                state.arrange_selected(&form.options)
            } else {
                state.repeat_items(&form.options)
            };
            match result {
                Ok(_) => close = true,
                Err(error) => form.error = Some(error),
            }
        }
    }
    if close || response.should_close() {
        ctx.data_mut(|data| data.remove::<Form>(form_id()));
    } else {
        ctx.data_mut(|data| data.insert_temp(form_id(), form));
    }
}
