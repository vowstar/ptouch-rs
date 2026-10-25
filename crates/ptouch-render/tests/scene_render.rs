// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! End-to-end rendering contracts for shapes, layers, transparency and anchors.

use std::{cell::RefCell, io::Cursor};

use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use ptouch_render::{
    bitmap::LabelBitmap,
    document::{
        ContentBox, LabelDocument, LabelElement, LabelObject, ObjectContent, PositionMode, TextBox,
        render_document,
    },
    object_layout::{LayoutOptions, Placement, WidthSizing},
    scene::{self, Anchor},
    shape::{ShapeFill, ShapeKind, ShapeStyle},
    stroke::StrokePattern,
    text::TextRenderer,
};

const MM_PER_DOT: f64 = 25.4 / 180.0;

thread_local! {
    static RENDERER: RefCell<TextRenderer> = RefCell::new(TextRenderer::new());
}

fn mm(dots: f64) -> f64 {
    dots * MM_PER_DOT
}

fn shape(x: f64, y: f64, width: f64, height: f64, fill: ShapeFill) -> LabelElement {
    let frame = TextBox {
        x_mm: mm(x),
        y_mm: mm(y),
        width_mm: mm(width),
        height_mm: mm(height),
        ..Default::default()
    };
    let mut object = LabelObject::text("", frame);
    object.content_box = Some(ContentBox {
        width_mm: mm(width),
        height_mm: mm(height),
        offset_x_mm: 0.0,
        offset_y_mm: 0.0,
    });
    object.content = ObjectContent::Shape {
        style: ShapeStyle {
            fill,
            stroke_width_mm: 0.0,
            ..Default::default()
        },
    };
    LabelElement::Object(object)
}

fn document(elements: Vec<LabelElement>) -> LabelDocument {
    LabelDocument {
        version: 12,
        tape_width_mm: 12,
        dpi: 180,
        font_name: String::new(),
        font_margin: 0,
        flip_h: false,
        flip_v: false,
        elements,
    }
}

fn render(doc: &LabelDocument, dpi: u16) -> LabelBitmap {
    RENDERER.with(|renderer| {
        let output = render_document(
            doc,
            76 * u32::from(dpi / 180),
            dpi,
            &mut renderer.borrow_mut(),
        )
        .unwrap();
        assert!(output.diagnostics.is_empty(), "{:?}", output.diagnostics);
        output.bitmap.unwrap()
    })
}

fn set_layer(element: &mut LabelElement, layer: i32) {
    element
        .as_object_mut()
        .unwrap()
        .scene
        .as_mut()
        .unwrap()
        .layer = layer;
}

fn id(element: &LabelElement) -> u64 {
    element.as_object().unwrap().scene.as_ref().unwrap().id
}

fn by_id(doc: &LabelDocument, object_id: u64) -> usize {
    doc.elements
        .iter()
        .position(|element| id(element) == object_id)
        .unwrap()
}

fn assert_dot(bitmap: &LabelBitmap, dpi: u16, x: u32, y: u32, ink: bool) {
    let scale = u32::from(dpi / 180);
    assert_eq!(
        bitmap.get_pixel(x * scale, y * scale),
        ink,
        "{x},{y} at {dpi} DPI"
    );
}

#[test]
fn paper_rotated_ellipse_erases_ink_without_erasing_its_empty_bounding_corners() {
    let mut ellipse = shape(35.0, 22.0, 30.0, 16.0, ShapeFill::Paper);
    let object = ellipse.as_object_mut().unwrap();
    object.rotation = 45.0;
    if let ObjectContent::Shape { style } = &mut object.content {
        style.kind = ShapeKind::Ellipse;
    }
    let mut doc = document(vec![shape(0.0, 0.0, 100.0, 60.0, ShapeFill::Ink), ellipse]);
    scene::enable(&mut doc.elements).unwrap();
    for dpi in [180, 360] {
        let bitmap = render(&doc, dpi);
        assert_dot(&bitmap, dpi, 50, 30, false);
        // These points are inside the 33-dot rotated bounding box and outside
        // the actual oval, so the underlying black rectangle remains visible.
        for (x, y) in [(35, 15), (65, 15), (35, 45), (65, 45)] {
            assert_dot(&bitmap, dpi, x, y, true);
        }
    }
}

