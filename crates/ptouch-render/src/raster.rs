// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Convert a [`LabelBitmap`] to raster lines suitable for Brother P-Touch
//! printers.
//!
//! The printer receives data column-by-column. Each raster line corresponds
//! to one vertical column of the label, and the image is centered vertically
//! on the tape.
//!
//! The bit layout within each raster line uses reversed byte order with
//! LSB-first bit ordering:
//!
//! ```text
//! rasterline[(size-1)-(pixel/8)] |= 1 << (pixel % 8)
//! ```

use crate::bitmap::LabelBitmap;

/// Set a single pixel in a raster line buffer.
///
/// - `rasterline`: byte buffer representing one vertical column
/// - `size`: total number of bytes in the raster line
/// - `pixel`: pixel index (0 = bottom of physical tape)
#[inline]
fn rasterline_setpixel(rasterline: &mut [u8], pixel: usize) {
    let size = rasterline.len();
    let byte_idx = size - 1 - pixel / 8;
    if byte_idx < size {
        rasterline[byte_idx] |= 1u8 << (pixel % 8);
    }
}

/// Convert a [`LabelBitmap`] into raster lines for the printer.
///
/// - `bitmap`: the rendered label bitmap
/// - `max_px`: maximum pixel height of the tape (from device/tape info)
///
/// Returns a `Vec` of raster lines. Each raster line is a `Vec<u8>` of
/// length `max_px / 8` bytes. One raster line per horizontal column of
/// the bitmap.
///
/// The image is centered vertically on the tape:
/// ```text
/// offset = (max_pixels / 2) - (image_height / 2)
/// ```
///
/// Within each column, pixels are read bottom-to-top from the bitmap
/// (y is flipped) to match the Brother P-Touch raster orientation.
pub fn bitmap_to_raster_lines(bitmap: &LabelBitmap, max_px: u16) -> Vec<Vec<u8>> {
    let raster_size = (max_px as usize) / 8;
    let bmp_height = bitmap.height() as usize;
    let bmp_width = bitmap.width() as usize;

    // Center the image vertically on the tape
    let offset = ((max_px as usize) / 2).saturating_sub(bmp_height / 2);

    let mut lines = Vec::with_capacity(bmp_width);

    for k in 0..bmp_width {
        let mut rasterline = vec![0u8; raster_size];

        for i in 0..bmp_height {
            // Read from the bitmap with Y flipped (bottom-to-top)
            let bmp_y = bmp_height - 1 - i;
            if bitmap.get_pixel(k as u32, bmp_y as u32) {
                rasterline_setpixel(&mut rasterline, offset + i);
            }
        }

        lines.push(rasterline);
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rasterline_setpixel_basic() {
        let mut line = vec![0u8; 4];
        // pixel 0 should set bit 0 of the last byte
        rasterline_setpixel(&mut line, 0);
        assert_eq!(line, [0, 0, 0, 1]);

        let mut line = vec![0u8; 4];
        // pixel 8 should set bit 0 of byte at index size-2
        rasterline_setpixel(&mut line, 8);
        assert_eq!(line, [0, 0, 1, 0]);

        let mut line = vec![0u8; 4];
        // pixel 7 should set bit 7 of the last byte
        rasterline_setpixel(&mut line, 7);
        assert_eq!(line, [0, 0, 0, 128]);
    }

    #[test]
    fn test_bitmap_to_raster_single_column() {
        // Create a 1-pixel wide, 8-pixel tall bitmap with top pixel set
        let mut bmp = LabelBitmap::new(1, 8);
        bmp.set_pixel(0, 0, true); // top pixel

        let lines = bitmap_to_raster_lines(&bmp, 16);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), 2); // 16/8 = 2 bytes

        // offset = 16/2 - 8/2 = 4
        // i iterates 0..8, bmp_y = 7 - i
        // When i=7, bmp_y=0, which is set -> pixel at offset+7 = 11
        // pixel 11 -> byte index = 2-1-11/8 = 2-1-1 = 0, bit = 11%8 = 3
        assert_eq!(lines[0][0], 0b0000_1000); // bit 3 set in byte 0
    }

    #[test]
    fn test_empty_bitmap_produces_empty_rasters() {
        let bmp = LabelBitmap::new(0, 8);
        let lines = bitmap_to_raster_lines(&bmp, 16);
        assert!(lines.is_empty());
    }

    #[test]
    fn test_raster_line_count_matches_width() {
        let bmp = LabelBitmap::new(100, 64);
        let lines = bitmap_to_raster_lines(&bmp, 128);
        assert_eq!(lines.len(), 100);
        for line in &lines {
            assert_eq!(line.len(), 16); // 128/8
        }
    }
}
