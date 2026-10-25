// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

use std::collections::BTreeMap;

use crate::{
    document::{
        ContentBox, LabelDocument, LabelElement, LabelObject, ObjectContent, PositionMode, TextBox,
    },
    object_layout::{self, LayoutOptions, Placement, WidthSizing},
    scene::{self, Anchor, GapCenter, SceneOptions},
    sequence,
    stroke::{PatternKind, StrokePattern},
    text::TextRenderer,
};

fn host(id: u64, sequence: u64, x: f64, relative: bool, gap: f64) -> LabelElement {
    let mut object = LabelObject::text(
        format!("PORT {id}"),
        TextBox {
            width_mm: 14.5,
            height_mm: 8.0,
            x_mm: x,
            y_mm: 1.0,
            gap_mm: gap,
            position: if relative {
                PositionMode::Relative
            } else {
                PositionMode::Absolute
            },
            ..Default::default()
        },
    );
    object.sequence = Some(sequence);
    object.content_box = Some(ContentBox {
        width_mm: 8.0,
        height_mm: 3.0,
        offset_x_mm: 0.0,
        offset_y_mm: 0.0,
    });
    object.layout = Some(LayoutOptions {
        placement: Placement::Flow,
        width: WidthSizing::Fixed,
        constrain: true,
        padding_mm: 0.0,
    });
    object.scene = Some(Box::new(SceneOptions {
        id,
        layer: id as i32,
        anchor: None,
    }));
    LabelElement::Object(object)
}

fn marker(id: u64, target: u64, tail: f64) -> LabelElement {
    let mut object = LabelObject::text("", TextBox::default());
    object.content = ObjectContent::Line {
        pattern: StrokePattern {
            kind: PatternKind::Dotted,
            ..Default::default()
        },
    };
    object.rotation = 90.0;
    object.content_box = Some(ContentBox {
        width_mm: 8.0,
        height_mm: 0.3,
        offset_x_mm: 0.0,
        offset_y_mm: 0.0,
    });
    object.layout = Some(LayoutOptions {
        placement: Placement::Free,
        width: WidthSizing::Fixed,
        constrain: true,
        padding_mm: 0.0,
    });
    object.scene = Some(Box::new(SceneOptions {
        id,
        layer: id as i32,
        anchor: Some(Anchor {
            target,
            horizontal: 1.0,
            vertical: 0.5,
            offset_x_mm: 1.825,
            offset_y_mm: 0.0,
            gap_center: Some(GapCenter { tail_gap_mm: tail }),
        }),
    }));
    object_layout::update_allocation(&mut object);
    LabelElement::Object(object)
}

fn document() -> LabelDocument {
    LabelDocument {
        version: 14,
        tape_width_mm: 12,
        dpi: 180,
        font_name: "Droid Sans Fallback".into(),
        font_margin: 0,
        flip_h: false,
        flip_v: false,
        elements: vec![
            host(1, 7, 5.0, false, 3.35),
            marker(2, 1, 3.35),
            host(3, 7, 0.0, true, 3.35),
            marker(4, 3, 3.35),
            host(5, 7, 0.0, true, 3.35),
            marker(6, 5, 3.35),
        ],
    }
}

fn anchor(element: &LabelElement) -> &Anchor {
    element
        .as_object()
        .unwrap()
        .scene
        .as_ref()
        .unwrap()
        .anchor
        .as_ref()
        .unwrap()
}

fn anchor_mut(element: &mut LabelElement) -> &mut Anchor {
    element
        .as_object_mut()
        .unwrap()
        .scene
        .as_mut()
        .unwrap()
        .anchor
        .as_mut()
        .unwrap()
}

fn index(elements: &[LabelElement], id: u64) -> usize {
    elements
        .iter()
        .position(|e| e.as_object().unwrap().scene.as_ref().unwrap().id == id)
        .unwrap()
}

fn close(actual: f64, expected: f64) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "{actual} differs from {expected}"
    );
}

