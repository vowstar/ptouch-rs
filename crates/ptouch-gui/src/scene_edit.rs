// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Scene transactions. Layout order and paint order share one selection, but
//! changing either has no implicit effect on the other.

use std::{io::Cursor, path::PathBuf};

use ptouch_render::{
    document::{ContentBox, LabelElement, LabelObject, ObjectContent, TextBox},
    image_loader::{ImageLoadOptions, load_layer_from_reader},
    object_layout::{self, LayoutOptions, Placement, WidthSizing},
    scene::{self, Anchor},
    sequence,
    shape::{ShapeKind, ShapeStyle},
};

use crate::state::{AppState, CanvasEditTarget, CanvasTarget};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerMove {
    Front,
    Forward,
    Backward,
    Back,
}

/// Structural edits are infrequent. Preserve the full model and relevant UI
/// state so a failed relation edit cannot clear the preview or change history.
pub(crate) struct SceneSnapshot {
    elements: Vec<LabelElement>,
    document_version: u32,
    selected: Option<usize>,
    target: CanvasEditTarget,
    hovered: Option<CanvasTarget>,
    dragged: Option<CanvasTarget>,
    edit_revision: u64,
    selection_revision: u64,
    editor_generation: u64,
    needs_rerender: bool,
}

impl SceneSnapshot {
    pub(crate) fn take(state: &AppState) -> Self {
        Self {
            elements: state.elements.clone(),
            document_version: state.document_version,
            selected: state.selected_element,
            target: state.canvas_edit_target,
            hovered: state.canvas_hover_target,
            dragged: state.canvas_drag_target,
            edit_revision: state.edit_revision,
            selection_revision: state.selection_revision,
            editor_generation: state.editor_generation,
            needs_rerender: state.needs_rerender,
        }
    }

    pub(crate) fn restore(self, state: &mut AppState) {
        state.elements = self.elements;
        state.document_version = self.document_version;
        state.selected_element = self.selected;
        state.canvas_edit_target = self.target;
        state.canvas_hover_target = self.hovered;
        state.canvas_drag_target = self.dragged;
        state.edit_revision = self.edit_revision;
        state.selection_revision = self.selection_revision;
        state.editor_generation = self.editor_generation;
        state.needs_rerender = self.needs_rerender;
    }
}

fn free_layout() -> LayoutOptions {
    LayoutOptions {
        placement: Placement::Free,
        width: WidthSizing::Fixed,
        constrain: true,
        padding_mm: 0.0,
    }
}

fn content_box(frame: &TextBox) -> ContentBox {
    ContentBox {
        width_mm: frame.width_mm,
        height_mm: frame.height_mm,
        offset_x_mm: 0.0,
        offset_y_mm: 0.0,
    }
}

fn scene_id(elements: &[LabelElement], index: usize) -> Option<u64> {
    Some(elements.get(index)?.as_object()?.scene.as_ref()?.id)
}

fn fresh_id(elements: &[LabelElement]) -> Result<u64, String> {
    elements
        .iter()
        .filter_map(|element| Some(element.as_object()?.scene.as_ref()?.id))
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .filter(|id| *id <= i64::MAX as u64)
        .ok_or_else(|| "Scene identifier limit reached".into())
}

fn anchor_of(object: &LabelObject) -> Option<&Anchor> {
    object.scene.as_ref()?.anchor.as_ref()
}

fn is_attached(element: &LabelElement) -> bool {
    element.as_object().is_some_and(|o| anchor_of(o).is_some())
}

impl AppState {
    /// Lightweight validation used during pointer movement. No font system,
    /// TOML encoding or source-image decoding runs in this path.
    pub(crate) fn validate_scene_geometry(&mut self) -> Result<(), String> {
        for object in self
            .elements
            .iter_mut()
            .filter_map(LabelElement::as_object_mut)
        {
            object_layout::update_allocation(object);
            object_layout::validate(object, 12).map_err(|error| error.to_string())?;
            object
                .validate_features(12)
                .map_err(|error| error.to_string())?;
            object_layout::check_allocation(object).map_err(|error| error.to_string())?;
            if !object.rotation.is_finite() {
                return Err("Object rotation must be finite".into());
            }
        }
        scene::validate(&self.elements, 12).map_err(|error| error.to_string())?;
        let frames = scene::world_frames(&self.elements).map_err(|error| error.to_string())?;
        let height = self.printable_height_mm();
        for frame in &frames {
            if frame.width_mm <= 0.0
                || frame.height_mm <= 0.0
                || frame.x_mm < 0.0
                || frame.y_mm < 0.0
                || frame.y_mm + frame.height_mm > height + 1e-7
                || !(frame.x_mm + frame.width_mm).is_finite()
            {
                return Err("This change would place an object outside the printable area".into());
            }
        }
        for (element, world) in self.elements.iter_mut().zip(frames) {
            if is_attached(element) {
                let object = element.as_object_mut().unwrap();
                object.frame.x_mm = world.x_mm;
                object.frame.y_mm = world.y_mm;
            }
        }
        Ok(())
    }

    /// A document font change can resize every auto-width host. Measure once
    /// with a cached renderer and reject the whole change if an anchor leaves tape.
    pub(crate) fn apply_scene_font(&mut self, font: String) -> Result<(), String> {
        if self.font_name == font {
            return Ok(());
        }
        let before = SceneSnapshot::take(self);
        let old_font = std::mem::replace(&mut self.font_name, font);
        let mut doc = self.document();
        let renderer = self
            .scene_measurement_renderer
            .get_or_insert_with(ptouch_render::text::TextRenderer::new);
        let result = object_layout::resolve(&mut doc, renderer)
            .map_err(|error| error.to_string())
            .and_then(|()| {
                self.elements = doc.elements;
                self.validate_scene_geometry()
            });
        if let Err(error) = result {
            before.restore(self);
            self.font_name = old_font;
            return Err(error);
        }
        self.mark_dirty();
        Ok(())
    }

    fn finish_scene_transaction(
        &mut self,
        before: SceneSnapshot,
        selected: Option<usize>,
        message: &str,
    ) -> bool {
        if let Err(error) = self.validate_scene_geometry() {
            before.restore(self);
            self.status_message = error;
            return false;
        }
        self.document_version = self.document_version.max(12);
        let structure_changed = before.elements.len() != self.elements.len()
            || before
                .elements
                .iter()
                .zip(&self.elements)
                .any(|(old, new)| {
                    old.as_object().and_then(|o| o.scene.as_ref()).map(|s| s.id)
                        != new.as_object().and_then(|o| o.scene.as_ref()).map(|s| s.id)
                });
        if structure_changed {
            self.validate_selection();
        }
        let selected = selected
            .filter(|_| !self.elements.is_empty())
            .map(|index| index.min(self.elements.len() - 1));
        self.select_canvas_target(selected.map(|element| CanvasTarget {
            element,
            edit: self.canvas_edit_target,
        }));
        self.canvas_hover_target = None;
        self.canvas_drag_target = None;
        self.status_message = message.into();
        self.mark_dirty();
        true
    }

    fn fail_scene_transaction(&mut self, before: SceneSnapshot, error: String) -> bool {
        before.restore(self);
        self.status_message = error;
        false
    }

