// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Printer status packet parsing for Brother P-Touch printers.
//!
//! The printer returns a 32-byte status packet. This module parses
//! that packet and provides human-readable descriptions for media type,
//! tape color, and text color fields.

use serde::{Deserialize, Serialize};

/// Size of the printer status packet in bytes.
pub const STATUS_PACKET_SIZE: usize = 32;

/// Parsed printer status from a 32-byte status packet.
///
/// Field layout matches the Brother P-Touch status protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrinterStatus {
    /// Print head mark (byte 0).
    pub print_head_mark: u8,
    /// Size byte (byte 1).
    pub size: u8,
    /// Brother code (byte 2).
    pub brother_code: u8,
    /// Series code (byte 3).
    pub series_code: u8,
    /// Model code (byte 4).
    pub model_code: u8,
    /// Country code (byte 5).
    pub country_code: u8,
    /// Battery level (byte 6).
    pub battery_level: u8,
    /// Extended error byte (byte 7).
    pub extended_error: u8,
    /// Error info 1 (byte 8).
    pub error_info_1: u8,
    /// Error info 2 (byte 9).
    pub error_info_2: u8,
    /// Media width in millimeters (byte 10).
    pub media_width: u8,
    /// Media type (byte 11).
    pub media_type: u8,
    /// Number of colors (byte 12).
    pub num_colors: u8,
    /// Fonts (byte 13).
    pub fonts: u8,
    /// Japanese fonts (byte 14).
    pub japanese_fonts: u8,
    /// Mode (byte 15).
    pub mode: u8,
    /// Density (byte 16).
    pub density: u8,
    /// Media length in millimeters (byte 17).
    pub media_length: u8,
    /// Status type (byte 18).
    pub status_type: u8,
    /// Phase type (byte 19).
    pub phase_type: u8,
    /// Phase number high byte (byte 20).
    pub phase_number_hi: u8,
    /// Phase number low byte (byte 21).
    pub phase_number_lo: u8,
    /// Notification number (byte 22).
    pub notification_number: u8,
    /// Expansion area (byte 23).
    pub expansion_area: u8,
    /// Tape color info (byte 24).
    pub tape_color: u8,
    /// Text color info (byte 25).
    pub text_color: u8,
    /// Hardware settings 1 (byte 26).
    pub hw_settings_1: u8,
    /// Hardware settings 2 (byte 27).
    pub hw_settings_2: u8,
    /// Hardware settings 3 (byte 28).
    pub hw_settings_3: u8,
    /// Hardware settings 4 (byte 29).
    pub hw_settings_4: u8,
    /// Hardware settings 5 (byte 30).
    pub hw_settings_5: u8,
    /// Hardware settings 6 (byte 31).
    pub hw_settings_6: u8,
}

impl PrinterStatus {
    /// Parse a 32-byte status packet into a PrinterStatus struct.
    ///
    /// Returns `None` if the buffer is too short.
    pub fn from_bytes(buf: &[u8]) -> Option<Self> {
        if buf.len() < STATUS_PACKET_SIZE {
            return None;
        }
        Some(PrinterStatus {
            print_head_mark: buf[0],
            size: buf[1],
            brother_code: buf[2],
            series_code: buf[3],
            model_code: buf[4],
            country_code: buf[5],
            battery_level: buf[6],
            extended_error: buf[7],
            error_info_1: buf[8],
            error_info_2: buf[9],
            media_width: buf[10],
            media_type: buf[11],
            num_colors: buf[12],
            fonts: buf[13],
            japanese_fonts: buf[14],
            mode: buf[15],
            density: buf[16],
            media_length: buf[17],
            status_type: buf[18],
            phase_type: buf[19],
            phase_number_hi: buf[20],
            phase_number_lo: buf[21],
            notification_number: buf[22],
            expansion_area: buf[23],
            tape_color: buf[24],
            text_color: buf[25],
            hw_settings_1: buf[26],
            hw_settings_2: buf[27],
            hw_settings_3: buf[28],
            hw_settings_4: buf[29],
            hw_settings_5: buf[30],
            hw_settings_6: buf[31],
        })
    }

    /// Serialize this status back into a 32-byte array.
    pub fn to_bytes(&self) -> [u8; STATUS_PACKET_SIZE] {
        [
            self.print_head_mark,
            self.size,
            self.brother_code,
            self.series_code,
            self.model_code,
            self.country_code,
            self.battery_level,
            self.extended_error,
            self.error_info_1,
            self.error_info_2,
            self.media_width,
            self.media_type,
            self.num_colors,
            self.fonts,
            self.japanese_fonts,
            self.mode,
            self.density,
            self.media_length,
            self.status_type,
            self.phase_type,
            self.phase_number_hi,
            self.phase_number_lo,
            self.notification_number,
            self.expansion_area,
            self.tape_color,
            self.text_color,
            self.hw_settings_1,
            self.hw_settings_2,
            self.hw_settings_3,
            self.hw_settings_4,
            self.hw_settings_5,
            self.hw_settings_6,
        ]
    }

    /// Returns true if any error bits are set.
    pub fn has_error(&self) -> bool {
        self.error_info_1 != 0 || self.error_info_2 != 0
    }