fn center(elements: &[LabelElement], child: u64) -> f64 {
    let frames = scene::world_frames(elements).unwrap();
    let frame = &frames[index(elements, child)];
    frame.x_mm + frame.width_mm / 2.0
}

#[test]
fn markers_follow_real_gaps_and_the_saved_tail_without_occupying_space() {
    let mut doc = document();
    let original = doc.elements.clone();
    close(center(&doc.elements, 2), 5.0 + 14.5 + 1.675);
    close(center(&doc.elements, 4), 22.85 + 14.5 + 1.675);
    close(center(&doc.elements, 6), 40.7 + 14.5 + 1.675);
    assert_eq!(scene::next_layout_host(&doc.elements, 0), Some(2));
    assert_eq!(scene::next_layout_host(&doc.elements, 1), None);
    close(scene::gap_after(&doc.elements, 0).unwrap().unwrap(), 3.35);
    assert_eq!(scene::gap_after(&doc.elements, 4).unwrap(), None);
    assert!(scene::gap_after(&doc.elements, 1).is_err());
    assert!(scene::gap_after(&doc.elements, 100).is_err());
    assert_eq!(doc.elements, original);

    doc.elements[2].as_object_mut().unwrap().frame.gap_mm = 6.0;
    close(center(&doc.elements, 2), 5.0 + 14.5 + 3.0);
    anchor_mut(&mut doc.elements[5])
        .gap_center
        .as_mut()
        .unwrap()
        .tail_gap_mm = 8.0;
    let frames = scene::world_frames(&doc.elements).unwrap();
    close(center(&doc.elements, 6), frames[4].x_mm + 14.5 + 4.0);
    close(frames[2].x_mm, 25.5);
}

#[test]
fn gap_mode_ignores_old_manual_x_and_keeps_y_alignment() {
    let mut doc = document();
    let expected = scene::world_frames(&doc.elements).unwrap();
    let anchor = anchor_mut(&mut doc.elements[1]);
    anchor.horizontal = 0.0;
    anchor.offset_x_mm = 900.0;
    anchor.offset_y_mm = 0.2;
    let actual = scene::world_frames(&doc.elements).unwrap();
    close(actual[1].x_mm, expected[1].x_mm);
    close(actual[1].y_mm, expected[1].y_mm + 0.2);
}

#[test]
fn interleaved_sequences_and_reordering_use_the_current_successor() {
    let mut doc = document();
    doc.elements.extend([
        host(7, 9, 100.0, false, 7.0),
        marker(8, 7, 7.0),
        host(9, 9, 0.0, true, 7.0),
        marker(10, 9, 7.0),
    ]);
    let from = index(&doc.elements, 3);
    let to = index(&doc.elements, 9);
    sequence::reorder(&mut doc.elements, from, to, 14).unwrap();
    assert_eq!(
        doc.elements[index(&doc.elements, 3)]
            .as_object()
            .unwrap()
            .sequence,
        Some(9)
    );
    let frames = scene::world_frames(&doc.elements).unwrap();
    let host9 = index(&doc.elements, 9);
    close(
        scene::gap_after(&doc.elements, host9).unwrap().unwrap(),
        3.35,
    );
    close(center(&doc.elements, 10), frames[host9].x_mm + 14.5 + 1.675);
    let moved = index(&doc.elements, 3);
    assert_eq!(scene::gap_after(&doc.elements, moved).unwrap(), None);
    close(center(&doc.elements, 4), frames[moved].x_mm + 14.5 + 1.675);
    let host7 = index(&doc.elements, 7);
    close(center(&doc.elements, 8), frames[host7].x_mm + 14.5 + 3.5);

    // A physically unrelated row and an attachment can precede the successor.
    let mut independent = host(11, 12, 200.0, false, 0.0);
    independent.as_object_mut().unwrap().sequence = None;
    doc.elements.insert(host7 + 1, independent);
    assert_eq!(
        scene::next_layout_host(&doc.elements, index(&doc.elements, 7)),
        Some(index(&doc.elements, 9))
    );
}

