// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Label document model and shared element rendering.
//!
//! A [`LabelDocument`] is the serializable description of a label: global tape
//! and font settings plus an ordered list of [`LabelElement`]s. The same model
//! drives both the GUI canvas and command-line rendering, so a design composed
//! in one can be reproduced exactly in the other.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::path::PathBuf;

use log::error;
use serde::{Deserialize, Serialize};

use crate::RenderError;
use crate::Result;
use crate::bitmap::LabelBitmap;
use crate::compose;
use crate::image_loader::{self, ImageLoadOptions};
use crate::text::{TextAlign, TextRenderer};

/// Current on-disk layout format version.
pub const DOCUMENT_VERSION: u32 = 1;

/// A complete label design: global settings plus an ordered element list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabelDocument {
    /// Layout format version. See [`DOCUMENT_VERSION`].
    pub version: u32,
    /// Tape width in millimeters the design was created for.
    pub tape_width_mm: u8,
    /// Font family name used for text elements.
    pub font_name: String,
    /// Font top/bottom margin in pixels.
    pub font_margin: u32,
    /// Elements in composition order (left to right).
    pub elements: Vec<LabelElement>,
}

impl LabelDocument {
    /// Serialize the document to a TOML string.
    pub fn to_toml_string(&self) -> Result<String> {
        Ok(toml::to_string(self)?)
    }

    /// Parse a document from a TOML string.
    ///
    /// Rejects an unsupported format version and decodes every embedded image
    /// into its render cache, so the returned document is ready to render.
    pub fn from_toml_str(text: &str) -> Result<Self> {
        let mut doc: LabelDocument = toml::from_str(text)?;
        if doc.version == 0 || doc.version > DOCUMENT_VERSION {
            return Err(RenderError::Layout(format!(
                "unsupported layout version {} (this build supports up to {})",
                doc.version, DOCUMENT_VERSION
            )));
        }
        doc.decode_image_caches()?;
        Ok(doc)
    }

    /// Decode every image element's embedded bytes into its render cache.
    ///
    /// Returns an error if any embedded image cannot be decoded, so a corrupt
    /// layout fails loudly rather than rendering blank.
    pub fn decode_image_caches(&mut self) -> Result<()> {
        for element in &mut self.elements {
            if let LabelElement::Image {
                image_data, bitmap, ..
            } = element
                && bitmap.is_none()
                && !image_data.is_empty()
            {
                let decoded = image_loader::load_image_from_reader(
                    Cursor::new(image_data.as_slice()),
                    &ImageLoadOptions::default(),
                )?;
                *bitmap = Some(decoded);
            }
        }
        Ok(())
    }

    /// Collect the unique `{{name}}` placeholder names used in text elements.
    ///
    /// The result is sorted for stable display and de-duplication.
    pub fn placeholders(&self) -> Vec<String> {
        let mut names = BTreeSet::new();
        for element in &self.elements {
            if let LabelElement::Text { content, .. } = element {
                collect_placeholder_names(content, &mut names);
            }
        }
        names.into_iter().collect()
    }

    /// Replace `{{name}}` placeholders in all text elements using `values`.
    ///
    /// A placeholder with no matching value is replaced with an empty string;
    /// callers that want to reject missing values should check
    /// [`LabelDocument::placeholders`] against the provided keys first.
    pub fn apply_values(&mut self, values: &BTreeMap<String, String>) {
        for element in &mut self.elements {
            if let LabelElement::Text { content, .. } = element {
                *content = substitute_placeholders(content, |name| values.get(name).cloned());
            }
        }
    }
}

/// True if `name` is a valid placeholder identifier: non-empty and made of
/// ASCII letters, digits, or underscores.
fn is_valid_placeholder(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Collect placeholder names from `content` into `names`.
fn collect_placeholder_names(content: &str, names: &mut BTreeSet<String>) {
    let mut rest = content;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break;
        };
        let name = after[..end].trim();
        if is_valid_placeholder(name) {
            names.insert(name.to_string());
        }
        rest = &after[end + 2..];
    }
}

/// Replace every valid `{{name}}` in `content`. `lookup` returns the value for a
/// name, or `None` to substitute an empty string. Text that is not a valid
/// placeholder is left untouched.
fn substitute_placeholders(content: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut result = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            break;
        };
        let name = after[..end].trim();
        if is_valid_placeholder(name) {
            result.push_str(&rest[..start]);
            if let Some(value) = lookup(name) {
                result.push_str(&value);
            }
            rest = &after[end + 2..];
        } else {
            // Not a placeholder; keep the literal "{{" and continue past it.
            result.push_str(&rest[..start + 2]);
            rest = after;
        }
    }
    result.push_str(rest);
    result
}

