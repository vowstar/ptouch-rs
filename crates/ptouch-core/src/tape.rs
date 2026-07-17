// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>
// SPDX-FileCopyrightText: Dominic Radermacher and the ptouch-print contributors
//
// Portions derived from ptouch-print, licensed GPL-3.0-or-later:
// https://git.familie-radermacher.ch/linux/ptouch-print.git

//! Tape width information for Brother P-Touch label printers.
//!
//! Maps nominal tape widths (in mm) to printable pixel counts and margin
//! sizes. Printable pixels depend on the print head resolution, so lookups
//! take the device DPI. The caller must clamp the result to the device
//! head width (`max_px`); narrow heads cannot cover the full print area
//! of wide tapes.

/// Information about a specific tape width.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TapeInfo {
    /// Nominal tape width in millimeters.
    pub width_mm: u8,
    /// Number of printable pixels across the tape.
    pub pixels: u16,
    /// Margin size in millimeters.
    pub margin_mm: f64,
}

/// Tape print areas for 180 dpi printers.
static TAPE_TABLE_180: &[TapeInfo] = &[
    TapeInfo {
        width_mm: 4,
        pixels: 24,
        margin_mm: 0.5,
    }, //  3.5 mm tape
    TapeInfo {
        width_mm: 6,
        pixels: 32,
        margin_mm: 1.0,
    }, //  6 mm tape
    TapeInfo {
        width_mm: 9,
        pixels: 52,
        margin_mm: 1.0,
    }, //  9 mm tape
    TapeInfo {
        width_mm: 12,
        pixels: 76,
        margin_mm: 2.0,
    }, // 12 mm tape
    TapeInfo {
        width_mm: 18,
        pixels: 120,
        margin_mm: 3.0,
    }, // 18 mm tape
    TapeInfo {
        width_mm: 21,
        pixels: 124,
        margin_mm: 3.0,
    }, // 21 mm tape
    TapeInfo {
        width_mm: 24,
        pixels: 128,
        margin_mm: 3.0,
    }, // 24 mm tape
    TapeInfo {
        width_mm: 36,
        pixels: 192,
        margin_mm: 4.5,
    }, // 36 mm tape
];

/// Tape print areas for 360 dpi printers (PT-9200DX, PT-9500PC, PT-9700PC).
///
/// Values are the official TZe print areas from the Brother PT-P900 raster
/// command reference and the PT-9700PC ESC/P command reference. They are
/// not simply double the 180 dpi values: the 180 dpi table is limited by
/// the 128 pin heads of those models (e.g. 24 mm tape has a 320 dot print
/// area at 360 dpi, not 256).
static TAPE_TABLE_360: &[TapeInfo] = &[
    TapeInfo {
        width_mm: 4,
        pixels: 48,
        margin_mm: 0.5,
    }, //  3.5 mm tape
    TapeInfo {
        width_mm: 6,
        pixels: 64,
        margin_mm: 1.0,
    }, //  6 mm tape
    TapeInfo {
        width_mm: 9,
        pixels: 106,
        margin_mm: 1.0,
    }, //  9 mm tape
    TapeInfo {
        width_mm: 12,
        pixels: 150,
        margin_mm: 2.0,
    }, // 12 mm tape
    TapeInfo {
        width_mm: 18,
        pixels: 234,
        margin_mm: 3.0,
    }, // 18 mm tape
    TapeInfo {
        width_mm: 24,
        pixels: 320,
        margin_mm: 3.0,
    }, // 24 mm tape
    TapeInfo {
        width_mm: 36,
        pixels: 454,
        margin_mm: 4.5,
    }, // 36 mm tape
];

/// Select the tape table for a given print resolution.
fn table_for_dpi(dpi: u16) -> &'static [TapeInfo] {
    if dpi >= 360 {
        TAPE_TABLE_360
    } else {
        TAPE_TABLE_180
    }
}

/// Look up tape info by nominal width in millimeters and printer DPI.
///
/// Returns `Some(&TapeInfo)` if the width is recognized, `None` otherwise.
pub fn find_tape(width_mm: u8, dpi: u16) -> Option<&'static TapeInfo> {
    table_for_dpi(dpi).iter().find(|t| t.width_mm == width_mm)
}

/// Returns the list of supported tape sizes for a given printer DPI.
pub fn supported_tapes(dpi: u16) -> &'static [TapeInfo] {
    table_for_dpi(dpi)
}

/// Get the printable pixel count for a given tape width and printer DPI.
///
/// Returns `None` if the tape width is not recognized.
pub fn tape_pixels(width_mm: u8, dpi: u16) -> Option<u16> {
    find_tape(width_mm, dpi).map(|t| t.pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_24mm_tape() {
        let tape = find_tape(24, 180).unwrap();
        assert_eq!(tape.pixels, 128);
        assert!((tape.margin_mm - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_find_unknown_tape() {
        assert!(find_tape(15, 180).is_none());
        assert!(find_tape(15, 360).is_none());
    }

    #[test]
    fn test_tape_count() {
        assert_eq!(supported_tapes(180).len(), 8);
        assert_eq!(supported_tapes(360).len(), 7);
    }

    #[test]
    fn test_tape_pixels_180() {
        assert_eq!(tape_pixels(36, 180), Some(192));
        assert_eq!(tape_pixels(4, 180), Some(24));
        assert_eq!(tape_pixels(99, 180), None);
    }

    #[test]
    fn test_tape_pixels_360() {
        // Official print areas, PT-9700PC ESC/P reference:
        // 24mm = 320 dots (positions 33-352 on the 384 pin head)
        assert_eq!(tape_pixels(24, 360), Some(320));
        assert_eq!(tape_pixels(18, 360), Some(234));
        assert_eq!(tape_pixels(12, 360), Some(150));
        assert_eq!(tape_pixels(9, 360), Some(106));
        assert_eq!(tape_pixels(6, 360), Some(64));
        // 36mm = 454 on the P900 560 pin head; 384 pin heads clamp to 384
        assert_eq!(tape_pixels(36, 360), Some(454));
        // 21mm has no documented 360 dpi print area
        assert_eq!(tape_pixels(21, 360), None);
    }
}
