// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Error types for the ptouch-core crate.

use thiserror::Error;

/// Result type alias using [`PtouchError`].
pub type Result<T> = std::result::Result<T, PtouchError>;

/// Errors that can occur when communicating with a Brother P-Touch printer.
#[derive(Debug, Error)]
pub enum PtouchError {
    /// USB communication error from rusb.
    #[error("USB error: {0}")]
    UsbError(#[from] rusb::Error),

    /// No matching device was found on the USB bus.
    #[error("Device not found")]
    DeviceNotFound,

    /// Device is in PLite mode, which is not supported by this driver.
    #[error("Device is in PLite mode: {0}")]
    PLiteMode(String),

    /// Device uses an unsupported raster format.
    #[error("Unsupported raster format: {0}")]
    UnsupportedRaster(String),

    /// The tape width reported by the device is not recognized.
    #[error("Unknown tape width: {0} mm")]
    UnknownTapeWidth(u8),

    /// An error was reported in the printer status.
    #[error("Status error: {0}")]
    StatusError(String),

    /// A USB transfer timed out.
    #[error("USB transfer timed out")]
    Timeout,

    /// The image height exceeds the maximum for the current tape.
    #[error("Image too large: height {height} exceeds max {max}")]
    ImageTooLarge {
        /// Actual height in pixels.
        height: u16,
        /// Maximum allowed height in pixels for the current tape.
        max: u16,
    },

    /// Failed to send data to the printer.
    #[error("Send failed: {0}")]
    SendFailed(String),

    /// The printer has not been initialized yet.
    #[error("Printer not initialized")]
    NotInitialized,
}
