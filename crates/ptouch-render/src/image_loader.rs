// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Image loading and conversion to 1-bit bitmap.
//!
//! Loads images in any format supported by the `image` crate (PNG, JPEG,
//! GIF, BMP, TIFF, WebP, etc.) plus SVG via `resvg`. Optionally scales to
//! a target height and produces a [`LabelBitmap`] using either threshold
//! binarization (Otsu's method) or Floyd-Steinberg dithering.

use std::io::Read;
use std::path::Path;

use image::imageops::FilterType;
use image::{DynamicImage, GrayImage};

use crate::bitmap::LabelBitmap;
use crate::Result;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Binarization algorithm selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BinarizeMode {
    /// Automatically choose between threshold and dithering based on
    /// histogram analysis.
    #[default]
    Auto,
    /// Otsu's threshold -- best for text, line art, barcodes.
    Threshold,
    /// Floyd-Steinberg dithering -- best for photos and gradients.
    Dither,
}

/// Options controlling how an image is loaded and converted to bitmap.
#[derive(Debug, Clone)]
pub struct ImageLoadOptions {
    /// Binarization algorithm to use.
    pub binarize: BinarizeMode,
    /// If set, scale the image so its height matches this value (maintaining
    /// aspect ratio). Scaling is done in grayscale before binarization.
    pub target_height: Option<u32>,
    /// Auto-invert images with dark backgrounds so the minority colour
    /// becomes the foreground. Defaults to true.
    pub auto_invert: bool,
}

