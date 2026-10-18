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

#[cfg(test)]
mod tests {
    use super::*;

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
