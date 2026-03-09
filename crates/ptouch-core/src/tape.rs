// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Tape width information for Brother P-Touch label printers.
//!
//! Maps nominal tape widths (in mm) to printable pixel counts and margin sizes.

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

/// Static table of supported tape sizes.
static TAPE_TABLE: &[TapeInfo] = &[
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

/// Look up tape info by nominal width in millimeters.
///
/// Returns `Some(&TapeInfo)` if the width is recognized, `None` otherwise.
pub fn find_tape(width_mm: u8) -> Option<&'static TapeInfo> {
    TAPE_TABLE.iter().find(|t| t.width_mm == width_mm)
}

/// Returns the full list of supported tape sizes.
pub fn supported_tapes() -> &'static [TapeInfo] {
    TAPE_TABLE
}

/// Get the printable pixel count for a given tape width.
///
/// Returns `None` if the tape width is not recognized.
pub fn tape_pixels(width_mm: u8) -> Option<u16> {
    find_tape(width_mm).map(|t| t.pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_24mm_tape() {
        let tape = find_tape(24).unwrap();
        assert_eq!(tape.pixels, 128);
        assert!((tape.margin_mm - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_find_unknown_tape() {
        assert!(find_tape(15).is_none());
    }

    #[test]
    fn test_tape_count() {
        assert_eq!(supported_tapes().len(), 8);
    }

    #[test]
    fn test_tape_pixels() {
        assert_eq!(tape_pixels(36), Some(192));
        assert_eq!(tape_pixels(4), Some(24));
        assert_eq!(tape_pixels(99), None);
    }
}