impl Default for ImageLoadOptions {
    fn default() -> Self {
        Self {
            binarize: BinarizeMode::default(),
            target_height: None,
            auto_invert: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Load an image from a file path and convert to a 1-bit [`LabelBitmap`].
///
/// SVG files are detected by extension and rendered via `resvg`. All other
/// formats are auto-detected from magic bytes via the `image` crate (PNG,
/// JPEG, GIF, BMP, TIFF, WebP, ICO, PNM, TGA, QOI, HDR, OpenEXR, etc.).
pub fn load_image(path: &Path, options: &ImageLoadOptions) -> Result<LabelBitmap> {
    let is_svg = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("svg") || ext.eq_ignore_ascii_case("svgz"));

    let img = if is_svg {
        load_svg_file(path, options.target_height)?
    } else {
        image::ImageReader::open(path)?
            .with_guessed_format()?
            .decode()?
    };
    Ok(convert_to_bitmap(img, options))
}

/// Load an image from a reader and convert to a 1-bit [`LabelBitmap`].
///
/// The reader contents are buffered into memory for format detection and
/// decoding.
pub fn load_image_from_reader<R: Read>(
    mut reader: R,
    options: &ImageLoadOptions,
) -> Result<LabelBitmap> {
    let mut buf = Vec::new();
    reader.read_to_end(&mut buf)?;
    let cursor = std::io::Cursor::new(buf);
    let img = image::ImageReader::new(cursor)
        .with_guessed_format()?
        .decode()?;
    Ok(convert_to_bitmap(img, options))
}

/// Load a PNG image from a file path (backward-compatible wrapper).
pub fn load_png(path: &Path) -> Result<LabelBitmap> {
    load_image(path, &ImageLoadOptions::default())
}

/// Load a PNG image from a reader (backward-compatible wrapper).
pub fn load_png_from_reader<R: Read>(reader: R) -> Result<LabelBitmap> {
    load_image_from_reader(reader, &ImageLoadOptions::default())
}

// ---------------------------------------------------------------------------
// SVG rendering
// ---------------------------------------------------------------------------

/// Render an SVG file to a `DynamicImage`.
///
/// If `target_height` is set, the SVG is scaled so its height matches that
/// value. Otherwise it is rendered at 180 DPI (matching Brother P-Touch
/// printer resolution).
fn load_svg_file(path: &Path, target_height: Option<u32>) -> Result<DynamicImage> {
    let svg_data = std::fs::read(path)?;
    render_svg_data(&svg_data, target_height)
}

/// Render SVG data bytes to a `DynamicImage`.
fn render_svg_data(data: &[u8], target_height: Option<u32>) -> Result<DynamicImage> {
    use resvg::tiny_skia;
    use resvg::usvg;

    let mut opt = usvg::Options {
        dpi: 180.0,
        ..usvg::Options::default()
    };

    let fontdb = std::sync::Arc::make_mut(&mut opt.fontdb);
    fontdb.load_system_fonts();

    let tree = usvg::Tree::from_data(data, &opt)
        .map_err(|e| crate::RenderError::Text(format!("SVG parse error: {}", e)))?;

    let svg_size = tree.size();
    let (mut w, mut h) = (svg_size.width() as u32, svg_size.height() as u32);
    if w == 0 || h == 0 {
        return Ok(DynamicImage::ImageRgba8(image::RgbaImage::new(1, 1)));
    }

    // Scale to target height if requested.
    let scale = if let Some(th) = target_height {
        if th > 0 && th != h {
            let s = th as f32 / h as f32;
            w = (w as f32 * s).round() as u32;
            h = th;
            s
        } else {
            1.0
        }
    } else {
        1.0
    };

    let pixmap = tiny_skia::Pixmap::new(w.max(1), h.max(1)).ok_or_else(|| {
        crate::RenderError::Text(format!("Failed to create {}x{} pixmap for SVG", w, h))
    })?;

    // Fill with white background (label printers print on white tape).
    let mut pixmap = pixmap;
    pixmap.fill(tiny_skia::Color::WHITE);

    let transform = tiny_skia::Transform::from_scale(scale, scale);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    let rgba = image::RgbaImage::from_raw(w, h, pixmap.take()).ok_or_else(|| {
        crate::RenderError::Text("Failed to convert SVG pixmap to image".to_string())
    })?;

    Ok(DynamicImage::ImageRgba8(rgba))
}

// ---------------------------------------------------------------------------
// Conversion pipeline
// ---------------------------------------------------------------------------

/// Convert a `DynamicImage` to a 1-bit `LabelBitmap`.
///
/// Pipeline: grayscale -> optional scale -> auto-invert detect -> binarize.
fn convert_to_bitmap(img: DynamicImage, options: &ImageLoadOptions) -> LabelBitmap {
    let mut gray = img.to_luma8();
    let (w, h) = gray.dimensions();

    if w == 0 || h == 0 {
        return LabelBitmap::new(0, 0);
    }

    // Scale to target height if requested, preserving aspect ratio.
    if let Some(target_h) = options.target_height {
        if target_h > 0 && target_h != h {
            let scale = target_h as f64 / h as f64;
            let new_w = ((w as f64 * scale).round() as u32).max(1);
            gray = image::imageops::resize(&gray, new_w, target_h, FilterType::Lanczos3);
        }
    }

    // Determine if the image should be inverted (dark background).
    let inverted = if options.auto_invert {
        let total: u64 = gray.pixels().map(|p| p.0[0] as u64).sum();
        let count = (gray.width() as u64) * (gray.height() as u64);
        let mean = (total / count) as u8;
        mean < 128
    } else {
        false
    };

    // Pick binarization mode.
    let mode = match options.binarize {
        BinarizeMode::Auto => detect_binarize_mode(&gray),
        other => other,
    };

    match mode {
        BinarizeMode::Dither | BinarizeMode::Auto => floyd_steinberg_dither(&gray, inverted),
        BinarizeMode::Threshold => {
            let thresh = otsu_threshold(&gray);
            threshold_binarize(&gray, thresh, inverted)
        }
    }
}

// ---------------------------------------------------------------------------
// Binarization algorithms
// ---------------------------------------------------------------------------

/// Compute the optimal threshold using Otsu's method.
///
/// Finds the threshold that maximizes between-class variance in a 256-bin
/// histogram.
fn otsu_threshold(gray: &GrayImage) -> u8 {
    let mut hist = [0u64; 256];
    for p in gray.pixels() {
        hist[p.0[0] as usize] += 1;
    }

    let total = (gray.width() as u64) * (gray.height() as u64);
    if total == 0 {
        return 128;
    }

    let mut sum_all: u64 = 0;
    for (i, &count) in hist.iter().enumerate() {
        sum_all += i as u64 * count;
    }

    let mut best_thresh: u8 = 0;
    let mut best_variance: f64 = 0.0;
    let mut w0: u64 = 0;
    let mut sum0: u64 = 0;

    for (t, &count) in hist.iter().enumerate() {
        w0 += count;
        if w0 == 0 {
            continue;
        }
        let w1 = total - w0;
        if w1 == 0 {
            break;
        }

        sum0 += t as u64 * count;
        let sum1 = sum_all - sum0;

        let mu0 = sum0 as f64 / w0 as f64;
        let mu1 = sum1 as f64 / w1 as f64;
        let diff = mu0 - mu1;
        let variance = w0 as f64 * w1 as f64 * diff * diff;

        if variance > best_variance {
            best_variance = variance;
            best_thresh = t as u8;
        }
    }

    best_thresh
}

/// Apply a fixed threshold to produce a bitmap.
fn threshold_binarize(gray: &GrayImage, thresh: u8, inverted: bool) -> LabelBitmap {
    let (w, h) = gray.dimensions();
    let mut bmp = LabelBitmap::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let luma = gray.get_pixel(x, y).0[0];
            let is_fg = if inverted {
                luma > thresh
            } else {
                luma <= thresh
            };
            if is_fg {
                bmp.set_pixel(x, y, true);
            }
        }
    }
    bmp
}

