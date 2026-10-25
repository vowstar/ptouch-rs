// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Repeat or arrange explicit host objects as one fixed-width sequence.

use std::collections::{BTreeMap, BTreeSet};

use ptouch_render::{
    document::{ContentBox, LabelElement, LabelObject, ObjectContent, PositionMode, TextBox},
    object_layout::{self, LayoutOptions, Placement, WidthSizing},
    scene::{self, Anchor, GapCenter, SceneOptions},
    sequence,
    stroke::{PatternKind, StrokePattern},
    styled_text::StyledText,
};

use crate::{scene_edit::SceneSnapshot, state::AppState};

const MAX_OBJECTS: usize = 4096;
const MAX_COPY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct RepeatOptions {
    pub total_items: usize,
    pub start_x_mm: f64,
    pub item_width_mm: f64,
    pub gap_mm: f64,
    pub number_text: bool,
    pub cut_lines: bool,
    pub end_line: bool,
    pub existing_line: Option<u64>,
}

impl Default for RepeatOptions {
    fn default() -> Self {
        Self {
            total_items: 24,
            start_x_mm: 0.0,
            item_width_mm: 14.5,
            gap_mm: 3.35,
            number_text: false,
            cut_lines: false,
            end_line: false,
            existing_line: None,
        }
    }
}

fn id(element: &LabelElement) -> Option<u64> {
    Some(element.as_object()?.scene.as_ref()?.id)
}

fn attachment(element: &LabelElement) -> Option<&Anchor> {
    element.as_object()?.scene.as_ref()?.anchor.as_ref()
}

fn footprint(element: &LabelElement) -> Result<usize, String> {
    let Some(object) = element.as_object() else {
        return Ok(512);
    };
    let payload = match &object.content {
        ObjectContent::Image {
            image_data,
            bitmap,
            coverage,
            ..
        } => image_data
            .len()
            .checked_add(bitmap.as_ref().map_or(0, |b| b.data().len()))
            .and_then(|n| n.checked_add(coverage.as_ref().map_or(0, |b| b.data().len()))),
        ObjectContent::Text {
            content, styles, ..
        } => styles.iter().try_fold(content.len(), |bytes, span| {
            bytes
                .checked_add(std::mem::size_of_val(span))
                .and_then(|n| n.checked_add(span.style.font_name.as_ref().map_or(0, String::len)))
        }),
        _ => Some(0),
    };
    payload
        .and_then(|n| n.checked_add(512))
        .ok_or_else(|| "These items are too large to copy.".into())
}

struct Fresh {
    id: u64,
    layer: i32,
}

impl Fresh {
    fn new(elements: &[LabelElement]) -> Self {
        Self {
            id: elements.iter().filter_map(id).max().unwrap_or(0),
            layer: elements
                .iter()
                .filter_map(|e| Some(e.as_object()?.scene.as_ref()?.layer))
                .max()
                .unwrap_or(0),
        }
    }

    fn id(&mut self) -> Result<u64, String> {
        self.id = self
            .id
            .checked_add(1)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or("Object identifier limit reached.")?;
        Ok(self.id)
    }

    fn layer(&mut self) -> Result<i32, String> {
        self.layer = self.layer.checked_add(1).ok_or("Layer limit reached.")?;
        Ok(self.layer)
    }
}

impl AppState {
    pub fn repeat_items(&mut self, options: &RepeatOptions) -> Result<bool, String> {
        let roots = self.repeat_hosts()?;
        if roots.len() != 1 {
            return Err("Select one host object to repeat.".into());
        }
        self.repeat_transaction(options, &roots, true)
    }

    pub fn arrange_selected(&mut self, options: &RepeatOptions) -> Result<bool, String> {
        let roots = self.repeat_hosts()?;
        self.repeat_transaction(options, &roots, false)
    }

    fn repeat_hosts(&self) -> Result<Vec<usize>, String> {
        if !self.is_free_layout() {
            return Err("Convert the layout before you arrange these objects.".into());
        }
        if self.overall_flip_h {
            return Err(
                "Before you arrange these items, turn off horizontal label reflection.".into(),
            );
        }
        let selected: BTreeSet<_> = self.selected_indices().into_iter().collect();
        let roots: Vec<_> = self
            .layout_order()
            .into_iter()
            .filter(|index| {
                selected.contains(index) && attachment(&self.elements[*index]).is_none()
            })
            .collect();
        if roots.is_empty() {
            return Err("Select at least one host object.".into());
        }
        Ok(roots)
    }

