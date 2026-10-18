// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Explicit object allocation policies, shared by GUI, templates and CLI.

use serde::{Deserialize, Serialize};

use crate::{
    RenderError, Result,
    document::{LabelDocument, LabelElement, LabelObject, ObjectContent},
    text::TextRenderer,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WidthSizing {
    Fixed,
    Content,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    Free,
    Flow,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayoutOptions {
    pub placement: Placement,
    pub width: WidthSizing,
    pub constrain: bool,
    pub padding_mm: f64,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            placement: Placement::Free,
            width: WidthSizing::Content,
            constrain: true,
            padding_mm: 0.0,
        }
    }
}

pub fn validate(object: &LabelObject, version: u32) -> Result<()> {
    let Some(policy) = &object.layout else {
        return Ok(());
    };
    if version < 7 {
        return Err(RenderError::Layout(
            "object layout policies require layout version 7".into(),
        ));
    }
    if !policy.padding_mm.is_finite() || !(0.0..=1000.0).contains(&policy.padding_mm) {
        return Err(RenderError::Layout(
            "layout padding must be in 0..=1000 mm".into(),
        ));
    }
    if policy.placement == Placement::Free && policy.padding_mm != 0.0 {
        return Err(RenderError::Layout(
            "free objects have no allocation padding; use a flow allocation for inner margins"
                .into(),
        ));
    }
    if policy.placement == Placement::Free
        && object.frame.position != crate::layout::PositionMode::Absolute
    {
        return Err(RenderError::Layout(
            "free objects require absolute positioning".into(),
        ));
    }
    let content = object.content_box.as_ref().ok_or_else(|| {
        RenderError::Layout("object layout policies require independent content geometry".into())
    })?;
    content.validate()?;
    if policy.placement == Placement::Free
        && (content.offset_x_mm != 0.0 || content.offset_y_mm != 0.0)
    {
        return Err(RenderError::Layout(
            "free objects use frame X/Y; content offsets require a flow allocation".into(),
        ));
    }
    Ok(())
}

fn check_allocation(object: &LabelObject) -> Result<()> {
    let Some(policy) = &object.layout else {
        return Ok(());
    };
    if policy.placement != Placement::Flow || !policy.constrain {
        return Ok(());
    }
    let (x, y, w, h) = content_extent(object).unwrap();
    let padding = policy.padding_mm;
    if x < -object.frame.width_mm / 2.0 + padding - 1e-7
        || y < -object.frame.height_mm / 2.0 + padding - 1e-7
        || x + w > object.frame.width_mm / 2.0 - padding + 1e-7
        || y + h > object.frame.height_mm / 2.0 - padding + 1e-7
    {
        return Err(RenderError::Layout("content exceeds its allocation; fit content, enlarge the allocation or disable containment".into()));
    }
    Ok(())
}

