// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Select a tape interval from the final bitmap without changing its layout.

use std::{borrow::Cow, ops::Range};

use crate::{RenderError, Result, bitmap::LabelBitmap};

const MAX_OUTPUT_WIDTH: u32 = 1_000_000;
const MAX_OUTPUT_PIXELS: u64 = 67_108_864;

/// Positions on the final preview ruler, before printer calibration.
/// The start is included and the end is excluded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputRange {
    start_mm: f64,
    end_mm: f64,
}

impl OutputRange {
    pub fn new(start_mm: f64, end_mm: f64) -> Result<Self> {
        if !start_mm.is_finite() || !end_mm.is_finite() {
            return Err(RenderError::Layout(
                "Range start and end must be finite.".into(),
            ));
        }
        if start_mm < 0.0 {
            return Err(RenderError::Layout(
                "Range start must be zero or greater.".into(),
            ));
        }
        if end_mm <= start_mm {
            return Err(RenderError::Layout(
                "Range end must be greater than its start.".into(),
            ));
        }
        Ok(Self { start_mm, end_mm })
    }

    pub fn start_mm(self) -> f64 {
        self.start_mm
    }

    pub fn end_mm(self) -> f64 {
        self.end_mm
    }

    /// Round each absolute endpoint once to the printer pixel grid.
    /// Differences of less than half a column can resolve to the same edge.
    /// Reject an interval outside the bitmap or smaller than one column.
    pub fn pixel_bounds(self, dpi: u16, bitmap_width: u32) -> Result<Range<u32>> {
        if dpi == 0 {
            return Err(RenderError::Layout("DPI must be greater than zero.".into()));
        }
        let scale = f64::from(dpi) / 25.4;
        let start = (self.start_mm * scale).round();
        let end = (self.end_mm * scale).round();
        if !start.is_finite()
            || !end.is_finite()
            || start > f64::from(bitmap_width)
            || end > f64::from(bitmap_width)
        {
            return Err(RenderError::Layout(
                "Range extends past the label end.".into(),
            ));
        }
        if start >= end {
            return Err(RenderError::Layout(
                "Range is smaller than one print column.".into(),
            ));
        }
        // Bounds are checked before converting to an integer. A large input
        // cannot become a valid endpoint through a saturating float cast.
        Ok(start as u32..end as u32)
    }
}

/// Check range geometry and output limits without allocating a cropped image.
/// A GUI can use this check to show range feedback.
pub fn output_bounds(
    bitmap: &LabelBitmap,
    dpi: u16,
    range: Option<OutputRange>,
) -> Result<Range<u32>> {
    let Some(range) = range else {
        return Ok(0..bitmap.width());
    };
    let bounds = range.pixel_bounds(dpi, bitmap.width())?;
    if bitmap.height() == 0 {
        return Err(RenderError::DimensionMismatch(
            "The label has no printable pixels.".into(),
        ));
    }
    let width = bounds.end - bounds.start;
    if width > MAX_OUTPUT_WIDTH || u64::from(width) * u64::from(bitmap.height()) > MAX_OUTPUT_PIXELS
    {
        return Err(RenderError::DimensionMismatch(
            "The range is larger than the output size limit.".into(),
        ));
    }
    Ok(bounds)
}