#[test]
fn changing_layer_changes_printed_pixels_without_changing_sequence_geometry() {
    let mut background = shape(0.0, 0.0, 80.0, 60.0, ShapeFill::Ink);
    background.as_object_mut().unwrap().sequence = Some(7);
    let mut follower = shape(0.0, 10.0, 10.0, 10.0, ShapeFill::Ink);
    let object = follower.as_object_mut().unwrap();
    object.sequence = Some(7);
    object.frame.position = PositionMode::Relative;
    object.frame.gap_mm = mm(5.0);
    let mut doc = document(vec![
        background,
        follower,
        shape(30.0, 20.0, 20.0, 20.0, ShapeFill::Paper),
    ]);
    scene::enable(&mut doc.elements).unwrap();
    let geometry: Vec<_> = doc
        .elements
        .iter()
        .map(|element| {
            let object = element.as_object().unwrap();
            (object.frame.clone(), object.sequence)
        })
        .collect();
    let positions = scene::world_frames(&doc.elements).unwrap();
    let before = [render(&doc, 180), render(&doc, 360)];
    set_layer(&mut doc.elements[0], 10);
    assert_eq!(scene::world_frames(&doc.elements).unwrap(), positions);
    for (element, (frame, sequence)) in doc.elements.iter().zip(geometry) {
        let object = element.as_object().unwrap();
        assert_eq!(object.frame, frame);
        assert_eq!(object.sequence, sequence);
    }
    for (dpi, previous) in [180, 360].into_iter().zip(before) {
        let after = render(&doc, dpi);
        assert_dot(&previous, dpi, 40, 30, false);
        assert_dot(&after, dpi, 40, 30, true);
        assert_ne!(previous, after);
    }
}

#[test]
fn equal_layers_use_persistent_identity_after_physical_list_reordering() {
    let mut doc = document(vec![
        shape(0.0, 0.0, 100.0, 60.0, ShapeFill::Ink),
        shape(30.0, 20.0, 20.0, 20.0, ShapeFill::Paper),
    ]);
    scene::enable(&mut doc.elements).unwrap();
    for element in &mut doc.elements {
        set_layer(element, 3);
    }
    let before = [render(&doc, 180), render(&doc, 360)];
    doc.elements.swap(0, 1);
    for (dpi, previous) in [180, 360].into_iter().zip(before) {
        let after = render(&doc, dpi);
        assert_dot(&after, dpi, 40, 30, false);
        assert_eq!(previous, after);
    }
}

fn png_icon() -> Vec<u8> {
    // Hidden black RGB in the transparent middle must never become ink or
    // replace the underlying pixels. The left third is opaque paper white.
    let image = RgbaImage::from_fn(24, 12, |x, _| match x {
        0..8 => Rgba([255, 255, 255, 255]),
        8..16 => Rgba([0, 0, 0, 0]),
        _ => Rgba([0, 0, 0, 255]),
    });
    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image)
        .write_to(&mut bytes, ImageFormat::Png)
        .unwrap();
    bytes.into_inner()
}

fn svg_icon() -> Vec<u8> {
    br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="12"><rect width="8" height="12" fill="white"/><rect x="16" width="8" height="12" fill="black"/></svg>"##.to_vec()
}

fn alpha_scene(bytes: Vec<u8>) -> LabelDocument {
    let frame = TextBox {
        x_mm: mm(20.0),
        y_mm: mm(20.0),
        width_mm: mm(24.0),
        height_mm: mm(12.0),
        ..Default::default()
    };
    let mut icon = LabelObject::image_from_bytes(None, bytes, frame);
    icon.content_box = Some(ContentBox {
        width_mm: mm(24.0),
        height_mm: mm(12.0),
        offset_x_mm: 0.0,
        offset_y_mm: 0.0,
    });
    if let ObjectContent::Image {
        preserve_alpha,
        bitmap,
        coverage,
        ..
    } = &mut icon.content
    {
        *preserve_alpha = true;
        *bitmap = None;
        *coverage = None;
    }
    let mut doc = document(vec![
        shape(0.0, 0.0, 100.0, 60.0, ShapeFill::Paper),
        shape(0.0, 0.0, 36.0, 60.0, ShapeFill::Ink),
        LabelElement::Object(icon),
    ]);
    scene::enable(&mut doc.elements).unwrap();
    doc.decode_image_caches().unwrap();
    doc
}

