// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Change layout relationships without changing the visible object geometry.

use ptouch_render::{
    document::{LabelElement, PositionMode, TextBox},
    object_layout::{LayoutOptions, Placement, WidthSizing, content_extent},
    scene,
};

use crate::{scene_edit::SceneSnapshot, state::AppState};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Positioning {
    Free,
    Sequence,
    Absolute,
    AfterPrevious,
}

impl AppState {
    /// Resolve every position together. Relative fields do not contain world X.
    pub fn position_frames(&self) -> Vec<Option<TextBox>> {
        if self.is_free_layout() {
            return scene::world_frames(&self.elements)
                .map(|frames| frames.into_iter().map(Some).collect())
                .unwrap_or_else(|_| vec![None; self.elements.len()]);
        }
        (0..self.elements.len())
            .map(|index| {
                let (x, y, width, height) = self.element_rect_mm(index)?;
                Some(TextBox {
                    x_mm: x,
                    y_mm: y,
                    width_mm: width,
                    height_mm: height,
                    ..Default::default()
                })
            })
            .collect()
    }

    /// A relationship change is one transaction. Drag clamping is not used.
    pub fn set_positioning(&mut self, index: usize, choice: Positioning) -> Result<bool, String> {
        let original = self
            .elements
            .get(index)
            .and_then(LabelElement::as_object)
            .ok_or("Select an object first.")?
            .clone();
        if original.scene.as_ref().is_some_and(|s| s.anchor.is_some()) {
            return Err("Before you change the position mode, select Detach.".into());
        }
        let placement = original.layout.as_ref().map(|p| p.placement);
        let unchanged = match choice {
            Positioning::Free => placement == Some(Placement::Free),
            Positioning::Sequence => placement == Some(Placement::Flow),
            Positioning::Absolute => original.frame.position == PositionMode::Absolute,
            Positioning::AfterPrevious => original.frame.position == PositionMode::Relative,
        };
        if unchanged {
            return Ok(false);
        }
        if original.content_box.is_none()
            && matches!(choice, Positioning::Free | Positioning::Sequence)
        {
            return Err(
                "Before you change the position mode, clear Follow allocation size.".into(),
            );
        }
        let frames = self.position_frames();
        let world = frames
            .get(index)
            .and_then(Option::as_ref)
            .ok_or("The object position is not available.")?;
        let previous_free = self.previous_layout_index(index).and_then(|previous| {
            let object = self.elements[previous].as_object()?;
            (object.layout.as_ref()?.placement == Placement::Free)
                .then(|| (previous, object.frame.clone(), object.content_box.clone()))
        });
        let mirrored_lengths = self.mirrored_lengths();
        let hosts: BTreeMap<_, _> = self
            .elements
            .iter()
            .enumerate()
            .filter_map(|(index, element)| Some((element.as_object()?.scene.as_ref()?.id, index)))
            .collect();
        let children: Vec<_> = self
            .elements
            .iter()
            .enumerate()
            .filter_map(|(child, element)| {
                let anchor = element.as_object()?.scene.as_ref()?.anchor.as_ref()?;
                let host = *hosts.get(&anchor.target)?;
                Some((child, host))
            })
            .collect();
        let mut object = original.clone();
        if matches!(choice, Positioning::Free | Positioning::Sequence)
            || choice == Positioning::AfterPrevious && placement == Some(Placement::Free)
        {
            object.content_box = Some(
                self.resolved_content_box(index)
                    .ok_or("The object dimensions are not available.")?,
            );
            object.layout = Some(LayoutOptions {
                placement: Placement::Flow,
                width: WidthSizing::Fixed,
                constrain: false,
                padding_mm: 0.0,
            });
            object.frame.x_mm = world.x_mm;
            if choice == Positioning::Free {
                let (dx, dy, width, height) =
                    content_extent(&object).ok_or("The object dimensions are not available.")?;
                object.frame.x_mm = world.x_mm + world.width_mm / 2.0 + dx;
                object.frame.y_mm = world.y_mm + world.height_mm / 2.0 + dy;
                object.frame.width_mm = width;
                object.frame.height_mm = height;
                object.frame.position = PositionMode::Absolute;
                let content = object.content_box.as_mut().unwrap();
                content.offset_x_mm = 0.0;
                content.offset_y_mm = 0.0;
                let policy = object.layout.as_mut().unwrap();
                policy.placement = Placement::Free;
                policy.constrain = true;
            }
        }
        if choice != Positioning::Free {
            let absolute = choice == Positioning::Absolute
                || choice == Positioning::Sequence && self.previous_layout_index(index).is_none();
            if absolute {
                object.frame.position = PositionMode::Absolute;
                object.frame.x_mm = world.x_mm;
            } else {
                let previous = self
                    .previous_layout_index(index)
                    .and_then(|previous| frames.get(previous)?.as_ref())
                    .ok_or("A sequence must start at an absolute position.")?;
                let gap = world.x_mm - previous.x_mm - previous.width_mm;
                if gap < -1e-7 {
                    return Err("The objects have an overlap. Keep the absolute position or move this object first.".into());
                }
                object.frame.position = PositionMode::Relative;
                object.frame.gap_mm = gap.max(0.0);
            }
        }
        let snapshot = SceneSnapshot::take(self);
        let result = (|| {
            // The base transaction changes sequence links without checking an
            // intermediate attachment position against the tape boundaries.
            self.apply_object_edit_base(index, object)?;
            if let Some((previous, frame, content)) = previous_free
                && matches!(choice, Positioning::Sequence | Positioning::AfterPrevious)
            {
                let previous = self.elements[previous].as_object_mut().unwrap();
                previous.frame.x_mm = frame.x_mm;
                previous.frame.y_mm = frame.y_mm;
                previous.frame.width_mm = frame.width_mm;
                previous.frame.height_mm = frame.height_mm;
                previous.content_box = content;
            }
            let next_frames = self.position_frames();
            for (child, host) in children {
                if frames[host] == next_frames[host] {
                    continue;
                }
                let old_world = frames[child]
                    .as_ref()
                    .ok_or("The attached object position is not available.")?;
                let child = self.elements[child].as_object_mut().unwrap();
                let metadata = child.scene.as_mut().unwrap();
                metadata.anchor = Some(scene::offset_for_frame(
                    metadata.anchor.as_ref().unwrap(),
                    old_world,
                    next_frames[host]
                        .as_ref()
                        .ok_or("The object position is not available.")?,
                ));
            }
            self.validate_scene_geometry()?;
            if !mirrored_lengths.is_empty() {
                let document = self.document();
                let renderer = self
                    .scene_measurement_renderer
                    .get_or_insert_with(ptouch_render::text::TextRenderer::new);
                for (dpi, height, width) in mirrored_lengths {
                    let result =
                        ptouch_render::document::render_document(&document, height, dpi, renderer)
                            .map_err(|error| error.to_string())?;
                    if result.bitmap.as_ref().map(|bitmap| bitmap.width()) != width {
                        return Err("Clear Flip horizontal in the left panel, then change the position mode.".into());
                    }
                }
            }
            Ok(())
        })();
        if let Err(error) = result {
            snapshot.restore(self);
            return Err(error);
        }
        Ok(true)
    }

    /// Whole-label reflection uses the final raster length as its axis.
    /// Compare only profiles that can already render the current document.
    fn mirrored_lengths(&mut self) -> Vec<(u16, u32, Option<u32>)> {
        if !self.overall_flip_h {
            return Vec::new();
        }
        let document = self.document();
        let target = self.target_dpi();
        let target_height = self.tape_width_px;
        let renderer = self
            .scene_measurement_renderer
            .get_or_insert_with(ptouch_render::text::TextRenderer::new);
        let mut resolutions = vec![180, 360, target];
        resolutions.sort_unstable();
        resolutions.dedup();
        resolutions
            .into_iter()
            .filter_map(|dpi| {
                let height = if dpi == target {
                    target_height
                } else {
                    u32::from(ptouch_core::tape::find_tape(self.tape_width_mm, dpi)?.pixels)
                };
                let result =
                    ptouch_render::document::render_document(&document, height, dpi, renderer)
                        .ok()?;
                Some((
                    dpi,
                    height,
                    result.bitmap.as_ref().map(|bitmap| bitmap.width()),
                ))
            })
            .collect()
    }
}
