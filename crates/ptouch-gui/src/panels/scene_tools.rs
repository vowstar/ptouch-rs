// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Drawing order and explicit attachments, sharing the normal object selection.

use ptouch_render::scene::Anchor;
use ptouch_render::shape::ShapeKind;

use crate::state::{AppState, CanvasEditTarget, CanvasTarget, LayerMove};

#[derive(Clone, Copy)]
struct LayerDrag(usize);

enum LayerAction {
    Arrange(usize, LayerMove),
    Reorder(usize, usize),
}

/// Drawing order is independent of the physical sequence shown in Elements.
pub fn show_layers(ui: &mut egui::Ui, state: &mut AppState) {
    ui.small("Front to back. Drag rows to change stacking.");
    ui.add_space(4.0);
    let order: Vec<_> = ptouch_render::scene::paint_order(&state.elements)
        .into_iter()
        .rev()
        .collect();
    if order.is_empty() {
        ui.weak("(no elements)");
        return;
    }
    crate::editor_navigation::list_keyboard(ui, state, &order);
    let can_arrange = state.is_free_layout();
    let mut selection = None;
    let mut action = None;
    for (position, &index) in order.iter().enumerate() {
        let name = state.elements[index].display_name();
        let response = ui
            .horizontal(|ui| {
                ui.add_sized(
                    [ui.available_width().max(20.0), ui.spacing().interact_size.y],
                    egui::Button::selectable(
                        state.selected_element == Some(index),
                        (&name, egui::Atom::grow()),
                    )
                    .truncate()
                    .sense(if can_arrange {
                        egui::Sense::click_and_drag()
                    } else {
                        egui::Sense::click()
                    }),
                )
            })
            .inner
            .on_hover_text(format!(
                "{name}\nLayout element {}. Stacking changes keep X/Y and sequence order.",
                index + 1
            ));
        crate::editor_navigation::reveal_selected_row(ui, &response, state, index);
        if response.clicked() || response.drag_started() {
            selection = Some(index);
        }
        if !can_arrange {
            continue;
        }
        response.dnd_set_drag_payload(LayerDrag(index));
        if let Some(source) = response.dnd_hover_payload::<LayerDrag>()
            && source.0 != index
            && let Some(source_position) = order.iter().position(|&item| item == source.0)
        {
            let y = if source_position < position {
                response.rect.bottom()
            } else {
                response.rect.top()
            };
            let color = ui.visuals().selection.stroke.color;
            ui.painter().line_segment(
                [
                    egui::pos2(response.rect.left(), y),
                    egui::pos2(response.rect.right(), y),
                ],
                egui::Stroke::new(2.0, color),
            );
            let text = ui.painter().layout_no_wrap(
                "Change stacking · keep positions".into(),
                egui::FontId::proportional(11.0),
                color,
            );
            let text_position = egui::pos2(
                response.rect.right() - text.size().x - 2.0,
                y - text.size().y - 2.0,
            );
            ui.painter().rect_filled(
                egui::Rect::from_min_size(text_position, text.size() + egui::vec2(2.0, 2.0)),
                2.0,
                ui.visuals().panel_fill,
            );
            ui.painter().galley(text_position, text, color);
        }
        if let Some(source) = response.dnd_release_payload::<LayerDrag>()
            && source.0 != index
        {
            action = Some(LayerAction::Reorder(source.0, index));
        }
    }
    if let Some(element) = selection {
        state.select_canvas_target(Some(CanvasTarget {
            element,
            edit: CanvasEditTarget::Content,
        }));
        crate::editor_navigation::set_focus(
            ui.ctx(),
            crate::editor_navigation::EditorFocus::Elements,
        );
        crate::editor_navigation::request_canvas_reveal(ui.ctx(), element);
    }
    if let Some(LayerAction::Reorder(from, to)) = action {
        state.reorder_layer(from, to);
    }
}