/// Resolve at the design DPI so changing output resolution cannot alter the
/// physical spacing of a sequential template. Rotation never sets allocation
/// width. Old files have no policy and take precisely their original path.
pub fn resolve(doc: &mut LabelDocument, renderer: &mut TextRenderer) -> Result<()> {
    if doc.dpi == 0 {
        return Err(RenderError::Layout("design DPI must be positive".into()));
    }
    for object in doc
        .elements
        .iter_mut()
        .filter_map(LabelElement::as_object_mut)
    {
        validate(object, doc.version)?;
        object.validate_features(doc.version)?;
        let Some(policy) = &object.layout else {
            continue;
        };
        if policy.width != WidthSizing::Content {
            update_allocation(object);
            check_allocation(object)?;
            continue;
        }
        let content = object.content_box.as_mut().ok_or_else(|| {
            RenderError::Layout("content sizing requires independent content geometry".into())
        })?;
        content.validate()?;
        if let ObjectContent::Text {
            content: text,
            font_size,
            align,
            line_spacing,
            styles,
        } = &object.content
            && !text.trim().is_empty()
        {
            let ppm = f64::from(doc.dpi) / 25.4;
            let inset = object
                .border
                .as_ref()
                .map(|b| b.inset_pixels(doc.dpi))
                .transpose()?
                .unwrap_or(0);
            let target_h = content.height_mm * ppm - 2.0 * f64::from(inset);
            if font_size.is_none() && target_h < 1.0 {
                return Err(RenderError::Layout("border and spacing leave no room for text; enlarge the object or reduce the border".into()));
            }
            let normalized: Vec<_> = if font_size.is_none() && !styles.is_empty() {
                crate::styled_text::style_regions(text, styles)
                    .into_iter()
                    .map(|(range, mut style)| {
                        style.font_size = Some(style.font_size.unwrap_or(12.0));
                        crate::styled_text::StyleSpan {
                            start: range.start,
                            end: range.end,
                            style,
                        }
                    })
                    .collect()
            } else {
                Vec::new()
            };
            let base_size = if normalized.is_empty() {
                font_size.unwrap_or(12.0)
            } else {
                normalized
                    .iter()
                    .filter_map(|s| s.style.font_size)
                    .fold(0.1_f32, f32::max)
            };
            let render_styles = if normalized.is_empty() {
                styles
            } else {
                &normalized
            };
            let mut render = |size| {
                renderer.render_unbounded_styled(
                    text,
                    &doc.font_name,
                    size,
                    *align,
                    *line_spacing,
                    render_styles,
                    base_size,
                )
            };
            let ink = if let Some(size) = font_size {
                render(*size * f32::from(doc.dpi) / 72.0)?
            } else {
                let (mut lo, mut hi) = (1.0_f32, (target_h as f32 * 4.0).clamp(4.0, 4096.0));
                let mut best = render(lo)?;
                while hi < 4096.0 {
                    match render(hi) {
                        Ok(candidate) if f64::from(candidate.height()) <= target_h => {
                            lo = hi;
                            best = candidate;
                            hi = (hi * 2.0).min(4096.0);
                        }
                        _ => break,
                    }
                }
                for _ in 0..16 {
                    let mid = (lo + hi) / 2.0;
                    match render(mid) {
                        Ok(candidate) if f64::from(candidate.height()) <= target_h => {
                            lo = mid;
                            best = candidate;
                        }
                        Ok(_) | Err(RenderError::DimensionMismatch(_)) => hi = mid,
                        Err(e) => return Err(e),
                    }
                }
                best
            };
            // One design dot avoids hinting/edge quantization clipping at
            // the next output DPI, while keeping the edit box near ink.
            content.width_mm = f64::from(ink.width() + 1 + 2 * inset) / ppm;
            if let Some(size) = font_size {
                content.height_mm = f64::from(ink.height() + 1 + 2 * inset) / ppm;
                if let Some(border) = &object.border {
                    // Border and glyph rounding differ between printer grids.
                    // Reserve the largest physical extent at the supported
                    // resolutions, independently of the connected printer.
                    for dpi in [180, 360] {
                        if dpi == doc.dpi {
                            continue;
                        }
                        let measured = render(*size * f32::from(dpi) / 72.0)?;
                        let inset = border.inset_pixels(dpi)?;
                        let ppm = f64::from(dpi) / 25.4;
                        content.width_mm = content
                            .width_mm
                            .max(f64::from(measured.width() + 1 + 2 * inset) / ppm);
                        content.height_mm = content
                            .height_mm
                            .max(f64::from(measured.height() + 1 + 2 * inset) / ppm);
                    }
                }
            }
        }
        update_allocation(object);
        check_allocation(object)?;
    }
    Ok(())
}

/// Establish a provisional geometric frame without loading a font system.
/// The shared render pass subsequently measures any content-sized text.
pub fn update_allocation(object: &mut LabelObject) {
    let Some(policy) = &object.layout else { return };
    let Some(content) = &object.content_box else {
        return;
    };
    if policy.placement == Placement::Free {
        let (sin, cos) = f64::from(object.rotation).to_radians().sin_cos();
        object.frame.width_mm =
            content.width_mm * cos.abs() + content.height_mm * sin.abs() + 2.0 * policy.padding_mm;
        object.frame.height_mm =
            content.width_mm * sin.abs() + content.height_mm * cos.abs() + 2.0 * policy.padding_mm;
    } else if policy.width == WidthSizing::Content {
        object.frame.width_mm = content.width_mm + 2.0 * policy.padding_mm;
    }
}

