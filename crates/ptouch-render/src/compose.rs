// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Image composition helpers for label building.
//!
//! Provides cut marks, padding, and horizontal concatenation of bitmaps.

use crate::bitmap::LabelBitmap;

/// Create a cut-mark bitmap: a 9-pixel wide image with a dashed vertical
/// line at x=5.
///
/// The dashed pattern is 3 pixels on, 3 pixels off.
///
/// - `print_width`: height of the tape in pixels (bitmap height)
pub fn cutmark(print_width: u32) -> LabelBitmap {
    let mut bmp = LabelBitmap::new(9, print_width);

    // Dashed line at x=5: 3 black, 3 white, repeating
    let mut y = 0u32;
    while y < print_width {
        // 3 black pixels
        for dy in 0..3 {
            if y + dy < print_width {
                bmp.set_pixel(5, y + dy, true);
            }
        }
        // skip 3 white pixels (they are already white)
        y += 6;
    }

    bmp
}

/// Create a blank padding bitmap of the given width.
///
/// - `print_width`: height of the tape in pixels (bitmap height)
/// - `length`: width of the padding in pixels (horizontal extent)
pub fn padding(print_width: u32, length: u32) -> LabelBitmap {
    LabelBitmap::new(length, print_width)
}

/// Horizontally concatenate a slice of bitmaps in order.
///
/// Returns `None` if the slice is empty.
pub fn append_all(bitmaps: &[&LabelBitmap]) -> Option<LabelBitmap> {
    let mut iter = bitmaps.iter();
    let first = iter.next()?;
    let mut result = (*first).clone();
    for bmp in iter {
        result = result.append(bmp);
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cutmark_dimensions() {
        let cm = cutmark(64);
        assert_eq!(cm.width(), 9);
        assert_eq!(cm.height(), 64);
    }

    #[test]
    fn test_cutmark_dashed_pattern() {
        let cm = cutmark(12);
        // First 3 pixels at x=5 should be black
        assert!(cm.get_pixel(5, 0));
        assert!(cm.get_pixel(5, 1));
        assert!(cm.get_pixel(5, 2));
        // Next 3 should be white
        assert!(!cm.get_pixel(5, 3));
        assert!(!cm.get_pixel(5, 4));
        assert!(!cm.get_pixel(5, 5));
        // Next 3 should be black again
        assert!(cm.get_pixel(5, 6));
        assert!(cm.get_pixel(5, 7));
        assert!(cm.get_pixel(5, 8));

        // Pixels at other x positions should all be white
        assert!(!cm.get_pixel(0, 0));
        assert!(!cm.get_pixel(4, 0));
        assert!(!cm.get_pixel(6, 0));
        assert!(!cm.get_pixel(8, 0));
    }

    #[test]
    fn test_padding_is_blank() {
        let p = padding(64, 20);
        assert_eq!(p.width(), 20);
        assert_eq!(p.height(), 64);
        for y in 0..64 {
            for x in 0..20 {
                assert!(!p.get_pixel(x, y));
            }
        }
    }

    #[test]
    fn test_append_all_empty() {
        let result = append_all(&[]);
        assert!(result.is_none());
    }

    #[test]
    fn test_append_all_single() {
        let bmp = LabelBitmap::new(10, 8);
        let result = append_all(&[&bmp]);
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.width(), 10);
        assert_eq!(r.height(), 8);
    }

    #[test]
    fn test_append_all_multiple() {
        let a = LabelBitmap::new(5, 8);
        let b = LabelBitmap::new(10, 8);
        let c = LabelBitmap::new(3, 8);
        let result = append_all(&[&a, &b, &c]);
        assert!(result.is_some());
        let r = result.unwrap();
        assert_eq!(r.width(), 18);
        assert_eq!(r.height(), 8);
    }
}