#[test]
fn implicit_chains_can_be_materialized_without_moving_the_marker() {
    let mut doc = document();
    for element in &mut doc.elements {
        element.as_object_mut().unwrap().sequence = None;
    }
    let before = scene::world_frames(&doc.elements).unwrap();
    assert_eq!(scene::next_layout_host(&doc.elements, 0), Some(2));
    sequence::materialize(&mut doc.elements).unwrap();
    assert_eq!(scene::world_frames(&doc.elements).unwrap(), before);
    let mut standalone = document();
    standalone.elements.truncate(2);
    standalone.elements[0].as_object_mut().unwrap().sequence = None;
    assert_eq!(scene::gap_after(&standalone.elements, 0).unwrap(), None);
    close(center(&standalone.elements, 2), 21.175);
}

#[test]
fn zero_and_narrow_gaps_keep_the_requested_spacing_and_stroke_size() {
    let mut doc = document();
    for gap in [0.0, 0.1] {
        doc.elements[2].as_object_mut().unwrap().frame.gap_mm = gap;
        let before = doc.elements.clone();
        close(center(&doc.elements, 2), 5.0 + 14.5 + gap / 2.0);
        let mut renderer = TextRenderer::with_test_font();
        crate::document::render_document(&doc, 76, 180, &mut renderer).unwrap();
        assert_eq!(doc.elements, before);
        close(
            doc.elements[1]
                .as_object()
                .unwrap()
                .content_box
                .as_ref()
                .unwrap()
                .height_mm,
            0.3,
        );
    }
}

#[test]
fn the_new_anchor_round_trips_and_requires_version_fourteen() {
    let mut doc = document();
    let saved = doc.to_toml_string().unwrap();
    assert!(saved.contains("[elements.scene.anchor.gap_center]"));
    assert_eq!(LabelDocument::from_toml_str(&saved).unwrap(), doc);
    assert!(
        LabelDocument::from_toml_str(&saved.replacen("version = 14", "version = 13", 1)).is_err()
    );
    assert!(scene::validate(&doc.elements, 13).is_err());
    doc.version = 13;
    assert!(
        doc.elements[1]
            .as_object()
            .unwrap()
            .validate_features(13)
            .is_err()
    );
    assert!(doc.to_toml_string().unwrap().starts_with("version = 14"));
    assert_eq!(doc.elements[1].minimum_version(), 14);
}

#[test]
fn invalid_tail_values_and_overlapping_sequence_gaps_are_rejected() {
    for tail in [-0.01, 10_000.01, f64::NAN, f64::INFINITY] {
        let mut doc = document();
        anchor_mut(&mut doc.elements[1]).gap_center = Some(GapCenter { tail_gap_mm: tail });
        assert!(scene::validate(&doc.elements, 14).is_err());
        assert!(scene::world_frames(&doc.elements).is_err());
        assert!(doc.to_toml_string().is_err());
    }
    for tail in [0.0, 10_000.0] {
        GapCenter { tail_gap_mm: tail }.validate().unwrap();
    }
    let mut doc = document();
    doc.elements[2].as_object_mut().unwrap().frame.gap_mm = -0.3;
    assert!(scene::world_frames(&doc.elements).is_err());
    close(doc.elements[2].as_object().unwrap().frame.gap_mm, -0.3);
    let saved = document().to_toml_string().unwrap();
    assert!(
        LabelDocument::from_toml_str(&saved.replacen(
            "tail_gap_mm = 3.35",
            "tail_gap_mm = 3.35\nunknown_field = 1",
            1,
        ))
        .is_err()
    );
    for value in ["nan", "inf", "-0.1", "10000.1"] {
        assert!(
            LabelDocument::from_toml_str(&saved.replacen(
                "tail_gap_mm = 3.35",
                &format!("tail_gap_mm = {value}"),
                1
            ))
            .is_err()
        );
    }
}

