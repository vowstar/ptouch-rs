// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>
// SPDX-FileCopyrightText: Dominic Radermacher and the ptouch-print contributors
//
// Portions derived from ptouch-print, licensed GPL-3.0-or-later:
// https://git.familie-radermacher.ch/linux/ptouch-print.git

//! USB discovery and backwards-compatible P-Touch device API.

use crate::session::{PrinterSession, Transport};
use crate::{
    CancellationToken,
    device::{self, DeviceFlags, DeviceInfo},
    error::{PtouchError, Result},
    protocol,
    status::PrinterStatus,
};
use log::{debug, info};
use rusb::{Context, Device, DeviceHandle, UsbContext};
use std::time::{Duration, Instant};

const USB_INTERFACE: u8 = 0;
const USB_WRITE_POLL_TIMEOUT: Duration = Duration::from_millis(250);
const USB_MIN_TIMEOUT: Duration = Duration::from_millis(1);
const USB_ZERO_WRITE_DELAY: Duration = Duration::from_millis(10);

struct UsbTransport {
    handle: DeviceHandle<Context>,
    ep_out: u8,
    ep_in: u8,
    cancellation: CancellationToken,
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

/// Continue a USB byte stream through flow-control pauses without replaying bytes.
///
/// rusb 0.9 returns `Ok(n)` when libusb times out after a partial transfer, and
/// `Err(Timeout)` only when no bytes were transferred. Keep that byte count here;
/// retrying the transport-independent `send` operation could repeat a prefix.
/// The clock and pause callbacks make deadline and cancellation tests deterministic.
fn send_usb_bytes(
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

enum UsbBackend {
    Libusb(UsbTransport),
    #[cfg(windows)]
    UsbPrint(crate::usbprint::UsbPrintTransport),
}
impl Transport for UsbBackend {
    fn send(&self, data: &[u8], timeout: Duration) -> Result<()> {
        match self {
            Self::Libusb(t) => t.send(data, timeout),
            #[cfg(windows)]
            Self::UsbPrint(t) => t.send(data, timeout),
        }
    }
    fn receive(&self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        match self {
            Self::Libusb(t) => t.receive(buf, timeout),
            #[cfg(windows)]
            Self::UsbPrint(t) => t.receive(buf, timeout),
        }
    }
    fn close(self) -> Result<()> {
        match self {
            Self::Libusb(t) => t.close(),
            #[cfg(windows)]
            Self::UsbPrint(t) => t.close(),
        }
    }
}

/// A connection to a Brother P-Touch USB printer.
///
/// This facade keeps the USB constructors and method signatures unchanged.
pub struct PtouchDevice {
    session: PrinterSession<UsbBackend>,
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
        Self::open_matching(Some((vid, pid)), None)
    }

    /// Open a printer at the bus/address reported by `ptouch doctor`.
    /// Addresses can change after reconnecting the printer.
    pub fn open_at(location: UsbLocation) -> Result<Self> {
        Self::open_matching(None, Some(location))
    }

    /// Open the only supported printer. Multiple matches require selection.
    pub fn open_first() -> Result<Self> {
        #[cfg(windows)]
        if let Some(instance_id) = crate::usbprint::automatic()? {
            return Self::open_usbprint(&instance_id);
        }
        Self::open_matching(None, None)
    }

    /// Open an explicitly selected PT-P710BT through the existing Windows driver.
    #[cfg(windows)]
    pub fn open_usbprint(instance_id: &str) -> Result<Self> {
        let dev_info = device::find_device(0x04f9, 0x20af).unwrap().clone();
        let transport = crate::usbprint::UsbPrintTransport::open(instance_id)?;
        Ok(Self {
            session: PrinterSession::new(UsbBackend::UsbPrint(transport), (&dev_info).into()),
            dev_info,
        })
    }

    fn open_matching(ids: Option<(u16, u16)>, location: Option<UsbLocation>) -> Result<Self> {
        let context = Context::new()?;
        let devices = context.devices()?;
        let mut candidates = Vec::new();
        for usb in devices.iter() {
            if location.is_some_and(|where_| where_ != UsbLocation::of(&usb)) {
                continue;
            }
            let desc = match usb.device_descriptor() {
                Ok(desc) => desc,
                Err(error) if location.is_some() => return Err(error.into()),
                Err(_) => continue,
            };
            if ids.is_some_and(|pair| pair != (desc.vendor_id(), desc.product_id())) {
                continue;
            }
            let Some(info) = device::find_device(desc.vendor_id(), desc.product_id()) else {
                continue;
            };
            if ids.is_none()
                && location.is_none()
                && info
                    .flags
                    .intersects(DeviceFlags::PLITE | DeviceFlags::UNSUP_RASTER)
            {
                continue;
            }
            candidates.push((usb, info.clone()));
        }
        let (usb, info) = only_candidate(candidates)?;
        Self::open_device(usb, info)
    }

    fn open_device(usb: Device<Context>, dev_info: DeviceInfo) -> Result<Self> {
        if dev_info.flags.contains(DeviceFlags::PLITE) {
            return Err(PtouchError::PLiteMode(dev_info.name.to_string()));
        }
        if dev_info.flags.contains(DeviceFlags::UNSUP_RASTER) {
            return Err(PtouchError::UnsupportedRaster(dev_info.name.to_string()));
        }
        let connection_error = |stage, source| PtouchError::UsbConnection {
            stage,
            vid: dev_info.vid,
            pid: dev_info.pid,
            source,
        };
        // Open the enumerated device itself, preserving the original error.
        let handle = usb.open().map_err(|e| connection_error("open", e))?;
        let config = usb
            .active_config_descriptor()
            .map_err(|e| connection_error("read configuration", e))?;
        let endpoints = find_bulk_endpoints(&config)?;
        if handle.kernel_driver_active(USB_INTERFACE).unwrap_or(false) {
            // libusb reattaches the kernel driver when the handle is dropped.
            handle
                .set_auto_detach_kernel_driver(true)
                .map_err(|e| connection_error("enable kernel driver detach", e))?;
        }
        handle
            .claim_interface(USB_INTERFACE)
            .map_err(|e| connection_error("claim interface", e))?;
        endpoints
            .activate(|alternate| handle.set_alternate_setting(USB_INTERFACE, alternate))
            .map_err(|e| connection_error("select alternate setting", e))?;
        info!("Opened {} at {}", dev_info.name, UsbLocation::of(&usb));
        debug!(
            "Endpoints: OUT={:#04x}, IN={:#04x}, alternate={}",
            endpoints.out, endpoints.input, endpoints.alternate
        );
        Ok(Self {
            session: PrinterSession::new(
                UsbBackend::Libusb(UsbTransport {
                    handle,
                    ep_out: endpoints.out,
                    ep_in: endpoints.input,
                    cancellation: CancellationToken::default(),
                }),
                (&dev_info).into(),
            ),
            dev_info,
        })
    }

    /// Set a fresh cancellation token before initialization or a job.
    /// USB writes check cancellation between transfers bounded to 250 milliseconds.
    /// A blocked status read returns within its transfer timeout.
    pub fn set_cancellation_token(&mut self, token: crate::CancellationToken) {
        match &mut self.session.transport {
            UsbBackend::Libusb(transport) => transport.cancellation = token.clone(),
            #[cfg(windows)]
            UsbBackend::UsbPrint(transport) => transport.set_cancellation(token.clone()),
        }
        self.session.cancellation = token;
    }

    /// Set the maximum time for sending and waiting for one USB label (default: 600 seconds).
    pub fn set_job_timeout(&mut self, timeout: Duration) -> Result<()> {
        if timeout < Duration::from_millis(1) {
            return Err(PtouchError::UnsupportedOperation(
                "Job timeout must be at least one millisecond",
            ));
        }
        self.session.job_timeout = timeout;
        Ok(())
    }

    /// Get a reference to the USB device info.
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
    /// Send raw bytes through the USB OUT endpoint.
    pub fn send(&self, data: &[u8]) -> Result<()> {
        self.session.send(data)
    }
    /// Receive raw bytes from the USB IN endpoint.
    pub fn receive(&self, buf: &mut [u8]) -> Result<usize> {
        self.session.receive(buf)
    }
    /// Initialize the printer and query status.
    pub fn init(&mut self) -> Result<()> {
        self.session.init()
    }
    /// Query status without resetting the printer.
    pub fn query_status(&mut self) -> Result<&PrinterStatus> {
        self.session.query_status()
    }
    /// Probe status without reset, allowing an absent PT-E550W status reply.
    pub fn probe_status(&mut self) -> Result<Option<&PrinterStatus>> {
        self.session.probe_status()
    }
    /// Prepare a job using a manual TZe width when PT-E550W status is unavailable.
    pub fn prepare_print(&mut self, manual_width_mm: Option<u8>) -> Result<()> {
        self.session.prepare_print(manual_width_mm)
    }
    /// Submit a job and distinguish physical completion from an unconfirmed send.
    pub fn print_raster_outcome(
        &mut self,
        lines: &[Vec<u8>],
        chain_print: bool,
        precut: bool,
        quality: protocol::PrintQuality,
    ) -> Result<crate::PrintOutcome> {
        self.session
            .print_raster_outcome(lines, chain_print, precut, quality)
    }
    /// Request and read the printer status.
    pub fn get_status(&mut self) -> Result<&PrinterStatus> {
        self.session.get_status()
    }
    /// Print raster lines with the existing USB job options.
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
    /// Release the USB interface and close the device.
    pub fn close(self) -> Result<()> {
        self.session.close()
    }
}

/// USB location within the current enumeration; reconnecting can change it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct UsbLocation {
    /// USB bus number.
    pub bus: u8,
    /// USB device address.
    pub address: u8,
}

