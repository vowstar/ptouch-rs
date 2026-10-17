// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Bounded importer for supported Brother LBX label objects.

use std::io::{Cursor, Read, Write};
use std::path::{Component, Path, PathBuf};

use zip::ZipArchive;

use crate::document::{ContentBox, LabelDocument, LabelElement, LabelObject, ObjectContent};
use crate::layout::{PositionMode, TextBox, VerticalAlign};
use crate::styled_text::{StyleSpan, TextStyle};
use crate::text::TextAlign;
use crate::{RenderError, Result};
use qrcodegen::{QrCode, QrCodeEcc, QrSegment, Version};

const PT_TO_MM: f64 = 25.4 / 72.0;
pub const MAX_ARCHIVE_BYTES: usize = 64 * 1024 * 1024;
const MAX_XML_BYTES: u64 = 8 * 1024 * 1024;
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;

/// Printer identity written into a generated LBX paper definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LbxPrinter {
    /// PT-E550W uses the PT7 paper map and 180 dpi LBX profile.
    PtE550w,
    /// PT-P900W uses the PT9 paper map and 360 dpi LBX profile.
    PtP900w,
}

/// Export supported absolute text and raster-image objects as a Brother LBX archive.
///
/// The output uses the Brother LBX XML namespaces and archive layout. Imported
/// template wrappers, inline text styles, relative objects, vector-only image
/// sources, mirroring, and non-object layout elements are rejected rather than
/// silently discarded. Opening the output in a Brother application remains to
/// be verified against that application's runtime.
pub fn export(document: &LabelDocument, printer: LbxPrinter) -> Result<Vec<u8>> {
    if document.flip_h || document.flip_v {
        return Err(RenderError::Layout(
            "LBX export does not support whole-label mirroring".into(),
        ));
    }
    let media = lbx_media(document.tape_width_mm, printer).ok_or_else(|| {
        RenderError::Layout(format!(
            "tape width {} mm has no verified LBX media mapping for {printer:?}",
            document.tape_width_mm
        ))
    })?;
    let mut objects = String::new();
    let mut assets = Vec::<(String, Vec<u8>)>::new();
    let mut resource_bytes = 0usize;
    let mut max_length_mm = 4.0_f64;
    for (index, element) in document.elements.iter().enumerate() {
        let LabelElement::Object(object) = element else {
            return Err(RenderError::Layout(format!(
                "LBX export does not support non-object layout element {index}"
            )));
        };
        let qr_content_box_supported = matches!(object.content, ObjectContent::QrCode { .. })
            && object.content_box.as_ref().is_none_or(|content| {
                content.offset_x_mm == 0.0
                    && content.offset_y_mm == 0.0
                    && content.width_mm.is_finite()
                    && content.height_mm.is_finite()
                    && content.width_mm > 0.0
                    && content.height_mm > 0.0
                    && content.width_mm <= object.frame.width_mm
                    && content.height_mm <= object.frame.height_mm
            });
        if object.frame.position != PositionMode::Absolute
            || (object.content_box.is_some() && !qr_content_box_supported)
            || object.flip_h
            || object.flip_v
        {
            return Err(RenderError::Layout(format!(
                "LBX export does not support relative positioning, independent content boxes, or mirroring on object {index}"
            )));
        }
        if !object.frame.x_mm.is_finite()
            || !object.frame.y_mm.is_finite()
            || object.frame.x_mm < 0.0
            || object.frame.y_mm < 0.0
            || !object.frame.width_mm.is_finite()
            || !object.frame.height_mm.is_finite()
            || object.frame.width_mm <= 0.0
            || object.frame.height_mm <= 0.0
            || !object.rotation.is_finite()
        {
            return Err(RenderError::Layout(format!(
                "LBX object {index} has invalid geometry"
            )));
        }
        if object.frame.y_mm + object.frame.height_mm > f64::from(document.tape_width_mm) {
            return Err(RenderError::Layout(format!(
                "LBX object {index} extends beyond the selected tape width"
            )));
        }
        max_length_mm = max_length_mm.max(object.frame.x_mm + object.frame.width_mm);
        match &object.content {
            ObjectContent::Text {
                content,
                font_size,
                align,
                line_spacing,
                styles,
            } => {
                if !styles.is_empty() {
                    return Err(RenderError::Layout(format!(
                        "LBX export does not support inline text styles on object {index}"
                    )));
                }
                if *line_spacing != crate::text::DEFAULT_LINE_SPACING {
                    return Err(RenderError::Layout(format!(
                        "LBX export does not support custom line spacing on object {index}"
                    )));
                }
                let font_size = font_size.unwrap_or(10.0);
                if !font_size.is_finite() || font_size <= 0.0 {
                    return Err(RenderError::Layout(format!(
                        "LBX text object {index} has an invalid font size"
                    )));
                }
                let (x, y, width, height) = style_geometry(object);
                let alignment = match align {
                    TextAlign::Left => "LEFT",
                    TextAlign::Center => "CENTER",
                    TextAlign::Right => "RIGHT",
                };
                let vertical_alignment = match object.frame.vertical_align {
                    VerticalAlign::Top => "TOP",
                    VerticalAlign::Center => "CENTER",
                    VerticalAlign::Bottom => "BOTTOM",
                };
                objects.push_str(&format!(
                    "<text:text><pt:objectStyle x=\"{x}pt\" y=\"{y}pt\" width=\"{width}pt\" height=\"{height}pt\" angle=\"{}\" anchor=\"TOPLEFT\" flip=\"NONE\"><pt:pen style=\"NULL\" widthX=\"0.5pt\" widthY=\"0.5pt\" color=\"#000000\" printColorNumber=\"1\"/><pt:brush style=\"NULL\" color=\"#000000\" printColorNumber=\"1\" id=\"0\"/></pt:objectStyle><text:ptFontInfo><text:logFont name=\"{}\" width=\"0\" italic=\"false\" weight=\"400\" charSet=\"0\" pitchAndFamily=\"34\"/><text:fontExt effect=\"NOEFFECT\" underline=\"0\" strikeout=\"0\" size=\"{font_size}pt\" orgSize=\"{font_size}pt\" textColor=\"#000000\" textPrintColorNumber=\"1\"/></text:ptFontInfo><text:textControl control=\"FIXEDFRAME\" clipFrame=\"false\" aspectNormal=\"true\" shrink=\"true\" autoLF=\"false\" avoidImage=\"false\"/><text:textAlign horizontalAlignment=\"{alignment}\" verticalAlignment=\"{vertical_alignment}\" inLineAlignment=\"BASELINE\"/><pt:data>{}</pt:data></text:text>",
                    object.rotation,
                    xml_escape(&document.font_name)?,
                    xml_escape(content)?,
                ));
            }
            ObjectContent::Image { image_data, .. } => {
                if image_data.len() as u64 > MAX_IMAGE_BYTES {
                    return Err(RenderError::Layout(format!(
                        "LBX image object {index} exceeds the 32 MiB resource limit"
                    )));
                }
                resource_bytes = resource_bytes
                    .checked_add(image_data.len())
                    .filter(|total| *total <= MAX_ARCHIVE_BYTES)
                    .ok_or_else(|| {
                        RenderError::Layout(
                            "LBX image resources exceed the 64 MiB archive limit".into(),
                        )
                    })?;
                let extension = image::guess_format(image_data)
                    .ok()
                    .and_then(image_extension)
                    .ok_or_else(|| {
                        RenderError::Layout(format!(
                            "LBX export supports PNG, JPEG, BMP, GIF, and TIFF images; object {index} has an unsupported or vector-only image"
                        ))
                    })?;
                let name = format!("assets/object-{index}.{extension}");
                let (x, y, width, height) = style_geometry(object);
                objects.push_str(&format!(
                    "<image:image><pt:objectStyle x=\"{x}pt\" y=\"{y}pt\" width=\"{width}pt\" height=\"{height}pt\" angle=\"{}\" anchor=\"TOPLEFT\" flip=\"NONE\"><pt:pen style=\"NULL\" widthX=\"0pt\" widthY=\"0pt\" color=\"#000000\"/><pt:brush style=\"NULL\" color=\"#000000\"/></pt:objectStyle><image:imageStyle fileName=\"{name}\"/><image:trimming flag=\"false\"/></image:image>",
                    object.rotation
                ));
                assets.push((name, image_data.clone()));
            }
            ObjectContent::QrCode {
                data,
                error_correction,
                version,
            } => {
                if data.is_empty() || data.len() > 4096 {
                    return Err(RenderError::Layout(format!(
                        "LBX QR object {index} has empty or oversized data"
                    )));
                }
                // Validate capacity before emitting a native barcode object.
                let qr_bitmap = crate::barcode::render_qr(data, *error_correction, *version)?;
                let (x, y, width, height) = style_geometry(object);
                let frame_width = width.parse::<f64>().map_err(|_| {
                    RenderError::Layout(format!("LBX QR object {index} has invalid width"))
                })?;
                let frame_height = height.parse::<f64>().map_err(|_| {
                    RenderError::Layout(format!("LBX QR object {index} has invalid height"))
                })?;
                let modules_with_quiet_zone = f64::from(qr_bitmap.width() / 8);
                let content_width = object
                    .content_box
                    .as_ref()
                    .map_or(frame_width, |content| content.width_mm * 72.0 / 25.4);
                let content_height = object
                    .content_box
                    .as_ref()
                    .map_or(frame_height, |content| content.height_mm * 72.0 / 25.4);
                let cell_size = content_width.min(content_height) / modules_with_quiet_zone;
                if !cell_size.is_finite() || cell_size <= 0.0 {
                    return Err(RenderError::Layout(format!(
                        "LBX QR object {index} has an invalid physical frame"
                    )));
                }
                objects.push_str(&format!(
                    "<barcode:barcode><pt:objectStyle x=\"{x}pt\" y=\"{y}pt\" width=\"{width}pt\" height=\"{height}pt\" angle=\"{}\" anchor=\"TOPLEFT\" flip=\"NONE\"><pt:pen style=\"NULL\" widthX=\"0pt\" widthY=\"0pt\" color=\"#000000\"/><pt:brush style=\"NULL\" color=\"#000000\"/></pt:objectStyle><barcode:barcodeStyle protocol=\"QRCODE\" margin=\"false\"/><pt:data>{}</pt:data><barcode:qrcodeStyle model=\"2\" eccLevel=\"{}\" cellSize=\"{cell_size:.3}pt\" mbcs=\"932\" removeCharKind=\"0\" removeCharString=\"\" joint=\"1\" jointSpace=\"8\" jointVertically=\"false\" version=\"{}\"/></barcode:barcode>",
                    object.rotation,
                    xml_escape(data)?,
                    error_correction.lbx_name(),
                    version.map_or_else(|| "auto".to_string(), |v| v.to_string()),
                ));
            }
            ObjectContent::Code39 {
                data,
                bar_width_pt,
                check_digit,
                human_readable,
            } => {
                if !bar_width_pt.is_finite() || !(0.1..=10.0).contains(bar_width_pt) {
                    return Err(RenderError::Layout(format!(
                        "LBX Code 39 object {index} has an invalid narrow bar width"
                    )));
                }
                // Validate payload before emitting a native barcode object.
                crate::barcode::render_code39(data, *check_digit, 2)?;
                let (x, y, width, height) = style_geometry(object);
                objects.push_str(&format!(
                    "<barcode:barcode><pt:objectStyle x=\"{x}pt\" y=\"{y}pt\" width=\"{width}pt\" height=\"{height}pt\" angle=\"{}\" anchor=\"TOPLEFT\" flip=\"NONE\"><pt:pen style=\"NULL\" widthX=\"0pt\" widthY=\"0pt\" color=\"#000000\"/><pt:brush style=\"NULL\" color=\"#000000\"/></pt:objectStyle><barcode:barcodeStyle protocol=\"CODE39\" lengths=\"{}\" zeroFill=\"false\" barWidth=\"{bar_width_pt}pt\" barRatio=\"1:3\" humanReadable=\"{}\" humanReadableAlignment=\"CENTER\" checkDigit=\"{}\" autoLengths=\"false\" margin=\"false\" sameLengthBar=\"false\" bearerBar=\"false\"/><pt:data>{}</pt:data></barcode:barcode>",
                    object.rotation,
                    data.len(),
                    human_readable,
                    check_digit,
                    xml_escape(data)?,
                ));
            }
            ObjectContent::CutMark | ObjectContent::Padding => {
                return Err(RenderError::Layout(format!(
                    "LBX export does not support cut marks or padding objects (object {index})"
                )));
            }
        }
    }
    if objects.is_empty() {
        return Err(RenderError::Layout(
            "LBX export requires at least one text, image, or QR object".into(),
        ));
    }
    let length_pt = max_length_mm * 72.0 / 25.4;
    let tape_width_pt = f64::from(document.tape_width_mm) * 72.0 / 25.4;
    let (printer_id, printer_name) = match printer {
        LbxPrinter::PtE550w => ("26160", "PT_E550W"),
        LbxPrinter::PtP900w => ("28464", "PT_P900W"),
    };
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><pt:document xmlns:pt=\"http://schemas.brother.info/ptouch/2007/lbx/main\" xmlns:style=\"http://schemas.brother.info/ptouch/2007/lbx/style\" xmlns:text=\"http://schemas.brother.info/ptouch/2007/lbx/text\" xmlns:draw=\"http://schemas.brother.info/ptouch/2007/lbx/draw\" xmlns:image=\"http://schemas.brother.info/ptouch/2007/lbx/image\" xmlns:barcode=\"http://schemas.brother.info/ptouch/2007/lbx/barcode\" xmlns:database=\"http://schemas.brother.info/ptouch/2007/lbx/database\" xmlns:table=\"http://schemas.brother.info/ptouch/2007/lbx/table\" xmlns:cable=\"http://schemas.brother.info/ptouch/2007/lbx/cable\" version=\"1.8\" generator=\"ptouch-rs\"><pt:body currentSheet=\"Sheet1\" direction=\"LTR\"><style:sheet name=\"Sheet1\"><style:paper media=\"0\" width=\"{tape_width_pt:.2}pt\" height=\"{length_pt:.2}pt\" marginLeft=\"0pt\" marginTop=\"0pt\" marginRight=\"0pt\" marginBottom=\"0pt\" orientation=\"landscape\" autoLength=\"true\" monochromeDisplay=\"true\" printColorDisplay=\"false\" printColorsID=\"0\" paperColor=\"#FFFFFF\" paperInk=\"#000000\" split=\"1\" format=\"{media}\" backgroundTheme=\"0\" printerID=\"{printer_id}\" printerName=\"{printer_name}\"/><style:cutLine regularCut=\"0pt\" freeCut=\"\"/><pt:objects>{objects}</pt:objects></style:sheet></pt:body></pt:document>"
    );
    if xml.len() > MAX_XML_BYTES as usize
        || resource_bytes.saturating_add(xml.len()) > MAX_ARCHIVE_BYTES
    {
        return Err(RenderError::Layout(
            "generated LBX exceeds the 64 MiB archive or 8 MiB XML limit".into(),
        ));
    }
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    writer.start_file("label.xml", options).map_err(zip_error)?;
    writer.write_all(xml.as_bytes())?;
    for (name, data) in assets {
        writer.start_file(name, options).map_err(zip_error)?;
        writer.write_all(&data)?;
    }
    writer.start_file("prop.xml", options).map_err(zip_error)?;
    writer.write_all(
        b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><meta:properties xmlns:meta=\"http://schemas.brother.info/ptouch/2007/lbx/meta\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\"><meta:appName>ptouch-rs</meta:appName><dc:title></dc:title><dc:subject></dc:subject><dc:creator>ptouch-rs</dc:creator><meta:keyword></meta:keyword><dc:description></dc:description><meta:template></meta:template><dcterms:created>2026-10-17T00:00:00Z</dcterms:created><dcterms:modified>2026-10-17T00:00:00Z</dcterms:modified><meta:lastPrinted></meta:lastPrinted><meta:modifiedBy>ptouch-rs</meta:modifiedBy><meta:revision>1</meta:revision><meta:editTime>0</meta:editTime><meta:numPages>1</meta:numPages><meta:numWords>0</meta:numWords><meta:numChars>0</meta:numChars><meta:security>0</meta:security></meta:properties>",
    )?;
    let cursor = writer.finish().map_err(zip_error)?;
    let output = cursor.into_inner();
    if output.len() > MAX_ARCHIVE_BYTES {
        return Err(RenderError::Layout(
            "generated LBX exceeds the 64 MiB archive limit".into(),
        ));
    }
    Ok(output)
}

