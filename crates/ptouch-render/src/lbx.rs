// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Bounded importer for supported Brother LBX label objects.

use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use zip::ZipArchive;

use crate::document::{LabelDocument, LabelElement, LabelObject, ObjectContent};
use crate::layout::{PositionMode, TextBox, VerticalAlign};
use crate::styled_text::{StyleSpan, TextStyle};
use crate::text::TextAlign;
use crate::{RenderError, Result};

const PT_TO_MM: f64 = 25.4 / 72.0;
pub const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const MAX_XML_BYTES: u64 = 8 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// An imported LBX document and any structures that were skipped.
#[derive(Debug)]
pub struct LbxImport {
    pub document: LabelDocument,
    pub warnings: Vec<String>,
}

/// Import supported text, PT Dingbats symbols, images, and vector objects from LBX.
///
/// Specialized cable, table, barcode, drawing, trimmed image, and image-effect
/// objects are skipped with a warning. A file with no supported objects is
/// rejected to avoid silently opening a blank label.
pub fn import(bytes: &[u8]) -> Result<LbxImport> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(RenderError::Layout(
            "LBX archive exceeds the 64 MiB import limit".into(),
        ));
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| RenderError::Layout(format!("invalid LBX archive: {error}")))?;
    let mut xml = String::new();
    {
        let entry = archive.by_name("label.xml").map_err(|error| {
            RenderError::Layout(format!("LBX archive has no label.xml: {error}"))
        })?;
        if entry.size() > MAX_XML_BYTES {
            return Err(RenderError::Layout(format!(
                "LBX label.xml exceeds the {} MiB import limit",
                MAX_XML_BYTES / (1024 * 1024)
            )));
        }
        entry.take(MAX_XML_BYTES + 1).read_to_string(&mut xml)?;
    }
    if xml.len() as u64 > MAX_XML_BYTES {
        return Err(RenderError::Layout(
            "LBX label.xml exceeds the 8 MiB import limit".into(),
        ));
    }
    let parsed = roxmltree::Document::parse(&xml)
        .map_err(|error| RenderError::Layout(format!("invalid LBX label.xml: {error}")))?;
    let paper = parsed
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "paper")
        .ok_or_else(|| RenderError::Layout("LBX label.xml has no paper settings".into()))?;
    let paper_width = point_attr(paper, "width")?;
    let tape_width_mm = (paper_width * PT_TO_MM).round().clamp(1.0, 255.0) as u8;
    let orientation = paper.attribute("orientation").unwrap_or("portrait");
    let margin_left = optional_point_attr(paper, "marginLeft")?.unwrap_or(0.0);
    let margin_top = optional_point_attr(paper, "marginTop")?.unwrap_or(0.0);
    let page = match orientation.to_ascii_lowercase().as_str() {
        "portrait" => LbxPage {
            offset_x_pt: margin_left,
            offset_y_pt: margin_top,
        },
        "landscape" => LbxPage {
            offset_x_pt: margin_top,
            offset_y_pt: margin_left,
        },
        _ => {
            return Err(RenderError::Layout(format!(
                "LBX paper orientation '{orientation}' is unsupported"
            )));
        }
    };
    let objects = parsed
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "objects")
        .ok_or_else(|| RenderError::Layout("LBX label.xml has no object list".into()))?;

    let mut elements = Vec::new();
    let mut warnings = Vec::new();
    let mut font_name: Option<String> = None;
    for node in objects.children().filter(|node| node.is_element()) {
        import_container(
            node,
            true,
            page,
            &mut archive,
            &mut elements,
            &mut warnings,
            &mut font_name,
        )?;
    }
    if elements.is_empty() {
        let detail = if warnings.is_empty() {
            String::new()
        } else {
            format!("; {}", warnings.join("; "))
        };
        return Err(RenderError::Layout(format!(
            "LBX file contains no supported text, PT Dingbats symbols, untrimmed images, or unfilled vector objects{detail}"
        )));
    }
    Ok(LbxImport {
        document: LabelDocument {
            version: crate::document::DOCUMENT_VERSION,
            tape_width_mm,
            dpi: 180,
            font_name: font_name.unwrap_or_else(|| "Arial".into()),
            font_margin: 0,
            flip_h: false,
            flip_v: false,
            elements,
        },
        warnings,
    })
}

