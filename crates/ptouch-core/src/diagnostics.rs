// SPDX-License-Identifier: GPL-3.0-or-later
//! Device discovery without printer commands or driver replacement.

use crate::device::{self, BROTHER_VENDOR_ID};
use crate::transport::{BulkEndpoints, UsbLocation, find_bulk_endpoints};
use rusb::{Context, UsbContext};
use serde::Serialize;

#[cfg(windows)]
mod windows;

/// A diagnostic stage, including the underlying error when available.
#[derive(Debug, Serialize)]
pub struct Check {
    /// `ok`, `error`, or `not_requested`.
    pub status: &'static str,
    /// Human-readable result; not intended as a stable error code.
    pub detail: String,
}

impl Check {
    fn result(result: rusb::Result<()>) -> Self {
        match result {
            Ok(()) => Self {
                status: "ok",
                detail: String::new(),
            },
            Err(error) => Self {
                status: "error",
                detail: format!("{error:?}: {error}"),
            },
        }
    }
    fn skipped() -> Self {
        Self {
            status: "not_requested",
            detail: "Use --probe to test opening and claiming; no print commands are sent".into(),
        }
    }
}

/// A printer observed through libusb, including unsupported Brother models.
#[derive(Debug, Serialize)]
pub struct UsbDiagnostic {
    /// Location usable with the CLI's `--usb` option.
    pub location: UsbLocation,
    /// Hexadecimal vendor ID.
    pub vid: String,
    /// Hexadecimal product ID.
    pub pid: String,
    /// Recognized model, if present in the table.
    pub model: Option<&'static str>,
    /// Descriptor read errors remain separate from opening errors.
    pub configuration_error: Option<String>,
    pub(crate) endpoints: Option<BulkEndpoints>,
    /// Open is optional because even opening a device can affect other users.
    pub open: Check,
    /// Probe never detaches a kernel driver or selects an alternate setting.
    pub claim: Check,
    /// Explicit release result when claim succeeded.
    pub release: Check,
}

/// Windows PnP evidence collected independently of libusb accessibility.
#[derive(Debug, Serialize)]
pub struct DriverBinding {
    /// Device instance ID. This can include a serial number.
    pub instance_id: String,
    /// Hardware IDs used by Windows driver matching.
    pub hardware_ids: Vec<String>,
    /// Bound service, such as `usbprint` or `WinUSB`.
    pub service: Option<String>,
    /// Device description from the registry.
    pub description: Option<String>,
}

/// A supported printer available through the native Windows USBPRINT backend.
#[derive(Debug, Serialize)]
pub struct UsbPrintDiagnostic {
    /// Explicit selector. Can contain a serial number.
    pub instance_id: String,
    /// Backend used by the normal application.
    pub backend: &'static str,
    /// Optional open/close check. Never sends a printer command.
    pub open: Check,
}

/// Versioned diagnostic report. Missing hardware is a report, not a crash.
#[derive(Debug, Serialize)]
pub struct DoctorReport {
    /// JSON schema version.
    pub schema_version: u32,
    /// Library version.
    pub version: &'static str,
    /// Operating system.
    pub os: &'static str,
    /// Architecture of this executable.
    pub process_arch: &'static str,
    /// Native architecture, if the operating system query succeeded.
    pub native_arch: Option<String>,
    /// Actual linked libusb version.
    pub libusb_version: String,
    /// Whether opening/claiming was requested.
    pub probe: bool,
    /// Errors from discovery itself, including partial enumeration failures.
    pub errors: Vec<String>,
    /// Brother USB devices exposed by libusb.
    pub devices: Vec<UsbDiagnostic>,
    /// Windows PnP devices, including those inaccessible to libusb.
    pub windows_bindings: Vec<DriverBinding>,
    /// Native USBPRINT candidates, independent of libusb opening support.
    pub usbprint_devices: Vec<UsbPrintDiagnostic>,
}

