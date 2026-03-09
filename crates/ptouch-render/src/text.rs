// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Text rendering using cosmic-text.
//!
//! Renders multi-line text into a [`LabelBitmap`] using system fonts.
//! Supports auto-sizing, alignment, and font selection.

use cosmic_text::{Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache};

use crate::bitmap::LabelBitmap;
use crate::RenderError;
use crate::Result;

/// Text alignment within the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Render one or more lines of text into a [`LabelBitmap`].
    ///
    /// - `lines`: text lines to render (joined with newlines)
    /// - `print_width`: height of the tape in pixels (this becomes the bitmap
    ///   height; called "print_width" for compatibility with the C API where
    ///   the tape width is the printable width)
    /// - `font_name`: font family name to use (e.g. "DejaVu Sans")
    /// - `font_size`: explicit font size in points, or `None` for auto-detect
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
        buffer.set_size(
            &mut self.font_system,
            Some(layout_width),
            Some(available_height),
        );
        buffer.set_text(
            &mut self.font_system,
            &text,
            &attrs,
            Shaping::Advanced,
            cosmic_align,
        );
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
}