fn import_text(
    node: roxmltree::Node<'_, '_>,
    page: LbxPage,
    elements: &mut Vec<LabelElement>,
    warnings: &mut Vec<String>,
    font_name: &mut Option<String>,
) -> Result<()> {
    let style = object_style(node, "text")?;
    let frame = frame_from_style(style, page)?;
    let content = node
        .children()
        .find(|child| child.is_element() && child.tag_name().name() == "data")
        .and_then(|data| data.text())
        .unwrap_or_default()
        .to_owned();
    let rotation = angle_from_style(style)?;
    let align = node
        .descendants()
        .find(|child| child.is_element() && child.tag_name().name() == "textAlign")
        .and_then(|child| child.attribute("horizontalAlignment"))
        .map(|value| match value {
            "CENTER" => TextAlign::Center,
            "RIGHT" => TextAlign::Right,
            _ => TextAlign::Left,
        })
        .unwrap_or(TextAlign::Left);
    let font_size = node
        .descendants()
        .find(|child| child.is_element() && child.tag_name().name() == "textStyle")
        .and_then(|child| child.attribute("orgPoint"))
        .and_then(parse_points)
        .map(|value| value as f32);
    if let Some(name) = node
        .descendants()
        .find(|child| child.is_element() && child.tag_name().name() == "logFont")
        .and_then(|child| child.attribute("name"))
    {
        match font_name {
            Some(existing) if existing != name => {
                warnings.push(format!(
                    "LBX uses multiple font families; '{name}' falls back to '{existing}'"
                ));
            }
            None => *font_name = Some(name.to_owned()),
            _ => {}
        }
    }
    elements.push(LabelElement::Object(LabelObject {
        frame,
        content_box: None,
        rotation,
        flip_h: false,
        flip_v: false,
        content: ObjectContent::Text {
            content,
            font_size,
            align,
            line_spacing: crate::text::DEFAULT_LINE_SPACING,
            styles: Vec::new(),
        },
    }));
    Ok(())
}

fn import_symbol(
    node: roxmltree::Node<'_, '_>,
    page: LbxPage,
    elements: &mut Vec<LabelElement>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let style = object_style(node, "symbol")?;
    let symbol_style = node
        .descendants()
        .find(|child| child.is_element() && child.tag_name().name() == "symbolStyle")
        .ok_or_else(|| RenderError::Layout("LBX symbol has no symbolStyle".into()))?;
    let font_name = symbol_style
        .attribute("fontName")
        .ok_or_else(|| RenderError::Layout("LBX symbolStyle has no fontName".into()))?;
    let code = symbol_style
        .attribute("code")
        .ok_or_else(|| RenderError::Layout("LBX symbolStyle has no code".into()))?
        .parse::<u32>()
        .map_err(|_| RenderError::Layout("LBX symbol code is not an integer".into()))?;
    if !font_name.contains("PT Dingbats")
        || (!(33..=126).contains(&code) && !(161..=233).contains(&code))
    {
        warnings.push(format!(
            "skipped LBX symbol with unsupported font '{font_name}' or code {code}"
        ));
        return Ok(());
    }
    let glyph = char::from_u32(0xF000 + code)
        .ok_or_else(|| RenderError::Layout("LBX symbol code is not a Unicode scalar".into()))?;
    let object = LabelObject {
        frame: frame_from_style(style, page)?,
        content_box: None,
        rotation: angle_from_style(style)?,
        flip_h: false,
        flip_v: false,
        content: ObjectContent::Text {
            content: glyph.to_string(),
            font_size: None,
            align: TextAlign::Center,
            line_spacing: crate::text::DEFAULT_LINE_SPACING,
            styles: vec![StyleSpan {
                start: 0,
                end: 1,
                style: TextStyle {
                    font_name: Some(font_name.to_owned()),
                    ..TextStyle::default()
                },
            }],
        },
    };
    elements.push(LabelElement::Object(object));
    warnings.push(format!(
        "converted LBX symbol '{font_name}' code {code} to editable text; native symbol metadata is not retained"
    ));
    if !crate::font::has_glyph(font_name, glyph) {
        warnings.push(format!(
            "LBX symbol glyph U+{0:04X} is unavailable in installed font family '{font_name}'; rendering may substitute another font",
            glyph as u32
        ));
    }
    Ok(())
}

enum ImageImport {
    Imported(LabelObject),
    Skipped(String),
}

#[derive(Clone, Copy)]
struct LbxPage {
    offset_x_pt: f64,
    offset_y_pt: f64,
}

