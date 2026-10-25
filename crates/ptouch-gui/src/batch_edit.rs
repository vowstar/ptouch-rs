// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Batch edits commit the complete candidate or restore the original state.

use std::collections::{BTreeMap, BTreeSet};

use ptouch_render::{
    document::{LabelElement, ObjectContent},
    object_layout::{Placement, WidthSizing},
    scene,
    styled_text::{StylePatch, StyledText},
};

use crate::{scene_edit::SceneSnapshot, state::AppState};

#[derive(Clone, Debug)]
pub enum SharedEdit {
    Width(f64),
    Height(f64),
    ItemWidth(f64),
    Gap(f64),
    Rotation(f32),
    FontSize(f32),
    Style(StylePatch),
    Containment(bool),
}

fn scene_id(element: &LabelElement) -> Option<u64> {
    Some(element.as_object()?.scene.as_ref()?.id)
}

fn footprint(element: &LabelElement) -> usize {
    if let Some(object) = element.as_object() {
        match &object.content {
            ObjectContent::Image {
                image_data,
                bitmap,
                coverage,
                ..
            } => {
                image_data.len()
                    + bitmap.as_ref().map_or(0, |bitmap| bitmap.data().len())
                    + coverage.as_ref().map_or(0, |mask| mask.data().len())
                    + 512
            }
            ObjectContent::Text {
                content, styles, ..
            } => {
                content.len()
                    + styles
                        .iter()
                        .map(|span| {
                            std::mem::size_of_val(span)
                                + span.style.font_name.as_ref().map_or(0, String::len)
                        })
                        .sum::<usize>()
                    + 512
            }
            _ => 512,
        }
    } else {
        512
    }
}

impl AppState {
    pub fn can_translate_selection(&self) -> bool {
        let selected = self.selected_indices();
        let ids: BTreeSet<_> = selected
            .iter()
            .filter_map(|index| scene_id(&self.elements[*index]))
            .collect();
        !selected.is_empty()
            && selected.iter().all(|index| {
                self.elements[*index].as_object().is_some_and(|object| {
                    object
                        .scene
                        .as_ref()
                        .and_then(|scene| scene.anchor.as_ref())
                        .is_some_and(|anchor| ids.contains(&anchor.target))
                        || object
                            .layout
                            .as_ref()
                            .is_some_and(|layout| layout.placement == Placement::Free)
                })
            })
    }

