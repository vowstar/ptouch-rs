// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Batch operations preserve scene identity, geometry and complete selection.

use std::collections::{BTreeMap, BTreeSet};

use ptouch_render::{
    document::{
        ContentBox, LabelDocument, LabelElement, LabelObject, ObjectContent, PositionMode, TextBox,
    },
    object_layout::{self, LayoutOptions, Placement, WidthSizing},
    scene::{self, Anchor, GapCenter, SceneOptions},
    shape::{ShapeFill, ShapeStyle},
    styled_text::{StylePatch, StyleSpan, StyledText, TextStyle},
};

use crate::{batch_edit::SharedEdit, state::AppState};

fn id(element: &LabelElement) -> u64 {
    element.as_object().unwrap().scene.as_ref().unwrap().id
}

fn index(state: &AppState, wanted: u64) -> usize {
    state
        .elements
        .iter()
        .position(|element| id(element) == wanted)
        .unwrap()
}

fn host(id: u64, sequence: Option<u64>, x: f64, relative: bool, height: f64) -> LabelObject {
    let mut object = LabelObject::text(
        format!("Port {id}"),
        TextBox {
            x_mm: x,
            width_mm: 12.0,
            height_mm: height,
            position: if relative {
                PositionMode::Relative
            } else {
                PositionMode::Absolute
            },
            gap_mm: 2.0,
            ..Default::default()
        },
    );
    object.sequence = sequence;
    object.content_box = Some(ContentBox {
        width_mm: 8.0,
        height_mm: 3.0,
        offset_x_mm: 0.0,
        offset_y_mm: 0.0,
    });
    object.layout = Some(LayoutOptions {
        placement: if sequence.is_some() {
            Placement::Flow
        } else {
            Placement::Free
        },
        width: WidthSizing::Fixed,
        constrain: false,
        padding_mm: 0.0,
    });
    if let ObjectContent::Text { font_size, .. } = &mut object.content {
        *font_size = Some(6.0);
    }
    object.scene = Some(Box::new(SceneOptions {
        id,
        layer: id as i32,
        anchor: None,
    }));
    if sequence.is_none() {
        object.frame.y_mm = 2.0;
    }
    object_layout::update_allocation(&mut object);
    object
}

fn child(id: u64, target: u64, line: bool) -> LabelObject {
    let mut object = host(id, None, 0.0, false, 3.0);
    let dimensions = object.content_box.as_mut().unwrap();
    dimensions.width_mm = if line { 8.0 } else { 1.0 };
    dimensions.height_mm = if line { 0.3 } else { 1.0 };
    object.rotation = if line { 90.0 } else { 0.0 };
    object.content = if line {
        ObjectContent::Line {
            pattern: Default::default(),
        }
    } else {
        ObjectContent::Shape {
            style: ShapeStyle::default(),
        }
    };
    object.scene.as_mut().unwrap().anchor = Some(Anchor {
        target,
        horizontal: if line { 1.0 } else { 0.5 },
        vertical: 0.5,
        offset_x_mm: if line { 1.0 } else { 0.0 },
        offset_y_mm: 0.0,
        gap_center: None,
    });
    object_layout::update_allocation(&mut object);
    object
}

fn fixture() -> AppState {
    let mut state = AppState {
        document_version: 13,
        design_dpi: 180,
        ..Default::default()
    };
    state.update_tape_pixels();
    let height = state.printable_height_mm();
    state.elements = vec![
        host(1, Some(20), 5.0, false, height),
        child(2, 1, true),
        host(3, Some(10), 55.0, false, height),
        child(4, 3, true),
        host(5, Some(20), 0.0, true, height),
        child(6, 5, true),
        host(7, Some(10), 0.0, true, height),
        host(8, None, 100.0, false, height),
        child(9, 8, false),
        host(10, None, 120.0, false, height),
        host(11, Some(20), 0.0, true, height),
    ]
    .into_iter()
    .map(LabelElement::Object)
    .collect();
    state.validate_scene_geometry().unwrap();
    state.set_selected_indices(&[0], Some(0));
    state.needs_rerender = false;
    state
}

fn frames(state: &AppState) -> BTreeMap<u64, TextBox> {
    state
        .elements
        .iter()
        .zip(scene::world_frames(&state.elements).unwrap())
        .map(|(element, frame)| (id(element), frame))
        .collect()
}

