// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Flat drawing order and one-level attachments to physical layout boxes.
//!
//! Element order still determines sequence placement. Drawing order and host
//! identity are persistent, so arranging objects cannot move a sequence and
//! reordering a sequence cannot change the scene's stacking order.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    RenderError, Result,
    document::{DOCUMENT_VERSION, LabelDocument, LabelElement, PositionMode, TextBox},
    object_layout::Placement,
};

/// Explicit scene metadata, introduced only by an intentional v12 edit.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneOptions {
    pub id: u64,
    #[serde(default)]
    pub layer: i32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
}

/// Follow an unrotated layout box, without inheriting its content transforms.
///
/// Fractions select start/center/end alignment (0/0.5/1); the millimeter
/// offsets move the child independently of the selected alignment.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub target: u64,
    pub horizontal: f64,
    pub vertical: f64,
    pub offset_x_mm: f64,
    pub offset_y_mm: f64,
    /// Follow the center of the gap after the host. Absent keeps manual X.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gap_center: Option<GapCenter>,
}

/// Use the actual next gap, or this saved gap when the host has no successor.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GapCenter {
    pub tail_gap_mm: f64,
}

impl GapCenter {
    pub fn validate(&self) -> Result<()> {
        if !self.tail_gap_mm.is_finite() || !(0.0..=10_000.0).contains(&self.tail_gap_mm) {
            return Err(layout_error("Tail gap must be between 0 and 10000 mm."));
        }
        Ok(())
    }
}

fn layout_error(message: &str) -> RenderError {
    RenderError::Layout(message.into())
}

fn has_scene(elements: &[LabelElement]) -> bool {
    elements
        .iter()
        .any(|element| element.as_object().is_some_and(|o| o.scene.is_some()))
}

fn validate_metadata(
    elements: &[LabelElement],
    version: u32,
    scenes: &[Option<&SceneOptions>],
) -> Result<()> {
    if version < 12 {
        return Err(layout_error("scene objects require layout version 12"));
    }
    let mut ids = BTreeMap::new();
    for (index, (element, scene)) in elements.iter().zip(scenes).enumerate() {
        if element.as_object().is_none() {
            return Err(layout_error(
                "convert legacy elements before editing shapes, layers or attachments",
            ));
        }
        let scene = scene.ok_or_else(|| {
            layout_error("all objects in a scene require persistent scene metadata")
        })?;
        if scene.id == 0 || scene.id > i64::MAX as u64 {
            return Err(layout_error("scene identifiers must be in 1..=i64::MAX"));
        }
        if ids.insert(scene.id, index).is_some() {
            return Err(layout_error("scene identifiers must be unique"));
        }
    }
    for (index, scene) in scenes.iter().enumerate() {
        let scene = scene.expect("metadata completeness checked above");
        let Some(anchor) = &scene.anchor else {
            continue;
        };
        if let Some(gap) = &anchor.gap_center {
            if version < 14 {
                return Err(layout_error(
                    "Gap center attachments require layout version 14.",
                ));
            }
            gap.validate()?;
        }
        if !anchor.horizontal.is_finite()
            || !(0.0..=1.0).contains(&anchor.horizontal)
            || !anchor.vertical.is_finite()
            || !(0.0..=1.0).contains(&anchor.vertical)
            || !anchor.offset_x_mm.is_finite()
            || !anchor.offset_y_mm.is_finite()
        {
            return Err(layout_error(
                "attachment alignment must be in 0..=1 and offsets must be finite",
            ));
        }
        let object = elements[index].as_object().unwrap();
        if object.sequence.is_some()
            || object.frame.position != PositionMode::Absolute
            || object
                .layout
                .as_ref()
                .is_none_or(|policy| policy.placement != Placement::Free)
        {
            return Err(layout_error(
                "attached objects require free, absolute positioning outside sequences",
            ));
        }
        let target = ids
            .get(&anchor.target)
            .copied()
            .ok_or_else(|| layout_error("attachment target does not exist"))?;
        if target == index {
            return Err(layout_error("an object cannot attach to itself"));
        }
        if scenes[target]
            .expect("metadata completeness checked above")
            .anchor
            .is_some()
        {
            return Err(layout_error(
                "attachments have one level; an attached object cannot host another object",
            ));
        }
    }
    Ok(())
}

