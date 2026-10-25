// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Editor focus and selection visibility do not change the label document.

use crate::state::{AppState, CanvasEditTarget, CanvasTarget};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum EditorFocus {
    #[default]
    Canvas,
    Elements,
}

pub fn focus(ctx: &egui::Context) -> EditorFocus {
    ctx.data_mut(|data| data.get_temp(egui::Id::new("editor_focus")))
        .unwrap_or_default()
}

pub fn set_focus(ctx: &egui::Context, target: EditorFocus) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("editor_focus"), target));
}

pub fn request_canvas_reveal(ctx: &egui::Context, index: usize) {
    ctx.data_mut(|data| data.insert_temp(egui::Id::new("canvas_reveal"), index));
    ctx.request_repaint();
}

pub fn take_canvas_reveal(ctx: &egui::Context) -> Option<usize> {
    ctx.data_mut(|data| data.remove_temp(egui::Id::new("canvas_reveal")))
}

/// Navigate in the current visible order. Consume the key before the canvas.
pub fn list_keyboard(ui: &egui::Ui, state: &mut AppState, order: &[usize]) {
    if focus(ui.ctx()) != EditorFocus::Elements
        || ui.ctx().text_edit_focused()
        || egui::Popup::is_any_open(ui.ctx())
        || state.pending_file_action.is_some()
        || order.is_empty()
    {
        return;
    }
    let movement = ui.input_mut(|input| {
        if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown) {
            Some(true)
        } else if input.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp) {
            Some(false)
        } else {
            None
        }
    });
    let Some(forward) = movement else { return };
    let current = state
        .selected_element
        .and_then(|selected| order.iter().position(|index| *index == selected));
    let position = match (current, forward) {
        (Some(position), true) => (position + 1).min(order.len() - 1),
        (Some(position), false) => position.saturating_sub(1),
        (None, true) => 0,
        (None, false) => order.len() - 1,
    };
    let index = order[position];
    if state.selected_element != Some(index) {
        state.select_canvas_target(Some(CanvasTarget {
            element: index,
            edit: CanvasEditTarget::Content,
        }));
        request_canvas_reveal(ui.ctx(), index);
    }
}

/// A selection change reveals its row once. Manual scrolling stays in place.
pub fn reveal_selected_row(
    ui: &egui::Ui,
    response: &egui::Response,
    state: &AppState,
    index: usize,
) {
    if state.selected_element != Some(index) {
        return;
    }
    let key = egui::Id::new(("revealed_selection", state.layers_view));
    let size = ui.clip_rect().size();
    let revision = (
        state.selection_revision,
        index,
        size.x.to_bits(),
        size.y.to_bits(),
    );
    let reveal = response.ctx.data_mut(|data| {
        if data.get_temp::<(u64, usize, u32, u32)>(key) == Some(revision) {
            false
        } else {
            data.insert_temp(key, revision);
            true
        }
    });
    let viewport = ui.clip_rect();
    if reveal
        && (response.rect.top() < viewport.top() || response.rect.bottom() > viewport.bottom())
    {
        response.scroll_to_me(None);
    }
}

pub fn scroll_drag_near_edge<T>(ui: &egui::Ui, scroll: &egui::scroll_area::ScrollAreaOutput<T>) {
    if !egui::DragAndDrop::has_any_payload(ui.ctx()) || !ui.input(|input| input.pointer.any_down())
    {
        return;
    }
    let Some(pointer) = ui.input(|input| input.pointer.hover_pos()) else {
        return;
    };
    if !scroll.inner_rect.contains(pointer) {
        return;
    }
    let direction = if pointer.y < scroll.inner_rect.top() + 24.0 {
        -1.0
    } else if pointer.y > scroll.inner_rect.bottom() - 24.0 {
        1.0
    } else {
        return;
    };
    let mut next = scroll.state;
    let distance = ui.input(|input| input.stable_dt).clamp(0.001, 0.05) * 400.0;
    next.offset.y = (next.offset.y + direction * distance).clamp(
        0.0,
        (scroll.content_size.y - scroll.inner_rect.height()).max(0.0),
    );
    next.store(ui.ctx(), scroll.id);
    ui.ctx().request_repaint();
}

#[cfg(test)]
mod tests {
    use super::*;
    use ptouch_render::document::{LabelElement, LabelObject, TextBox};

