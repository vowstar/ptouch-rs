// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Central canvas panel for label preview display.

use ptouch_render::document::{ContentBox, ElementBounds, PositionMode, TextBox};

use crate::state::{AppState, CanvasEditTarget, CanvasTarget};

const PADDING: f32 = 24.0;
const RULER_HEIGHT: f32 = 30.0;
const RULER_WIDTH: f32 = 52.0;
const ROTATE_RADIUS: f32 = 6.0;
const HANDLE_MARGIN: f32 = 24.0;

#[derive(Clone)]
struct RotationDrag {
    last_pointer: egui::Pos2,
    accumulated: f32,
    initial: f32,
    reverse: bool,
}

#[derive(Clone)]
struct ContentDrag {
    original: ContentBox,
    corners: [egui::Pos2; 4],
}

#[derive(Clone)]
struct CanvasDrag {
    before: Option<ptouch_render::document::LabelDocument>,
    reorder: Option<ReorderDrag>,
    gap: bool,
    scroll_offset: egui::Vec2,
    element: usize,
    start: egui::Pos2,
    frame: TextBox,
    left_mm: f64,
    pixels_per_mm: f64,
    resize: bool,
    pointer_moved: bool,
    flip_h: bool,
    flip_v: bool,
    rotation: Option<RotationDrag>,
    content: Option<ContentDrag>,
    font_size: Option<f32>,
    font_styles: Vec<ptouch_render::styled_text::StyleSpan>,
}

#[derive(Clone)]
struct ReorderDrag {
    start: usize,
    centers_mm: Vec<f64>,
    current: usize,
}

