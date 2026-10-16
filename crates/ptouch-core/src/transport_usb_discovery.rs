// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>
// SPDX-FileCopyrightText: Dominic Radermacher and the ptouch-print contributors

use crate::{PtouchError, Result};
use rusb::{Device, UsbContext};

use super::{USB_INTERFACE, UsbTransport};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub(crate) struct BulkEndpoints {
    pub alternate: u8,
    pub out: u8,
    pub input: u8,
    #[serde(skip)]
    pub(crate) requires_selection: bool,
}

impl BulkEndpoints {
    pub(crate) fn activate(&self, select: impl FnOnce(u8) -> rusb::Result<()>) -> rusb::Result<()> {
        if self.requires_selection {
            select(self.alternate)?;
        }
        Ok(())
    }
}

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

pub(crate) fn select_bulk_endpoints(
    settings: impl IntoIterator<Item = (u8, Vec<u8>)>,
) -> Result<BulkEndpoints> {
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
            if out.next().is_some() || input.next().is_some() {
                None
            } else {
                Some(pair)
            }
        })
        .min_by_key(|pair| pair.alternate)
        .ok_or(PtouchError::InvalidUsbInterface)
}

pub(crate) fn open_matching(
    ids: Option<(u16, u16)>,
    location: Option<UsbLocation>,
) -> Result<(UsbTransport, crate::DeviceInfo)> {
    let context = rusb::Context::new()?;
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
        let Some(info) = crate::device::find_device(desc.vendor_id(), desc.product_id()) else {
            continue;
        };
        if ids.is_none()
            && location.is_none()
            && info
                .flags
                .intersects(crate::DeviceFlags::PLITE | crate::DeviceFlags::UNSUP_RASTER)
        {
            continue;
        }
        candidates.push((usb, info.clone()));
    }
    let (usb, dev_info) = only_candidate(candidates)?;
    let transport = open_device(usb, &dev_info)?;
    Ok((transport, dev_info))
}

fn open_device(usb: Device<rusb::Context>, dev_info: &crate::DeviceInfo) -> Result<UsbTransport> {
    super::super::validate_device(dev_info)?;
    let connection_error = |stage, source| PtouchError::UsbConnection {
        stage,
        vid: dev_info.vid,
        pid: dev_info.pid,
        source,
    };
    let handle = usb.open().map_err(|e| connection_error("open", e))?;
    let config = usb
        .active_config_descriptor()
        .map_err(|e| connection_error("read configuration", e))?;
    let endpoints = find_bulk_endpoints(&config)?;
    if handle.kernel_driver_active(USB_INTERFACE).unwrap_or(false) {
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
    log::info!("Opened {} at {}", dev_info.name, UsbLocation::of(&usb));
    log::debug!(
        "Endpoints: OUT={:#04x}, IN={:#04x}, alternate={}",
        endpoints.out,
        endpoints.input,
        endpoints.alternate
    );
    Ok(UsbTransport {
        handle,
        ep_out: endpoints.out,
        ep_in: endpoints.input,
        cancellation: crate::CancellationToken::default(),
    })
}

pub(crate) fn only_candidate<T>(mut candidates: Vec<T>) -> Result<T> {
    match candidates.len() {
        0 => Err(PtouchError::DeviceNotFound),
        1 => Ok(candidates.remove(0)),
        _ => Err(PtouchError::AmbiguousDevice),
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
