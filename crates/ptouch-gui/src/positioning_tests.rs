// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

use ptouch_render::{
    bitmap::LabelBitmap,
    document::{
        ContentBox, LabelDocument, LabelElement, LabelObject, ObjectContent, PositionMode, TextBox,
    },
    object_layout::{self, LayoutOptions, Placement, WidthSizing},
    scene::{self, Anchor, SceneOptions},
    text::TextRenderer,
};

use crate::{
    positioning::Positioning,
    state::{AppState, CanvasEditTarget, CanvasTarget},
};

fn host(id: u64, x: f64, position: PositionMode, height: f64) -> LabelObject {
    let mut object = LabelObject::text(
        format!("PORT {id}"),
        TextBox {
            x_mm: x,
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
        offset_x_mm: 0.4,
        offset_y_mm: 0.2,
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

fn line(id: u64, target: u64) -> LabelObject {
    let mut object = LabelObject::text("", TextBox::default());
    object.rotation = 90.0;
    object.layout = Some(LayoutOptions {
        placement: Placement::Free,
        width: WidthSizing::Fixed,
        constrain: true,
        padding_mm: 0.0,
    });
    object.content = ObjectContent::Line {
        pattern: Default::default(),
    };
    object.content_box = Some(ContentBox {
        width_mm: 8.0,
        height_mm: 0.3,
        offset_x_mm: 0.0,
        offset_y_mm: 0.0,
    });
    object.scene = Some(Box::new(SceneOptions {
        id,
        layer: id as i32,
        anchor: Some(Anchor {
            gap_center: None,
            target,
            horizontal: 1.0,
            vertical: 0.5,
            offset_x_mm: 1.825,
            offset_y_mm: 0.0,
        }),
    }));
    object_layout::update_allocation(&mut object);
    object
}

fn fixture(dpi: u16) -> AppState {
    let mut state = AppState {
        document_version: 13,
        design_dpi: dpi,
        needs_rerender: false,
        ..Default::default()
    };
    state.update_tape_pixels();
    let height = state.printable_height_mm();
    for (id, position) in [
        (1, PositionMode::Absolute),
        (3, PositionMode::Relative),
        (5, PositionMode::Relative),
    ] {
        state
            .elements
            .push(LabelElement::Object(host(id, 5.0, position, height)));
        state.elements.push(LabelElement::Object(line(id + 1, id)));
    }
    state.validate_scene_geometry().unwrap();
    state.selected_element = Some(2);
    state.canvas_edit_target = CanvasEditTarget::Slot;
    state.canvas_hover_target = Some(CanvasTarget {
        element: 1,
        edit: CanvasEditTarget::Content,
    });
    state.canvas_drag_target = Some(CanvasTarget {
        element: 4,
        edit: CanvasEditTarget::Slot,
    });
    state.edit_revision = 41;
    state.selection_revision = 42;
    state.editor_generation = 43;
    state.needs_rerender = false;
    state
}

fn render(state: &AppState, renderer: &mut TextRenderer) -> LabelBitmap {
    ptouch_render::document::render_document(
        &state.document(),
        state.tape_width_px,
        state.target_dpi(),
        renderer,
    )
    .unwrap()
    .bitmap
    .unwrap()
}

fn assert_frame_equal(actual: &TextBox, expected: &TextBox) {
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

fn assert_other_frames_unchanged(state: &AppState, before: &[TextBox], changed: &[usize]) {
    let after = scene::world_frames(&state.elements).unwrap();
    for (index, (actual, expected)) in after.iter().zip(before).enumerate() {
        if !changed.contains(&index) {
            assert_frame_equal(actual, expected);
        }
    }
}

#[track_caller]
fn assert_pixels_equal(actual: &LabelBitmap, expected: &LabelBitmap) {
    assert_eq!(
        (actual.width(), actual.height()),
        (expected.width(), expected.height())
    );
    if actual.data() != expected.data() {
        let mut count = 0;
        let mut first = Vec::new();
        for y in 0..actual.height() {
            for x in 0..actual.width() {
                if actual.get_pixel(x, y) != expected.get_pixel(x, y) {
                    count += 1;
                    if first.len() < 6 {
                        first.push((x, y));
                    }
                }
            }
        }
        panic!("{count} pixels changed. First changed positions: {first:?}");
    }
}

fn content_center_mm(state: &AppState, index: usize) -> (f64, f64) {
    let frames = scene::world_frames(&state.elements).unwrap();
    let frame = &frames[index];
    let object = state.elements[index].as_object().unwrap();
    let (x, y, width, height) = object_layout::content_extent(object).unwrap();
    (
        frame.x_mm + frame.width_mm / 2.0 + x + width / 2.0,
        frame.y_mm + frame.height_mm / 2.0 + y + height / 2.0,
    )
}

fn isolated_object(
    state: &AppState,
    index: usize,
    total_width: u32,
    renderer: &mut TextRenderer,
) -> LabelBitmap {
    let frames = scene::world_frames(&state.elements).unwrap();
    let mut object = state.elements[index].as_object().unwrap().clone();
    object.frame = frames[index].clone();
    object.frame.position = PositionMode::Absolute;
    object.sequence = None;
    object.scene = None;
    // The blank end marker retains the full label's mirror axis while this
    // render contains ink from only the object under test.
    let mm_per_dot = 25.4 / f64::from(state.target_dpi());
    let mut marker = LabelObject::text(
        "",
        TextBox {
            x_mm: f64::from(total_width - 1) * mm_per_dot,
            width_mm: mm_per_dot,
            height_mm: mm_per_dot,
            ..Default::default()
        },
    );
    marker.content = ObjectContent::Padding;
    let mut document = state.document();
    document.elements = vec![LabelElement::Object(object), LabelElement::Object(marker)];
    ptouch_render::document::render_document(
        &document,
        state.tape_width_px,
        state.target_dpi(),
        renderer,
    )
    .unwrap()
    .bitmap
    .unwrap()
}

fn assert_at_most_two_pixel_translation(
    actual: &LabelBitmap,
    expected: &LabelBitmap,
) -> (i64, i64) {
    use std::collections::BTreeSet;
    let black = |bitmap: &LabelBitmap| -> BTreeSet<(i64, i64)> {
        (0..bitmap.height())
            .flat_map(|y| {
                (0..bitmap.width()).filter_map(move |x| {
                    bitmap
                        .get_pixel(x, y)
                        .then_some((i64::from(x), i64::from(y)))
                })
            })
            .collect()
    };
    assert_eq!(
        (actual.width(), actual.height()),
        (expected.width(), expected.height())
    );
    let before = black(expected);
    let after = black(actual);
    assert!(!before.is_empty(), "The fixture must contain visible ink.");
    assert_eq!(before.len(), after.len(), "The ink count changed.");
    // Two rounded frame edges and one rounded raster edge determine a mirror.
    // The old and new expressions contain six rounding errors. Their sum is
    // strictly below three, so the integer shift is at most two dots.
    (-2..=2)
        .find_map(|dy| {
            (-2..=2)
                .find(|dx| {
                    before
                        .iter()
                        .all(|(x, y)| after.contains(&(x + dx, y + dy)))
                })
                .map(|dx| (dx, dy))
        })
        .expect("The object changed beyond two pixels of translation.")
}

#[derive(Debug, PartialEq)]
struct UnchangedState {
    document: LabelDocument,
    selected: Option<usize>,
    target: CanvasEditTarget,
    hovered: Option<CanvasTarget>,
    dragged: Option<CanvasTarget>,
    edit_revision: u64,
    selection_revision: u64,
    editor_generation: u64,
    needs_rerender: bool,
    can_undo: bool,
    can_redo: bool,
}

fn unchanged_state(state: &AppState) -> UnchangedState {
    UnchangedState {
        document: state.document(),
        selected: state.selected_element,
        target: state.canvas_edit_target,
        hovered: state.canvas_hover_target,
        dragged: state.canvas_drag_target,
        edit_revision: state.edit_revision,
        selection_revision: state.selection_revision,
        editor_generation: state.editor_generation,
        needs_rerender: state.needs_rerender,
        can_undo: state.history.can_undo(),
        can_redo: state.history.can_redo(),
    }
}

#[test]
fn fixed_sizes_and_all_attachments_survive_each_sequence_position() {
    let mut renderer = TextRenderer::new();
    for dpi in [180, 360] {
        for index in [0, 2, 4] {
            let mut state = fixture(dpi);
            let frames = scene::world_frames(&state.elements).unwrap();
            let original = state.elements[index].as_object().unwrap().clone();
            let bitmap = render(&state, &mut renderer);
            assert!(state.set_positioning(index, Positioning::Free).unwrap());
            let object = state.elements[index].as_object().unwrap();
            let content = object.content_box.as_ref().unwrap();
            let original_content = original.content_box.as_ref().unwrap();
            assert_eq!(content.width_mm, original_content.width_mm);
            assert_eq!(content.height_mm, original_content.height_mm);
            assert_eq!(object.layout.as_ref().unwrap().width, WidthSizing::Fixed);
            assert_eq!(object.sequence, None);
            assert_other_frames_unchanged(&state, &frames, &[index]);
            assert_pixels_equal(&render(&state, &mut renderer), &bitmap);
        }
    }
}

#[test]
fn rotation_and_mirrors_preserve_mm_geometry_with_only_mirror_axis_rounding() {
    let mut renderer = TextRenderer::new();
    let mut measurement_renderer = Some(TextRenderer::new());
    for dpi in [180, 360] {
        for (index, fractional_frame) in [
            (0, false),
            (2, false),
            (4, false),
            (0, true),
            (2, true),
            (4, true),
        ] {
            for rotation in [0.0, 37.0, 90.0, 270.0] {
                for (flip_h, flip_v) in [(false, false), (true, false), (false, true), (true, true)]
                {
                    for (whole_h, whole_v) in
                        [(false, false), (true, false), (false, true), (true, true)]
                    {
                        let mut state = fixture(dpi);
                        state.scene_measurement_renderer = measurement_renderer.take();
                        if fractional_frame {
                            state.elements[0].as_object_mut().unwrap().frame.x_mm = 7.33;
                            let object = state.elements[index].as_object_mut().unwrap();
                            object.frame.y_mm = 0.21;
                            object.frame.height_mm = 8.8;
                            let content = object.content_box.as_mut().unwrap();
                            content.width_mm = 6.0;
                            content.height_mm = 2.2;
                            content.offset_x_mm = 0.27;
                            content.offset_y_mm = 0.15;
                        }
                        let object = state.elements[index].as_object_mut().unwrap();
                        object.rotation = rotation;
                        object.flip_h = flip_h;
                        object.flip_v = flip_v;
                        state.overall_flip_h = whole_h;
                        state.overall_flip_v = whole_v;
                        let frames = scene::world_frames(&state.elements).unwrap();
                        let original = state.elements[index].as_object().unwrap().clone();
                        let center = content_center_mm(&state, index);
                        let bitmap = render(&state, &mut renderer);
                        let isolated =
                            isolated_object(&state, index, bitmap.width(), &mut renderer);
                        assert!(state.set_positioning(index, Positioning::Free).unwrap());
                        assert_other_frames_unchanged(&state, &frames, &[index]);
                        let object = state.elements[index].as_object().unwrap();
                        let content = object.content_box.as_ref().unwrap();
                        let old_content = original.content_box.as_ref().unwrap();
                        assert_eq!(content.width_mm, old_content.width_mm);
                        assert_eq!(content.height_mm, old_content.height_mm);
                        assert_eq!(object.rotation, original.rotation);
                        assert_eq!(object.flip_h, original.flip_h);
                        assert_eq!(object.flip_v, original.flip_v);
                        let after_center = content_center_mm(&state, index);
                        assert!((center.0 - after_center.0).abs() < 1e-9);
                        assert!((center.1 - after_center.1).abs() < 1e-9);
                        assert_eq!(state.overall_flip_h, whole_h);
                        assert_eq!(state.overall_flip_v, whole_v);
                        // Old mirrors use rounded allocation edges. The new
                        // free frame can change that pixel axis by two dots
                        // while retaining the millimeter center and size.
                        let after = isolated_object(&state, index, bitmap.width(), &mut renderer);
                        assert_at_most_two_pixel_translation(&after, &isolated);
                        if !flip_h && !flip_v {
                            assert_pixels_equal(&render(&state, &mut renderer), &bitmap);
                        }
                        measurement_renderer = state.scene_measurement_renderer.take();
                    }
                }
            }
        }
    }
}

#[test]
fn every_content_type_and_multiple_attachments_keep_size_content_and_pixels() {
    use ptouch_render::{
        shape::{ShapeKind, ShapeStyle},
        stroke::{BorderStyle, PatternKind, StrokePattern},
        text::TextAlign,
    };
    let mut source = image::RgbaImage::new(7, 5);
    for (x, y, pixel) in source.enumerate_pixels_mut() {
        *pixel = if (x + y * 3) % 5 < 2 {
            image::Rgba([0, 0, 0, 255])
        } else {
            image::Rgba([255, 255, 255, 255])
        };
    }
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(source)
        .write_to(&mut encoded, image::ImageFormat::Png)
        .unwrap();
    let contents = [
        ObjectContent::Text {
            content: "PORT".into(),
            font_size: Some(6.0),
            align: TextAlign::Center,
            line_spacing: ptouch_render::text::DEFAULT_LINE_SPACING,
            styles: Vec::new(),
        },
        ObjectContent::Image {
            path: None,
            image_data: encoded.into_inner(),
            bitmap: None,
            preserve_alpha: false,
            coverage: None,
        },
        ObjectContent::Line {
            pattern: StrokePattern {
                kind: PatternKind::Dotted,
                ..Default::default()
            },
        },
        ObjectContent::Shape {
            style: ShapeStyle {
                kind: ShapeKind::Ellipse,
                ..Default::default()
            },
        },
        ObjectContent::CutMark,
        ObjectContent::Padding,
    ];
    let mut renderer = TextRenderer::new();
    for dpi in [180, 360] {
        for (kind, content) in contents.iter().enumerate() {
            let mut state = fixture(dpi);
            let object = state.elements[2].as_object_mut().unwrap();
            object.content = content.clone();
            if kind == 0 {
                object.border = Some(BorderStyle {
                    width_mm: 0.2,
                    padding_mm: 0.1,
                    ..Default::default()
                });
            }
            let mut second_child = line(7, 3);
            second_child.rotation = 0.0;
            second_child.content_box.as_mut().unwrap().width_mm = 3.3;
            second_child.scene.as_mut().unwrap().anchor = Some(Anchor {
                gap_center: None,
                target: 3,
                horizontal: 0.0,
                vertical: 0.0,
                offset_x_mm: 1.0,
                offset_y_mm: 1.0,
            });
            object_layout::update_allocation(&mut second_child);
            state.elements.push(LabelElement::Object(second_child));
            let mut decoded = state.document();
            decoded.decode_image_caches().unwrap();
            state.elements = decoded.elements;
            state.validate_scene_geometry().unwrap();
            let frames = scene::world_frames(&state.elements).unwrap();
            let center = content_center_mm(&state, 2);
            let original = state.elements[2].as_object().unwrap().clone();
            let before = render(&state, &mut renderer);
            state.set_positioning(2, Positioning::Free).unwrap();
            let object = state.elements[2].as_object().unwrap();
            assert_eq!(object.content, original.content);
            assert_eq!(object.border, original.border);
            let geometry = object.content_box.as_ref().unwrap();
            let old_geometry = original.content_box.as_ref().unwrap();
            assert_eq!(geometry.width_mm, old_geometry.width_mm);
            assert_eq!(geometry.height_mm, old_geometry.height_mm);
            assert_eq!(center, content_center_mm(&state, 2));
            assert_other_frames_unchanged(&state, &frames, &[2]);
            assert_pixels_equal(&render(&state, &mut renderer), &before);
            let saved = state.document().to_toml_string().unwrap();
            let reopened = LabelDocument::from_toml_str(&saved).unwrap();
            assert_eq!(reopened, state.document());
        }
    }
}

#[test]
fn fractional_mirror_boundaries_can_change_the_raster_position_by_two_dots() {
    let mut renderer = TextRenderer::new();
    for index in [0, 2, 4] {
        let mut state = fixture(180);
        let object = state.elements[index].as_object_mut().unwrap();
        object.frame.y_mm = 0.4910217098401981;
        object.frame.height_mm = 7.477225583070865;
        object.rotation = 270.0;
        object.flip_v = true;
        let content = object.content_box.as_mut().unwrap();
        content.width_mm = 6.4315450398354335;
        content.height_mm = 1.074784811255505;
        content.offset_y_mm = 0.35714375331412496;
        state.validate_scene_geometry().unwrap();
        let frames = scene::world_frames(&state.elements).unwrap();
        let center = content_center_mm(&state, index);
        let width = render(&state, &mut renderer).width();
        let before = isolated_object(&state, index, width, &mut renderer);
        state.set_positioning(index, Positioning::Free).unwrap();
        let after = isolated_object(&state, index, width, &mut renderer);
        assert_eq!(
            assert_at_most_two_pixel_translation(&after, &before),
            (0, 2)
        );
        assert_eq!(center, content_center_mm(&state, index));
        assert_other_frames_unchanged(&state, &frames, &[index]);
    }
}

#[test]
fn content_at_the_printable_edges_is_not_clamped_during_conversion() {
    let mut renderer = TextRenderer::new();
    for dpi in [180, 360] {
        for bottom_edge in [false, true] {
            let mut state = fixture(dpi);
            let height = state.printable_height_mm();
            let object = state.elements[0].as_object_mut().unwrap();
            object.frame.x_mm = 0.0;
            let content = object.content_box.as_mut().unwrap();
            content.offset_x_mm = -3.25;
            content.offset_y_mm = if bottom_edge {
                (height - content.height_mm) / 2.0
            } else {
                -(height - content.height_mm) / 2.0
            };
            let frames = scene::world_frames(&state.elements).unwrap();
            let center = content_center_mm(&state, 0);
            let before = render(&state, &mut renderer);
            state.set_positioning(0, Positioning::Free).unwrap();
            assert_other_frames_unchanged(&state, &frames, &[0]);
            assert_eq!(center, content_center_mm(&state, 0));
            assert_pixels_equal(&render(&state, &mut renderer), &before);
        }
    }
}

#[test]
fn whole_label_horizontal_mirror_rejects_a_conversion_that_shortens_the_tape() {
    let mut renderer = TextRenderer::new();
    for dpi in [180, 360] {
        let mut state = fixture(dpi);
        // The last text has no attached line to retain the old tape endpoint.
        state.elements.pop();
        let original_width = render(&state, &mut renderer).width();
        state.overall_flip_h = true;
        let before = unchanged_state(&state);
        assert!(state.set_positioning(4, Positioning::Free).is_err());
        assert_eq!(unchanged_state(&state), before);

        state.overall_flip_h = false;
        let frames = scene::world_frames(&state.elements).unwrap();
        assert!(state.set_positioning(4, Positioning::Free).unwrap());
        assert!(render(&state, &mut renderer).width() < original_width);
        assert_other_frames_unchanged(&state, &frames, &[4]);
    }
}

#[test]
fn joining_free_objects_preserves_the_previous_hosts_attachments() {
    let mut renderer = TextRenderer::new();
    for dpi in [180, 360] {
        let mut state = fixture(dpi);
        state.set_positioning(0, Positioning::Free).unwrap();
        state.set_positioning(2, Positioning::Free).unwrap();
        let before_frames = scene::world_frames(&state.elements).unwrap();
        let before = render(&state, &mut renderer);
        assert!(state.set_positioning(2, Positioning::Sequence).unwrap());
        let first = state.elements[0].as_object().unwrap();
        let second = state.elements[2].as_object().unwrap();
        assert_eq!(first.sequence, second.sequence);
        assert_eq!(second.frame.position, PositionMode::Relative);
        assert_other_frames_unchanged(&state, &before_frames, &[0, 2]);
        assert_pixels_equal(&render(&state, &mut renderer), &before);
    }
}

#[test]
fn absolute_split_and_after_previous_join_preserve_all_geometry() {
    let mut renderer = TextRenderer::new();
    for dpi in [180, 360] {
        let mut state = fixture(dpi);
        let frames = scene::world_frames(&state.elements).unwrap();
        let before = render(&state, &mut renderer);
        assert!(state.set_positioning(2, Positioning::Absolute).unwrap());
        assert_ne!(
            state.elements[0].as_object().unwrap().sequence,
            state.elements[2].as_object().unwrap().sequence
        );
        assert_eq!(
            state.elements[2].as_object().unwrap().sequence,
            state.elements[4].as_object().unwrap().sequence
        );
        assert_other_frames_unchanged(&state, &frames, &[]);
        assert_pixels_equal(&render(&state, &mut renderer), &before);
        assert!(
            state
                .set_positioning(2, Positioning::AfterPrevious)
                .unwrap()
        );
        assert_eq!(
            state.elements[0].as_object().unwrap().sequence,
            state.elements[4].as_object().unwrap().sequence
        );
        assert_other_frames_unchanged(&state, &frames, &[]);
        assert_pixels_equal(&render(&state, &mut renderer), &before);
    }
}

#[test]
fn one_undo_restores_the_entire_relationship_change() {
    let mut state = fixture(180);
    let before = state.document();
    state
        .history
        .observe(before.clone(), Some(2), false, false, 0.0);
    state.set_positioning(2, Positioning::Free).unwrap();
    let after = state.document();
    state
        .history
        .observe(after.clone(), Some(2), false, false, 1.0);
    assert!(state.history.can_undo());
    state.history_step(false);
    assert_eq!(state.document(), before);
    assert!(!state.history.can_undo());
    state.history_step(true);
    assert_eq!(state.document(), after);
}

#[test]
fn overlap_and_missing_previous_fail_without_changes() {
    let mut state = fixture(180);
    state.set_positioning(2, Positioning::Free).unwrap();
    state.elements[2].as_object_mut().unwrap().frame.x_mm = 1.0;
    state.validate_scene_geometry().unwrap();
    for choice in [Positioning::Sequence, Positioning::AfterPrevious] {
        let before = unchanged_state(&state);
        assert!(state.set_positioning(2, choice).is_err());
        assert_eq!(unchanged_state(&state), before);
    }
    let before = unchanged_state(&state);
    assert!(
        state
            .set_positioning(0, Positioning::AfterPrevious)
            .is_err()
    );
    assert_eq!(unchanged_state(&state), before);
}

#[test]
fn unrepresentable_edge_geometry_rolls_back_the_full_transaction() {
    for (offset_x, offset_y) in [(-6.0, 0.0), (0.0, -6.0), (0.0, 6.0)] {
        let mut state = fixture(180);
        let object = state.elements[0].as_object_mut().unwrap();
        object.frame.x_mm = 0.0;
        let content = object.content_box.as_mut().unwrap();
        content.offset_x_mm = offset_x;
        content.offset_y_mm = offset_y;
        state.validate_scene_geometry().unwrap();
        let before = unchanged_state(&state);
        assert!(state.set_positioning(0, Positioning::Free).is_err());
        assert_eq!(unchanged_state(&state), before);
    }
}

#[test]
fn selection_queries_and_no_op_changes_leave_the_old_document_unchanged() {
    let mut state = fixture(180);
    let before = unchanged_state(&state);
    let frames = state.position_frames();
    assert_eq!(frames.len(), 6);
    assert!((frames[2].as_ref().unwrap().x_mm - 22.85).abs() < 1e-9);
    assert!(!state.set_positioning(0, Positioning::Absolute).unwrap());
    assert!(!state.set_positioning(2, Positioning::Sequence).unwrap());
    assert!(
        !state
            .set_positioning(2, Positioning::AfterPrevious)
            .unwrap()
    );
    assert_eq!(unchanged_state(&state), before);
    for choice in [
        Positioning::Free,
        Positioning::Sequence,
        Positioning::Absolute,
        Positioning::AfterPrevious,
    ] {
        assert!(state.set_positioning(1, choice).is_err());
        assert_eq!(unchanged_state(&state), before);
    }
}

#[test]
fn legacy_fit_objects_require_an_explicit_size_before_free_or_sequence_conversion() {
    let mut renderer = TextRenderer::new();
    for version in [3, 4] {
        for dpi in [180, 360] {
            for rotation in [0.0, 37.0, 90.0] {
                for font in [None, Some(6.0)] {
                    let mut state = AppState {
                        document_version: version,
                        design_dpi: dpi,
                        selected_element: Some(0),
                        needs_rerender: false,
                        ..Default::default()
                    };
                    state.update_tape_pixels();
                    let mut object = LabelObject::text(
                        "PORT 01",
                        TextBox {
                            x_mm: 5.0,
                            y_mm: 1.0,
                            width_mm: 14.5,
                            height_mm: 8.0,
                            ..Default::default()
                        },
                    );
                    object.rotation = rotation;
                    if let ObjectContent::Text { font_size, .. } = &mut object.content {
                        *font_size = font;
                    }
                    let end = LabelObject::text(
                        "END",
                        TextBox {
                            x_mm: 30.0,
                            y_mm: 3.0,
                            width_mm: 5.0,
                            height_mm: 3.0,
                            ..Default::default()
                        },
                    );
                    state.elements = vec![LabelElement::Object(object), LabelElement::Object(end)];
                    let document = state.document();
                    let preview = ptouch_render::document::render_document(
                        &document,
                        state.tape_width_px,
                        dpi,
                        &mut renderer,
                    )
                    .unwrap();
                    state.preview_bounds = preview.bounds;
                    state.preview_content_bounds = preview.content_bounds;
                    state.preview_bitmap = preview.bitmap.clone();
                    state.history.observe(document, Some(0), false, false, 0.0);
                    assert!(state.resolved_content_box(0).is_some());
                    for choice in [Positioning::Free, Positioning::Sequence] {
                        let before = unchanged_state(&state);
                        assert!(state.set_positioning(0, choice).is_err());
                        assert_eq!(unchanged_state(&state), before);
                        assert_eq!(state.document().version, version);
                    }

                    // Absolute and relative frame relations retain the legacy
                    // fitting path. They do not create independent content.
                    assert!(
                        state
                            .set_positioning(1, Positioning::AfterPrevious)
                            .unwrap()
                    );
                    assert!(state.elements[1].as_object().unwrap().content_box.is_none());
                    assert_eq!(state.document().version, version);
                    assert_pixels_equal(
                        &render(&state, &mut renderer),
                        preview.bitmap.as_ref().unwrap(),
                    );
                    assert!(state.set_positioning(1, Positioning::Absolute).unwrap());
                    assert!(state.elements[1].as_object().unwrap().content_box.is_none());
                    assert_eq!(state.document().version, version);
                    assert_pixels_equal(
                        &render(&state, &mut renderer),
                        preview.bitmap.as_ref().unwrap(),
                    );
                }
            }
        }
    }
}