/// A single element in the label composition.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LabelElement {
    /// A text block with content, optional font size, alignment, and rotation.
    Text {
        /// Text content; newlines separate lines.
        content: String,
        /// Explicit font size in points. `None` means auto-fit to tape height.
        #[serde(skip_serializing_if = "Option::is_none")]
        font_size: Option<f32>,
        /// Horizontal alignment of the text block.
        align: TextAlign,
        /// Rotation angle in degrees (clockwise). 0.0 = horizontal.
        rotation: f32,
    },
    /// An image embedded as its original source bytes.
    Image {
        /// Original file path, kept for display only. Never required to exist.
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<PathBuf>,
        /// Source image bytes (PNG/JPEG/SVG/...), stored as base64 in the file.
        #[serde(with = "crate::base64_bytes")]
        image_data: Vec<u8>,
        /// Decoded render cache; rebuilt from `image_data`, never serialized.
        #[serde(skip)]
        bitmap: Option<LabelBitmap>,
        /// Rotation angle in degrees (clockwise). 0.0 = horizontal.
        rotation: f32,
        /// Target height in pixels. `None` = auto (fit to tape height).
        #[serde(skip_serializing_if = "Option::is_none")]
        target_height: Option<u32>,
    },
    /// A cut mark separator.
    CutMark,
    /// Horizontal padding in pixels.
    Padding {
        /// Padding width in pixels.
        pixels: u32,
    },
}

impl LabelElement {
    /// Build an image element from decoded source bytes, decoding a render
    /// cache eagerly so previews and rendering are fast.
    pub fn image_from_bytes(path: Option<PathBuf>, image_data: Vec<u8>) -> Self {
        let bitmap = image_loader::load_image_from_reader(
            Cursor::new(&image_data),
            &ImageLoadOptions::default(),
        )
        .ok();
        LabelElement::Image {
            path,
            image_data,
            bitmap,
            rotation: 0.0,
            target_height: None,
        }
    }

    /// Set the decoded render cache for an image element.
    pub fn set_image_bitmap(&mut self, decoded: Option<LabelBitmap>) {
        if let LabelElement::Image { bitmap, .. } = self {
            *bitmap = decoded;
        }
    }

    /// Returns a short display name for the element list.
    pub fn display_name(&self) -> String {
        match self {
            LabelElement::Text { content, .. } => {
                let preview: String = content.chars().take(20).collect();
                if content.chars().count() > 20 {
                    format!("Text: {}...", preview)
                } else {
                    format!("Text: {}", preview)
                }
            }
            LabelElement::Image { path, .. } => {
                let name = path
                    .as_ref()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "embedded".to_string());
                format!("Image: {}", name)
            }
            LabelElement::CutMark => "Cut Mark".to_string(),
            LabelElement::Padding { pixels } => format!("Padding: {} px", pixels),
        }
    }
}

/// Font settings shared across all text elements in a document.
struct FontSettings<'a> {
    /// Font family name.
    name: &'a str,
    /// Top/bottom margin in pixels.
    margin: u32,
}

/// Render an ordered element list into a single label bitmap.
///
/// Elements are rendered to `tape_width_px` tall segments and concatenated left
/// to right. Per-element rendering failures (empty text, undecodable image) are
/// logged and skipped rather than aborting the whole label. Returns `None` when
/// no element produced any output.
pub fn render_elements(
    elements: &[LabelElement],
    tape_width_px: u32,
    font_name: &str,
    font_margin: u32,
    renderer: &mut TextRenderer,
) -> Result<Option<LabelBitmap>> {
    let mut result: Option<LabelBitmap> = None;

    for element in elements {
        let segment = match element {
            LabelElement::Text {
                content,
                font_size,
                align,
                rotation,
            } => match render_text_segment(
                renderer,
                content,
                *font_size,
                *align,
                *rotation,
                tape_width_px,
                &FontSettings {
                    name: font_name,
                    margin: font_margin,
                },
            ) {
                Some(seg) => seg,
                None => continue,
            },
            LabelElement::Image {
                image_data,
                bitmap,
                rotation,
                target_height,
                ..
            } => match render_image_segment(
                bitmap.as_ref(),
                image_data,
                *rotation,
                *target_height,
                tape_width_px,
            ) {
                Some(seg) => seg,
                None => continue,
            },
            LabelElement::CutMark => compose::cutmark(tape_width_px),
            LabelElement::Padding { pixels } => compose::padding(tape_width_px, *pixels),
        };

        result = Some(match result {
            Some(prev) => prev.append(&segment),
            None => segment,
        });
    }

    Ok(result)
}