/// Axis-aligned extent of the rotated content relative to the allocation
/// centre. Reflection changes the centre offset, not its half extents.
pub fn content_extent(object: &LabelObject) -> Option<(f64, f64, f64, f64)> {
    let c = object.content_box.as_ref()?;
    let (sin, cos) = f64::from(object.rotation).to_radians().sin_cos();
    let w = c.width_mm * cos.abs() + c.height_mm * sin.abs();
    let h = c.width_mm * sin.abs() + c.height_mm * cos.abs();
    let x = c.offset_x_mm * if object.flip_h { -1.0 } else { 1.0 };
    let y = c.offset_y_mm * if object.flip_v { -1.0 } else { 1.0 };
    Some((x - w / 2.0, y - h / 2.0, w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bordered_content_width_and_fixed_fonts_converge_without_growing_each_pass() {
        for size in [None, Some(6.0)] {
            let mut doc = document();
            doc.version = 10;
            doc.apply_values(&std::collections::BTreeMap::from([(
                "port".into(),
                "端口01".into(),
            )]));
            let object = doc.elements[0].as_object_mut().unwrap();
            object.border = Some(Default::default());
            if let ObjectContent::Text { font_size, .. } = &mut object.content {
                *font_size = size;
            }
            let mut renderer = TextRenderer::with_test_font();
            resolve(&mut doc, &mut renderer).unwrap();
            let settled = doc.clone();
            for _ in 0..8 {
                resolve(&mut doc, &mut renderer).unwrap();
                assert_eq!(doc, settled);
                let saved = doc.to_toml_string().unwrap();
                doc = LabelDocument::from_toml_str(&saved).unwrap();
            }
            let object = doc.elements[0].as_object().unwrap();
            let inset = object.border.as_ref().unwrap().inset_mm(doc.dpi).unwrap();
            let box_ = object.content_box.as_ref().unwrap();
            assert!(box_.width_mm > 2.0 * inset && box_.height_mm > 2.0 * inset);
        }
    }

    #[test]
    fn attached_border_does_not_move_fixed_allocations_or_following_objects() {
        let mut doc = document();
        doc.version = 10;
        let object = doc.elements[0].as_object_mut().unwrap();
        object.layout.as_mut().unwrap().width = WidthSizing::Fixed;
        object.frame.width_mm = 24.0;
        object.content_box.as_mut().unwrap().width_mm = 22.0;
        object.border = Some(Default::default());
        for element in &mut doc.elements {
            if let ObjectContent::Text { font_size, .. } =
                &mut element.as_object_mut().unwrap().content
            {
                *font_size = Some(3.0);
            }
        }
        let before = doc
            .elements
            .iter()
            .map(|e| e.as_object().unwrap().frame.clone())
            .collect::<Vec<_>>();
        let mut renderer = TextRenderer::with_test_font();
        for value in ["A", "PORT 012345", "端口 02"] {
            let mut sample = doc.clone();
            sample.apply_values(&std::collections::BTreeMap::from([(
                "port".into(),
                value.into(),
            )]));
            resolve(&mut sample, &mut renderer).unwrap();
            assert_eq!(
                sample
                    .elements
                    .iter()
                    .map(|e| e.as_object().unwrap().frame.clone())
                    .collect::<Vec<_>>(),
                before
            );
            for (dpi, tape) in [(180, 76), (360, 152)] {
                let output =
                    crate::layout::render_document(&sample, tape, dpi, &mut renderer).unwrap();
                assert!(
                    output.diagnostics.is_empty(),
                    "{value} at {dpi}: {:?}",
                    output.diagnostics
                );
            }
        }
    }

    #[test]
    fn content_sized_borders_do_not_clip_fixed_fonts_when_output_dpi_changes() {
        let mut renderer = TextRenderer::with_test_font();
        for design in [180, 360] {
            for stroke in [0.1, 0.2, 0.3] {
                for padding in [0.0, 0.1, 0.2, 0.5] {
                    let mut doc = document();
                    doc.version = 10;
                    doc.dpi = design;
                    doc.apply_values(&std::collections::BTreeMap::from([(
                        "port".into(),
                        "端口01".into(),
                    )]));
                    let object = doc.elements[0].as_object_mut().unwrap();
                    object.border = Some(crate::stroke::BorderStyle {
                        width_mm: stroke,
                        padding_mm: padding,
                        ..Default::default()
                    });
                    if let ObjectContent::Text { font_size, .. } = &mut object.content {
                        *font_size = Some(6.0);
                    }
                    resolve(&mut doc, &mut renderer).unwrap();
                    for (dpi, tape) in [(180, 76), (360, 152)] {
                        let rendered =
                            crate::layout::render_document(&doc, tape, dpi, &mut renderer).unwrap();
                        assert!(
                            rendered.diagnostics.is_empty(),
                            "design {design}, output {dpi}, stroke {stroke}, spacing {padding}: {:?}",
                            rendered.diagnostics
                        );
                    }
                }
            }
        }
    }
    use crate::document::{ContentBox, PositionMode, TextBox};

    fn document() -> LabelDocument {
        let mut object = LabelObject::text(
            "{{port}}",
            TextBox {
                height_mm: 8.0,
                ..Default::default()
            },
        );
        object.content_box = Some(ContentBox {
            width_mm: 10.0,
            height_mm: 6.0,
            offset_x_mm: 0.0,
            offset_y_mm: 0.0,
        });
        object.layout = Some(LayoutOptions {
            placement: Placement::Flow,
            constrain: false,
            ..Default::default()
        });
        let next = LabelObject::text(
            "B",
            TextBox {
                position: PositionMode::Relative,
                gap_mm: 2.0,
                height_mm: 8.0,
                ..Default::default()
            },
        );
        LabelDocument {
            version: 7,
            tape_width_mm: 12,
            dpi: 180,
            font_name: String::new(),
            font_margin: 0,
            flip_h: false,
            flip_v: false,
            elements: vec![LabelElement::Object(object), LabelElement::Object(next)],
        }
    }

    #[test]
    fn content_width_reflows_template_values_but_fixed_allocation_and_rotation_do_not() {
        let base = document();
        let mut renderer = TextRenderer::with_test_font();
        let resolve_value = |text: &str, fixed: bool, angle: f32, renderer: &mut TextRenderer| {
            let mut doc = base.clone();
            doc.apply_values(&std::collections::BTreeMap::from([(
                "port".into(),
                text.into(),
            )]));
            let object = doc.elements[0].as_object_mut().unwrap();
            object.rotation = angle;
            if fixed {
                object.layout.as_mut().unwrap().width = WidthSizing::Fixed;
            }
            resolve(&mut doc, renderer).unwrap();
            doc.elements[0].as_object().unwrap().frame.width_mm
        };
        let short = resolve_value("A", false, 0.0, &mut renderer);
        assert!(resolve_value("PORT 012345", false, 0.0, &mut renderer) > short * 2.0);
        assert_eq!(resolve_value("A", false, 90.0, &mut renderer), short);
        assert_eq!(
            resolve_value("PORT 012345", true, 90.0, &mut renderer),
            10.0
        );
    }

    #[test]
    fn old_documents_are_not_normalized_and_new_policy_is_version_gated() {
        let mut doc = document();
        doc.elements[0].as_object_mut().unwrap().layout = None;
        doc.version = 4;
        let before = doc.clone();
        resolve(&mut doc, &mut TextRenderer::with_test_font()).unwrap();
        assert_eq!(doc, before);
        doc.elements[0].as_object_mut().unwrap().layout = Some(LayoutOptions::default());
        let encoded = doc.to_toml_string().unwrap();
        assert!(encoded.contains("version = 7"));
        assert!(
            LabelDocument::from_toml_str(&encoded.replace("version = 7", "version = 6")).is_err()
        );
        let decoded = LabelDocument::from_toml_str(&encoded).unwrap();
        assert_eq!(decoded.version, 7);
        assert_eq!(
            decoded.elements[0].as_object().unwrap().layout,
            Some(LayoutOptions::default())
        );
    }

    #[test]
    fn explicit_containment_is_enforced_by_the_shared_renderer_policy() {
        let mut doc = document();
        let object = doc.elements[0].as_object_mut().unwrap();
        object.layout.as_mut().unwrap().width = WidthSizing::Fixed;
        object.layout.as_mut().unwrap().constrain = true;
        object.content_box.as_mut().unwrap().offset_x_mm = 20.0;
        assert!(resolve(&mut doc, &mut TextRenderer::with_test_font()).is_err());
        doc.elements[0]
            .as_object_mut()
            .unwrap()
            .layout
            .as_mut()
            .unwrap()
            .constrain = false;
        resolve(&mut doc, &mut TextRenderer::with_test_font()).unwrap();
    }

    #[test]
    fn rich_auto_width_uses_proportions_independent_of_uniform_nominal_sizes() {
        let mut renderer = TextRenderer::with_test_font();
        let mut widths = Vec::new();
        for size in [0.1, 12.0, 512.0] {
            let mut doc = document();
            let ObjectContent::Text {
                content, styles, ..
            } = &mut doc.elements[0].as_object_mut().unwrap().content
            else {
                unreachable!()
            };
            *content = "PORT 01".into();
            *styles = vec![crate::styled_text::StyleSpan {
                start: 0,
                end: 7,
                style: crate::styled_text::TextStyle {
                    font_size: Some(size),
                    ..Default::default()
                },
            }];
            resolve(&mut doc, &mut renderer).unwrap();
            widths.push(doc.elements[0].as_object().unwrap().frame.width_mm);
        }
        assert!(
            widths.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-9),
            "{widths:?}"
        );
    }
}