    fn repeat_transaction(
        &mut self,
        options: &RepeatOptions,
        roots: &[usize],
        repeat: bool,
    ) -> Result<bool, String> {
        let count = if repeat {
            options.total_items
        } else {
            roots.len()
        };
        self.check_repeat_options(options, roots, count, repeat)?;
        let snapshot = SceneSnapshot::take(self);
        let result = self
            .repeat_candidate(options, roots, count, repeat)
            .and_then(|hosts| {
                self.check_batch()?;
                hosts
                    .into_iter()
                    .map(|id| self.index_for_scene(id))
                    .collect::<Result<Vec<_>, _>>()
            });
        let indices = match result {
            Ok(indices) => indices,
            Err(message) => {
                snapshot.restore(self);
                return Err(message);
            }
        };
        self.editor_generation = self.editor_generation.wrapping_add(1);
        self.set_selected_indices(&indices, indices.first().copied());
        self.canvas_hover_target = None;
        self.canvas_drag_target = None;
        self.mark_dirty();
        self.status_message = format!("Arranged {count} items.");
        Ok(true)
    }

    fn check_repeat_options(
        &self,
        options: &RepeatOptions,
        roots: &[usize],
        count: usize,
        repeat: bool,
    ) -> Result<(), String> {
        if count == 0 || count > MAX_OBJECTS {
            return Err("Enter an item count from 1 to 4096.".into());
        }
        if !options.start_x_mm.is_finite()
            || !(0.0..=10_000.0).contains(&options.start_x_mm)
            || !options.item_width_mm.is_finite()
            || !(f64::MIN_POSITIVE..=10_000.0).contains(&options.item_width_mm)
            || !options.gap_mm.is_finite()
            || !(0.0..=10_000.0).contains(&options.gap_mm)
        {
            return Err("Enter a valid start position, item width and gap.".into());
        }
        let end = options.start_x_mm
            + count as f64 * options.item_width_mm
            + (count - 1) as f64 * options.gap_mm;
        let columns = (end * f64::from(self.target_dpi()) / 25.4).round();
        if !columns.is_finite()
            || columns > 1_000_000.0
            || columns * f64::from(self.tape_width_px) > 67_108_864.0
        {
            return Err("These items exceed the label size limit.".into());
        }
        for index in roots {
            let object = self.elements[*index]
                .as_object()
                .ok_or("Select host objects.")?;
            if object.content_box.is_none() {
                return Err("Before you arrange these items, clear Follow allocation size.".into());
            }
            if options.number_text && !matches!(object.content, ObjectContent::Text { .. }) {
                return Err("Select text objects to add numbers.".into());
            }
        }
        if options.cut_lines
            && let Some(line) = options.existing_line
        {
            let root_id = id(&self.elements[roots[0]]);
            let matching = self
                .elements
                .iter()
                .find(|element| id(element) == Some(line));
            if !matching.is_some_and(|element| {
                matches!(
                    element.as_object().map(|o| &o.content),
                    Some(ObjectContent::Line { .. })
                ) && attachment(element).is_some_and(|anchor| Some(anchor.target) == root_id)
            }) {
                return Err("Select a line attached to the first item.".into());
            }
        }
        let closure: BTreeSet<_> = roots
            .iter()
            .copied()
            .chain(
                roots
                    .iter()
                    .flat_map(|index| scene::children(&self.elements, *index)),
            )
            .collect();
        let bytes = closure.iter().try_fold(0usize, |sum, index| {
            sum.checked_add(footprint(&self.elements[*index])?)
                .ok_or_else(|| "These items are too large to copy.".to_owned())
        })?;
        let copies = if repeat { count - 1 } else { 0 };
        let wanted_markers = count - usize::from(!options.end_line);
        let new_markers = if !options.cut_lines {
            0
        } else if options.existing_line.is_none() {
            wanted_markers
        } else if repeat {
            0
        } else {
            wanted_markers.saturating_sub(1)
        };
        let removed_marker = usize::from(
            options.cut_lines
                && options.existing_line.is_some()
                && !options.end_line
                && (repeat || count == 1),
        );
        let growth = closure
            .len()
            .checked_mul(copies)
            .and_then(|n| n.checked_add(new_markers))
            .ok_or("These items exceed the object count limit.")?;
        if self
            .elements
            .len()
            .checked_add(growth)
            .and_then(|n| n.checked_sub(removed_marker))
            .is_none_or(|n| n > MAX_OBJECTS)
            || bytes.checked_mul(copies).is_none_or(|n| n > MAX_COPY_BYTES)
        {
            return Err("These items are too large to copy.".into());
        }
        Ok(())
    }