#[track_caller]
fn assert_frame(actual: &TextBox, expected: &TextBox) {
    for (actual, expected) in [
        (actual.x_mm, expected.x_mm),
        (actual.y_mm, expected.y_mm),
        (actual.width_mm, expected.width_mm),
        (actual.height_mm, expected.height_mm),
    ] {
        assert!(
            (actual - expected).abs() < 1e-8,
            "{actual} differs from {expected}"
        );
    }
}

fn observe(state: &mut AppState, time: f64) {
    let document = state.document();
    let selection = state.selected_indices();
    state
        .history
        .observe(document, state.selected_element, false, false, time);
    state.history.set_selection_set(&selection);
}

fn selected_ids(state: &AppState) -> BTreeSet<u64> {
    state
        .selected_indices()
        .into_iter()
        .map(|index| id(&state.elements[index]))
        .collect()
}

type Fingerprint = (
    LabelDocument,
    Vec<usize>,
    (u64, u64, u64, bool),
    (bool, bool),
);
fn fingerprint(state: &AppState) -> Fingerprint {
    (
        state.document(),
        state.selected_indices(),
        (
            state.edit_revision,
            state.selection_revision,
            state.editor_generation,
            state.needs_rerender,
        ),
        (state.history.can_undo(), state.history.can_redo()),
    )
}

#[test]
fn multi_delete_removes_selected_hosts_with_their_attachments_once() {
    let mut state = fixture();
    let before = frames(&state);
    state.set_selected_indices(&[0, 1, 3, 7, 8], Some(7));
    observe(&mut state, 0.0);
    let document = state.document();
    let selection = selected_ids(&state);
    assert!(state.delete_selection(), "{}", state.status_message);
    let remaining: BTreeSet<_> = state.elements.iter().map(id).collect();
    assert_eq!(remaining, BTreeSet::from([3, 5, 6, 7, 10, 11]));
    assert!(scene::validate(&state.elements, 14).is_ok());
    let after = frames(&state);
    for id in [3, 7, 10] {
        assert_frame(&after[&id], &before[&id]);
    }
    observe(&mut state, 1.0);
    state.history_step(false);
    assert_eq!(state.document(), document);
    assert_eq!(selected_ids(&state), selection);
    assert!(!state.history.can_undo());
    state.history_step(true);
    assert_eq!(
        state.elements.iter().map(id).collect::<BTreeSet<_>>(),
        remaining
    );
}

#[test]
fn deleting_all_objects_has_no_dangling_selection_or_anchors() {
    let mut state = fixture();
    state.select_all_objects();
    assert!(state.delete_selection(), "{}", state.status_message);
    assert!(state.elements.is_empty());
    assert!(state.selected_indices().is_empty());
    assert_eq!(state.selected_element, None);
}

#[test]
fn delete_failure_restores_every_object_and_the_selection() {
    let mut state = fixture();
    state.elements[9]
        .as_object_mut()
        .unwrap()
        .content_box
        .as_mut()
        .unwrap()
        .width_mm = 0.0;
    state.set_selected_indices(&[0, 1, 4], Some(4));
    let before = fingerprint(&state);
    assert!(!state.delete_selection());
    assert_eq!(fingerprint(&state), before);
}

#[test]
fn non_contiguous_copy_keeps_each_sequence_block_in_source_order() {
    let mut state = fixture();
    state.set_selected_indices(&[0, 1, 2, 10], Some(10));
    let old_ids: BTreeSet<_> = state.elements.iter().map(id).collect();
    let before = frames(&state);
    assert!(state.copy_selection(), "{}", state.status_message);
    let copies = selected_ids(&state);
    assert_eq!(copies.len(), 5);
    assert!(copies.is_disjoint(&old_ids));
    assert_eq!(state.elements.len(), 16);
    for sequence in [10, 20] {
        let order: Vec<_> = state
            .layout_order()
            .into_iter()
            .filter_map(|index| {
                let object = state.elements[index].as_object().unwrap();
                if object.sequence != Some(sequence) {
                    return None;
                }
                let ObjectContent::Text { content, .. } = &object.content else {
                    panic!()
                };
                Some(content.clone())
            })
            .collect();
        let expected = if sequence == 10 {
            vec!["Port 3", "Port 3", "Port 7"]
        } else {
            vec!["Port 1", "Port 5", "Port 11", "Port 1", "Port 11"]
        };
        assert_eq!(order, expected);
    }
    let after = frames(&state);
    for id in [8, 9, 10] {
        assert_frame(&after[&id], &before[&id]);
    }
    for root in state.selected_indices() {
        if state.elements[root]
            .as_object()
            .unwrap()
            .scene
            .as_ref()
            .unwrap()
            .anchor
            .is_none()
        {
            for child in scene::children(&state.elements, root) {
                assert!(copies.contains(&id(&state.elements[child])));
            }
        }
    }
}