/// Check persistent identity, drawing metadata and one-level relationships.
/// Documents without scene metadata keep the legacy path without conversion.
pub fn validate(elements: &[LabelElement], version: u32) -> Result<()> {
    if !has_scene(elements) {
        return Ok(());
    }
    let scenes: Vec<_> = elements
        .iter()
        .map(|element| element.as_object().and_then(|o| o.scene.as_deref()))
        .collect();
    validate_metadata(elements, version, &scenes)
}

/// Establish scene identity without changing layout positions or sequence links.
///
/// A complete existing scene is left untouched. Newly added objects receive
/// fresh IDs and are placed above the existing scene. Dense layers preserve
/// the previous paint order, including equal-layer persistent-ID ties.
/// Metadata is committed only after the entire candidate has been validated.
pub fn enable(elements: &mut [LabelElement]) -> Result<()> {
    if elements.is_empty() {
        return Ok(());
    }
    if elements.iter().any(|element| element.as_object().is_none()) {
        return Err(layout_error(
            "convert legacy elements before editing shapes, layers or attachments",
        ));
    }
    if elements
        .iter()
        .all(|element| element.as_object().unwrap().scene.is_some())
    {
        return validate(elements, DOCUMENT_VERSION);
    }
    if elements.len() > i32::MAX as usize {
        return Err(layout_error("scene layer limit reached"));
    }
    let mut scenes: Vec<_> = elements
        .iter()
        .map(|element| element.as_object().unwrap().scene.as_deref().cloned())
        .collect();
    let mut next_id = scenes
        .iter()
        .flatten()
        .map(|scene| scene.id)
        .max()
        .unwrap_or(0);
    let mut order: Vec<_> = scenes
        .iter()
        .enumerate()
        .filter_map(|(index, scene)| scene.as_ref().map(|_| index))
        .collect();
    order.sort_by_key(|&index| {
        let scene = scenes[index].as_ref().unwrap();
        (scene.layer, scene.id)
    });
    for (index, scene) in scenes.iter_mut().enumerate() {
        if scene.is_none() {
            next_id = next_id
                .checked_add(1)
                .filter(|id| *id <= i64::MAX as u64)
                .ok_or_else(|| layout_error("scene identifier limit reached"))?;
            *scene = Some(SceneOptions {
                id: next_id,
                layer: 0,
                anchor: None,
            });
            order.push(index);
        }
    }
    for (layer, &index) in order.iter().enumerate() {
        scenes[index].as_mut().unwrap().layer = layer as i32;
    }
    let references: Vec<_> = scenes.iter().map(Option::as_ref).collect();
    validate_metadata(elements, DOCUMENT_VERSION, &references)?;
    for (element, scene) in elements.iter_mut().zip(scenes) {
        element.as_object_mut().unwrap().scene = scene.map(Box::new);
    }
    Ok(())
}

/// Element indices from back to front. Persistent IDs break equal-layer ties.
pub fn paint_order(elements: &[LabelElement]) -> Vec<usize> {
    let mut indices: Vec<_> = (0..elements.len()).collect();
    if has_scene(elements) {
        indices.sort_by_key(|&index| {
            elements[index]
                .as_object()
                .and_then(|object| object.scene.as_ref())
                .map_or((0, index as u64), |scene| (scene.layer, scene.id))
        });
    }
    indices
}