/// Render a single text element to a tape-height bitmap, or `None` to skip it.
fn render_text_segment(
    renderer: &mut TextRenderer,
    content: &str,
    font_size: Option<f32>,
    align: TextAlign,
    rotation: f32,
    tape_width_px: u32,
    font: &FontSettings,
) -> Option<LabelBitmap> {
    if content.is_empty() {
        return None;
    }
    let lines: Vec<&str> = content.lines().collect();
    let is_rotated = is_rotated(rotation);

    // For rotated text with auto size, pick a size whose rotated bounding box
    // fits within the tape height.
    let effective_font_size = rotation_aware_font_size(font_size, rotation, &lines, tape_width_px);

    // Rotated text needs a taller render area so all lines stay visible; the
    // height becomes tape length after rotation.
    let render_height = if is_rotated {
        if let Some(fs) = effective_font_size {
            let line_h = (fs * 1.2).ceil();
            let text_h = (lines.len() as f32 * line_h).ceil() as u32 + font.margin * 2;
            text_h.max(tape_width_px)
        } else {
            tape_width_px
        }
    } else {
        tape_width_px
    };

    let bmp = match renderer.render_text(
        &lines,
        render_height,
        font.name,
        effective_font_size,
        font.margin,
        align,
    ) {
        Ok(bmp) => bmp,
        Err(e) => {
            error!("Text render failed: {}", e);
            return None;
        }
    };

    if is_rotated {
        // Trim whitespace so the rotated bounding box reflects actual content,
        // not full tape-height padding.
        Some(
            bmp.trim_vertical()
                .rotate(rotation)
                .fit_height(tape_width_px),
        )
    } else {
        Some(bmp)
    }
}

/// Render a single image element to a tape-height bitmap, or `None` to skip it.
fn render_image_segment(
    cached: Option<&LabelBitmap>,
    image_data: &[u8],
    rotation: f32,
    target_height: Option<u32>,
    tape_width_px: u32,
) -> Option<LabelBitmap> {
    let bmp = if let Some(bmp) = cached {
        bmp.clone()
    } else if !image_data.is_empty() {
        match image_loader::load_image_from_reader(
            Cursor::new(image_data),
            &ImageLoadOptions::default(),
        ) {
            Ok(bmp) => bmp,
            Err(e) => {
                error!("Image decode failed: {}", e);
                return None;
            }
        }
    } else {
        error!("Image element has no data");
        return None;
    };

    // Scale to target height (auto = tape height, manual = specified).
    let bmp = bmp.scale_to_height(target_height.unwrap_or(tape_width_px));

    if is_rotated(rotation) {
        Some(bmp.rotate(rotation).fit_height(tape_width_px))
    } else {
        Some(bmp.fit_height(tape_width_px))
    }
}

/// Returns true if the angle is not effectively a multiple of 360 degrees.
fn is_rotated(rotation_deg: f32) -> bool {
    let norm = ((rotation_deg % 360.0) + 360.0) % 360.0;
    !(norm.abs() < 0.5 || (norm - 360.0).abs() < 0.5)
}

