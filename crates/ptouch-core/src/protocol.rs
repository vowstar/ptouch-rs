// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Protocol command construction for Brother P-Touch printers.
//!
//! All functions in this module are pure -- they construct command byte
//! sequences as `Vec<u8>` without performing any I/O. The transport layer
//! is responsible for sending these commands to the printer.

use crate::device::DeviceFlags;

/// Construct the initialization sequence.
///
/// Sends 100 zero bytes followed by the ESC @ (initialize) command.
/// This resets the printer to a known state.
pub fn cmd_init() -> Vec<u8> {
    let mut buf = vec![0u8; 100];
    buf.push(0x1B); // ESC
    buf.push(0x40); // @
    buf
}

/// Construct the status request command.
///
/// Sends ESC i S to request a 32-byte status packet from the printer.
pub fn cmd_status_request() -> Vec<u8> {
    vec![0x1B, 0x69, 0x53]
}

/// Construct the raster start command.
///
/// For devices with FLAG_P700_INIT, uses "ESC i a 0x01" (switch to raster mode).
/// For other devices, uses "ESC i R 0x01" (standard raster start).
pub fn cmd_raster_start(flags: DeviceFlags) -> Vec<u8> {
    if flags.contains(DeviceFlags::P700_INIT) {
        vec![0x1B, 0x69, 0x61, 0x01]
    } else {
        vec![0x1B, 0x69, 0x52, 0x01]
    }
}

/// Construct the PackBits compression enable command.
///
/// Sends M 0x02 to enable PackBits compression for raster data.
pub fn cmd_enable_packbits() -> Vec<u8> {
    vec![0x4D, 0x02]
}

/// Construct a line feed command.
///
/// Sends 0x5A to advance one raster line without printing.
pub fn cmd_line_feed() -> Vec<u8> {
    vec![0x5A]
}

/// Construct a form feed command.
///
/// Sends 0x0C to end the current page.
pub fn cmd_form_feed() -> Vec<u8> {
    vec![0x0C]
}

/// Construct the finalize (end of print) command.
///
/// In eject mode, sends 0x1A to eject and cut the tape.
/// In chain mode, sends 0x0C (form feed), except for D460BT-type devices
/// which always use 0x1A.
pub fn cmd_finalize(chain_print: bool, flags: DeviceFlags) -> Vec<u8> {
    if chain_print && !flags.contains(DeviceFlags::D460BT_MAGIC) {
        vec![0x0C]
    } else {
        vec![0x1A]
    }
}

/// Construct the info command (ESC i z).
///
/// Sets media width and raster line count. Used by devices with
/// FLAG_USE_INFO_CMD.
///
/// Format: ESC i z 0x00 0x00 <width_mm> 0x00
///         <raster_lines as 4 LE bytes> <n9> 0x00
///
/// For D460BT devices, n9 is set to 0x02 to feed the last label properly.
pub fn cmd_info(media_width: u8, raster_lines: u32, flags: DeviceFlags) -> Vec<u8> {
    let mut buf = vec![0x1B, 0x69, 0x7A]; // ESC i z
    buf.push(0x00); // n1: quality flags
    buf.push(0x00); // n2: media type
    buf.push(media_width); // n3: media width
    buf.push(0x00); // n4
    buf.push((raster_lines & 0xFF) as u8); // n5: raster lines (LE)
    buf.push(((raster_lines >> 8) & 0xFF) as u8);
    buf.push(((raster_lines >> 16) & 0xFF) as u8);
    buf.push(((raster_lines >> 24) & 0xFF) as u8);
    // n9: D460BT feed control (0x02 to feed last label properly)
    buf.push(if flags.contains(DeviceFlags::D460BT_MAGIC) {
        0x02
    } else {
        0x00
    });
    buf.push(0x00); // n10
    buf
}