    fn layer_order_change(&mut self, order: Vec<usize>) -> bool {
        if !self.is_free_layout() {
            self.status_message = "Convert legacy elements before arranging layers".into();
            return false;
        }
        if let Err(error) = scene::enable(&mut self.elements) {
            self.status_message = error.to_string();
            return false;
        }
        if order.len() > i32::MAX as usize {
            self.status_message = "Scene layer limit reached".into();
            return false;
        }
        for (layer, index) in order.into_iter().enumerate() {
            self.elements[index]
                .as_object_mut()
                .unwrap()
                .scene
                .as_mut()
                .unwrap()
                .layer = layer as i32;
        }
        self.document_version = self.document_version.max(12);
        self.status_message = "Layer order changed; layout positions preserved".into();
        self.mark_dirty();
        true
    }

    pub fn change_layer(&mut self, index: usize, movement: LayerMove) -> bool {
        let mut order = scene::paint_order(&self.elements);
        let Some(from) = order.iter().position(|&item| item == index) else {
            return false;
        };
        let to = match movement {
            LayerMove::Front => order.len() - 1,
            LayerMove::Forward => (from + 1).min(order.len() - 1),
            LayerMove::Backward => from.saturating_sub(1),
            LayerMove::Back => 0,
        };
        if from == to {
            return false;
        }
        let item = order.remove(from);
        order.insert(to, item);
        self.layer_order_change(order)
    }

    /// Both indices identify objects, regardless of which list is displayed.
    pub fn reorder_layer(&mut self, from: usize, to: usize) -> bool {
        let mut order = scene::paint_order(&self.elements);
        let (Some(source), Some(target)) = (
            order.iter().position(|&item| item == from),
            order.iter().position(|&item| item == to),
        ) else {
            return false;
        };
        if source == target {
            return false;
        }
        let item = order.remove(source);
        order.insert(target, item);
        self.layer_order_change(order)
    }

    fn append_scene_object(&mut self, object: LabelObject, host: Option<usize>) -> bool {
        if !self.is_free_layout() {
            self.status_message = "Convert legacy elements before adding scene objects".into();
            return false;
        }
        if host.is_some_and(|index| {
            self.elements.get(index).is_none() || is_attached(&self.elements[index])
        }) {
            self.status_message = "Select an independent object to host the overlay".into();
            return false;
        }
        let before = SceneSnapshot::take(self);
        if let Err(error) = scene::enable(&mut self.elements)
            .and_then(|_| sequence::materialize(&mut self.elements))
        {
            return self.fail_scene_transaction(before, error.to_string());
        }
        let host_id = host.and_then(|index| scene_id(&self.elements, index));
        let index = self.elements.len();
        self.elements.push(LabelElement::Object(object));
        if let Err(error) = scene::enable(&mut self.elements) {
            return self.fail_scene_transaction(before, error.to_string());
        }
        if let Some(target) = host_id {
            self.elements[index]
                .as_object_mut()
                .unwrap()
                .scene
                .as_mut()
                .unwrap()
                .anchor = Some(Anchor {
                target,
                horizontal: 1.0,
                vertical: 0.0,
                offset_x_mm: 0.0,
                offset_y_mm: 0.0,
            });
        }
        self.canvas_edit_target = CanvasEditTarget::Content;
        self.finish_scene_transaction(
            before,
            Some(index),
            if host.is_some() {
                "Overlay follows the layout box; rotation stays independent"
            } else {
                "Shape added with free positioning"
            },
        )
    }

    fn shape_object(&self, kind: ShapeKind, circle: bool, frame: TextBox) -> LabelObject {
        let mut object = LabelObject::text("", frame);
        object.layout = Some(free_layout());
        object.content_box = Some(content_box(&object.frame));
        object.content = ObjectContent::Shape {
            style: ShapeStyle {
                kind: if circle { ShapeKind::Ellipse } else { kind },
                ..Default::default()
            },
        };
        object
    }

    pub fn add_shape(&mut self, kind: ShapeKind, circle: bool) -> bool {
        let height = (self.printable_height_mm() * 0.7).min(7.0);
        let width = if circle { height } else { height * 1.6 };
        let mut frame = self.default_object_frame(width, height);
        frame.y_mm = (self.printable_height_mm() - height) / 2.0;
        let object = self.shape_object(kind, circle, frame);
        self.append_scene_object(object, None)
    }

    fn overlay_frame(&self, host: usize, width: f64, height: f64) -> Option<TextBox> {
        let world = scene::world_frames(&self.elements).ok()?;
        let host = world.get(host)?;
        Some(TextBox {
            width_mm: width,
            height_mm: height,
            x_mm: host.x_mm + host.width_mm - width,
            y_mm: host.y_mm,
            ..Default::default()
        })
    }

    pub fn add_overlay_text(&mut self, host: usize) -> bool {
        let Some((_, _, width, height)) = self.element_rect_mm(host) else {
            return false;
        };
        let Some(frame) = self.overlay_frame(
            host,
            (width * 0.45).clamp(0.3, 10.0),
            (height * 0.4).clamp(0.3, 3.0),
        ) else {
            return false;
        };
        let mut object = LabelObject::text("Note", frame);
        object.content_box = Some(content_box(&object.frame));
        object.layout = Some(free_layout());
        self.append_scene_object(object, Some(host))
    }

    pub fn add_overlay_image_bytes(
        &mut self,
        host: usize,
        path: Option<PathBuf>,
        bytes: Vec<u8>,
    ) -> bool {
        let Some((_, _, host_width, host_height)) = self.element_rect_mm(host) else {
            return false;
        };
        let layer = match load_layer_from_reader(Cursor::new(&bytes), &ImageLoadOptions::default())
        {
            Ok(layer) if layer.ink.width() > 0 && layer.ink.height() > 0 => layer,
            Ok(_) => {
                self.status_message = "Image has no dimensions".into();
                return false;
            }
            Err(error) => {
                self.status_message = format!("Image load error: {error}");
                return false;
            }
        };
        let aspect = f64::from(layer.ink.width()) / f64::from(layer.ink.height());
        let height = (host_height * 0.45)
            .min(4.0)
            .min((host_width * 0.45).min(10.0) / aspect)
            .max(0.15);
        let Some(frame) = self.overlay_frame(host, height * aspect, height) else {
            return false;
        };
        let mut object = LabelObject::text("", frame);
        object.layout = Some(free_layout());
        object.content_box = Some(content_box(&object.frame));
        object.content = ObjectContent::Image {
            path,
            image_data: bytes,
            bitmap: Some(layer.ink),
            preserve_alpha: true,
            coverage: layer.coverage.map(Box::new),
        };
        self.append_scene_object(object, Some(host))
    }

    pub fn add_overlay_shape(&mut self, host: usize, kind: ShapeKind, circle: bool) -> bool {
        let Some((_, _, width, height)) = self.element_rect_mm(host) else {
            return false;
        };
        let mut width = width * 0.8;
        let mut height = height * 0.7;
        if circle {
            width = width.min(height);
            height = width;
        }
        let Some(frame) = self.overlay_frame(host, width, height) else {
            return false;
        };
        let object = self.shape_object(kind, circle, frame);
        self.append_scene_object(object, Some(host))
    }