fn import_container(
    node: roxmltree::Node<'_, '_>,
    top_level: bool,
    page: LbxPage,
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    elements: &mut Vec<LabelElement>,
    warnings: &mut Vec<String>,
    font_name: &mut Option<String>,
) -> Result<()> {
    let kind = node.tag_name().name();
    match kind {
        "text" => import_text(node, page, elements, warnings, font_name)?,
        "symbol" => import_symbol(node, page, elements, warnings)?,
        "image" => match import_image(archive, node, page)? {
            ImageImport::Imported(object) => elements.push(LabelElement::Object(object)),
            ImageImport::Skipped(reason) => warnings.push(reason),
        },
        "poly" | "rect" => match import_shape(node, kind, page)? {
            ImageImport::Imported(object) => {
                elements.push(LabelElement::Object(object));
                warnings.push(format!(
                    "converted LBX {kind} into an embedded SVG image; native shape editing is not retained"
                ));
            }
            ImageImport::Skipped(reason) => warnings.push(reason),
        },
        _ => {
            let nested = node
                .children()
                .filter(|child| child.is_element())
                .collect::<Vec<_>>();
            let contains_supported = nested.iter().any(|child| {
                matches!(
                    child.tag_name().name(),
                    "text" | "symbol" | "image" | "poly" | "rect"
                ) || child.descendants().any(|descendant| {
                    descendant.is_element()
                        && matches!(
                            descendant.tag_name().name(),
                            "text" | "symbol" | "image" | "poly" | "rect"
                        )
                })
            });
            if contains_supported {
                if top_level {
                    warnings.push(format!(
                        "flattened specialized LBX object '{kind}' into child objects; its wrapper behavior is not retained"
                    ));
                }
                for child in nested {
                    import_container(child, false, page, archive, elements, warnings, font_name)?;
                }
            } else if is_unsupported_object(kind) || top_level {
                warnings.push(format!("skipped unsupported LBX object '{kind}'"));
            }
        }
    }
    Ok(())
}

fn is_unsupported_object(kind: &str) -> bool {
    matches!(
        kind,
        "barcode" | "datetime" | "dateTime" | "frame" | "symbol" | "group"
    )
}

fn import_image(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    node: roxmltree::Node<'_, '_>,
    page: LbxPage,
) -> Result<ImageImport> {
    if node.descendants().any(|child| {
        child.is_element()
            && child.tag_name().name() == "trimming"
            && child.attribute("flag") == Some("true")
    }) {
        return Ok(ImageImport::Skipped(
            "skipped trimmed LBX image; crop geometry is not represented by the Rust image object"
                .into(),
        ));
    }
    if node.descendants().any(|child| {
        if !child.is_element() || child.tag_name().name() != "effect" {
            return false;
        }
        child
            .attribute("effect")
            .is_some_and(|value| value != "NONE")
            || child
                .attribute("brightness")
                .is_some_and(|value| value != "50")
            || child
                .attribute("contrast")
                .is_some_and(|value| value != "50")
    }) || node.descendants().any(|child| {
        child.is_element()
            && child.tag_name().name() == "transparent"
            && child.attribute("flag") == Some("true")
    }) {
        return Ok(ImageImport::Skipped(
            "skipped LBX image with transparency or non-default image effects that the Rust image object cannot preserve".into(),
        ));
    }
    let style = node
        .descendants()
        .find(|child| child.is_element() && child.tag_name().name() == "imageStyle")
        .ok_or_else(|| RenderError::Layout("LBX image object has no imageStyle".into()))?;
    let file_name = style
        .attribute("fileName")
        .ok_or_else(|| RenderError::Layout("LBX imageStyle has no fileName".into()))?;
    let resource_path = safe_resource_path(file_name).ok_or_else(|| {
        RenderError::Layout(format!("LBX image has unsafe archive path '{file_name}'"))
    })?;
    let entry = match archive.by_name(file_name) {
        Ok(entry) => entry,
        Err(_) => {
            return Ok(ImageImport::Skipped(format!(
                "skipped LBX image '{file_name}'; its embedded archive resource is missing"
            )));
        }
    };
    if entry.size() > MAX_IMAGE_BYTES {
        return Ok(ImageImport::Skipped(format!(
            "skipped LBX image '{file_name}'; its resource exceeds the 32 MiB import limit"
        )));
    }
    let mut image_data = Vec::with_capacity(entry.size() as usize);
    entry
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut image_data)?;
    if image_data.len() as u64 > MAX_IMAGE_BYTES {
        return Ok(ImageImport::Skipped(format!(
            "skipped LBX image '{file_name}'; its resource exceeds the 32 MiB import limit"
        )));
    }
    let frame = frame_from_style(object_style(node, "image")?, page)?;
    let rotation = angle_from_style(object_style(node, "image")?)?;
    let mut object =
        LabelObject::image_from_bytes(Some(PathBuf::from(resource_path)), image_data, frame);
    object.rotation = rotation;
    if matches!(&object.content, ObjectContent::Image { bitmap: None, .. }) {
        return Err(RenderError::Layout(format!(
            "LBX image '{file_name}' has an unsupported image format"
        )));
    }
    Ok(ImageImport::Imported(object))
}