/// Calculate a font size that fits within `tape_height` after rotation.
///
/// For 0 degrees, returns `font_size` unchanged (`None` lets the renderer
/// auto-size). For other angles, estimates the largest font size whose rotated
/// bounding box fits within the tape height.
fn rotation_aware_font_size(
    font_size: Option<f32>,
    rotation_deg: f32,
    lines: &[&str],
    tape_height: u32,
) -> Option<f32> {
    // User-specified font size: use it directly, no auto-adjustment.
    if font_size.is_some() {
        return font_size;
    }
    if !is_rotated(rotation_deg) {
        return None;
    }

    let norm = ((rotation_deg % 360.0) + 360.0) % 360.0;
    let angle_rad = norm.to_radians();
    let sin_a = angle_rad.sin().abs();
    let cos_a = angle_rad.cos().abs();
    let num_lines = lines.len().max(1) as f32;
    let max_chars = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1) as f32;
    let available = tape_height as f32;

    // Rotated bounding-box height:
    //   bbox_h = text_width * |sin| + text_height * |cos|
    // with text_width  ~ max_chars * font_size * 0.6
    //      text_height ~ num_lines * font_size * 1.2
    // Solve for font_size.
    let denom = max_chars * 0.6 * sin_a + num_lines * 1.2 * cos_a;
    if denom > 0.01 {
        Some((available / denom).max(4.0))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Encode a solid black image of the given size as PNG bytes.
    fn png_bytes(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255]));
        let mut buf = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new(Cursor::new(&mut buf));
        image::ImageEncoder::write_image(
            encoder,
            img.as_raw(),
            w,
            h,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
        buf
    }

    #[test]
    fn test_empty_elements_returns_none() {
        let mut renderer = TextRenderer::new();
        let result = render_elements(&[], 64, "", 0, &mut renderer).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_padding_and_cutmark_dimensions() {
        let mut renderer = TextRenderer::new();
        let elements = vec![LabelElement::Padding { pixels: 10 }, LabelElement::CutMark];
        let bmp = render_elements(&elements, 64, "", 0, &mut renderer)
            .unwrap()
            .unwrap();
        assert_eq!(bmp.height(), 64);
        // Padding (10) followed by the 9-pixel cut mark.
        assert_eq!(bmp.width(), 19);
    }

    #[test]
    fn test_empty_text_is_skipped() {
        let mut renderer = TextRenderer::new();
        let elements = vec![LabelElement::Text {
            content: String::new(),
            font_size: Some(24.0),
            align: TextAlign::Left,
            rotation: 0.0,
        }];
        let result = render_elements(&elements, 64, "", 0, &mut renderer).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_image_element_fits_tape_height() {
        let mut renderer = TextRenderer::new();
        let element = LabelElement::image_from_bytes(None, png_bytes(40, 40));
        let bmp = render_elements(&[element], 64, "", 0, &mut renderer)
            .unwrap()
            .unwrap();
        assert_eq!(bmp.height(), 64);
        assert!(bmp.width() > 0);
    }

    #[test]
    fn test_rotated_image_fits_tape_height() {
        let mut renderer = TextRenderer::new();
        let element = match LabelElement::image_from_bytes(None, png_bytes(40, 20)) {
            LabelElement::Image {
                path,
                image_data,
                bitmap,
                target_height,
                ..
            } => LabelElement::Image {
                path,
                image_data,
                bitmap,
                rotation: 90.0,
                target_height,
            },
            other => other,
        };
        let bmp = render_elements(&[element], 64, "", 0, &mut renderer)
            .unwrap()
            .unwrap();
        assert_eq!(bmp.height(), 64);
    }

    /// Build a small document containing one of each element kind.
    fn sample_document() -> LabelDocument {
        LabelDocument {
            version: DOCUMENT_VERSION,
            tape_width_mm: 12,
            font_name: "DejaVuSans".into(),
            font_margin: 2,
            elements: vec![
                LabelElement::Text {
                    content: "Hi".into(),
                    font_size: Some(24.0),
                    align: TextAlign::Center,
                    rotation: 0.0,
                },
                LabelElement::image_from_bytes(Some("logo.png".into()), png_bytes(8, 8)),
                LabelElement::CutMark,
                LabelElement::Padding { pixels: 20 },
            ],
        }
    }

    #[test]
    fn test_document_round_trip_all_kinds() {
        let doc = sample_document();
        let text = doc.to_toml_string().unwrap();
        let parsed = LabelDocument::from_toml_str(&text).unwrap();

        assert_eq!(parsed.elements.len(), 4);
        assert_eq!(parsed.tape_width_mm, 12);
        assert_eq!(parsed.font_name, "DejaVuSans");

        // Embedded image bytes survive the round-trip exactly.
        match (&doc.elements[1], &parsed.elements[1]) {
            (
                LabelElement::Image { image_data: a, .. },
                LabelElement::Image {
                    image_data: b,
                    bitmap,
                    ..
                },
            ) => {
                assert_eq!(a, b);
                // The render cache is rebuilt on load.
                assert!(bitmap.is_some());
            }
            _ => panic!("expected image elements"),
        }
    }

    #[test]
    fn test_cutmark_serializes_as_tagged_table() {
        let doc = sample_document();
        let text = doc.to_toml_string().unwrap();
        assert!(text.contains("cut_mark"));
    }

    #[test]
    fn test_newer_version_is_rejected() {
        let doc = LabelDocument {
            version: DOCUMENT_VERSION + 1,
            tape_width_mm: 12,
            font_name: "x".into(),
            font_margin: 0,
            elements: vec![LabelElement::CutMark],
        };
        let text = doc.to_toml_string().unwrap();
        assert!(LabelDocument::from_toml_str(&text).is_err());
    }

    #[test]
    fn test_corrupt_image_data_is_rejected() {
        let doc = LabelDocument {
            version: DOCUMENT_VERSION,
            tape_width_mm: 12,
            font_name: "x".into(),
            font_margin: 0,
            elements: vec![LabelElement::Image {
                path: None,
                image_data: b"not a real image".to_vec(),
                bitmap: None,
                rotation: 0.0,
                target_height: None,
            }],
        };
        let text = doc.to_toml_string().unwrap();
        assert!(LabelDocument::from_toml_str(&text).is_err());
    }

    #[test]
    fn test_invalid_base64_is_rejected() {
        let text = "version = 1\ntape_width_mm = 12\nfont_name = \"x\"\n\
                    font_margin = 0\n\n[[elements]]\ntype = \"image\"\n\
                    image_data = \"not valid base64 !!!\"\nrotation = 0.0\n";
        assert!(LabelDocument::from_toml_str(text).is_err());
    }

    fn text_doc(content: &str) -> LabelDocument {
        LabelDocument {
            version: DOCUMENT_VERSION,
            tape_width_mm: 12,
            font_name: "x".into(),
            font_margin: 0,
            elements: vec![LabelElement::Text {
                content: content.into(),
                font_size: Some(24.0),
                align: TextAlign::Left,
                rotation: 0.0,
            }],
        }
    }

    fn text_of(doc: &LabelDocument) -> &str {
        match &doc.elements[0] {
            LabelElement::Text { content, .. } => content,
            _ => panic!("expected text"),
        }
    }

    #[test]
    fn test_placeholders_unique_sorted() {
        let doc = text_doc("{{name}} {{id}}\n{{name}}");
        assert_eq!(
            doc.placeholders(),
            vec!["id".to_string(), "name".to_string()]
        );
    }

    #[test]
    fn test_placeholders_ignore_invalid() {
        // Spaces make it not a placeholder; it stays literal and is not listed.
        let doc = text_doc("{{ not a var }} {{ok}}");
        assert_eq!(doc.placeholders(), vec!["ok".to_string()]);
    }

    #[test]
    fn test_apply_values_substitutes() {
        let mut doc = text_doc("Hi {{name}} ({{id}})");
        let mut values = BTreeMap::new();
        values.insert("name".to_string(), "Alice".to_string());
        values.insert("id".to_string(), "A001".to_string());
        doc.apply_values(&values);
        assert_eq!(text_of(&doc), "Hi Alice (A001)");
    }

    #[test]
    fn test_apply_values_blanks_unresolved() {
        let mut doc = text_doc("Hi {{name}}!");
        doc.apply_values(&BTreeMap::new());
        assert_eq!(text_of(&doc), "Hi !");
    }

    #[test]
    fn test_apply_values_trims_and_keeps_literals() {
        let mut doc = text_doc("{{ name }} {{bad var}}");
        let mut values = BTreeMap::new();
        values.insert("name".to_string(), "Bob".to_string());
        doc.apply_values(&values);
        // Whitespace inside braces is tolerated; the invalid one stays literal.
        assert_eq!(text_of(&doc), "Bob {{bad var}}");
    }

    #[test]
    fn test_unterminated_placeholder_left_intact() {
        let mut doc = text_doc("price {{name");
        doc.apply_values(&BTreeMap::new());
        assert_eq!(text_of(&doc), "price {{name");
    }

    #[test]
    fn test_image_from_bytes_decodes_cache() {
        let element = LabelElement::image_from_bytes(Some("logo.png".into()), png_bytes(8, 8));
        match element {
            LabelElement::Image { bitmap, path, .. } => {
                assert!(bitmap.is_some());
                assert_eq!(path.unwrap().file_name().unwrap(), "logo.png");
            }
            _ => panic!("expected image element"),
        }
    }
}
