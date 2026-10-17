// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>
// SPDX-FileCopyrightText: Dominic Radermacher and the ptouch-print contributors
//
// Portions derived from ptouch-print, licensed GPL-3.0-or-later:
// https://git.familie-radermacher.ch/linux/ptouch-print.git

//! Printer connection facade and transport selection.

#[path = "transport_tcp.rs"]
mod tcp;
#[path = "transport_usb.rs"]
mod usb;
#[cfg(test)]
#[path = "transport/usb_discovery_test.rs"]
mod usb_discovery_tests;
#[cfg(test)]
#[path = "transport/usb_io_test.rs"]
mod usb_io_tests;
#[cfg(test)]
#[path = "transport/virtual_printer_test.rs"]
mod virtual_printer_tests;

use usb::UsbBackend;
pub use usb::UsbLocation;
pub(crate) use usb::{BulkEndpoints, find_bulk_endpoints};
#[cfg(test)]
use usb::{USB_WRITE_POLL_TIMEOUT, only_candidate, select_bulk_endpoints, send_usb_bytes};

use crate::model::{Dialect, ModelProfile};
use crate::session::{PrinterSession, Transport};
use crate::{
    CancellationToken,
    device::{self, BROTHER_VENDOR_ID, DeviceFlags, DeviceInfo},
    error::{PtouchError, Result},
    protocol,
    status::PrinterStatus,
};
use std::{net::SocketAddr, time::Duration};

/// Default raw TCP printing port used by Brother network printers.
pub const TCP_PRINTING_PORT: u16 = 9100;

enum PrinterTransport {
    Usb(UsbBackend),
    Tcp(tcp::TcpTransport),
}

impl PrinterTransport {
    fn tcp_address(&self) -> Option<SocketAddr> {
        match self {
            Self::Usb(_) => None,
            Self::Tcp(transport) => Some(transport.address()),
        }
    }
}

impl Transport for PrinterTransport {
    fn send(&self, data: &[u8], timeout: Duration) -> Result<()> {
        match self {
            Self::Usb(backend) => backend.send(data, timeout),
            Self::Tcp(backend) => backend.send(data, timeout),
        }
    }

    fn receive(&self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        match self {
            Self::Usb(backend) => backend.receive(buf, timeout),
            Self::Tcp(backend) => backend.receive(buf, timeout),
        }
    }

    fn close(self) -> Result<()> {
        match self {
            Self::Usb(backend) => backend.close(),
            Self::Tcp(backend) => backend.close(),
        }
    }
}

fn validate_device(dev_info: &DeviceInfo) -> Result<()> {
    if dev_info.flags.contains(DeviceFlags::PLITE) {
        return Err(PtouchError::PLiteMode(dev_info.name.to_string()));
    }
    if dev_info.flags.contains(DeviceFlags::UNSUP_RASTER) {
        return Err(PtouchError::UnsupportedRaster(dev_info.name.to_string()));
    }
    Ok(())
}

