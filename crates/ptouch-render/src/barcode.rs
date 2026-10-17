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

pub(crate) fn encode_qr(
    data: &str,
    correction: QrErrorCorrection,
    version: Option<u8>,
) -> std::result::Result<qrcodegen::QrCode, qrcodegen::DataTooLong> {
    if let Some(version) = version {
        let version = qrcodegen::Version::new(version);
        let segments = crate::qr_segments::make_segments(data, version);
        return crate::qr_mask::encode_with_brother_mask_score(
            &segments,
            correction.level(),
            version,
        );
    }

    let mut last_error = None;
    for (first, last) in [(1, 9), (10, 26), (27, 40)] {
        let min_version = qrcodegen::Version::new(first);
        let max_version = qrcodegen::Version::new(last);
        let segments = crate::qr_segments::make_segments(data, min_version);
        match qrcodegen::QrCode::encode_segments_advanced(
            &segments,
            correction.level(),
            min_version,
            max_version,
            Some(qrcodegen::Mask::new(0)),
            false,
        ) {
            Ok(qr) => {
                let version = qr.version();
                let segments = crate::qr_segments::make_segments(data, version);
                return crate::qr_mask::encode_with_brother_mask_score(
                    &segments,
                    correction.level(),
                    version,
                );
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(last_error.expect("all QR version groups are evaluated"))
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
    if version.is_some_and(|version| !(1..=40).contains(&version)) {
        return Err(RenderError::Layout(
            "QR code version must be between 1 and 40".into(),
        ));
    }
    let qr = encode_qr(data, correction, version)
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
    fn automatic_qr_preserves_requested_error_correction_without_boosting() {
        let data = "12345678901234567890";
        let qr = encode_qr(data, QrErrorCorrection::Medium, None).unwrap();
        assert_eq!(qr.error_correction_level(), qrcodegen::QrCodeEcc::Medium);
        assert_eq!(qr.version().value(), 1);
    }

    #[test]
    fn fixed_version_mixed_text_fits_when_segmented_for_its_modes() {
        let bitmap =
            render_qr("HELLO WORLD 1234567890", QrErrorCorrection::Medium, Some(1)).unwrap();

        assert_eq!(bitmap.width(), 29 * 8);
    }

    #[test]
    fn automatic_mixed_text_selects_compact_version_one() {
        let bitmap = render_qr("HELLO WORLD 1234567890", QrErrorCorrection::Medium, None).unwrap();

        assert_eq!(bitmap.width(), 29 * 8);
    }

    #[test]
    fn automatic_mixed_byte_numeric_qr_matches_brother_modules() {
        let data = "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz01234567";
        let bitmap = render_qr(data, QrErrorCorrection::Medium, None).unwrap();
        let reference = include_str!("../tests/fixtures/p900w-qr-mixed-byte-numeric-v4-m.txt");
        let mut lines = reference.lines();
        let metadata = lines.next().unwrap();
        assert_eq!(metadata, "version=4 ecc=Medium mask=2 size=33");
        let matrix: Vec<_> = lines.collect();
        assert_eq!(bitmap.width(), 41 * 8);
        assert_eq!(matrix.len(), 33);
        for (row, expected) in matrix.iter().enumerate() {
            assert_eq!(expected.len(), 33);
            for (column, module) in expected.bytes().enumerate() {
                let actual =
                    bitmap.get_pixel((column as u32 + 4) * 8 + 4, (row as u32 + 4) * 8 + 4);
                assert_eq!(actual, module == b'#', "module ({column}, {row})");
            }
        }
    }

    #[test]
    fn brother_mask_score_matches_seventeen_of_nineteen_captured_controls() {
        let repetitive_50 = "a".repeat(49) + "b";
        let repetitive_55 = "a".repeat(54) + "b";
        let repetitive_60 = "a".repeat(60);
        let repetitive_62 = "a".repeat(61) + "b";
        let diverse = format!("{}01234567", "abcdefghijklmnopqrstuvwxyz".repeat(2));
        let cases: [(&str, u8, u8, u8); 19] = [
            (
                "gxblbbqsxkhyydmtkxmdimqqhiazutqwmauiodqeohcibebppkzrqnhskkfj",
                4,
                3,
                3,
            ),
            (
                "qouatbosdcqajactpykvuhyhyrluwmbndhwkifpqcbyoeesrmj",
                4,
                6,
                6,
            ),
            (
                "wwfcwnwvossfrjgfznlzrsrlwoegdewjaclteytkxdnlisauqh",
                4,
                2,
                2,
            ),
            (
                "xqdnhsmyrqqxbeyumaklmasjpgepnvcneypegxvntuycegqkhnmszbp",
                4,
                7,
                7,
            ),
            (
                "qoufsayimvaynaomakmknraredxggrlynkyjnduzwxggmzrrwc",
                4,
                6,
                6,
            ),
            (
                "lhvlfpscnrkrrnolqsymlmlwourphxhjdqbjtulzanhmcotclvcjuoa",
                4,
                6,
                6,
            ),
            (
                "wncwtqywaygnonydebwdqtygxmcyubpxoygolobofszssuusvj",
                4,
                0,
                0,
            ),
            ("hello 1234567890", 1, 0, 0),
            ("1234567890hello", 1, 3, 3),
            ("HELLO WORLD 1234567890", 1, 3, 3),
            ("abc123DEF456", 1, 3, 3),
            // Brother selects mask 0; the observed segment candidate scores as mask 3.
            ("ABC123abc789", 1, 3, 0),
            (&repetitive_50, 4, 1, 1),
            // Brother selects mask 1; both qrcodegen and this score select mask 2.
            (&repetitive_55, 4, 2, 1),
            (&repetitive_60, 4, 1, 1),
            (&repetitive_62, 4, 1, 1),
            ("12345678901234567890", 1, 4, 4),
            ("https://example.test/ptouch-cycle17", 3, 7, 7),
            (&diverse, 4, 2, 2),
        ];
        let mut matches = 0;
        for (data, version, expected, brother_mask) in cases {
            let qr = encode_qr(data, QrErrorCorrection::Medium, Some(version)).unwrap();
            assert_eq!(qr.mask().value(), expected, "score prediction for {data}");
            if expected == brother_mask {
                matches += 1;
            }
        }
        assert_eq!(matches, 17);
    }

    #[test]
    fn selected_masks_reproduce_all_cycle40_brother_matrices() {
        let cases = [
            (
                "gxblbbqsxkhyydmtkxmdimqqhiazutqwmauiodqeohcibebppkzrqnhskkfj",
                3,
                include_str!(
                    "../../../docs/re/windows-lbx-runtime/qr-matrices/mask-score-probes/02-mask3.txt"
                ),
            ),
            (
                "qouatbosdcqajactpykvuhyhyrluwmbndhwkifpqcbyoeesrmj",
                6,
                include_str!(
                    "../../../docs/re/windows-lbx-runtime/qr-matrices/mask-score-probes/14-mask6.txt"
                ),
            ),
            (
                "wwfcwnwvossfrjgfznlzrsrlwoegdewjaclteytkxdnlisauqh",
                2,
                include_str!(
                    "../../../docs/re/windows-lbx-runtime/qr-matrices/mask-score-probes/16-mask2.txt"
                ),
            ),
            (
                "xqdnhsmyrqqxbeyumaklmasjpgepnvcneypegxvntuycegqkhnmszbp",
                7,
                include_str!(
                    "../../../docs/re/windows-lbx-runtime/qr-matrices/mask-score-probes/20-mask7.txt"
                ),
            ),
            (
                "qoufsayimvaynaomakmknraredxggrlynkyjnduzwxggmzrrwc",
                6,
                include_str!(
                    "../../../docs/re/windows-lbx-runtime/qr-matrices/mask-score-probes/23-mask6.txt"
                ),
            ),
            (
                "lhvlfpscnrkrrnolqsymlmlwourphxhjdqbjtulzanhmcotclvcjuoa",
                6,
                include_str!(
                    "../../../docs/re/windows-lbx-runtime/qr-matrices/mask-score-probes/29-mask6.txt"
                ),
            ),
            (
                "wncwtqywaygnonydebwdqtygxmcyubpxoygolobofszssuusvj",
                0,
                include_str!(
                    "../../../docs/re/windows-lbx-runtime/qr-matrices/mask-score-probes/30-mask0.txt"
                ),
            ),
        ];

        for (data, expected_mask, reference) in cases {
            let qr = encode_qr(data, QrErrorCorrection::Medium, Some(4)).unwrap();
            let mut lines = reference.lines();
            assert_eq!(
                lines.next(),
                Some(&format!("version=4 ecc=Medium mask={expected_mask} size=33")[..])
            );
            assert_eq!(qr.mask().value(), expected_mask);
            for (y, row) in lines.enumerate() {
                for (x, module) in row.bytes().enumerate() {
                    assert_eq!(
                        qr.get_module(x as i32, y as i32),
                        module == b'#',
                        "probe {data}, module ({x}, {y})"
                    );
                }
            }
        }
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
