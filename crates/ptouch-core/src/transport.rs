// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! USB transport layer for Brother P-Touch printers.
//!
//! Provides the [`PtouchDevice`] struct for opening, initializing, and
//! communicating with a P-Touch printer over USB.

use std::time::Duration;

use log::{debug, info, warn};
use rusb::{Context, DeviceHandle, UsbContext};

use crate::device::{self, DeviceFlags, DeviceInfo, BROTHER_VENDOR_ID};
use crate::error::{PtouchError, Result};
use crate::protocol;
use crate::status::{PrinterStatus, STATUS_PACKET_SIZE};
use crate::tape;

/// Default USB timeout for bulk transfers.
const USB_TIMEOUT: Duration = Duration::from_secs(5);

/// USB interface number for P-Touch printers.
const USB_INTERFACE: u8 = 0;

/// A connection to a Brother P-Touch USB printer.
pub struct PtouchDevice {
    /// USB device handle.
    handle: DeviceHandle<Context>,
    /// Device information from the supported device table.
    dev_info: DeviceInfo,
    /// Bulk OUT endpoint address.
    ep_out: u8,
    /// Bulk IN endpoint address.
    ep_in: u8,
    /// Most recently read printer status.
    status: Option<PrinterStatus>,
    /// Tape width in pixels (resolved after status query).
    tape_width_px: Option<u16>,
    /// Whether the device has been initialized.
    initialized: bool,
}

impl PtouchDevice {
    /// Open a P-Touch printer by USB vendor/product ID.
    ///
    /// Scans the USB bus for a device matching the given VID/PID, looks it up
    /// in the supported device table, claims the USB interface, and returns
    /// a [`PtouchDevice`] ready for initialization.
    ///
    /// # Errors
    ///
    /// Returns [`PtouchError::DeviceNotFound`] if no matching USB device is
    /// found or the device is not in the supported table. Returns
    /// [`PtouchError::PLiteMode`] if the device is in PLite mode. Returns
    /// [`PtouchError::UnsupportedRaster`] if the device does not support
    /// raster printing.
    pub fn open(vid: u16, pid: u16) -> Result<Self> {
        let dev_info = device::find_device(vid, pid)
            .ok_or(PtouchError::DeviceNotFound)?
            .clone();

        if dev_info.flags.contains(DeviceFlags::PLITE) {
            return Err(PtouchError::PLiteMode(dev_info.name.to_string()));
        }

        if dev_info.flags.contains(DeviceFlags::UNSUP_RASTER) {
            return Err(PtouchError::UnsupportedRaster(dev_info.name.to_string()));
        }

        info!(
            "Opening device: {} (VID={:#06x}, PID={:#06x})",
            dev_info.name, vid, pid
        );

        let context = Context::new()?;
        let handle = context
            .open_device_with_vid_pid(vid, pid)
            .ok_or(PtouchError::DeviceNotFound)?;

        // Detach kernel driver if active
        if handle.kernel_driver_active(USB_INTERFACE).unwrap_or(false) {
            debug!("Detaching kernel driver from interface {}", USB_INTERFACE);
            handle.detach_kernel_driver(USB_INTERFACE)?;
        }

        handle.claim_interface(USB_INTERFACE)?;

        // Find the bulk endpoints
        let (ep_out, ep_in) = find_bulk_endpoints(&handle)?;
        debug!("Endpoints: OUT={:#04x}, IN={:#04x}", ep_out, ep_in);

        Ok(PtouchDevice {
            handle,
            dev_info,
            ep_out,
            ep_in,
            status: None,
            tape_width_px: None,
            initialized: false,
        })
    }

    /// Open the first Brother P-Touch printer found on the USB bus.
    ///
    /// Scans all USB devices, looking for any with the Brother vendor ID
    /// that matches an entry in the supported device table.
    pub fn open_first() -> Result<Self> {
        let context = Context::new()?;
        let devices = context.devices()?;

        for usb_dev in devices.iter() {
            let desc = match usb_dev.device_descriptor() {
                Ok(d) => d,
                Err(_) => continue,
            };

            if desc.vendor_id() != BROTHER_VENDOR_ID {
                continue;
            }

            if let Some(dev_info) = device::find_device(desc.vendor_id(), desc.product_id()) {
                if dev_info.flags.contains(DeviceFlags::PLITE)
                    || dev_info.flags.contains(DeviceFlags::UNSUP_RASTER)
                {
                    continue;
                }

                return Self::open(desc.vendor_id(), desc.product_id());
            }
        }

        Err(PtouchError::DeviceNotFound)
    }