/// Floyd-Steinberg error-diffusion dithering.
///
/// Produces a bitmap where local dot density approximates the original
/// grayscale intensity -- ideal for photos and smooth gradients.
fn floyd_steinberg_dither(gray: &GrayImage, inverted: bool) -> LabelBitmap {
    let (w, h) = gray.dimensions();
    let w = w as usize;
    let h = h as usize;

    // Work buffer in f32 to accumulate fractional errors.
    let mut buf: Vec<f32> = gray.pixels().map(|p| p.0[0] as f32).collect();

    let mut bmp = LabelBitmap::new(w as u32, h as u32);

    for y in 0..h {
        for x in 0..w {
            let idx = y * w + x;
            let old = buf[idx].clamp(0.0, 255.0);
            let new = if old < 128.0 { 0.0 } else { 255.0 };
            let err = old - new;

            // Pixel is foreground if it quantized to black (0) for normal,
            // or to white (255) for inverted images.
            let is_fg = if inverted { new == 255.0 } else { new == 0.0 };
            if is_fg {
                bmp.set_pixel(x as u32, y as u32, true);
            }

            // Distribute error to neighbors.
            if x + 1 < w {
                buf[idx + 1] += err * (7.0 / 16.0);
            }
            if y + 1 < h {
                if x > 0 {
                    buf[(y + 1) * w + (x - 1)] += err * (3.0 / 16.0);
                }
                buf[(y + 1) * w + x] += err * (5.0 / 16.0);
                if x + 1 < w {
                    buf[(y + 1) * w + (x + 1)] += err * (1.0 / 16.0);
                }
            }
        }
    }

    bmp
}