fn empty_report() -> DoctorReport {
    DoctorReport {
        schema_version: 1,
        version: env!("CARGO_PKG_VERSION"),
        os: std::env::consts::OS,
        process_arch: std::env::consts::ARCH,
        native_arch: None,
        libusb_version: format!(
            "{}.{}.{}",
            rusb::version().major(),
            rusb::version().minor(),
            rusb::version().micro()
        ),
        probe: false,
        errors: Vec::new(),
        devices: Vec::new(),
        windows_bindings: Vec::new(),
        usbprint_devices: Vec::new(),
    }
}

#[cfg(windows)]
pub(crate) fn native_bindings() -> crate::Result<Vec<DriverBinding>> {
    let mut report = empty_report();
    windows::inspect(&mut report);
    if !report.errors.is_empty() {
        return Err(crate::PtouchError::UsbPrint(format!(
            "PnP discovery: {}",
            report.errors.join("; ")
        )));
    }
    Ok(report.windows_bindings)
}

/// Inspect discovery and driver binding. `probe` also attempts open/claim/release.
/// No path sends printer commands, changes drivers, or detaches a kernel driver.
pub fn doctor(probe: bool) -> DoctorReport {
    let mut report = empty_report();
    report.probe = probe;
    #[cfg(windows)]
    {
        windows::inspect(&mut report);
        match crate::usbprint::list() {
            Ok(printers) => {
                for printer in printers {
                    let open = if probe {
                        match crate::usbprint::probe(&printer.instance_id) {
                            Ok(()) => Check {
                                status: "ok",
                                detail: "USBPRINT open/close succeeded".into(),
                            },
                            Err(error) => Check {
                                status: "error",
                                detail: error.to_string(),
                            },
                        }
                    } else {
                        Check { status: "not_requested", detail: "Use --probe to check USBPRINT open/close; no printer commands are sent".into() }
                    };
                    report.usbprint_devices.push(UsbPrintDiagnostic {
                        instance_id: printer.instance_id,
                        backend: "usbprint",
                        open,
                    });
                }
            }
            Err(error) => report.errors.push(error.to_string()),
        }
    }
    #[cfg(not(windows))]
    {
        report.native_arch = std::process::Command::new("uname")
            .arg("-m")
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned());
    }
    let devices = match Context::new().and_then(|context| context.devices()) {
        Ok(devices) => devices,
        Err(error) => {
            report.errors.push(format!("USB enumeration: {error}"));
            return report;
        }
    };
    for usb in devices.iter() {
        let desc = match usb.device_descriptor() {
            Ok(desc) => desc,
            Err(error) => {
                report
                    .errors
                    .push(format!("Descriptor at {}: {error}", UsbLocation::of(&usb)));
                continue;
            }
        };
        if desc.vendor_id() != BROTHER_VENDOR_ID {
            continue;
        }
        let mut diagnostic = UsbDiagnostic {
            location: UsbLocation::of(&usb),
            vid: format!("{:04x}", desc.vendor_id()),
            pid: format!("{:04x}", desc.product_id()),
            model: device::find_device(desc.vendor_id(), desc.product_id()).map(|info| info.name),
            configuration_error: None,
            endpoints: None,
            open: Check::skipped(),
            claim: Check::skipped(),
            release: Check::skipped(),
        };
        match usb
            .active_config_descriptor()
            .map_err(crate::PtouchError::from)
            .and_then(|config| find_bulk_endpoints(&config))
        {
            Ok(endpoints) => diagnostic.endpoints = Some(endpoints),
            Err(error) => diagnostic.configuration_error = Some(error.to_string()),
        }
        if probe && diagnostic.model.is_some() {
            for stage in [
                &mut diagnostic.open,
                &mut diagnostic.claim,
                &mut diagnostic.release,
            ] {
                *stage = Check {
                    status: "not_attempted",
                    detail: "An earlier stage did not succeed".into(),
                };
            }
            match usb.open() {
                Ok(handle) => {
                    diagnostic.open = Check::result(Ok(()));
                    diagnostic.claim = Check::result(handle.claim_interface(0));
                    if diagnostic.claim.status == "ok" {
                        diagnostic.release = Check::result(handle.release_interface(0));
                    }
                }
                Err(error) => diagnostic.open = Check::result(Err(error)),
            }
        }
        report.devices.push(diagnostic);
    }
    report
}
