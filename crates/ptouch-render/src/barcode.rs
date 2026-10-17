// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Bounded barcode primitives shared by editing, preview, and print rendering.

use serde::{Deserialize, Serialize};

use crate::bitmap::LabelBitmap;
use crate::{RenderError, Result};

const CODE39_ALPHABET: &str = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ-. $/+%";
const CODE39_PATTERNS: [u16; 44] = [
    0b000110100,
    0b100100001,
    0b001100001,
    0b101100000,
    0b000110001,
    0b100110000,
    0b001110000,
    0b000100101,
    0b100100100,
    0b001100100,
    0b100001001,
    0b001001001,
    0b101001000,
    0b000011001,
    0b100011000,
    0b001011000,
    0b000001101,
    0b100001100,
    0b001001100,
    0b000011100,
    0b100000011,
    0b001000011,
    0b101000010,
    0b000010011,
    0b100010010,
    0b001010010,
    0b000000111,
    0b100000110,
    0b001000110,
    0b000010110,
    0b110000001,
    0b011000001,
    0b111000000,
    0b010010001,
    0b110010000,
    0b011010000,
    0b010000101,
    0b110000100,
    0b011000100,
    0b010101000,
    0b010100010,
    0b010001010,
    0b000101010,
    0b010010100,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QrErrorCorrection {
    Low,
    #[default]
    Medium,
    Quartile,
    High,
}

impl QrErrorCorrection {
    pub(crate) fn lbx_name(self) -> &'static str {
        match self {
            Self::Low => "7%",
            Self::Medium => "15%",
            Self::Quartile => "25%",
            Self::High => "30%",
        }
    }

    fn level(self) -> qrcodegen::QrCodeEcc {
        match self {
            Self::Low => qrcodegen::QrCodeEcc::Low,
            Self::Medium => qrcodegen::QrCodeEcc::Medium,
            Self::Quartile => qrcodegen::QrCodeEcc::Quartile,
            Self::High => qrcodegen::QrCodeEcc::High,
        }
    }
}

/// Render a QR symbol with a four-module quiet zone at a stable 8 px/module.
pub fn render_qr(
    data: &str,
    correction: QrErrorCorrection,
    version: Option<u8>,
) -> Result<LabelBitmap> {
    if data.is_empty() {
        return Err(RenderError::Layout("QR code data cannot be empty".into()));
    }
    let segments = qrcodegen::QrSegment::make_segments(data);
    let qr = if let Some(version) = version {
        if !(1..=40).contains(&version) {
            return Err(RenderError::Layout(
                "QR code version must be between 1 and 40".into(),
            ));
        }
        qrcodegen::QrCode::encode_segments_advanced(
            &segments,
            correction.level(),
            qrcodegen::Version::new(version),
            qrcodegen::Version::new(version),
            None,
            false,
        )
    } else {
        qrcodegen::QrCode::encode_segments(&segments, correction.level())
    }
    .map_err(|_| RenderError::Layout("QR code data exceeds the supported capacity".into()))?;
    let modules = u32::try_from(qr.size())
        .map_err(|_| RenderError::Layout("QR symbol dimensions are invalid".into()))?;
    let side = (modules + 8) * 8;
    if side > 4096 {
        return Err(RenderError::Layout(
            "QR symbol is too large to render".into(),
        ));
    }
    let mut bitmap = LabelBitmap::new(side, side);
    for y in 0..modules {
        for x in 0..modules {
            if qr.get_module(x as i32, y as i32) {
                for dy in 0..8 {
                    for dx in 0..8 {
                        bitmap.set_pixel((x + 4) * 8 + dx, (y + 4) * 8 + dy, true);
                    }
                }
            }
        }
    }
    Ok(bitmap)
}

