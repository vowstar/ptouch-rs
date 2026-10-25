// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Freeze the gesture on press. A selection gesture cannot become a transform.

use crate::state::{AppState, CanvasEditTarget, CanvasTarget};
use ptouch_render::document::LabelDocument;
use std::sync::Arc;

pub struct Input<'a> {
    pub bounds: egui::Rect,
    pub shapes: &'a [(usize, [egui::Pos2; 4])],
    pub press: Option<egui::Pos2>,
    pub hit: Option<(CanvasTarget, bool)>,
    pub pixels_per_mm: f64,
    pub snap: bool,
}

#[derive(Clone)]
enum Gesture {
    Click {
        collapse: Option<usize>,
        dragged: bool,
    },
    Marquee {
        start: egui::Pos2,
        before: Vec<usize>,
        primary: Option<usize>,
        add: bool,
    },
    Move {
        start: egui::Pos2,
        before: Arc<LabelDocument>,
        selection: Vec<usize>,
        primary: Option<usize>,
        clicked: usize,
        applied: egui::Vec2,
        moved: bool,
        scale: f64,
        snap: bool,
    },
}

fn intersects(quad: [egui::Pos2; 4], rect: egui::Rect) -> bool {
    let other = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    let mut axes = vec![egui::vec2(1.0, 0.0), egui::vec2(0.0, 1.0)];
    axes.extend((0..4).map(|i| (quad[(i + 1) % 4] - quad[i]).rot90()));
    axes.into_iter().all(|axis| {
        let range = |points: &[egui::Pos2; 4]| {
            points
                .iter()
                .map(|point| point.to_vec2().dot(axis))
                .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), value| {
                    (lo.min(value), hi.max(value))
                })
        };
        let (a, b) = range(&quad);
        let (c, d) = range(&other);
        a <= d && c <= b
    })
}