pub fn show_layer_actions(ui: &mut egui::Ui, state: &mut AppState) {
    let order: Vec<_> = ptouch_render::scene::paint_order(&state.elements)
        .into_iter()
        .rev()
        .collect();
    let can_arrange = state.is_free_layout();
    let mut action = None;
    ui.add_space(4.0);
    let selected = state.selected_element.and_then(|index| {
        order
            .iter()
            .position(|&item| item == index)
            .map(|p| (index, p))
    });
    ui.horizontal_wrapped(|ui| {
        for (label, movement, available) in [
            (
                "Front",
                LayerMove::Front,
                selected.is_some_and(|(_, p)| p > 0),
            ),
            (
                "Forward",
                LayerMove::Forward,
                selected.is_some_and(|(_, p)| p > 0),
            ),
            (
                "Backward",
                LayerMove::Backward,
                selected.is_some_and(|(_, p)| p + 1 < order.len()),
            ),
            (
                "Back",
                LayerMove::Back,
                selected.is_some_and(|(_, p)| p + 1 < order.len()),
            ),
        ] {
            if ui
                .add_enabled(can_arrange && available, egui::Button::new(label))
                .on_hover_text("Change drawing order without moving objects")
                .clicked()
                && let Some((index, _)) = selected
            {
                action = Some(LayerAction::Arrange(index, movement));
            }
        }
    });
    match action {
        Some(LayerAction::Arrange(index, movement)) => {
            state.change_layer(index, movement);
        }
        Some(LayerAction::Reorder(from, to)) => {
            state.reorder_layer(from, to);
        }
        None => {}
    }
}

enum AttachmentAction {
    AddText,
    AddImage,
    AddShape(ShapeKind, bool),
    Attach(usize),
    Detach,
    Align(f64, f64),
    SetAnchor(Anchor),
}