fn import_shape(node: roxmltree::Node<'_, '_>, kind: &str, page: LbxPage) -> Result<ImageImport> {
    let style = object_style(node, kind)?;
    let frame = frame_from_style(style, page)?;
    let raw_width = point_attr_raw(style, "width")?;
    let raw_height = point_attr_raw(style, "height")?;
    let raw_x = point_attr_raw(style, "x")?;
    let raw_y = point_attr_raw(style, "y")?;
    let Some((stroke, stroke_width, dash)) = svg_pen(style) else {
        return Ok(ImageImport::Skipped(format!(
            "skipped LBX {kind}; its pen style or width is unsupported"
        )));
    };
    let brush = style
        .children()
        .find(|child| child.is_element() && child.tag_name().name() == "brush")
        .and_then(|child| child.attribute("style"))
        .unwrap_or("NULL");
    if brush != "NULL" {
        return Ok(ImageImport::Skipped(format!(
            "skipped filled LBX {kind}; fill patterns are not preserved"
        )));
    }

    let primitive = match kind {
        "poly" => {
            let poly_style = node
                .descendants()
                .find(|child| child.is_element() && child.tag_name().name() == "polyStyle")
                .ok_or_else(|| RenderError::Layout("LBX poly object has no polyStyle".into()))?;
            let shape = poly_style
                .attribute("shape")
                .ok_or_else(|| RenderError::Layout("LBX polyStyle has no shape".into()))?;
            if !matches!(
                poly_style.attribute("arrowBegin").unwrap_or("SQUARE"),
                "SQUARE" | "NONE"
            ) || !matches!(
                poly_style.attribute("arrowEnd").unwrap_or("SQUARE"),
                "SQUARE" | "NONE"
            ) {
                return Ok(ImageImport::Skipped(
                    "skipped LBX poly with arrowheads that are not preserved".into(),
                ));
            }
            let points = poly_style
                .descendants()
                .find(|child| child.is_element() && child.tag_name().name() == "polyLinePoints")
                .and_then(|child| child.attribute("points"))
                .ok_or_else(|| RenderError::Layout("LBX polyStyle has no line points".into()))?;
            let points = parse_poly_points(points, raw_x, raw_y)?;
            let points_attr = points
                .iter()
                .map(|(x, y)| format!("{},{}", svg_num(*x), svg_num(*y)))
                .collect::<Vec<_>>()
                .join(" ");
            match shape.to_ascii_uppercase().as_str() {
                "LINE" if points.len() == 2 => {
                    let (x1, y1) = points[0];
                    let (x2, y2) = points[1];
                    format!(
                        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{}\"{dash}/> ",
                        svg_num(x1),
                        svg_num(y1),
                        svg_num(x2),
                        svg_num(y2),
                        svg_num(stroke_width)
                    )
                }
                "POLYLINE" if points.len() >= 2 => format!(
                    "<polyline points=\"{points_attr}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{}\"{dash}/> ",
                    svg_num(stroke_width)
                ),
                "POLYGON" if points.len() >= 3 => format!(
                    "<polygon points=\"{points_attr}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{}\"{dash}/> ",
                    svg_num(stroke_width)
                ),
                _ => {
                    return Ok(ImageImport::Skipped(format!(
                        "skipped LBX poly with unsupported shape '{shape}' or point count"
                    )));
                }
            }
        }
        "rect" => {
            let rect_style = node
                .descendants()
                .find(|child| child.is_element() && child.tag_name().name() == "rectStyle")
                .ok_or_else(|| RenderError::Layout("LBX rectangle has no rectStyle".into()))?;
            let shape = rect_style
                .attribute("shape")
                .ok_or_else(|| RenderError::Layout("LBX rectStyle has no shape".into()))?;
            let inset = stroke_width / 2.0;
            let width = (raw_width - stroke_width).max(0.0);
            let height = (raw_height - stroke_width).max(0.0);
            let common = format!(
                "fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{}\"{dash}",
                svg_num(stroke_width)
            );
            match shape.to_ascii_uppercase().replace(['_', '-'], "").as_str() {
                "RECTANGLE" => format!(
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" {common}/> ",
                    svg_num(inset),
                    svg_num(inset),
                    svg_num(width),
                    svg_num(height)
                ),
                "ELLIPSE" => format!(
                    "<ellipse cx=\"{}\" cy=\"{}\" rx=\"{}\" ry=\"{}\" {common}/> ",
                    svg_num(raw_width / 2.0),
                    svg_num(raw_height / 2.0),
                    svg_num(width / 2.0),
                    svg_num(height / 2.0)
                ),
                _ => {
                    return Ok(ImageImport::Skipped(format!(
                        "skipped LBX rectangle style '{shape}'"
                    )));
                }
            }
        }
        _ => unreachable!("only poly and rect are imported as vector shapes"),
    };

    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}pt\" height=\"{}pt\" viewBox=\"0 0 {} {}\"><rect width=\"100%\" height=\"100%\" fill=\"white\"/>{primitive}</svg>",
        svg_num(raw_width),
        svg_num(raw_height),
        svg_num(raw_width),
        svg_num(raw_height)
    );
    let mut object = LabelObject::image_from_bytes(None, svg.into_bytes(), frame);
    object.rotation = angle_from_style(style)?;
    if matches!(&object.content, ObjectContent::Image { bitmap: None, .. }) {
        return Err(RenderError::Layout(format!(
            "LBX {kind} could not be rasterized from its vector source"
        )));
    }
    Ok(ImageImport::Imported(object))
}