/// Returns true while this module owns the current pointer gesture.
pub fn handle(ui: &egui::Ui, state: &mut AppState, input: Input<'_>) -> bool {
    let id = egui::Id::new("multiple_selection_gesture");
    let cancelled = id.with("cancelled");
    if ui
        .ctx()
        .data_mut(|data| data.get_temp::<u64>(id.with("generation")))
        .is_some_and(|generation| generation != state.editor_generation)
    {
        clear(ui.ctx());
    }
    let down = ui.input(|input| input.pointer.primary_down());
    if ui
        .ctx()
        .data_mut(|data| data.get_temp::<bool>(cancelled).unwrap_or(false))
    {
        if !down {
            ui.ctx().data_mut(|data| data.remove::<bool>(cancelled));
        }
        return true;
    }
    let mut gesture = ui.ctx().data_mut(|data| data.get_temp::<Gesture>(id));
    if let Some(active) = gesture.as_ref()
        && ui.input(|input| input.key_pressed(egui::Key::Escape))
    {
        match active {
            Gesture::Marquee {
                before, primary, ..
            } => state.set_selected_indices(before, *primary),
            Gesture::Move {
                before,
                selection,
                primary,
                ..
            } => {
                state.restore_document((**before).clone());
                state.set_selected_indices(selection, *primary);
            }
            Gesture::Click { .. } => {}
        }
        ui.ctx().data_mut(|data| {
            data.remove::<Gesture>(id);
            data.insert_temp(cancelled, down);
        });
        return true;
    }
    if gesture.is_none()
        && let Some(start) = input.press.filter(|start| input.bounds.contains(*start))
    {
        let modifiers = ui.input(|input| input.modifiers);
        let selected = state.selected_indices();
        match input.hit {
            Some((target, true)) if modifiers.shift || modifiers.command || modifiers.ctrl => {
                state.toggle_canvas_item(target.element);
                gesture = Some(Gesture::Click {
                    collapse: None,
                    dragged: false,
                });
            }
            None => {
                gesture = Some(Gesture::Marquee {
                    start,
                    before: selected,
                    primary: state.selected_element,
                    add: modifiers.shift || modifiers.command || modifiers.ctrl,
                });
                if !modifiers.shift && !modifiers.command && !modifiers.ctrl {
                    state.set_selected_indices(&[], None);
                }
            }
            Some((target, true))
                if selected.len() > 1
                    && selected.contains(&target.element)
                    && !state.overall_flip_h
                    && state.can_translate_selection() =>
            {
                gesture = Some(Gesture::Move {
                    start,
                    before: Arc::new(state.document()),
                    selection: selected,
                    primary: state.selected_element,
                    clicked: target.element,
                    applied: egui::Vec2::ZERO,
                    moved: false,
                    scale: input.pixels_per_mm,
                    snap: input.snap,
                });
            }
            Some((target, true)) if selected.len() > 1 && selected.contains(&target.element) => {
                gesture = Some(Gesture::Click {
                    collapse: Some(target.element),
                    dragged: false,
                });
            }
            Some((target, _)) => {
                state.set_selected_indices(&[target.element], Some(target.element));
                // The normal canvas owns single-object move and handle edits.
                return false;
            }
        }
    }
    let Some(active) = gesture.as_mut() else {
        return false;
    };
    ui.ctx()
        .data_mut(|data| data.insert_temp(id.with("generation"), state.editor_generation));
    if let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) {
        match active {
            Gesture::Marquee {
                start, before, add, ..
            } => {
                let rect = egui::Rect::from_two_pos(*start, pointer).intersect(input.bounds);
                let mut indices = if *add { before.clone() } else { Vec::new() };
                if ui.input(|input| input.pointer.is_decidedly_dragging()) || !down {
                    indices.extend(
                        input
                            .shapes
                            .iter()
                            .filter(|(_, quad)| intersects(*quad, rect))
                            .map(|(index, _)| *index),
                    );
                }
                indices.sort_unstable();
                indices.dedup();
                state.set_selected_indices(&indices, indices.last().copied());
                let painter = ui.painter().with_clip_rect(input.bounds);
                painter.rect_filled(
                    rect,
                    0.0,
                    egui::Color32::from_rgba_unmultiplied(30, 100, 200, 25),
                );
                painter.rect_stroke(
                    rect,
                    0.0,
                    egui::Stroke::new(1.0, egui::Color32::from_rgb(30, 100, 200)),
                    egui::StrokeKind::Inside,
                );
            }
            Gesture::Move {
                start,
                clicked,
                applied,
                moved,
                scale,
                snap,
                ..
            } => {
                if ui.input(|input| input.pointer.is_decidedly_dragging()) {
                    *moved = true;
                }
                if *moved {
                    let delta = (pointer - *start) / (*scale as f32);
                    let total = if *snap {
                        egui::vec2(
                            (delta.x * 10.0).round() / 10.0,
                            (delta.y * 10.0).round() / 10.0,
                        )
                    } else {
                        delta
                    };
                    let total =
                        total * egui::vec2(1.0, if state.overall_flip_v { -1.0 } else { 1.0 });
                    match state.translate_selection(total - *applied) {
                        Ok(actual) => *applied += actual,
                        Err(message) => state.status_message = message,
                    }
                    state.canvas_drag_target = Some(CanvasTarget {
                        element: *clicked,
                        edit: CanvasEditTarget::Content,
                    });
                }
            }
            Gesture::Click { collapse, dragged } => {
                if collapse.is_some() && ui.input(|input| input.pointer.is_decidedly_dragging()) {
                    *dragged = true;
                }
            }
        }
    }
    if !down {
        if let Gesture::Click {
            collapse: Some(index),
            dragged: false,
        } = active
        {
            state.set_selected_indices(&[*index], Some(*index));
        }
        if let Gesture::Move {
            clicked,
            moved: false,
            ..
        } = active
        {
            state.set_selected_indices(&[*clicked], Some(*clicked));
        }
        ui.ctx().data_mut(|data| data.remove::<Gesture>(id));
    } else {
        ui.ctx()
            .data_mut(|data| data.insert_temp(id, gesture.unwrap()));
    }
    ui.ctx().request_repaint();
    true
}