#[test]
fn png_and_svg_transparency_share_composition_and_object_transform_geometry() {
    let mut png = alpha_scene(png_icon());
    let mut svg = alpha_scene(svg_icon());
    for dpi in [180, 360] {
        let baseline = render(&png, dpi);
        assert_dot(&baseline, dpi, 23, 25, false); // opaque white erases black
        assert_dot(&baseline, dpi, 31, 25, true); // transparent preserves black
        assert_dot(&baseline, dpi, 39, 25, true); // opaque black covers paper
        for rotation in [0.0, 90.0, 37.0] {
            for (flip_h, flip_v) in [(false, false), (true, false), (false, true), (true, true)] {
                for doc in [&mut png, &mut svg] {
                    let icon = doc.elements[2].as_object_mut().unwrap();
                    icon.rotation = rotation;
                    icon.flip_h = flip_h;
                    icon.flip_v = flip_v;
                }
                let png_bitmap = render(&png, dpi);
                assert_eq!(png_bitmap, render(&svg, dpi));
                if rotation == 90.0 && !flip_h && !flip_v {
                    assert_dot(&png_bitmap, dpi, 31, 17, false);
                    assert_dot(&png_bitmap, dpi, 31, 25, true);
                    assert_dot(&png_bitmap, dpi, 31, 33, true);
                }
            }
        }
        // The next resolution starts from the same untransformed fixture.
        for doc in [&mut png, &mut svg] {
            let icon = doc.elements[2].as_object_mut().unwrap();
            icon.rotation = 0.0;
            icon.flip_h = false;
            icon.flip_v = false;
        }
    }
}

fn flow_scene(tagged: bool) -> (LabelDocument, u64, u64, u64) {
    let mut host = shape(5.0, 10.0, 20.0, 20.0, ShapeFill::Ink);
    let mut child = shape(500.0, 50.0, 6.0, 6.0, ShapeFill::Paper);
    child.as_object_mut().unwrap().layout = Some(LayoutOptions {
        placement: Placement::Free,
        width: WidthSizing::Fixed,
        constrain: true,
        padding_mm: 0.0,
    });
    let mut next = shape(0.0, 10.0, 15.0, 20.0, ShapeFill::Ink);
    let object = next.as_object_mut().unwrap();
    object.frame.position = PositionMode::Relative;
    object.frame.gap_mm = mm(5.0);
    if tagged {
        host.as_object_mut().unwrap().sequence = Some(9);
        next.as_object_mut().unwrap().sequence = Some(9);
    }
    let mut doc = document(vec![host, child, next]);
    scene::enable(&mut doc.elements).unwrap();
    let (host_id, child_id, next_id) = (
        id(&doc.elements[0]),
        id(&doc.elements[1]),
        id(&doc.elements[2]),
    );
    doc.elements[1]
        .as_object_mut()
        .unwrap()
        .scene
        .as_mut()
        .unwrap()
        .anchor = Some(Anchor {
        gap_center: None,
        target: host_id,
        horizontal: 1.0,
        vertical: 0.0,
        offset_x_mm: mm(-1.0),
        offset_y_mm: mm(2.0),
    });
    // Paint the icon above its host regardless of its physical list location.
    set_layer(&mut doc.elements[1], 10);
    (doc, host_id, child_id, next_id)
}