    /// Get a reference to the device info.
    pub fn device_info(&self) -> &DeviceInfo {
        &self.dev_info
    }

    /// Get the device flags.
    pub fn flags(&self) -> DeviceFlags {
        self.dev_info.flags
    }

    /// Get the most recently read printer status, if available.
    pub fn status(&self) -> Option<&PrinterStatus> {
        self.status.as_ref()
    }

    /// Get the tape width in pixels, if known.
    pub fn tape_width_px(&self) -> Option<u16> {
        self.tape_width_px
    }

    /// Get the maximum printable pixels for this device.
    pub fn max_px(&self) -> u16 {
        self.dev_info.max_px
    }

    /// Whether the device has been initialized.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Send raw bytes to the printer (bulk OUT transfer).
    pub fn send(&self, data: &[u8]) -> Result<()> {
        let written = self
            .handle
            .write_bulk(self.ep_out, data, USB_TIMEOUT)
            .map_err(|e| {
                if e == rusb::Error::Timeout {
                    PtouchError::Timeout
                } else {
                    PtouchError::UsbError(e)
                }
            })?;

        if written != data.len() {
            return Err(PtouchError::SendFailed(format!(
                "Expected to write {} bytes, wrote {}",
                data.len(),
                written
            )));
        }

        Ok(())
    }

    /// Receive raw bytes from the printer (bulk IN transfer).
    ///
    /// Returns the number of bytes actually read into `buf`.
    pub fn receive(&self, buf: &mut [u8]) -> Result<usize> {
        let read = self
            .handle
            .read_bulk(self.ep_in, buf, USB_TIMEOUT)
            .map_err(|e| {
                if e == rusb::Error::Timeout {
                    PtouchError::Timeout
                } else {
                    PtouchError::UsbError(e)
                }
            })?;

        Ok(read)
    }

    /// Initialize the printer.
    ///
    /// Sends the init sequence, queries the status, and resolves the tape width.
    /// For devices with FLAG_P700_INIT, sends the P700-style raster start.
    /// For devices with FLAG_D460BT_MAGIC, sends the magic init sequence.
    pub fn init(&mut self) -> Result<()> {
        // Send the init command (100 zeros + ESC @)
        self.send(&protocol::cmd_init())?;

        // For P700-style devices, send raster mode switch
        if self.dev_info.flags.contains(DeviceFlags::P700_INIT) {
            self.send(&protocol::cmd_raster_start(self.dev_info.flags))?;
        }

        // Request and read status
        self.get_status()?;

        // D460BT magic init if needed
        if self.dev_info.flags.contains(DeviceFlags::D460BT_MAGIC) {
            self.send(&protocol::cmd_d460bt_magic())?;
        }

        self.initialized = true;
        info!(
            "Device initialized: {}, tape={}mm ({}px)",
            self.dev_info.name,
            self.status.as_ref().map_or(0, |s| s.media_width),
            self.tape_width_px.unwrap_or(0)
        );

        Ok(())
    }

    /// Request and read the printer status.
    ///
    /// Sends the status request command and reads the 32-byte response.
    /// Updates internal status and tape width fields.
    pub fn get_status(&mut self) -> Result<&PrinterStatus> {
        self.send(&protocol::cmd_status_request())?;

        let mut buf = [0u8; STATUS_PACKET_SIZE];
        let read = self.receive(&mut buf)?;

        if read < STATUS_PACKET_SIZE {
            return Err(PtouchError::StatusError(format!(
                "Status packet too short: {} bytes (expected {})",
                read, STATUS_PACKET_SIZE
            )));
        }

        let status = PrinterStatus::from_bytes(&buf)
            .ok_or_else(|| PtouchError::StatusError("Failed to parse status packet".to_string()))?;

        debug!(
            "Status: type={}, media_width={}mm, media_type={}, tape_color={}, text_color={}",
            status.status_type_name(),
            status.media_width,
            status.media_type_name(),
            status.tape_color_name(),
            status.text_color_name()
        );

        if status.has_error() {
            warn!("Printer reports error: {}", status.error_description());
        }

        // Resolve tape width to pixel count
        self.tape_width_px = tape::tape_pixels(status.media_width);
        if self.tape_width_px.is_none() && status.media_width != 0 {
            warn!("Unknown tape width: {} mm", status.media_width);
        }

        self.status = Some(status);

        // The unwrap is safe because we just assigned Some above
        Ok(self.status.as_ref().unwrap())
    }

