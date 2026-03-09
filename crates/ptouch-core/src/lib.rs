// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Brother P-Touch printer USB protocol library.
//!
//! This crate provides low-level communication with Brother P-Touch label
//! printers over USB. It handles device discovery, protocol command
//! construction, status parsing, and raster data transmission.

pub mod device;
pub mod error;
pub mod protocol;
pub mod status;
pub mod tape;
pub mod transport;

// Re-export commonly used types at the crate root.
pub use device::{DeviceFlags, DeviceInfo};
pub use error::{PtouchError, Result};
pub use status::PrinterStatus;
pub use tape::TapeInfo;
pub use transport::PtouchDevice;