/// Render and edit at the original raster scale, independent of GPU downsampling.
pub fn show_canvas(ui: &mut egui::Ui, state: &mut AppState) {
    let drag_id = ui.id().with("object_drag");
    let snap_id = ui.id().with("snap_mm");
    let press_id = ui.id().with("canvas_press");
    let menu_id = ui.id().with("canvas_menu_target");
    let previous_hover = state.canvas_hover_target;
    let previous_drag = state.canvas_drag_target;
    if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        let active = ui
            .ctx()
            .data_mut(|data| data.get_temp::<CanvasDrag>(drag_id));
        if let Some(active) = active
            && let Some(before) = active.before
        {
            state.restore_document(before);
            state.selected_element = Some(active.element);
        }
        ui.ctx().data_mut(|data| {
            data.remove::<CanvasDrag>(drag_id);
            data.remove::<CanvasPress>(press_id);
        });
        state.canvas_drag_target = None;
    }
    state.canvas_hover_target = None;
    let mut snap = ui
        .ctx()
        .data_mut(|data| data.get_temp::<bool>(snap_id).unwrap_or(true));
    ui.horizontal(|ui| {
        if ui.button("Fit").clicked() {
            state.zoom_fit = true;
        }
        if ui.button("1:1").clicked() {
            state.zoom = 1.0;
            state.zoom_fit = false;
        }
        if ui.button("+").clicked() {
            state.zoom = (state.zoom * 1.25).min(20.0);
            state.zoom_fit = false;
        }
        if ui.button("−").clicked() {
            state.zoom = (state.zoom / 1.25).max(0.001);
            state.zoom_fit = false;
        }
        ui.label(format!("Zoom: {:.0}%", state.zoom * 100.0));
        ui.checkbox(&mut snap, "Snap 0.1 mm");
    });
    ui.ctx().data_mut(|data| data.insert_temp(snap_id, snap));
    ui.small(format!("{} mm tape · {:.2} mm printable · edge placement schematic",
        if state.tape_width_mm == 4 { 3.5 } else { f64::from(state.tape_width_mm) },
        state.printable_height_mm()))
        .on_hover_text("Y measures the full tape. Dark bands cannot be printed. Exact top/bottom head offsets are not known; properties use the printable area's origin.");
    let can_edit = state.is_free_layout();
    let can_drag = can_edit && !state.overall_flip_h;
    if let Some(diagnostic) = state.preview_diagnostics.first() {
        let message = if state.preview_diagnostics.len() > 1 {
            format!(
                "{diagnostic} (+{} more)",
                state.preview_diagnostics.len() - 1
            )
        } else {
            diagnostic.clone()
        };
        ui.colored_label(ui.visuals().warn_fg_color, message)
            .on_hover_text(state.preview_diagnostics.join("\n"));
    } else {
        // Reserve the diagnostic row so clipping feedback cannot move the
        // document origin under a held pointer.
        ui.label(" ");
    }
    ui.separator();

    let canvas_size = ui.available_size();
    let (Some(texture), Some(bitmap)) = (&state.preview_texture, &state.preview_bitmap) else {
        ui.ctx().data_mut(|data| {
            data.remove::<CanvasDrag>(drag_id);
            data.remove::<CanvasPress>(press_id);
        });
        state.canvas_drag_target = None;
        if previous_hover.is_some() || previous_drag.is_some() {
            ui.ctx().request_repaint();
        }
        let (rect, _) = ui.allocate_exact_size(canvas_size, egui::Sense::hover());
        ui.painter()
            .rect_filled(rect, 0.0, egui::Color32::from_gray(200));
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Add elements to preview",
            egui::FontId::proportional(18.0),
            egui::Color32::from_gray(100),
        );
        return;
    };
    let printable_size = egui::vec2(bitmap.width() as f32, bitmap.height() as f32);
    let physical_mm = if state.tape_width_mm == 4 {
        3.5
    } else {
        f32::from(state.tape_width_mm)
    };
    let physical_height =
        (physical_mm * f32::from(state.target_dpi()) / 25.4).max(printable_size.y);
    let original_size = egui::vec2(printable_size.x, physical_height);
    // Selecting an item can widen the properties panel before drag detection.
    // Keep the press geometry stable until release, including that first frame.
    if state.zoom_fit
        && !ui
            .ctx()
            .data_mut(|data| data.get_temp::<CanvasPress>(press_id).is_some())
    {
        state.zoom = fit_zoom(original_size, canvas_size);
    }
    let display_size = original_size * state.zoom;
    let viewport_size = canvas_size - egui::vec2(RULER_WIDTH, RULER_HEIGHT);
    let content_size = egui::vec2(
        (display_size.x + PADDING * 2.0 + HANDLE_MARGIN).max(viewport_size.x - 12.0),
        (display_size.y + PADDING * 2.0).max(viewport_size.y - 12.0),
    );
    let texture_id = texture.id();
    let dpi = state.target_dpi();
    let (canvas_rect, _) = ui.allocate_exact_size(canvas_size, egui::Sense::hover());
    ui.painter()
        .rect_filled(canvas_rect, 0.0, egui::Color32::from_gray(200));
    let viewport_rect = egui::Rect::from_min_max(
        canvas_rect.min + egui::vec2(RULER_WIDTH, RULER_HEIGHT),
        canvas_rect.max,
    );
    let mut canvas_ui = ui.new_child(
        egui::UiBuilder::new()
            .id(ui.id().with("canvas_scroll"))
            .max_rect(viewport_rect),
    );
    let mut ruler_label = None;
    let scroll = egui::ScrollArea::both()
        .auto_shrink([false, false])
        .scroll_source(egui::scroll_area::ScrollSource {
            drag: egui::scroll_area::DragScroll::Never,
            ..Default::default()
        })
        .show(&mut canvas_ui, |ui| {
            let (rect, response) =
                ui.allocate_exact_size(content_size, egui::Sense::click_and_drag());
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, egui::Color32::from_gray(200));
            // A stable left/top origin prevents auto-length changes from moving
            // the canvas under the pointer during a drag.
            let tape_rect =
                egui::Rect::from_min_size(rect.min + egui::vec2(PADDING, PADDING), display_size);
            let margin = (physical_height - printable_size.y) * state.zoom / 2.0;
            let label_rect = egui::Rect::from_min_size(
                tape_rect.min + egui::vec2(0.0, margin),
                printable_size * state.zoom,
            );
            ruler_label = Some((tape_rect, label_rect));
            painter.rect_filled(tape_rect, 0.0, egui::Color32::from_gray(110));
            painter.rect_filled(label_rect, 0.0, egui::Color32::WHITE);
            painter.image(
                texture_id,
                label_rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
            painter.rect_stroke(
                label_rect,
                0.0,
                egui::Stroke::new(1.0, egui::Color32::from_gray(128)),
                egui::StrokeKind::Outside,
            );
            let geometry: Vec<_> = state
                .preview_bounds
                .iter()
                .filter_map(|bound| editing_geometry(state, label_rect.min, bound))
                .collect();
            let selected = state.selected_canvas_target();
            let hit = |pos| {
                canvas_hit(
                    pos,
                    label_rect,
                    painter.clip_rect(),
                    &geometry,
                    selected,
                    can_drag,
                    state.overall_flip_v,
                )
            };
            // Resolve once on press, before the drag threshold. Crossing a
            // different target later must never change the meaning of a gesture.
            let raw_press = ui.input(|i| {
                if i.key_pressed(egui::Key::Escape) {
                    return None;
                }
                i.events.iter().find_map(|e| match e {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        ..
                    } => Some(*pos),
                    _ => None,
                })
            });
            if let Some(start) = raw_press
                && response.rect.contains(start)
                && painter.clip_rect().contains(start)
                && ui.ctx().layer_id_at(start) == Some(ui.layer_id())
            {
                let press_hit = hit(start);
                let drag = press_hit.filter(|_| can_drag).and_then(|hit| {
                    let index = hit.target.element;
                    let object = state.elements.get(index)?.as_object()?;
                    let content_edit = hit.target.edit == CanvasEditTarget::Content;
                    let content = if content_edit {
                        Some(ContentDrag {
                            original: state.resolved_content_box(index)?,
                            corners: geometry.iter().find(|g| g.element == index)?.object?,
                        })
                    } else {
                        None
                    };
                    Some(CanvasDrag {
                        before: Some(state.document()),
                        gap: hit.action == CanvasAction::Gap,
                        scroll_offset: viewport_rect.min - rect.min,
                        reorder: if !content_edit && hit.action == CanvasAction::Move {
                            state
                                .flow_chain(index)
                                .filter(|chain| chain.start() != chain.end())
                                .map(|chain| ReorderDrag {
                                    start: *chain.start(),
                                    centers_mm: chain
                                        .filter(|i| *i != index)
                                        .filter_map(|i| {
                                            state.element_rect_mm(i).map(|(x, _, w, _)| x + w / 2.0)
                                        })
                                        .collect(),
                                    current: index,
                                })
                        } else {
                            None
                        },
                        element: index,
                        start,
                        frame: object.frame.clone(),
                        left_mm: state
                            .element_rect_mm(index)
                            .map_or(object.frame.x_mm, |r| r.0),
                        pixels_per_mm: f64::from(dpi) / 25.4 * f64::from(state.zoom),
                        resize: hit.action == CanvasAction::Resize,
                        pointer_moved: false,
                        flip_h: state.overall_flip_h ^ (content_edit && object.flip_h),
                        flip_v: state.overall_flip_v ^ (content_edit && object.flip_v),
                        rotation: (hit.action == CanvasAction::Rotate).then_some(RotationDrag {
                            last_pointer: start,
                            accumulated: 0.0,
                            initial: object.rotation,
                            reverse: object.flip_h
                                ^ object.flip_v
                                ^ state.overall_flip_h
                                ^ state.overall_flip_v,
                        }),
                        content,
                        font_styles: match &object.content {
                            ptouch_render::document::ObjectContent::Text { styles, .. } => {
                                styles.clone()
                            }
                            _ => Vec::new(),
                        },
                        font_size: match &object.content {
                            ptouch_render::document::ObjectContent::Text { font_size, .. } => {
                                *font_size
                            }
                            _ => None,
                        },
                    })
                });
                let target = press_hit.map(|hit| hit.target);
                ui.ctx().data_mut(|data| {
                    data.insert_temp(
                        press_id,
                        CanvasPress {
                            target,
                            action: press_hit.map_or(CanvasAction::Move, |h| h.action),
                            drag,
                        },
                    )
                });
                state.select_canvas_target(target);
                ui.ctx().request_repaint();
            }
            if response.secondary_clicked()
                && let Some(pos) = response.interact_pointer_pos()
            {
                let target = canvas_hit(
                    pos,
                    label_rect,
                    painter.clip_rect(),
                    &geometry,
                    state.selected_canvas_target(),
                    false,
                    state.overall_flip_v,
                )
                .map(|h| h.target);
                ui.ctx().data_mut(|data| data.insert_temp(menu_id, target));
            }
            response.context_menu(|ui| {
                let target = ui
                    .ctx()
                    .data_mut(|data| data.get_temp::<Option<CanvasTarget>>(menu_id))
                    .flatten();
                if let Some(target) = target {
                    if ui.button("Edit content").clicked() {
                        state.select_canvas_target(Some(CanvasTarget {
                            edit: CanvasEditTarget::Content,
                            ..target
                        }));
                        ui.close();
                    }
                    let has_slot = state
                        .elements
                        .get(target.element)
                        .is_some_and(|e| e.as_object().is_some())
                        || geometry
                            .iter()
                            .any(|g| g.element == target.element && g.has_slot);
                    if ui
                        .add_enabled(has_slot, egui::Button::new("Adjust layout…"))
                        .clicked()
                    {
                        state.select_canvas_target(Some(CanvasTarget {
                            edit: CanvasEditTarget::Slot,
                            ..target
                        }));
                        ui.close();
                    }
                } else {
                    ui.weak("No object here");
                }
            });
            let raw_release = ui.input(|i| {
                i.events.iter().rev().find_map(|e| match e {
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        ..
                    } => Some(*pos),
                    _ => None,
                })
            });
            // egui may see movement and release in the same frame and never
            // report drag_started, even when the press was an earlier frame.
            // Finish from that frozen press; do not restart an active rotation.
            let fast_release = raw_release.is_some_and(|end| {
                ui.ctx().data_mut(|data| {
                    data.get_temp::<CanvasDrag>(drag_id).is_none()
                        && data
                            .get_temp::<CanvasPress>(press_id)
                            .and_then(|p| p.drag)
                            .is_some_and(|d| d.start.distance(end) > 4.0)
                })
            });
            if can_drag
                && (response.drag_started() || fast_release)
                && let Some(press) = ui
                    .ctx()
                    .data_mut(|data| data.get_temp::<CanvasPress>(press_id))
                && let Some(drag) = press.drag
            {
                state.canvas_drag_target = press.target;
                state.zoom_fit = false;
                ui.ctx().data_mut(|data| data.insert_temp(drag_id, drag));
            }
            let active_press = ui
                .ctx()
                .data_mut(|data| data.get_temp::<CanvasPress>(press_id));
            let hover_hit = if ui.input(|i| i.pointer.primary_down()) && active_press.is_some() {
                active_press.as_ref().and_then(|p| {
                    p.target.map(|target| CanvasHit {
                        target,
                        action: p.action,
                    })
                })
            } else {
                response.hover_pos().and_then(|pos| {
                    canvas_hit(
                        pos,
                        label_rect,
                        painter.clip_rect(),
                        &geometry,
                        state.selected_canvas_target(),
                        can_drag,
                        state.overall_flip_v,
                    )
                })
            };
            state.canvas_hover_target = hover_hit.map(|hit| hit.target);
            if let Some(hit) = hover_hit.filter(|_| can_drag) {
                let cursor = match hit.action {
                    CanvasAction::Rotate => egui::CursorIcon::Grab,
                    CanvasAction::Resize => {
                        let quad = target_quad(&geometry, hit.target).unwrap();
                        resize_cursor(quad[2] - quad[0])
                    }
                    CanvasAction::Move => egui::CursorIcon::Move,
                    CanvasAction::Gap => egui::CursorIcon::ResizeHorizontal,
                };
                response.clone().on_hover_cursor(cursor);
            }
            draw_targets(painter, state, &geometry, can_drag);
            if can_edit
                && (response.dragged()
                    || response.drag_stopped()
                    || fast_release
                    || raw_release.is_some())
                && let Some(pos) = response.interact_pointer_pos().or(raw_release)
                && let Some(mut drag) = ui
                    .ctx()
                    .data_mut(|data| data.get_temp::<CanvasDrag>(drag_id))
            {
                let scroll_offset = viewport_rect.min - rect.min;
                if drag.rotation.is_none() {
                    drag.start -= scroll_offset - drag.scroll_offset;
                    let visible = ui.clip_rect();
                    if ui.input(|i| i.pointer.primary_down())
                        && (pos.x < visible.left() + 24.0
                            || pos.x > visible.right() - 24.0
                            || pos.y < visible.top() + 24.0
                            || pos.y > visible.bottom() - 24.0)
                    {
                        ui.scroll_to_rect(
                            egui::Rect::from_center_size(pos, egui::vec2(64.0, 64.0)),
                            None,
                        );
                        ui.ctx().request_repaint();
                    }
                }
                drag.scroll_offset = scroll_offset;
                drag.pointer_moved |= pos != drag.start;
                // A long press can cross egui's drag threshold without moving.
                // It must not pin legacy content or quantize saved coordinates.
                if drag.pointer_moved
                    && let Some(reorder) = drag.reorder.as_mut()
                {
                    let x_mm = f64::from(pos.x - label_rect.left()) / drag.pixels_per_mm;
                    let to =
                        reorder.start + reorder.centers_mm.iter().filter(|x| x_mm > **x).count();
                    if to != reorder.current
                        && let Some(before) = &drag.before
                    {
                        state.elements = before.elements.clone();
                        state.selected_element = Some(drag.element);
                        if to != drag.element {
                            state.reorder_object(drag.element, to);
                        } else {
                            state.mark_dirty();
                        }
                        state.selected_element = Some(to);
                        reorder.current = to;
                        let target = CanvasTarget {
                            element: to,
                            edit: CanvasEditTarget::Slot,
                        };
                        state.canvas_drag_target = Some(target);
                        ui.ctx().data_mut(|data| {
                            if let Some(mut press) = data.get_temp::<CanvasPress>(press_id) {
                                press.target = Some(target);
                                data.insert_temp(press_id, press);
                            }
                        });
                    }
                    if let Some(g) = geometry.iter().find(|g| g.element == reorder.current) {
                        let x = g.slot.left();
                        painter.line_segment(
                            [
                                egui::pos2(x, g.slot.top() - 8.0),
                                egui::pos2(x, g.slot.bottom() + 8.0),
                            ],
                            egui::Stroke::new(3.0, egui::Color32::from_rgb(205, 115, 0)),
                        );
                    }
                } else if drag.pointer_moved {
                    let rotation = if let Some(rotation) = drag.rotation.as_mut() {
                        let center = geometry
                            .iter()
                            .find(|g| g.element == drag.element)
                            .and_then(|g| g.object)
                            .map(quad_center);
                        center.and_then(|center| {
                            let mut angle = None;
                            let events = ui.input(|i| i.events.clone());
                            let first = if raw_press.is_some() {
                                events
                                    .iter()
                                    .rposition(|e| {
                                        matches!(
                                            e,
                                            egui::Event::PointerButton {
                                                button: egui::PointerButton::Primary,
                                                pressed: true,
                                                ..
                                            }
                                        )
                                    })
                                    .map_or(0, |i| i + 1)
                            } else {
                                0
                            };
                            for event in &events[first..] {
                                if let egui::Event::PointerMoved(point) = event {
                                    angle = dragged_rotation(
                                        rotation,
                                        *point,
                                        center,
                                        ui.input(|i| i.modifiers.shift),
                                    );
                                }
                            }
                            dragged_rotation(rotation, pos, center, ui.input(|i| i.modifiers.shift))
                                .or(angle)
                        })
                    } else {
                        None
                    };
                    let new_content = if drag.rotation.is_none() && can_drag {
                        drag.content.as_ref().map(|content| {
                            dragged_content(
                                &drag,
                                content,
                                pos - drag.start,
                                snap,
                                25.4 / f64::from(dpi),
                            )
                        })
                    } else {
                        None
                    };
                    let frame = if drag.content.is_none() && drag.rotation.is_none() && can_drag {
                        let mut frame = dragged_frame(
                            &drag,
                            pos - drag.start,
                            f64::from(state.tape_width_px) * 25.4 / f64::from(dpi),
                            snap,
                            25.4 / f64::from(dpi),
                        );
                        if drag.gap {
                            frame = drag.frame.clone();
                            frame.gap_mm = (drag.frame.gap_mm
                                + f64::from(pos.x - drag.start.x) / drag.pixels_per_mm)
                                .max(0.0);
                            if snap {
                                frame.gap_mm = (frame.gap_mm * 10.0).round() / 10.0;
                            }
                        }
                        Some(frame)
                    } else {
                        None
                    };
                    let rotation = rotation.filter(|angle| {
                        state
                            .elements
                            .get(drag.element)
                            .and_then(|e| e.as_object())
                            .is_some_and(|o| o.rotation != *angle)
                    });
                    if let Some(before) = state
                        .elements
                        .get(drag.element)
                        .and_then(|e| e.as_object())
                        .cloned()
                    {
                        let mut object = before.clone();
                        if let Some(frame) = frame {
                            if drag.resize
                                && frame.width_mm != drag.frame.width_mm
                                && let Some(policy) = &mut object.layout
                            {
                                policy.width = ptouch_render::object_layout::WidthSizing::Fixed;
                            }
                            if drag.frame.position == PositionMode::Relative
                                && frame.position == PositionMode::Absolute
                            {
                                state.status_message =
                                    "Moved before the previous slot; using absolute position"
                                        .into();
                            }
                            object.frame = frame;
                        } else if let Some(content) = new_content {
                            if drag.resize
                                && object.layout.as_ref().is_some_and(|p| {
                                    p.placement == ptouch_render::object_layout::Placement::Free
                                })
                            {
                                object.layout.as_mut().unwrap().width =
                                    ptouch_render::object_layout::WidthSizing::Fixed;
                            }
                            let mut allowed = true;
                            if drag.resize
                                && let Some(size) = drag.font_size
                                && let Some(start) = &drag.content
                                && let ptouch_render::document::ObjectContent::Text {
                                    font_size,
                                    styles,
                                    ..
                                } = &mut object.content
                            {
                                let ratio = (content.width_mm / start.original.width_mm) as f32;
                                let mut scaled = drag.font_styles.clone();
                                for span in &mut scaled {
                                    if let Some(size) = &mut span.style.font_size {
                                        *size *= ratio;
                                    }
                                }
                                allowed = scaled.iter().all(|s| s.style.validate().is_ok());
                                if allowed {
                                    *font_size = Some(size * ratio);
                                    *styles = scaled;
                                }
                            }
                            if allowed {
                                object.content_box = Some(content);
                            } else {
                                state.status_message =
                                    "Resize exceeds the inline font size limit".into();
                            }
                        } else if let Some(rotation) = rotation {
                            if object.content_box.is_none() {
                                object.content_box =
                                    drag.content.as_ref().map(|c| c.original.clone());
                            }
                            object.rotation = rotation;
                        }
                        let original = drag
                            .before
                            .as_ref()
                            .and_then(|d| d.elements.get(drag.element))
                            .and_then(|e| e.as_object())
                            .unwrap_or(&before);
                        match state.constrain_transform(drag.element, original, &mut object) {
                            Err(message) => state.status_message = message.into(),
                            Ok(clamped) => {
                                state.transform_feedback(
                                    clamped,
                                    object.layout.as_ref().is_some_and(|p| {
                                        p.placement == ptouch_render::object_layout::Placement::Free
                                    }),
                                );
                                if object != before {
                                    state.elements[drag.element] =
                                        crate::state::LabelElement::Object(object);
                                    state.document_version = state.document_version.max(4);
                                    state.mark_dirty();
                                }
                            }
                        }
                    }
                }
                ui.ctx().data_mut(|data| data.insert_temp(drag_id, drag));
            }
            if response.drag_stopped() || !ui.input(|i| i.pointer.primary_down()) {
                ui.ctx().data_mut(|data| {
                    data.remove::<CanvasDrag>(drag_id);
                    data.remove::<CanvasPress>(press_id);
                });
                state.canvas_drag_target = None;
            }
        });
    if let Some((tape, label)) = ruler_label {
        draw_rulers(
            ui.painter(),
            canvas_rect,
            scroll.inner_rect,
            tape,
            label,
            dpi,
            state.zoom,
        );
        // The scale and the tape share the same physical band boundaries.
        let y_clip = egui::Rect::from_min_max(
            egui::pos2(canvas_rect.left(), scroll.inner_rect.top()),
            egui::pos2(scroll.inner_rect.left(), scroll.inner_rect.bottom()),
        );
        let band_painter = ui.painter().with_clip_rect(y_clip);
        for y in [label.top(), label.bottom()] {
            band_painter.line_segment(
                [
                    egui::pos2(scroll.inner_rect.left() - 10.0, y),
                    egui::pos2(scroll.inner_rect.left(), y),
                ],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(30, 110, 160)),
            );
        }
    }
    if previous_hover != state.canvas_hover_target || previous_drag != state.canvas_drag_target {
        ui.ctx().request_repaint();
    }
    if can_drag {
        keyboard_move(ui, state, dpi);
    }
}