    pub fn can_attach_overlay(&self, child: usize, host: usize) -> bool {
        child != host
            && self.is_free_layout()
            && self.elements.get(child).is_some()
            && self.elements.get(host).is_some()
            && !is_attached(&self.elements[host])
            && scene::children(&self.elements, child).is_empty()
    }

    pub fn attach_overlay(&mut self, child: usize, host: usize) -> bool {
        if !self.can_attach_overlay(child, host) {
            self.status_message =
                "An overlay needs an independent host and cannot host overlays".into();
            return false;
        }
        let before = SceneSnapshot::take(self);
        let world = match scene::world_frames(&self.elements) {
            Ok(world) => world,
            Err(error) => return self.fail_scene_transaction(before, error.to_string()),
        };
        if let Err(error) = scene::enable(&mut self.elements)
            .and_then(|_| sequence::detach(&mut self.elements, child))
        {
            return self.fail_scene_transaction(before, error.to_string());
        }
        let target = scene_id(&self.elements, host).unwrap();
        let resolved = self.resolved_content_box(child);
        let object = self.elements[child].as_object_mut().unwrap();
        object.content_box = object.content_box.clone().or(resolved);
        let Some((dx, dy, width, height)) = object_layout::content_extent(object) else {
            return self.fail_scene_transaction(before, "Object geometry is unavailable".into());
        };
        let frame = TextBox {
            x_mm: world[child].x_mm + world[child].width_mm / 2.0 + dx,
            y_mm: world[child].y_mm + world[child].height_mm / 2.0 + dy,
            width_mm: width,
            height_mm: height,
            ..world[child].clone()
        };
        let content = object.content_box.as_mut().unwrap();
        content.offset_x_mm = 0.0;
        content.offset_y_mm = 0.0;
        object.frame = frame.clone();
        object.sequence = None;
        object.layout = Some(free_layout());
        let anchor = Anchor {
            target,
            horizontal: 1.0,
            vertical: 0.0,
            offset_x_mm: 0.0,
            offset_y_mm: 0.0,
        };
        object.scene.as_mut().unwrap().anchor =
            Some(scene::offset_for_frame(&anchor, &frame, &world[host]));
        self.finish_scene_transaction(
            before,
            Some(child),
            "Overlay attached; its current position is preserved",
        )
    }

    pub fn detach_overlay(&mut self, index: usize) -> bool {
        if !self.elements.get(index).is_some_and(is_attached) {
            return false;
        }
        let before = SceneSnapshot::take(self);
        let world = match scene::world_frames(&self.elements) {
            Ok(world) => world,
            Err(error) => return self.fail_scene_transaction(before, error.to_string()),
        };
        let object = self.elements[index].as_object_mut().unwrap();
        object.scene.as_mut().unwrap().anchor = None;
        object.frame = world[index].clone();
        object.sequence = None;
        self.finish_scene_transaction(
            before,
            Some(index),
            "Overlay detached at its current position",
        )
    }

    pub fn set_overlay_alignment(&mut self, index: usize, horizontal: f64, vertical: f64) -> bool {
        let Some(mut anchor) = self
            .elements
            .get(index)
            .and_then(LabelElement::as_object)
            .and_then(anchor_of)
            .cloned()
        else {
            return false;
        };
        let world = match scene::world_frames(&self.elements) {
            Ok(world) => world,
            Err(error) => {
                self.status_message = error.to_string();
                return false;
            }
        };
        let Some(host) = self.elements.iter().position(|element| {
            element
                .as_object()
                .and_then(|o| o.scene.as_ref())
                .is_some_and(|s| s.id == anchor.target)
        }) else {
            return false;
        };
        anchor.horizontal = horizontal;
        anchor.vertical = vertical;
        anchor = scene::offset_for_frame(&anchor, &world[index], &world[host]);
        self.set_overlay_anchor(index, anchor)
    }

    pub fn set_overlay_anchor(&mut self, index: usize, anchor: Anchor) -> bool {
        let Some(object) = self.elements.get(index).and_then(LabelElement::as_object) else {
            return false;
        };
        if object.scene.is_none() || anchor_of(object) == Some(&anchor) {
            return false;
        }
        let before = SceneSnapshot::take(self);
        self.elements[index]
            .as_object_mut()
            .unwrap()
            .scene
            .as_mut()
            .unwrap()
            .anchor = Some(anchor);
        self.finish_scene_transaction(before, Some(index), "Overlay alignment updated")
    }

    pub fn apply_scene_object_edit(
        &mut self,
        index: usize,
        mut object: LabelObject,
    ) -> Result<(), String> {
        // A hand-written v12 shape/alpha document may not have used layers
        // yet. Its first GUI scene edit establishes identity explicitly.
        if self
            .elements
            .get(index)
            .and_then(LabelElement::as_object)
            .is_some_and(|before| before.scene.is_none())
        {
            let snapshot = SceneSnapshot::take(self);
            if let Err(error) = scene::enable(&mut self.elements) {
                snapshot.restore(self);
                return Err(error.to_string());
            }
            object.scene = self.elements[index].as_object().unwrap().scene.clone();
            if let Err(error) = self.apply_scene_object_edit(index, object) {
                snapshot.restore(self);
                return Err(error);
            }
            self.document_version = self.document_version.max(12);
            return Ok(());
        }
        let before = self
            .elements
            .get(index)
            .and_then(LabelElement::as_object)
            .ok_or("Select an object")?;
        if object.scene.as_ref().map(|s| s.id) != before.scene.as_ref().map(|s| s.id) {
            return Err("An edit cannot change persistent object identity".into());
        }
        let placement = |object: &LabelObject| object.layout.as_ref().map(|p| p.placement);
        let structural = placement(before) != placement(&object)
            || before.sequence != object.sequence
            || before.frame.position != object.frame.position
            || anchor_of(before) != anchor_of(&object)
            || placement(&object) == Some(Placement::Free)
                && anchor_of(before).is_none()
                && self.flow_chain(index).is_some();
        let old_anchor = anchor_of(before).cloned();
        let measure = object
            .layout
            .as_ref()
            .is_some_and(|p| p.width == WidthSizing::Content)
            && matches!(object.content, ObjectContent::Text { .. })
            && (before.content != object.content
                || before.content_box != object.content_box
                || before.border != object.border
                || before.layout != object.layout);
        if measure {
            let renderer = self
                .scene_measurement_renderer
                .get_or_insert_with(ptouch_render::text::TextRenderer::new);
            object_layout::resolve_object(
                &mut object,
                12,
                self.design_dpi,
                &self.font_name,
                renderer,
            )
            .map_err(|error| error.to_string())?;
        }
        object_layout::update_allocation(&mut object);
        if let Some(anchor) = old_anchor.as_ref()
            && anchor_of(&object) == Some(anchor)
        {
            let world = scene::world_frames(&self.elements).map_err(|error| error.to_string())?;
            let host = self
                .elements
                .iter()
                .position(|element| {
                    element
                        .as_object()
                        .and_then(|o| o.scene.as_ref())
                        .is_some_and(|s| s.id == anchor.target)
                })
                .ok_or("Overlay host is unavailable")?;
            let desired = object.frame.clone();
            object.scene.as_mut().unwrap().anchor =
                Some(scene::offset_for_frame(anchor, &desired, &world[host]));
        }
        if structural {
            let snapshot = SceneSnapshot::take(self);
            if let Err(error) = self.apply_object_edit_base(index, object) {
                snapshot.restore(self);
                return Err(error);
            }
            if let Err(error) = self.validate_scene_geometry() {
                snapshot.restore(self);
                return Err(error);
            }
            self.document_version = self.document_version.max(12);
            return Ok(());
        }
        // Pointer frames retain only light geometry snapshots. Moving an
        // icon must not clone every embedded image in the document.
        let frames: Vec<_> = self
            .elements
            .iter()
            .filter_map(|e| e.as_object().map(|o| o.frame.clone()))
            .collect();
        let previous = std::mem::replace(&mut self.elements[index], LabelElement::Object(object));
        if let Err(error) = self.validate_scene_geometry() {
            self.elements[index] = previous;
            for (element, frame) in self.elements.iter_mut().zip(frames) {
                element.as_object_mut().unwrap().frame = frame;
            }
            return Err(error);
        }
        self.mark_dirty();
        Ok(())
    }

