// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Physical monochrome stroke patterns, shared by printed lines and borders.

use serde::{Deserialize, Serialize};

use crate::{RenderError, Result, bitmap::LabelBitmap};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatternKind {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StrokePattern {
    pub kind: PatternKind,
    pub dash_mm: f64,
    pub gap_mm: f64,
}

impl Default for StrokePattern {
    fn default() -> Self {
        Self {
            kind: PatternKind::Solid,
            dash_mm: 2.0,
            gap_mm: 1.0,
        }
    }
}

impl StrokePattern {
    pub fn validate(&self) -> Result<()> {
        if !self.dash_mm.is_finite()
            || !self.gap_mm.is_finite()
            || !(0.01..=1000.0).contains(&self.dash_mm)
            || !(0.01..=1000.0).contains(&self.gap_mm)
        {
            return Err(RenderError::Layout(
                "stroke dash and gap must be in 0.01–1000 mm".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn raster_line(&self, width: u32, height: u32, dpi: u16) -> Result<LabelBitmap> {
        self.validate()?;
        if width == 0
            || height == 0
            || dpi == 0
            || u64::from(width) * u64::from(height) > 16_777_216
        {
            return Err(RenderError::Layout(
                "line dimensions exceed the raster limit".into(),
            ));
        }
        let pixels = |mm: f64| (mm * f64::from(dpi) / 25.4).round().max(1.0) as u32;
        let dash = pixels(self.dash_mm);
        let gap = pixels(self.gap_mm);
        let mut bitmap = LabelBitmap::new(width, height);
        for x in 0..width {
            for y in 0..height {
                let ink = match self.kind {
                    PatternKind::Solid => true,
                    PatternKind::Dashed => x % (dash + gap) < dash,
                    PatternKind::Dotted => {
                        let local = x % (height + gap);
                        let center = f64::from(height - 1) / 2.0;
                        (f64::from(local) - center).powi(2) + (f64::from(y) - center).powi(2)
                            <= (f64::from(height) / 2.0).powi(2)
                    }
                };
                if ink {
                    bitmap.set_pixel(x, y, true);
                }
            }
        }
        Ok(bitmap)
    }
}

/// Printed outline inside an object's outer content box.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BorderStyle {
    pub width_mm: f64,
    /// Clear distance between the stroke and the text/image interior.
    pub padding_mm: f64,
    pub pattern: StrokePattern,
}

impl Default for BorderStyle {
    fn default() -> Self {
        Self {
            width_mm: 0.3,
            padding_mm: 0.5,
            pattern: Default::default(),
        }
    }
}

impl BorderStyle {
    pub fn validate(&self) -> Result<()> {
        if !self.width_mm.is_finite()
            || !(0.01..=1000.0).contains(&self.width_mm)
            || !self.padding_mm.is_finite()
            || !(0.0..=1000.0).contains(&self.padding_mm)
        {
            return Err(RenderError::Layout(
                "border width must be in 0.01–1000 mm and spacing in 0–1000 mm".into(),
            ));
        }
        self.pattern.validate()
    }

    pub(crate) fn inset_pixels(&self, dpi: u16) -> Result<u32> {
        self.validate()?;
        if dpi == 0 {
            return Err(RenderError::Layout("border DPI must be positive".into()));
        }
        let ppm = f64::from(dpi) / 25.4;
        Ok((self.width_mm * ppm).round().max(1.0) as u32 + (self.padding_mm * ppm).round() as u32)
    }

    pub fn inset_mm(&self, dpi: u16) -> Result<f64> {
        Ok(f64::from(self.inset_pixels(dpi)?) * 25.4 / f64::from(dpi))
    }

    pub(crate) fn draw(&self, bitmap: &mut LabelBitmap, dpi: u16) -> Result<()> {
        let inset = self.inset_pixels(dpi)?;
        let (w, h) = (bitmap.width(), bitmap.height());
        if w <= 2 * inset || h <= 2 * inset {
            return Err(RenderError::Layout("border and spacing leave no room for content; enlarge the object or reduce the border".into()));
        }
        let stroke = (self.width_mm * f64::from(dpi) / 25.4).round().max(1.0) as u32;
        let horizontal = self.pattern.raster_line(w, stroke, dpi)?;
        let vertical = self.pattern.raster_line(h, stroke, dpi)?;
        // Each edge starts its own physical pattern. White never erases ink.
        for x in 0..w {
            for y in 0..stroke {
                if horizontal.get_pixel(x, y) {
                    bitmap.set_pixel(x, y, true);
                    bitmap.set_pixel(x, h - 1 - y, true);
                }
            }
        }
        for y in 0..h {
            for x in 0..stroke {
                if vertical.get_pixel(y, x) {
                    bitmap.set_pixel(x, y, true);
                    bitmap.set_pixel(w - 1 - x, y, true);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_borders_and_empty_interiors_are_rejected() {
        for width in [0.0, -1.0, f64::NAN, f64::INFINITY, 1001.0] {
            assert!(
                BorderStyle {
                    width_mm: width,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        for padding in [-1.0, f64::NAN, f64::INFINITY, 1001.0] {
            assert!(
                BorderStyle {
                    padding_mm: padding,
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
        let border = BorderStyle::default();
        assert!(border.inset_pixels(0).is_err());
        assert!(border.draw(&mut LabelBitmap::new(2, 2), 180).is_err());
        for dpi in [180, 360] {
            let mut bitmap = LabelBitmap::new(100, 60);
            border.draw(&mut bitmap, dpi).unwrap();
            assert!(bitmap.get_pixel(0, 0));
            assert!(!bitmap.get_pixel(50, 30));
        }
    }

    #[test]
    fn patterns_scale_physical_periods_and_keep_one_dot_rules_visible() {
        for (dpi, scale) in [(180, 1), (360, 2)] {
            let pattern = StrokePattern {
                kind: PatternKind::Dashed,
                dash_mm: 25.4 / 180.0 * 4.0,
                gap_mm: 25.4 / 180.0 * 2.0,
            };
            let bitmap = pattern.raster_line(24 * scale, scale, dpi).unwrap();
            for x in 0..24 * scale {
                assert_eq!(bitmap.get_pixel(x, 0), x % (6 * scale) < 4 * scale);
            }
            let dots = StrokePattern {
                kind: PatternKind::Dotted,
                ..Default::default()
            };
            assert!(dots.raster_line(10, 1, dpi).unwrap().get_pixel(0, 0));
        }
    }

    #[test]
    fn invalid_periods_and_raster_budgets_are_rejected_before_drawing() {
        for gap in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(
                StrokePattern {
                    gap_mm: gap,
                    ..Default::default()
                }
                .raster_line(10, 2, 180)
                .is_err()
            );
        }
        assert!(
            StrokePattern::default()
                .raster_line(100_000, 100_000, 180)
                .is_err()
        );
    }
}