/// Render a bounded Code 39 symbol with ten-module quiet zones.
///
/// The optional Mod 43 check character is encoded after the payload and is
/// included in the returned human-readable value.
pub fn render_code39(
    data: &str,
    check_digit: bool,
    narrow_width: u32,
) -> Result<(LabelBitmap, String)> {
    if data.is_empty() || data.len() > 512 {
        return Err(RenderError::Layout(
            "Code 39 data must contain between 1 and 512 ASCII characters".into(),
        ));
    }
    if !data.is_ascii()
        || data
            .bytes()
            .any(|byte| !CODE39_ALPHABET.as_bytes().contains(&byte))
    {
        return Err(RenderError::Layout(
            "Code 39 supports only uppercase letters, digits, space, and - . $ / + %".into(),
        ));
    }

    let mut encoded = data.to_owned();
    if check_digit {
        let sum: usize = data
            .bytes()
            .map(|byte| {
                CODE39_ALPHABET
                    .as_bytes()
                    .iter()
                    .position(|candidate| *candidate == byte)
                    .unwrap()
            })
            .sum();
        encoded.push(CODE39_ALPHABET.as_bytes()[sum % CODE39_ALPHABET.len()] as char);
    }

    if narrow_width == 0 || narrow_width > 256 {
        return Err(RenderError::Layout(
            "Code 39 narrow bar width must be between 1 and 256 pixels".into(),
        ));
    }
    let quiet_zone = 10 * narrow_width;
    let char_width = 15 * narrow_width;
    let inter_character_gap = narrow_width;
    let module_width = quiet_zone * 2
        + (encoded.len() as u32 + 2) * char_width
        + (encoded.len() as u32 + 1) * inter_character_gap;
    if module_width > 16_384 {
        return Err(RenderError::Layout(
            "Code 39 symbol is too large to render".into(),
        ));
    }
    let mut bitmap = LabelBitmap::new(module_width, 80);
    let mut x = quiet_zone;
    for (character_index, byte) in std::iter::once(b'*')
        .chain(encoded.bytes())
        .chain(std::iter::once(b'*'))
        .enumerate()
    {
        let index = if byte == b'*' {
            43
        } else {
            CODE39_ALPHABET
                .as_bytes()
                .iter()
                .position(|candidate| *candidate == byte)
                .unwrap()
        };
        let pattern = CODE39_PATTERNS[index];
        for run_index in 0..9 {
            let wide = pattern & (1 << (8 - run_index)) != 0;
            let run_width = narrow_width * if wide { 3 } else { 1 };
            if run_index % 2 == 0 {
                for y in 0..bitmap.height() {
                    for bar_x in x..x + run_width {
                        bitmap.set_pixel(bar_x, y, true);
                    }
                }
            }
            x += run_width;
        }
        if character_index + 1 < encoded.len() + 2 {
            x += inter_character_gap;
        }
    }
    Ok((bitmap, encoded))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_output_keeps_quiet_zone_and_encodes_modules() {
        let bitmap = render_qr("PARITY", QrErrorCorrection::Medium, None).unwrap();
        assert_eq!(bitmap.width(), bitmap.height());
        assert!(bitmap.width() >= 232);
        for coordinate in 0..bitmap.width() {
            assert!(!bitmap.get_pixel(coordinate, 0));
            assert!(!bitmap.get_pixel(0, coordinate));
        }
        assert!(
            (0..bitmap.height()).any(|y| { (0..bitmap.width()).any(|x| bitmap.get_pixel(x, y)) })
        );
    }

    #[test]
    fn rejects_empty_or_oversized_payloads() {
        assert!(render_qr("", QrErrorCorrection::Medium, None).is_err());
        assert!(render_qr(&"x".repeat(4000), QrErrorCorrection::High, None).is_err());
    }

    #[test]
    fn fixed_version_is_preserved_and_rejects_payloads_that_do_not_fit() {
        let bitmap = render_qr("FIXED VERSION", QrErrorCorrection::Quartile, Some(5)).unwrap();
        assert_eq!(bitmap.width(), 45 * 8);
        assert!(render_qr(&"x".repeat(100), QrErrorCorrection::High, Some(1)).is_err());
    }

    #[test]
    fn code39_adds_start_stop_and_mod43_check_character() {
        let (without_check, value) = render_code39("ABC", false, 2).unwrap();
        assert_eq!(value, "ABC");
        let (with_check, value) = render_code39("ABC", true, 2).unwrap();
        assert_eq!(value, "ABCX");
        assert_eq!(without_check.width(), 2 * 10 * 2 + (3 + 2) * 15 * 2 + 4 * 2);
        assert_eq!(with_check.width(), without_check.width() + 32);
        assert!(with_check.get_pixel(20, 0));
        assert!(!with_check.get_pixel(0, 0));
        assert!(!with_check.get_pixel(with_check.width() - 1, 0));
    }

    #[test]
    fn code39_rejects_unsupported_alphabet_and_unbounded_data() {
        assert!(render_code39("lower", false, 2).is_err());
        assert!(render_code39("ABC*", false, 2).is_err());
        assert!(render_code39(&"A".repeat(513), false, 2).is_err());
    }
}
