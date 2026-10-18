// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Text rendering using cosmic-text.
//!
//! Renders multi-line text into a [`LabelBitmap`] using system fonts.
//! Supports auto-sizing, alignment, and font selection.

use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Wrap,
};
use serde::{Deserialize, Serialize};

use crate::RenderError;
use crate::Result;
use crate::bitmap::LabelBitmap;

/// Historical baseline distance, as a multiple of the font size.
pub const DEFAULT_LINE_SPACING: f32 = 1.2;
pub const LINE_SPACING_RANGE: std::ops::RangeInclusive<f32> = 0.5..=3.0;

pub(crate) fn default_line_spacing() -> f32 {
    DEFAULT_LINE_SPACING
}
pub(crate) fn is_default_line_spacing(value: &f32) -> bool {
    *value == DEFAULT_LINE_SPACING
}
pub(crate) fn validate_line_spacing(value: f32) -> Result<()> {
    if !value.is_finite() || !LINE_SPACING_RANGE.contains(&value) {
        return Err(RenderError::Layout(
            "line spacing must be between 0.5 and 3.0 times the font size".into(),
        ));
    }
    Ok(())
}

/// Text alignment within the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextAlign {
    /// Align text to the left edge.
    Left,
    /// Center text horizontally.
    Center,
    /// Align text to the right edge.
    Right,
}

impl TextAlign {
    /// Convert to cosmic-text's alignment type.
    fn to_cosmic(self) -> Align {
        match self {
            TextAlign::Left => Align::Left,
            TextAlign::Center => Align::Center,
            TextAlign::Right => Align::Right,
        }
    }
}

/// Text renderer backed by cosmic-text.
pub struct TextRenderer {
    font_system: FontSystem,
    swash_cache: SwashCache,
}