#[test]
fn old_manual_anchors_keep_their_version_fields_and_pixels() {
    let mut legacy = document();
    legacy.version = 13;
    for element in &mut legacy.elements {
        if let Some(anchor) = element
            .as_object_mut()
            .unwrap()
            .scene
            .as_mut()
            .unwrap()
            .anchor
            .as_mut()
        {
            anchor.gap_center = None;
        }
    }
    let saved = legacy.to_toml_string().unwrap();
    assert!(!saved.contains("gap_center"));
    let reopened = LabelDocument::from_toml_str(&saved).unwrap();
    assert_eq!(reopened.version, 13);
    assert_eq!(reopened.to_toml_string().unwrap(), saved);
    let modern = document();
    let mut renderer = TextRenderer::with_test_font();
    for (dpi, height) in [(180, 76), (360, 150)] {
        let before = crate::document::render_document(&legacy, height, dpi, &mut renderer).unwrap();
        let after = crate::document::render_document(&modern, height, dpi, &mut renderer).unwrap();
        assert_eq!(before.bitmap, after.bitmap);
    }
    assert_eq!(legacy.to_toml_string().unwrap(), saved);
}

#[test]
fn template_values_and_output_dpi_do_not_change_physical_spacing() {
    let mut doc = document();
    if let ObjectContent::Text { content, .. } =
        &mut doc.elements[0].as_object_mut().unwrap().content
    {
        *content = "{{port}}".into();
    }
    let expected = scene::world_frames(&doc.elements).unwrap();
    let mut renderer = TextRenderer::with_test_font();
    for text in ["1", "PORT 01", ""] {
        let mut filled = doc.clone();
        filled.apply_values(&BTreeMap::from([("port".into(), text.into())]));
        assert_eq!(scene::world_frames(&filled.elements).unwrap(), expected);
        for (dpi, height) in [(180, 76), (360, 150)] {
            let rendered =
                crate::document::render_document(&filled, height, dpi, &mut renderer).unwrap();
            for child in [1, 3, 5] {
                let bound = rendered
                    .bounds
                    .iter()
                    .find(|b| b.element_index == child)
                    .unwrap();
                let center_mm =
                    (f64::from(bound.x) + f64::from(bound.width) / 2.0) * 25.4 / f64::from(dpi);
                let true_center = expected[child].x_mm + expected[child].width_mm / 2.0;
                assert!((center_mm - true_center).abs() <= 25.4 / f64::from(dpi));
            }
        }
    }
}

#[test]
fn expressing_a_world_frame_as_offsets_explicitly_returns_to_manual_positioning() {
    let doc = document();
    let frames = scene::world_frames(&doc.elements).unwrap();
    let original = anchor(&doc.elements[1]);
    let manual = scene::offset_for_frame(original, &frames[1], &frames[0]);
    assert_eq!(manual.gap_center, None);
    let mut changed = doc.clone();
    *anchor_mut(&mut changed.elements[1]) = manual;
    assert_eq!(scene::world_frames(&changed.elements).unwrap(), frames);
}

#[test]
fn public_twenty_four_port_example_has_exact_pitch_and_centered_markers() {
    let doc =
        LabelDocument::from_toml_str(include_str!("../../../examples/patch-panel-24-ports.ptl"))
            .unwrap();
    assert_eq!(doc.version, 14);
    assert_eq!(doc.elements.len(), 48);
    let frames = scene::world_frames(&doc.elements).unwrap();
    for port in 0..24 {
        close(frames[port * 2].x_mm, port as f64 * 17.85);
        close(
            frames[port * 2 + 1].x_mm + frames[port * 2 + 1].width_mm / 2.0,
            port as f64 * 17.85 + 14.5 + 1.675,
        );
    }
    let mut renderer = TextRenderer::with_test_font();
    for (dpi, height) in [(180, 76), (360, 150)] {
        let rendered = crate::document::render_document(&doc, height, dpi, &mut renderer).unwrap();
        assert!(rendered.bitmap.is_some());
        assert!(rendered.diagnostics.is_empty());
    }
    let saved = doc.to_toml_string().unwrap();
    assert_eq!(LabelDocument::from_toml_str(&saved).unwrap(), doc);
}