    pub fn translate_selection(&mut self, requested: egui::Vec2) -> Result<egui::Vec2, String> {
        if !requested.is_finite() {
            return Err("Enter a finite movement distance.".into());
        }
        if self.overall_flip_h {
            return Err("Clear Flip horizontal before moving objects together.".into());
        }
        let selected = self.selected_indices();
        if selected.is_empty() {
            return Ok(egui::Vec2::ZERO);
        }
        let ids: BTreeSet<_> = selected
            .iter()
            .filter_map(|index| scene_id(&self.elements[*index]))
            .collect();
        let roots: Vec<_> = selected
            .iter()
            .copied()
            .filter(|index| {
                self.elements[*index]
                    .as_object()
                    .and_then(|object| object.scene.as_ref())
                    .and_then(|scene| scene.anchor.as_ref())
                    .is_none_or(|anchor| !ids.contains(&anchor.target))
            })
            .collect();
        if roots.iter().any(|index| {
            self.elements[*index].as_object().is_none_or(|object| {
                object
                    .layout
                    .as_ref()
                    .is_none_or(|layout| layout.placement != Placement::Free)
            })
        }) {
            return Err("Use free positioning to move these objects together.".into());
        }
        let frames = scene::world_frames(&self.elements).map_err(|error| error.to_string())?;
        let mut affected: BTreeSet<_> = roots.iter().copied().collect();
        for index in &roots {
            affected.extend(scene::children(&self.elements, *index));
        }
        let min_x = affected
            .iter()
            .map(|index| frames[*index].x_mm)
            .fold(f64::INFINITY, f64::min);
        let min_y = affected
            .iter()
            .map(|index| frames[*index].y_mm)
            .fold(f64::INFINITY, f64::min);
        let bottom = affected
            .iter()
            .map(|index| frames[*index].y_mm + frames[*index].height_mm)
            .fold(0.0, f64::max);
        let dx = f64::from(requested.x).max(-min_x);
        let dy =
            f64::from(requested.y).clamp(-min_y, (self.printable_height_mm() - bottom).max(-min_y));
        if dx == 0.0 && dy == 0.0 {
            return Ok(egui::Vec2::ZERO);
        }
        let all_frames: Vec<_> = self
            .elements
            .iter()
            .map(|element| element.as_object().map(|object| object.frame.clone()))
            .collect();
        let anchors: Vec<_> = roots
            .iter()
            .map(|index| {
                (
                    *index,
                    self.elements[*index]
                        .as_object()
                        .unwrap()
                        .scene
                        .as_ref()
                        .and_then(|scene| scene.anchor.clone()),
                )
            })
            .collect();
        for index in &roots {
            let object = self.elements[*index].as_object_mut().unwrap();
            object.frame.x_mm = frames[*index].x_mm + dx;
            object.frame.y_mm = frames[*index].y_mm + dy;
            if let Some(anchor) = object
                .scene
                .as_ref()
                .and_then(|scene| scene.anchor.as_ref())
                .cloned()
            {
                let host = frames[self
                    .elements
                    .iter()
                    .position(|element| scene_id(element) == Some(anchor.target))
                    .ok_or("The host object is not available.")?]
                .clone();
                let object = self.elements[*index].as_object_mut().unwrap();
                let mut next = scene::offset_for_frame(&anchor, &object.frame, &host);
                if dx.abs() < 1e-9 {
                    next.gap_center = anchor.gap_center;
                }
                object.scene.as_mut().unwrap().anchor = Some(next);
            }
        }
        let result = self.validate_scene_geometry().and_then(|()| {
            let frames = scene::world_frames(&self.elements).map_err(|error| error.to_string())?;
            let right = frames
                .iter()
                .map(|frame| frame.x_mm + frame.width_mm)
                .fold(0.0, f64::max);
            let width = (right * f64::from(self.target_dpi()) / 25.4).round();
            if !width.is_finite()
                || width > 1_000_000.0
                || width * f64::from(self.tape_width_px) > 67_108_864.0
            {
                return Err("This movement would exceed the label size limit.".into());
            }
            Ok(())
        });
        if let Err(message) = result {
            for (element, frame) in self.elements.iter_mut().zip(all_frames) {
                if let (Some(object), Some(frame)) = (element.as_object_mut(), frame) {
                    object.frame = frame;
                }
            }
            for (index, anchor) in anchors {
                if let Some(scene) = self.elements[index].as_object_mut().unwrap().scene.as_mut() {
                    scene.anchor = anchor;
                }
            }
            return Err(message);
        }
        self.mark_dirty();
        Ok(egui::vec2(dx as f32, dy as f32))
    }

