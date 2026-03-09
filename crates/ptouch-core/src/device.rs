// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Device definitions for Brother P-Touch label printers.
//!
//! Contains the supported device table, device flags, and lookup functions.

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

/// Brother USB vendor ID.
pub const BROTHER_VENDOR_ID: u16 = 0x04f9;

bitflags! {
    /// Device capability flags.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct DeviceFlags: u32 {
        /// No special flags.
        const NONE            = 0;
        /// Raster mode is unsupported on this device.
        const UNSUP_RASTER    = 1 << 0;
        /// Device uses PackBits compression for raster data.
        const RASTER_PACKBITS = 1 << 1;
        /// Device is in PLite mode (not supported by this driver).
        const PLITE           = 1 << 2;
        /// Device requires P700-style initialization sequence.
        const P700_INIT       = 1 << 3;
        /// Device supports the info command (ESC i z).
        const USE_INFO_CMD    = 1 << 4;
        /// Device has a pre-cut function.
        const HAS_PRECUT      = 1 << 5;
        /// Device requires D460BT magic initialization.
        const D460BT_MAGIC    = 1 << 6;
    }
}

/// Information about a supported P-Touch printer model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// USB vendor ID (always 0x04f9 for Brother).
    pub vid: u16,
    /// USB product ID.
    pub pid: u16,
    /// Human-readable model name.
    pub name: &'static str,
    /// Maximum number of printable pixels per raster line.
    pub max_px: u16,
    /// Print resolution in DPI.
    pub dpi: u16,
    /// Device capability flags.
    pub flags: DeviceFlags,
}

/// Static table of all supported P-Touch devices.
static DEVICE_TABLE: &[DeviceInfo] = &[
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2001,
        name: "PT-9200DX",
        max_px: 384,
        dpi: 360,
        flags: DeviceFlags::RASTER_PACKBITS.union(DeviceFlags::HAS_PRECUT),
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2002,
        name: "PT-9200DX",
        max_px: 384,
        dpi: 360,
        flags: DeviceFlags::RASTER_PACKBITS.union(DeviceFlags::HAS_PRECUT),
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2004,
        name: "PT-2300",
        max_px: 112,
        dpi: 180,
        flags: DeviceFlags::RASTER_PACKBITS.union(DeviceFlags::HAS_PRECUT),
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2007,
        name: "PT-2420PC",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::RASTER_PACKBITS,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2011,
        name: "PT-2450PC",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::RASTER_PACKBITS,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x201a,
        name: "PT-18R",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::RASTER_PACKBITS,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2019,
        name: "PT-1950",
        max_px: 112,
        dpi: 180,
        flags: DeviceFlags::RASTER_PACKBITS,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x201f,
        name: "PT-2700",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::HAS_PRECUT,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x202c,
        name: "PT-1230PC",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::NONE,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x202d,
        name: "PT-2430PC",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::NONE,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2030,
        name: "PT-1230PC (PLite Mode)",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::PLITE,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2031,
        name: "PT-2430PC (PLite Mode)",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::PLITE,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2041,
        name: "PT-2730",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::NONE,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x205e,
        name: "PT-H500",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::RASTER_PACKBITS.union(DeviceFlags::HAS_PRECUT),
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x205f,
        name: "PT-E500",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::RASTER_PACKBITS,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2060,
        name: "PT-E550W",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::UNSUP_RASTER,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2061,
        name: "PT-P700",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::RASTER_PACKBITS
            .union(DeviceFlags::P700_INIT)
            .union(DeviceFlags::HAS_PRECUT),
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2062,
        name: "PT-P750W",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::RASTER_PACKBITS.union(DeviceFlags::P700_INIT),
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2064,
        name: "PT-P700 (PLite Mode)",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::PLITE,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2065,
        name: "PT-P750W (PLite Mode)",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::PLITE,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x20df,
        name: "PT-D410",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::USE_INFO_CMD
            .union(DeviceFlags::HAS_PRECUT)
            .union(DeviceFlags::D460BT_MAGIC),
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2073,
        name: "PT-D450",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::USE_INFO_CMD,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x20e0,
        name: "PT-D460BT",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::P700_INIT
            .union(DeviceFlags::USE_INFO_CMD)
            .union(DeviceFlags::HAS_PRECUT)
            .union(DeviceFlags::D460BT_MAGIC),
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2074,
        name: "PT-D600",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::RASTER_PACKBITS,
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x20e1,
        name: "PT-D610BT",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::P700_INIT
            .union(DeviceFlags::USE_INFO_CMD)
            .union(DeviceFlags::HAS_PRECUT)
            .union(DeviceFlags::D460BT_MAGIC),
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x20af,
        name: "PT-P710BT",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::RASTER_PACKBITS.union(DeviceFlags::HAS_PRECUT),
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2201,
        name: "PT-E310BT",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::P700_INIT
            .union(DeviceFlags::USE_INFO_CMD)
            .union(DeviceFlags::D460BT_MAGIC),
    },
    DeviceInfo {
        vid: 0x04f9,
        pid: 0x2203,
        name: "PT-E560BT",
        max_px: 128,
        dpi: 180,
        flags: DeviceFlags::P700_INIT
            .union(DeviceFlags::USE_INFO_CMD)
            .union(DeviceFlags::D460BT_MAGIC),
    },
];

/// Find a device by USB vendor and product ID.
///
/// Returns `Some(&DeviceInfo)` if the device is in the supported table,
/// or `None` if not found.
pub fn find_device(vid: u16, pid: u16) -> Option<&'static DeviceInfo> {
    DEVICE_TABLE.iter().find(|d| d.vid == vid && d.pid == pid)
}

/// Returns the full list of supported devices.
pub fn supported_devices() -> &'static [DeviceInfo] {
    DEVICE_TABLE
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_known_device() {
        let dev = find_device(0x04f9, 0x2061).unwrap();
        assert_eq!(dev.name, "PT-P700");
        assert_eq!(dev.max_px, 128);
        assert_eq!(dev.dpi, 180);
        assert!(dev.flags.contains(DeviceFlags::RASTER_PACKBITS));
        assert!(dev.flags.contains(DeviceFlags::P700_INIT));
        assert!(dev.flags.contains(DeviceFlags::HAS_PRECUT));
    }

    #[test]
    fn test_find_unknown_device() {
        assert!(find_device(0x04f9, 0xFFFF).is_none());
    }

    #[test]
    fn test_device_count() {
        assert_eq!(supported_devices().len(), 28);
    }

    #[test]
    fn test_plite_device() {
        let dev = find_device(0x04f9, 0x2064).unwrap();
        assert!(dev.flags.contains(DeviceFlags::PLITE));
        assert_eq!(dev.name, "PT-P700 (PLite Mode)");
    }

    #[test]
    fn test_d460bt_magic() {
        let dev = find_device(0x04f9, 0x20e0).unwrap();
        assert!(dev.flags.contains(DeviceFlags::D460BT_MAGIC));
        assert!(dev.flags.contains(DeviceFlags::P700_INIT));
        assert!(dev.flags.contains(DeviceFlags::USE_INFO_CMD));
        assert!(dev.flags.contains(DeviceFlags::HAS_PRECUT));
    }
}