#[test]
fn attached_child_cannot_push_flow_and_follows_a_later_or_reordered_host() {
    for tagged in [false, true] {
        let (mut doc, host_id, child_id, next_id) = flow_scene(tagged);
        let before = [render(&doc, 180), render(&doc, 360)];
        for dpi in [180, 360] {
            let mut renderer = TextRenderer::new();
            let rendered =
                render_document(&doc, 76 * u32::from(dpi / 180), dpi, &mut renderer).unwrap();
            let next = rendered
                .bounds
                .iter()
                .find(|bound| bound.element_index == 2)
                .unwrap();
            assert_eq!(next.x, 30 * u32::from(dpi / 180));
        }
        let child = doc.elements.remove(1);
        doc.elements.insert(0, child);
        assert_eq!(id(&doc.elements[0]), child_id);
        assert_eq!(id(&doc.elements[1]), host_id);
        for (dpi, previous) in [180, 360].into_iter().zip(before) {
            assert_eq!(render(&doc, dpi), previous);
        }
        if tagged {
            let from = by_id(&doc, next_id);
            let to = by_id(&doc, host_id);
            ptouch_render::sequence::reorder(&mut doc.elements, from, to, doc.version).unwrap();
            doc.version = 13;
            let host = by_id(&doc, host_id);
            let child = by_id(&doc, child_id);
            let world = scene::world_frames(&doc.elements).unwrap();
            assert!((world[host].x_mm - mm(25.0)).abs() < 1e-10);
            assert!((world[child].x_mm - mm(38.0)).abs() < 1e-10);
            assert!((world[child].y_mm - mm(12.0)).abs() < 1e-10);
            for dpi in [180, 360] {
                let bitmap = render(&doc, dpi);
                assert_dot(&bitmap, dpi, 40, 14, false);
                assert_dot(&bitmap, dpi, 30, 14, true);
            }
        }
    }
}

#[test]
fn v12_scene_and_untouched_v11_documents_round_trip_without_rendering_changes() {
    let mut scene_doc = alpha_scene(png_icon());
    let host_id = id(&scene_doc.elements[0]);
    let icon = scene_doc.elements[2].as_object_mut().unwrap();
    icon.layout = Some(LayoutOptions {
        placement: Placement::Free,
        width: WidthSizing::Fixed,
        constrain: true,
        padding_mm: 0.0,
    });
    icon.scene.as_mut().unwrap().anchor = Some(Anchor {
        gap_center: None,
        target: host_id,
        horizontal: 0.5,
        vertical: 0.5,
        offset_x_mm: mm(-18.0),
        offset_y_mm: mm(-4.0),
    });
    let encoded = scene_doc.to_toml_string().unwrap();
    let reopened = LabelDocument::from_toml_str(&encoded).unwrap();
    assert_eq!(reopened.version, 12);
    assert_eq!(
        reopened.elements[2].as_object().unwrap().scene,
        scene_doc.elements[2].as_object().unwrap().scene
    );
    for dpi in [180, 360] {
        assert_eq!(render(&scene_doc, dpi), render(&reopened, dpi));
    }

    let mut old = document(vec![
        shape(5.0, 10.0, 20.0, 3.0, ShapeFill::Ink),
        shape(0.0, 10.0, 15.0, 3.0, ShapeFill::Ink),
    ]);
    old.version = 11;
    for element in &mut old.elements {
        let object = element.as_object_mut().unwrap();
        object.content = ObjectContent::Line {
            pattern: StrokePattern::default(),
        };
        object.sequence = Some(3);
        assert_eq!(element.minimum_version(), 11);
    }
    old.elements[1].as_object_mut().unwrap().frame.position = PositionMode::Relative;
    old.elements[1].as_object_mut().unwrap().frame.gap_mm = mm(5.0);
    let encoded = old.to_toml_string().unwrap();
    let reopened = LabelDocument::from_toml_str(&encoded).unwrap();
    assert_eq!(reopened, old);
    assert_eq!(reopened.version, 11);
    assert!(!encoded.contains("scene"));
    for dpi in [180, 360] {
        assert_eq!(render(&old, dpi), render(&reopened, dpi));
    }
}