fn svg_pen(style: roxmltree::Node<'_, '_>) -> Option<(String, f64, String)> {
    let pen = style
        .children()
        .find(|child| child.is_element() && child.tag_name().name() == "pen")?;
    let pen_style = pen.attribute("style").unwrap_or("SOLID");
    let dash = match pen_style {
        "SOLID" => String::new(),
        "DASH" => " stroke-dasharray=\"4 2\"".into(),
        "DOT" => " stroke-dasharray=\"1 2\"".into(),
        _ => return None,
    };
    let color = pen.attribute("color").unwrap_or("#000000");
    if !valid_svg_color(color) {
        return None;
    }
    let width_x = pen
        .attribute("widthX")
        .and_then(parse_points)
        .unwrap_or(0.0);
    let width_y = pen
        .attribute("widthY")
        .and_then(parse_points)
        .unwrap_or(0.0);
    let width = width_x.max(width_y);
    (width > 0.0 && width.is_finite()).then(|| (color.to_owned(), width, dash))
}

fn valid_svg_color(value: &str) -> bool {
    value.len() == 7
        && value.starts_with('#')
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn parse_poly_points(value: &str, origin_x: f64, origin_y: f64) -> Result<Vec<(f64, f64)>> {
    let mut points = Vec::new();
    for pair in value.split_ascii_whitespace() {
        let (x, y) = pair
            .split_once(',')
            .ok_or_else(|| RenderError::Layout("LBX poly point is not a coordinate pair".into()))?;
        let x = parse_points(x)
            .filter(|value| value.is_finite())
            .ok_or_else(|| RenderError::Layout("LBX poly point has invalid x coordinate".into()))?;
        let y = parse_points(y)
            .filter(|value| value.is_finite())
            .ok_or_else(|| RenderError::Layout("LBX poly point has invalid y coordinate".into()))?;
        points.push((x - origin_x, y - origin_y));
    }
    if points.len() > 10_000 {
        return Err(RenderError::Layout(
            "LBX poly exceeds the 10,000 point import limit".into(),
        ));
    }
    Ok(points)
}

fn point_attr_raw(node: roxmltree::Node<'_, '_>, name: &str) -> Result<f64> {
    node.attribute(name)
        .and_then(parse_points)
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= 100_000.0)
        .ok_or_else(|| RenderError::Layout(format!("LBX has invalid {name} point value")))
}

fn svg_num(value: f64) -> String {
    format!("{value:.4}")
}

fn safe_resource_path(file_name: &str) -> Option<&Path> {
    let path = Path::new(file_name);
    if file_name.is_empty() || file_name.contains('\\') || path.is_absolute() {
        return None;
    }
    path.components()
        .all(|component| matches!(component, Component::Normal(_)))
        .then_some(path)
}

fn object_style<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    kind: &str,
) -> Result<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == "objectStyle")
        .ok_or_else(|| RenderError::Layout(format!("LBX {kind} object has no objectStyle")))
}

fn frame_from_style(style: roxmltree::Node<'_, '_>, page: LbxPage) -> Result<TextBox> {
    let width = point_attr(style, "width")? * PT_TO_MM;
    let height = point_attr(style, "height")? * PT_TO_MM;
    if width == 0.0 || height == 0.0 {
        return Err(RenderError::Layout(
            "LBX object has a zero-sized frame".into(),
        ));
    }
    Ok(TextBox {
        width_mm: width,
        height_mm: height,
        position: PositionMode::Absolute,
        x_mm: (point_attr(style, "x")? - page.offset_x_pt) * PT_TO_MM,
        y_mm: (point_attr(style, "y")? - page.offset_y_pt) * PT_TO_MM,
        gap_mm: 0.0,
        vertical_align: VerticalAlign::Center,
    })
}

fn angle_from_style(style: roxmltree::Node<'_, '_>) -> Result<f32> {
    style
        .attribute("angle")
        .unwrap_or("0")
        .parse::<f32>()
        .ok()
        .filter(|value| value.is_finite())
        .ok_or_else(|| RenderError::Layout("LBX object has an invalid angle".into()))
}

fn parse_points(value: &str) -> Option<f64> {
    value.strip_suffix("pt")?.parse().ok()
}

fn point_attr(node: roxmltree::Node<'_, '_>, name: &str) -> Result<f64> {
    node.attribute(name)
        .and_then(parse_points)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| RenderError::Layout(format!("LBX has invalid {name} point value")))
}