    /// Describe the error bits as a human-readable string.
    pub fn error_description(&self) -> String {
        let mut errors = Vec::new();

        // Error info 1 bits
        if self.error_info_1 & 0x01 != 0 {
            errors.push("No media");
        }
        if self.error_info_1 & 0x02 != 0 {
            errors.push("End of media");
        }
        if self.error_info_1 & 0x04 != 0 {
            errors.push("Cutter jam");
        }
        if self.error_info_1 & 0x08 != 0 {
            errors.push("Weak battery");
        }
        if self.error_info_1 & 0x10 != 0 {
            errors.push("Printer in use");
        }
        if self.error_info_1 & 0x20 != 0 {
            errors.push("Printer turned off");
        }
        if self.error_info_1 & 0x40 != 0 {
            errors.push("High-voltage adapter");
        }
        if self.error_info_1 & 0x80 != 0 {
            errors.push("Fan motor error");
        }

        // Error info 2 bits
        if self.error_info_2 & 0x01 != 0 {
            errors.push("Replace media");
        }
        if self.error_info_2 & 0x02 != 0 {
            errors.push("Expansion buffer full");
        }
        if self.error_info_2 & 0x04 != 0 {
            errors.push("Communication error");
        }
        if self.error_info_2 & 0x08 != 0 {
            errors.push("Communication buffer full");
        }
        if self.error_info_2 & 0x10 != 0 {
            errors.push("Cover open");
        }
        if self.error_info_2 & 0x20 != 0 {
            errors.push("Cancel key");
        }
        if self.error_info_2 & 0x40 != 0 {
            errors.push("Media cannot be fed");
        }
        if self.error_info_2 & 0x80 != 0 {
            errors.push("System error");
        }

        if errors.is_empty() {
            "No errors".to_string()
        } else {
            errors.join(", ")
        }
    }

    /// Get the human-readable media type name.
    pub fn media_type_name(&self) -> &'static str {
        media_type_name(self.media_type)
    }

    /// Get the human-readable tape color name.
    pub fn tape_color_name(&self) -> &'static str {
        tape_color_name(self.tape_color)
    }

    /// Get the human-readable text color name.
    pub fn text_color_name(&self) -> &'static str {
        text_color_name(self.text_color)
    }

    /// Status type: 0x00 = reply to status request, 0x01 = printing completed,
    /// 0x02 = error occurred, 0x04 = IF mode finished, 0x05 = power off,
    /// 0x06 = notification, 0x07 = phase change.
    pub fn status_type_name(&self) -> &'static str {
        match self.status_type {
            0x00 => "Reply to status request",
            0x01 => "Printing completed",
            0x02 => "Error occurred",
            0x04 => "IF mode finished",
            0x05 => "Power off",
            0x06 => "Notification",
            0x07 => "Phase change",
            _ => "Unknown",
        }
    }
}

/// Get the human-readable media type name for a media type byte.
pub fn media_type_name(media_type: u8) -> &'static str {
    match media_type {
        0x00 => "No media",
        0x01 => "Laminated tape",
        0x03 => "Non-laminated tape",
        0x11 => "Heat-shrink tube (2:1)",
        0x17 => "Heat-shrink tube (3:1)",
        0xFF => "Incompatible tape",
        _ => "Unknown media type",
    }
}

/// Get the human-readable tape color name for a tape color byte.
pub fn tape_color_name(tape_color: u8) -> &'static str {
    match tape_color {
        0x01 => "White",
        0x02 => "Other",
        0x03 => "Clear",
        0x04 => "Red",
        0x05 => "Blue",
        0x06 => "Yellow",
        0x07 => "Green",
        0x08 => "Black",
        0x09 => "Clear (white text)",
        0x20 => "Matte white",
        0x21 => "Matte clear",
        0x22 => "Matte silver",
        0x23 => "Satin gold",
        0x24 => "Satin silver",
        0x30 => "Blue (D)",
        0x31 => "Red (D)",
        0x40 => "Fluorescent orange",
        0x41 => "Fluorescent yellow",
        0x50 => "Berry pink (S)",
        0x51 => "Light gray (S)",
        0x52 => "Lime green (S)",
        0x60 => "Yellow (F)",
        0x61 => "Pink (F)",
        0x62 => "Blue (F)",
        0x70 => "White (heat-shrink tube)",
        0x90 => "White (flex. ID)",
        0x91 => "Yellow (flex. ID)",
        0xF0 => "Cleaning",
        0xF1 => "Stencil",
        0xFF => "Incompatible",
        _ => "Unknown tape color",
    }
}

/// Get the human-readable text color name for a text color byte.
pub fn text_color_name(text_color: u8) -> &'static str {
    match text_color {
        0x01 => "White",
        0x04 => "Red",
        0x05 => "Blue",
        0x08 => "Black",
        0x0A => "Gold",
        0xF0 => "Cleaning",
        0xF1 => "Stencil",
        0x02 => "Other",
        0xFF => "Incompatible",
        _ => "Unknown text color",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_status() {
        let mut buf = [0u8; 32];
        buf[10] = 24; // media width = 24mm
        buf[11] = 0x01; // laminated tape
        buf[24] = 0x01; // white tape
        buf[25] = 0x08; // black text

        let status = PrinterStatus::from_bytes(&buf).unwrap();
        assert_eq!(status.media_width, 24);
        assert_eq!(status.media_type_name(), "Laminated tape");
        assert_eq!(status.tape_color_name(), "White");
        assert_eq!(status.text_color_name(), "Black");
        assert!(!status.has_error());
    }

    #[test]
    fn test_short_buffer() {
        let buf = [0u8; 16];
        assert!(PrinterStatus::from_bytes(&buf).is_none());
    }

    #[test]
    fn test_error_detection() {
        let mut buf = [0u8; 32];
        buf[8] = 0x01; // No media error
        let status = PrinterStatus::from_bytes(&buf).unwrap();
        assert!(status.has_error());
        assert!(status.error_description().contains("No media"));
    }

    #[test]
    fn test_roundtrip() {
        let mut buf = [0u8; 32];
        for (i, b) in buf.iter_mut().enumerate() {
            *b = i as u8;
        }
        let status = PrinterStatus::from_bytes(&buf).unwrap();
        assert_eq!(status.to_bytes(), buf);
    }
}