#[test]
fn whole_label_mirroring_is_the_exact_final_pixel_transform() {
    let mut doc = alpha_scene(svg_icon());
    let icon = doc.elements[2].as_object_mut().unwrap();
    icon.rotation = 37.0;
    icon.flip_h = true;
    let mut mask = shape(65.0, 20.0, 15.0, 10.0, ShapeFill::Paper);
    let object = mask.as_object_mut().unwrap();
    object.rotation = 45.0;
    if let ObjectContent::Shape { style } = &mut object.content {
        style.kind = ShapeKind::Ellipse;
    }
    doc.elements.push(mask);
    scene::enable(&mut doc.elements).unwrap();
    for dpi in [180, 360] {
        let baseline = render(&doc, dpi);
        for (flip_h, flip_v) in [(false, false), (true, false), (false, true), (true, true)] {
            let mut mirrored = doc.clone();
            mirrored.flip_h = flip_h;
            mirrored.flip_v = flip_v;
            assert_eq!(render(&mirrored, dpi), baseline.mirrored(flip_h, flip_v));
        }
    }
}

#[test]
fn legacy_zero_width_padding_keeps_pixels_and_version_when_opened_and_saved() {
    let mut old = document(vec![
        LabelElement::CutMark,
        LabelElement::Padding { pixels: 0 },
        LabelElement::CutMark,
    ]);
    old.version = 1;
    let mut without_padding = old.clone();
    without_padding.elements.remove(1);
    let encoded = old.to_toml_string().unwrap();
    let reopened = LabelDocument::from_toml_str(&encoded).unwrap();
    assert_eq!(reopened.version, 1);
    assert!(!encoded.contains("scene"));
    for dpi in [180, 360] {
        let bitmap = render(&reopened, dpi);
        assert_eq!(bitmap.width(), 18);
        assert_eq!(bitmap, render(&without_padding, dpi));
    }
}

#[test]
fn legacy_empty_padding_compatibility_does_not_allow_zero_width_shapes() {
    let invalid = document(vec![shape(0.0, 0.0, 0.0, 10.0, ShapeFill::Paper)]);
    RENDERER.with(|renderer| {
        assert!(render_document(&invalid, 76, 180, &mut renderer.borrow_mut()).is_err());
    });
}

#[test]
fn a_rotated_single_dot_paper_outline_keeps_its_positive_ink_and_coverage() {
    for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse] {
        for angle in [0.0, 1.0, 17.0, 45.0, 89.0, 90.0] {
            for (flip_h, flip_v) in [(false, false), (true, false), (false, true)] {
                let mut tiny = shape(20.0, 20.0, 30.0, 30.0, ShapeFill::Paper);
                let object = tiny.as_object_mut().unwrap();
                object.rotation = angle;
                object.flip_h = flip_h;
                object.flip_v = flip_v;
                let content = object.content_box.as_mut().unwrap();
                content.width_mm = mm(1.0);
                content.height_mm = mm(1.0);
                if let ObjectContent::Shape { style } = &mut object.content {
                    style.kind = kind;
                    style.stroke_width_mm = 0.01;
                }
                for dpi in [180, 360] {
                    let bitmap = render(&document(vec![tiny.clone()]), dpi);
                    assert!(
                        bitmap.data().iter().any(|byte| *byte != 0),
                        "{kind:?}, {angle} degrees, flips {flip_h}/{flip_v}, {dpi} DPI"
                    );
                }
            }
        }
    }
}

#[test]
fn a_rotated_single_dot_paper_fill_covers_at_least_one_background_pixel() {
    for kind in [ShapeKind::Rectangle, ShapeKind::Ellipse] {
        for angle in [0.0, 1.0, 17.0, 45.0, 89.0, 90.0] {
            let mut tiny = shape(20.0, 20.0, 30.0, 30.0, ShapeFill::Paper);
            let object = tiny.as_object_mut().unwrap();
            object.rotation = angle;
            let content = object.content_box.as_mut().unwrap();
            content.width_mm = mm(1.0);
            content.height_mm = mm(1.0);
            if let ObjectContent::Shape { style } = &mut object.content {
                style.kind = kind;
            }
            let doc = document(vec![shape(0.0, 0.0, 100.0, 60.0, ShapeFill::Ink), tiny]);
            for dpi in [180, 360] {
                let bitmap = render(&doc, dpi);
                let scale = u32::from(dpi / 180);
                assert!(
                    (20 * scale..50 * scale)
                        .any(|y| { (20 * scale..50 * scale).any(|x| !bitmap.get_pixel(x, y)) }),
                    "paper coverage vanished: {kind:?}, {angle} degrees, {dpi} DPI"
                );
            }
        }
    }
}
