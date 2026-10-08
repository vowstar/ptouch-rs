// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Error types for the ptouch-core crate.

use thiserror::Error;

/// Result type alias using [`PtouchError`].
pub type Result<T> = std::result::Result<T, PtouchError>;

/// Errors that can occur when communicating with a Brother P-Touch printer.
#[derive(Debug, Error)]
pub enum PtouchError {
    /// Native Windows USB printer failure, preserving the operation and system error.
    #[error("USBPRINT {0}")]
    UsbPrint(String),

    /// USB communication error from rusb.
    #[error("USB error: {0}")]
    UsbError(#[from] rusb::Error),

    /// A detected device failed at a specific USB connection stage.
    #[error(
        "USB {stage} failed for {vid:04x}:{pid:04x}: {source}. Run `ptouch doctor` for driver and access diagnostics"
    )]
    UsbConnection {
        /// Operation that failed.
        stage: &'static str,
        /// Vendor ID.
        vid: u16,
        /// Product ID.
        pid: u16,
        /// Original libusb error.
        #[source]
        source: rusb::Error,
    },

    /// A printer interface has no usable bulk endpoint pair.
    #[error("No bulk IN/OUT pair in one alternate setting of USB interface 0")]
    InvalidUsbInterface,

    /// More than one printer matches the selection.
    #[error(
        "Multiple printers match; select --usb BUS:ADDRESS or --usbprint INSTANCE_ID from `ptouch doctor`"
    )]
    AmbiguousDevice,

    /// Native Bluetooth communication or setup error.
    #[error("Bluetooth error: {0}")]
    Bluetooth(String),

    /// A model does not support the requested operation.
    #[error("Unsupported operation: {0}")]
    UnsupportedOperation(&'static str),

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

    /// A printer transfer or status deadline expired.
    #[error("Printer communication timed out")]
    Timeout,

    /// Sending finished, but the printer did not confirm completion/readiness.
    #[error(
        "Label sent, but completion is unconfirmed. Check the printer before retrying; the label may already have printed"
    )]
    CompletionUnknown,

    /// The caller cancelled the operation. Already sent data cannot be recalled.
    #[error("Printer operation cancelled; already sent data may still print")]
    Cancelled,

    /// Input continued beyond the bounded drain operation.
    #[error("Printer input did not become idle within the drain limit")]
    InputNotIdle,

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

    /// The requested print quality is not supported by this device.
    #[error("Print quality not supported by {0}")]
    UnsupportedQuality(String),
}