/// Compute absolute frames after content sizes have been resolved at design DPI.
/// Attached children do not take part in either implicit or explicit flow.
pub fn world_frames(elements: &[LabelElement]) -> Result<Vec<TextBox>> {
    validate(elements, DOCUMENT_VERSION)?;
    crate::sequence::validate(elements, DOCUMENT_VERSION)?;
    let mut frames: Vec<_> = elements
        .iter()
        .map(|element| {
            element
                .as_object()
                .map(|object| object.frame.clone())
                .ok_or_else(|| layout_error("scene geometry requires physical objects"))
        })
        .collect::<Result<_>>()?;
    let mut hosts = BTreeMap::new();
    let mut cursor = crate::sequence::LayoutCursor::default();
    let mut successors = None;
    for (index, element) in elements.iter().enumerate() {
        let object = element.as_object().unwrap();
        if object.scene.as_ref().is_some_and(|s| s.anchor.is_some()) {
            continue;
        }
        frames[index].x_mm = cursor.place(&object.frame, object.sequence)?;
        frames[index].position = PositionMode::Absolute;
        if let Some(scene) = &object.scene {
            hosts.insert(scene.id, index);
        }
    }
    for (index, element) in elements.iter().enumerate() {
        let Some(anchor) = element
            .as_object()
            .and_then(|object| object.scene.as_ref())
            .and_then(|scene| scene.anchor.as_ref())
        else {
            continue;
        };
        let host_index = *hosts
            .get(&anchor.target)
            .ok_or_else(|| layout_error("attachment target has no world geometry"))?;
        let host = &frames[host_index];
        let x = if let Some(gap_center) = &anchor.gap_center {
            let next = successors.get_or_insert_with(|| layout_successors(elements))[host_index];
            let gap = next
                .map(|next| resolved_gap(&frames, host_index, next))
                .transpose()?
                .unwrap_or(gap_center.tail_gap_mm);
            host.x_mm + host.width_mm + gap / 2.0 - frames[index].width_mm / 2.0
        } else {
            host.x_mm
                + anchor.horizontal * (host.width_mm - frames[index].width_mm)
                + anchor.offset_x_mm
        };
        let y = host.y_mm
            + anchor.vertical * (host.height_mm - frames[index].height_mm)
            + anchor.offset_y_mm;
        frames[index].x_mm = x;
        frames[index].y_mm = y;
        frames[index].position = PositionMode::Absolute;
    }
    if frames.iter().any(|frame| {
        [frame.x_mm, frame.y_mm, frame.width_mm, frame.height_mm]
            .iter()
            .any(|value| !value.is_finite())
    }) {
        return Err(layout_error("resolved scene geometry must be finite"));
    }
    Ok(frames)
}

/// Next host in the same sequence. Attached children never occupy a gap.
pub fn next_layout_host(elements: &[LabelElement], index: usize) -> Option<usize> {
    layout_successors(elements).get(index).copied().flatten()
}

/// Current physical gap after a host. None means the host is the last member.
pub fn gap_after(elements: &[LabelElement], index: usize) -> Result<Option<f64>> {
    let object = elements
        .get(index)
        .and_then(LabelElement::as_object)
        .ok_or_else(|| layout_error("Select a layout object."))?;
    if object.scene.as_ref().is_some_and(|s| s.anchor.is_some()) {
        return Err(layout_error("Select a host object."));
    }
    let Some(next) = next_layout_host(elements, index) else {
        return Ok(None);
    };
    let frames = world_frames(elements)?;
    resolved_gap(&frames, index, next).map(Some)
}

fn layout_successors(elements: &[LabelElement]) -> Vec<Option<usize>> {
    let mut successors = vec![None; elements.len()];
    let mut previous_explicit = BTreeMap::new();
    let mut previous_implicit = None;
    for (index, element) in elements.iter().enumerate() {
        let Some(object) = element.as_object() else {
            previous_implicit = None;
            continue;
        };
        if object.scene.as_ref().is_some_and(|s| s.anchor.is_some()) {
            continue;
        }
        if let Some(sequence) = object.sequence {
            if let Some(previous) = previous_explicit.insert(sequence, index) {
                successors[previous] = Some(index);
            }
            previous_implicit = None;
        } else {
            if object.frame.position == PositionMode::Relative
                && let Some(previous) = previous_implicit
            {
                successors[previous] = Some(index);
            }
            previous_implicit = Some(index);
        }
    }
    successors
}

