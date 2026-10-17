// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Bounded importer for ordinary Brother LBX label objects.

use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};

use zip::ZipArchive;

use crate::document::{LabelDocument, LabelElement, LabelObject, ObjectContent};
use crate::layout::{PositionMode, TextBox, VerticalAlign};
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

/// Import ordinary text and untrimmed image objects from an LBX ZIP archive.
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
    let objects = parsed
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "objects")
        .ok_or_else(|| RenderError::Layout("LBX label.xml has no object list".into()))?;

    let mut elements = Vec::new();
    let mut warnings = Vec::new();
    let mut font_name: Option<String> = None;
    for node in objects.children().filter(|node| node.is_element()) {
        match node.tag_name().name() {
            "text" => {
                import_text(node, &mut elements, &mut warnings, &mut font_name)?;
            }
            "image" => match import_image(&mut archive, node)? {
                ImageImport::Imported(object) => {
                    elements.push(LabelElement::Object(object));
                }
                ImageImport::Skipped(reason) => warnings.push(reason),
            },
            kind => warnings.push(format!(
                "skipped unsupported LBX object '{kind}'; supported objects are ordinary text and untrimmed images"
            )),
        }
    }
    if elements.is_empty() {
        let detail = if warnings.is_empty() {
            String::new()
        } else {
            format!("; {}", warnings.join("; "))
        };
        return Err(RenderError::Layout(format!(
            "LBX file contains no supported text or untrimmed image objects{detail}"
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
    elements: &mut Vec<LabelElement>,
    warnings: &mut Vec<String>,
    font_name: &mut Option<String>,
) -> Result<()> {
    let style = object_style(node, "text")?;
    let frame = frame_from_style(style)?;
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

enum ImageImport {
    Imported(LabelObject),
    Skipped(String),
}

fn import_image(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    node: roxmltree::Node<'_, '_>,
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
    let frame = frame_from_style(object_style(node, "image")?)?;
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

fn frame_from_style(style: roxmltree::Node<'_, '_>) -> Result<TextBox> {
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
        x_mm: point_attr(style, "x")? * PT_TO_MM,
        y_mm: point_attr(style, "y")? * PT_TO_MM,
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

    const PREFIX: &str = r#"<document xmlns:pt="urn:pt" xmlns:style="urn:style" xmlns:text="urn:text" xmlns:image="urn:image"><style:paper width="68pt"/><pt:objects>"#;
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
        assert!(error.contains("no supported text or untrimmed image objects"));
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
}