    pub fn duplicate_scene_element(&mut self, index: usize) -> bool {
        if index >= self.elements.len() {
            return false;
        }
        let before = SceneSnapshot::take(self);
        if let Err(error) = scene::enable(&mut self.elements) {
            return self.fail_scene_transaction(before, error.to_string());
        }
        let source_id = scene_id(&self.elements, index).unwrap();
        let was_child = is_attached(&self.elements[index]);
        let mut child_indices = scene::children(&self.elements, index);
        child_indices.sort_by_key(|&child| {
            let scene = self.elements[child]
                .as_object()
                .unwrap()
                .scene
                .as_ref()
                .unwrap();
            (scene.layer, scene.id)
        });
        let source_order: Vec<_> = scene::paint_order(&self.elements)
            .into_iter()
            .filter(|&item| item == index || child_indices.contains(&item))
            .map(|item| scene_id(&self.elements, item).unwrap())
            .collect();
        let mut copied_ids = std::collections::BTreeMap::new();
        let new_id = match fresh_id(&self.elements) {
            Ok(id) => id,
            Err(error) => return self.fail_scene_transaction(before, error),
        };
        if let Err(error) = sequence::materialize(&mut self.elements) {
            return self.fail_scene_transaction(before, error.to_string());
        }
        if !self.duplicate_element_base(index) {
            let error = self.status_message.clone();
            return self.fail_scene_transaction(before, error);
        }
        let copy_index = self.selected_element.unwrap();
        let copy = self.elements[copy_index].as_object_mut().unwrap();
        copy.scene.as_mut().unwrap().id = new_id;
        copied_ids.insert(source_id, new_id);
        if was_child {
            copy.scene
                .as_mut()
                .unwrap()
                .anchor
                .as_mut()
                .unwrap()
                .offset_x_mm += 0.5;
        }
        for child in child_indices {
            let mut copy = before.elements[child].clone();
            let id = match fresh_id(&self.elements) {
                Ok(id) => id,
                Err(error) => return self.fail_scene_transaction(before, error),
            };
            let metadata = copy.as_object_mut().unwrap().scene.as_mut().unwrap();
            copied_ids.insert(metadata.id, id);
            metadata.id = id;
            let anchor = metadata.anchor.as_mut().unwrap();
            debug_assert_eq!(anchor.target, source_id);
            anchor.target = new_id;
            self.elements.push(copy);
        }
        // Append the copied subset above the originals in its source paint
        // order, including equal-rank ID ties. Allocation order is unrelated.
        let mut order: Vec<_> = scene::paint_order(&self.elements)
            .into_iter()
            .filter(|&i| {
                !copied_ids
                    .values()
                    .any(|&id| scene_id(&self.elements, i) == Some(id))
            })
            .collect();
        for id in source_order {
            let copied = copied_ids[&id];
            order.push(
                self.elements
                    .iter()
                    .position(|element| {
                        element
                            .as_object()
                            .and_then(|o| o.scene.as_ref())
                            .is_some_and(|s| s.id == copied)
                    })
                    .unwrap(),
            );
        }
        for (rank, item) in order.into_iter().enumerate() {
            self.elements[item]
                .as_object_mut()
                .unwrap()
                .scene
                .as_mut()
                .unwrap()
                .layer = i32::try_from(rank).expect("scene element count fits layer ranks");
        }
        self.finish_scene_transaction(
            before,
            Some(copy_index),
            "Duplicated object and its attached overlays",
        )
    }

    pub fn delete_scene_element(&mut self, index: usize) -> bool {
        if index >= self.elements.len() {
            return false;
        }
        let before = SceneSnapshot::take(self);
        let world = match scene::world_frames(&self.elements) {
            Ok(world) => world,
            Err(error) => return self.fail_scene_transaction(before, error.to_string()),
        };
        let was_child = is_attached(&self.elements[index]);
        let children = scene::children(&self.elements, index);
        if let Err(error) = sequence::materialize(&mut self.elements) {
            return self.fail_scene_transaction(before, error.to_string());
        }
        for child in children {
            let object = self.elements[child].as_object_mut().unwrap();
            object.scene.as_mut().unwrap().anchor = None;
            object.frame = world[child].clone();
            object.sequence = None;
            object.layout = Some(free_layout());
        }
        if was_child {
            self.elements.remove(index);
            if let Some(selected) = &mut self.selected_element
                && *selected > index
            {
                *selected -= 1;
            }
        } else if !self.delete_element_base(index) {
            let error = self.status_message.clone();
            return self.fail_scene_transaction(before, error);
        }
        let selected = self.selected_element;
        self.finish_scene_transaction(
            before,
            selected,
            "Deleted object; attached overlays keep their positions",
        )
    }

    /// Present attachments beneath their host without changing stored layers
    /// or rewriting an untouched document's element order.
    pub fn layout_order(&self) -> Vec<usize> {
        let mut children = std::collections::BTreeMap::<u64, Vec<usize>>::new();
        for (index, element) in self.elements.iter().enumerate() {
            if let Some(anchor) = element.as_object().and_then(anchor_of) {
                children.entry(anchor.target).or_default().push(index);
            }
        }
        let mut result = Vec::with_capacity(self.elements.len());
        for (index, element) in self.elements.iter().enumerate() {
            if is_attached(element) {
                continue;
            }
            result.push(index);
            if let Some(id) = element
                .as_object()
                .and_then(|o| o.scene.as_ref())
                .map(|s| s.id)
                && let Some(attached) = children.remove(&id)
            {
                result.extend(attached);
            }
        }
        // Invalid or unfinished scene metadata must not hide a list row.
        for attached in children.into_values() {
            result.extend(attached);
        }
        result
    }

    /// Up/Down addresses physical layout peers, never an attached child row.
    pub fn layout_neighbor(&self, index: usize, down: bool) -> Option<usize> {
        if is_attached(self.elements.get(index)?) {
            return None;
        }
        if let Some(chain) = self.flow_chain(index) {
            let position = chain.iter().position(|i| *i == index)?;
            return if down {
                chain.get(position + 1).copied()
            } else {
                position.checked_sub(1).map(|i| chain[i])
            };
        }
        if down {
            (index + 1..self.elements.len()).find(|i| !is_attached(&self.elements[*i]))
        } else {
            self.previous_layout_index(index)
        }
    }