/// Apply document actions before the caller clones the selected object.
pub fn show_attachment(ui: &mut egui::Ui, state: &mut AppState) -> bool {
    if !state.is_free_layout() {
        return false;
    }
    let Some(index) = state.selected_element else {
        return false;
    };
    let Some(object) = state
        .elements
        .get(index)
        .and_then(|element| element.as_object())
    else {
        return false;
    };
    let identity = object.scene.as_ref().map_or(index as u64, |scene| scene.id);
    let anchor = object.scene.as_ref().and_then(|scene| scene.anchor.clone());
    let host_name = anchor.as_ref().and_then(|anchor| {
        state.elements.iter().find_map(|element| {
            let scene = element.as_object()?.scene.as_ref()?;
            (scene.id == anchor.target).then(|| element.display_name())
        })
    });
    let targets: Vec<_> = if anchor.is_none() {
        state
            .elements
            .iter()
            .enumerate()
            .filter(|(host, _)| state.can_attach_overlay(index, *host))
            .map(|(host, element)| (host, element.display_name()))
            .collect()
    } else {
        Vec::new()
    };
    let mut action = None;
    egui::CollapsingHeader::new("Attachment")
        .id_salt(("scene_attachment", identity, anchor.is_some()))
        .default_open(anchor.is_some())
        .show(ui, |ui| {
            if let Some(mut anchor) = anchor {
                ui.label(format!(
                    "Attached to: {}",
                    host_name.as_deref().unwrap_or("Missing object")
                ));
                if ui
                    .button("Detach")
                    .on_hover_text("Keep this object's current position")
                    .clicked()
                {
                    action = Some(AttachmentAction::Detach);
                    return;
                }
                ui.small("Tracks the layout box. Size and rotation stay independent.");
                ui.add_space(4.0);
                ui.label("Anchor");
                egui::Grid::new(("overlay_alignment", index))
                    .spacing(egui::vec2(2.0, 2.0))
                    .show(ui, |ui| {
                        for (row, symbols) in [["↖", "↑", "↗"], ["←", "•", "→"], ["↙", "↓", "↘"]]
                            .iter()
                            .enumerate()
                        {
                            for (column, symbol) in symbols.iter().enumerate() {
                                let horizontal = column as f64 / 2.0;
                                let vertical = row as f64 / 2.0;
                                let selected = (anchor.horizontal - horizontal).abs() < 1e-7
                                    && (anchor.vertical - vertical).abs() < 1e-7;
                                if ui
                                    .add_sized(
                                        [24.0, 24.0],
                                        egui::Button::selectable(selected, *symbol),
                                    )
                                    .on_hover_text(
                                        "Choose the frame anchor without moving the object",
                                    )
                                    .clicked()
                                    && !selected
                                {
                                    action = Some(AttachmentAction::Align(horizontal, vertical));
                                }
                            }
                            ui.end_row();
                        }
                    });
                if action.is_some() {
                    return;
                }
                let mut changed = false;
                egui::Grid::new(("overlay_offsets", index))
                    .num_columns(2)
                    .show(ui, |ui| {
                        for (label, value) in [
                            ("Offset X", &mut anchor.offset_x_mm),
                            ("Offset Y", &mut anchor.offset_y_mm),
                        ] {
                            ui.label(label);
                            changed |= ui
                                .add(
                                    crate::widgets::decimal_drag(value)
                                        .speed(0.1)
                                        .clamp_existing_to_range(false)
                                        .range(-10_000.0..=10_000.0)
                                        .min_decimals(2)
                                        .max_decimals(3)
                                        .suffix(" mm"),
                                )
                                .changed();
                            ui.end_row();
                        }
                    });
                if changed {
                    action = Some(AttachmentAction::SetAnchor(anchor));
                    return;
                }
                if ui
                    .add_enabled(
                        anchor.offset_x_mm != 0.0 || anchor.offset_y_mm != 0.0,
                        egui::Button::new("Align to anchor"),
                    )
                    .on_hover_text("Set both offsets to zero")
                    .clicked()
                {
                    anchor.offset_x_mm = 0.0;
                    anchor.offset_y_mm = 0.0;
                    action = Some(AttachmentAction::SetAnchor(anchor));
                }
            } else {
                ui.menu_button("Add overlay", |ui| {
                    for (label, next) in [
                        ("Text", AttachmentAction::AddText),
                        ("Image…", AttachmentAction::AddImage),
                        (
                            "Rectangle",
                            AttachmentAction::AddShape(ShapeKind::Rectangle, false),
                        ),
                        (
                            "Ellipse",
                            AttachmentAction::AddShape(ShapeKind::Ellipse, false),
                        ),
                        (
                            "Circle",
                            AttachmentAction::AddShape(ShapeKind::Ellipse, true),
                        ),
                    ] {
                        if ui.button(label).clicked() {
                            action = Some(next);
                            ui.close();
                            return;
                        }
                    }
                });
                if action.is_some() {
                    return;
                }
                ui.add_enabled_ui(!targets.is_empty(), |ui| {
                    ui.menu_button("Attach to…", |ui| {
                        for (host, name) in &targets {
                            if ui
                                .button(name)
                                .on_hover_text(format!("Layout element {}", host + 1))
                                .clicked()
                            {
                                action = Some(AttachmentAction::Attach(*host));
                                ui.close();
                                return;
                            }
                        }
                    })
                    .response
                    .on_hover_text("Keep this object's position and follow another layout box")
                    .on_disabled_hover_text(
                        "Choose an object without overlays and another unattached target",
                    );
                });
            }
        });
    match action {
        Some(AttachmentAction::AddText) => state.add_overlay_text(index),
        Some(AttachmentAction::AddImage) => {
            let Some(path) = crate::widgets::image_file_dialog().pick_file() else {
                return false;
            };
            match std::fs::read(&path) {
                Ok(bytes) => state.add_overlay_image_bytes(index, Some(path), bytes),
                Err(error) => {
                    state.status_message = format!("Image read error: {error}");
                    false
                }
            }
        }
        Some(AttachmentAction::AddShape(kind, circle)) => {
            state.add_overlay_shape(index, kind, circle)
        }
        Some(AttachmentAction::Attach(host)) => state.attach_overlay(index, host),
        Some(AttachmentAction::Detach) => state.detach_overlay(index),
        Some(AttachmentAction::Align(horizontal, vertical)) => {
            state.set_overlay_alignment(index, horizontal, vertical)
        }
        Some(AttachmentAction::SetAnchor(anchor)) => state.set_overlay_anchor(index, anchor),
        None => false,
    }
}