/// Detect whether the image is better suited for threshold or dithering.
///
/// Builds a histogram and checks for bimodality: if there are two distinct
/// peaks with a deep valley between them, threshold works well. Otherwise
/// dithering preserves more detail.
fn detect_binarize_mode(gray: &GrayImage) -> BinarizeMode {
    let mut hist = [0u32; 256];
    for p in gray.pixels() {
        hist[p.0[0] as usize] += 1;
    }

    // Smooth histogram with a simple box filter to reduce noise.
    let mut smooth = [0u32; 256];
    for (i, slot) in smooth.iter_mut().enumerate() {
        let lo = i.saturating_sub(2);
        let hi = (i + 2).min(255);
        let window = &hist[lo..=hi];
        *slot = window.iter().sum::<u32>() / window.len() as u32;
    }

    // Find the two highest peaks (at least 30 bins apart).
    let mut peak1_idx = 0;
    let mut peak1_val = 0u32;
    for (i, &v) in smooth.iter().enumerate() {
        if v > peak1_val {
            peak1_val = v;
            peak1_idx = i;
        }
    }

    let mut peak2_idx = 0;
    let mut peak2_val = 0u32;
    for (i, &v) in smooth.iter().enumerate() {
        if (i as isize - peak1_idx as isize).unsigned_abs() >= 30 && v > peak2_val {
            peak2_val = v;
            peak2_idx = i;
        }
    }

    // No second peak found -> dither.
    if peak2_val == 0 {
        return BinarizeMode::Dither;
    }

    // Find the minimum valley between the two peaks.
    let (lo, hi) = if peak1_idx < peak2_idx {
        (peak1_idx, peak2_idx)
    } else {
        (peak2_idx, peak1_idx)
    };

    let valley_min = smooth[lo..=hi].iter().copied().min().unwrap_or(0);
    let smaller_peak = peak1_val.min(peak2_val);

    // If the valley is less than 40% of the smaller peak, the distribution
    // is clearly bimodal -> threshold works well.
    if smaller_peak > 0 && valley_min < smaller_peak * 2 / 5 {
        BinarizeMode::Threshold
    } else {
        BinarizeMode::Dither
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use image::{GrayImage, Luma};

    #[test]
    fn test_convert_bright_background() {
        let mut img = GrayImage::from_pixel(10, 10, Luma([255u8]));
        for y in 2..5 {
            for x in 2..5 {
                img.put_pixel(x, y, Luma([0u8]));
            }
        }
        let dyn_img = DynamicImage::ImageLuma8(img);
        let bmp = convert_to_bitmap(dyn_img, &ImageLoadOptions::default());
        assert_eq!(bmp.width(), 10);
        assert_eq!(bmp.height(), 10);
        assert!(bmp.get_pixel(2, 2));
        assert!(!bmp.get_pixel(0, 0));
    }

    #[test]
    fn test_convert_dark_background() {
        let mut img = GrayImage::from_pixel(10, 10, Luma([0u8]));
        for y in 2..5 {
            for x in 2..5 {
                img.put_pixel(x, y, Luma([255u8]));
            }
        }
        let dyn_img = DynamicImage::ImageLuma8(img);
        let bmp = convert_to_bitmap(dyn_img, &ImageLoadOptions::default());
        assert!(bmp.get_pixel(2, 2));
        assert!(!bmp.get_pixel(0, 0));
    }

    #[test]
    fn test_load_png_from_reader() {
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

    #[test]
    fn test_otsu_bimodal() {
        // Two groups with spread: dark (20-80) and bright (180-240).
        // With spread, Otsu has a unique maximum between the groups.
        let mut img = GrayImage::new(200, 100);
        for y in 0..100 {
            for x in 0..200 {
                let val = if y < 50 {
                    20 + (x as u32 * 60 / 199) as u8 // 20..80
                } else {
                    180 + (x as u32 * 60 / 199) as u8 // 180..240
                };
                img.put_pixel(x, y, Luma([val]));
            }
        }
        let thresh = otsu_threshold(&img);
        assert!(
            thresh >= 80 && thresh <= 180,
            "Expected threshold between 80 and 180, got {}",
            thresh
        );
    }

    #[test]
    fn test_otsu_skewed() {
        // 10% dark foreground (10-50) on bright background (190-230).
        let mut img = GrayImage::new(100, 100);
        for y in 0..100 {
            for x in 0..100 {
                let val = if y < 10 {
                    10 + (x as u32 * 40 / 99) as u8 // 10..50
                } else {
                    190 + (x as u32 * 40 / 99) as u8 // 190..230
                };
                img.put_pixel(x, y, Luma([val]));
            }
        }
        let thresh = otsu_threshold(&img);
        // Threshold should split between the dark and bright groups.
        assert!(
            thresh >= 50 && thresh <= 190,
            "Expected threshold between 50 and 190, got {}",
            thresh
        );
    }

    #[test]
    fn test_dither_density() {
        // Uniform gray(128) should produce roughly 50% black pixels.
        let img = GrayImage::from_pixel(100, 100, Luma([128u8]));
        let bmp = floyd_steinberg_dither(&img, false);
        let total = (bmp.width() * bmp.height()) as usize;
        let black: usize = (0..bmp.height())
            .flat_map(|y| (0..bmp.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| bmp.get_pixel(x, y))
            .count();
        let ratio = black as f64 / total as f64;
        assert!(
            (0.40..=0.60).contains(&ratio),
            "Expected ~50% black pixels, got {:.1}%",
            ratio * 100.0
        );
    }

    #[test]
    fn test_dither_black() {
        let img = GrayImage::from_pixel(20, 20, Luma([0u8]));
        let bmp = floyd_steinberg_dither(&img, false);
        for y in 0..20 {
            for x in 0..20 {
                assert!(bmp.get_pixel(x, y), "Pixel ({},{}) should be black", x, y);
            }
        }
    }

    #[test]
    fn test_dither_white() {
        let img = GrayImage::from_pixel(20, 20, Luma([255u8]));
        let bmp = floyd_steinberg_dither(&img, false);
        for y in 0..20 {
            for x in 0..20 {
                assert!(!bmp.get_pixel(x, y), "Pixel ({},{}) should be white", x, y);
            }
        }
    }

    #[test]
    fn test_load_with_scaling() {
        // Create a 100x100 PNG in memory, load with target_height=50.
        let img = image::RgbaImage::from_pixel(100, 100, image::Rgba([128, 128, 128, 255]));
        let mut buf = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut buf);
            let encoder = image::codecs::png::PngEncoder::new(cursor);
            image::ImageEncoder::write_image(
                encoder,
                img.as_raw(),
                100,
                100,
                image::ExtendedColorType::Rgba8,
            )
            .unwrap();
        }
        let options = ImageLoadOptions {
            target_height: Some(50),
            ..ImageLoadOptions::default()
        };
        let bmp = load_image_from_reader(std::io::Cursor::new(&buf), &options).unwrap();
        assert_eq!(bmp.height(), 50);
        assert_eq!(bmp.width(), 50);
    }

    #[test]
    fn test_render_svg() {
        // Minimal SVG: small black rectangle on white background.
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <rect x="30" y="30" width="40" height="40" fill="black"/>
        </svg>"#;
        let img = render_svg_data(svg, Some(50)).unwrap();
        let bmp = convert_to_bitmap(img, &ImageLoadOptions::default());
        assert_eq!(bmp.height(), 50);
        assert_eq!(bmp.width(), 50);
        // Center should be black (the rectangle).
        assert!(bmp.get_pixel(25, 25));
        // Corner should be white (outside the rectangle).
        assert!(!bmp.get_pixel(0, 0));
    }

    #[test]
    fn test_auto_invert() {
        // Dark background with bright foreground -- auto_invert should
        // mark the bright pixels as foreground.
        let mut img = GrayImage::from_pixel(20, 20, Luma([10u8]));
        for y in 5..15 {
            for x in 5..15 {
                img.put_pixel(x, y, Luma([245u8]));
            }
        }
        let dyn_img = DynamicImage::ImageLuma8(img);
        let bmp = convert_to_bitmap(dyn_img, &ImageLoadOptions::default());
        // Bright square on dark bg -> bright pixels are foreground.
        assert!(bmp.get_pixel(7, 7));
        // Dark background pixel should not be foreground.
        assert!(!bmp.get_pixel(0, 0));
    }
}