#[derive(Clone)]
struct CanvasPress {
    target: Option<CanvasTarget>,
    action: CanvasAction,
    drag: Option<CanvasDrag>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CanvasAction {
    Move,
    Resize,
    Rotate,
    Gap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CanvasHit {
    target: CanvasTarget,
    action: CanvasAction,
}

struct CanvasGeometry {
    line: bool,
    element: usize,
    slot: egui::Rect,
    object: Option<[egui::Pos2; 4]>,
    has_slot: bool,
    relative: bool,
}

/// Independent objects have exact physical edit geometry. Raster bounds stay
/// unchanged: rounding a width and rounding two edges can differ by one dot.
fn editing_geometry(
    state: &AppState,
    origin: egui::Pos2,
    bound: &ElementBounds,
) -> Option<CanvasGeometry> {
    let index = bound.element_index;
    let element = state.elements.get(index)?;
    let mut slot = screen_bounds(origin, state.zoom, bound);
    let mut object = state
        .preview_content_bounds
        .iter()
        .find(|g| g.element_index == index)
        .map(|g| {
            g.corners
                .map(|p| origin + egui::vec2(p[0], p[1]) * state.zoom)
        });
    if let Some(item) = element.as_object()
        && let Some(content) = &item.content_box
        && let Some((x, y, w, h)) = state.element_rect_mm(index)
    {
        let ppm = f64::from(state.target_dpi()) / 25.4;
        let tape_width = state
            .preview_bitmap
            .as_ref()
            .map_or(0.0, |b| f64::from(b.width()));
        let screen = |mut x: f64, mut y: f64| {
            x *= ppm;
            y *= ppm;
            if state.overall_flip_h {
                x = tape_width - x;
            }
            if state.overall_flip_v {
                y = f64::from(state.tape_width_px) - y;
            }
            origin + egui::vec2(x as f32, y as f32) * state.zoom
        };
        slot = egui::Rect::from_two_pos(screen(x, y), screen(x + w, y + h));
        let center = [x + w / 2.0, y + h / 2.0];
        let (sin, cos) = f64::from(item.rotation).to_radians().sin_cos();
        object = Some(
            [[-0.5, -0.5], [0.5, -0.5], [0.5, 0.5], [-0.5, 0.5]].map(|[dx, dy]| {
                let dx = dx * content.width_mm;
                let dy = dy * content.height_mm;
                let mut px = dx * cos - dy * sin + content.offset_x_mm;
                let mut py = dx * sin + dy * cos + content.offset_y_mm;
                if item.flip_h {
                    px = -px;
                }
                if item.flip_v {
                    py = -py;
                }
                screen(center[0] + px, center[1] + py)
            }),
        );
    }
    if object.is_none() && element.as_object().is_none() {
        object = Some(rect_quad(slot));
    }
    Some(CanvasGeometry {
        line: matches!(
            element.as_object().map(|o| &o.content),
            Some(ptouch_render::document::ObjectContent::Line { .. })
        ),
        element: index,
        slot,
        object,
        relative: element
            .as_object()
            .is_some_and(|o| o.frame.position == PositionMode::Relative),
        has_slot: matches!(
            element,
            crate::state::LabelElement::Object(_)
                | crate::state::LabelElement::Text {
                    text_box: Some(_),
                    ..
                }
        ) && !element
            .as_object()
            .and_then(|o| o.layout.as_ref())
            .is_some_and(|p| p.placement == ptouch_render::object_layout::Placement::Free),
    })
}

/// Omit only coincident edges, not merely nearby shapes at a low zoom.
#[cfg(test)]
fn same_edge(a: egui::Pos2, b: egui::Pos2, c: egui::Pos2, d: egui::Pos2) -> bool {
    let close = |x: egui::Pos2, y: egui::Pos2| x.distance_sq(y) < 0.0001;
    close(a, c) && close(b, d) || close(a, d) && close(b, c)
}

fn target_quad(geometry: &[CanvasGeometry], target: CanvasTarget) -> Option<[egui::Pos2; 4]> {
    let g = geometry.iter().find(|g| g.element == target.element)?;
    match target.edit {
        CanvasEditTarget::Content => g.object,
        CanvasEditTarget::Slot => g.has_slot.then(|| rect_quad(g.slot)),
    }
}

fn resize_position(quad: [egui::Pos2; 4], target: CanvasTarget, flip_v: bool) -> egui::Pos2 {
    if target.edit == CanvasEditTarget::Slot && flip_v {
        quad[1]
    } else {
        quad[2]
    }
}

fn gap_position(quad: [egui::Pos2; 4]) -> egui::Pos2 {
    quad[0].lerp(quad[3], 0.5) - egui::vec2(14.0, 0.0)
}

fn canvas_hit(
    pos: egui::Pos2,
    _paper: egui::Rect,
    clip: egui::Rect,
    geometry: &[CanvasGeometry],
    selected: Option<CanvasTarget>,
    can_drag: bool,
    flip_v: bool,
) -> Option<CanvasHit> {
    if !clip.contains(pos) {
        return None;
    }
    if can_drag
        && let Some(target) = selected
        && let Some(quad) = target_quad(geometry, target)
    {
        let action = if target.edit == CanvasEditTarget::Content
            && rotation_handle(quad).distance(pos) <= ROTATE_RADIUS + 4.0
        {
            Some(CanvasAction::Rotate)
        } else if target.edit == CanvasEditTarget::Slot
            && geometry
                .iter()
                .any(|g| g.element == target.element && g.relative)
            && egui::Rect::from_center_size(gap_position(quad), egui::vec2(12.0, 12.0))
                .contains(pos)
        {
            Some(CanvasAction::Gap)
        } else if egui::Rect::from_center_size(
            resize_position(quad, target, flip_v),
            egui::vec2(12.0, 12.0),
        )
        .contains(pos)
        {
            Some(CanvasAction::Resize)
        } else {
            None
        };
        if let Some(action) = action {
            return Some(CanvasHit { target, action });
        }
        // Layout editing is entered explicitly in properties. Its interior is
        // then a layout drag surface, never an implicit second selection.
        if target.edit == CanvasEditTarget::Slot && quad_contains(quad, pos) {
            return Some(CanvasHit {
                target,
                action: CanvasAction::Move,
            });
        }
        // The selected dashed border doubles as a move handle. It keeps
        // fully covered slots and lower overlapping objects mouse-accessible.
        if (0..4).any(|i| {
            let a = quad[i];
            let edge = quad[(i + 1) % 4] - a;
            let t = ((pos - a).dot(edge) / edge.length_sq().max(f32::EPSILON)).clamp(0.0, 1.0);
            (a + edge * t).distance(pos) <= 4.0
        }) {
            return Some(CanvasHit {
                target,
                action: CanvasAction::Move,
            });
        }
    }
    if let Some(g) = geometry.iter().rev().find(|g| {
        g.object
            .is_some_and(|q| quad_contains(q, pos) || (g.line && quad_edge_distance(q, pos) <= 4.0))
    }) {
        return Some(CanvasHit {
            target: CanvasTarget {
                element: g.element,
                edit: CanvasEditTarget::Content,
            },
            action: CanvasAction::Move,
        });
    }
    None
}

fn draw_targets(
    painter: &egui::Painter,
    state: &AppState,
    geometry: &[CanvasGeometry],
    can_drag: bool,
) {
    let selected = state.selected_canvas_target();
    let hovered = state.canvas_hover_target;
    let blue = egui::Color32::from_rgb(0, 90, 190);
    let orange = egui::Color32::from_rgb(205, 115, 0);
    for target in [selected, hovered.filter(|target| Some(*target) != selected)]
        .into_iter()
        .flatten()
        .filter(|t| t.edit == CanvasEditTarget::Slot)
    {
        for index in state.slot_related(target.element) {
            if let Some(g) = geometry.iter().find(|g| g.element == index) {
                dashed_quad(
                    painter,
                    rect_quad(g.slot),
                    egui::Stroke::new(1.0, orange.gamma_multiply(0.28)),
                );
            }
        }
        if let Some(quad) = target_quad(
            geometry,
            CanvasTarget {
                edit: CanvasEditTarget::Content,
                ..target
            },
        ) {
            dashed_quad(
                painter,
                quad,
                egui::Stroke::new(1.0, blue.gamma_multiply(0.4)),
            );
        }
    }
    if let Some(target) = hovered.filter(|t| Some(*t) != selected)
        && let Some(quad) = target_quad(geometry, target)
    {
        let color = if target.edit == CanvasEditTarget::Slot {
            orange
        } else {
            blue
        };
        dashed_quad(painter, quad, egui::Stroke::new(1.5, color));
    }
    if let Some(target) = selected
        && let Some(quad) = target_quad(geometry, target)
    {
        let color = if target.edit == CanvasEditTarget::Slot {
            orange
        } else {
            blue
        };
        dashed_quad(painter, quad, egui::Stroke::new(2.0, color));
        if can_drag {
            painter.rect_filled(
                egui::Rect::from_center_size(
                    resize_position(quad, target, state.overall_flip_v),
                    egui::vec2(10.0, 10.0),
                ),
                1.0,
                color,
            );
            if target.edit == CanvasEditTarget::Content {
                let center = rotation_handle(quad);
                painter.line_segment([quad[1], center], egui::Stroke::new(1.5, blue));
                painter.circle_filled(center, ROTATE_RADIUS, egui::Color32::WHITE);
                painter.circle_stroke(center, ROTATE_RADIUS, egui::Stroke::new(2.0, blue));
            } else if state.elements[target.element]
                .as_object()
                .is_some_and(|o| o.frame.position == PositionMode::Relative)
            {
                let center = gap_position(quad);
                painter.line_segment(
                    [quad[0].lerp(quad[3], 0.5), center],
                    egui::Stroke::new(1.0, orange),
                );
                painter.rect_filled(
                    egui::Rect::from_center_size(center, egui::vec2(10.0, 8.0)),
                    1.0,
                    orange,
                );
                painter.text(
                    center,
                    egui::Align2::CENTER_CENTER,
                    "↔",
                    egui::FontId::proportional(10.0),
                    egui::Color32::WHITE,
                );
            }
        }
    }
}

// Clip before expanding dashes: a long label at high zoom must cost only
// visible perimeter, not millions of offscreen segments.
fn clipped_segment(a: egui::Pos2, b: egui::Pos2, clip: egui::Rect) -> Option<[egui::Pos2; 2]> {
    let delta = b - a;
    let mut start = 0.0_f32;
    let mut end = 1.0_f32;
    for (origin, direction, min, max) in [
        (a.x, delta.x, clip.min.x, clip.max.x),
        (a.y, delta.y, clip.min.y, clip.max.y),
    ] {
        if direction.abs() < f32::EPSILON {
            if origin < min || origin > max {
                return None;
            }
        } else {
            let t1 = (min - origin) / direction;
            let t2 = (max - origin) / direction;
            start = start.max(t1.min(t2));
            end = end.min(t1.max(t2));
            if start > end {
                return None;
            }
        }
    }
    Some([a + delta * start, a + delta * end])
}

fn dashed_quad(painter: &egui::Painter, quad: [egui::Pos2; 4], stroke: egui::Stroke) {
    let clip = painter.clip_rect().expand(stroke.width);
    let edges = (0..4)
        .map(|i| {
            let a = quad[i];
            let b = quad[(i + 1) % 4];
            let shapes = clipped_segment(a, b, clip).map_or_else(Vec::new, |line| {
                egui::Shape::dashed_line(&line, stroke, 6.0, 4.0)
            });
            egui::Shape::Vec(shapes)
        })
        .collect();
    painter.add(egui::Shape::Vec(edges));
}

fn dragged_frame(
    drag: &CanvasDrag,
    delta: egui::Vec2,
    height_mm: f64,
    snap: bool,
    minimum: f64,
) -> TextBox {
    let mut frame = drag.frame.clone();
    let dx = f64::from(delta.x) / drag.pixels_per_mm * if drag.flip_h { -1.0 } else { 1.0 };
    let dy = f64::from(delta.y) / drag.pixels_per_mm * if drag.flip_v { -1.0 } else { 1.0 };
    let quantize = |value: f64| {
        if snap {
            (value * 10.0).round() / 10.0
        } else {
            value
        }
    };
    if drag.resize {
        frame.width_mm = quantize(frame.width_mm + dx).clamp(minimum, 10_000.0);
        frame.height_mm =
            quantize(frame.height_mm + dy).clamp(minimum, (height_mm - frame.y_mm).max(minimum));
    } else {
        let left = quantize(drag.left_mm + dx).max(0.0);
        if frame.position == PositionMode::Relative {
            let gap = left - (drag.left_mm - drag.frame.gap_mm);
            frame.gap_mm = gap.max(0.0);
        } else {
            frame.x_mm = left;
        }
        frame.y_mm = quantize(frame.y_mm + dy).clamp(0.0, (height_mm - frame.height_mm).max(0.0));
    }
    frame
}

fn keyboard_move(ui: &egui::Ui, state: &mut AppState, dpi: u16) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let delta = ui.input(|i| {
        let step = if i.modifiers.shift { 1.0 } else { 0.1 };
        egui::vec2(
            (i.key_pressed(egui::Key::ArrowRight) as i8 - i.key_pressed(egui::Key::ArrowLeft) as i8)
                as f32
                * step,
            (i.key_pressed(egui::Key::ArrowDown) as i8 - i.key_pressed(egui::Key::ArrowUp) as i8)
                as f32
                * step,
        )
    });
    if delta == egui::Vec2::ZERO {
        return;
    }
    if let Some(index) = state.selected_element
        && let Some(object) = state.elements[index].as_object()
    {
        if state.canvas_edit_target == CanvasEditTarget::Content {
            let before = object.clone();
            let flip_h = object.flip_h ^ state.overall_flip_h;
            let flip_v = object.flip_v ^ state.overall_flip_v;
            let resolved = if before.content_box.is_none() {
                state.pin_content_box(index)
            } else {
                state.resolved_content_box(index)
            };
            if let Some(mut content) = resolved {
                content.offset_x_mm += f64::from(delta.x) * if flip_h { -1.0 } else { 1.0 };
                content.offset_y_mm += f64::from(delta.y) * if flip_v { -1.0 } else { 1.0 };
                let mut candidate = before.clone();
                candidate.content_box = Some(content);
                match state.constrain_transform(index, &before, &mut candidate) {
                    Err(message) => state.status_message = message.into(),
                    Ok(clamped) => {
                        state.transform_feedback(
                            clamped,
                            candidate.layout.as_ref().is_some_and(|p| {
                                p.placement == ptouch_render::object_layout::Placement::Free
                            }),
                        );
                        if candidate != before {
                            state.elements[index] = crate::state::LabelElement::Object(candidate);
                            state.document_version = state.document_version.max(4);
                            state.mark_dirty();
                        }
                    }
                }
            }
            return;
        }
        let drag = CanvasDrag {
            before: None,
            gap: false,
            scroll_offset: egui::Vec2::ZERO,
            reorder: None,
            element: index,
            start: egui::Pos2::ZERO,
            frame: object.frame.clone(),
            left_mm: state
                .element_rect_mm(index)
                .map_or(object.frame.x_mm, |r| r.0),
            pixels_per_mm: 1.0,
            resize: false,
            pointer_moved: false,
            flip_h: state.overall_flip_h,
            flip_v: state.overall_flip_v,
            rotation: None,
            content: None,
            font_size: None,
            font_styles: Vec::new(),
        };
        let frame = dragged_frame(
            &drag,
            delta,
            f64::from(state.tape_width_px) * 25.4 / f64::from(dpi),
            true,
            25.4 / f64::from(dpi),
        );
        state.elements[index].as_object_mut().unwrap().frame = frame;
        state.mark_dirty();
    }
}

fn fit_zoom(original_size: egui::Vec2, canvas_size: egui::Vec2) -> f32 {
    let available = canvas_size
        - egui::vec2(
            PADDING * 2.0 + RULER_WIDTH + HANDLE_MARGIN + 12.0,
            PADDING * 2.0 + RULER_HEIGHT + 12.0,
        );
    (available.x.max(1.0) / original_size.x)
        .min(available.y.max(1.0) / original_size.y)
        .clamp(f32::EPSILON, 10.0)
}

fn screen_bounds(origin: egui::Pos2, zoom: f32, bound: &ElementBounds) -> egui::Rect {
    egui::Rect::from_min_size(
        origin + egui::vec2(bound.x as f32, bound.y as f32) * zoom,
        egui::vec2(bound.width as f32, bound.height as f32) * zoom,
    )
}

fn rect_quad(rect: egui::Rect) -> [egui::Pos2; 4] {
    [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ]
}

fn quad_center(quad: [egui::Pos2; 4]) -> egui::Pos2 {
    (quad[0].to_vec2() + quad[1].to_vec2() + quad[2].to_vec2() + quad[3].to_vec2()).to_pos2() / 4.0
}

fn rotation_handle(quad: [egui::Pos2; 4]) -> egui::Pos2 {
    quad[1] + (quad[1] - quad_center(quad)).normalized() * 16.0
}

fn quad_edge_distance(quad: [egui::Pos2; 4], point: egui::Pos2) -> f32 {
    (0..4)
        .map(|i| {
            let a = quad[i];
            let edge = quad[(i + 1) % 4] - a;
            let t = ((point - a).dot(edge) / edge.length_sq().max(f32::EPSILON)).clamp(0.0, 1.0);
            (a + edge * t).distance(point)
        })
        .fold(f32::INFINITY, f32::min)
}

fn quad_contains(quad: [egui::Pos2; 4], point: egui::Pos2) -> bool {
    let mut positive = false;
    let mut negative = false;
    for i in 0..4 {
        let edge = quad[(i + 1) % 4] - quad[i];
        let delta = point - quad[i];
        let cross = edge.x * delta.y - edge.y * delta.x;
        positive |= cross > 0.01;
        negative |= cross < -0.01;
    }
    !(positive && negative)
}

fn resize_cursor(direction: egui::Vec2) -> egui::CursorIcon {
    let sector = ((direction.angle().to_degrees().rem_euclid(180.0) + 22.5) / 45.0) as u32 % 4;
    match sector {
        0 => egui::CursorIcon::ResizeHorizontal,
        1 => egui::CursorIcon::ResizeNwSe,
        2 => egui::CursorIcon::ResizeVertical,
        _ => egui::CursorIcon::ResizeNeSw,
    }
}

fn dragged_content(
    drag: &CanvasDrag,
    content: &ContentDrag,
    delta: egui::Vec2,
    snap: bool,
    minimum: f64,
) -> ContentBox {
    let mut result = content.original.clone();
    let mut shift = delta;
    if drag.resize {
        // Project onto the object's own diagonal, preserving its aspect ratio
        // and anchoring its opposite corner, regardless of rotation/mirroring.
        let diagonal = content.corners[2] - content.corners[0];
        let mut scale = f64::from((diagonal + delta).dot(diagonal) / diagonal.length_sq().max(1.0));
        if snap {
            scale = (result.width_mm * scale * 10.0).round() / 10.0 / result.width_mm;
        }
        scale = scale.clamp(
            (minimum / result.width_mm).max(minimum / result.height_mm),
            (10_000.0 / result.width_mm).min(10_000.0 / result.height_mm),
        );
        result.width_mm *= scale;
        result.height_mm *= scale;
        shift = diagonal * ((scale - 1.0) as f32 / 2.0);
    }
    let quantize = |value: f64| {
        if snap && !drag.resize {
            (value * 10.0).round() / 10.0
        } else {
            value
        }
    };
    result.offset_x_mm = quantize(
        result.offset_x_mm
            + f64::from(shift.x) / drag.pixels_per_mm * if drag.flip_h { -1.0 } else { 1.0 },
    );
    result.offset_y_mm = quantize(
        result.offset_y_mm
            + f64::from(shift.y) / drag.pixels_per_mm * if drag.flip_v { -1.0 } else { 1.0 },
    );
    result
}

/// Accumulate wrapped pointer steps so crossing ±180° and multiple turns are
/// continuous. Mirroring reverses the visible angular direction of content.
fn dragged_rotation(
    drag: &mut RotationDrag,
    pointer: egui::Pos2,
    center: egui::Pos2,
    snap: bool,
) -> Option<f32> {
    // Measure both pointer positions around the current frame center. Diagnostics
    // or window resizing can move the canvas during a gesture; a stationary
    // pointer must not rotate content when that happens.
    let previous = drag.last_pointer - center;
    let vector = pointer - center;
    drag.last_pointer = pointer;
    if vector.length_sq() < 16.0 || previous.length_sq() < 16.0 {
        return None;
    }
    let step = (vector.angle() - previous.angle() + std::f32::consts::PI)
        .rem_euclid(std::f32::consts::TAU)
        - std::f32::consts::PI;
    drag.accumulated += step.to_degrees();
    let value = drag.initial + drag.accumulated * if drag.reverse { -1.0 } else { 1.0 };
    Some(
        (if snap {
            (value / 15.0).round() * 15.0
        } else {
            value
        })
        .rem_euclid(360.0),
    )
}

struct RulerTick {
    screen: f32,
    mm: f32,
    major: bool,
}

/// Use original raster coordinates, even if the preview texture was reduced.
fn ruler_ticks(
    visible_start: f32,
    visible_end: f32,
    origin: f32,
    length: f32,
    pixels_per_mm: f32,
    label_spacing: f32,
) -> (f32, Vec<RulerTick>) {
    let step = ruler_step(pixels_per_mm * 55.0 / label_spacing);
    let minor = step / 5.0;
    let start = (((visible_start - origin) / pixels_per_mm / minor).floor() as i64).max(0);
    let end = (((visible_end.min(origin + length) - origin) / pixels_per_mm / minor).ceil() as i64)
        .max(-1);
    let ticks = (start..=end)
        .filter_map(|tick| {
            let mm = tick as f32 * minor;
            let screen = origin + mm * pixels_per_mm;
            (screen >= visible_start && screen <= visible_end && screen <= origin + length + 0.01)
                .then_some(RulerTick {
                    screen,
                    mm,
                    major: tick % 5 == 0,
                })
        })
        .collect();
    (step, ticks)
}

fn draw_rulers(
    painter: &egui::Painter,
    canvas: egui::Rect,
    viewport: egui::Rect,
    label: egui::Rect,
    printable: egui::Rect,
    dpi: u16,
    zoom: f32,
) {
    let pixels_per_mm = f32::from(dpi) / 25.4 * zoom;
    let color = egui::Color32::from_gray(65);
    let font = egui::FontId::monospace(11.0);
    // Fixed gutters stay visible while the label scrolls in either direction.
    // Only tick coordinates follow the scrolled label's original pixel origin.
    let x_band = egui::Rect::from_min_max(
        egui::pos2(viewport.left(), canvas.top()),
        egui::pos2(viewport.right(), viewport.top()),
    );
    let y_band = egui::Rect::from_min_max(
        egui::pos2(canvas.left(), viewport.top()),
        egui::pos2(viewport.left(), viewport.bottom()),
    );
    let x_painter = painter.with_clip_rect(painter.clip_rect().intersect(x_band));
    let y_painter = painter.with_clip_rect(painter.clip_rect().intersect(y_band));
    y_painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(y_band.left(), label.top()),
            egui::pos2(y_band.right(), label.bottom()),
        ),
        0.0,
        egui::Color32::from_gray(150),
    );
    y_painter.rect_filled(
        egui::Rect::from_min_max(
            egui::pos2(y_band.left(), printable.top()),
            egui::pos2(y_band.right(), printable.bottom()),
        ),
        0.0,
        egui::Color32::from_gray(235),
    );
    let text = |mm: f32, step: f32| {
        if step < 1.0 {
            format!("{mm:.1}")
        } else {
            format!("{mm:.0}")
        }
    };
    let x_baseline = viewport.top() - 3.0;
    let y_baseline = viewport.left() - 3.0;
    let (x_step, x_ticks) = ruler_ticks(
        viewport.left(),
        viewport.right(),
        label.left(),
        label.width(),
        pixels_per_mm,
        55.0,
    );
    for tick in x_ticks {
        x_painter.line_segment(
            [
                egui::pos2(tick.screen, x_baseline),
                egui::pos2(tick.screen, x_baseline - if tick.major { 9.0 } else { 4.0 }),
            ],
            egui::Stroke::new(1.0, color),
        );
        if tick.major {
            x_painter.text(
                egui::pos2(tick.screen, x_baseline - 11.0),
                egui::Align2::CENTER_BOTTOM,
                text(tick.mm, x_step),
                font.clone(),
                color,
            );
        }
    }
    if label.right() > viewport.left() && label.left() < viewport.right() {
        x_painter.line_segment(
            [
                egui::pos2(label.left().max(viewport.left()), x_baseline),
                egui::pos2(label.right().min(viewport.right()), x_baseline),
            ],
            egui::Stroke::new(1.0, color),
        );
    }
    let (y_step, y_ticks) = ruler_ticks(
        viewport.top(),
        viewport.bottom(),
        label.top(),
        label.height(),
        pixels_per_mm,
        28.0,
    );
    let end_visible = label.bottom() >= viewport.top() && label.bottom() + 7.0 <= viewport.bottom();
    for tick in y_ticks {
        y_painter.line_segment(
            [
                egui::pos2(y_baseline, tick.screen),
                egui::pos2(y_baseline - if tick.major { 9.0 } else { 4.0 }, tick.screen),
            ],
            egui::Stroke::new(1.0, color),
        );
        if tick.major && (!end_visible || label.bottom() - tick.screen > 17.0) {
            y_painter.text(
                egui::pos2(y_baseline - 12.0, tick.screen),
                egui::Align2::RIGHT_CENTER,
                text(tick.mm, y_step),
                font.clone(),
                color,
            );
        }
    }
    if label.bottom() > viewport.top() && label.top() < viewport.bottom() {
        y_painter.line_segment(
            [
                egui::pos2(y_baseline, label.top().max(viewport.top())),
                egui::pos2(y_baseline, label.bottom().min(viewport.bottom())),
            ],
            egui::Stroke::new(1.0, color),
        );
    }
    if end_visible {
        y_painter.line_segment(
            [
                egui::pos2(y_baseline, label.bottom()),
                egui::pos2(y_baseline - 9.0, label.bottom()),
            ],
            egui::Stroke::new(1.0, color),
        );
        y_painter.text(
            egui::pos2(y_baseline - 12.0, label.bottom()),
            egui::Align2::RIGHT_CENTER,
            format!("{:.2}", label.height() / pixels_per_mm),
            font.clone(),
            color,
        );
    }
    painter.text(
        egui::pos2(
            canvas.left() + RULER_WIDTH / 2.0,
            canvas.top() + RULER_HEIGHT / 2.0,
        ),
        egui::Align2::CENTER_CENTER,
        "mm",
        font,
        color,
    );
}