fn resolved_gap(frames: &[TextBox], host: usize, next: usize) -> Result<f64> {
    let gap = frames[next].x_mm - frames[host].x_mm - frames[host].width_mm;
    if !gap.is_finite() || gap < -1e-7 {
        return Err(layout_error("Gap center needs a nonnegative gap."));
    }
    // Cancel only floating-point residue. Stored spacing is never changed.
    Ok(gap.max(0.0))
}

/// Apply only derived attachment positions. The stored anchor is never clamped
/// or rewritten. The caller handles printable bounds as a candidate transaction.
pub fn resolve(doc: &mut LabelDocument) -> Result<()> {
    if !has_scene(&doc.elements) {
        return Ok(());
    }
    validate(&doc.elements, doc.version)?;
    let frames = world_frames(&doc.elements)?;
    for (element, frame) in doc.elements.iter_mut().zip(frames) {
        let object = element.as_object_mut().unwrap();
        if object.scene.as_ref().is_some_and(|s| s.anchor.is_some()) {
            object.frame.x_mm = frame.x_mm;
            object.frame.y_mm = frame.y_mm;
        }
    }
    Ok(())
}

/// Direct child indices, independent of element and drawing order.
pub fn children(elements: &[LabelElement], host_index: usize) -> Vec<usize> {
    let Some(host_id) = elements
        .get(host_index)
        .and_then(LabelElement::as_object)
        .and_then(|object| object.scene.as_ref())
        .map(|scene| scene.id)
    else {
        return Vec::new();
    };
    elements
        .iter()
        .enumerate()
        .filter_map(|(index, element)| {
            let anchor = element.as_object()?.scene.as_ref()?.anchor.as_ref()?;
            (anchor.target == host_id).then_some(index)
        })
        .collect()
}