    fn repeat_candidate(
        &mut self,
        options: &RepeatOptions,
        roots: &[usize],
        count: usize,
        repeat: bool,
    ) -> Result<Vec<u64>, String> {
        let original_frames = scene::world_frames(&self.elements).map_err(|e| e.to_string())?;
        scene::enable(&mut self.elements).map_err(|e| e.to_string())?;
        let world: BTreeMap<_, _> = self
            .elements
            .iter()
            .enumerate()
            .filter_map(|(index, e)| Some((id(e)?, original_frames[index].clone())))
            .collect();
        let mut fresh = Fresh::new(&self.elements);
        sequence::materialize(&mut self.elements).map_err(|e| e.to_string())?;
        let sequence = self
            .elements
            .iter()
            .filter_map(|e| e.as_object()?.sequence)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .filter(|n| *n <= i64::MAX as u64)
            .ok_or("Sequence identifier limit reached.")?;
        for index in roots {
            sequence::detach(&mut self.elements, *index).map_err(|e| e.to_string())?;
        }
        let mut blocks = Vec::new();
        let mut remove = BTreeSet::new();
        for root in roots {
            let children = scene::children(&self.elements, *root);
            let indices: Vec<_> = std::iter::once(*root).chain(children).collect();
            remove.extend(indices.iter().copied());
            blocks.push(
                indices
                    .into_iter()
                    .map(|index| self.elements[index].clone())
                    .collect::<Vec<_>>(),
            );
        }
        let insertion = (0..roots[0])
            .filter(|index| !remove.contains(index))
            .count();
        let line_template = if options.cut_lines {
            options
                .existing_line
                .and_then(|wanted| blocks[0].iter().find(|e| id(e) == Some(wanted)))
                .cloned()
        } else {
            None
        };
        let mut groups = Vec::new();
        for item in 0..count {
            let mut block = if repeat && item != 0 {
                clone_block(&blocks[0], &mut fresh)?
            } else {
                blocks[if repeat { 0 } else { item }].clone()
            };
            let host_id = id(&block[0]).ok_or("The item identity is not available.")?;
            configure_host(block[0].as_object_mut().unwrap(), options, sequence, item);
            if options.cut_lines {
                let needs_line = item + 1 < count || options.end_line;
                let current_line = if repeat || item == 0 {
                    if options.existing_line.is_some() {
                        let source = line_template.as_ref().unwrap();
                        let source_slot =
                            blocks[0].iter().position(|e| id(e) == id(source)).unwrap();
                        Some(source_slot)
                    } else {
                        None
                    }
                } else {
                    None
                };
                if let Some(line) = current_line {
                    if needs_line {
                        configure_line(&mut block[line], host_id, options.gap_mm);
                    } else {
                        block.remove(line);
                    }
                } else if needs_line {
                    let mut line = if let Some(template) = &line_template {
                        let mut line = template.clone();
                        let metadata = line.as_object_mut().unwrap().scene.as_mut().unwrap();
                        metadata.id = fresh.id()?;
                        metadata.layer = fresh.layer()?;
                        line
                    } else {
                        self.new_repeat_line(host_id, &block[0], &mut fresh)?
                    };
                    configure_line(&mut line, host_id, options.gap_mm);
                    block.push(line);
                }
            }
            groups.push((host_id, block));
        }
        let ids: Vec<_> = groups.iter().map(|(id, _)| *id).collect();
        let selected_ids: BTreeSet<_> = ids.iter().copied().collect();
        let mut retained: Vec<_> = std::mem::take(&mut self.elements)
            .into_iter()
            .enumerate()
            .filter_map(|(index, element)| (!remove.contains(&index)).then_some(element))
            .collect();
        let added = groups.into_iter().flat_map(|(_, block)| block);
        retained.splice(insertion..insertion, added);
        self.elements = retained;
        // Removing selected members can change a gap after an unselected host.
        // Keep its existing attachments at their old world positions.
        let after = scene::world_frames(&self.elements).map_err(|e| e.to_string())?;
        let hosts: BTreeMap<_, _> = self
            .elements
            .iter()
            .enumerate()
            .filter_map(|(index, e)| Some((id(e)?, index)))
            .collect();
        for index in 0..self.elements.len() {
            let Some(anchor) = attachment(&self.elements[index]).cloned() else {
                continue;
            };
            if selected_ids.contains(&anchor.target) {
                continue;
            }
            let Some(old) = world.get(&id(&self.elements[index]).unwrap()) else {
                continue;
            };
            if (after[index].x_mm - old.x_mm).abs() > 1e-8
                || (after[index].y_mm - old.y_mm).abs() > 1e-8
            {
                let host = *hosts
                    .get(&anchor.target)
                    .ok_or("The attachment host is not available.")?;
                self.elements[index]
                    .as_object_mut()
                    .unwrap()
                    .scene
                    .as_mut()
                    .unwrap()
                    .anchor = Some(scene::offset_for_frame(&anchor, old, &after[host]));
            }
        }
        self.document_version = self.document_version.max(13);
        Ok(ids)
    }

