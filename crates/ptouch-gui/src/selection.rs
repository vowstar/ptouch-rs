// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Selection is editor state. Viewing an old file does not create scene IDs.

use std::collections::BTreeSet;

use ptouch_render::document::LabelElement;

use crate::state::{AppState, CanvasEditTarget, CanvasTarget};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Scene(u64),
    Temporary { generation: u64, index: usize },
}

fn key(elements: &[LabelElement], generation: u64, index: usize) -> Option<Key> {
    let element = elements.get(index)?;
    Some(
        element
            .as_object()
            .and_then(|object| object.scene.as_ref())
            .map_or(Key::Temporary { generation, index }, |scene| {
                Key::Scene(scene.id)
            }),
    )
}

#[derive(Clone, Debug, Default)]
pub struct Selection {
    members: BTreeSet<Key>,
    anchor: Option<Key>,
}

impl Selection {
    pub fn clear(&mut self) {
        self.members.clear();
        self.anchor = None;
    }
}

impl AppState {
    pub fn selected_indices(&self) -> Vec<usize> {
        let Some(primary) = self
            .selected_element
            .filter(|index| *index < self.elements.len())
        else {
            return Vec::new();
        };
        let indices: Vec<_> = (0..self.elements.len())
            .filter(|index| {
                key(&self.elements, self.editor_generation, *index)
                    .is_some_and(|key| self.selection.members.contains(&key))
            })
            .collect();
        if indices.contains(&primary) {
            indices
        } else {
            vec![primary]
        }
    }

    pub fn set_selected_indices(&mut self, indices: &[usize], primary: Option<usize>) {
        let previous = self.selected_indices();
        let indices: Vec<_> = indices
            .iter()
            .copied()
            .filter(|index| *index < self.elements.len())
            .collect();
        let primary = primary
            .filter(|index| indices.contains(index))
            .or_else(|| indices.last().copied());
        self.select_canvas_target(primary.map(|element| CanvasTarget {
            element,
            edit: CanvasEditTarget::Content,
        }));
        self.selection.members = indices
            .iter()
            .filter_map(|index| key(&self.elements, self.editor_generation, *index))
            .collect();
        if previous != self.selected_indices() {
            self.selection_revision = self.selection_revision.wrapping_add(1);
        }
    }

    pub fn select_list_item(&mut self, index: usize, modifiers: egui::Modifiers, order: &[usize]) {
        if !order.contains(&index) {
            return;
        }
        let old_anchor = self.selection.anchor;
        let current = self.selected_indices();
        let mut selected = if modifiers.command || modifiers.ctrl {
            current.clone()
        } else {
            Vec::new()
        };
        if modifiers.shift {
            let anchor = order
                .iter()
                .copied()
                .find(|item| key(&self.elements, self.editor_generation, *item) == old_anchor)
                .or(self.selected_element)
                .unwrap_or(index);
            let start = order.iter().position(|item| *item == anchor).unwrap_or(0);
            let end = order.iter().position(|item| *item == index).unwrap();
            selected.extend_from_slice(&order[start.min(end)..=start.max(end)]);
        } else if modifiers.command || modifiers.ctrl {
            if let Some(position) = selected.iter().position(|item| *item == index) {
                selected.remove(position);
            } else {
                selected.push(index);
            }
        } else {
            selected.push(index);
        }
        selected.sort_unstable();
        selected.dedup();
        self.set_selected_indices(&selected, Some(index));
        self.selection.anchor = if modifiers.shift {
            old_anchor.or_else(|| {
                current
                    .first()
                    .and_then(|index| key(&self.elements, self.editor_generation, *index))
            })
        } else {
            key(&self.elements, self.editor_generation, index)
        };
    }

    pub fn toggle_canvas_item(&mut self, index: usize) {
        let mut indices = self.selected_indices();
        if let Some(position) = indices.iter().position(|item| *item == index) {
            indices.remove(position);
        } else {
            indices.push(index);
        }
        self.set_selected_indices(&indices, Some(index));
        self.selection.anchor = key(&self.elements, self.editor_generation, index);
    }

    pub fn select_all_objects(&mut self) {
        let indices: Vec<_> = (0..self.elements.len()).collect();
        self.set_selected_indices(&indices, self.selected_element);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ptouch_render::document::{LabelObject, TextBox};

    fn state() -> AppState {
        AppState {
            elements: (0..6)
                .map(|i| {
                    LabelElement::Object(LabelObject::text(format!("Port {i}"), TextBox::default()))
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn range_and_toggle_follow_the_visible_order_without_editing_the_file() {
        let mut state = state();
        let document = state.document();
        let revision = state.edit_revision;
        let order = [5, 3, 1, 4, 2, 0];
        state.select_list_item(3, egui::Modifiers::NONE, &order);
        state.select_list_item(4, egui::Modifiers::SHIFT, &order);
        assert_eq!(state.selected_indices(), vec![1, 3, 4]);
        state.select_list_item(1, egui::Modifiers::CTRL, &order);
        assert_eq!(state.selected_indices(), vec![3, 4]);
        state.toggle_canvas_item(2);
        assert_eq!(state.selected_indices(), vec![2, 3, 4]);
        assert_eq!(state.document(), document);
        assert_eq!(state.edit_revision, revision);
    }

    #[test]
    fn old_temporary_keys_cannot_select_different_objects_after_a_structural_change() {
        let mut state = state();
        state.set_selected_indices(&[0, 2, 4], Some(2));
        state.elements.swap(0, 1);
        state.editor_generation += 1;
        assert_eq!(state.selected_indices(), vec![2]);
    }

    #[test]
    fn scene_identity_survives_reordering_and_plain_selection_can_replace_the_set() {
        let mut state = state();
        ptouch_render::scene::enable(&mut state.elements).unwrap();
        state.set_selected_indices(&[0, 2, 4], Some(2));
        state.elements.swap(0, 1);
        state.editor_generation += 1;
        assert_eq!(state.selected_indices(), vec![1, 2, 4]);
        state.select_list_item(2, egui::Modifiers::NONE, &[0, 1, 2, 3, 4, 5]);
        assert_eq!(state.selected_indices(), vec![2]);
        state.select_canvas_target(None);
        assert!(state.selected_indices().is_empty());
    }
}