/// Keep numbers legible at every zoom, including very long labels in Fit mode.
fn ruler_step(pixels_per_mm: f32) -> f32 {
    let desired = 55.0 / pixels_per_mm;
    let base = 10.0_f32.powf(desired.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .into_iter()
        .map(|multiple| multiple * base)
        .find(|step| *step >= desired)
        .unwrap_or(base * 10.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thin_printed_lines_get_mouse_slop_without_expanding_their_geometry() {
        let paper = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(500.0, 200.0));
        let bounds = egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(80.0, 1.0));
        let mut geometry = [CanvasGeometry {
            element: 0,
            slot: bounds,
            object: Some(rect_quad(bounds)),
            line: true,
            has_slot: false,
            relative: false,
        }];
        let near = egui::pos2(140.0, 104.0);
        assert!(canvas_hit(near, paper, paper, &geometry, None, true, false).is_some());
        assert!(
            canvas_hit(
                egui::pos2(140.0, 106.0),
                paper,
                paper,
                &geometry,
                None,
                true,
                false
            )
            .is_none()
        );
        assert_eq!(geometry[0].object.unwrap(), rect_quad(bounds));
        geometry[0].line = false;
        assert!(canvas_hit(near, paper, paper, &geometry, None, true, false).is_none());
    }

    #[test]
    fn blank_layout_bands_never_implicitly_select_a_second_target() {
        let rect = |x, y, w, h| egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h));
        let geometry = vec![
            CanvasGeometry {
                line: false,
                element: 0,
                slot: rect(10.0, 10.0, 40.0, 20.0),
                object: Some([
                    egui::pos2(30.0, 10.0),
                    egui::pos2(50.0, 30.0),
                    egui::pos2(30.0, 50.0),
                    egui::pos2(10.0, 30.0),
                ]),
                has_slot: true,
                relative: false,
            },
            CanvasGeometry {
                line: false,
                element: 1,
                slot: rect(10.0, 60.0, 40.0, 20.0),
                object: None,
                has_slot: true,
                relative: false,
            },
            CanvasGeometry {
                line: false,
                element: 2,
                slot: rect(10.0, 60.0, 40.0, 20.0),
                object: None,
                has_slot: true,
                relative: false,
            },
        ];
        let paper = rect(0.0, 0.0, 100.0, 100.0);
        let hit = |x, y| {
            canvas_hit(
                egui::pos2(x, y),
                paper,
                paper.expand(20.0),
                &geometry,
                None,
                true,
                false,
            )
            .map(|h| h.target)
        };
        let target = |element, edit| Some(CanvasTarget { element, edit });
        assert_eq!(hit(30.0, 40.0), target(0, CanvasEditTarget::Content)); // content beyond slot
        assert_eq!(hit(11.0, 11.0), None); // outside rotated quad
        assert_eq!(hit(30.0, 95.0), None); // X band, not full rectangle
        assert_eq!(hit(11.0, 45.0), None); // nearest tie, reverse order
        assert_eq!(hit(30.0, 105.0), None); // gray workspace
        assert_eq!(hit(70.0, 20.0), None);
        let selected = CanvasTarget {
            element: 0,
            edit: CanvasEditTarget::Content,
        };
        let circle = rotation_handle(geometry[0].object.unwrap());
        let handle = canvas_hit(
            circle,
            paper,
            paper.expand(20.0),
            &geometry,
            Some(selected),
            true,
            false,
        )
        .unwrap();
        assert_eq!(handle.action, CanvasAction::Rotate);
        assert_eq!(handle.target, selected);
        assert!(
            canvas_hit(
                circle,
                paper,
                rect(70.0, 70.0, 20.0, 20.0),
                &geometry,
                Some(selected),
                true,
                false
            )
            .is_none()
        );
    }

    #[test]
    fn selected_slot_border_remains_draggable_when_object_covers_it() {
        let rect = egui::Rect::from_min_max(egui::pos2(10.0, 10.0), egui::pos2(90.0, 90.0));
        let geometry = [CanvasGeometry {
            line: false,
            element: 0,
            slot: rect,
            object: Some(rect_quad(rect)),
            has_slot: true,
            relative: false,
        }];
        let selected = CanvasTarget {
            element: 0,
            edit: CanvasEditTarget::Slot,
        };
        let paper = rect.expand(10.0);
        let hit = |pos, selected| {
            canvas_hit(pos, paper, paper, &geometry, selected, true, false).unwrap()
        };
        assert_eq!(hit(rect.left_center(), Some(selected)).target, selected);
        assert_eq!(
            hit(rect.center(), Some(selected)).target.edit,
            CanvasEditTarget::Slot
        );
        assert_eq!(
            hit(rect.left_center(), None).target.edit,
            CanvasEditTarget::Content
        );
        assert_eq!(
            hit(rect.right_bottom(), Some(selected)).action,
            CanvasAction::Resize
        );
    }

    #[test]
    fn selected_object_border_keeps_lower_overlapping_objects_accessible() {
        let rect = egui::Rect::from_min_max(egui::pos2(10.0, 10.0), egui::pos2(90.0, 90.0));
        let geometry: Vec<_> = (0..2)
            .map(|element| CanvasGeometry {
                line: false,
                element,
                slot: rect,
                object: Some(rect_quad(rect)),
                has_slot: true,
                relative: false,
            })
            .collect();
        let selected = CanvasTarget {
            element: 0,
            edit: CanvasEditTarget::Content,
        };
        let paper = rect.expand(20.0);
        let hit =
            |pos| canvas_hit(pos, paper, paper, &geometry, Some(selected), true, false).unwrap();
        assert_eq!(hit(rect.left_center()).target, selected);
        assert_eq!(hit(rect.center()).target.element, 1);
        assert_eq!(
            hit(rotation_handle(rect_quad(rect))).action,
            CanvasAction::Rotate
        );
        assert_eq!(hit(rect.right_bottom()).action, CanvasAction::Resize);
    }

    #[test]
    fn hover_and_fast_click_do_not_pin_legacy_objects_or_change_keyboard_target() {
        let context = egui::Context::default();
        let mut state = canvas_fixture(&context, false, 0);
        state.elements[0].as_object_mut().unwrap().content_box = None;
        state.document_version = 3;
        state.needs_rerender = false;
        state.selected_element = None;
        let before = state.document().to_toml_string().unwrap();
        let idle = canvas_event(&context, &mut state, vec![]);
        assert!(
            !idle
                .shapes
                .iter()
                .any(|s| matches!(s.shape, egui::Shape::Vec(_)))
        );
        // Use rendered geometry with the actual paper origin from the output image.
        let paper = idle
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => {
                    Some(mesh.calc_bounds())
                }
                _ => None,
            })
            .unwrap();
        let g = &state.preview_content_bounds[0];
        let quad = g
            .corners
            .map(|p| paper.min + egui::vec2(p[0], p[1]) * state.zoom);
        let center = quad_center(quad);
        let hover = canvas_event(
            &context,
            &mut state,
            vec![egui::Event::PointerMoved(center)],
        );
        assert_eq!(
            state.canvas_hover_target,
            Some(CanvasTarget {
                element: 0,
                edit: CanvasEditTarget::Content
            })
        );
        assert_eq!(state.selected_element, None);
        assert!(
            !hover
                .shapes
                .iter()
                .any(|s| matches!(&s.shape,egui::Shape::Circle(c) if c.radius==ROTATE_RADIUS))
        );
        assert_eq!(state.document().to_toml_string().unwrap(), before);
        canvas_event(
            &context,
            &mut state,
            vec![pointer_button(center, true), pointer_button(center, false)],
        );
        assert_eq!(state.selected_element, Some(0));
        assert_eq!(state.document().to_toml_string().unwrap(), before);
        assert!(!state.needs_rerender);
        let blank = paper.max + egui::vec2(15.0, 15.0);
        canvas_event(
            &context,
            &mut state,
            vec![
                egui::Event::PointerMoved(blank),
                pointer_button(blank, true),
                pointer_button(blank, false),
            ],
        );
        assert_eq!(state.selected_element, None);
        assert_eq!(state.document().to_toml_string().unwrap(), before);
    }

    #[test]
    fn slot_press_stays_slot_across_object_and_empty_preview_clears_drag() {
        let context = egui::Context::default();
        let mut state = canvas_fixture(&context, false, 0);
        state.canvas_edit_target = CanvasEditTarget::Slot;
        let output = canvas_event(&context, &mut state, vec![]);
        let slot = selected_slot(&output);
        let start = slot.left_center();
        let content = state.elements[0].as_object().unwrap().content_box.clone();
        canvas_event(
            &context,
            &mut state,
            vec![
                egui::Event::PointerMoved(start),
                pointer_button(start, true),
            ],
        );
        let end = slot.center();
        canvas_event(&context, &mut state, vec![egui::Event::PointerMoved(end)]);
        assert_eq!(
            state.canvas_drag_target,
            Some(CanvasTarget {
                element: 0,
                edit: CanvasEditTarget::Slot
            })
        );
        assert_eq!(state.elements[0].as_object().unwrap().content_box, content);
        assert!(state.elements[0].as_object().unwrap().frame.x_mm > 5.0);
        state.preview_content_bounds.clear();
        state.preview_bitmap = None;
        state.preview_texture = None;
        canvas_event(&context, &mut state, vec![pointer_button(end, false)]);
        assert_eq!(state.canvas_drag_target, None);
        assert_eq!(state.canvas_hover_target, None);
    }

    #[test]
    fn holding_without_moving_never_pins_or_quantizes_but_later_motion_works() {
        for slot in [false, true] {
            for legacy in [false, true] {
                let context = egui::Context::default();
                let mut state = canvas_fixture(&context, false, 0);
                let object = state.elements[0].as_object_mut().unwrap();
                object.frame.x_mm = 5.123;
                if legacy {
                    object.content_box = None;
                    state.document_version = 3;
                } else {
                    object.content_box.as_mut().unwrap().offset_x_mm = 0.123;
                }
                state.canvas_edit_target = if slot {
                    CanvasEditTarget::Slot
                } else {
                    CanvasEditTarget::Content
                };
                state.needs_rerender = false;
                let before = state.document().to_toml_string().unwrap();
                let output = canvas_event(&context, &mut state, vec![]);
                let quad = selected_quad(&output);
                let start = if slot {
                    (quad[0] + quad[3].to_vec2()) / 2.0
                } else {
                    quad_center(quad)
                };
                canvas_event(
                    &context,
                    &mut state,
                    vec![
                        egui::Event::PointerMoved(start),
                        pointer_button(start, true),
                    ],
                );
                let time = context.input(|i| i.time) + 2.0;
                let _ = context.run_ui(
                    egui::RawInput {
                        time: Some(time),
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800.0, 600.0),
                        )),
                        ..Default::default()
                    },
                    |ui| show_canvas(ui, &mut state),
                );
                assert_eq!(state.document().to_toml_string().unwrap(), before);
                assert!(!state.needs_rerender);
                canvas_event(
                    &context,
                    &mut state,
                    vec![egui::Event::PointerMoved(start + egui::vec2(20.0, 0.0))],
                );
                assert_ne!(state.document().to_toml_string().unwrap(), before);
                assert!(state.needs_rerender);
            }
        }
    }

    #[test]
    fn pending_press_keeps_fit_scale_when_properties_panel_changes_width() {
        let context = egui::Context::default();
        let mut state = canvas_fixture(&context, false, 0);
        state.zoom_fit = true;
        let output = canvas_event(&context, &mut state, vec![]);
        let quad = selected_quad(&output);
        let start = quad_center(quad);
        canvas_event(
            &context,
            &mut state,
            vec![
                egui::Event::PointerMoved(start),
                pointer_button(start, true),
            ],
        );
        let zoom = state.zoom;
        let _ = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(700.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| show_canvas(ui, &mut state),
        );
        assert_eq!(state.zoom, zoom);
    }

    #[test]
    fn clipping_long_dashed_edges_is_bounded_by_visible_viewport() {
        let clip = egui::Rect::from_min_max(egui::pos2(10.0, 10.0), egui::pos2(110.0, 110.0));
        let line = clipped_segment(
            egui::pos2(-1_000_000.0, 50.0),
            egui::pos2(1_000_000.0, 50.0),
            clip,
        )
        .unwrap();
        assert!((line[1] - line[0]).length() <= 101.0);
        assert!(
            clipped_segment(
                egui::pos2(-1_000_000.0, 0.0),
                egui::pos2(1_000_000.0, 0.0),
                clip
            )
            .is_none()
        );
    }

    fn drag(frame: TextBox, left_mm: f64) -> CanvasDrag {
        CanvasDrag {
            before: None,
            gap: false,
            scroll_offset: egui::Vec2::ZERO,
            reorder: None,
            element: 0,
            start: egui::Pos2::ZERO,
            frame,
            left_mm,
            pixels_per_mm: 20.0,
            resize: false,
            pointer_moved: false,
            flip_h: false,
            flip_v: false,
            rotation: None,
            content: None,
            font_size: None,
            font_styles: Vec::new(),
        }
    }

    fn canvas_event(
        context: &egui::Context,
        state: &mut AppState,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        if !state.preview_content_bounds.is_empty() {
            render_fixture(context, state);
        }
        let mut output = context.run_ui(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                events,
                ..Default::default()
            },
            |ui| show_canvas(ui, state),
        );
        output.textures_delta.clear();
        output
    }

    fn render_fixture(context: &egui::Context, state: &mut AppState) {
        thread_local! {
            static RENDERER: std::cell::RefCell<ptouch_render::text::TextRenderer> =
                std::cell::RefCell::new(ptouch_render::text::TextRenderer::new());
        }
        let rendered = RENDERER.with(|renderer| {
            ptouch_render::document::render_document(
                &state.document(),
                state.tape_width_px,
                state.target_dpi(),
                &mut renderer.borrow_mut(),
            )
            .unwrap()
        });
        state.preview_bounds = rendered.bounds;
        state.preview_content_bounds = rendered.content_bounds;
        state.preview_bitmap = rendered.bitmap;
        let bitmap = state.preview_bitmap.as_ref().unwrap().to_rgba_image();
        state.preview_texture = Some(context.load_texture(
            "gesture_fixture",
            egui::ColorImage::from_rgba_unmultiplied(
                [bitmap.width() as usize, bitmap.height() as usize],
                bitmap.as_raw(),
            ),
            Default::default(),
        ));
    }

    #[test]
    fn equal_physical_boxes_share_edit_edges_despite_raster_rounding() {
        use ptouch_render::document::{LabelElement, LabelObject};
        for dpi in [180, 360] {
            for mirrors in 0..16 {
                let context = egui::Context::default();
                let mut object = LabelObject::text(
                    "PORT",
                    TextBox {
                        x_mm: 17.3,
                        y_mm: 0.0,
                        width_mm: 14.5,
                        height_mm: 10.0,
                        ..Default::default()
                    },
                );
                object.content_box = Some(ContentBox {
                    width_mm: 14.5,
                    height_mm: 10.0,
                    offset_x_mm: 0.0,
                    offset_y_mm: 0.0,
                });
                object.flip_h = mirrors & 1 != 0;
                object.flip_v = mirrors & 2 != 0;
                let mut state = AppState {
                    document_version: 4,
                    design_dpi: dpi,
                    tape_width_px: if dpi == 180 { 76 } else { 150 },
                    elements: vec![LabelElement::Object(object)],
                    overall_flip_h: mirrors & 4 != 0,
                    overall_flip_v: mirrors & 8 != 0,
                    zoom: 6.4,
                    ..Default::default()
                };
                render_fixture(&context, &mut state);
                let geometry =
                    editing_geometry(&state, egui::pos2(100.0, 60.0), &state.preview_bounds[0])
                        .unwrap();
                let content = geometry.object.unwrap();
                let slot = rect_quad(geometry.slot);
                for edge in 0..4 {
                    assert!(
                        (0..4).any(|i| same_edge(
                            content[edge],
                            content[(edge + 1) % 4],
                            slot[i],
                            slot[(i + 1) % 4]
                        )),
                        "dpi={dpi}, mirrors={mirrors}"
                    );
                }
                // A genuinely narrower object must remain distinguishable.
                state.elements[0]
                    .as_object_mut()
                    .unwrap()
                    .content_box
                    .as_mut()
                    .unwrap()
                    .width_mm = 14.0;
                let narrower =
                    editing_geometry(&state, egui::Pos2::ZERO, &state.preview_bounds[0]).unwrap();
                assert!(
                    narrower
                        .object
                        .unwrap()
                        .iter()
                        .all(|p| p.x > narrower.slot.left() && p.x < narrower.slot.right())
                );
            }
        }
    }

    fn canvas_fixture(context: &egui::Context, image: bool, mirrors: u8) -> AppState {
        use ptouch_render::document::{LabelElement, LabelObject};
        let frame = TextBox {
            x_mm: 5.0,
            y_mm: 1.0,
            width_mm: 10.0,
            height_mm: 4.0,
            ..TextBox::default()
        };
        let mut object = if image {
            let mut bytes = std::io::Cursor::new(Vec::new());
            image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
                8,
                8,
                image::Rgba([0, 0, 0, 255]),
            ))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
            LabelObject::image_from_bytes(None, bytes.into_inner(), frame)
        } else {
            LabelObject::text("PORT 1", frame)
        };
        object.content_box = Some(ContentBox {
            width_mm: 7.0,
            height_mm: 3.0,
            offset_x_mm: 0.0,
            offset_y_mm: 1.0,
        });
        object.rotation = 30.0;
        object.flip_h = mirrors & 1 != 0;
        object.flip_v = mirrors & 2 != 0;
        let mut state = AppState {
            zoom_fit: false,
            zoom: 2.0,
            overall_flip_h: mirrors & 4 != 0,
            overall_flip_v: mirrors & 8 != 0,
            elements: vec![LabelElement::Object(object)],
            selected_element: Some(0),
            ..AppState::default()
        };
        render_fixture(context, &mut state);
        state
    }

    fn selected_quad(output: &egui::FullOutput) -> [egui::Pos2; 4] {
        output
            .shapes
            .iter()
            .find_map(|shape| {
                let egui::Shape::Vec(edges) = &shape.shape else {
                    return None;
                };
                let corners: Option<Vec<_>> = edges
                    .iter()
                    .map(|edge| {
                        let egui::Shape::Vec(dashes) = edge else {
                            return None;
                        };
                        match dashes.first()? {
                            egui::Shape::LineSegment { points, stroke }
                                if (stroke.color == egui::Color32::from_rgb(0, 90, 190)
                                    || stroke.color == egui::Color32::from_rgb(205, 115, 0))
                                    && stroke.width == 2.0 =>
                            {
                                Some(points[0])
                            }
                            _ => None,
                        }
                    })
                    .collect();
                corners?.try_into().ok()
            })
            .unwrap()
    }

    fn selected_quad_and_circle(output: &egui::FullOutput) -> ([egui::Pos2; 4], egui::Pos2) {
        let circle = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Circle(c)
                    if c.stroke.color == egui::Color32::from_rgb(0, 90, 190)
                        && c.radius == ROTATE_RADIUS =>
                {
                    Some(c.center)
                }
                _ => None,
            })
            .unwrap();
        (selected_quad(output), circle)
    }

    fn selected_slot(output: &egui::FullOutput) -> egui::Rect {
        let quad = selected_quad(output);
        egui::Rect::from_min_max(quad[0], quad[2])
    }

    fn pointer_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }

    #[test]
    fn round_handle_rotates_text_and_images_with_mirrors_and_release_position() {
        for image in [false, true] {
            for mirrors in [0_u8, 1, 2, 3, 8, 9, 10, 11] {
                let context = egui::Context::default();
                let mut state = canvas_fixture(&context, image, mirrors);
                let original = state.elements[0].as_object().unwrap().frame.clone();
                let original_content = state.elements[0].as_object().unwrap().content_box.clone();
                let output = canvas_event(&context, &mut state, vec![]);
                let (frame, circle) = selected_quad_and_circle(&output);
                // Clicking an external handle must not trigger blank-canvas deselection.
                canvas_event(
                    &context,
                    &mut state,
                    vec![
                        egui::Event::PointerMoved(circle),
                        pointer_button(circle, true),
                    ],
                );
                canvas_event(&context, &mut state, vec![pointer_button(circle, false)]);
                assert_eq!(state.selected_element, Some(0));
                canvas_event(&context, &mut state, vec![pointer_button(circle, true)]);
                let vector = circle - quad_center(frame);
                let moved = quad_center(frame)
                    + egui::emath::Rot2::from_angle(90_f32.to_radians()) * vector;
                canvas_event(&context, &mut state, vec![egui::Event::PointerMoved(moved)]);
                let released = quad_center(frame)
                    + egui::emath::Rot2::from_angle(105_f32.to_radians()) * vector;
                canvas_event(
                    &context,
                    &mut state,
                    vec![
                        egui::Event::PointerMoved(released),
                        pointer_button(released, false),
                    ],
                );
                let object = state.elements[0].as_object().unwrap();
                let expected = if mirrors.count_ones() % 2 == 0 {
                    135.0
                } else {
                    285.0
                };
                assert!(
                    (object.rotation - expected).abs() < 0.01,
                    "image={image} mirrors={mirrors}: {}",
                    object.rotation
                );
                assert_eq!(object.frame, original);
                assert_eq!(object.content_box, original_content);
                let saved = state.document().to_toml_string().unwrap();
                let reopened =
                    ptouch_render::document::LabelDocument::from_toml_str(&saved).unwrap();
                assert_eq!(reopened.elements[0].as_object().unwrap().frame, original);
                assert_eq!(
                    reopened.elements[0].as_object().unwrap().rotation,
                    object.rotation
                );
                let (rotated, new_circle) =
                    selected_quad_and_circle(&canvas_event(&context, &mut state, vec![]));
                assert!(new_circle.distance(circle) > 20.0);
                assert!(
                    ((rotated[1] - rotated[0]).length() - (frame[1] - frame[0]).length()).abs()
                        < 0.01
                );
            }
        }
    }

    #[test]
    fn fast_move_resize_and_rotation_finish_with_same_or_separate_press_frame() {
        for separate_press in [false, true] {
            for action in [
                CanvasAction::Move,
                CanvasAction::Resize,
                CanvasAction::Rotate,
            ] {
                let context = egui::Context::default();
                let mut state = canvas_fixture(&context, true, 0);
                let output = canvas_event(&context, &mut state, vec![]);
                let (quad, circle) = selected_quad_and_circle(&output);
                let center = quad_center(quad);
                let original = state.elements[0]
                    .as_object()
                    .unwrap()
                    .content_box
                    .clone()
                    .unwrap();
                let (start, end) = match action {
                    CanvasAction::Rotate => (
                        circle,
                        center + egui::vec2(-(circle - center).y, (circle - center).x),
                    ),
                    CanvasAction::Resize => (quad[2], quad[2] + (quad[2] - quad[0]) * 0.2),
                    _ => (center, center + egui::vec2(20.0, 10.0)),
                };
                let mut events = vec![
                    egui::Event::PointerMoved(start),
                    pointer_button(start, true),
                ];
                if separate_press {
                    canvas_event(&context, &mut state, std::mem::take(&mut events));
                }
                events.extend([egui::Event::PointerMoved(end), pointer_button(end, false)]);
                canvas_event(&context, &mut state, events);
                assert_eq!(state.selected_element, Some(0));
                assert!(state.canvas_drag_target.is_none());
                let object = state.elements[0].as_object().unwrap();
                let content = object.content_box.as_ref().unwrap();
                match action {
                    CanvasAction::Rotate => assert!(
                        (object.rotation - 120.0).abs() < 0.01,
                        "angle: {}",
                        object.rotation
                    ),
                    CanvasAction::Resize => assert!(content.width_mm > original.width_mm * 1.1),
                    _ => assert!(content.offset_x_mm > original.offset_x_mm),
                }
            }
        }
    }

    #[test]
    fn diagnostic_layout_changes_do_not_shift_rotation_angles() {
        let context = egui::Context::default();
        let mut state = canvas_fixture(&context, false, 0);
        let output = canvas_event(&context, &mut state, vec![]);
        let (frame, circle) = selected_quad_and_circle(&output);
        canvas_event(
            &context,
            &mut state,
            vec![
                egui::Event::PointerMoved(circle),
                pointer_button(circle, true),
            ],
        );
        let pointer = quad_center(frame)
            + egui::emath::Rot2::from_angle(45_f32.to_radians()) * (circle - quad_center(frame));
        canvas_event(
            &context,
            &mut state,
            vec![egui::Event::PointerMoved(pointer)],
        );
        assert!((state.elements[0].as_object().unwrap().rotation - 75.0).abs() < 0.01);
        state.preview_diagnostics = vec!["Fixed-size text was clipped".into()];
        let output = canvas_event(&context, &mut state, vec![]);
        let (shifted, _) = selected_quad_and_circle(&output);
        assert!(quad_center(shifted).y > quad_center(frame).y);
        assert!((state.elements[0].as_object().unwrap().rotation - 75.0).abs() < 0.01);
        let moved = quad_center(shifted)
            + egui::emath::Rot2::from_angle(30_f32.to_radians()) * (pointer - quad_center(shifted));
        let after = canvas_event(&context, &mut state, vec![egui::Event::PointerMoved(moved)]);
        assert!(
            (state.elements[0].as_object().unwrap().rotation - 105.0).abs() < 0.01,
            "rotation={} old={:?} new={:?}",
            state.elements[0].as_object().unwrap().rotation,
            shifted,
            selected_quad_and_circle(&after).0
        );
        state.preview_diagnostics.clear();
        canvas_event(&context, &mut state, vec![pointer_button(moved, false)]);
        assert!((state.elements[0].as_object().unwrap().rotation - 105.0).abs() < 0.01);
    }

    #[test]
    fn moving_and_resizing_still_work_beside_the_fixed_rulers() {
        for image in [false, true] {
            for resize in [false, true] {
                let context = egui::Context::default();
                let mut state = canvas_fixture(&context, image, 0);
                state.canvas_edit_target = CanvasEditTarget::Slot;
                let original = state.elements[0].as_object().unwrap().content_box.clone();
                let output = canvas_event(&context, &mut state, vec![]);
                let frame = selected_slot(&output);
                let start = if resize {
                    frame.right_bottom()
                } else {
                    egui::pos2(frame.left() + 3.0, frame.top() + 2.0)
                };
                canvas_event(
                    &context,
                    &mut state,
                    vec![
                        egui::Event::PointerMoved(start),
                        pointer_button(start, true),
                    ],
                );
                let end = start + egui::vec2(20.0, 14.0);
                canvas_event(&context, &mut state, vec![egui::Event::PointerMoved(end)]);
                canvas_event(&context, &mut state, vec![pointer_button(end, false)]);
                let object = state.elements[0].as_object().unwrap();
                assert_eq!(object.rotation, 30.0);
                assert_eq!(object.content_box, original);
                if resize {
                    assert_eq!((object.frame.x_mm, object.frame.y_mm), (5.0, 1.0));
                    assert_eq!((object.frame.width_mm, object.frame.height_mm), (11.4, 5.0));
                } else {
                    assert_eq!((object.frame.x_mm, object.frame.y_mm), (6.4, 2.0));
                    assert_eq!((object.frame.width_mm, object.frame.height_mm), (10.0, 4.0));
                }
            }
        }
    }

    #[test]
    fn object_gestures_leave_slots_fixed_and_scale_in_rotated_local_axes() {
        use ptouch_render::document::ObjectContent;
        for image in [false, true] {
            for mirrors in [0, 1, 2, 3, 8, 9, 10, 11] {
                for resize in [false, true] {
                    let context = egui::Context::default();
                    let mut state = canvas_fixture(&context, image, mirrors);
                    if let ObjectContent::Text {
                        font_size, styles, ..
                    } = &mut state.elements[0].as_object_mut().unwrap().content
                    {
                        *font_size = Some(5.0);
                        styles.push(ptouch_render::styled_text::StyleSpan {
                            start: 0,
                            end: 1,
                            style: ptouch_render::styled_text::TextStyle {
                                font_size: Some(7.5),
                                bold: true,
                                ..Default::default()
                            },
                        });
                    }
                    let original = state.elements[0].as_object().unwrap().clone();
                    let (quad, _) =
                        selected_quad_and_circle(&canvas_event(&context, &mut state, vec![]));
                    let start = if resize { quad[2] } else { quad_center(quad) };
                    let delta = if resize {
                        (quad[2] - quad[0]) * 0.2
                    } else {
                        egui::vec2(14.0, 7.0)
                    };
                    canvas_event(
                        &context,
                        &mut state,
                        vec![
                            egui::Event::PointerMoved(start),
                            pointer_button(start, true),
                        ],
                    );
                    canvas_event(
                        &context,
                        &mut state,
                        vec![egui::Event::PointerMoved(start + delta)],
                    );
                    canvas_event(
                        &context,
                        &mut state,
                        vec![pointer_button(start + delta, false)],
                    );
                    let object = state.elements[0].as_object().unwrap();
                    assert_eq!(object.frame, original.frame);
                    assert_eq!(object.rotation, original.rotation);
                    let edited = object.content_box.as_ref().unwrap();
                    let old = original.content_box.as_ref().unwrap();
                    if resize {
                        assert!((edited.width_mm - old.width_mm * 1.2).abs() < 0.001);
                        assert!(
                            (edited.height_mm / edited.width_mm - old.height_mm / old.width_mm)
                                .abs()
                                < 1e-8
                        );
                        if let ObjectContent::Text {
                            font_size, styles, ..
                        } = &object.content
                        {
                            assert!((font_size.unwrap() - 6.0).abs() < 0.001);
                            assert!((styles[0].style.font_size.unwrap() - 9.0).abs() < 0.001);
                            assert!(styles[0].style.bold);
                        }
                        let (resized, _) =
                            selected_quad_and_circle(&canvas_event(&context, &mut state, vec![]));
                        assert!(
                            resized[0].distance(quad[0]) < state.zoom * 1.5,
                            "opposite corner moved: {quad:?} -> {resized:?}"
                        );
                    } else {
                        assert_eq!(
                            (edited.width_mm, edited.height_mm),
                            (old.width_mm, old.height_mm)
                        );
                        let dx = if mirrors & 1 != 0 { -1.0 } else { 1.0 };
                        let dy = if (mirrors & 2 != 0) ^ (mirrors & 8 != 0) {
                            -0.5
                        } else {
                            0.5
                        };
                        assert!((edited.offset_x_mm - old.offset_x_mm - dx).abs() < 0.001);
                        assert!((edited.offset_y_mm - old.offset_y_mm - dy).abs() < 0.001);
                    }
                }
            }
        }
    }

    #[test]
    fn keyboard_moves_object_offsets_or_slot_according_to_edit_target() {
        for target in [CanvasEditTarget::Content, CanvasEditTarget::Slot] {
            let context = egui::Context::default();
            let mut state = canvas_fixture(&context, true, 1);
            state.canvas_edit_target = target;
            let original = state.elements[0].as_object().unwrap().clone();
            canvas_event(&context, &mut state, vec![]);
            canvas_event(
                &context,
                &mut state,
                vec![egui::Event::Key {
                    key: egui::Key::ArrowRight,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
            );
            let object = state.elements[0].as_object().unwrap();
            if target == CanvasEditTarget::Content {
                assert_eq!(object.frame, original.frame);
                assert!((object.content_box.as_ref().unwrap().offset_x_mm + 0.1).abs() < 1e-6);
            } else {
                assert_eq!(object.content_box, original.content_box);
                assert_eq!(object.frame.x_mm, 5.1);
            }
        }
    }

    #[test]
    fn whole_label_horizontal_mirror_disables_mouse_transforms_with_dynamic_extent() {
        let context = egui::Context::default();
        let mut state = canvas_fixture(&context, true, 4);
        let object = state.elements[0].as_object_mut().unwrap();
        object.content_box.as_mut().unwrap().width_mm = 20.0;
        let before = state.document().to_toml_string().unwrap();
        let output = canvas_event(&context, &mut state, vec![]);
        assert!(!output.shapes.iter().any(|shape| matches!(
            &shape.shape, egui::Shape::Circle(c) if c.radius == ROTATE_RADIUS
        )));
        canvas_event(
            &context,
            &mut state,
            vec![egui::Event::Key {
                key: egui::Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        assert_eq!(state.document().to_toml_string().unwrap(), before);
    }

    #[test]
    fn rotation_unwraps_multiple_turns_ignores_the_center_and_snaps_angles() {
        let point =
            |degrees: f32| egui::Pos2::ZERO + egui::Vec2::angled(degrees.to_radians()) * 50.0;
        let mut rotation = RotationDrag {
            last_pointer: point(179.0),
            accumulated: 0.0,
            initial: 30.0,
            reverse: false,
        };
        assert!(
            (dragged_rotation(&mut rotation, point(-179.0), egui::Pos2::ZERO, false).unwrap()
                - 32.0)
                .abs()
                < 0.01
        );
        assert_eq!(
            dragged_rotation(&mut rotation, egui::pos2(1.0, 1.0), egui::Pos2::ZERO, false),
            None
        );
        // Leaving the ignored center re-establishes the pointer without a jump.
        assert_eq!(
            dragged_rotation(&mut rotation, point(-179.0), egui::Pos2::ZERO, false),
            None
        );
        for angle in [-89.0, 1.0, 91.0, -179.0] {
            dragged_rotation(&mut rotation, point(angle), egui::Pos2::ZERO, false);
        }
        assert!((rotation.accumulated - 362.0).abs() < 0.01);
        assert_eq!(
            dragged_rotation(&mut rotation, point(-170.0), egui::Pos2::ZERO, true),
            Some(45.0)
        );
    }

    #[test]
    fn y_ruler_uses_physical_tape_and_tracks_scrolled_original_coordinates() {
        for (dpi, height, endpoint) in [(180, 76, "12.00"), (360, 150, "12.00")] {
            let context = egui::Context::default();
            let mut state = canvas_fixture(&context, false, 0);
            state.design_dpi = dpi;
            state.preview_content_bounds.clear();
            state.preview_bitmap = Some(ptouch_render::bitmap::LabelBitmap::new(200, height));
            let output = canvas_event(&context, &mut state, vec![]);
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(t) if t.galley.job.text == endpoint)));
            let scale = f32::from(dpi) / 25.4 * 2.0;
            let (_, ticks) = ruler_ticks(100.0, 400.0, -75.0, 1000.0, scale, 28.0);
            assert!(!ticks.is_empty());
            for tick in ticks {
                assert!((tick.screen + 75.0 - tick.mm * scale).abs() < 0.001);
                assert!((100.0..=400.0).contains(&tick.screen));
            }
        }
    }

    #[test]
    fn legacy_flow_elements_remain_selectable_without_conversion() {
        use ptouch_render::document::LabelElement;
        let context = egui::Context::default();
        let mut state = canvas_fixture(&context, true, 0);
        state.document_version = 1;
        state.elements = vec![LabelElement::Padding { pixels: 80 }];
        state.selected_element = None;
        render_fixture(&context, &mut state);
        let output = canvas_event(&context, &mut state, vec![]);
        let label = output
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect) if rect.fill == egui::Color32::WHITE => Some(rect.rect),
                _ => None,
            })
            .unwrap();
        let point = label.center();
        canvas_event(
            &context,
            &mut state,
            vec![
                egui::Event::PointerMoved(point),
                pointer_button(point, true),
            ],
        );
        canvas_event(&context, &mut state, vec![pointer_button(point, false)]);
        assert_eq!(state.selected_element, Some(0));
        assert_eq!(state.document().version, 1);
        assert!(matches!(
            state.elements[0],
            LabelElement::Padding { pixels: 80 }
        ));
    }

    #[test]
    fn clicking_blank_canvas_deselects_and_arrow_keys_leave_objects_in_place() {
        use ptouch_render::document::{LabelElement, LabelObject};
        let context = egui::Context::default();
        let mut state = AppState {
            zoom_fit: false,
            zoom: 2.0,
            elements: vec![LabelElement::Object(LabelObject::text(
                "A",
                TextBox {
                    x_mm: 5.0,
                    ..TextBox::default()
                },
            ))],
            selected_element: Some(0),
            preview_bitmap: Some(ptouch_render::bitmap::LabelBitmap::new(200, 76)),
            preview_bounds: vec![ElementBounds {
                element_index: 0,
                x: 35,
                y: 0,
                width: 71,
                height: 57,
            }],
            ..AppState::default()
        };
        state.preview_texture = Some(context.load_texture(
            "test_canvas",
            egui::ColorImage::filled([200, 76], egui::Color32::WHITE),
            Default::default(),
        ));
        let blank = egui::pos2(500.0, 400.0);
        for events in [
            vec![],
            vec![
                egui::Event::PointerMoved(blank),
                egui::Event::PointerButton {
                    pos: blank,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
            ],
            vec![egui::Event::PointerButton {
                pos: blank,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            }],
            vec![egui::Event::Key {
                key: egui::Key::ArrowRight,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        ] {
            let mut output = context.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| show_canvas(ui, &mut state),
            );
            output.textures_delta.clear();
        }
        assert_eq!(state.selected_element, None);
        assert_eq!(state.elements[0].as_object().unwrap().frame.x_mm, 5.0);
    }

    #[test]
    fn dragging_relative_allocations_stops_at_the_predecessor_without_detaching() {
        let original = TextBox {
            position: PositionMode::Relative,
            gap_mm: 3.0,
            height_mm: 4.0,
            ..TextBox::default()
        };
        let drag = drag(original, 18.0);
        let moved = dragged_frame(&drag, egui::vec2(40.0, 20.0), 10.0, true, 0.15);
        assert_eq!(moved.position, PositionMode::Relative);
        assert_eq!((moved.gap_mm, moved.y_mm), (5.0, 1.0));
        let overlap = dragged_frame(&drag, egui::vec2(-80.0, 0.0), 10.0, true, 0.15);
        assert_eq!(overlap.position, PositionMode::Relative);
        assert_eq!(overlap.gap_mm, 0.0);
    }

    #[test]
    fn dragging_uses_frozen_scale_and_respects_printable_height() {
        let mut drag = drag(
            TextBox {
                x_mm: 5.0,
                height_mm: 4.0,
                ..TextBox::default()
            },
            5.0,
        );
        let moved = dragged_frame(&drag, egui::vec2(-200.0, 500.0), 10.0, false, 0.15);
        assert_eq!((moved.x_mm, moved.y_mm), (0.0, 6.0));
        drag.flip_h = true;
        drag.flip_v = true;
        let mirrored = dragged_frame(&drag, egui::vec2(-40.0, -20.0), 10.0, false, 0.15);
        assert_eq!((mirrored.x_mm, mirrored.y_mm), (7.0, 1.0));
        drag.resize = true;
        let resized = dragged_frame(&drag, egui::vec2(-40.0, -500.0), 10.0, false, 0.15);
        assert_eq!(
            (resized.x_mm, resized.width_mm, resized.height_mm),
            (5.0, 12.0, 10.0)
        );
    }

    #[test]
    fn long_label_fit_uses_original_pixels_and_keeps_bounds_on_ruler() {
        let original = egui::vec2(100_000.0, 128.0);
        let canvas = egui::vec2(800.0, 500.0);
        let zoom = fit_zoom(original, canvas);
        assert!(zoom < 0.1);
        assert!(original.x * zoom < canvas.x);
        let bound = ElementBounds {
            element_index: 0,
            x: 709,
            y: 0,
            width: 142,
            height: 100,
        };
        let rect = screen_bounds(egui::Pos2::ZERO, zoom, &bound);
        assert!((rect.min.x - 709.0 * zoom).abs() < 0.001);
        let measured_mm = rect.min.x / zoom * 25.4 / 180.0;
        assert!((measured_mm - 100.05).abs() < 0.02);
        assert!(ruler_step(180.0 / 25.4 * zoom) * 180.0 / 25.4 * zoom >= 55.0);
    }
}