    fn new_repeat_line(
        &self,
        target: u64,
        host: &LabelElement,
        fresh: &mut Fresh,
    ) -> Result<LabelElement, String> {
        let host = host.as_object().unwrap();
        let length = self.printable_height_mm() * 0.8;
        let mut line = LabelObject::text("", TextBox::default());
        line.rotation = 90.0;
        line.content_box = Some(ContentBox {
            width_mm: length,
            height_mm: 0.3,
            offset_x_mm: 0.0,
            offset_y_mm: 0.0,
        });
        line.layout = Some(LayoutOptions {
            placement: Placement::Free,
            width: WidthSizing::Fixed,
            constrain: true,
            padding_mm: 0.0,
        });
        line.content = ObjectContent::Line {
            pattern: StrokePattern {
                kind: PatternKind::Dotted,
                ..Default::default()
            },
        };
        line.scene = Some(Box::new(SceneOptions {
            id: fresh.id()?,
            layer: fresh.layer()?,
            anchor: Some(Anchor {
                target,
                horizontal: 1.0,
                vertical: 0.5,
                offset_x_mm: 0.0,
                offset_y_mm: self.printable_height_mm() / 2.0
                    - host.frame.y_mm
                    - host.frame.height_mm / 2.0,
                gap_center: None,
            }),
        }));
        object_layout::update_allocation(&mut line);
        Ok(LabelElement::Object(line))
    }
}

fn configure_host(object: &mut LabelObject, options: &RepeatOptions, sequence: u64, item: usize) {
    object.sequence = Some(sequence);
    object.frame.position = if item == 0 {
        PositionMode::Absolute
    } else {
        PositionMode::Relative
    };
    object.frame.x_mm = if item == 0 { options.start_x_mm } else { 0.0 };
    object.frame.width_mm = options.item_width_mm;
    object.frame.gap_mm = options.gap_mm;
    let policy = object.layout.get_or_insert(LayoutOptions {
        placement: Placement::Flow,
        width: WidthSizing::Fixed,
        constrain: false,
        padding_mm: 0.0,
    });
    policy.placement = Placement::Flow;
    policy.width = WidthSizing::Fixed;
    if options.number_text
        && let ObjectContent::Text {
            content, styles, ..
        } = &mut object.content
    {
        let mut styled = StyledText {
            text: content.clone(),
            spans: styles.clone(),
        };
        let style = styled.style_at(0);
        styled.replace(0..styled.len(), &(item + 1).to_string(), &style);
        *content = styled.text;
        *styles = styled.spans;
    }
}

fn configure_line(element: &mut LabelElement, target: u64, gap: f64) {
    let object = element.as_object_mut().unwrap();
    let anchor = object.scene.as_mut().unwrap().anchor.as_mut().unwrap();
    anchor.target = target;
    anchor.gap_center = Some(GapCenter { tail_gap_mm: gap });
}