/// Construct the pre-cut on/off command.
///
/// Sends ESC i M followed by 0x40 (auto-cut on) or 0x00 (auto-cut off).
pub fn cmd_precut(enabled: bool) -> Vec<u8> {
    vec![0x1B, 0x69, 0x4D, if enabled { 0x40 } else { 0x00 }]
}

/// Construct the D460BT magic initialization command.
///
/// Sends ESC i d 0x01 0x00 0x4D 0x00 0x00.
/// The trailing 0x00 is a required NUL terminator.
pub fn cmd_d460bt_magic() -> Vec<u8> {
    vec![0x1B, 0x69, 0x64, 0x01, 0x00, 0x4D, 0x00, 0x00]
}

/// Construct the D460BT chain print command.
///
/// Sends ESC i K 0x00 0x00.
/// The trailing 0x00 is a required NUL terminator.
pub fn cmd_d460bt_chain() -> Vec<u8> {
    vec![0x1B, 0x69, 0x4B, 0x00, 0x00]
}

/// Construct a raster data command without PackBits compression.
///
/// Format: G <length as 2 LE bytes> <data>
pub fn cmd_send_raster(data: &[u8]) -> Vec<u8> {
    let len = data.len() as u16;
    let mut buf = Vec::with_capacity(3 + data.len());
    buf.push(0x47); // G
    buf.push((len & 0xFF) as u8);
    buf.push((len >> 8) as u8);
    buf.extend_from_slice(data);
    buf
}

/// Construct a raster data command with PackBits "fake" compression.
///
/// The PackBits variant wraps the data with a header byte that indicates
/// the run length. For our purposes, we send uncompressed data with the
/// PackBits framing: G <total_len as 2 LE bytes> <(data.len()-1) as u8> <data>.
///
/// This is a "fake" PackBits encoding where the entire line is sent as a
/// single literal run.
pub fn cmd_send_raster_packbits(data: &[u8]) -> Vec<u8> {
    if data.is_empty() {
        return cmd_send_raster(data);
    }
    // PackBits literal run: control byte = (count - 1) for literal run
    let control_byte = (data.len() - 1) as u8;
    let total_len = (1 + data.len()) as u16;
    let mut buf = Vec::with_capacity(3 + 1 + data.len());
    buf.push(0x47); // G
    buf.push((total_len & 0xFF) as u8);
    buf.push((total_len >> 8) as u8);
    buf.push(control_byte);
    buf.extend_from_slice(data);
    buf
}

/// Construct the page flags / margin command.
///
/// Sends ESC i d followed by 2 bytes for the margin value (little-endian).
pub fn cmd_page_flags(margin: u16) -> Vec<u8> {
    vec![
        0x1B,
        0x69,
        0x64,
        (margin & 0xFF) as u8,
        ((margin >> 8) & 0xFF) as u8,
    ]
}

/// Create a blank raster line of the given width in pixels.
///
/// Returns a zeroed byte buffer of length `ceil(max_px / 8)`.
pub fn rasterline_blank(max_px: u16) -> Vec<u8> {
    vec![0u8; (max_px as usize).div_ceil(8)]
}