fn lbx_media(width_mm: u8, printer: LbxPrinter) -> Option<u16> {
    let media: &[(u8, u16)] = match printer {
        LbxPrinter::PtE550w => &[(6, 257), (9, 258), (12, 259), (18, 260), (24, 261)],
        LbxPrinter::PtP900w => &[
            (6, 257),
            (9, 258),
            (12, 259),
            (18, 260),
            (24, 261),
            (36, 262),
        ],
    };
    media
        .iter()
        .find_map(|(width, code)| (*width == width_mm).then_some(*code))
}

fn style_geometry(object: &LabelObject) -> (String, String, String, String) {
    let to_points = |mm: f64| format!("{:.3}", mm * 72.0 / 25.4);
    (
        to_points(object.frame.x_mm),
        to_points(object.frame.y_mm),
        to_points(object.frame.width_mm),
        to_points(object.frame.height_mm),
    )
}

fn image_extension(format: image::ImageFormat) -> Option<&'static str> {
    match format {
        image::ImageFormat::Png => Some("png"),
        image::ImageFormat::Jpeg => Some("jpg"),
        image::ImageFormat::Bmp => Some("bmp"),
        image::ImageFormat::Gif => Some("gif"),
        image::ImageFormat::Tiff => Some("tif"),
        _ => None,
    }
}