impl UsbLocation {
    pub(crate) fn of<T: UsbContext>(device: &Device<T>) -> Self {
        Self {
            bus: device.bus_number(),
            address: device.address(),
        }
    }
}

impl std::fmt::Display for UsbLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.bus, self.address)
    }
}

impl std::str::FromStr for UsbLocation {
    type Err = &'static str;
    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        let (bus, address) = value.split_once(':').ok_or("expected BUS:ADDRESS")?;
        Ok(Self {
            bus: bus.parse().map_err(|_| "invalid USB bus (0..255)")?,
            address: address
                .parse()
                .map_err(|_| "invalid USB address (0..255)")?,
        })
    }
}

fn only_candidate<T>(mut candidates: Vec<T>) -> Result<T> {
    match candidates.len() {
        0 => Err(PtouchError::DeviceNotFound),
        1 => Ok(candidates.remove(0)),
        _ => Err(PtouchError::AmbiguousDevice),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct BulkEndpoints {
    pub alternate: u8,
    pub out: u8,
    pub input: u8,
    #[serde(skip)]
    requires_selection: bool,
}

impl BulkEndpoints {
    fn activate(&self, select: impl FnOnce(u8) -> rusb::Result<()>) -> rusb::Result<()> {
        // A sole setting zero is already active. Some printers reject SET_INTERFACE
        // in this case. With multiple settings, explicitly select the chosen one,
        // including zero, because a previous user could have activated another.
        if self.requires_selection {
            select(self.alternate)?;
        }
        Ok(())
    }
}

/// Select a pair from one alternate setting, preferring setting zero.
pub(crate) fn find_bulk_endpoints(config: &rusb::ConfigDescriptor) -> Result<BulkEndpoints> {
    select_bulk_endpoints(config.interfaces().flat_map(|interface| {
        interface
            .descriptors()
            .filter(|desc| desc.interface_number() == USB_INTERFACE)
            .map(|desc| {
                (
                    desc.setting_number(),
                    desc.endpoint_descriptors()
                        .filter(|ep| ep.transfer_type() == rusb::TransferType::Bulk)
                        .map(|ep| ep.address())
                        .collect::<Vec<_>>(),
                )
            })
    }))
}

fn select_bulk_endpoints(
    settings: impl IntoIterator<Item = (u8, Vec<u8>)>,
) -> Result<BulkEndpoints> {
    // Count every setting, including those without a usable bulk endpoint pair.
    let settings: Vec<_> = settings.into_iter().collect();
    let setting_count = settings.len();
    settings
        .into_iter()
        .filter_map(|(alternate, addresses)| {
            if addresses.iter().any(|ep| ep & 0x70 != 0 || ep & 0x0f == 0) {
                return None;
            }
            let mut out = addresses
                .iter()
                .copied()
                .filter(|ep| *ep != 0 && *ep & 0x80 == 0);
            let mut input = addresses
                .iter()
                .copied()
                .filter(|ep| *ep & 0x80 != 0 && *ep != 0x80);
            let pair = BulkEndpoints {
                alternate,
                out: out.next()?,
                input: input.next()?,
                requires_selection: setting_count != 1 || alternate != 0,
            };
            // Multiple pairs in one setting need an explicit model-specific policy.
            if out.next().is_some() || input.next().is_some() {
                None
            } else {
                Some(pair)
            }
        })
        .min_by_key(|pair| pair.alternate)
        .ok_or(PtouchError::InvalidUsbInterface)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn usb_write_resumes_only_the_unwritten_suffix_after_partial_progress() {
        let mut writes = Vec::new();
        let mut results = [Ok(2), Err(rusb::Error::Timeout), Ok(4)].into_iter();
        send_usb_bytes(
            &[1, 2, 3, 4, 5, 6],
            Duration::from_secs(30),
            &CancellationToken::default(),
            |data, timeout| {
                assert_eq!(timeout, USB_WRITE_POLL_TIMEOUT);
                writes.push(data.to_vec());
                results.next().expect("unexpected replay")
            },
            || Duration::ZERO,
            |_| panic!("nonzero writes do not need pacing"),
        )
        .unwrap();
        assert_eq!(
            writes,
            [vec![1, 2, 3, 4, 5, 6], vec![3, 4, 5, 6], vec![3, 4, 5, 6]]
        );
    }

    #[test]
    fn usb_write_checks_cancellation_before_sending_and_after_a_partial_transfer() {
        for cancel_before_write in [true, false] {
            let token = CancellationToken::default();
            if cancel_before_write {
                token.cancel();
            }
            let mut writes = 0;
            let result = send_usb_bytes(
                &[1, 2, 3],
                Duration::from_secs(30),
                &token,
                |_, _| {
                    writes += 1;
                    token.cancel();
                    Ok(1)
                },
                || Duration::ZERO,
                |_| {},
            );
            assert!(matches!(result, Err(PtouchError::Cancelled)));
            assert_eq!(writes, usize::from(!cancel_before_write));
        }
    }

    #[test]
    fn usb_write_uses_one_deadline_across_progress_and_timeouts() {
        let elapsed = Cell::new(Duration::ZERO);
        let mut poll_timeouts = Vec::new();
        let mut writes = Vec::new();
        let result = send_usb_bytes(
            &[1, 2, 3],
            Duration::from_millis(600),
            &CancellationToken::default(),
            |data, timeout| {
                poll_timeouts.push(timeout);
                writes.push(data.to_vec());
                elapsed.set(elapsed.get() + timeout);
                if writes.len() == 1 {
                    Ok(1)
                } else {
                    Err(rusb::Error::Timeout)
                }
            },
            || elapsed.get(),
            |_| {},
        );
        assert!(matches!(result, Err(PtouchError::Timeout)));
        assert_eq!(
            poll_timeouts,
            [
                Duration::from_millis(250),
                Duration::from_millis(250),
                Duration::from_millis(100)
            ]
        );
        assert_eq!(writes, [vec![1, 2, 3], vec![2, 3], vec![2, 3]]);
    }

    #[test]
    fn usb_write_aborts_on_disconnect_or_other_non_timeout_errors() {
        for error in [
            rusb::Error::NoDevice,
            rusb::Error::Pipe,
            rusb::Error::Io,
            rusb::Error::Interrupted,
        ] {
            let mut writes = 0;
            let result = send_usb_bytes(
                &[1, 2, 3],
                Duration::from_secs(30),
                &CancellationToken::default(),
                |data, _| {
                    writes += 1;
                    match writes {
                        1 => Ok(1),
                        2 => {
                            assert_eq!(data, [2, 3]);
                            Err(error)
                        }
                        _ => panic!("must not retry a failed transfer"),
                    }
                },
                || Duration::ZERO,
                |_| {},
            );
            assert!(matches!(result, Err(PtouchError::UsbError(actual)) if actual == error));
            assert_eq!(writes, 2);
        }
    }

    #[test]
    fn usb_write_never_passes_a_submillisecond_timeout_to_libusb() {
        for already_elapsed in [Duration::ZERO, Duration::from_millis(1)] {
            let mut writes = 0;
            let result = send_usb_bytes(
                &[1],
                already_elapsed + Duration::from_micros(999),
                &CancellationToken::default(),
                |_, _| {
                    writes += 1;
                    Ok(1)
                },
                || already_elapsed,
                |_| {},
            );
            assert!(matches!(result, Err(PtouchError::Timeout)));
            assert_eq!(writes, 0);
        }
    }

    #[test]
    fn usb_zero_length_writes_are_paced_and_still_obey_the_deadline() {
        let elapsed = Cell::new(Duration::ZERO);
        let mut pauses = Vec::new();
        let mut writes = 0;
        let result = send_usb_bytes(
            &[1],
            Duration::from_millis(25),
            &CancellationToken::default(),
            |_, _| {
                writes += 1;
                Ok(0)
            },
            || elapsed.get(),
            |duration| {
                pauses.push(duration);
                elapsed.set(elapsed.get() + duration);
            },
        );
        assert!(matches!(result, Err(PtouchError::Timeout)));
        assert_eq!(writes, 3);
        assert_eq!(
            pauses,
            [
                Duration::from_millis(10),
                Duration::from_millis(10),
                Duration::from_millis(5)
            ]
        );
    }

    #[test]
    fn usb_write_rejects_an_impossible_byte_count() {
        let result = send_usb_bytes(
            &[1],
            Duration::from_secs(1),
            &CancellationToken::default(),
            |_, _| Ok(2),
            || Duration::ZERO,
            |_| {},
        );
        assert!(matches!(result, Err(PtouchError::SendFailed(_))));
    }

    #[test]
    fn endpoint_pair_cannot_cross_alternate_settings() {
        assert!(matches!(
            select_bulk_endpoints([(0, vec![0x01]), (1, vec![0x82])]),
            Err(PtouchError::InvalidUsbInterface)
        ));
        assert_eq!(
            select_bulk_endpoints([(1, vec![0x02, 0x83]), (0, vec![0x01, 0x82])]).unwrap(),
            BulkEndpoints {
                alternate: 0,
                out: 1,
                input: 0x82,
                requires_selection: true,
            }
        );
        assert!(select_bulk_endpoints([(0, vec![1, 2, 0x81])]).is_err());
    }

    #[test]
    fn sole_default_setting_does_not_send_set_interface() {
        let endpoints = select_bulk_endpoints([(0, vec![0x02, 0x81])]).unwrap();
        endpoints
            .activate(|_| panic!("PT-D610BT must not receive SET_INTERFACE"))
            .unwrap();
    }

    #[test]
    fn multiple_settings_explicitly_activate_default_setting() {
        for other_endpoints in [vec![], vec![0x03], vec![0x03, 0x84]] {
            let endpoints =
                select_bulk_endpoints([(0, vec![0x02, 0x81]), (1, other_endpoints)]).unwrap();
            let mut requests = Vec::new();
            endpoints
                .activate(|alternate| {
                    requests.push(alternate);
                    Ok(())
                })
                .unwrap();
            assert_eq!(requests, [0]);
        }
    }

    #[test]
    fn nondefault_setting_is_always_activated() {
        for settings in [
            vec![(1, vec![0x02, 0x81])],
            vec![(0, vec![]), (1, vec![0x02, 0x81])],
        ] {
            let endpoints = select_bulk_endpoints(settings).unwrap();
            let mut requests = Vec::new();
            endpoints
                .activate(|alternate| {
                    requests.push(alternate);
                    Ok(())
                })
                .unwrap();
            assert_eq!(requests, [1]);
        }
    }

    #[test]
    fn alternate_selection_errors_are_preserved() {
        let endpoints = select_bulk_endpoints([(0, vec![0x02, 0x81]), (1, vec![])]).unwrap();
        for error in [
            rusb::Error::Timeout,
            rusb::Error::Pipe,
            rusb::Error::NoDevice,
        ] {
            assert_eq!(endpoints.activate(|_| Err(error)), Err(error));
        }
    }

    #[test]
    fn duplicate_printers_require_explicit_selection() {
        assert!(matches!(
            only_candidate(vec![1, 2]),
            Err(PtouchError::AmbiguousDevice)
        ));
        assert!(matches!(
            only_candidate(Vec::<u8>::new()),
            Err(PtouchError::DeviceNotFound)
        ));
        assert_eq!(only_candidate(vec![7]).unwrap(), 7);
        assert_eq!(
            "2:17".parse::<UsbLocation>().unwrap(),
            UsbLocation {
                bus: 2,
                address: 17
            }
        );
        for value in ["2", "2:17:3", "256:1", "1:-1"] {
            assert!(value.parse::<UsbLocation>().is_err());
        }
    }

    #[test]
    fn usb_facade_preserves_send_and_sync() {
        fn assert_traits<T: Send + Sync>() {}
        assert_traits::<PtouchDevice>();
    }
}
