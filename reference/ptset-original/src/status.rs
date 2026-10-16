//! 32-byte status reply to `ESC i S`, per Brother's PT-P900 raster command reference.

use anyhow::{Result, bail};
use std::fmt;

pub const LEN: usize = 32;

/// Invalidate (NULs), ESC @ initialize, ESC i S status request.
pub fn request() -> Vec<u8> {
    let mut cmd = vec![0u8; 100];
    cmd.extend_from_slice(&[0x1B, 0x40, 0x1B, 0x69, 0x53]);
    cmd
}

#[derive(Debug, Clone)]
pub struct Status {
    pub raw: [u8; LEN],
}

impl Status {
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < LEN || bytes[0] != 0x80 || bytes[1] != 0x20 || bytes[2] != b'B' {
            bail!("not a Brother status reply: {:02X?}", bytes);
        }
        let mut raw = [0u8; LEN];
        raw.copy_from_slice(&bytes[..LEN]);
        Ok(Self { raw })
    }
    pub fn series_code(&self) -> u8 { self.raw[3] }
    pub fn model_code(&self) -> u8 { self.raw[4] }
    pub fn error1(&self) -> u8 { self.raw[8] }
    pub fn error2(&self) -> u8 { self.raw[9] }
    pub fn media_width_mm(&self) -> u8 { self.raw[10] }
    pub fn media_type(&self) -> u8 { self.raw[11] }
    pub fn status_type(&self) -> u8 { self.raw[18] }
    pub fn tape_color(&self) -> u8 { self.raw[24] }
    pub fn text_color(&self) -> u8 { self.raw[25] }

    pub fn model_name(&self) -> &'static str {
        // Codes from DeviceSettings\PT900\brdvset.ini (SeriesCode 0x30).
        match (self.series_code(), self.model_code()) {
            (0x30, 0x71) => "PT-P900",
            (0x30, 0x6F) => "PT-P900W",
            (0x30, 0x70) => "PT-P950NW",
            _ => "unknown",
        }
    }

    pub fn errors(&self) -> Vec<&'static str> {
        const E1: [&str; 8] = ["no media", "end of media", "cutter jam", "weak batteries",
            "printer in use", "printer turned off", "high-voltage adapter", "fan motor error"];
        const E2: [&str; 8] = ["replace media", "expansion buffer full", "communication error",
            "communication buffer full", "cover open", "overheating", "black marking not detected",
            "system error"];
        let bits = |v: u8, names: &'static [&'static str; 8]| {
            (0..8).filter(move |i| v & (1 << i) != 0).map(move |i| names[i])
        };
        bits(self.error1(), &E1).chain(bits(self.error2(), &E2)).collect()
    }
}

fn media_type_name(v: u8) -> &'static str {
    match v {
        0x00 => "none",
        0x01 => "laminated tape",
        0x03 => "non-laminated tape",
        0x04 => "fabric tape",
        0x11 => "heat-shrink tube (2:1)",
        0x13 => "flexible ID tape",
        0x14 => "satin tape",
        0x17 => "heat-shrink tube (3:1)",
        0xFF => "incompatible tape",
        _ => "unknown",
    }
}

fn color_name(v: u8) -> &'static str {
    match v {
        0x01 => "white", 0x02 => "other", 0x03 => "clear", 0x04 => "red", 0x05 => "blue",
        0x06 => "yellow", 0x07 => "green", 0x08 => "black", 0x09 => "clear (white text)",
        0x20 => "matte white", 0x21 => "matte clear", 0x22 => "matte silver",
        0x23 => "satin gold", 0x24 => "satin silver", 0x30 => "blue (D)", 0x31 => "red (D)",
        0x40 => "fluorescent orange", 0x41 => "fluorescent yellow", 0x50 => "berry pink",
        0x51 => "light gray", 0x52 => "lime green", 0x60 => "yellow (F)", 0x61 => "pink (F)",
        0x62 => "blue (F)", 0x70 => "white (heat-shrink)", 0x90 => "white (flex)",
        0x91 => "yellow (flex)", 0xF0 => "cleaning", 0xF1 => "stencil", 0xFF => "incompatible",
        _ => "unknown",
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let errors = self.errors();
        writeln!(f, "Model:      {} (series 0x{:02X}, model 0x{:02X})",
            self.model_name(), self.series_code(), self.model_code())?;
        writeln!(f, "Errors:     {}", if errors.is_empty() { "none".into() } else { errors.join(", ") })?;
        writeln!(f, "Media:      {} mm {} (0x{:02X})",
            self.media_width_mm(), media_type_name(self.media_type()), self.media_type())?;
        writeln!(f, "Tape color: {} (0x{:02X})", color_name(self.tape_color()), self.tape_color())?;
        write!(f, "Text color: {} (0x{:02X})", color_name(self.text_color()), self.text_color())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Captured from the real PT-P900W on 2026-10-15.
    const CAPTURED: [u8; 32] = [
        0x80, 0x20, 0x42, 0x30, 0x6F, 0x30, 0x04, 0x00, 0x00, 0x00, 0x18, 0x01, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x08, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00,
    ];

    #[test]
    fn parses_captured_reply() {
        let s = Status::parse(&CAPTURED).unwrap();
        assert_eq!(s.model_name(), "PT-P900W");
        assert!(s.errors().is_empty());
        assert_eq!(s.media_width_mm(), 24);
        assert_eq!(color_name(s.tape_color()), "white");
        assert_eq!(color_name(s.text_color()), "black");
    }

    #[test]
    fn rejects_garbage() {
        assert!(Status::parse(&[0u8; 32]).is_err());
    }
}