fn xml_escape(value: &str) -> Result<String> {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '\"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            '\u{9}' | '\u{a}' | '\u{d}' => escaped.push(character),
            c if c >= '\u{20}' && ((c as u32) <= 0xD7FF || (c as u32) >= 0xE000) => {
                escaped.push(character)
            }
            _ => {
                return Err(RenderError::Layout(
                    "text contains a character forbidden by XML 1.0".into(),
                ));
            }
        }
    }
    Ok(escaped)
}

fn zip_error(error: zip::result::ZipError) -> RenderError {
    RenderError::Layout(format!("failed to write LBX archive: {error}"))
}

/// An imported LBX document and any structures that were skipped.
#[derive(Debug)]
pub struct LbxImport {
    pub document: LabelDocument,
    pub warnings: Vec<String>,
}

/// Import supported text, PT Dingbats symbols, QR codes, images, and vectors.
///
/// Unsupported barcode protocols, cable/table/drawing objects, trimmed images,
/// and non-default image effects are skipped with warnings. A file with no
/// supported objects is rejected to avoid silently opening a blank label.
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
            "LBX file contains no supported text, PT Dingbats symbols, QR codes, untrimmed images, or unfilled vector objects{detail}"
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

fn import_barcode(node: roxmltree::Node<'_, '_>, page: LbxPage) -> Result<ImageImport> {
    let barcode_style = node
        .descendants()
        .find(|child| child.is_element() && child.tag_name().name() == "barcodeStyle")
        .ok_or_else(|| RenderError::Layout("LBX barcode has no barcodeStyle".into()))?;
    let protocol = barcode_style
        .attribute("protocol")
        .ok_or_else(|| RenderError::Layout("LBX barcodeStyle has no protocol".into()))?;
    if protocol == "CODE39" {
        if barcode_style
            .attribute("barRatio")
            .is_some_and(|ratio| ratio != "1:3")
            || barcode_style
                .attribute("humanReadableAlignment")
                .is_some_and(|alignment| alignment != "CENTER")
        {
            return Ok(ImageImport::Skipped(
                "skipped LBX Code 39 barcode with unsupported bar ratio or readable-text alignment"
                    .into(),
            ));
        }
        let data = node
            .children()
            .find(|child| child.is_element() && child.tag_name().name() == "data")
            .and_then(|child| child.text())
            .unwrap_or_default();
        if let Err(error) = crate::barcode::render_code39(data, false, 2) {
            return Ok(ImageImport::Skipped(format!(
                "skipped LBX Code 39 barcode: {error}"
            )));
        }
        let style = object_style(node, "barcode")?;
        let frame = frame_from_style(style, page)?;
        let bar_width_pt = match barcode_style.attribute("barWidth") {
            None => 1.2,
            Some(value) => match parse_points(value) {
                Some(width) if width.is_finite() && (0.1..=10.0).contains(&width) => width as f32,
                _ => {
                    return Ok(ImageImport::Skipped(
                        "skipped LBX Code 39 barcode with unsupported bar width".into(),
                    ));
                }
            },
        };
        if !bar_width_pt.is_finite() || !(0.1..=10.0).contains(&bar_width_pt) {
            return Ok(ImageImport::Skipped(
                "skipped LBX Code 39 barcode with unsupported bar width".into(),
            ));
        }
        let mut object = LabelObject::code39(data, frame);
        object.rotation = angle_from_style(style)?;
        if let ObjectContent::Code39 {
            bar_width_pt: imported_width,
            check_digit: imported_check_digit,
            human_readable: imported_human_readable,
            ..
        } = &mut object.content
        {
            *imported_width = bar_width_pt;
            *imported_check_digit = barcode_style
                .attribute("checkDigit")
                .is_some_and(|value| value.eq_ignore_ascii_case("true"));
            *imported_human_readable = barcode_style
                .attribute("humanReadable")
                .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        }
        return Ok(ImageImport::Imported(object));
    }
    if protocol != "QRCODE" {
        return Ok(ImageImport::Skipped(format!(
            "skipped LBX barcode protocol '{protocol}'; only Code 39 and QR codes are imported"
        )));
    }
    let qr_style = node
        .descendants()
        .find(|child| child.is_element() && child.tag_name().name() == "qrcodeStyle")
        .ok_or_else(|| RenderError::Layout("LBX QR barcode has no qrcodeStyle".into()))?;
    if qr_style.attribute("model").unwrap_or("2") != "2"
        || qr_style.attribute("mbcs").is_some_and(|mbcs| mbcs != "932")
        || qr_style
            .attribute("removeCharKind")
            .is_some_and(|kind| kind != "0")
        || qr_style
            .attribute("removeCharString")
            .is_some_and(|value| !value.is_empty())
        || qr_style
            .attribute("jointVertically")
            .is_some_and(|value| value != "false")
    {
        return Ok(ImageImport::Skipped(
            "skipped LBX QR code with unsupported model, encoding, or character transformation settings".into(),
        ));
    }
    let data = node
        .children()
        .find(|child| child.is_element() && child.tag_name().name() == "data")
        .and_then(|child| child.text())
        .unwrap_or_default();
    if !data.is_ascii() {
        return Ok(ImageImport::Skipped(
            "skipped non-ASCII LBX QR payload; source MBCS conversion is not reproduced".into(),
        ));
    }
    let (ecc, error_correction) = match qr_style.attribute("eccLevel").unwrap_or("15%") {
        "7%" => (QrCodeEcc::Low, crate::barcode::QrErrorCorrection::Low),
        "15%" => (QrCodeEcc::Medium, crate::barcode::QrErrorCorrection::Medium),
        "25%" => (
            QrCodeEcc::Quartile,
            crate::barcode::QrErrorCorrection::Quartile,
        ),
        "30%" => (QrCodeEcc::High, crate::barcode::QrErrorCorrection::High),
        level => {
            return Ok(ImageImport::Skipped(format!(
                "skipped LBX QR code with unsupported error-correction level '{level}'"
            )));
        }
    };
    let version_attr = qr_style.attribute("version").unwrap_or("auto");
    let fixed_version = if version_attr.eq_ignore_ascii_case("auto") {
        None
    } else {
        Some(match version_attr.parse::<u8>() {
            Ok(version @ 1..=40) => version,
            _ => {
                return Ok(ImageImport::Skipped(format!(
                    "skipped LBX QR code with unsupported version '{version_attr}'"
                )));
            }
        })
    };
    let segments = QrSegment::make_segments(data);
    let qr = match fixed_version {
        None => match QrCode::encode_segments_advanced(
            &segments,
            ecc,
            Version::MIN,
            Version::MAX,
            None,
            false,
        ) {
            Ok(qr) => qr,
            Err(_) => {
                return Ok(ImageImport::Skipped(
                    "skipped LBX QR payload that exceeds the Model 2 QR capacity".into(),
                ));
            }
        },
        Some(fixed_version) => {
            let version = Version::new(fixed_version);
            match QrCode::encode_segments_advanced(&segments, ecc, version, version, None, false) {
                Ok(qr) => qr,
                Err(_) => {
                    return Ok(ImageImport::Skipped(
                        "skipped LBX QR payload that exceeds its configured version".into(),
                    ));
                }
            }
        }
    };

    let style = object_style(node, "barcode")?;
    let frame = frame_from_style(style, page)?;
    let width_pt = point_attr_raw(style, "width")?;
    let height_pt = point_attr_raw(style, "height")?;
    let cell_size = match qr_style.attribute("cellSize").and_then(parse_points) {
        Some(size) if size > 0.0 => size,
        _ => {
            return Ok(ImageImport::Skipped(
                "skipped LBX QR code with missing or invalid cellSize".into(),
            ));
        }
    };
    let modules = qr.size() as f64;
    let total_modules = modules + 8.0;
    let fitted_cell_size = cell_size
        .min(width_pt / total_modules)
        .min(height_pt / total_modules);
    if !fitted_cell_size.is_finite() || fitted_cell_size <= 0.0 {
        return Ok(ImageImport::Skipped(
            "skipped LBX QR code with an invalid physical frame".into(),
        ));
    }
    let total_size = total_modules * fitted_cell_size;
    let mut object = LabelObject::qr_code(data, frame);
    object.rotation = angle_from_style(style)?;
    object.content_box = Some(ContentBox {
        width_mm: total_size * PT_TO_MM,
        height_mm: total_size * PT_TO_MM,
        offset_x_mm: 0.0,
        offset_y_mm: 0.0,
    });
    if let ObjectContent::QrCode {
        error_correction: imported_correction,
        version,
        ..
    } = &mut object.content
    {
        *imported_correction = error_correction;
        *version = fixed_version;
    }
    Ok(ImageImport::Imported(object))
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
        "barcode" => match import_barcode(node, page)? {
            ImageImport::Imported(object) => elements.push(LabelElement::Object(object)),
            ImageImport::Skipped(reason) => warnings.push(reason),
        },
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
                    "text" | "symbol" | "barcode" | "image" | "poly" | "rect"
                ) || child.descendants().any(|descendant| {
                    descendant.is_element()
                        && matches!(
                            descendant.tag_name().name(),
                            "text" | "symbol" | "barcode" | "image" | "poly" | "rect"
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

    const PREFIX: &str = r#"<document xmlns:pt="urn:pt" xmlns:style="urn:style" xmlns:text="urn:text" xmlns:image="urn:image" xmlns:draw="urn:draw" xmlns:barcode="urn:barcode"><style:paper width="68pt"/><pt:objects>"#;
    const SUFFIX: &str = "</pt:objects></document>";

    fn text_document(width: u8, text: &str) -> LabelDocument {
        LabelDocument {
            version: crate::document::DOCUMENT_VERSION,
            tape_width_mm: width,
            dpi: 180,
            font_name: "Arial & Sans".into(),
            font_margin: 0,
            flip_h: false,
            flip_v: false,
            elements: vec![LabelElement::Object(LabelObject::text(
                text,
                TextBox {
                    width_mm: 20.0,
                    height_mm: 8.0,
                    position: PositionMode::Absolute,
                    x_mm: 3.0,
                    y_mm: 2.0,
                    gap_mm: 0.0,
                    vertical_align: VerticalAlign::Center,
                },
            ))],
        }
    }

    #[test]
    fn exports_and_reimports_plain_text_with_target_media_and_geometry() {
        let source = text_document(24, "A&B <wire>");
        let bytes = export(&source, LbxPrinter::PtE550w).unwrap();
        let mut zip = ZipArchive::new(Cursor::new(&bytes)).unwrap();
        assert!(zip.by_name("prop.xml").is_ok());
        let mut xml = String::new();
        zip.by_name("label.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        let parsed = roxmltree::Document::parse(&xml).unwrap();
        let paper = parsed
            .descendants()
            .find(|node| node.is_element() && node.tag_name().name() == "paper")
            .unwrap();
        assert_eq!(paper.attribute("printerID"), Some("26160"));
        assert_eq!(paper.attribute("format"), Some("261"));
        let imported = import(&bytes).unwrap();
        let object = imported.document.elements[0].as_object().unwrap();
        assert!((object.frame.x_mm - 3.0).abs() < 0.0002);
        assert!((object.frame.y_mm - 2.0).abs() < 0.0002);
        assert!((object.frame.width_mm - 20.0).abs() < 0.0002);
        let ObjectContent::Text { content, .. } = &object.content else {
            panic!("expected exported text object");
        };
        assert_eq!(content, "A&B <wire>");
    }

    #[test]
    fn export_rejects_media_not_mapped_for_the_selected_target() {
        assert!(export(&text_document(36, "wide"), LbxPrinter::PtE550w).is_err());
        let bytes = export(&text_document(36, "wide"), LbxPrinter::PtP900w).unwrap();
        let imported = import(&bytes).unwrap();
        assert_eq!(imported.document.tape_width_mm, 36);
    }

    #[test]
    fn exports_raster_images_as_embedded_archive_resources() {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(crate::bitmap::LabelBitmap::new(4, 4).to_rgba_image())
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let png_bytes = png.into_inner();
        let frame = TextBox {
            width_mm: 12.0,
            height_mm: 8.0,
            position: PositionMode::Absolute,
            x_mm: 0.0,
            y_mm: 0.0,
            gap_mm: 0.0,
            vertical_align: VerticalAlign::Center,
        };
        let mut document = text_document(24, "placeholder");
        document.elements = vec![LabelElement::Object(LabelObject::image_from_bytes(
            None,
            png_bytes.clone(),
            frame,
        ))];
        let bytes = export(&document, LbxPrinter::PtE550w).unwrap();
        let mut archive = ZipArchive::new(Cursor::new(&bytes)).unwrap();
        assert!(archive.by_name("assets/object-0.png").is_ok());
        let imported = import(&bytes).unwrap();
        assert!(matches!(
            &imported.document.elements[0],
            LabelElement::Object(LabelObject {
                content: ObjectContent::Image { image_data, .. },
                ..
            }) if image_data == &png_bytes
        ));
    }

    #[test]
    fn exports_editable_qr_as_native_lbx_barcode() {
        let mut document = text_document(24, "unused");
        document.elements = vec![LabelElement::Object(LabelObject::qr_code(
            "https://example.test/?a=1&b=2",
            TextBox {
                width_mm: 17.0,
                height_mm: 17.0,
                position: PositionMode::Absolute,
                x_mm: 2.0,
                y_mm: 1.0,
                gap_mm: 0.0,
                vertical_align: VerticalAlign::Center,
            },
        ))];
        let bytes = export(&document, LbxPrinter::PtE550w).unwrap();
        let mut zip = ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut xml = String::new();
        zip.by_name("label.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        assert!(xml.contains("<barcode:barcode>"));
        assert!(xml.contains("protocol=\"QRCODE\""));
        assert!(xml.contains("eccLevel=\"15%\""));
        assert!(xml.contains("https://example.test/?a=1&amp;b=2"));
        assert!(!xml.contains("image:imageStyle"));
    }

    #[test]
    fn exports_editable_code39_as_a_native_lbx_barcode() {
        let mut object = LabelObject::code39(
            "ABC-123",
            TextBox {
                width_mm: 45.0,
                height_mm: 12.0,
                ..TextBox::default()
            },
        );
        if let ObjectContent::Code39 {
            bar_width_pt,
            check_digit,
            human_readable,
            ..
        } = &mut object.content
        {
            *bar_width_pt = 1.6;
            *check_digit = true;
            *human_readable = false;
        }
        let mut document = text_document(24, "");
        document.elements = vec![LabelElement::Object(object)];
        let exported = export(&document, LbxPrinter::PtE550w).unwrap();
        let mut zip = ZipArchive::new(Cursor::new(exported)).unwrap();
        let mut xml = String::new();
        zip.by_name("label.xml")
            .unwrap()
            .read_to_string(&mut xml)
            .unwrap();
        assert!(xml.contains("protocol=\"CODE39\""));
        assert!(xml.contains("barWidth=\"1.6pt\""));
        assert!(xml.contains("checkDigit=\"true\""));
        assert!(xml.contains("humanReadable=\"false\""));
        assert!(xml.contains("<pt:data>ABC-123</pt:data>"));
    }

    #[test]
    fn export_rejects_content_that_would_be_silently_lost() {
        let mut document = text_document(24, "rich");
        let LabelElement::Object(object) = &mut document.elements[0] else {
            unreachable!();
        };
        if let ObjectContent::Text { styles, .. } = &mut object.content {
            styles.push(StyleSpan {
                start: 0,
                end: 4,
                style: TextStyle {
                    bold: true,
                    ..TextStyle::default()
                },
            });
        }
        assert!(
            export(&document, LbxPrinter::PtE550w)
                .unwrap_err()
                .to_string()
                .contains("inline text styles")
        );
    }

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
            "no supported text, PT Dingbats symbols, QR codes, untrimmed images, or unfilled vector objects"
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
    fn imports_supported_code39_as_an_editable_native_object() {
        let xml = format!(
            "{PREFIX}<barcode:barcode><pt:objectStyle x=\"2pt\" y=\"1pt\" width=\"72pt\" height=\"24pt\" angle=\"0\"/><barcode:barcodeStyle protocol=\"CODE39\" lengths=\"7\" barWidth=\"1.6pt\" barRatio=\"1:3\" humanReadable=\"true\" humanReadableAlignment=\"CENTER\" checkDigit=\"true\"/><pt:data>ABC-123</pt:data></barcode:barcode>{SUFFIX}"
        );
        let result = import(&archive(&xml)).unwrap();
        assert!(result.warnings.is_empty());
        assert!(matches!(
            &result.document.elements[0].as_object().unwrap().content,
            ObjectContent::Code39 {
                data,
                bar_width_pt,
                check_digit: true,
                human_readable: true,
            } if data == "ABC-123" && *bar_width_pt == 1.6
        ));
        let serialized = result.document.to_toml_string().unwrap();
        assert!(serialized.contains("version = 9"));
        let restored = LabelDocument::from_toml_str(&serialized).unwrap();
        let exported = export(&restored, LbxPrinter::PtE550w).unwrap();
        let mut zip = ZipArchive::new(Cursor::new(exported)).unwrap();
        let mut exported_xml = String::new();
        zip.by_name("label.xml")
            .unwrap()
            .read_to_string(&mut exported_xml)
            .unwrap();
        assert!(exported_xml.contains("protocol=\"CODE39\""));
        assert!(exported_xml.contains("barWidth=\"1.6pt\""));
        assert!(exported_xml.contains("checkDigit=\"true\""));
    }

    #[test]
    fn skips_code39_settings_that_the_native_model_cannot_preserve() {
        for style in [
            "barRatio=\"1:2\" humanReadableAlignment=\"CENTER\"",
            "barRatio=\"1:3\" humanReadableAlignment=\"LEFT\"",
        ] {
            let xml = format!(
                "{PREFIX}<barcode:barcode><pt:objectStyle x=\"0pt\" y=\"0pt\" width=\"58pt\" height=\"20pt\"/><barcode:barcodeStyle protocol=\"CODE39\" {style}/><pt:data>ABC123</pt:data></barcode:barcode>{SUFFIX}"
            );
            let error = import(&archive(&xml)).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("unsupported bar ratio or readable-text alignment")
            );
        }
    }

    #[test]
    fn imports_model_2_qr_from_editor_barcode_tags() {
        let xml = format!(
            "{PREFIX}<barcode:barcode><pt:objectStyle x=\"0pt\" y=\"0pt\" width=\"58pt\" height=\"58pt\" angle=\"0\"/><barcode:barcodeStyle protocol=\"QRCODE\" margin=\"false\"/><pt:data>http://alecto.ca</pt:data><barcode:qrcodeStyle model=\"2\" eccLevel=\"15%\" cellSize=\"2.0pt\" mbcs=\"932\" removeCharKind=\"0\" removeCharString=\"\" joint=\"1\" jointSpace=\"8\" jointVertically=\"false\" version=\"auto\"/></barcode:barcode>{SUFFIX}"
        );
        let result = import(&archive(&xml)).unwrap();
        assert_eq!(result.document.elements.len(), 1);
        assert!(result.warnings.is_empty());
        let Some(LabelElement::Object(object)) = result.document.elements.first() else {
            panic!("expected native QR object");
        };
        assert!(matches!(
            &object.content,
            ObjectContent::QrCode {
                data,
                error_correction: crate::barcode::QrErrorCorrection::Medium,
                version: None,
            } if data == "http://alecto.ca"
        ));
        let content = object.content_box.as_ref().unwrap();
        assert!((content.width_mm - 58.0 * PT_TO_MM).abs() < 0.001);
        assert!((content.height_mm - 58.0 * PT_TO_MM).abs() < 0.001);
        let rendered = crate::layout::render_document(
            &result.document,
            256,
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
                .any(|pixel| *pixel != 0)
        );
    }

    #[test]
    fn imports_fixed_qr_version_and_correction_as_native_fields() {
        let xml = format!(
            "{PREFIX}<barcode:barcode><pt:objectStyle x=\"0pt\" y=\"0pt\" width=\"120pt\" height=\"60pt\" angle=\"0\"/><barcode:barcodeStyle protocol=\"QRCODE\" margin=\"false\"/><pt:data>FIXED QR</pt:data><barcode:qrcodeStyle model=\"2\" eccLevel=\"25%\" cellSize=\"1.25pt\" mbcs=\"932\" removeCharKind=\"0\" removeCharString=\"\" joint=\"1\" jointSpace=\"8\" jointVertically=\"false\" version=\"5\"/></barcode:barcode>{SUFFIX}"
        );
        let result = import(&archive(&xml)).unwrap();
        assert!(result.warnings.is_empty());
        let object = result.document.elements[0].as_object().unwrap();
        assert!(matches!(
            &object.content,
            ObjectContent::QrCode {
                data,
                error_correction: crate::barcode::QrErrorCorrection::Quartile,
                version: Some(5),
            } if data == "FIXED QR"
        ));
        let qr_size_pt = (37.0 + 8.0) * 1.25;
        let content = object.content_box.as_ref().unwrap();
        assert!((content.width_mm - qr_size_pt * PT_TO_MM).abs() < 0.001);
        assert!((content.height_mm - qr_size_pt * PT_TO_MM).abs() < 0.001);
        let serialized = result.document.to_toml_string().unwrap();
        assert!(serialized.contains("version = 9"));
        let restored = LabelDocument::from_toml_str(&serialized).unwrap();
        assert!(matches!(
            &restored.elements[0].as_object().unwrap().content,
            ObjectContent::QrCode {
                version: Some(5),
                ..
            }
        ));
        let exported = export(&restored, LbxPrinter::PtE550w).unwrap();
        let mut zip = ZipArchive::new(Cursor::new(exported)).unwrap();
        let mut exported_xml = String::new();
        zip.by_name("label.xml")
            .unwrap()
            .read_to_string(&mut exported_xml)
            .unwrap();
        assert!(exported_xml.contains("version=\"5\""));
        assert!(exported_xml.contains("cellSize=\"1.250pt\""));
    }

    #[test]
    fn skips_non_qr_barcodes_with_an_explicit_protocol_warning() {
        let xml = format!(
            "{PREFIX}<barcode:barcode><pt:objectStyle x=\"0pt\" y=\"0pt\" width=\"58pt\" height=\"20pt\"/><barcode:barcodeStyle protocol=\"CODE128\"/><pt:data>ABC123</pt:data></barcode:barcode><text:text><pt:objectStyle x=\"0pt\" y=\"0pt\" width=\"20pt\" height=\"12pt\"/><pt:data>Text</pt:data></text:text>{SUFFIX}"
        );
        let result = import(&archive(&xml)).unwrap();
        assert_eq!(result.document.elements.len(), 1);
        assert!(result.warnings.iter().any(|warning| {
            warning.contains("protocol 'CODE128'; only Code 39 and QR codes are imported")
        }));
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