fn optional_point_attr(node: roxmltree::Node<'_, '_>, name: &str) -> Result<Option<f64>> {
    node.attribute(name)
        .map(|_| point_attr(node, name))
        .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;

    fn archive(xml: &str) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("label.xml", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(xml.as_bytes()).unwrap();
        writer.finish().unwrap().into_inner()
    }

    fn archive_with_image(xml: &str, image_name: &str, image: &[u8]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file("label.xml", SimpleFileOptions::default())
            .unwrap();
        writer.write_all(xml.as_bytes()).unwrap();
        writer
            .start_file(image_name, SimpleFileOptions::default())
            .unwrap();
        writer.write_all(image).unwrap();
        writer.finish().unwrap().into_inner()
    }

    const PREFIX: &str = r#"<document xmlns:pt="urn:pt" xmlns:style="urn:style" xmlns:text="urn:text" xmlns:image="urn:image" xmlns:draw="urn:draw"><style:paper width="68pt"/><pt:objects>"#;
    const SUFFIX: &str = "</pt:objects></document>";

    #[test]
    fn imports_ordinary_text_geometry_and_alignment() {
        let xml = format!(
            "{PREFIX}<text:text><pt:objectStyle x=\"72pt\" y=\"0pt\" width=\"144pt\" height=\"18pt\" angle=\"90\"/><text:textAlign horizontalAlignment=\"CENTER\"/><text:textStyle orgPoint=\"10pt\"/><pt:data>Hello &amp; world</pt:data></text:text>{SUFFIX}"
        );
        let result = import(&archive(&xml)).unwrap();
        assert_eq!(result.document.tape_width_mm, 24);
        assert!(result.warnings.is_empty());
        let Some(LabelElement::Object(object)) = result.document.elements.first() else {
            panic!("expected object element");
        };
        assert!((object.frame.x_mm - 25.4).abs() < 0.001);
        assert!((object.frame.width_mm - 50.8).abs() < 0.001);
        assert_eq!(object.rotation, 90.0);
        assert!(matches!(
            &object.content,
            ObjectContent::Text { content, font_size: Some(10.0), align: TextAlign::Center, .. }
                if content == "Hello & world"
        ));
    }

    #[test]
    fn landscape_page_margins_are_swapped_into_printable_origin() {
        let xml = r#"<document xmlns:pt="urn:pt" xmlns:style="urn:style" xmlns:text="urn:text"><style:paper width="68pt" height="152.8pt" marginLeft="8.4pt" marginTop="5.6pt" orientation="landscape"/><pt:objects><text:text><pt:objectStyle x="5.7pt" y="8.5pt" width="141.4pt" height="16.8pt"/><pt:data>Label</pt:data></text:text></pt:objects></document>"#;
        let result = import(&archive(xml)).unwrap();
        let Some(LabelElement::Object(object)) = result.document.elements.first() else {
            panic!("expected text object");
        };
        assert!((object.frame.x_mm - 0.1 * PT_TO_MM).abs() < 0.001);
        assert!((object.frame.y_mm - 0.1 * PT_TO_MM).abs() < 0.001);
    }

    #[test]
    fn rejects_files_without_supported_objects() {
        let xml = format!("{PREFIX}<cable:cable xmlns:cable=\"urn:cable\"/>{SUFFIX}");
        let error = import(&archive(&xml)).unwrap_err().to_string();
        assert!(error.contains(
            "no supported text, PT Dingbats symbols, untrimmed images, or unfilled vector objects"
        ));
        assert!(error.contains("cable"));
    }

    #[test]
    fn reports_specialized_objects_when_text_is_also_present() {
        let xml = format!(
            "{PREFIX}<cable:cable xmlns:cable=\"urn:cable\"/><text:text><pt:objectStyle x=\"0pt\" y=\"0pt\" width=\"24pt\" height=\"18pt\"/><pt:data>Hi</pt:data></text:text>{SUFFIX}"
        );
        let result = import(&archive(&xml)).unwrap();
        assert_eq!(result.document.elements.len(), 1);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("cable"));
    }

    #[test]
    fn imports_untrimmed_embedded_image_with_physical_frame() {
        let mut image = Cursor::new(Vec::new());
        let mut source = image::RgbImage::from_pixel(3, 3, image::Rgb([255, 255, 255]));
        source.put_pixel(1, 1, image::Rgb([0, 0, 0]));
        image::DynamicImage::ImageRgb8(source)
            .write_to(&mut image, image::ImageFormat::Png)
            .unwrap();
        let xml = format!(
            "{PREFIX}<image:image><pt:objectStyle x=\"36pt\" y=\"2pt\" width=\"72pt\" height=\"24pt\" angle=\"0\"/><image:imageStyle fileName=\"assets/picture.png\"/><image:trimming flag=\"false\"/></image:image>{SUFFIX}"
        );
        let result = import(&archive_with_image(
            &xml,
            "assets/picture.png",
            image.get_ref(),
        ))
        .unwrap();
        assert!(result.warnings.is_empty());
        let Some(LabelElement::Object(object)) = result.document.elements.first() else {
            panic!("expected object element");
        };
        assert!((object.frame.x_mm - 12.7).abs() < 0.001);
        assert!((object.frame.width_mm - 25.4).abs() < 0.001);
        assert!(matches!(
            &object.content,
            ObjectContent::Image {
                image_data,
                bitmap: Some(_),
                ..
            } if image_data == image.get_ref()
        ));
        let rendered = crate::layout::render_document(
            &result.document,
            170,
            180,
            &mut crate::text::TextRenderer::with_test_font(),
        )
        .unwrap();
        let bitmap = rendered.bitmap.unwrap();
        assert!(
            rendered.bounds.iter().any(|bounds| {
                (bounds.y..bounds.y + bounds.height)
                    .any(|y| (bounds.x..bounds.x + bounds.width).any(|x| bitmap.get_pixel(x, y)))
            }),
            "rendered bitmap had no black pixels: bounds={:?}, bytes={:?}",
            rendered.bounds,
            bitmap.data()
        );
    }

    #[test]
    fn shared_document_loader_opens_an_lbx_image_file() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let mut image = Cursor::new(Vec::new());
        let mut source = image::RgbImage::from_pixel(2, 2, image::Rgb([255, 255, 255]));
        source.put_pixel(0, 0, image::Rgb([0, 0, 0]));
        image::DynamicImage::ImageRgb8(source)
            .write_to(&mut image, image::ImageFormat::Png)
            .unwrap();
        let xml = format!(
            "{PREFIX}<image:image><pt:objectStyle x=\"0pt\" y=\"0pt\" width=\"24pt\" height=\"18pt\"/><image:imageStyle fileName=\"picture.png\"/></image:image>{SUFFIX}"
        );
        let archive = archive_with_image(&xml, "picture.png", image.get_ref());
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("ptouch-lbx-{}-{unique}.lbx", std::process::id()));
        std::fs::write(&path, archive).unwrap();
        let loaded = LabelDocument::from_path(&path);
        std::fs::remove_file(&path).unwrap();
        let (document, warnings) = loaded.unwrap();
        assert!(warnings.is_empty());
        assert!(matches!(
            document.elements.first(),
            Some(LabelElement::Object(LabelObject {
                content: ObjectContent::Image {
                    bitmap: Some(_),
                    ..
                },
                ..
            }))
        ));
    }

    #[test]
    fn skips_trimmed_images_with_an_explicit_warning() {
        let mut image = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(1, 1, image::Rgb([0, 0, 0])))
            .write_to(&mut image, image::ImageFormat::Png)
            .unwrap();
        let xml = format!(
            "{PREFIX}<image:image><pt:objectStyle x=\"0pt\" y=\"0pt\" width=\"12pt\" height=\"12pt\"/><image:imageStyle fileName=\"pic.png\"/><image:trimming flag=\"true\"/></image:image><text:text><pt:objectStyle x=\"0pt\" y=\"0pt\" width=\"20pt\" height=\"12pt\"/><pt:data>Text</pt:data></text:text>{SUFFIX}"
        );
        let result = import(&archive_with_image(&xml, "pic.png", image.get_ref())).unwrap();
        assert_eq!(result.document.elements.len(), 1);
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("trimmed LBX image"));
    }

    #[test]
    fn imports_dashed_polyline_as_embedded_vector_content() {
        let xml = format!(
            "{PREFIX}<draw:poly><pt:objectStyle x=\"0pt\" y=\"0pt\" width=\"40pt\" height=\"18pt\" angle=\"0\"><pt:pen style=\"DASH\" widthX=\"0.5pt\" widthY=\"0.5pt\" color=\"#000000\"/><pt:brush style=\"NULL\" color=\"#000000\"/></pt:objectStyle><draw:polyStyle shape=\"LINE\" arrowBegin=\"SQUARE\" arrowEnd=\"SQUARE\"><draw:polyLinePoints points=\"1pt,1pt 39pt,17pt\"/></draw:polyStyle></draw:poly>{SUFFIX}"
        );
        let result = import(&archive(&xml)).unwrap();
        assert_eq!(result.document.elements.len(), 1);
        assert!(result.warnings[0].contains("native shape editing is not retained"));
        let Some(LabelElement::Object(object)) = result.document.elements.first() else {
            panic!("expected vector image object");
        };
        assert!(matches!(
            &object.content,
            ObjectContent::Image { image_data, bitmap: Some(_), .. }
                if String::from_utf8_lossy(image_data).contains("stroke-dasharray=\"4 2\"")
        ));
        let rendered = crate::layout::render_document(
            &result.document,
            128,
            180,
            &mut crate::text::TextRenderer::with_test_font(),
        )
        .unwrap();
        assert!(
            rendered
                .bitmap
                .as_ref()
                .unwrap()
                .data()
                .iter()
                .any(|byte| *byte != 0)
        );
    }

    #[test]
    fn imports_pt_dingbats_symbols_as_styled_editable_glyphs() {
        let xml = format!(
            "{PREFIX}<draw:symbol><pt:objectStyle x=\"3pt\" y=\"4pt\" width=\"20pt\" height=\"18pt\" angle=\"0\"/><draw:symbolStyle fontName=\"PT Dingbats 1\" code=\"65\"/></draw:symbol>{SUFFIX}"
        );
        let result = import(&archive(&xml)).unwrap();
        let Some(LabelElement::Object(object)) = result.document.elements.first() else {
            panic!("expected imported symbol text object");
        };
        assert_eq!(object.frame.x_mm, 3.0 * PT_TO_MM);
        assert_eq!(object.frame.y_mm, 4.0 * PT_TO_MM);
        assert!(matches!(
            &object.content,
            ObjectContent::Text { content, styles, .. }
                if content == "\u{f041}"
                    && styles.len() == 1
                    && styles[0].style.font_name.as_deref() == Some("PT Dingbats 1")
        ));
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| { warning.contains("native symbol metadata is not retained") })
        );
        let glyph_available = crate::font::has_glyph("PT Dingbats 1", '\u{f041}');
        assert_eq!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("glyph U+F041 is unavailable")),
            !glyph_available
        );
        let serialized = result.document.to_toml_string().unwrap();
        let reloaded = LabelDocument::from_toml_str(&serialized).unwrap();
        assert!(matches!(
            &reloaded.elements[0],
            LabelElement::Object(LabelObject {
                content: ObjectContent::Text { content, styles, .. },
                ..
            }) if content == "\u{f041}"
                && styles[0].style.font_name.as_deref() == Some("PT Dingbats 1")
        ));
        if glyph_available {
            let rendered = crate::layout::render_document(
                &reloaded,
                128,
                180,
                &mut crate::text::TextRenderer::new(),
            )
            .unwrap();
            assert!(
                rendered
                    .bitmap
                    .as_ref()
                    .unwrap()
                    .data()
                    .iter()
                    .any(|pixel| *pixel != 0)
            );
        }
    }

    #[test]
    fn unsupported_symbol_families_are_reported_with_their_code() {
        let xml = format!(
            "{PREFIX}<draw:symbol><pt:objectStyle x=\"0pt\" y=\"0pt\" width=\"20pt\" height=\"18pt\"/><draw:symbolStyle fontName=\"Arial\" code=\"65\"/></draw:symbol>{SUFFIX}"
        );
        let error = import(&archive(&xml)).unwrap_err().to_string();
        assert!(error.contains("unsupported font 'Arial' or code 65"));
    }

    #[test]
    fn imports_ellipse_outline_and_flattens_nested_template_objects() {
        let rect = "<draw:rect><pt:objectStyle x=\"0pt\" y=\"0pt\" width=\"30pt\" height=\"18pt\"><pt:pen style=\"SOLID\" widthX=\"0.5pt\" widthY=\"0.5pt\" color=\"#000000\"/><pt:brush style=\"NULL\"/></pt:objectStyle><draw:rectStyle shape=\"ELLIPSE\"/></draw:rect>";
        let xml = format!("{PREFIX}{rect}{SUFFIX}");
        let result = import(&archive(&xml)).unwrap();
        assert!(matches!(
            result.document.elements.first(),
            Some(LabelElement::Object(LabelObject {
                content: ObjectContent::Image { image_data, .. },
                ..
            })) if String::from_utf8_lossy(image_data).contains("<ellipse")
        ));

        let nested = format!(
            "{PREFIX}<cable:cable xmlns:cable=\"urn:cable\"><table:cells xmlns:table=\"urn:table\"><table:cell><text:text><pt:objectStyle x=\"1pt\" y=\"1pt\" width=\"20pt\" height=\"12pt\"/><pt:data>Label</pt:data></text:text><draw:poly><pt:objectStyle x=\"22pt\" y=\"1pt\" width=\"1pt\" height=\"12pt\"><pt:pen style=\"SOLID\" widthX=\"0.5pt\" widthY=\"0.5pt\" color=\"#000000\"/><pt:brush style=\"NULL\"/></pt:objectStyle><draw:polyStyle shape=\"LINE\"><draw:polyLinePoints points=\"22.5pt,1pt 22.5pt,13pt\"/></draw:polyStyle></draw:poly></table:cell></table:cells></cable:cable>{SUFFIX}"
        );
        let result = import(&archive(&nested)).unwrap();
        assert_eq!(result.document.elements.len(), 2);
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("cable"))
        );
        assert!(matches!(
            &result.document.elements[0],
            LabelElement::Object(LabelObject {
                content: ObjectContent::Text { content, .. },
                ..
            }) if content == "Label"
        ));
        assert!(matches!(
            &result.document.elements[1],
            LabelElement::Object(LabelObject {
                content: ObjectContent::Image { .. },
                ..
            })
        ));
    }
}