/// Keep a final world frame as manual offsets, with the same host and Y alignment.
pub fn offset_for_frame(anchor: &Anchor, child_world: &TextBox, host_world: &TextBox) -> Anchor {
    Anchor {
        gap_center: None,
        offset_x_mm: child_world.x_mm
            - host_world.x_mm
            - anchor.horizontal * (host_world.width_mm - child_world.width_mm),
        offset_y_mm: child_world.y_mm
            - host_world.y_mm
            - anchor.vertical * (host_world.height_mm - child_world.height_mm),
        ..anchor.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        document::{LabelObject, ObjectContent},
        object_layout::{LayoutOptions, WidthSizing},
    };

    fn object(name: &str, x: f64, width: f64) -> LabelElement {
        LabelElement::Object(LabelObject::text(
            name,
            TextBox {
                x_mm: x,
                width_mm: width,
                height_mm: 6.0,
                ..Default::default()
            },
        ))
    }

    fn anchor(target: u64) -> Anchor {
        Anchor {
            gap_center: None,
            target,
            horizontal: 1.0,
            vertical: 0.0,
            offset_x_mm: -0.5,
            offset_y_mm: 0.5,
        }
    }

    fn attach(element: &mut LabelElement, anchor: Anchor) {
        let object = element.as_object_mut().unwrap();
        object.layout = Some(LayoutOptions {
            placement: Placement::Free,
            width: WidthSizing::Fixed,
            constrain: true,
            padding_mm: 0.0,
        });
        object.frame.position = PositionMode::Absolute;
        object.sequence = None;
        object.scene.as_mut().unwrap().anchor = Some(anchor);
    }

    fn name(element: &LabelElement) -> &str {
        match &element.as_object().unwrap().content {
            ObjectContent::Text { content, .. } => content,
            _ => unreachable!(),
        }
    }

    fn paint_names(elements: &[LabelElement]) -> Vec<&str> {
        paint_order(elements)
            .into_iter()
            .map(|index| name(&elements[index]))
            .collect()
    }

    fn document(elements: Vec<LabelElement>, version: u32) -> LabelDocument {
        LabelDocument {
            version,
            tape_width_mm: 12,
            dpi: 180,
            font_name: "DejaVuSans".into(),
            font_margin: 0,
            flip_h: false,
            flip_v: false,
            elements,
        }
    }

    #[test]
    fn enabling_preserves_frames_sequences_and_complete_scene_identity() {
        let mut elements = vec![object("A", 5.0, 10.0), object("B", 0.0, 10.0)];
        elements[1].as_object_mut().unwrap().frame.position = PositionMode::Relative;
        elements[1].as_object_mut().unwrap().frame.gap_mm = 3.0;
        let before = elements.clone();
        enable(&mut elements).unwrap();
        for (old, new) in before.iter().zip(&elements) {
            assert_eq!(
                old.as_object().unwrap().frame,
                new.as_object().unwrap().frame
            );
            assert_eq!(
                old.as_object().unwrap().sequence,
                new.as_object().unwrap().sequence
            );
        }
        let enabled = elements.clone();
        enable(&mut elements).unwrap();
        assert_eq!(elements, enabled);
        assert!(validate(&elements, 11).is_err());
        validate(&elements, 12).unwrap();
    }

    #[test]
    fn persistent_ties_and_layer_changes_never_depend_on_layout_order() {
        let mut elements = vec![object("A", 5.0, 10.0), object("B", 18.0, 10.0)];
        enable(&mut elements).unwrap();
        elements[1]
            .as_object_mut()
            .unwrap()
            .scene
            .as_mut()
            .unwrap()
            .layer = 0;
        assert_eq!(paint_names(&elements), ["A", "B"]);
        elements.swap(0, 1);
        assert_eq!(paint_names(&elements), ["A", "B"]);
        let before = world_frames(&elements).unwrap();
        elements[1]
            .as_object_mut()
            .unwrap()
            .scene
            .as_mut()
            .unwrap()
            .layer = 2;
        assert_eq!(paint_names(&elements), ["B", "A"]);
        assert_eq!(world_frames(&elements).unwrap(), before);
    }

    #[test]
    fn new_objects_get_fresh_identity_above_existing_paint_order() {
        let mut elements = vec![object("A", 0.0, 10.0), object("B", 13.0, 10.0)];
        enable(&mut elements).unwrap();
        elements[0]
            .as_object_mut()
            .unwrap()
            .scene
            .as_mut()
            .unwrap()
            .layer = i32::MAX;
        elements[1]
            .as_object_mut()
            .unwrap()
            .scene
            .as_mut()
            .unwrap()
            .layer = -20;
        elements.insert(0, object("new", 26.0, 10.0));
        enable(&mut elements).unwrap();
        assert_eq!(paint_names(&elements), ["B", "A", "new"]);
        assert_eq!(
            elements[0].as_object().unwrap().scene.as_ref().unwrap().id,
            3
        );
        assert_eq!(
            elements[1].as_object().unwrap().scene.as_ref().unwrap().id,
            1
        );
        assert_eq!(
            elements[2].as_object().unwrap().scene.as_ref().unwrap().id,
            2
        );
    }

    #[test]
    fn attachment_is_resolved_after_a_later_host_without_changing_flow_cursor() {
        let mut elements = vec![
            object("child", 999.0, 2.0),
            object("host", 5.0, 10.0),
            object("next", 0.0, 10.0),
        ];
        elements[2].as_object_mut().unwrap().frame.position = PositionMode::Relative;
        elements[2].as_object_mut().unwrap().frame.gap_mm = 3.0;
        enable(&mut elements).unwrap();
        let host_id = elements[1].as_object().unwrap().scene.as_ref().unwrap().id;
        attach(&mut elements[0], anchor(host_id));
        let world = world_frames(&elements).unwrap();
        assert_eq!(world[0].x_mm, 12.5);
        assert_eq!(world[0].y_mm, 0.5);
        assert_eq!(world[1].x_mm, 5.0);
        assert_eq!(world[2].x_mm, 18.0);
        assert_eq!(children(&elements, 1), [0]);
        elements.swap(0, 2);
        // Put the legacy relative item back after its host; only the child can
        // freely cross it without changing the ordinary cursor.
        elements.swap(0, 1);
        let world = world_frames(&elements).unwrap();
        assert_eq!(world[0].x_mm, 5.0);
        assert_eq!(world[1].x_mm, 18.0);
        assert_eq!(world[2].x_mm, 12.5);
    }

    #[test]
    fn interleaved_attached_child_cannot_push_an_implicit_or_tagged_sequence() {
        for tagged in [false, true] {
            let mut elements = vec![
                object("A", 5.0, 10.0),
                object("child", 500.0, 2.0),
                object("B", 0.0, 10.0),
            ];
            elements[2].as_object_mut().unwrap().frame.position = PositionMode::Relative;
            elements[2].as_object_mut().unwrap().frame.gap_mm = 3.0;
            if tagged {
                elements[0].as_object_mut().unwrap().sequence = Some(7);
                elements[2].as_object_mut().unwrap().sequence = Some(7);
            }
            enable(&mut elements).unwrap();
            let host_id = elements[0].as_object().unwrap().scene.as_ref().unwrap().id;
            attach(&mut elements[1], anchor(host_id));
            let world = world_frames(&elements).unwrap();
            assert_eq!(world[2].x_mm, 18.0, "tagged={tagged}");
        }
    }

    #[test]
    fn invalid_ids_incomplete_scenes_and_failed_enable_are_transactional() {
        let mut elements = vec![object("A", 0.0, 10.0), object("B", 13.0, 10.0)];
        enable(&mut elements).unwrap();
        for invalid in [0, i64::MAX as u64 + 1, 1] {
            let mut candidate = elements.clone();
            candidate[1]
                .as_object_mut()
                .unwrap()
                .scene
                .as_mut()
                .unwrap()
                .id = invalid;
            assert!(validate(&candidate, 12).is_err());
        }
        elements[0]
            .as_object_mut()
            .unwrap()
            .scene
            .as_mut()
            .unwrap()
            .id = i64::MAX as u64;
        elements[1].as_object_mut().unwrap().scene = None;
        assert!(validate(&elements, 12).is_err());
        let before = elements.clone();
        assert!(enable(&mut elements).is_err());
        assert_eq!(elements, before);
        let mut mixed = vec![object("A", 0.0, 10.0), LabelElement::CutMark];
        let before = mixed.clone();
        assert!(enable(&mut mixed).is_err());
        assert_eq!(mixed, before);
    }

    #[test]
    fn self_missing_nested_and_nonfree_attachments_are_rejected() {
        let mut elements = vec![
            object("host", 0.0, 10.0),
            object("child", 0.0, 2.0),
            object("third", 0.0, 2.0),
        ];
        enable(&mut elements).unwrap();
        attach(&mut elements[1], anchor(1));
        validate(&elements, 12).unwrap();
        for target in [2, 99] {
            let mut invalid = elements.clone();
            invalid[1]
                .as_object_mut()
                .unwrap()
                .scene
                .as_mut()
                .unwrap()
                .anchor
                .as_mut()
                .unwrap()
                .target = target;
            assert!(validate(&invalid, 12).is_err());
        }
        let mut nested = elements.clone();
        attach(&mut nested[0], anchor(3));
        assert!(validate(&nested, 12).is_err());
        let mut nonfree = elements.clone();
        nonfree[1]
            .as_object_mut()
            .unwrap()
            .layout
            .as_mut()
            .unwrap()
            .placement = Placement::Flow;
        assert!(validate(&nonfree, 12).is_err());
        let mut sequenced = elements.clone();
        sequenced[1].as_object_mut().unwrap().sequence = Some(1);
        assert!(validate(&sequenced, 12).is_err());
        let mut relative = elements;
        relative[1].as_object_mut().unwrap().frame.position = PositionMode::Relative;
        assert!(validate(&relative, 12).is_err());
    }

    #[test]
    fn invalid_anchor_coordinates_are_rejected_but_negative_offsets_are_valid() {
        let mut elements = vec![object("host", 0.0, 10.0), object("child", 0.0, 2.0)];
        enable(&mut elements).unwrap();
        attach(&mut elements[1], anchor(1));
        for invalid in [-0.1, 1.1, f64::NAN, f64::INFINITY] {
            for horizontal in [false, true] {
                let mut candidate = elements.clone();
                let a = candidate[1]
                    .as_object_mut()
                    .unwrap()
                    .scene
                    .as_mut()
                    .unwrap()
                    .anchor
                    .as_mut()
                    .unwrap();
                if horizontal {
                    a.horizontal = invalid;
                } else {
                    a.vertical = invalid;
                }
                assert!(validate(&candidate, 12).is_err());
            }
        }
        for invalid in [f64::NAN, f64::NEG_INFINITY] {
            let mut candidate = elements.clone();
            candidate[1]
                .as_object_mut()
                .unwrap()
                .scene
                .as_mut()
                .unwrap()
                .anchor
                .as_mut()
                .unwrap()
                .offset_x_mm = invalid;
            assert!(validate(&candidate, 12).is_err());
        }
        validate(&elements, 12).unwrap();
    }

    #[test]
    fn offsets_roundtrip_a_changed_world_frame_without_clamping() {
        let mut elements = vec![object("host", 5.25, 10.5), object("child", 0.0, 2.0)];
        enable(&mut elements).unwrap();
        let mut a = anchor(1);
        a.horizontal = 0.5;
        a.vertical = 1.0;
        attach(&mut elements[1], a.clone());
        let world = world_frames(&elements).unwrap();
        let mut moved = world[1].clone();
        moved.x_mm = -2.75;
        moved.y_mm = 1.25;
        moved.width_mm = 4.5;
        moved.height_mm = 2.0;
        let changed = offset_for_frame(&a, &moved, &world[0]);
        let child = elements[1].as_object_mut().unwrap();
        child.frame.width_mm = moved.width_mm;
        child.frame.height_mm = moved.height_mm;
        child.scene.as_mut().unwrap().anchor = Some(changed.clone());
        assert_eq!(world_frames(&elements).unwrap()[1], moved);
        let mut doc = document(elements, 12);
        resolve(&mut doc).unwrap();
        assert_eq!(doc.elements[1].as_object().unwrap().frame, moved);
        assert_eq!(
            doc.elements[1]
                .as_object()
                .unwrap()
                .scene
                .as_ref()
                .unwrap()
                .anchor,
            Some(changed)
        );
    }

    #[test]
    fn legacy_resolve_is_noop_and_scene_resolve_never_rewrites_host_placement() {
        let mut legacy = document(vec![LabelElement::CutMark], 1);
        let before = legacy.clone();
        resolve(&mut legacy).unwrap();
        assert_eq!(legacy, before);
        assert_eq!(paint_order(&legacy.elements), [0]);

        let mut elements = vec![
            object("first", 5.0, 10.0),
            object("host", 0.0, 10.0),
            object("child", 0.0, 2.0),
        ];
        elements[1].as_object_mut().unwrap().frame.position = PositionMode::Relative;
        elements[1].as_object_mut().unwrap().frame.gap_mm = 3.0;
        enable(&mut elements).unwrap();
        attach(&mut elements[2], anchor(2));
        let host_before = elements[1].clone();
        let mut doc = document(elements, 12);
        resolve(&mut doc).unwrap();
        assert_eq!(doc.elements[1], host_before);
        assert_eq!(doc.elements[2].as_object().unwrap().frame.x_mm, 25.5);
    }

    #[test]
    fn scene_metadata_serde_defaults_and_rejects_unknown_fields() {
        let scene: SceneOptions = toml::from_str("id = 1\n").unwrap();
        assert_eq!(scene.layer, 0);
        assert_eq!(scene.anchor, None);
        assert!(toml::from_str::<SceneOptions>("id = 1\nunknown = 2\n").is_err());
        assert!(toml::from_str::<Anchor>(
            "target = 1\nhorizontal = 0.0\nvertical = 1.0\noffset_x_mm = 0.0\noffset_y_mm = 0.0\nunknown = 2\n"
        ).is_err());
    }
}
