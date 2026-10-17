// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Bounded barcode primitives shared by editing, preview, and print rendering.

use serde::{Deserialize, Serialize};

use crate::bitmap::LabelBitmap;
use crate::{RenderError, Result};

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
}