    fn frame(ctx: &egui::Context, state: &mut AppState, events: Vec<egui::Event>, order: &[usize]) {
        let mut output = ctx.run_ui(
            egui::RawInput {
                events,
                ..Default::default()
            },
            |ui| list_keyboard(ui, state, order),
        );
        output.textures_delta.clear();
    }

    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }
    }

    #[test]
    fn list_keys_follow_visible_order_without_editing_or_wraparound() {
        let ctx = egui::Context::default();
        let mut state = AppState {
            elements: (0..3)
                .map(|i| LabelElement::Object(LabelObject::text(i.to_string(), TextBox::default())))
                .collect(),
            ..Default::default()
        };
        let original = state.document();
        let revision = state.edit_revision;
        set_focus(&ctx, EditorFocus::Elements);
        let order = [2, 0, 1];
        frame(&ctx, &mut state, vec![key(egui::Key::ArrowDown)], &order);
        assert_eq!(state.selected_element, Some(2));
        frame(&ctx, &mut state, vec![key(egui::Key::ArrowDown)], &order);
        assert_eq!(state.selected_element, Some(0));
        frame(&ctx, &mut state, vec![key(egui::Key::ArrowDown)], &order);
        frame(&ctx, &mut state, vec![key(egui::Key::ArrowDown)], &order);
        assert_eq!(state.selected_element, Some(1));
        frame(&ctx, &mut state, vec![key(egui::Key::ArrowUp)], &order);
        assert_eq!(state.selected_element, Some(0));
        assert_eq!(state.document(), original);
        assert_eq!(state.edit_revision, revision);
        assert_eq!(take_canvas_reveal(&ctx), Some(0));
    }

    #[test]
    fn text_input_keeps_its_arrow_keys_when_the_list_was_last_selected() {
        let ctx = egui::Context::default();
        let mut state = AppState::default();
        state.add_text();
        state.add_text();
        let selected = state.selected_element;
        set_focus(&ctx, EditorFocus::Elements);
        let mut text = "Editing text".to_string();
        for step in 0..3 {
            let mut output = ctx.run_ui(
                egui::RawInput {
                    events: if step == 2 {
                        vec![key(egui::Key::ArrowUp)]
                    } else {
                        vec![]
                    },
                    ..Default::default()
                },
                |ui| {
                    ui.text_edit_singleline(&mut text).request_focus();
                    list_keyboard(ui, &mut state, &[0, 1]);
                },
            );
            output.textures_delta.clear();
        }
        assert_eq!(state.selected_element, selected);
    }

    #[test]
    fn canvas_focus_does_not_navigate_the_list() {
        let ctx = egui::Context::default();
        let mut state = AppState::default();
        state.add_text();
        state.add_text();
        let selected = state.selected_element;
        frame(&ctx, &mut state, vec![key(egui::Key::ArrowUp)], &[0, 1]);
        assert_eq!(state.selected_element, selected);
        assert_eq!(take_canvas_reveal(&ctx), None);
    }
}

#[cfg(test)]
mod visible_row_tests {
    use super::*;
    use crate::state::{CanvasEditTarget, CanvasTarget};
    use ptouch_render::document::{LabelElement, LabelObject, TextBox};
    #[test]
    fn selecting_a_fully_visible_row_keeps_the_list_stationary() {
        let ctx = egui::Context::default();
        let mut state = AppState {
            elements: (0..30)
                .map(|i| LabelElement::Object(LabelObject::text(i.to_string(), TextBox::default())))
                .collect(),
            ..Default::default()
        };
        let mut offsets = Vec::new();
        for step in 0..6 {
            if step == 2 {
                state.select_canvas_target(Some(CanvasTarget {
                    element: 3,
                    edit: CanvasEditTarget::Content,
                }));
            }
            let mut offset = 0.0;
            let mut output = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(250.0, 120.0),
                    )),
                    time: Some(step as f64 * 0.2),
                    ..Default::default()
                },
                |ui| {
                    let scroll = egui::ScrollArea::vertical().show(ui, |ui| {
                        for index in 0..30 {
                            let response = ui.selectable_label(
                                state.selected_element == Some(index),
                                format!("Item {index}"),
                            );
                            reveal_selected_row(ui, &response, &state, index);
                        }
                    });
                    offset = scroll.state.offset.y;
                },
            );
            output.textures_delta.clear();
            offsets.push(offset);
        }
        assert!(
            offsets.iter().all(|offset| offset.abs() < 0.01),
            "{offsets:?}"
        );
    }
}
