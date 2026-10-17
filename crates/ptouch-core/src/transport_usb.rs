// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>
// SPDX-FileCopyrightText: Dominic Radermacher and the ptouch-print contributors

use crate::session::Transport;
use crate::{CancellationToken, PtouchError, Result};
use rusb::{Context, DeviceHandle};
use std::time::{Duration, Instant};

const USB_INTERFACE: u8 = 0;
pub(super) const USB_WRITE_POLL_TIMEOUT: Duration = Duration::from_millis(250);
const USB_MIN_TIMEOUT: Duration = Duration::from_millis(1);
const USB_ZERO_WRITE_DELAY: Duration = Duration::from_millis(10);

pub(super) struct UsbTransport {
    handle: DeviceHandle<Context>,
    ep_out: u8,
    ep_in: u8,
    cancellation: CancellationToken,
}

impl UsbTransport {
    pub(super) fn set_cancellation(&mut self, token: CancellationToken) {
        self.cancellation = token;
    }
}

impl Transport for UsbTransport {
    fn send(&self, data: &[u8], timeout: Duration) -> Result<()> {
        let started = Instant::now();
        send_usb_bytes(
            data,
            timeout,
            &self.cancellation,
            |remaining, poll_timeout| self.handle.write_bulk(self.ep_out, remaining, poll_timeout),
            || started.elapsed(),
            std::thread::sleep,
        )
    }

    fn receive(&self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        let read = self
            .handle
            .read_bulk(self.ep_in, buf, timeout)
            .map_err(|e| {
                if e == rusb::Error::Timeout {
                    PtouchError::Timeout
                } else {
                    PtouchError::UsbError(e)
                }
            })?;
        Ok(read)
    }

    fn close(self) -> Result<()> {
        self.handle.release_interface(USB_INTERFACE)?;
        Ok(())
    }
}

pub(super) enum UsbBackend {
    Libusb(UsbTransport),
    #[cfg(windows)]
    UsbPrint(crate::usbprint::UsbPrintTransport),
}

impl Transport for UsbBackend {
    fn send(&self, data: &[u8], timeout: Duration) -> Result<()> {
        match self {
            Self::Libusb(transport) => transport.send(data, timeout),
            #[cfg(windows)]
            Self::UsbPrint(transport) => transport.send(data, timeout),
        }
    }

    fn receive(&self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        match self {
            Self::Libusb(transport) => transport.receive(buf, timeout),
            #[cfg(windows)]
            Self::UsbPrint(transport) => transport.receive(buf, timeout),
        }
    }

    fn close(self) -> Result<()> {
        match self {
            Self::Libusb(transport) => transport.close(),
            #[cfg(windows)]
            Self::UsbPrint(transport) => transport.close(),
        }
    }
}

pub(super) fn send_usb_bytes(
    data: &[u8],
    timeout: Duration,
    cancellation: &CancellationToken,
    mut write: impl FnMut(&[u8], Duration) -> rusb::Result<usize>,
    elapsed: impl Fn() -> Duration,
    mut pause: impl FnMut(Duration),
) -> Result<()> {
    let mut offset = 0;
    while offset < data.len() {
        cancellation.check()?;
        let remaining = timeout
            .checked_sub(elapsed())
            .filter(|duration| *duration >= USB_MIN_TIMEOUT)
            .ok_or(PtouchError::Timeout)?;
        let pending = &data[offset..];
        match write(pending, remaining.min(USB_WRITE_POLL_TIMEOUT)) {
            Ok(written) if written > pending.len() => {
                return Err(PtouchError::SendFailed(format!(
                    "USB write reported {written} bytes for a {}-byte buffer",
                    pending.len()
                )));
            }
            Ok(0) => pause(timeout.saturating_sub(elapsed()).min(USB_ZERO_WRITE_DELAY)),
            Ok(written) => offset += written,
            Err(rusb::Error::Timeout) => {}
            Err(error) => return Err(PtouchError::UsbError(error)),
        }
    }
    Ok(())
}

#[path = "transport_usb_discovery.rs"]
mod discovery;

pub use discovery::UsbLocation;
pub(super) use discovery::open_matching;
pub(crate) use discovery::{BulkEndpoints, find_bulk_endpoints};
#[cfg(test)]
pub(super) use discovery::{only_candidate, select_bulk_endpoints};