    pub(crate) fn check_batch(&mut self) -> Result<(), String> {
        if self.is_free_layout() {
            self.validate_scene_geometry()?;
        }
        let document = self.document();
        let dpi = self.target_dpi();
        let height = self.tape_width_px;
        let renderer = self
            .scene_measurement_renderer
            .get_or_insert_with(ptouch_render::text::TextRenderer::new);
        ptouch_render::document::render_document(&document, height, dpi, renderer)
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn delete_selection(&mut self) -> bool {
        let selected = self.selected_indices();
        if selected.is_empty() {
            return false;
        }
        let mut remove: BTreeSet<_> = selected.into_iter().collect();
        for index in remove.clone() {
            remove.extend(scene::children(&self.elements, index));
        }
        let snapshot = SceneSnapshot::take(self);
        let result = (|| {
            for index in remove.iter().rev().copied() {
                if !self.delete_element(index) {
                    return Err(self.status_message.clone());
                }
            }
            self.check_batch()?;
            Ok(())
        })();
        if let Err(message) = result {
            snapshot.restore(self);
            self.status_message = message;
            return false;
        }
        let next = self
            .selected_element
            .filter(|index| *index < self.elements.len());
        self.set_selected_indices(&next.into_iter().collect::<Vec<_>>(), next);
        self.status_message = format!("Deleted {} objects.", remove.len());
        true
    }

    pub fn copy_selection(&mut self) -> bool {
        let selected = self.selected_indices();
        if selected.is_empty() {
            return false;
        }
        if selected.len() == 1 {
            let index = selected[0];
            let mut closure = scene::children(&self.elements, index);
            closure.push(index);
            let bytes = closure
                .iter()
                .map(|index| footprint(&self.elements[*index]))
                .sum::<usize>();
            if bytes > 64 * 1024 * 1024 || self.elements.len().saturating_add(closure.len()) > 4096
            {
                self.status_message = "This selection is too large to copy.".into();
                return false;
            }
            let snapshot = SceneSnapshot::take(self);
            let edit = self.canvas_edit_target;
            if !self.duplicate_element(index) {
                let message = self.status_message.clone();
                snapshot.restore(self);
                self.status_message = message;
                return false;
            }
            if let Err(message) = self.check_batch() {
                snapshot.restore(self);
                self.status_message = message;
                return false;
            }
            let primary = self.selected_element;
            self.set_selected_indices(&primary.into_iter().collect::<Vec<_>>(), primary);
            self.select_canvas_target(
                primary.map(|element| crate::state::CanvasTarget { element, edit }),
            );
            return true;
        }
        if !self.is_free_layout() {
            self.status_message = "Convert the layout before you copy multiple objects.".into();
            return false;
        }
        let snapshot = SceneSnapshot::take(self);
        let result = self.copy_selection_inner(&selected).and_then(|ids| {
            self.check_batch()?;
            Ok(ids)
        });
        match result {
            Ok(ids) => {
                let indices: Vec<_> = self
                    .elements
                    .iter()
                    .enumerate()
                    .filter_map(|(index, element)| {
                        ids.contains(&scene_id(element)?).then_some(index)
                    })
                    .collect();
                self.set_selected_indices(&indices, indices.first().copied());
                self.status_message = format!("Copied {} objects.", indices.len());
                true
            }
            Err(message) => {
                snapshot.restore(self);
                self.status_message = message;
                false
            }
        }
    }

    fn copy_selection_inner(&mut self, selected: &[usize]) -> Result<BTreeSet<u64>, String> {
        let selected_set: BTreeSet<_> = selected.iter().copied().collect();
        let source_ids: BTreeSet<_> = selected
            .iter()
            .filter_map(|index| scene_id(&self.elements[*index]))
            .collect();
        let roots: Vec<_> = selected
            .iter()
            .copied()
            .filter(|index| {
                self.elements[*index]
                    .as_object()
                    .and_then(|object| object.scene.as_ref())
                    .and_then(|scene| scene.anchor.as_ref())
                    .is_none_or(|anchor| !source_ids.contains(&anchor.target))
            })
            .collect();
        let mut all = selected_set;
        for index in &roots {
            all.extend(scene::children(&self.elements, *index));
        }
        let bytes = all
            .iter()
            .try_fold(0usize, |sum, index| {
                sum.checked_add(footprint(&self.elements[*index]))
            })
            .ok_or("This selection is too large to copy.")?;
        if bytes > 64 * 1024 * 1024 || self.elements.len().saturating_add(all.len()) > 4096 {
            return Err("This selection is too large to copy.".into());
        }
        scene::enable(&mut self.elements).map_err(|error| error.to_string())?;
        ptouch_render::sequence::materialize(&mut self.elements)
            .map_err(|error| error.to_string())?;
        self.document_version = self.document_version.max(13);
        let source_paint: Vec<_> = scene::paint_order(&self.elements)
            .into_iter()
            .filter_map(|index| scene_id(&self.elements[index]))
            .collect();
        let mut mapping = BTreeMap::new();
        let mut groups: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
        let mut free = Vec::new();
        for index in roots {
            let object = self.elements[index].as_object().unwrap();
            let id = object.scene.as_ref().unwrap().id;
            if let Some(sequence) = object.sequence {
                groups.entry(sequence).or_default().push(id);
            } else {
                free.push(id);
            }
        }
        let mut copied = BTreeSet::new();
        for sources in groups.values() {
            let mut target = *sources.last().unwrap();
            for source in sources {
                let index = self.index_for_scene(*source)?;
                let next = self.duplicate_batch_root(index)?;
                copied.extend(next.values().copied());
                mapping.extend(next);
                let copy = self
                    .selected_element
                    .ok_or("The copied object is not available.")?;
                let copy_id = scene_id(&self.elements[copy]).unwrap();

                let destination = self.index_for_scene(target)?;
                if self.previous_layout_index(copy) != Some(destination)
                    && !self.reorder_object(copy, destination)
                {
                    return Err(self.status_message.clone());
                }
                target = copy_id;
            }
        }
        let frames = scene::world_frames(&self.elements).map_err(|error| error.to_string())?;
        let free_hosts: Vec<_> = free
            .iter()
            .filter_map(|id| {
                let index = self.index_for_scene(*id).ok()?;
                self.elements[index]
                    .as_object()?
                    .scene
                    .as_ref()?
                    .anchor
                    .is_none()
                    .then_some(index)
            })
            .collect();
        let right = frames
            .iter()
            .map(|frame| frame.x_mm + frame.width_mm)
            .fold(0.0, f64::max);
        let first = free_hosts
            .iter()
            .map(|index| frames[*index].x_mm)
            .fold(f64::INFINITY, f64::min);
        let free_offset = right + 3.0 - first;
        for source in free {
            let index = self.index_for_scene(source)?;
            let source_frame = scene::world_frames(&self.elements)
                .map_err(|error| error.to_string())?[index]
                .clone();
            let attached = self.elements[index]
                .as_object()
                .unwrap()
                .scene
                .as_ref()
                .unwrap()
                .anchor
                .is_some();
            let next = self.duplicate_batch_root(index)?;
            copied.extend(next.values().copied());
            mapping.extend(next);
            let copy = self
                .selected_element
                .ok_or("The copied object is not available.")?;

            let mut candidate = self.elements[copy].as_object().unwrap().clone();
            candidate.frame.x_mm = source_frame.x_mm + if attached { 3.0 } else { free_offset };
            candidate.frame.y_mm = source_frame.y_mm;
            self.apply_scene_object_edit(copy, candidate)?;
        }
        let mut paint: Vec<_> = source_paint.clone();
        paint.extend(
            source_paint
                .into_iter()
                .filter_map(|id| mapping.get(&id).copied()),
        );
        for (rank, id) in paint.into_iter().enumerate() {
            let index = self.index_for_scene(id)?;
            self.elements[index]
                .as_object_mut()
                .unwrap()
                .scene
                .as_mut()
                .unwrap()
                .layer = i32::try_from(rank).map_err(|_| "The layer limit has been reached.")?;
        }
        Ok(copied)
    }

    fn duplicate_batch_root(&mut self, index: usize) -> Result<BTreeMap<u64, u64>, String> {
        let mut subset: BTreeSet<_> = scene::children(&self.elements, index).into_iter().collect();
        subset.insert(index);
        let order: Vec<_> = scene::paint_order(&self.elements)
            .into_iter()
            .filter(|index| subset.contains(index))
            .filter_map(|index| scene_id(&self.elements[index]))
            .collect();
        let before: BTreeSet<_> = self.elements.iter().filter_map(scene_id).collect();
        if !self.duplicate_element(index) {
            return Err(self.status_message.clone());
        }
        let created: Vec<_> = scene::paint_order(&self.elements)
            .into_iter()
            .filter_map(|index| scene_id(&self.elements[index]))
            .filter(|id| !before.contains(id))
            .collect();
        if created.len() != order.len() {
            return Err("The copied objects could not be matched.".into());
        }
        Ok(order.into_iter().zip(created).collect())
    }

    pub(crate) fn index_for_scene(&self, id: u64) -> Result<usize, String> {
        self.elements
            .iter()
            .position(|element| scene_id(element) == Some(id))
            .ok_or_else(|| "The object is not available.".into())
    }

    pub fn apply_shared_edit(&mut self, edit: SharedEdit) -> bool {
        let indices = self.selected_indices();
        if indices.is_empty() {
            return false;
        }
        let selection = indices.clone();
        let primary = self.selected_element;
        let snapshot = SceneSnapshot::take(self);
        let result = (|| {
            for index in indices {
                let before = self.elements[index]
                    .as_object()
                    .ok_or("Convert the layout before you change multiple objects.")?
                    .clone();
                let mut object = before.clone();
                match &edit {
                    SharedEdit::Width(value) | SharedEdit::Height(value) => {
                        if !value.is_finite() || *value <= 0.0 || *value > 10_000.0 {
                            return Err("Enter valid object dimensions.".into());
                        }
                        let mut dimensions = self
                            .resolved_content_box(index)
                            .ok_or("The object dimensions are not available.")?;
                        let ratio = if matches!(edit, SharedEdit::Width(_)) {
                            *value / dimensions.width_mm
                        } else {
                            *value / dimensions.height_mm
                        };
                        dimensions.width_mm *= ratio;
                        dimensions.height_mm *= ratio;
                        object.content_box = Some(dimensions);
                        if let Some(layout) = object.layout.as_mut() {
                            layout.width = WidthSizing::Fixed;
                        }
                        if let ObjectContent::Text {
                            font_size, styles, ..
                        } = &mut object.content
                        {
                            if let Some(size) = font_size {
                                *size *= ratio as f32;
                            }
                            for span in styles {
                                if let Some(size) = &mut span.style.font_size {
                                    *size *= ratio as f32;
                                }
                            }
                        }
                    }
                    SharedEdit::ItemWidth(value) | SharedEdit::Gap(value) => {
                        if !value.is_finite() || *value < 0.0 || *value > 10_000.0 {
                            return Err("Enter a valid item width or gap.".into());
                        }
                        if object
                            .scene
                            .as_ref()
                            .is_some_and(|scene| scene.anchor.is_some())
                        {
                            continue;
                        }
                        if object
                            .layout
                            .as_ref()
                            .is_none_or(|layout| layout.placement != Placement::Flow)
                        {
                            return Err("Select sequence items to change their spacing.".into());
                        }
                        if matches!(edit, SharedEdit::ItemWidth(_)) {
                            if *value == 0.0 {
                                return Err("Item width must be greater than zero.".into());
                            }
                            object.frame.width_mm = *value;
                            object.layout.as_mut().unwrap().width = WidthSizing::Fixed;
                        } else {
                            object.frame.gap_mm = *value;
                        }
                    }
                    SharedEdit::Rotation(value) => {
                        if !value.is_finite() {
                            return Err("Enter a valid angle.".into());
                        }
                        object.rotation = *value;
                    }
                    SharedEdit::FontSize(value) => {
                        if !value.is_finite() || !(0.1..=512.0).contains(value) {
                            return Err("Enter a font size from 0.1 to 512 points.".into());
                        }
                        let ObjectContent::Text {
                            content,
                            font_size,
                            styles,
                            ..
                        } = &mut object.content
                        else {
                            return Err("Select text objects to change the font.".into());
                        };
                        *font_size = Some(*value);
                        let mut styled = StyledText {
                            text: content.clone(),
                            spans: styles.clone(),
                        };
                        let len = styled.len();
                        styled.format(
                            0..len,
                            &StylePatch {
                                font_size: Some(None),
                                ..Default::default()
                            },
                        );
                        *styles = styled.spans;
                    }
                    SharedEdit::Style(patch) => {
                        let ObjectContent::Text {
                            content, styles, ..
                        } = &mut object.content
                        else {
                            return Err("Select text objects to change the text format.".into());
                        };
                        let mut styled = StyledText {
                            text: content.clone(),
                            spans: styles.clone(),
                        };
                        let len = styled.len();
                        styled.format(0..len, patch);
                        *styles = styled.spans;
                    }
                    SharedEdit::Containment(enabled) => {
                        object
                            .layout
                            .as_mut()
                            .ok_or("Set the object layout first.")?
                            .constrain = *enabled;
                    }
                }
                if object == before {
                    continue;
                }
                let needs_transform = matches!(
                    edit,
                    SharedEdit::Width(_) | SharedEdit::Height(_) | SharedEdit::Rotation(_)
                );
                if needs_transform
                    && self
                        .constrain_transform(index, &before, &mut object)
                        .map_err(str::to_string)?
                {
                    return Err("An object cannot keep its position at this size.".into());
                }
                self.apply_object_edit(index, object)?;
            }
            self.check_batch()?;
            Ok(())
        })();
        if let Err(message) = result {
            snapshot.restore(self);
            self.status_message = message;
            return false;
        }
        self.set_selected_indices(&selection, primary);
        self.status_message = "Changed selected objects.".into();
        true
    }
}