pub fn shortcuts(ctx: &egui::Context, state: &mut AppState) {
    if state.pending_file_action.is_some()
        || ctx.text_edit_focused()
        || ctx.input(|input| input.pointer.any_down())
        || egui::Popup::is_any_open(ctx)
    {
        return;
    }
    if crate::panels::repeat_tools::is_open(ctx) {
        return;
    }
    if ctx
        .data_mut(|data| data.get_temp::<Gesture>(egui::Id::new("multiple_selection_gesture")))
        .is_some()
    {
        return;
    }
    if ctx.input_mut(|input| input.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
        state.select_all_objects();
    }
    if ctx.input_mut(|input| {
        input.consume_key(egui::Modifiers::NONE, egui::Key::Delete)
            || input.consume_key(egui::Modifiers::NONE, egui::Key::Backspace)
    }) {
        state.delete_selection();
    }
    if ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        state.set_selected_indices(&[], None);
    }
}

pub fn clear(ctx: &egui::Context) {
    ctx.data_mut(|data| {
        let id = egui::Id::new("multiple_selection_gesture");
        data.remove::<Gesture>(id);
        data.remove::<bool>(id.with("cancelled"));
        data.remove::<u64>(id.with("generation"));
    });
}

pub fn active(ctx: &egui::Context) -> bool {
    ctx.data_mut(|data| {
        data.get_temp::<Gesture>(egui::Id::new("multiple_selection_gesture"))
            .is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ptouch_render::object_layout::Placement;
    use ptouch_render::{
        document::{ContentBox, LabelObject, ObjectContent, TextBox},
        object_layout::{self, LayoutOptions, WidthSizing},
        scene::{self, Anchor, SceneOptions},
    };

    fn fixture() -> AppState {
        let mut state = AppState {
            document_version: 13,
            ..Default::default()
        };
        state.update_tape_pixels();
        for (id, x, y) in [(1, 10.0, 2.0), (2, 0.0, 0.0), (3, 24.0, 4.0)] {
            let mut object = LabelObject::text(
                format!("Object {id}"),
                TextBox {
                    x_mm: x,
                    y_mm: y,
                    width_mm: 8.0,
                    height_mm: 3.0,
                    ..Default::default()
                },
            );
            object.content_box = Some(ContentBox {
                width_mm: if id == 2 { 1.0 } else { 8.0 },
                height_mm: if id == 2 { 1.0 } else { 3.0 },
                offset_x_mm: 0.0,
                offset_y_mm: 0.0,
            });
            object.layout = Some(LayoutOptions {
                placement: Placement::Free,
                width: WidthSizing::Fixed,
                constrain: false,
                padding_mm: 0.0,
            });
            object.content = ObjectContent::Padding;
            object.scene = Some(Box::new(SceneOptions {
                id,
                layer: id as i32,
                anchor: (id == 2).then_some(Anchor {
                    target: 1,
                    horizontal: 0.5,
                    vertical: 0.5,
                    offset_x_mm: 0.0,
                    offset_y_mm: 0.0,
                    gap_center: None,
                }),
            }));
            object_layout::update_allocation(&mut object);
            state
                .elements
                .push(ptouch_render::document::LabelElement::Object(object));
        }
        state.validate_scene_geometry().unwrap();
        state.set_selected_indices(&[0], Some(0));
        state.needs_rerender = false;
        state
    }

    struct Pointer {
        context: egui::Context,
        time: f64,
    }

    impl Pointer {
        fn new() -> Self {
            Self {
                context: egui::Context::default(),
                time: 0.0,
            }
        }

        fn frame(
            &mut self,
            state: &mut AppState,
            mut events: Vec<egui::Event>,
            modifiers: egui::Modifiers,
            snap: bool,
        ) -> bool {
            self.time += 0.05;
            events.insert(0, egui::Event::ModifiersChanged(modifiers));
            let mut owned = false;
            let mut output = self.context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(700.0, 250.0),
                    )),
                    time: Some(self.time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let bounds =
                        egui::Rect::from_min_max(egui::pos2(20.0, 20.0), egui::pos2(650.0, 200.0));
                    let shapes: Vec<_> = scene::world_frames(&state.elements)
                        .unwrap()
                        .iter()
                        .enumerate()
                        .map(|(index, frame)| {
                            let rect = egui::Rect::from_min_size(
                                egui::pos2(
                                    20.0 + frame.x_mm as f32 * 10.0,
                                    20.0 + frame.y_mm as f32 * 10.0,
                                ),
                                egui::vec2(
                                    frame.width_mm as f32 * 10.0,
                                    frame.height_mm as f32 * 10.0,
                                ),
                            );
                            (
                                index,
                                [
                                    rect.left_top(),
                                    rect.right_top(),
                                    rect.right_bottom(),
                                    rect.left_bottom(),
                                ],
                            )
                        })
                        .collect();
                    let press = ui.input(|input| {
                        input.events.iter().find_map(|event| match event {
                            egui::Event::PointerButton {
                                pos,
                                button: egui::PointerButton::Primary,
                                pressed: true,
                                ..
                            } => Some(*pos),
                            _ => None,
                        })
                    });
                    let pointer = ui.input(|input| input.pointer.interact_pos());
                    let hit = pointer
                        .and_then(|point| {
                            shapes.iter().rev().find(|(_, quad)| {
                                egui::Rect::from_min_max(quad[0], quad[2]).contains(point)
                            })
                        })
                        .map(|(element, _)| {
                            (
                                CanvasTarget {
                                    element: *element,
                                    edit: CanvasEditTarget::Content,
                                },
                                true,
                            )
                        });
                    owned = handle(
                        ui,
                        state,
                        Input {
                            bounds,
                            shapes: &shapes,
                            press,
                            hit,
                            pixels_per_mm: 10.0,
                            snap,
                        },
                    );
                },
            );
            output.textures_delta.clear();
            owned
        }

        fn press(
            &mut self,
            state: &mut AppState,
            point: egui::Pos2,
            modifiers: egui::Modifiers,
            snap: bool,
        ) -> bool {
            self.frame(
                state,
                vec![
                    egui::Event::PointerMoved(point),
                    button(point, true, modifiers),
                ],
                modifiers,
                snap,
            )
        }

        fn move_to(
            &mut self,
            state: &mut AppState,
            point: egui::Pos2,
            modifiers: egui::Modifiers,
            snap: bool,
        ) -> bool {
            self.frame(
                state,
                vec![egui::Event::PointerMoved(point)],
                modifiers,
                snap,
            )
        }

        fn release(
            &mut self,
            state: &mut AppState,
            point: egui::Pos2,
            modifiers: egui::Modifiers,
            snap: bool,
        ) -> bool {
            self.frame(
                state,
                vec![
                    egui::Event::PointerMoved(point),
                    button(point, false, modifiers),
                ],
                modifiers,
                snap,
            )
        }

        fn escape(&mut self, state: &mut AppState) -> bool {
            self.frame(
                state,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                egui::Modifiers::NONE,
                false,
            )
        }
    }

    fn button(pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        }
    }

    fn frames(state: &AppState) -> Vec<TextBox> {
        scene::world_frames(&state.elements).unwrap()
    }

    fn assert_delta(after: &[TextBox], before: &[TextBox], delta: egui::Vec2) {
        for (after, before) in after.iter().zip(before) {
            assert!((after.x_mm - before.x_mm - f64::from(delta.x)).abs() < 1e-6);
            assert!((after.y_mm - before.y_mm - f64::from(delta.y)).abs() < 1e-6);
        }
    }

    #[test]
    fn shift_and_control_click_toggle_objects_without_editing_the_document() {
        let mut state = fixture();
        let original = state.document();
        let revision = state.edit_revision;
        let mut pointer = Pointer::new();
        let first = egui::pos2(130.0, 45.0);
        let second = egui::pos2(270.0, 75.0);
        assert!(pointer.press(&mut state, second, egui::Modifiers::SHIFT, false));
        pointer.release(&mut state, second, egui::Modifiers::SHIFT, false);
        assert_eq!(state.selected_indices(), vec![0, 2]);
        pointer.press(&mut state, first, egui::Modifiers::CTRL, false);
        pointer.release(&mut state, first, egui::Modifiers::CTRL, false);
        assert_eq!(state.selected_indices(), vec![2]);
        pointer.press(&mut state, second, egui::Modifiers::CTRL, false);
        pointer.release(&mut state, second, egui::Modifiers::CTRL, false);
        assert!(state.selected_indices().is_empty());
        assert_eq!(state.document(), original);
        assert_eq!(state.edit_revision, revision);
    }

    #[test]
    fn blank_marquee_escape_restores_the_selection_until_pointer_release() {
        let mut state = fixture();
        state.set_selected_indices(&[2], Some(2));
        let original = state.document();
        let mut pointer = Pointer::new();
        let start = egui::pos2(95.0, 100.0);
        let end = egui::pos2(350.0, 30.0);
        assert!(pointer.press(&mut state, start, egui::Modifiers::NONE, false));
        assert!(state.selected_indices().is_empty());
        pointer.move_to(&mut state, end, egui::Modifiers::NONE, false);
        assert_eq!(state.selected_indices(), vec![0, 1, 2]);
        assert!(pointer.escape(&mut state));
        assert_eq!(state.selected_indices(), vec![2]);
        assert!(!active(&pointer.context));
        pointer.move_to(&mut state, start, egui::Modifiers::NONE, false);
        pointer.release(&mut state, start, egui::Modifiers::NONE, false);
        assert_eq!(state.selected_indices(), vec![2]);
        assert_eq!(state.document(), original);
    }

    #[test]
    fn additive_marquee_keeps_the_original_selection() {
        let mut state = fixture();
        state.set_selected_indices(&[2], Some(2));
        let original = state.document();
        let mut pointer = Pointer::new();
        let start = egui::pos2(95.0, 100.0);
        let end = egui::pos2(210.0, 30.0);
        pointer.press(&mut state, start, egui::Modifiers::SHIFT, false);
        pointer.move_to(&mut state, end, egui::Modifiers::SHIFT, false);
        pointer.release(&mut state, end, egui::Modifiers::SHIFT, false);
        assert_eq!(state.selected_indices(), vec![0, 1, 2]);
        assert_eq!(state.document(), original);
    }

    #[test]
    fn free_group_drag_moves_attachments_once_and_escape_restores_every_object() {
        for cancel in [false, true] {
            let mut state = fixture();
            state.set_selected_indices(&[0, 1, 2], Some(2));
            let original = state.document();
            let before = frames(&state);
            let mut pointer = Pointer::new();
            let start = egui::pos2(130.0, 45.0);
            let end = start + egui::vec2(20.0, 5.0);
            assert!(pointer.press(&mut state, start, egui::Modifiers::NONE, false));
            assert_eq!(state.selected_indices(), vec![0, 1, 2]);
            pointer.move_to(&mut state, end, egui::Modifiers::NONE, false);
            assert_delta(&frames(&state), &before, egui::vec2(2.0, 0.5));
            assert_ne!(state.document(), original);
            if cancel {
                pointer.escape(&mut state);
                assert_eq!(state.document(), original);
                assert_eq!(state.selected_element, Some(2));
            }
            pointer.release(&mut state, end, egui::Modifiers::NONE, false);
            assert_eq!(state.selected_indices(), vec![0, 1, 2]);
            assert!(!active(&pointer.context));
            if cancel {
                assert_eq!(state.document(), original);
            } else {
                assert_delta(&frames(&state), &before, egui::vec2(2.0, 0.5));
            }
        }
    }

    #[test]
    fn a_selection_with_sequence_items_does_not_move_or_collapse_on_drag() {
        let mut state = fixture();
        let object = state.elements[2].as_object_mut().unwrap();
        object.layout.as_mut().unwrap().placement = Placement::Flow;
        object.sequence = Some(7);
        state.set_selected_indices(&[0, 2], Some(2));
        let original = state.document();
        let mut pointer = Pointer::new();
        let start = egui::pos2(130.0, 45.0);
        let end = start + egui::vec2(20.0, 5.0);
        assert!(pointer.press(&mut state, start, egui::Modifiers::NONE, false));
        pointer.move_to(&mut state, end, egui::Modifiers::NONE, false);
        pointer.release(&mut state, end, egui::Modifiers::NONE, false);
        assert_eq!(state.selected_indices(), vec![0, 2]);
        assert_eq!(state.document(), original);
        // A click without dragging can still select one member.
        pointer.press(&mut state, start, egui::Modifiers::NONE, false);
        pointer.release(&mut state, start, egui::Modifiers::NONE, false);
        assert_eq!(state.selected_indices(), vec![0]);
        assert_eq!(state.document(), original);
    }

    #[test]
    fn an_old_gesture_cannot_restore_a_newly_loaded_document() {
        let mut state = fixture();
        state.set_selected_indices(&[0, 1, 2], Some(0));
        let original = state.document();
        let mut pointer = Pointer::new();
        let start = egui::pos2(130.0, 45.0);
        let end = start + egui::vec2(20.0, 5.0);
        pointer.press(&mut state, start, egui::Modifiers::NONE, false);
        pointer.move_to(&mut state, end, egui::Modifiers::NONE, false);
        let mut replacement = fixture();
        replacement.elements[0].as_object_mut().unwrap().content = ObjectContent::Text {
            content: "New label".into(),
            font_size: Some(6.0),
            align: ptouch_render::text::TextAlign::Center,
            line_spacing: ptouch_render::text::DEFAULT_LINE_SPACING,
            styles: Vec::new(),
        };
        state.restore_document(replacement.document());
        let loaded = state.document();
        assert_ne!(loaded, original);
        pointer.escape(&mut state);
        pointer.release(&mut state, end, egui::Modifiers::NONE, false);
        assert_eq!(state.document(), loaded);
        assert!(!active(&pointer.context));
    }

    #[test]
    fn group_drag_respects_the_snap_switch() {
        for snap in [false, true] {
            let mut state = fixture();
            state.set_selected_indices(&[0, 1, 2], Some(0));
            let before = frames(&state);
            let mut pointer = Pointer::new();
            let start = egui::pos2(130.0, 45.0);
            let end = start + egui::vec2(13.6, 4.6);
            pointer.press(&mut state, start, egui::Modifiers::NONE, snap);
            pointer.move_to(&mut state, end, egui::Modifiers::NONE, snap);
            pointer.release(&mut state, end, egui::Modifiers::NONE, snap);
            assert_delta(
                &frames(&state),
                &before,
                if snap {
                    egui::vec2(1.4, 0.5)
                } else {
                    egui::vec2(1.36, 0.46)
                },
            );
            assert_eq!(state.selected_indices(), vec![0, 1, 2]);
        }
    }
    #[test]
    fn selection_uses_rotated_geometry_instead_of_its_bounding_rectangle() {
        let quad = [
            egui::pos2(5.0, 0.0),
            egui::pos2(10.0, 5.0),
            egui::pos2(5.0, 10.0),
            egui::pos2(0.0, 5.0),
        ];
        assert!(!intersects(
            quad,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0))
        ));
        assert!(intersects(
            quad,
            egui::Rect::from_min_max(egui::pos2(4.0, 4.0), egui::pos2(6.0, 6.0))
        ));
    }
}