impl Default for TextRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl TextRenderer {
    /// Create a new text renderer with system fonts loaded.
    pub fn new() -> Self {
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
        }
    }

    /// Render a complete, unwrapped text block without tape-height clipping.
    /// The tight ink bounds include glyph bearings and preserve the alignment
    /// of explicit newline-separated lines. Used before box rotation/fitting.
    pub(crate) fn render_unbounded(
        &mut self,
        text: &str,
        font_name: &str,
        font_size_px: f32,
        align: TextAlign,
        line_spacing: f32,
    ) -> Result<LabelBitmap> {
        self.render_unbounded_styled(
            text,
            font_name,
            font_size_px,
            align,
            line_spacing,
            &[],
            12.0,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_unbounded_styled(
        &mut self,
        text: &str,
        font_name: &str,
        font_size_px: f32,
        align: TextAlign,
        line_spacing: f32,
        styles: &[crate::styled_text::StyleSpan],
        base_size_pt: f32,
    ) -> Result<LabelBitmap> {
        validate_line_spacing(line_spacing)?;
        crate::styled_text::validate_spans(text, styles)?;
        if !font_size_px.is_finite() || font_size_px <= 0.0 || font_size_px > 4096.0 {
            return Err(RenderError::Text(
                "font size must be in (0, 4096] pixels".into(),
            ));
        }
        if text.trim().is_empty() {
            return Ok(LabelBitmap::new(1, 1));
        }
        let family = if font_name.is_empty() {
            Family::SansSerif
        } else {
            Family::Name(font_name)
        };
        let attrs = Attrs::new().family(family);
        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(font_size_px, (font_size_px * line_spacing).ceil()),
        );
        buffer.set_wrap(Wrap::None);
        buffer.set_size(None, None);
        if styles.is_empty() {
            buffer.set_text(text, &attrs, Shaping::Advanced, Some(align.to_cosmic()));
        } else {
            let mut boundaries: Vec<usize> = text.char_indices().map(|(byte, _)| byte).collect();
            boundaries.push(text.len());
            let regions = crate::styled_text::style_regions(text, styles);
            let mut spans = Vec::with_capacity(regions.len());
            for (range, style) in &regions {
                let mut attr = attrs.clone();
                if let Some(name) = &style.font_name {
                    attr = attr.family(Family::Name(name));
                }
                attr = attr
                    .weight(if style.bold {
                        cosmic_text::Weight::BOLD
                    } else {
                        cosmic_text::Weight::NORMAL
                    })
                    .style(if style.italic {
                        cosmic_text::Style::Italic
                    } else {
                        cosmic_text::Style::Normal
                    });
                if style.underline {
                    attr = attr.underline(cosmic_text::UnderlineStyle::Single);
                }
                if style.strikethrough {
                    attr = attr.strikethrough();
                }
                let size = font_size_px * style.font_size.unwrap_or(base_size_pt) / base_size_pt;
                if !size.is_finite() || size > 4096.0 {
                    return Err(RenderError::DimensionMismatch(
                        "inline font raster is too large".into(),
                    ));
                }
                attr = attr.metrics(Metrics::new(size, (size * line_spacing).ceil()));
                spans.push((&text[boundaries[range.start]..boundaries[range.end]], attr));
            }
            buffer.set_rich_text(spans, &attrs, Shaping::Advanced, Some(align.to_cosmic()));
        }
        buffer.shape_until_scroll(&mut self.font_system, true);
        let width = buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0_f32, f32::max);
        let height = if styles.is_empty() {
            text.split('\n').count() as f32 * (font_size_px * line_spacing).ceil()
        } else {
            buffer
                .layout_runs()
                .map(|r| r.line_top + r.line_height)
                .fold(0.0_f32, f32::max)
        };
        if !width.is_finite() || width > 1_000_000.0 || width * height > 16_777_216.0 {
            return Err(RenderError::DimensionMismatch(
                "text raster is too large".into(),
            ));
        }
        // Reflow alignment within the actual longest line instead of an
        // arbitrary large measurement surface. Never wrap long port names.
        buffer.set_size(Some(width.ceil().max(1.0)), None);
        buffer.shape_until_scroll(&mut self.font_system, true);
        let mut bounds: Option<(i32, i32, i32, i32)> = None;
        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            Color::rgb(0, 0, 0),
            |x, y, w, h, color| {
                if color.a() >= 128 {
                    let right = x + w as i32;
                    let bottom = y + h as i32;
                    bounds = Some(match bounds {
                        Some((left, top, r, b)) => {
                            (left.min(x), top.min(y), r.max(right), b.max(bottom))
                        }
                        None => (x, y, right, bottom),
                    });
                }
            },
        );
        let Some((left, top, right, bottom)) = bounds else {
            if font_size_px < 4.0 {
                return Ok(LabelBitmap::new(1, 1));
            }
            return Err(RenderError::Text(
                "no visible glyphs; select an installed font".into(),
            ));
        };
        let width = (right - left) as u32;
        let height = (bottom - top) as u32;
        if u64::from(width) * u64::from(height) > 16_777_216 {
            return Err(RenderError::DimensionMismatch(
                "text raster is too large".into(),
            ));
        }
        let mut bitmap = LabelBitmap::new(width, height);
        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            Color::rgb(0, 0, 0),
            |x, y, w, h, color| {
                if color.a() >= 128 {
                    for dy in 0..h {
                        for dx in 0..w {
                            bitmap.set_pixel((x - left) as u32 + dx, (y - top) as u32 + dy, true);
                        }
                    }
                }
            },
        );
        Ok(bitmap)
    }

    #[cfg(test)]
    pub(crate) fn with_test_font() -> Self {
        let mut db = cosmic_text::fontdb::Database::new();
        db.load_font_data(
            include_bytes!("../../ptouch-gui/assets/fonts/DroidSansFallback.ttf").to_vec(),
        );
        db.set_sans_serif_family("Droid Sans Fallback");
        let font_system = FontSystem::new_with_locale_and_db("en-US".into(), db);
        Self {
            font_system,
            swash_cache: SwashCache::new(),
        }
    }

    /// Render one or more lines of text into a [`LabelBitmap`].
    ///
    /// - `lines`: text lines to render (joined with newlines)
    /// - `print_width`: height of the tape in pixels (this becomes the bitmap
    ///   height; called "print_width" for compatibility with the C API where
    ///   the tape width is the printable width)
    /// - `font_name`: font family name to use (e.g. "DejaVu Sans")
    /// - `font_size`: explicit font size in raster pixels (legacy behavior),
    ///   or `None` for auto-detect; physical boxes convert points separately
    /// - `font_margin`: margin in pixels on each side (top and bottom)
    /// - `align`: horizontal text alignment
    ///
    /// The renderer will:
    /// 1. If font_size is None, auto-detect the largest size that fits
    /// 2. Render the text into a buffer
    /// 3. Convert the rendered glyphs to a 1-bit bitmap
    pub fn render_text(
        &mut self,
        lines: &[&str],
        print_width: u32,
        font_name: &str,
        font_size: Option<f32>,
        font_margin: u32,
        align: TextAlign,
    ) -> Result<LabelBitmap> {
        if print_width == 0 {
            return Err(RenderError::Text("print_width must be > 0".into()));
        }

        let text = lines.join("\n");
        if text.is_empty() {
            return Err(RenderError::Text("no text to render".into()));
        }

        let num_lines = lines.len() as f32;
        let available_height = print_width.saturating_sub(font_margin * 2) as f32;

        if available_height <= 0.0 {
            return Err(RenderError::Text(
                "font_margin too large for tape width".into(),
            ));
        }

        // Determine font size
        let font_size = font_size.unwrap_or_else(|| {
            // Auto-detect: largest size where all lines fit vertically.
            // Line height ~ font_size * 1.2, total ~ line_height * num_lines
            let size = available_height / (num_lines * 1.2);
            size.max(4.0)
        });

        let line_height = (font_size * 1.2).ceil();
        let metrics = Metrics::new(font_size, line_height);

        let family = if font_name.is_empty() {
            Family::SansSerif
        } else {
            Family::Name(font_name)
        };
        let attrs = Attrs::new().family(family);
        let cosmic_align = Some(align.to_cosmic());

        // We do not know the final horizontal width yet. Use a large initial
        // width so cosmic-text does not wrap, then we measure the actual
        // extent and create a tight bitmap.
        let layout_width = 16384.0f32;

        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(Some(layout_width), Some(available_height));
        buffer.set_text(&text, &attrs, Shaping::Advanced, cosmic_align);
        buffer.shape_until_scroll(&mut self.font_system, true);

        // Measure tight horizontal extent (min_x..max_x) from layout runs.
        // With Center/Right alignment, cosmic-text offsets glyphs within the
        // large layout_width. We subtract min_x so the bitmap is tight.
        let mut min_x: f32 = f32::MAX;
        let mut max_x: f32 = 0.0;
        for run in buffer.layout_runs() {
            for g in run.glyphs.iter() {
                min_x = min_x.min(g.x);
                max_x = max_x.max(g.x + g.w);
            }
        }
        if min_x == f32::MAX {
            min_x = 0.0;
        }

        let bitmap_width = ((max_x - min_x).ceil() as u32).max(1);
        let bitmap_height = print_width;
        let x_offset = min_x.floor() as i32;

        let mut bitmap = LabelBitmap::new(bitmap_width, bitmap_height);

        // Vertical centering offset
        let total_text_height = (num_lines * line_height) as u32;
        let y_offset = if total_text_height < bitmap_height {
            ((bitmap_height - total_text_height) / 2) as i32
        } else {
            font_margin as i32
        };

        // Draw glyphs onto the bitmap
        let text_color = Color::rgb(0, 0, 0);
        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            text_color,
            |x, y, w, h, color| {
                let alpha = color.a();
                if alpha < 128 {
                    return;
                }
                let px = x - x_offset;
                let py = y + y_offset;
                for dy in 0..h as i32 {
                    for dx in 0..w as i32 {
                        let fx = px + dx;
                        let fy = py + dy;
                        if fx >= 0 && fy >= 0 {
                            bitmap.set_pixel(fx as u32, fy as u32, true);
                        }
                    }
                }
            },
        );

        Ok(bitmap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_text_align_conversion() {
        assert_eq!(TextAlign::Left.to_cosmic(), Align::Left);
        assert_eq!(TextAlign::Center.to_cosmic(), Align::Center);
        assert_eq!(TextAlign::Right.to_cosmic(), Align::Right);
    }

    #[test]
    fn test_empty_text_error() {
        let mut renderer = TextRenderer::new();
        let result = renderer.render_text(&[], 64, "sans-serif", None, 2, TextAlign::Center);
        assert!(result.is_err());
    }

    #[test]
    fn test_zero_width_error() {
        let mut renderer = TextRenderer::new();
        let result = renderer.render_text(&["hello"], 0, "sans-serif", None, 0, TextAlign::Center);
        assert!(result.is_err());
    }

    #[test]
    fn test_render_basic_text() {
        let mut renderer = TextRenderer::new();
        // Use a generic family so it works even without specific fonts
        let result = renderer.render_text(&["Test"], 64, "", Some(24.0), 2, TextAlign::Left);
        // This may succeed or fail depending on available system fonts.
        // We just verify it does not panic.
        if let Ok(bmp) = result {
            assert!(bmp.width() > 0);
            assert_eq!(bmp.height(), 64);
        }
    }

    #[test]
    fn inline_sizes_and_underline_change_ink_without_altering_plain_output() {
        use crate::styled_text::{StyleSpan, TextStyle};
        let mut renderer = TextRenderer::with_test_font();
        let plain = renderer
            .render_unbounded("Ag口", "", 20.0, TextAlign::Left, 1.2)
            .unwrap();
        let empty = renderer
            .render_unbounded_styled("Ag口", "", 20.0, TextAlign::Left, 1.2, &[], 12.0)
            .unwrap();
        assert_eq!(plain.data(), empty.data());
        let styles = vec![StyleSpan {
            start: 2,
            end: 3,
            style: TextStyle {
                font_size: Some(24.0),
                underline: true,
                ..Default::default()
            },
        }];
        let rich = renderer
            .render_unbounded_styled("Ag口", "", 20.0, TextAlign::Left, 1.2, &styles, 12.0)
            .unwrap();
        assert!(rich.width() > plain.width());
        assert!(rich.height() > plain.height());
        assert!(rich.data().iter().any(|byte| *byte != 0));
    }

    #[test]
    fn strikethrough_renders_into_the_print_bitmap_at_multiple_sizes() {
        use crate::styled_text::{StyleSpan, TextStyle};
        let mut renderer = TextRenderer::with_test_font();
        for size in [16.0, 32.0] {
            let mut styles = vec![StyleSpan {
                start: 0,
                end: 5,
                style: TextStyle::default(),
            }];
            let plain = renderer
                .render_unbounded_styled("A  口B", "", size, TextAlign::Left, 1.2, &styles, 12.0)
                .unwrap();
            styles[0].style.strikethrough = true;
            let struck = renderer
                .render_unbounded_styled("A  口B", "", size, TextAlign::Left, 1.2, &styles, 12.0)
                .unwrap();
            let ink = |b: &LabelBitmap| b.data().iter().map(|v| v.count_ones()).sum::<u32>();
            assert!(
                ink(&struck) > ink(&plain),
                "decoration must be present in exported pixels"
            );
            styles[0].style.strikethrough = false;
            let cleared = renderer
                .render_unbounded_styled("A  口B", "", size, TextAlign::Left, 1.2, &styles, 12.0)
                .unwrap();
            assert_eq!(plain, cleared);
        }
    }
}
