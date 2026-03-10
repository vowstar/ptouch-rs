// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! 1-bit packed bitmap buffer for label rendering.
//!
//! The label is oriented as:
//! - width  = length along tape (horizontal / print direction)
//! - height = tape width in pixels (vertical)
//!
//! Pixels are packed row-major, 1 bit per pixel, MSB first.
//! A set bit (1) means black; a clear bit (0) means white.

use std::path::Path;

use image::{GrayImage, RgbaImage};

use crate::Result;

/// A 1-bit packed bitmap suitable for label printing.
#[derive(Debug, Clone)]
pub struct LabelBitmap {
    width: u32,
    height: u32,
    /// Packed bits, row-major order, MSB first. Each row is padded to
    /// a whole number of bytes.
    data: Vec<u8>,
}

impl LabelBitmap {
    /// Number of bytes per row (each row is padded to whole bytes).
    #[inline]
    fn row_stride(width: u32) -> usize {
        (width as usize).div_ceil(8)
    }

    /// Create a blank (all white) bitmap of the given dimensions.
    pub fn new(width: u32, height: u32) -> Self {
        let stride = Self::row_stride(width);
        Self {
            width,
            height,
            data: vec![0u8; stride * height as usize],
        }
    }

    /// Bitmap width in pixels (length along tape).
    #[inline]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Bitmap height in pixels (tape width).
    #[inline]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Read a single pixel. Returns `true` for black, `false` for white.
    /// Returns `false` if coordinates are out of bounds.
    #[inline]
    pub fn get_pixel(&self, x: u32, y: u32) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let stride = Self::row_stride(self.width);
        let byte_index = (y as usize) * stride + (x as usize) / 8;
        let bit_index = 7 - (x % 8);
        (self.data[byte_index] >> bit_index) & 1 != 0
    }

    /// Set a single pixel. `value = true` means black.
    /// Does nothing if coordinates are out of bounds.
    #[inline]
    pub fn set_pixel(&mut self, x: u32, y: u32, value: bool) {
        if x >= self.width || y >= self.height {
            return;
        }
        let stride = Self::row_stride(self.width);
        let byte_index = (y as usize) * stride + (x as usize) / 8;
        let bit_index = 7 - (x % 8);
        if value {
            self.data[byte_index] |= 1 << bit_index;
        } else {
            self.data[byte_index] &= !(1 << bit_index);
        }
    }

    /// Clear the entire bitmap to white.
    pub fn clear(&mut self) {
        self.data.fill(0);
    }

    /// Horizontally concatenate two bitmaps. The heights must match, or
    /// the shorter one is centered vertically within the taller one.
    pub fn append(&self, other: &LabelBitmap) -> LabelBitmap {
        let new_height = self.height.max(other.height);
        let new_width = self.width + other.width;
        let mut result = LabelBitmap::new(new_width, new_height);

        // Vertical offset to center self
        let y_off_a = (new_height - self.height) / 2;
        for y in 0..self.height {
            for x in 0..self.width {
                if self.get_pixel(x, y) {
                    result.set_pixel(x, y + y_off_a, true);
                }
            }
        }

        // Vertical offset to center other
        let y_off_b = (new_height - other.height) / 2;
        for y in 0..other.height {
            for x in 0..other.width {
                if other.get_pixel(x, y) {
                    result.set_pixel(self.width + x, y + y_off_b, true);
                }
            }
        }

        result
    }

    /// Convert to an RGBA image for PNG export or GUI display.
    /// Black pixels become (0, 0, 0, 255), white becomes (255, 255, 255, 255).
    pub fn to_rgba_image(&self) -> RgbaImage {
        let mut img = RgbaImage::new(self.width, self.height);
        for y in 0..self.height {
            for x in 0..self.width {
                let rgba = if self.get_pixel(x, y) {
                    image::Rgba([0, 0, 0, 255])
                } else {
                    image::Rgba([255, 255, 255, 255])
                };
                img.put_pixel(x, y, rgba);
            }
        }
        img
    }

    /// Create a `LabelBitmap` from a grayscale image by thresholding.
    /// Pixels with value <= `threshold` are considered black.
    pub fn from_gray_image(img: &GrayImage, threshold: u8) -> Self {
        let (w, h) = img.dimensions();
        let mut bmp = LabelBitmap::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let luma = img.get_pixel(x, y).0[0];
                if luma <= threshold {
                    bmp.set_pixel(x, y, true);
                }
            }
        }
        bmp
    }

    /// Save the bitmap as a PNG file at 180 DPI.
    pub fn save_png(&self, path: &Path) -> Result<()> {
        use image::codecs::png::PngEncoder;
        use image::ImageEncoder;
        use std::fs::File;
        use std::io::BufWriter;

        let rgba = self.to_rgba_image();
        let file = File::create(path)?;
        let writer = BufWriter::new(file);
        let encoder = PngEncoder::new(writer);

        // 180 DPI = 180 / 25.4 mm = ~7087 pixels per meter
        // The PNG encoder in the image crate does not directly support DPI
        // metadata through the simple API, so we encode the raw pixels.
        encoder.write_image(
            rgba.as_raw(),
            self.width,
            self.height,
            image::ExtendedColorType::Rgba8,
        )?;

        Ok(())
    }

    /// Rotate the bitmap by an arbitrary angle in degrees (clockwise).
    ///
    /// For exact multiples of 90 degrees, a lossless pixel-exact rotation
    /// is used. For other angles, nearest-neighbor sampling is applied.
    ///
    /// The result dimensions are the bounding box of the rotated image.
    pub fn rotate(&self, angle_deg: f32) -> LabelBitmap {
        if self.width == 0 || self.height == 0 {
            return self.clone();
        }

        // Normalize to [0, 360)
        let norm = ((angle_deg % 360.0) + 360.0) % 360.0;

        // Exact multiples of 90 -> lossless
        if (norm - 0.0).abs() < 0.5 || (norm - 360.0).abs() < 0.5 {
            return self.clone();
        }
        if (norm - 90.0).abs() < 0.5 {
            return self.rotate_exact(1);
        }
        if (norm - 180.0).abs() < 0.5 {
            return self.rotate_exact(2);
        }
        if (norm - 270.0).abs() < 0.5 {
            return self.rotate_exact(3);
        }

        self.rotate_arbitrary(norm)
    }

    /// Lossless rotation by exact 90-degree steps (1 = 90 CW, 2 = 180, 3 = 270 CW).
    fn rotate_exact(&self, steps: u8) -> LabelBitmap {
        match steps % 4 {
            0 => self.clone(),
            1 => {
                // 90 CW: (x,y) -> (H-1-y, x)
                let mut r = LabelBitmap::new(self.height, self.width);
                for y in 0..self.height {
                    for x in 0..self.width {
                        if self.get_pixel(x, y) {
                            r.set_pixel(self.height - 1 - y, x, true);
                        }
                    }
                }
                r
            }
            2 => {
                // 180: (x,y) -> (W-1-x, H-1-y)
                let mut r = LabelBitmap::new(self.width, self.height);
                for y in 0..self.height {
                    for x in 0..self.width {
                        if self.get_pixel(x, y) {
                            r.set_pixel(self.width - 1 - x, self.height - 1 - y, true);
                        }
                    }
                }
                r
            }
            3 => {
                // 270 CW: (x,y) -> (y, W-1-x)
                let mut r = LabelBitmap::new(self.height, self.width);
                for y in 0..self.height {
                    for x in 0..self.width {
                        if self.get_pixel(x, y) {
                            r.set_pixel(y, self.width - 1 - x, true);
                        }
                    }
                }
                r
            }
            _ => unreachable!(),
        }
    }

    /// Arbitrary-angle rotation using nearest-neighbor sampling.
    fn rotate_arbitrary(&self, angle_deg: f32) -> LabelBitmap {
        let angle_rad = angle_deg.to_radians();
        let cos_a = angle_rad.cos();
        let sin_a = angle_rad.sin();

        let sw = self.width as f32;
        let sh = self.height as f32;

        // Bounding box of rotated rectangle
        let new_w = (sw * cos_a.abs() + sh * sin_a.abs()).ceil() as u32;
        let new_h = (sw * sin_a.abs() + sh * cos_a.abs()).ceil() as u32;

        if new_w == 0 || new_h == 0 {
            return LabelBitmap::new(1, 1);
        }

        let cx_src = (sw - 1.0) / 2.0;
        let cy_src = (sh - 1.0) / 2.0;
        let cx_dst = (new_w as f32 - 1.0) / 2.0;
        let cy_dst = (new_h as f32 - 1.0) / 2.0;

        let mut result = LabelBitmap::new(new_w, new_h);

        for ny in 0..new_h {
            for nx in 0..new_w {
                let rel_x = nx as f32 - cx_dst;
                let rel_y = ny as f32 - cy_dst;

                // Inverse rotation to find source pixel
                let src_x = rel_x * cos_a + rel_y * sin_a + cx_src;
                let src_y = -rel_x * sin_a + rel_y * cos_a + cy_src;

                let sx = src_x.round() as i32;
                let sy = src_y.round() as i32;

                if sx >= 0
                    && sx < self.width as i32
                    && sy >= 0
                    && sy < self.height as i32
                    && self.get_pixel(sx as u32, sy as u32)
                {
                    result.set_pixel(nx, ny, true);
                }
            }
        }

        result
    }

    /// Adjust bitmap to a target height by centering vertically.
    ///
    /// If the current height is greater than `target_height`, the bitmap is
    /// cropped from the center. If smaller, it is padded with white.
    pub fn fit_height(&self, target_height: u32) -> LabelBitmap {
        if self.height == target_height {
            return self.clone();
        }
        let mut result = LabelBitmap::new(self.width, target_height);
        if self.height > target_height {
            // Crop: take the center portion
            let y_start = (self.height - target_height) / 2;
            for y in 0..target_height {
                for x in 0..self.width {
                    if self.get_pixel(x, y_start + y) {
                        result.set_pixel(x, y, true);
                    }
                }
            }
        } else {
            // Pad: center vertically
            let y_offset = (target_height - self.height) / 2;
            for y in 0..self.height {
                for x in 0..self.width {
                    if self.get_pixel(x, y) {
                        result.set_pixel(x, y_offset + y, true);
                    }
                }
            }
        }
        result
    }

    /// Trim unused white rows from top and bottom, returning a tight bitmap.
    ///
    /// This is useful before rotation so the rotated bounding box reflects
    /// only the actual content, not the full tape-height padding.
    pub fn trim_vertical(&self) -> LabelBitmap {
        if self.width == 0 || self.height == 0 {
            return self.clone();
        }

        // Find first and last row with any black pixel
        let mut first_row = None;
        let mut last_row = 0u32;

        for y in 0..self.height {
            for x in 0..self.width {
                if self.get_pixel(x, y) {
                    if first_row.is_none() {
                        first_row = Some(y);
                    }
                    last_row = y;
                    break;
                }
            }
        }

        let first_row = match first_row {
            Some(r) => r,
            None => return LabelBitmap::new(self.width, 1), // all white
        };

        let new_height = last_row - first_row + 1;
        let mut result = LabelBitmap::new(self.width, new_height);
        for y in 0..new_height {
            for x in 0..self.width {
                if self.get_pixel(x, first_row + y) {
                    result.set_pixel(x, y, true);
                }
            }
        }
        result
    }

    /// Scale bitmap to a target height, preserving aspect ratio.
    ///
    /// Uses nearest-neighbor interpolation. Returns a new bitmap whose
    /// height equals `target_height` and whose width is proportionally
    /// adjusted.
    pub fn scale_to_height(&self, target_height: u32) -> LabelBitmap {
        if self.height == 0 || self.width == 0 || target_height == 0 {
            return LabelBitmap::new(0, target_height);
        }
        if self.height == target_height {
            return self.clone();
        }
        let scale = target_height as f64 / self.height as f64;
        let target_width = ((self.width as f64 * scale).round() as u32).max(1);
        let mut result = LabelBitmap::new(target_width, target_height);
        for y in 0..target_height {
            let src_y = ((y as f64 / scale).floor() as u32).min(self.height - 1);
            for x in 0..target_width {
                let src_x = ((x as f64 / scale).floor() as u32).min(self.width - 1);
                if self.get_pixel(src_x, src_y) {
                    result.set_pixel(x, y, true);
                }
            }
        }
        result
    }

    /// Access the raw packed bit data.
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Number of bytes per row.
    pub fn stride(&self) -> usize {
        Self::row_stride(self.width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_bitmap_is_white() {
        let bmp = LabelBitmap::new(16, 8);
        for y in 0..8 {
            for x in 0..16 {
                assert!(!bmp.get_pixel(x, y));
            }
        }
    }

    #[test]
    fn test_set_get_pixel() {
        let mut bmp = LabelBitmap::new(32, 16);
        bmp.set_pixel(0, 0, true);
        bmp.set_pixel(7, 0, true);
        bmp.set_pixel(8, 0, true);
        bmp.set_pixel(31, 15, true);

        assert!(bmp.get_pixel(0, 0));
        assert!(bmp.get_pixel(7, 0));
        assert!(bmp.get_pixel(8, 0));
        assert!(bmp.get_pixel(31, 15));
        assert!(!bmp.get_pixel(1, 0));
        assert!(!bmp.get_pixel(30, 15));
    }

    #[test]
    fn test_clear() {
        let mut bmp = LabelBitmap::new(16, 8);
        bmp.set_pixel(5, 3, true);
        assert!(bmp.get_pixel(5, 3));
        bmp.clear();
        assert!(!bmp.get_pixel(5, 3));
    }

    #[test]
    fn test_out_of_bounds() {
        let mut bmp = LabelBitmap::new(8, 8);
        // Should not panic
        bmp.set_pixel(100, 100, true);
        assert!(!bmp.get_pixel(100, 100));
    }

    #[test]
    fn test_append() {
        let mut a = LabelBitmap::new(4, 8);
        a.set_pixel(0, 0, true);

        let mut b = LabelBitmap::new(4, 8);
        b.set_pixel(1, 1, true);

        let c = a.append(&b);
        assert_eq!(c.width(), 8);
        assert_eq!(c.height(), 8);
        assert!(c.get_pixel(0, 0));
        assert!(c.get_pixel(5, 1));
    }

    #[test]
    fn test_from_gray_image() {
        let img = GrayImage::from_fn(4, 4, |x, _y| {
            if x < 2 {
                image::Luma([0u8])
            } else {
                image::Luma([255u8])
            }
        });
        let bmp = LabelBitmap::from_gray_image(&img, 128);
        assert!(bmp.get_pixel(0, 0));
        assert!(bmp.get_pixel(1, 0));
        assert!(!bmp.get_pixel(2, 0));
        assert!(!bmp.get_pixel(3, 0));
    }

    #[test]
    fn test_rotate_90() {
        let mut bmp = LabelBitmap::new(3, 2);
        bmp.set_pixel(0, 0, true);
        bmp.set_pixel(2, 1, true);

        let rot = bmp.rotate(90.0);
        assert_eq!(rot.width(), 2);
        assert_eq!(rot.height(), 3);
        assert!(rot.get_pixel(1, 0));
        assert!(rot.get_pixel(0, 2));
        assert!(!rot.get_pixel(0, 0));
    }

    #[test]
    fn test_rotate_180() {
        let mut bmp = LabelBitmap::new(4, 4);
        bmp.set_pixel(0, 0, true);
        let rot = bmp.rotate(180.0);
        assert_eq!(rot.width(), 4);
        assert_eq!(rot.height(), 4);
        assert!(rot.get_pixel(3, 3));
        assert!(!rot.get_pixel(0, 0));
    }

    #[test]
    fn test_rotate_270() {
        let mut bmp = LabelBitmap::new(3, 2);
        bmp.set_pixel(0, 0, true);
        bmp.set_pixel(2, 1, true);

        let rot = bmp.rotate(270.0);
        assert_eq!(rot.width(), 2);
        assert_eq!(rot.height(), 3);
        assert!(rot.get_pixel(0, 2));
        assert!(rot.get_pixel(1, 0));
    }

    #[test]
    fn test_rotate_0_identity() {
        let mut bmp = LabelBitmap::new(5, 3);
        bmp.set_pixel(1, 2, true);
        bmp.set_pixel(4, 0, true);

        let rot = bmp.rotate(0.0);
        assert_eq!(rot.width(), 5);
        assert_eq!(rot.height(), 3);
        assert!(rot.get_pixel(1, 2));
        assert!(rot.get_pixel(4, 0));
    }

    #[test]
    fn test_rotate_360_identity() {
        let mut bmp = LabelBitmap::new(5, 3);
        bmp.set_pixel(1, 2, true);
        let rot = bmp.rotate(360.0);
        assert_eq!(rot.width(), 5);
        assert_eq!(rot.height(), 3);
        assert!(rot.get_pixel(1, 2));
    }

    #[test]
    fn test_rotate_negative() {
        let mut bmp = LabelBitmap::new(4, 4);
        bmp.set_pixel(0, 0, true);
        // -90 degrees = 270 degrees
        let rot = bmp.rotate(-90.0);
        assert_eq!(rot.width(), 4);
        assert_eq!(rot.height(), 4);
        assert!(rot.get_pixel(0, 3));
    }

    #[test]
    fn test_rotate_45_bounding_box() {
        let bmp = LabelBitmap::new(10, 10);
        let rot = bmp.rotate(45.0);
        // 45-degree bounding box of 10x10: ceil(10*0.707 + 10*0.707) = ceil(14.14) = 15
        assert!(rot.width() >= 14);
        assert!(rot.height() >= 14);
    }

    #[test]
    fn test_rotate_45_preserves_pixels() {
        let mut bmp = LabelBitmap::new(20, 20);
        // Draw a centered cross
        for i in 0..20 {
            bmp.set_pixel(10, i, true);
            bmp.set_pixel(i, 10, true);
        }
        let rot = bmp.rotate(45.0);
        // Center of the cross should still be black after rotation
        let cx = rot.width() / 2;
        let cy = rot.height() / 2;
        assert!(rot.get_pixel(cx, cy));
    }

    #[test]
    fn test_fit_height_pad() {
        let mut bmp = LabelBitmap::new(4, 2);
        bmp.set_pixel(1, 0, true);

        let fitted = bmp.fit_height(6);
        assert_eq!(fitted.width(), 4);
        assert_eq!(fitted.height(), 6);
        // Original y=0 -> centered at y=2
        assert!(fitted.get_pixel(1, 2));
        assert!(!fitted.get_pixel(1, 0));
    }

    #[test]
    fn test_fit_height_crop() {
        let mut bmp = LabelBitmap::new(4, 10);
        bmp.set_pixel(2, 5, true); // center pixel

        let fitted = bmp.fit_height(4);
        assert_eq!(fitted.width(), 4);
        assert_eq!(fitted.height(), 4);
        // y=5 in original, crop starts at y=3, so mapped to y=2
        assert!(fitted.get_pixel(2, 2));
    }

    #[test]
    fn test_fit_height_same() {
        let mut bmp = LabelBitmap::new(4, 8);
        bmp.set_pixel(0, 0, true);
        let fitted = bmp.fit_height(8);
        assert_eq!(fitted.height(), 8);
        assert!(fitted.get_pixel(0, 0));
    }

    #[test]
    fn test_to_rgba() {
        let mut bmp = LabelBitmap::new(2, 2);
        bmp.set_pixel(0, 0, true);
        let rgba = bmp.to_rgba_image();
        assert_eq!(rgba.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(rgba.get_pixel(1, 0).0, [255, 255, 255, 255]);
    }

    #[test]
    fn test_trim_vertical_basic() {
        let mut bmp = LabelBitmap::new(10, 20);
        // Put pixels in rows 5..10
        for y in 5..10 {
            bmp.set_pixel(3, y, true);
        }
        let trimmed = bmp.trim_vertical();
        assert_eq!(trimmed.width(), 10);
        assert_eq!(trimmed.height(), 5); // rows 5..9 inclusive
        assert!(trimmed.get_pixel(3, 0));
        assert!(trimmed.get_pixel(3, 4));
    }

    #[test]
    fn test_trim_vertical_all_white() {
        let bmp = LabelBitmap::new(10, 20);
        let trimmed = bmp.trim_vertical();
        assert_eq!(trimmed.height(), 1);
    }

    #[test]
    fn test_trim_vertical_full() {
        let mut bmp = LabelBitmap::new(4, 4);
        bmp.set_pixel(0, 0, true);
        bmp.set_pixel(0, 3, true);
        let trimmed = bmp.trim_vertical();
        assert_eq!(trimmed.height(), 4);
        assert!(trimmed.get_pixel(0, 0));
        assert!(trimmed.get_pixel(0, 3));
    }

    #[test]
    fn test_scale_to_height_downscale() {
        let mut bmp = LabelBitmap::new(100, 200);
        bmp.set_pixel(50, 100, true);
        let scaled = bmp.scale_to_height(100);
        assert_eq!(scaled.height(), 100);
        assert_eq!(scaled.width(), 50); // 100 * 0.5
        assert!(scaled.get_pixel(25, 50));
    }

    #[test]
    fn test_scale_to_height_upscale() {
        let mut bmp = LabelBitmap::new(10, 5);
        bmp.set_pixel(0, 0, true);
        let scaled = bmp.scale_to_height(10);
        assert_eq!(scaled.height(), 10);
        assert_eq!(scaled.width(), 20); // 10 * 2.0
        assert!(scaled.get_pixel(0, 0));
    }

    #[test]
    fn test_scale_to_height_same() {
        let mut bmp = LabelBitmap::new(10, 10);
        bmp.set_pixel(5, 5, true);
        let scaled = bmp.scale_to_height(10);
        assert_eq!(scaled.width(), 10);
        assert_eq!(scaled.height(), 10);
        assert!(scaled.get_pixel(5, 5));
    }

    #[test]
    fn test_scale_to_height_zero() {
        let bmp = LabelBitmap::new(10, 10);
        let scaled = bmp.scale_to_height(0);
        assert_eq!(scaled.height(), 0);
    }
}
