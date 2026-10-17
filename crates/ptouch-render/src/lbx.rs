// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Bounded importer for ordinary Brother LBX label objects.

use std::io::{Cursor, Read};

use zip::ZipArchive;

use crate::document::{LabelDocument, LabelElement, LabelObject, ObjectContent};
use crate::layout::{PositionMode, TextBox, VerticalAlign};
use crate::text::TextAlign;
use crate::{RenderError, Result};

const PT_TO_MM: f64 = 25.4 / 72.0;
pub const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const MAX_XML_BYTES: u64 = 8 * 1024 * 1024;

/// An imported LBX document and any structures that were skipped.
#[derive(Debug)]
pub struct LbxImport {
    pub document: LabelDocument,
    pub warnings: Vec<String>,
}

/// Import ordinary text objects from an LBX ZIP archive.
///
/// Specialized cable, table, barcode, drawing, and image objects are skipped
/// with a warning. A file with no supported objects is rejected to avoid
/// silently opening a blank label.
pub fn import(bytes: &[u8]) -> Result<LbxImport> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(RenderError::Layout(
            "LBX archive exceeds the 64 MiB import limit".into(),
        ));
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| RenderError::Layout(format!("invalid LBX archive: {error}")))?;
    let mut entry = archive
        .by_name("label.xml")
        .map_err(|error| RenderError::Layout(format!("LBX archive has no label.xml: {error}")))?;
    if entry.size() > MAX_XML_BYTES {
        return Err(RenderError::Layout(format!(
            "LBX label.xml exceeds the {} MiB import limit",
            MAX_XML_BYTES / (1024 * 1024)
        )));
    }
    let mut xml = String::new();
    entry.read_to_string(&mut xml)?;
    let parsed = roxmltree::Document::parse(&xml)
        .map_err(|error| RenderError::Layout(format!("invalid LBX label.xml: {error}")))?;
    let paper = parsed
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "paper")
        .ok_or_else(|| RenderError::Layout("LBX label.xml has no paper settings".into()))?;
    let paper_width = point_attr(paper, "width")?;
    let tape_width_mm = (paper_width * PT_TO_MM).round().clamp(1.0, 255.0) as u8;
    let objects = parsed
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "objects")
        .ok_or_else(|| RenderError::Layout("LBX label.xml has no object list".into()))?;

    let mut elements = Vec::new();
    let mut warnings = Vec::new();
    let mut font_name: Option<String> = None;
    for node in objects.children().filter(|node| node.is_element()) {
        if node.tag_name().name() != "text" {
            warnings.push(format!(
                "skipped unsupported LBX object '{}'; only ordinary text objects are imported",
                node.tag_name().name()
            ));
            continue;
        }
        let style = node
            .children()
            .find(|child| child.is_element() && child.tag_name().name() == "objectStyle")
            .ok_or_else(|| RenderError::Layout("LBX text object has no objectStyle".into()))?;
        let content = node
            .children()
            .find(|child| child.is_element() && child.tag_name().name() == "data")
            .and_then(|data| data.text())
            .unwrap_or_default()
            .to_owned();
        let width = point_attr(style, "width")? * PT_TO_MM;
        let height = point_attr(style, "height")? * PT_TO_MM;
        let x = point_attr(style, "x")? * PT_TO_MM;
        let y = point_attr(style, "y")? * PT_TO_MM;
        let rotation = style
            .attribute("angle")
            .unwrap_or("0")
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite())
            .ok_or_else(|| RenderError::Layout("LBX text object has an invalid angle".into()))?;
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
            match &font_name {
                Some(existing) if existing != name => {
                    warnings.push(format!(
                        "LBX uses multiple font families; '{name}' falls back to '{existing}'"
                    ));
                }
                None => font_name = Some(name.to_owned()),
                _ => {}
            }
        }
        let frame = TextBox {
            width_mm: width,
            height_mm: height,
            position: PositionMode::Absolute,
            x_mm: x,
            y_mm: y,
            gap_mm: 0.0,
            vertical_align: VerticalAlign::Center,
        };
        if width == 0.0 || height == 0.0 {
            return Err(RenderError::Layout(
                "LBX text object has a zero-sized frame".into(),
            ));
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
    }
    if elements.is_empty() {
        return Err(RenderError::Layout(
            "LBX file contains no supported ordinary text objects".into(),
        ));
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

fn parse_points(value: &str) -> Option<f64> {
    value.strip_suffix("pt")?.parse().ok()
}

fn point_attr(node: roxmltree::Node<'_, '_>, name: &str) -> Result<f64> {
    node.attribute(name)
        .and_then(parse_points)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| RenderError::Layout(format!("LBX has invalid {name} point value")))
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

    const PREFIX: &str = r#"<document xmlns:pt="urn:pt" xmlns:style="urn:style" xmlns:text="urn:text"><style:paper width="68pt"/><pt:objects>"#;
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
    fn rejects_files_without_supported_objects() {
        let xml = format!("{PREFIX}<cable:cable xmlns:cable=\"urn:cable\"/>{SUFFIX}");
        let error = import(&archive(&xml)).unwrap_err().to_string();
        assert!(error.contains("no supported ordinary text objects"));
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
}