/// A connection to a Brother P-Touch printer.
///
/// USB constructors remain available alongside explicit TCP connections.
pub struct PtouchDevice {
    session: PrinterSession<PrinterTransport>,
    dev_info: DeviceInfo,
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
        let (transport, dev_info) = usb::open_matching(Some((vid, pid)), None)?;
        Ok(Self::from_usb(transport, dev_info))
    }

    /// Open a printer at the bus/address reported by `ptouch doctor`.
    /// Addresses can change after reconnecting the printer.
    pub fn open_at(location: UsbLocation) -> Result<Self> {
        let (transport, dev_info) = usb::open_matching(None, Some(location))?;
        Ok(Self::from_usb(transport, dev_info))
    }

    /// Open the only supported printer. Multiple matches require selection.
    pub fn open_first() -> Result<Self> {
        #[cfg(windows)]
        if let Some(instance_id) = crate::usbprint::automatic()? {
            return Self::open_usbprint(&instance_id);
        }
        let (transport, dev_info) = usb::open_matching(None, None)?;
        Ok(Self::from_usb(transport, dev_info))
    }

    /// Connect to a supported printer at a caller-selected TCP socket address.
    ///
    /// The address includes its port. Brother network printers commonly use
    /// [`TCP_PRINTING_PORT`]. The product ID selects the printer model.
    ///
    /// # Errors
    ///
    /// Returns [`PtouchError::DeviceNotFound`] for an unknown product ID,
    /// [`PtouchError::PLiteMode`] for a PLite profile,
    /// [`PtouchError::UnsupportedRaster`] for an unsupported profile, and
    /// [`PtouchError::Timeout`] if the connection deadline expires.
    /// Other socket failures are returned as [`PtouchError::TcpError`].
    pub fn open_tcp(address: SocketAddr, pid: u16) -> Result<Self> {
        let dev_info = device::find_device(BROTHER_VENDOR_ID, pid)
            .ok_or(PtouchError::DeviceNotFound)?
            .clone();
        validate_device(&dev_info)?;
        let transport = tcp::TcpTransport::connect(address)?;
        let mut profile = ModelProfile::from(&dev_info);
        profile.dialect = Dialect::Tcp;
        Ok(Self {
            session: PrinterSession::new(PrinterTransport::Tcp(transport), profile),
            dev_info,
        })
    }

    /// Open an explicitly selected PT-P710BT through the existing Windows driver.
    #[cfg(windows)]
    pub fn open_usbprint(instance_id: &str) -> Result<Self> {
        let dev_info = device::find_device(BROTHER_VENDOR_ID, 0x20af)
            .ok_or(PtouchError::DeviceNotFound)?
            .clone();
        validate_device(&dev_info)?;
        let transport = crate::usbprint::UsbPrintTransport::open(instance_id)?;
        Ok(Self::from_usb_backend(
            UsbBackend::UsbPrint(transport),
            dev_info,
        ))
    }

    fn from_usb(transport: usb::UsbTransport, dev_info: DeviceInfo) -> Self {
        Self::from_usb_backend(UsbBackend::Libusb(transport), dev_info)
    }

    fn from_usb_backend(backend: UsbBackend, dev_info: DeviceInfo) -> Self {
        Self {
            session: PrinterSession::new(PrinterTransport::Usb(backend), (&dev_info).into()),
            dev_info,
        }
    }

    /// Set a fresh cancellation token before initialization or a job.
    /// USB and TCP writes check cancellation at bounded intervals; reads return
    /// within the configured transfer timeout.
    pub fn set_cancellation_token(&mut self, token: CancellationToken) {
        match &mut self.session.transport {
            PrinterTransport::Usb(UsbBackend::Libusb(transport)) => {
                transport.set_cancellation(token.clone());
            }
            #[cfg(windows)]
            PrinterTransport::Usb(UsbBackend::UsbPrint(transport)) => {
                transport.set_cancellation(token.clone());
            }
            PrinterTransport::Tcp(transport) => transport.set_cancellation(token.clone()),
        }
        self.session.cancellation = token;
    }

    /// Set the maximum time for sending and waiting for one label (default: 600 seconds).
    pub fn set_job_timeout(&mut self, timeout: Duration) -> Result<()> {
        if timeout < Duration::from_millis(1) {
            return Err(PtouchError::UnsupportedOperation(
                "Job timeout must be at least one millisecond",
            ));
        }
        self.session.job_timeout = timeout;
        Ok(())
    }

    /// Get a reference to the selected printer's device info.
    pub fn device_info(&self) -> &DeviceInfo {
        &self.dev_info
    }

    /// Get the device flags.
    pub fn flags(&self) -> DeviceFlags {
        self.session.flags()
    }

    /// Get the most recently read printer status.
    pub fn status(&self) -> Option<&PrinterStatus> {
        self.session.status()
    }

    /// Get the tape width in pixels, if known.
    pub fn tape_width_px(&self) -> Option<u16> {
        self.session.tape_width_px()
    }

    /// Get the maximum printable pixels for this device.
    pub fn max_px(&self) -> u16 {
        self.session.raster_width_px()
    }

    /// Whether the printer has been initialized.
    pub fn is_initialized(&self) -> bool {
        self.session.is_initialized()
    }

    pub fn is_tcp(&self) -> bool {
        self.session.transport.tcp_address().is_some()
    }

    pub fn set_media_width_override(&mut self, width_code: u8) -> Result<()> {
        self.session.set_media_width_override(width_code)
    }

    #[cfg(test)]
    pub(crate) fn set_media_details_for_test(&mut self, media_type: u8, text_color: u8) {
        self.session
            .set_media_details_for_test(media_type, text_color);
    }

    /// Send raw bytes through the selected transport.
    pub fn send(&self, data: &[u8]) -> Result<()> {
        self.session.send(data)
    }

    /// Receive raw bytes through the selected transport.
    pub fn receive(&self, buf: &mut [u8]) -> Result<usize> {
        self.session.receive(buf)
    }

    /// Initialize the printer and query status.
    pub fn init(&mut self) -> Result<()> {
        self.session.init()
    }

    /// Query status without resetting the printer.
    pub fn query_status(&mut self) -> Result<&PrinterStatus> {
        if let Some(address) = self.session.transport.tcp_address() {
            if !self.flags().contains(DeviceFlags::SNMP_STATUS) {
                return Err(PtouchError::UnsupportedOperation(
                    "Network status is unavailable for this model",
                ));
            }
            let community = crate::network_status::community_from_environment();
            let status = crate::network_status::query_p900w_status(address, &community)?;
            return self.session.update_network_status(status);
        }
        self.session.query_status()
    }

    /// Request and read the printer status.
    pub fn get_status(&mut self) -> Result<&PrinterStatus> {
        self.query_status()
    }

    /// Print raster lines with the selected printer's job options.
    pub fn print_raster(
        &mut self,
        lines: &[Vec<u8>],
        chain_print: bool,
        precut: bool,
        quality: protocol::PrintQuality,
    ) -> Result<()> {
        self.session
            .print_raster(lines, chain_print, precut, quality)
    }

    /// Feed tape forward and cut.
    pub fn feed_and_cut(&mut self) -> Result<()> {
        self.session.feed_and_cut()
    }

    /// Close the selected printer transport.
    pub fn close(self) -> Result<()> {
        self.session.close()
    }
}