/// Check if a raster line is entirely blank (all zeros).
pub fn rasterline_is_blank(line: &[u8]) -> bool {
    line.iter().all(|&b| b == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cmd_init() {
        let cmd = cmd_init();
        assert_eq!(cmd.len(), 102);
        assert!(cmd[..100].iter().all(|&b| b == 0));
        assert_eq!(cmd[100], 0x1B);
        assert_eq!(cmd[101], 0x40);
    }

    #[test]
    fn test_cmd_status_request() {
        assert_eq!(cmd_status_request(), vec![0x1B, 0x69, 0x53]);
    }

    #[test]
    fn test_cmd_raster_start_normal() {
        let cmd = cmd_raster_start(DeviceFlags::NONE);
        assert_eq!(cmd, vec![0x1B, 0x69, 0x52, 0x01]);
    }

    #[test]
    fn test_cmd_raster_start_p700() {
        let cmd = cmd_raster_start(DeviceFlags::P700_INIT);
        assert_eq!(cmd, vec![0x1B, 0x69, 0x61, 0x01]);
    }

    #[test]
    fn test_cmd_finalize_eject() {
        assert_eq!(cmd_finalize(false, DeviceFlags::NONE), vec![0x1A]);
    }

    #[test]
    fn test_cmd_finalize_chain() {
        assert_eq!(cmd_finalize(true, DeviceFlags::NONE), vec![0x0C]);
    }

    #[test]
    fn test_cmd_finalize_chain_d460bt() {
        // D460BT always uses eject
        assert_eq!(cmd_finalize(true, DeviceFlags::D460BT_MAGIC), vec![0x1A]);
    }

    #[test]
    fn test_cmd_precut() {
        assert_eq!(cmd_precut(true), vec![0x1B, 0x69, 0x4D, 0x40]);
        assert_eq!(cmd_precut(false), vec![0x1B, 0x69, 0x4D, 0x00]);
    }

    #[test]
    fn test_cmd_send_raster() {
        let data = vec![0xFF, 0x00, 0xAA];
        let cmd = cmd_send_raster(&data);
        assert_eq!(cmd[0], 0x47);
        assert_eq!(cmd[1], 0x03); // length low
        assert_eq!(cmd[2], 0x00); // length high
        assert_eq!(&cmd[3..], &data[..]);
    }

    #[test]
    fn test_cmd_send_raster_packbits() {
        let data = vec![0xFF, 0x00, 0xAA];
        let cmd = cmd_send_raster_packbits(&data);
        assert_eq!(cmd[0], 0x47);
        assert_eq!(cmd[1], 0x04); // total_len = 1 + 3 = 4
        assert_eq!(cmd[2], 0x00);
        assert_eq!(cmd[3], 0x02); // control byte = 3 - 1 = 2
        assert_eq!(&cmd[4..], &data[..]);
    }

    #[test]
    fn test_cmd_info() {
        let cmd = cmd_info(24, 100, DeviceFlags::NONE);
        assert_eq!(cmd.len(), 13);
        assert_eq!(cmd[0], 0x1B);
        assert_eq!(cmd[1], 0x69);
        assert_eq!(cmd[2], 0x7A);
        assert_eq!(cmd[3], 0x00); // n1: quality flags = 0
        assert_eq!(cmd[4], 0x00); // n2: media type = 0
        assert_eq!(cmd[5], 24); // n3: width
        assert_eq!(cmd[6], 0x00); // n4
        assert_eq!(cmd[7], 100); // n5: raster lines low byte
        assert_eq!(cmd[8], 0x00);
        assert_eq!(cmd[9], 0x00);
        assert_eq!(cmd[10], 0x00);
        assert_eq!(cmd[11], 0x00); // n9: not D460BT
        assert_eq!(cmd[12], 0x00); // n10
    }

    #[test]
    fn test_cmd_info_d460bt() {
        let cmd = cmd_info(24, 100, DeviceFlags::D460BT_MAGIC);
        assert_eq!(cmd[11], 0x02); // n9: D460BT feed control
    }

    #[test]
    fn test_cmd_d460bt_magic() {
        assert_eq!(
            cmd_d460bt_magic(),
            vec![0x1B, 0x69, 0x64, 0x01, 0x00, 0x4D, 0x00, 0x00]
        );
    }

    #[test]
    fn test_cmd_d460bt_chain() {
        assert_eq!(cmd_d460bt_chain(), vec![0x1B, 0x69, 0x4B, 0x00, 0x00]);
    }

    #[test]
    fn test_cmd_page_flags() {
        let cmd = cmd_page_flags(0x0300);
        assert_eq!(cmd, vec![0x1B, 0x69, 0x64, 0x00, 0x03]);
    }
}