    /// Print raster image data.
    ///
    /// `lines` is a slice of raster line buffers, each `ceil(max_px/8)` bytes
    /// wide. The printer will print one raster line per entry.
    ///
    /// # Arguments
    /// * `lines` - Raster image data, one byte-slice per line.
    /// * `chain_print` - If true, don't cut the tape (chain mode).
    ///
    /// # Errors
    ///
    /// Returns [`PtouchError::NotInitialized`] if [`init`](Self::init) was not called.
    /// Returns [`PtouchError::ImageTooLarge`] if any line exceeds the tape width.
    pub fn print_raster(&mut self, lines: &[Vec<u8>], chain_print: bool) -> Result<()> {
        if !self.initialized {
            return Err(PtouchError::NotInitialized);
        }

        let flags = self.dev_info.flags;
        let use_packbits = flags.contains(DeviceFlags::RASTER_PACKBITS);
        let use_info = flags.contains(DeviceFlags::USE_INFO_CMD);
        let has_precut = flags.contains(DeviceFlags::HAS_PRECUT);
        let is_d460bt = flags.contains(DeviceFlags::D460BT_MAGIC);

        // For P700 and non-P700, start raster mode
        if !flags.contains(DeviceFlags::P700_INIT) {
            self.send(&protocol::cmd_raster_start(flags))?;
        }

        // Send info command if supported
        if use_info {
            let media_type = self.status.as_ref().map_or(0x00, |s| s.media_type);
            let media_width = self.status.as_ref().map_or(0, |s| s.media_width);
            let raster_lines = lines.len() as u32;
            // flags: 0x80 (recover) | 0x40 (various quality)
            self.send(&protocol::cmd_info(
                media_type,
                media_width,
                raster_lines,
                0xC0,
            ))?;
        }

        // Enable PackBits if supported
        if use_packbits {
            self.send(&protocol::cmd_enable_packbits())?;
        }

        // Set pre-cut if supported
        if has_precut {
            self.send(&protocol::cmd_precut(!chain_print))?;
        }

        // Set margin
        self.send(&protocol::cmd_page_flags(0))?;

        // Send raster lines
        for line in lines {
            if protocol::rasterline_is_blank(line) {
                self.send(&protocol::cmd_line_feed())?;
            } else if use_packbits {
                self.send(&protocol::cmd_send_raster_packbits(line))?;
            } else {
                self.send(&protocol::cmd_send_raster(line))?;
            }
        }

        // D460BT chain command
        if is_d460bt && chain_print {
            self.send(&protocol::cmd_d460bt_chain())?;
        }

        // Finalize
        self.send(&protocol::cmd_finalize(chain_print, flags))?;

        // Wait for printing completed status
        let mut response_buf = [0u8; STATUS_PACKET_SIZE];
        match self.receive(&mut response_buf) {
            Ok(n) if n >= STATUS_PACKET_SIZE => {
                if let Some(status) = PrinterStatus::from_bytes(&response_buf) {
                    if status.has_error() {
                        return Err(PtouchError::StatusError(status.error_description()));
                    }
                    debug!("Print completed: status_type={}", status.status_type_name());
                    self.status = Some(status);
                }
            }
            Ok(n) => {
                debug!("Short status response after print: {} bytes", n);
            }
            Err(PtouchError::Timeout) => {
                debug!("Timeout waiting for print completion status");
            }
            Err(e) => return Err(e),
        }

        Ok(())
    }

    /// Release the USB interface and close the device.
    pub fn close(self) -> Result<()> {
        self.handle.release_interface(USB_INTERFACE)?;
        info!("Device closed: {}", self.dev_info.name);
        Ok(())
    }
}

/// Find the bulk IN and OUT endpoints for the printer interface.
fn find_bulk_endpoints(handle: &DeviceHandle<Context>) -> Result<(u8, u8)> {
    let device = handle.device();
    let config = device.active_config_descriptor()?;

    let mut ep_out: Option<u8> = None;
    let mut ep_in: Option<u8> = None;

    for interface in config.interfaces() {
        for desc in interface.descriptors() {
            if desc.interface_number() != USB_INTERFACE {
                continue;
            }
            for endpoint in desc.endpoint_descriptors() {
                if endpoint.transfer_type() != rusb::TransferType::Bulk {
                    continue;
                }
                match endpoint.direction() {
                    rusb::Direction::Out => {
                        ep_out = Some(endpoint.address());
                    }
                    rusb::Direction::In => {
                        ep_in = Some(endpoint.address());
                    }
                }
            }
        }
    }

    match (ep_out, ep_in) {
        (Some(out), Some(inp)) => Ok((out, inp)),
        _ => Err(PtouchError::DeviceNotFound),
    }
}
