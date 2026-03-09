// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! PNG image loading and conversion to 1-bit bitmap.
//!
//! Loads a PNG image, converts it to grayscale, auto-detects which colour
//! represents the foreground (darker), and produces a [`LabelBitmap`].

use std::io::Read;
use std::path::Path;

use image::DynamicImage;

use crate::bitmap::LabelBitmap;
use crate::Result;

/// Load a PNG image from a file path and convert to a 1-bit [`LabelBitmap`].
///
/// The image is converted to grayscale and then thresholded. The threshold
/// is determined by auto-detecting which extreme (black or white) has fewer
/// pixels, treating that extreme as the foreground.
pub fn load_png(path: &Path) -> Result<LabelBitmap> {
    let img = image::ImageReader::open(path)?.decode()?;
    Ok(convert_to_bitmap(img))
}

/// Load a PNG image from a reader and convert to a 1-bit [`LabelBitmap`].
///
/// The reader contents are buffered into memory so that the image decoder
/// can seek within them.
pub fn load_png_from_reader<R: Read>(mut reader: R) -> Result<LabelBitmap> {
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    let cursor = std::io::Cursor::new(buf);
    let img = image::ImageReader::with_format(cursor, image::ImageFormat::Png).decode()?;
    Ok(convert_to_bitmap(img))
}

/// Convert a `DynamicImage` to a 1-bit `LabelBitmap`.
///
/// Steps:
/// 1. Convert to grayscale (luma8).
/// 2. Compute mean luminance to determine threshold.
/// 3. Auto-detect foreground: if mean > 128, dark pixels are foreground;
///    otherwise, light pixels are foreground (inverted).
/// 4. Apply threshold to produce the bitmap.
fn convert_to_bitmap(img: DynamicImage) -> LabelBitmap {
    let gray = img.to_luma8();
    let (w, h) = gray.dimensions();

    if w == 0 || h == 0 {
        return LabelBitmap::new(0, 0);
    }

    // Compute mean luminance
    let total: u64 = gray.pixels().map(|p| p.0[0] as u64).sum();
    let count = (w as u64) * (h as u64);
    let mean = (total / count) as u8;

    // If the mean is above 128, the background is bright and dark pixels
    // are the foreground -> threshold at 128 (pixels <= 128 are black).
    // If the mean is below 128, the background is dark and light pixels
    // are the foreground -> we invert so that the minority colour becomes
    // the "black" in the bitmap. We still threshold at 128 but invert the
    // sense.
    let inverted = mean < 128;

    let mut bmp = LabelBitmap::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let luma = gray.get_pixel(x, y).0[0];
            let is_foreground = if inverted { luma > 128 } else { luma <= 128 };
            if is_foreground {
                bmp.set_pixel(x, y, true);
            }
        }
    }

    bmp
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Luma};

    #[test]
    fn test_convert_bright_background() {
        // White background with a black square
        let mut img = GrayImage::from_pixel(10, 10, Luma([255u8]));
        for y in 2..5 {
            for x in 2..5 {
                img.put_pixel(x, y, Luma([0u8]));
            }
        }
        let dyn_img = DynamicImage::ImageLuma8(img);
        let bmp = convert_to_bitmap(dyn_img);
        assert_eq!(bmp.width(), 10);
        assert_eq!(bmp.height(), 10);
        // Black pixel should be foreground
        assert!(bmp.get_pixel(2, 2));
        // White pixel should be background
        assert!(!bmp.get_pixel(0, 0));
    }

    #[test]
    fn test_convert_dark_background() {
        // Black background with a white square (inverted)
        let mut img = GrayImage::from_pixel(10, 10, Luma([0u8]));
        for y in 2..5 {
            for x in 2..5 {
                img.put_pixel(x, y, Luma([255u8]));
            }
        }
        let dyn_img = DynamicImage::ImageLuma8(img);
        let bmp = convert_to_bitmap(dyn_img);
        // White square on dark bg -> white pixels are foreground
        assert!(bmp.get_pixel(2, 2));
        assert!(!bmp.get_pixel(0, 0));
    }

    #[test]
    fn test_load_png_from_reader() {
        // Create a small in-memory PNG
        let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([0, 0, 0, 255]));
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let encoder = image::codecs::png::PngEncoder::new(cursor);
            image::ImageEncoder::write_image(
                encoder,
                img.as_raw(),
                4,
                4,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        }
        let result = load_png_from_reader(std::io::Cursor::new(&buf));
        assert!(result.is_ok());
    }
}