fn clone_block(source: &[LabelElement], fresh: &mut Fresh) -> Result<Vec<LabelElement>, String> {
    let mut result = source.to_vec();
    let mut order: Vec<_> = (0..result.len()).collect();
    order.sort_by_key(|index| {
        let scene = result[*index].as_object().unwrap().scene.as_ref().unwrap();
        (scene.layer, scene.id)
    });
    for element in &mut result {
        element.as_object_mut().unwrap().scene.as_mut().unwrap().id = fresh.id()?;
    }
    let host_id = id(&result[0]).unwrap();
    for element in result.iter_mut().skip(1) {
        element
            .as_object_mut()
            .unwrap()
            .scene
            .as_mut()
            .unwrap()
            .anchor
            .as_mut()
            .unwrap()
            .target = host_id;
    }
    for index in order {
        result[index]
            .as_object_mut()
            .unwrap()
            .scene
            .as_mut()
            .unwrap()
            .layer = fresh.layer()?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ptouch_render::{
        shape::ShapeStyle,
        styled_text::{StyleSpan, TextStyle},
    };

    fn host(id: u64, position: PositionMode, height: f64) -> LabelObject {
        let mut object = LabelObject::text(
            "PORT",
            TextBox {
                x_mm: 5.0,
                width_mm: 14.5,
                height_mm: height,
                position,
                gap_mm: 3.35,
                ..Default::default()
            },
        );
        object.sequence = Some(1);
        object.layout = Some(LayoutOptions {
            placement: Placement::Flow,
            width: WidthSizing::Fixed,
            constrain: false,
            padding_mm: 0.0,
        });
        object.content_box = Some(ContentBox {
            width_mm: 8.0,
            height_mm: 3.0,
            offset_x_mm: 0.0,
            offset_y_mm: 0.0,
        });
        if let ObjectContent::Text { font_size, .. } = &mut object.content {
            *font_size = Some(6.0);
        }
        object.scene = Some(Box::new(SceneOptions {
            id,
            layer: id as i32,
            anchor: None,
        }));
        object
    }

    fn child(id: u64, target: u64, line: bool) -> LabelObject {
        let mut object = LabelObject::text("", TextBox::default());
        object.content_box = Some(ContentBox {
            width_mm: if line { 8.0 } else { 1.0 },
            height_mm: if line { 0.3 } else { 1.0 },
            offset_x_mm: 0.0,
            offset_y_mm: 0.0,
        });
        object.layout = Some(LayoutOptions {
            placement: Placement::Free,
            width: WidthSizing::Fixed,
            constrain: true,
            padding_mm: 0.0,
        });
        object.rotation = if line { 90.0 } else { 0.0 };
        object.content = if line {
            ObjectContent::Line {
                pattern: StrokePattern::default(),
            }
        } else {
            ObjectContent::Shape {
                style: ShapeStyle::default(),
            }
        };
        object.scene = Some(Box::new(SceneOptions {
            id,
            layer: id as i32,
            anchor: Some(Anchor {
                target,
                horizontal: if line { 1.0 } else { 0.0 },
                vertical: 0.5,
                offset_x_mm: if line { 1.825 } else { 1.0 },
                offset_y_mm: 0.0,
                gap_center: None,
            }),
        }));
        object_layout::update_allocation(&mut object);
        object
    }

    fn fixture(dpi: u16) -> AppState {
        let mut state = AppState {
            document_version: 13,
            design_dpi: dpi,
            ..Default::default()
        };
        state.update_tape_pixels();
        let height = state.printable_height_mm();
        state.elements = vec![
            LabelElement::Object(host(1, PositionMode::Absolute, height)),
            LabelElement::Object(child(2, 1, true)),
            LabelElement::Object(child(3, 1, false)),
            LabelElement::Object(host(4, PositionMode::Relative, height)),
            LabelElement::Object(child(5, 4, true)),
            LabelElement::Object(host(6, PositionMode::Relative, height)),
            LabelElement::Object(child(7, 6, true)),
        ];
        let mut free = host(8, PositionMode::Absolute, height);
        free.sequence = None;
        free.frame.x_mm = 100.0;
        free.layout.as_mut().unwrap().placement = Placement::Free;
        object_layout::update_allocation(&mut free);
        state.elements.push(LabelElement::Object(free));
        state.validate_scene_geometry().unwrap();
        state.set_selected_indices(&[0], Some(0));
        state.needs_rerender = false;
        state
    }

    fn options() -> RepeatOptions {
        RepeatOptions {
            total_items: 4,
            ..Default::default()
        }
    }

    fn frames(state: &AppState) -> BTreeMap<u64, TextBox> {
        let frames = scene::world_frames(&state.elements).unwrap();
        state
            .elements
            .iter()
            .zip(frames)
            .map(|(e, f)| (id(e).unwrap(), f))
            .collect()
    }

    #[track_caller]
    fn assert_frame(actual: &TextBox, expected: &TextBox) {
        for (a, b) in [
            (actual.x_mm, expected.x_mm),
            (actual.y_mm, expected.y_mm),
            (actual.width_mm, expected.width_mm),
            (actual.height_mm, expected.height_mm),
        ] {
            assert!((a - b).abs() < 1e-8, "{a} differs from {b}");
        }
    }

    fn gap_lines(state: &AppState) -> Vec<usize> {
        state
            .elements
            .iter()
            .enumerate()
            .filter_map(|(i, e)| {
                attachment(e)
                    .is_some_and(|a| a.gap_center.is_some())
                    .then_some(i)
            })
            .collect()
    }

    #[test]
    fn repeat_keeps_content_all_attachments_and_unselected_world_geometry() {
        for dpi in [180, 360] {
            let mut state = fixture(dpi);
            let source = state.elements[0].as_object_mut().unwrap();
            source.rotation = 37.0;
            source.flip_h = true;
            source.flip_v = true;
            let source = source.clone();
            state.set_selected_indices(&[0, 1, 2], Some(0));
            let before = frames(&state);
            assert!(state.repeat_items(&options()).unwrap());
            assert_eq!(state.elements.len(), 17);
            assert_eq!(state.selected_indices().len(), 4);
            let ids: BTreeSet<_> = state.elements.iter().filter_map(id).collect();
            assert_eq!(ids.len(), state.elements.len());
            let after = frames(&state);
            for id in [4, 5, 6, 7, 8] {
                assert_frame(&after[&id], &before[&id]);
            }
            for (ordinal, index) in state.selected_indices().into_iter().enumerate() {
                let object = state.elements[index].as_object().unwrap();
                assert!(
                    (after[&id(&state.elements[index]).unwrap()].x_mm - ordinal as f64 * 17.85)
                        .abs()
                        < 1e-8
                );
                assert_eq!(object.content, source.content);
                assert_eq!(object.content_box, source.content_box);
                assert_eq!(
                    (object.rotation, object.flip_h, object.flip_v),
                    (37.0, true, true)
                );
                assert_eq!(scene::children(&state.elements, index).len(), 2);
            }
            assert!(gap_lines(&state).is_empty());
            assert_eq!(state.document().version, 13);
        }
    }

    #[test]
    fn new_cut_lines_are_explicit_and_end_line_is_separate() {
        for end in [false, true] {
            let mut state = fixture(180);
            let mut opts = options();
            opts.cut_lines = true;
            opts.end_line = end;
            state.repeat_items(&opts).unwrap();
            assert_eq!(gap_lines(&state).len(), if end { 4 } else { 3 });
            assert_eq!(state.document().version, 14);
            for line in gap_lines(&state) {
                let object = state.elements[line].as_object().unwrap();
                assert!(
                    matches!(&object.content,ObjectContent::Line {pattern} if pattern.kind==PatternKind::Dotted)
                );
                assert_eq!(
                    attachment(&state.elements[line])
                        .unwrap()
                        .gap_center
                        .as_ref()
                        .unwrap()
                        .tail_gap_mm,
                    3.35
                );
            }
            // The source's unrelated manual line and badge remain in every copy.
            for root in state.selected_indices() {
                let children = scene::children(&state.elements, root);
                assert_eq!(
                    children
                        .iter()
                        .filter(|i| attachment(&state.elements[**i])
                            .unwrap()
                            .gap_center
                            .is_none())
                        .count(),
                    2
                );
            }
        }
        let mut state = fixture(180);
        let opts = RepeatOptions {
            total_items: 1,
            cut_lines: true,
            ..options()
        };
        state.repeat_items(&opts).unwrap();
        assert!(gap_lines(&state).is_empty());
        assert_eq!(state.document().version, 13);
    }

    #[test]
    fn only_the_explicit_template_becomes_a_cut_line() {
        let mut state = fixture(180);
        let opts = RepeatOptions {
            cut_lines: true,
            existing_line: Some(2),
            ..options()
        };
        state.repeat_items(&opts).unwrap();
        assert_eq!(state.elements.len(), 16);
        assert_eq!(gap_lines(&state).len(), 3);
        assert!(
            attachment(state.elements.iter().find(|e| id(e) == Some(2)).unwrap())
                .unwrap()
                .gap_center
                .is_some()
        );
        for root in state.selected_indices() {
            let children = scene::children(&state.elements, root);
            assert_eq!(
                children
                    .iter()
                    .filter(|i| matches!(
                        &state.elements[**i].as_object().unwrap().content,
                        ObjectContent::Shape { .. }
                    ))
                    .count(),
                1
            );
        }
        let mut state = fixture(180);
        let opts = RepeatOptions {
            total_items: 1,
            cut_lines: true,
            existing_line: Some(2),
            ..options()
        };
        state.repeat_items(&opts).unwrap();
        assert!(!state.elements.iter().any(|e| id(e) == Some(2)));
        assert!(state.elements.iter().any(|e| id(e) == Some(3)));
        assert_eq!(state.document().version, 13);
    }

    #[test]
    fn arrange_uses_layout_order_and_preserves_unselected_items_and_lines() {
        let mut state = fixture(180);
        let before = frames(&state);
        // A selected child is not an additional host. A child without its host
        // is not silently moved into the new sequence.
        state.set_selected_indices(&[5, 4, 0, 1], Some(5));
        let opts = RepeatOptions {
            total_items: usize::MAX,
            cut_lines: true,
            existing_line: Some(2),
            ..options()
        };
        state.arrange_selected(&opts).unwrap();
        let selected = state.selected_indices();
        assert_eq!(
            selected
                .iter()
                .map(|i| id(&state.elements[*i]).unwrap())
                .collect::<Vec<_>>(),
            vec![1, 6]
        );
        let after = frames(&state);
        for id in [4, 5, 8] {
            assert_frame(&after[&id], &before[&id]);
        }
        assert_eq!(after[&1].x_mm, 0.0);
        assert!((after[&6].x_mm - 17.85).abs() < 1e-8);
        assert!(state.elements.iter().any(|e| id(e) == Some(7)));
        assert_eq!(state.elements.len(), 8);
        assert_eq!(gap_lines(&state).len(), 1);
        assert_eq!(
            state.elements[selected[0]].as_object().unwrap().sequence,
            state.elements[selected[1]].as_object().unwrap().sequence
        );
    }

    #[test]
    fn removing_a_host_preserves_an_unselected_gap_line_world_position() {
        let mut state = fixture(180);
        state.elements[1]
            .as_object_mut()
            .unwrap()
            .scene
            .as_mut()
            .unwrap()
            .anchor
            .as_mut()
            .unwrap()
            .gap_center = Some(GapCenter { tail_gap_mm: 3.35 });
        state.document_version = 14;
        state.validate_scene_geometry().unwrap();
        let before = frames(&state);
        state.set_selected_indices(&[3], Some(3));
        let opts = RepeatOptions {
            total_items: 3,
            start_x_mm: 60.0,
            ..options()
        };
        state.repeat_items(&opts).unwrap();
        let after = frames(&state);
        for id in [1, 2, 3, 6, 7, 8] {
            assert_frame(&after[&id], &before[&id]);
        }
        let line = state.elements.iter().find(|e| id(e) == Some(2)).unwrap();
        assert!(attachment(line).unwrap().gap_center.is_none());
    }

    fn assert_failure_unchanged(state: &mut AppState, options: &RepeatOptions) {
        let before = state.document();
        let selection = state.selected_indices();
        let revisions = (
            state.editor_generation,
            state.selection_revision,
            state.edit_revision,
            state.needs_rerender,
        );
        let history = (state.history.can_undo(), state.history.can_redo());
        assert!(state.repeat_items(options).is_err());
        assert_eq!(state.document(), before);
        assert_eq!(state.selected_indices(), selection);
        assert_eq!(
            (
                state.editor_generation,
                state.selection_revision,
                state.edit_revision,
                state.needs_rerender
            ),
            revisions
        );
        assert_eq!(
            (state.history.can_undo(), state.history.can_redo()),
            history
        );
    }

    #[test]
    fn bad_options_and_late_geometry_failures_are_atomic() {
        for options in [
            RepeatOptions {
                total_items: 0,
                ..options()
            },
            RepeatOptions {
                total_items: 4097,
                ..options()
            },
            RepeatOptions {
                start_x_mm: f64::NAN,
                ..options()
            },
            RepeatOptions {
                gap_mm: -1.0,
                ..options()
            },
            RepeatOptions {
                item_width_mm: 0.0,
                ..options()
            },
            RepeatOptions {
                cut_lines: true,
                existing_line: Some(99),
                ..options()
            },
            RepeatOptions {
                cut_lines: true,
                existing_line: Some(5),
                ..options()
            },
        ] {
            assert_failure_unchanged(&mut fixture(180), &options);
        }
        let mut state = fixture(180);
        state.elements[0]
            .as_object_mut()
            .unwrap()
            .layout
            .as_mut()
            .unwrap()
            .constrain = true;
        let narrow = RepeatOptions {
            item_width_mm: 0.1,
            ..options()
        };
        assert_failure_unchanged(&mut state, &narrow);
        let mut state = fixture(180);
        state.elements[0].as_object_mut().unwrap().content_box = None;
        assert_failure_unchanged(&mut state, &options());
        let mut state = fixture(180);
        state.elements[7]
            .as_object_mut()
            .unwrap()
            .scene
            .as_mut()
            .unwrap()
            .id = i64::MAX as u64;
        assert_failure_unchanged(&mut state, &options());
        let mut state = fixture(180);
        state.set_selected_indices(&[1], Some(1));
        assert_failure_unchanged(&mut state, &options());
        state.set_selected_indices(&[0, 3], Some(0));
        assert_failure_unchanged(&mut state, &options());
    }

    #[test]
    fn payload_budget_is_checked_before_image_decoding_or_copying() {
        let mut state = fixture(180);
        state.elements[2].as_object_mut().unwrap().content = ObjectContent::Image {
            path: None,
            image_data: vec![0; 80 * 1024],
            bitmap: None,
            preserve_alpha: true,
            coverage: None,
        };
        let opts = RepeatOptions {
            total_items: 1000,
            ..options()
        };
        let before = state.document();
        let message = state.repeat_items(&opts).unwrap_err();
        assert!(message.contains("too large to copy"), "{message}");
        assert_eq!(state.document(), before);
        assert_eq!(state.elements.len(), 8);
    }

    #[test]
    fn numbering_is_opt_in_and_retains_the_first_character_style() {
        let mut state = fixture(180);
        if let ObjectContent::Text { styles, .. } =
            &mut state.elements[0].as_object_mut().unwrap().content
        {
            styles.push(StyleSpan {
                start: 0,
                end: 4,
                style: TextStyle {
                    bold: true,
                    ..Default::default()
                },
            });
        }
        let opts = RepeatOptions {
            total_items: 12,
            number_text: true,
            ..options()
        };
        state.repeat_items(&opts).unwrap();
        for (ordinal, index) in state.selected_indices().into_iter().enumerate() {
            let ObjectContent::Text {
                content,
                styles,
                font_size,
                ..
            } = &state.elements[index].as_object().unwrap().content
            else {
                panic!()
            };
            assert_eq!(content, &(ordinal + 1).to_string());
            assert_eq!(*font_size, Some(6.0));
            assert_eq!(styles.len(), 1);
            assert!(styles[0].style.bold);
            assert_eq!(
                (styles[0].start, styles[0].end),
                (0, content.chars().count())
            );
        }
    }

    #[test]
    fn one_undo_restores_the_complete_repeat_and_selection() {
        let mut state = fixture(180);
        let before = state.document();
        state
            .history
            .observe(before.clone(), state.selected_element, false, false, 0.0);
        state.history.set_selection_set(&state.selected_indices());
        state.repeat_items(&options()).unwrap();
        let after = state.document();
        let selected = state.selected_indices();
        state
            .history
            .observe(after.clone(), state.selected_element, false, false, 1.0);
        state.history.set_selection_set(&selected);
        state.history_step(false);
        assert_eq!(state.document(), before);
        assert_eq!(state.selected_indices(), vec![0]);
        assert!(!state.history.can_undo());
        state.history_step(true);
        assert_eq!(state.document(), after);
        assert_eq!(state.selected_indices(), selected);
    }

    #[test]
    fn horizontal_label_reflection_requires_explicit_user_action() {
        let mut state = fixture(180);
        state.overall_flip_h = true;
        assert_failure_unchanged(&mut state, &options());
        state.overall_flip_h = false;
        state.overall_flip_v = true;
        state.repeat_items(&options()).unwrap();
        assert!(state.overall_flip_v);
    }

    #[test]
    fn non_text_hosts_and_real_image_attachments_keep_their_content() {
        let source = image::RgbaImage::from_fn(3, 2, |x, y| {
            image::Rgba(if x == y {
                [0, 0, 0, 255]
            } else {
                [255, 255, 255, 0]
            })
        });
        let mut encoded = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(source)
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let image = ObjectContent::Image {
            path: None,
            image_data: encoded.into_inner(),
            bitmap: None,
            preserve_alpha: true,
            coverage: None,
        };
        for content in [
            image.clone(),
            ObjectContent::Line {
                pattern: Default::default(),
            },
            ObjectContent::Shape {
                style: Default::default(),
            },
            ObjectContent::CutMark,
            ObjectContent::Padding,
        ] {
            let mut state = fixture(180);
            state.elements[0].as_object_mut().unwrap().content = content.clone();
            state.elements[2].as_object_mut().unwrap().content = image.clone();
            state.repeat_items(&options()).unwrap();
            for root in state.selected_indices() {
                assert_eq!(state.elements[root].as_object().unwrap().content, content);
                let badge = scene::children(&state.elements, root)
                    .into_iter()
                    .find(|index| {
                        matches!(
                            state.elements[*index].as_object().unwrap().content,
                            ObjectContent::Image { .. }
                        )
                    })
                    .unwrap();
                assert_eq!(state.elements[badge].as_object().unwrap().content, image);
            }
        }
    }

    #[test]
    fn arrange_template_tail_preserves_other_existing_lines() {
        let mut state = fixture(360);
        state.set_selected_indices(&[0, 3, 5], Some(0));
        let opts = RepeatOptions {
            cut_lines: true,
            end_line: true,
            existing_line: Some(2),
            ..options()
        };
        state.arrange_selected(&opts).unwrap();
        assert_eq!(state.elements.len(), 10);
        assert_eq!(gap_lines(&state).len(), 3);
        for existing in [5, 7] {
            let line = state
                .elements
                .iter()
                .find(|element| id(element) == Some(existing))
                .unwrap();
            assert!(attachment(line).unwrap().gap_center.is_none());
        }
        let frames = frames(&state);
        let last = *state.selected_indices().last().unwrap();
        let last_id = id(&state.elements[last]).unwrap();
        let marker = gap_lines(&state)
            .into_iter()
            .find(|index| attachment(&state.elements[*index]).unwrap().target == last_id)
            .unwrap();
        let line = &frames[&id(&state.elements[marker]).unwrap()];
        let host = &frames[&last_id];
        assert!(
            (line.x_mm + line.width_mm / 2.0 - host.x_mm - host.width_mm - opts.gap_mm / 2.0).abs()
                < 1e-8
        );
    }

    #[test]
    fn implicit_chains_materialize_before_the_new_sequence_id_is_allocated() {
        let mut state = fixture(180);
        for element in &mut state.elements {
            element.as_object_mut().unwrap().sequence = None;
        }
        let before = frames(&state);
        state.repeat_items(&options()).unwrap();
        let after = frames(&state);
        for id in [4, 5, 6, 7, 8] {
            assert_frame(&after[&id], &before[&id]);
        }
        let selected_id = state.elements[state.selected_indices()[0]]
            .as_object()
            .unwrap()
            .sequence;
        let unselected = state
            .elements
            .iter()
            .find(|element| id(element) == Some(4))
            .unwrap()
            .as_object()
            .unwrap();
        assert_ne!(selected_id, unselected.sequence);
        assert!(selected_id.is_some() && unselected.sequence.is_some());
    }
}