    pub fn previous_sequence_end(&self, index: usize) -> Option<usize> {
        let source = self.flow_chain(index)?;
        let first = *source.first()?;
        let id = self.elements[first].as_object()?.sequence?;
        let previous = (0..first).rev().find(|i| {
            self.elements[*i]
                .as_object()
                .is_some_and(|o| o.sequence.is_some_and(|other| other != id))
        })?;
        let destination = self.elements[previous].as_object()?.sequence?;
        // An interleaved container has no unambiguous preceding end.
        (!self.elements[previous + 1..].iter().any(|e| {
            e.as_object()
                .is_some_and(|o| o.sequence == Some(destination))
        }))
        .then_some(previous)
    }

    pub fn connect_sequence(&mut self, index: usize) -> bool {
        let Some(first) = self
            .flow_chain(index)
            .and_then(|chain| chain.first().copied())
        else {
            return false;
        };
        let Some(previous) = self.previous_sequence_end(index) else {
            return false;
        };
        let before = SceneSnapshot::take(self);
        if let Err(error) =
            sequence::connect(&mut self.elements, first, previous, self.document_version)
        {
            return self.fail_scene_transaction(before, error.to_string());
        }
        self.document_version = self.document_version.max(13);
        if self.uses_scene() {
            self.finish_scene_transaction(
                before,
                Some(index),
                "Sequences connected; positions preserved. Adjust Gap before to change spacing.",
            )
        } else {
            self.mark_dirty();
            self.status_message =
                "Sequences connected; positions preserved. Adjust Gap before to change spacing."
                    .into();
            true
        }
    }

    pub fn layout_host(&self, index: usize) -> Option<usize> {
        let element = self.elements.get(index)?;
        match element.as_object().and_then(anchor_of) {
            None => Some(index),
            Some(anchor) => self.elements.iter().position(|element| {
                element
                    .as_object()
                    .and_then(|o| o.scene.as_ref())
                    .is_some_and(|s| s.id == anchor.target)
            }),
        }
    }