/// Crop the composed bitmap before applying printer calibration.
/// With no range, or a range that covers the bitmap, borrow the original.
pub fn apply_output_range(
    bitmap: &LabelBitmap,
    dpi: u16,
    range: Option<OutputRange>,
) -> Result<Cow<'_, LabelBitmap>> {
    let bounds = output_bounds(bitmap, dpi, range)?;
    if bounds.start == 0 && bounds.end == bitmap.width() {
        return Ok(Cow::Borrowed(bitmap));
    }

    let width = bounds.end - bounds.start;

    let source_stride = (bitmap.width() as usize).div_ceil(8);
    let target_stride = (width as usize).div_ceil(8);
    let shift = bounds.start % 8;
    let first_byte = (bounds.start / 8) as usize;
    let mut cropped = LabelBitmap::new(width, bitmap.height());
    for (y, row) in bitmap.data().chunks_exact(source_stride).enumerate() {
        for byte in 0..target_stride {
            let source = first_byte + byte;
            let mut bits = row[source] << shift;
            if shift != 0 {
                bits |= row.get(source + 1).copied().unwrap_or(0) >> (8 - shift);
            }
            let x = byte * 8;
            let count = (width as usize - x).min(8);
            bits &= u8::MAX << (8 - count);
            // Read packed source bytes and skip white pixels. The bitmap's
            // public setter keeps its unused trailing bits clear.
            while bits != 0 {
                let bit = bits.leading_zeros();
                cropped.set_pixel((x + bit as usize) as u32, y as u32, true);
                bits &= !(0x80 >> bit);
            }
        }
    }
    Ok(Cow::Owned(cropped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn patterned(width: u32, height: u32) -> LabelBitmap {
        let mut bitmap = LabelBitmap::new(width, height);
        for y in 0..height {
            for x in 0..width {
                bitmap.set_pixel(x, y, (x * 7 + y * 11) % 13 < 5);
            }
        }
        bitmap
    }

    fn at_columns(start: u32, end: u32, dpi: u16) -> OutputRange {
        let mm_per_column = 25.4 / f64::from(dpi);
        OutputRange::new(
            f64::from(start) * mm_per_column,
            f64::from(end) * mm_per_column,
        )
        .unwrap()
    }

    #[test]
    fn all_bit_offsets_and_short_final_bytes_keep_exact_source_pixels() {
        let original = patterned(41, 9);
        let unchanged = original.clone();
        for dpi in [180, 360] {
            for start in 0..16 {
                for end in start + 1..=41 {
                    let cropped =
                        apply_output_range(&original, dpi, Some(at_columns(start, end, dpi)))
                            .unwrap();
                    assert_eq!(cropped.width(), end - start);
                    assert_eq!(cropped.height(), original.height());
                    for y in 0..original.height() {
                        for x in 0..cropped.width() {
                            assert_eq!(cropped.get_pixel(x, y), original.get_pixel(start + x, y));
                        }
                        if !cropped.width().is_multiple_of(8) {
                            let stride = (cropped.width() as usize).div_ceil(8);
                            let last = cropped.data()[y as usize * stride + stride - 1];
                            assert_eq!(last & (u8::MAX >> (cropped.width() % 8)), 0);
                        }
                    }
                }
            }
        }
        assert_eq!(original, unchanged);
    }

    #[test]
    fn adjacent_mm_ranges_share_one_pixel_boundary_at_each_dpi() {
        let original = patterned(600, 5);
        for dpi in [180, 360] {
            let first =
                apply_output_range(&original, dpi, Some(OutputRange::new(0.0, 10.0).unwrap()))
                    .unwrap();
            let second =
                apply_output_range(&original, dpi, Some(OutputRange::new(10.0, 30.0).unwrap()))
                    .unwrap();
            let combined = first.append(&second);
            let expected =
                apply_output_range(&original, dpi, Some(OutputRange::new(0.0, 30.0).unwrap()))
                    .unwrap();
            assert_eq!(combined, *expected);
        }
    }

    #[test]
    fn ranges_follow_the_final_mirrored_preview() {
        let original = patterned(137, 7);
        for horizontal in [false, true] {
            for vertical in [false, true] {
                let final_bitmap = original.mirrored(horizontal, vertical);
                for dpi in [180, 360] {
                    let cropped =
                        apply_output_range(&final_bitmap, dpi, Some(at_columns(13, 96, dpi)))
                            .unwrap();
                    for y in 0..cropped.height() {
                        for x in 0..cropped.width() {
                            let source_x = if horizontal { 136 - x - 13 } else { x + 13 };
                            let source_y = if vertical { 6 - y } else { y };
                            assert_eq!(
                                cropped.get_pixel(x, y),
                                original.get_pixel(source_x, source_y)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn whole_bitmap_paths_borrow_and_preserve_existing_behavior() {
        let original = patterned(37, 5);
        assert!(matches!(
            apply_output_range(&original, 0, None).unwrap(),
            Cow::Borrowed(_)
        ));
        for dpi in [180, 360] {
            let range = at_columns(0, original.width(), dpi);
            assert_eq!(range.start_mm(), 0.0);
            assert!(range.end_mm() > range.start_mm());
            assert!(matches!(
                apply_output_range(&original, dpi, Some(range)).unwrap(),
                Cow::Borrowed(_)
            ));
        }
        let empty = LabelBitmap::new(0, 0);
        assert!(matches!(
            apply_output_range(&empty, 0, None).unwrap(),
            Cow::Borrowed(_)
        ));
    }

    #[test]
    fn invalid_ranges_are_rejected_before_allocation() {
        for (start, end) in [
            (f64::NAN, 1.0),
            (0.0, f64::NAN),
            (f64::NEG_INFINITY, 1.0),
            (0.0, f64::INFINITY),
            (-0.01, 1.0),
            (0.0, 0.0),
            (2.0, 1.0),
        ] {
            assert!(OutputRange::new(start, end).is_err());
        }
        for dpi in [180, 360] {
            assert!(
                OutputRange::new(0.0, 1.0)
                    .unwrap()
                    .pixel_bounds(0, 100)
                    .is_err()
            );
            assert!(
                OutputRange::new(0.0, 1.0)
                    .unwrap()
                    .pixel_bounds(dpi, 0)
                    .is_err()
            );
            assert!(
                OutputRange::new(0.0, f64::MAX)
                    .unwrap()
                    .pixel_bounds(dpi, 100)
                    .is_err()
            );
            assert!(
                OutputRange::new(f64::MAX / 2.0, f64::MAX)
                    .unwrap()
                    .pixel_bounds(dpi, 100)
                    .is_err()
            );
            assert!(
                OutputRange::new(0.0, 1e-100)
                    .unwrap()
                    .pixel_bounds(dpi, 100)
                    .is_err()
            );
            assert!(at_columns(90, 101, dpi).pixel_bounds(dpi, 100).is_err());
            assert!(at_columns(100, 101, dpi).pixel_bounds(dpi, 100).is_err());
        }
        let no_rows = LabelBitmap::new(20, 0);
        assert!(apply_output_range(&no_rows, 180, Some(at_columns(1, 10, 180))).is_err());
    }

    #[test]
    fn finite_endpoint_rounding_does_not_clamp_a_range() {
        for dpi in [180, 360] {
            let mm_per_column = 25.4 / f64::from(dpi);
            let within_edge =
                OutputRange::new(0.49 * mm_per_column, 100.49 * mm_per_column).unwrap();
            assert_eq!(within_edge.pixel_bounds(dpi, 100).unwrap(), 0..100);
            let past_edge = OutputRange::new(0.0, 100.51 * mm_per_column).unwrap();
            assert!(past_edge.pixel_bounds(dpi, 100).is_err());
        }
    }

    #[test]
    fn output_budget_is_checked_even_for_a_full_explicit_range() {
        let too_wide = LabelBitmap::new(MAX_OUTPUT_WIDTH + 1, 1);
        assert!(
            apply_output_range(
                &too_wide,
                180,
                Some(at_columns(0, MAX_OUTPUT_WIDTH + 1, 180)),
            )
            .is_err()
        );
        let too_many_pixels = LabelBitmap::new(8193, 8192);
        assert!(apply_output_range(&too_many_pixels, 180, Some(at_columns(0, 8193, 180))).is_err());
        assert!(apply_output_range(&too_many_pixels, 180, Some(at_columns(0, 10, 180))).is_ok());
    }

    #[test]
    fn calibration_keeps_its_ink_loss_check_after_cropping() {
        let mut original = LabelBitmap::new(100, 3);
        original.set_pixel(40, 1, true);
        let cropped = apply_output_range(&original, 180, Some(at_columns(40, 70, 180))).unwrap();
        assert!(crate::calibration::apply_calibration(&cropped, 180, Some(-1.0)).is_err());
        let cropped = apply_output_range(&original, 180, Some(at_columns(20, 70, 180))).unwrap();
        assert!(crate::calibration::apply_calibration(&cropped, 180, Some(-1.0)).is_ok());
    }
}
