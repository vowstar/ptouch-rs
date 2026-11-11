// SPDX-License-Identifier: GPL-3.0-or-later
//! Experimental P-touch status probe through the existing Windows USBPRINT driver.
//! Does not print, cut, reset, install drivers, or modify registry settings.

#[cfg(windows)]
#[path = "support/usbprint_windows.rs"]
mod platform;

#[cfg(windows)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    platform::run()
}

#[cfg(not(windows))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err(
        "usbprint_probe requires Windows; the application still uses libusb for USB printing"
            .into(),
    )
}