    pub fn reorder_scene_element(&mut self, from: usize, mut to: usize) -> bool {
        if from == to || from >= self.elements.len() || to >= self.elements.len() {
            return false;
        }
        let moving_host = !is_attached(&self.elements[from]);
        if moving_host {
            let Some(host) = self.layout_host(to) else {
                return false;
            };
            to = host;
            if from == to {
                return false;
            }
        }
        let selected_id = scene_id(&self.elements, from);
        let crosses_sequence = self
            .flow_chain(from)
            .zip(self.flow_chain(to))
            .is_some_and(|(a, b)| a != b);
        let before = SceneSnapshot::take(self);
        if let Err(error) = sequence::materialize(&mut self.elements) {
            return self.fail_scene_transaction(before, error.to_string());
        }
        if is_attached(&self.elements[from]) {
            let element = self.elements.remove(from);
            self.elements.insert(to, element);
        } else if !self.reorder_object_base(from, to) {
            let error = self.status_message.clone();
            return self.fail_scene_transaction(before, error);
        }
        if moving_host {
            let order = self.layout_order();
            let mut elements: Vec<_> = std::mem::take(&mut self.elements)
                .into_iter()
                .map(Some)
                .collect();
            self.elements = order
                .into_iter()
                .map(|i| elements[i].take().unwrap())
                .collect();
        }
        let selected = selected_id.and_then(|id| {
            self.elements.iter().position(|element| {
                element
                    .as_object()
                    .and_then(|o| o.scene.as_ref())
                    .is_some_and(|s| s.id == id)
            })
        });
        self.finish_scene_transaction(
            before,
            selected,
            if crosses_sequence {
                "Moved to another sequence; fixed starts keep their X. Use After previous to join sequences."
            } else {
                "Host and attachments reordered; layer order preserved"
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ptouch_render::{document::PositionMode, shape::ShapeFill};

    fn fixture() -> AppState {
        let elements = (0..3)
            .map(|index| {
                let frame = TextBox {
                    x_mm: if index == 0 { 2.0 } else { 0.0 },
                    y_mm: 1.0,
                    width_mm: 10.0,
                    height_mm: 8.0,
                    gap_mm: 3.0,
                    position: if index == 0 {
                        PositionMode::Absolute
                    } else {
                        PositionMode::Relative
                    },
                    ..Default::default()
                };
                let mut object = LabelObject::text(format!("PORT {}", index + 1), frame);
                object.sequence = Some(7);
                object.content_box = Some(ContentBox {
                    width_mm: 8.0,
                    height_mm: 6.0,
                    offset_x_mm: 0.0,
                    offset_y_mm: 0.0,
                });
                object.layout = Some(LayoutOptions {
                    placement: Placement::Flow,
                    width: WidthSizing::Fixed,
                    constrain: true,
                    padding_mm: 0.0,
                });
                LabelElement::Object(object)
            })
            .collect();
        let mut state = AppState {
            elements,
            document_version: 11,
            needs_rerender: false,
            ..Default::default()
        };
        scene::enable(&mut state.elements).unwrap();
        state
    }

    fn worlds(state: &AppState) -> Vec<TextBox> {
        scene::world_frames(&state.elements).unwrap()
    }

    fn ids(state: &AppState) -> Vec<u64> {
        (0..state.elements.len())
            .map(|i| scene_id(&state.elements, i).unwrap())
            .collect()
    }

    #[test]
    fn host_reorder_moves_nonadjacent_children_as_a_block_and_child_drop_targets_host() {
        let mut state = fixture();
        assert!(state.add_overlay_text(0));
        assert!(state.add_overlay_text(0));
        let paint: Vec<_> = scene::paint_order(&state.elements)
            .into_iter()
            .map(|i| scene_id(&state.elements, i).unwrap())
            .collect();
        assert_eq!(state.layout_order(), vec![0, 3, 4, 1, 2]);
        assert_eq!(state.layout_neighbor(0, true), Some(1));
        assert_eq!(state.layout_neighbor(3, true), None);
        assert!(state.reorder_scene_element(0, 1));
        assert_eq!(ids(&state), vec![2, 1, 4, 5, 3]);
        assert_eq!(state.selected_element, Some(1));
        assert_close(worlds(&state)[0].x_mm, 2.0);
        assert_close(worlds(&state)[1].x_mm, 15.0);
        // Dropping the first host on a child of the second host addresses
        // that host and moves its whole block, without changing stacking.
        assert!(state.reorder_scene_element(0, 2));
        assert_eq!(ids(&state), vec![1, 4, 5, 2, 3]);
        assert_close(worlds(&state)[0].x_mm, 2.0);
        assert_eq!(
            scene::paint_order(&state.elements)
                .into_iter()
                .map(|i| scene_id(&state.elements, i).unwrap())
                .collect::<Vec<_>>(),
            paint
        );
    }

    #[test]
    fn crossing_sequences_keeps_both_origins_and_attached_children_with_the_host() {
        let mut state = fixture();
        let second = state.elements[2].as_object_mut().unwrap();
        second.sequence = Some(8);
        second.frame.position = PositionMode::Absolute;
        second.frame.x_mm = 52.5;
        assert!(state.add_overlay_text(0));
        assert!(state.reorder_scene_element(0, 2));
        assert_eq!(ids(&state), vec![2, 3, 1, 4]);
        let frames = worlds(&state);
        assert_close(frames[0].x_mm, 2.0);
        assert_close(frames[1].x_mm, 52.5);
        assert_close(frames[2].x_mm, 65.5);
        assert_eq!(state.elements[2].as_object().unwrap().sequence, Some(8));
        assert_eq!(
            anchor_of(state.elements[3].as_object().unwrap())
                .unwrap()
                .target,
            1
        );
        assert!(state.status_message.contains("fixed starts"));
    }

    #[test]
    fn arrows_stop_at_sequence_edges_and_connect_preserves_all_frames() {
        let mut state = fixture();
        let object = state.elements[2].as_object_mut().unwrap();
        object.sequence = Some(8);
        object.frame.position = PositionMode::Absolute;
        object.frame.x_mm = 52.5;
        assert!(state.add_overlay_text(1));
        assert_eq!(state.layout_neighbor(1, true), None);
        assert_eq!(state.layout_neighbor(2, false), None);
        assert_eq!(state.layout_neighbor(0, true), Some(1));
        let before = worlds(&state);
        assert!(state.connect_sequence(2));
        for (after, before) in worlds(&state).iter().zip(before) {
            assert_eq!(
                (after.x_mm, after.y_mm, after.width_mm, after.height_mm),
                (before.x_mm, before.y_mm, before.width_mm, before.height_mm)
            );
        }
        assert_eq!(state.layout_neighbor(1, true), Some(2));
        assert_eq!(state.document().version, 13);
        // Promoting the zero-gap follower preserves its dormant spacing.
        state.elements[1].as_object_mut().unwrap().frame.gap_mm = 0.0;
        assert!(state.reorder_scene_element(1, 0));
        let incoming = state.elements[0].as_object().unwrap().frame.gap_mm;
        assert_eq!(incoming, 0.0);
        assert!(state.duplicate_element(0));
        let duplicate = state.selected_element.unwrap();
        assert_eq!(
            state.elements[duplicate].as_object().unwrap().frame.gap_mm,
            0.0
        );
        assert!(state.delete_element(0));
        assert_eq!(state.elements[0].as_object().unwrap().frame.gap_mm, 0.0);
    }

    #[test]
    fn deleting_an_old_head_retains_the_promoted_members_original_gap() {
        let mut state = fixture();
        state.document_version = 12;
        state.elements[0].as_object_mut().unwrap().frame.gap_mm = 0.0;
        state.elements[1].as_object_mut().unwrap().frame.gap_mm = 2.8;
        state.elements[2].as_object_mut().unwrap().frame.gap_mm = 3.1;
        assert!(state.delete_element(0));
        assert_eq!(state.document_version, 13);
        assert_eq!(state.elements[0].as_object().unwrap().frame.gap_mm, 2.8);
        let geometry = worlds(&state);
        assert!(state.reorder_scene_element(0, 1));
        assert!(state.reorder_scene_element(1, 0));
        assert_eq!(worlds(&state), geometry);
    }

    #[test]
    fn generated_host_moves_restore_geometry_selection_and_layers() {
        let sample = ptouch_render::document::LabelDocument::from_toml_str(include_str!(
            "../../../examples/shapes-overlays.ptl"
        ))
        .unwrap();
        for case in 0..24 {
            let mut state = fixture();
            for index in 0..3 {
                let object = state.elements[index].as_object_mut().unwrap();
                object.content = match (case + index) % 6 {
                    1 => sample.elements[4].as_object().unwrap().content.clone(),
                    2 => sample.elements[3].as_object().unwrap().content.clone(),
                    3 => ObjectContent::Line {
                        pattern: Default::default(),
                    },
                    4 => ObjectContent::Padding,
                    5 => ObjectContent::CutMark,
                    _ => object.content.clone(),
                };
                object.frame.width_mm = 8.0 + ((case + index) % 11) as f64;
                object.frame.gap_mm = if (case + index) % 3 == 0 {
                    0.0
                } else {
                    2.8 + index as f64 / 10.0
                };
            }
            for host in 0..3 {
                for _ in 0..((case + host) % 3) {
                    assert!(state.add_overlay_text(host));
                }
            }
            scene::enable(&mut state.elements).unwrap();
            state.document_version = 13;
            state.elements = state
                .layout_order()
                .into_iter()
                .map(|i| state.elements[i].clone())
                .collect();
            state.validate_scene_geometry().unwrap();
            let baseline = state.document();
            let geometry = worlds(&state);
            let roots: Vec<_> = state
                .layout_order()
                .into_iter()
                .filter(|i| !is_attached(&state.elements[*i]))
                .map(|i| scene_id(&state.elements, i).unwrap())
                .collect();
            for from in 0..roots.len() {
                for to in 0..roots.len() {
                    if from == to {
                        continue;
                    }
                    let index_of = |s: &AppState, id| {
                        s.elements
                            .iter()
                            .position(|e| e.as_object().unwrap().scene.as_ref().unwrap().id == id)
                            .unwrap()
                    };
                    let a = index_of(&state, roots[from]);
                    let b = index_of(&state, roots[to]);
                    assert!(state.reorder_scene_element(a, b));
                    let moved = index_of(&state, roots[from]);
                    assert_eq!(state.selected_element, Some(moved));
                    let adjacent = if from < to { from + 1 } else { from - 1 };
                    let target = index_of(&state, roots[adjacent]);
                    assert!(state.reorder_scene_element(moved, target));
                    assert_eq!(state.document(), baseline, "case {case}, {from}->{to}");
                    assert_eq!(worlds(&state), geometry);
                    assert_eq!(state.selected_element, Some(index_of(&state, roots[from])));
                }
            }
        }
    }

    #[test]
    fn joining_after_an_attached_row_uses_the_host_without_editing_the_child() {
        let mut state = fixture();
        let second = state.elements[2].as_object_mut().unwrap();
        second.sequence = Some(8);
        second.frame.position = PositionMode::Absolute;
        second.frame.x_mm = 52.5;
        assert!(state.add_overlay_text(1));
        let child = state.elements.pop().unwrap();
        state.elements.insert(2, child.clone());
        assert_eq!(state.previous_layout_index(3), Some(1));
        let mut object = state.elements[3].as_object().unwrap().clone();
        object.frame.position = PositionMode::Relative;
        object.frame.gap_mm = 2.8;
        state.apply_object_edit(3, object).unwrap();
        assert_eq!(state.elements[2], child);
        assert_eq!(state.elements[3].as_object().unwrap().sequence, Some(7));
        assert_close(worlds(&state)[3].x_mm, 27.8);
    }

    #[test]
    fn enabling_flow_while_joining_a_legacy_policy_keeps_the_whole_sequence() {
        let mut state = fixture();
        let mut fourth = state.elements[2].clone();
        fourth.as_object_mut().unwrap().scene = None;
        fourth.as_object_mut().unwrap().sequence = Some(8);
        state.elements.push(fourth);
        scene::enable(&mut state.elements).unwrap();
        let head = state.elements[2].as_object_mut().unwrap();
        head.sequence = Some(8);
        head.layout = None;
        head.frame.position = PositionMode::Absolute;
        head.frame.x_mm = 52.5;
        let mut edited = head.clone();
        edited.layout = Some(LayoutOptions {
            placement: Placement::Flow,
            width: WidthSizing::Fixed,
            constrain: false,
            padding_mm: 0.0,
        });
        edited.frame.position = PositionMode::Relative;
        edited.frame.gap_mm = 27.5;
        state.apply_object_edit(2, edited).unwrap();
        assert!(
            state
                .elements
                .iter()
                .all(|e| e.as_object().unwrap().sequence == Some(7))
        );
        assert_eq!(state.flow_chain(0), Some(vec![0, 1, 2, 3]));
        assert_close(state.element_rect_mm(2).unwrap().0, 52.5);
    }

    #[test]
    fn rejected_block_reorder_restores_document_selection_and_revisions() {
        let mut state = fixture();
        assert!(state.add_overlay_text(1));
        let mut anchor = anchor_of(state.elements[3].as_object().unwrap())
            .unwrap()
            .clone();
        anchor.offset_x_mm = -8.0;
        assert!(state.set_overlay_anchor(3, anchor));
        let before = state.document();
        let selection = state.selected_element;
        let revision = state.edit_revision;
        assert!(!state.reorder_scene_element(1, 0));
        assert_eq!(state.document(), before);
        assert_eq!(state.selected_element, selection);
        assert_eq!(state.edit_revision, revision);
    }

    fn assert_close(left: f64, right: f64) {
        assert!((left - right).abs() < 1e-8, "{left} != {right}");
    }

    #[test]
    fn offset_drag_keeps_widget_identity_and_attaching_leaves_layout_mode() {
        let mut state = fixture();
        state.select_canvas_target(Some(CanvasTarget {
            element: 1,
            edit: CanvasEditTarget::Slot,
        }));
        assert!(state.attach_overlay(1, 0));
        assert_eq!(state.canvas_edit_target, CanvasEditTarget::Content);
        let generation = state.editor_generation;
        let revision = state.selection_revision;
        for _ in 0..20 {
            let mut anchor = anchor_of(state.elements[1].as_object().unwrap())
                .unwrap()
                .clone();
            anchor.offset_x_mm += 0.01;
            assert!(state.set_overlay_anchor(1, anchor));
        }
        assert_eq!(state.editor_generation, generation);
        assert_eq!(state.selection_revision, revision);
    }

    #[test]
    fn duplicate_preserves_internal_paint_order_when_ids_break_a_layer_tie() {
        let mut state = fixture();
        assert!(state.add_overlay_shape(1, ShapeKind::Rectangle, false));
        let host = state.elements[1]
            .as_object_mut()
            .unwrap()
            .scene
            .as_mut()
            .unwrap();
        host.id = 10;
        host.layer = 0;
        let child = state.elements[3]
            .as_object_mut()
            .unwrap()
            .scene
            .as_mut()
            .unwrap();
        child.layer = 0;
        child.anchor.as_mut().unwrap().target = 10;
        assert!(scene_id(&state.elements, 3) < scene_id(&state.elements, 1));
        assert!(state.duplicate_scene_element(1));
        let host_copy = state.selected_element.unwrap();
        let child_copy = scene::children(&state.elements, host_copy)[0];
        let order = scene::paint_order(&state.elements);
        assert!(
            order.iter().position(|&i| i == child_copy)
                < order.iter().position(|&i| i == host_copy)
        );
    }

    #[test]
    fn auto_width_text_is_measured_before_accepting_an_anchor_breaking_edit() {
        let mut state = fixture();
        state.elements.truncate(1);
        let object = state.elements[0].as_object_mut().unwrap();
        object.frame.x_mm = 0.0;
        object.sequence = None;
        object.layout.as_mut().unwrap().placement = Placement::Free;
        object.layout.as_mut().unwrap().width = WidthSizing::Content;
        object_layout::update_allocation(object);
        object_layout::resolve_object(
            object,
            12,
            180,
            "DejaVuSans",
            &mut ptouch_render::text::TextRenderer::new(),
        )
        .unwrap();
        assert!(state.add_overlay_shape(0, ShapeKind::Rectangle, false));
        let before = state.document();
        let mut object = state.elements[0].as_object().unwrap().clone();
        if let ObjectContent::Text { content, .. } = &mut object.content {
            *content = "I".into();
        }
        assert!(state.apply_scene_object_edit(0, object).is_err());
        assert_eq!(state.document(), before);
        assert!(state.scene_measurement_renderer.is_some());
    }

    #[test]
    fn layer_controls_change_only_metadata_and_keep_one_selection() {
        let mut state = fixture();
        assert!(state.add_overlay_shape(1, ShapeKind::Rectangle, false));
        let selected = state.selected_element;
        let before_worlds = worlds(&state);
        let before_order: Vec<_> = state
            .elements
            .iter()
            .map(|element| scene_id(std::slice::from_ref(element), 0).unwrap())
            .collect();
        assert!(state.change_layer(3, LayerMove::Back));
        assert_eq!(scene::paint_order(&state.elements)[0], 3);
        assert_eq!(worlds(&state), before_worlds);
        assert_eq!(state.selected_element, selected);
        assert!(state.change_layer(3, LayerMove::Forward));
        assert!(state.change_layer(3, LayerMove::Front));
        assert!(!state.change_layer(3, LayerMove::Front));
        assert!(state.reorder_layer(3, 1));
        assert_eq!(worlds(&state), before_worlds);
        assert_eq!(
            state
                .elements
                .iter()
                .map(|element| scene_id(std::slice::from_ref(element), 0).unwrap())
                .collect::<Vec<_>>(),
            before_order
        );
    }

    #[test]
    fn overlay_insertion_and_list_reorder_never_change_port_spacing_or_layers() {
        let mut state = fixture();
        let before_worlds = worlds(&state);
        assert!(state.add_overlay_text(1));
        assert_eq!(&worlds(&state)[..3], before_worlds);
        assert!(is_attached(&state.elements[3]));
        assert!(state.elements[3].as_object().unwrap().sequence.is_none());
        let layer_ids: Vec<_> = scene::paint_order(&state.elements)
            .into_iter()
            .map(|index| scene_id(&state.elements, index).unwrap())
            .collect();
        assert!(state.reorder_scene_element(3, 1));
        assert!(is_attached(&state.elements[1]));
        let resolved = worlds(&state);
        assert_eq!(resolved[0], before_worlds[0]);
        assert_eq!(resolved[2], before_worlds[1]);
        assert_eq!(resolved[3], before_worlds[2]);
        assert_eq!(
            scene::paint_order(&state.elements)
                .into_iter()
                .map(|index| scene_id(&state.elements, index).unwrap())
                .collect::<Vec<_>>(),
            layer_ids
        );
    }

    #[test]
    fn moving_or_rotating_an_anchored_child_updates_offsets_and_preserves_host() {
        let mut state = fixture();
        assert!(state.add_overlay_shape(1, ShapeKind::Rectangle, false));
        let before_worlds = worlds(&state);
        let mut moved = state.elements[3].as_object().unwrap().clone();
        moved.frame.x_mm += 1.25;
        state.apply_scene_object_edit(3, moved).unwrap();
        assert_close(worlds(&state)[3].x_mm, before_worlds[3].x_mm + 1.25);
        assert_eq!(&worlds(&state)[..3], &before_worlds[..3]);
        let mut rotated = state.elements[3].as_object().unwrap().clone();
        rotated.rotation = 90.0;
        rotated.content_box.as_mut().unwrap().height_mm = 3.0;
        rotated.content_box.as_mut().unwrap().width_mm = 4.0;
        object_layout::update_allocation(&mut rotated);
        let desired = rotated.frame.clone();
        state.apply_scene_object_edit(3, rotated).unwrap();
        assert_eq!(worlds(&state)[3], desired);
        assert_eq!(&worlds(&state)[..3], &before_worlds[..3]);
    }

    #[test]
    fn invalid_geometry_and_anchor_edits_restore_document_selection_and_preview_flags() {
        let mut state = fixture();
        assert!(state.add_overlay_shape(1, ShapeKind::Rectangle, false));
        state.needs_rerender = false;
        let before = state.document();
        let selected = state.selected_element;
        let revision = state.edit_revision;
        let mut invalid = state.elements[3].as_object().unwrap().clone();
        invalid.content_box.as_mut().unwrap().height_mm = 20.0;
        assert!(state.apply_scene_object_edit(3, invalid).is_err());
        assert_eq!(state.document(), before);
        assert_eq!(state.edit_revision, revision);
        assert!(!state.needs_rerender);
        let mut anchor = anchor_of(state.elements[3].as_object().unwrap())
            .unwrap()
            .clone();
        anchor.offset_y_mm = -100.0;
        assert!(!state.set_overlay_anchor(3, anchor));
        assert_eq!(state.document(), before);
        assert_eq!(state.selected_element, selected);
        assert_eq!(state.edit_revision, revision);
        assert!(!state.needs_rerender);
    }

    #[test]
    fn changing_alignment_and_detaching_preserve_the_current_world() {
        let mut state = fixture();
        assert!(state.add_overlay_text(1));
        let before = worlds(&state)[3].clone();
        assert!(state.set_overlay_alignment(3, 0.0, 1.0));
        assert_eq!(worlds(&state)[3], before);
        assert!(state.detach_overlay(3));
        assert!(!is_attached(&state.elements[3]));
        assert_eq!(worlds(&state)[3], before);
        assert!(!state.detach_overlay(3));
    }

    #[test]
    fn attaching_existing_flow_content_preserves_visible_world_and_detaches_sequence() {
        let mut state = fixture();
        let before = worlds(&state);
        assert!(state.can_attach_overlay(1, 0));
        assert!(state.attach_overlay(1, 0));
        let after = worlds(&state);
        assert_close(after[1].x_mm, before[1].x_mm + 1.0);
        assert_close(after[1].y_mm, before[1].y_mm + 1.0);
        assert_close(after[2].x_mm, before[2].x_mm);
        assert_close(after[2].y_mm, before[2].y_mm);
        assert_close(after[2].width_mm, before[2].width_mm);
        assert_close(after[2].height_mm, before[2].height_mm);
        assert!(state.elements[1].as_object().unwrap().sequence.is_none());
        assert!(!state.can_attach_overlay(0, 1));
        assert!(!state.can_attach_overlay(0, 2));
    }

    #[test]
    fn host_duplicate_copies_all_children_with_new_ids_and_matching_layer_relations() {
        let mut state = fixture();
        assert!(state.add_overlay_text(1));
        assert!(state.add_overlay_shape(1, ShapeKind::Ellipse, true));
        let old_world = worlds(&state);
        let old_ids: Vec<_> = state
            .elements
            .iter()
            .filter_map(|e| e.as_object()?.scene.as_ref().map(|s| s.id))
            .collect();
        assert!(state.duplicate_scene_element(1));
        let copy_index = state.selected_element.unwrap();
        let new_id = scene_id(&state.elements, copy_index).unwrap();
        assert!(!old_ids.contains(&new_id));
        let children = scene::children(&state.elements, copy_index);
        assert_eq!(children.len(), 2);
        assert_eq!(scene::children(&state.elements, 1).len(), 2);
        let new_world = worlds(&state);
        let shift = new_world[copy_index].x_mm - old_world[1].x_mm;
        for (&copy, original) in children.iter().zip([3, 4]) {
            assert_close(new_world[copy].x_mm, old_world[original].x_mm + shift);
            assert_close(new_world[copy].y_mm, old_world[original].y_mm);
            assert!(!old_ids.contains(&scene_id(&state.elements, copy).unwrap()));
        }
        scene::validate(&state.elements, 12).unwrap();
    }

    #[test]
    fn child_duplicate_retains_its_host_with_a_small_offset() {
        let mut state = fixture();
        assert!(state.add_overlay_text(1));
        let before_world = worlds(&state)[3].clone();
        let target = anchor_of(state.elements[3].as_object().unwrap())
            .unwrap()
            .target;
        assert!(state.duplicate_scene_element(3));
        let copy = state.selected_element.unwrap();
        assert_eq!(
            anchor_of(state.elements[copy].as_object().unwrap())
                .unwrap()
                .target,
            target
        );
        assert_close(worlds(&state)[copy].x_mm, before_world.x_mm + 0.5);
        assert_ne!(
            scene_id(&state.elements, copy),
            scene_id(&state.elements, 3)
        );
    }

    #[test]
    fn deleting_host_freezes_children_before_sequence_reflow_and_undo_restores_it() {
        let mut state = fixture();
        assert!(state.add_overlay_text(0));
        let before = state.document();
        let child_id = scene_id(&state.elements, 3).unwrap();
        let child_world = worlds(&state)[3].clone();
        state
            .history
            .observe(before.clone(), Some(0), false, false, 0.0);
        assert!(state.delete_scene_element(0));
        let child = state
            .elements
            .iter()
            .position(|e| {
                e.as_object()
                    .and_then(|o| o.scene.as_ref())
                    .is_some_and(|s| s.id == child_id)
            })
            .unwrap();
        assert!(!is_attached(&state.elements[child]));
        assert_eq!(worlds(&state)[child], child_world);
        assert_close(worlds(&state)[0].x_mm, 2.0);
        state
            .history
            .observe(state.document(), state.selected_element, false, false, 1.0);
        state.history_step(false);
        assert_eq!(state.document(), before);
    }

    #[test]
    fn deleting_a_child_does_not_reanchor_the_following_port() {
        let mut state = fixture();
        assert!(state.add_overlay_text(0));
        assert!(state.reorder_scene_element(3, 1));
        let before = worlds(&state);
        assert!(state.delete_scene_element(1));
        let after = worlds(&state);
        assert_eq!(after[0], before[0]);
        assert_eq!(after[1], before[2]);
        assert_eq!(after[2], before[3]);
    }

    #[test]
    fn shape_defaults_fit_the_printable_strip_and_keep_circle_dimensions_equal() {
        let mut state = AppState::default();
        assert!(state.add_shape(ShapeKind::Rectangle, false));
        assert!(state.add_shape(ShapeKind::Ellipse, true));
        let frames = worlds(&state);
        assert!(frames[1].x_mm > frames[0].x_mm + frames[0].width_mm);
        assert_close(frames[1].width_mm, frames[1].height_mm);
        for object in state.elements.iter().filter_map(LabelElement::as_object) {
            let ObjectContent::Shape { style } = &object.content else {
                panic!("expected shape");
            };
            assert_eq!(style.fill, ShapeFill::None);
            assert_eq!(object.layout.as_ref().unwrap().placement, Placement::Free);
            assert!(object.sequence.is_none());
        }
        state.validate_scene_geometry().unwrap();
    }
}