#[test]
fn copying_free_objects_preserves_group_offsets_and_attachment_membership() {
    let mut state = fixture();
    state.set_selected_indices(&[7, 8, 9], Some(7));
    let before = frames(&state);
    assert!(state.copy_selection(), "{}", state.status_message);
    let roots: Vec<_> = state
        .selected_indices()
        .into_iter()
        .filter(|index| {
            state.elements[*index]
                .as_object()
                .unwrap()
                .scene
                .as_ref()
                .unwrap()
                .anchor
                .is_none()
        })
        .collect();
    assert_eq!(roots.len(), 2);
    let after = frames(&state);
    let dx = after[&id(&state.elements[roots[0]])].x_mm - before[&8].x_mm;
    assert!(dx > 0.0);
    assert!((after[&id(&state.elements[roots[1]])].x_mm - before[&10].x_mm - dx).abs() < 1e-8);
    assert_eq!(scene::children(&state.elements, roots[0]).len(), 1);
    for id in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11] {
        assert_frame(&after[&id], &before[&id]);
    }
}

#[test]
fn copying_overlapping_hosts_preserves_the_selected_paint_order() {
    let mut state = fixture();
    for (wanted, layer, fill) in [(8, 30, ShapeFill::Ink), (10, 20, ShapeFill::Paper)] {
        let item = index(&state, wanted);
        let object = state.elements[item].as_object_mut().unwrap();
        object.content = ObjectContent::Shape {
            style: ShapeStyle {
                fill,
                stroke_width_mm: 0.0,
                ..Default::default()
            },
        };
        object.scene.as_mut().unwrap().layer = layer;
        object.frame.x_mm = 100.0;
    }
    state.elements[8]
        .as_object_mut()
        .unwrap()
        .scene
        .as_mut()
        .unwrap()
        .layer = 25;
    state.set_selected_indices(&[7, 9], Some(7));
    assert!(state.copy_selection(), "{}", state.status_message);
    let copied = selected_ids(&state);
    let order: Vec<_> = scene::paint_order(&state.elements)
        .into_iter()
        .filter_map(|index| {
            if !copied.contains(&id(&state.elements[index])) {
                return None;
            }
            match &state.elements[index].as_object().unwrap().content {
                ObjectContent::Shape { style } => Some(style.fill),
                _ => None,
            }
        })
        .collect();
    assert_eq!(
        order,
        vec![ShapeFill::Paper, ShapeFill::None, ShapeFill::Ink]
    );
}

#[test]
fn late_copy_identifier_failure_restores_partial_copies_and_history() {
    let mut state = fixture();
    state.elements[9]
        .as_object_mut()
        .unwrap()
        .scene
        .as_mut()
        .unwrap()
        .id = i64::MAX as u64 - 1;
    state.set_selected_indices(&[0, 2], Some(0));
    observe(&mut state, 0.0);
    let before = fingerprint(&state);
    assert!(!state.copy_selection());
    assert_eq!(fingerprint(&state), before);
}

#[test]
fn shared_unicode_style_uses_character_ranges_and_keeps_other_styles() {
    let mut state = fixture();
    for index in [0, 2] {
        let ObjectContent::Text {
            content, styles, ..
        } = &mut state.elements[index].as_object_mut().unwrap().content
        else {
            panic!()
        };
        *content = "Aé汉Ω".into();
        *styles = vec![StyleSpan {
            start: 1,
            end: 3,
            style: TextStyle {
                bold: true,
                font_size: Some(6.0),
                ..Default::default()
            },
        }];
    }
    state.set_selected_indices(&[0, 2], Some(2));
    assert!(
        state.apply_shared_edit(SharedEdit::Style(StylePatch {
            underline: Some(true),
            ..Default::default()
        })),
        "{}",
        state.status_message
    );
    assert!(
        state.apply_shared_edit(SharedEdit::FontSize(4.0)),
        "{}",
        state.status_message
    );
    for index in [0, 2] {
        let ObjectContent::Text {
            content,
            styles,
            font_size,
            ..
        } = &state.elements[index].as_object().unwrap().content
        else {
            panic!()
        };
        assert_eq!(content, "Aé汉Ω");
        assert_eq!(*font_size, Some(4.0));
        let styled = StyledText {
            text: content.clone(),
            spans: styles.clone(),
        };
        for index in 0..4 {
            let style = styled.style_at(index);
            assert!(style.underline);
            assert_eq!(style.bold, (1..3).contains(&index));
            assert_eq!(style.font_size, None);
        }
    }
    assert_eq!(selected_ids(&state), BTreeSet::from([1, 3]));
}

#[test]
fn mixed_content_shared_edit_rolls_back_the_text_changed_first() {
    let mut state = fixture();
    state.set_selected_indices(&[0, 8], Some(0));
    observe(&mut state, 0.0);
    let before = fingerprint(&state);
    assert!(!state.apply_shared_edit(SharedEdit::Style(StylePatch {
        bold: Some(true),
        ..Default::default()
    })));
    assert_eq!(fingerprint(&state), before);
}

#[test]
fn group_move_clamps_once_and_does_not_move_selected_children_twice() {
    let mut state = fixture();
    state.set_selected_indices(&[7, 8, 9], Some(7));
    let before = frames(&state);
    let delta = state
        .translate_selection(egui::vec2(-500.0, -500.0))
        .unwrap();
    assert_eq!(delta, egui::vec2(-100.0, -2.0));
    let after = frames(&state);
    for id in [8, 9, 10] {
        assert!((after[&id].x_mm - before[&id].x_mm - f64::from(delta.x)).abs() < 1e-8);
        assert!((after[&id].y_mm - before[&id].y_mm - f64::from(delta.y)).abs() < 1e-8);
    }
    for id in [1, 2, 3, 4, 5, 6, 7, 11] {
        assert_frame(&after[&id], &before[&id]);
    }
}

#[test]
fn group_move_rejects_non_finite_and_excessive_output_coordinates_atomically() {
    for delta in [
        egui::vec2(f32::NAN, 0.0),
        egui::vec2(0.0, f32::INFINITY),
        egui::vec2(f32::NEG_INFINITY, 0.0),
        egui::vec2(1e20, 0.0),
    ] {
        let mut state = fixture();
        state.set_selected_indices(&[7, 9], Some(7));
        let before = fingerprint(&state);
        assert!(
            state.translate_selection(delta).is_err(),
            "accepted {delta:?}"
        );
        assert_eq!(fingerprint(&state), before);
    }
}

#[test]
fn vertical_gap_line_move_keeps_mode_and_horizontal_move_returns_to_manual() {
    let mut state = fixture();
    state.elements[1]
        .as_object_mut()
        .unwrap()
        .scene
        .as_mut()
        .unwrap()
        .anchor
        .as_mut()
        .unwrap()
        .gap_center = Some(GapCenter { tail_gap_mm: 2.0 });
    state.document_version = 14;
    state.validate_scene_geometry().unwrap();
    state.set_selected_indices(&[1], Some(1));
    state.translate_selection(egui::vec2(0.0, 0.1)).unwrap();
    assert!(
        state.elements[1]
            .as_object()
            .unwrap()
            .scene
            .as_ref()
            .unwrap()
            .anchor
            .as_ref()
            .unwrap()
            .gap_center
            .is_some()
    );
    state.translate_selection(egui::vec2(0.2, 0.0)).unwrap();
    assert!(
        state.elements[1]
            .as_object()
            .unwrap()
            .scene
            .as_ref()
            .unwrap()
            .anchor
            .as_ref()
            .unwrap()
            .gap_center
            .is_none()
    );
}

#[test]
fn undo_and_redo_restore_the_full_set_and_primary_after_a_group_edit() {
    let mut state = fixture();
    state.set_selected_indices(&[7, 8, 9], Some(9));
    observe(&mut state, 0.0);
    let before = state.document();
    let selection = selected_ids(&state);
    state.translate_selection(egui::vec2(3.0, 0.5)).unwrap();
    observe(&mut state, 1.0);
    let after = state.document();
    state.set_selected_indices(&[0], Some(0));
    state.history_step(false);
    assert_eq!(state.document(), before);
    assert_eq!(selected_ids(&state), selection);
    assert_eq!(state.selected_element, Some(9));
    state.history_step(true);
    assert_eq!(state.document(), after);
    assert_eq!(selected_ids(&state), selection);
    assert_eq!(state.selected_element, Some(9));
}
