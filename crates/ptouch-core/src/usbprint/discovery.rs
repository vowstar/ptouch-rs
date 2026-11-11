// SPDX-License-Identifier: GPL-3.0-or-later
use super::{Printer, failure};
use crate::{DeviceFlags, DeviceInfo, Result, device};
use std::{
    mem::{offset_of, size_of},
    ptr,
};
use windows_sys::Win32::{
    Devices::DeviceAndDriverInstallation::*,
    Foundation::{
        ERROR_DEVICE_NOT_CONNECTED, ERROR_INSUFFICIENT_BUFFER, ERROR_INVALID_DATA,
        ERROR_NO_MORE_ITEMS, ERROR_NO_SUCH_DEVINST, GetLastError,
    },
    Graphics::Printing::GUID_DEVINTERFACE_USBPRINT,
};

struct DeviceSet(HDEVINFO);
impl Drop for DeviceSet {
    fn drop(&mut self) {
        // SAFETY: this set is owned and remains valid until destruction.
        unsafe {
            SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

fn property(set: HDEVINFO, device: &SP_DEVINFO_DATA, key: u32) -> Result<Vec<String>> {
    let mut data = vec![0u16; 32768];
    let mut bytes = 0;
    let mut kind = 0;
    // SAFETY: aligned output storage with the advertised byte capacity.
    if unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            set,
            device,
            key,
            &mut kind,
            data.as_mut_ptr().cast(),
            (data.len() * 2) as u32,
            &mut bytes,
        )
    } == 0
    {
        let error = std::io::Error::last_os_error();
        if matches!(
            error.raw_os_error().map(|code| code as u32),
            Some(ERROR_INVALID_DATA | ERROR_NO_SUCH_DEVINST | ERROR_DEVICE_NOT_CONNECTED)
        ) {
            return Ok(Vec::new());
        }
        return Err(failure("read device property", error));
    }
    if ![1, 7].contains(&kind) || bytes as usize > data.len() * 2 || !bytes.is_multiple_of(2) {
        return Err(failure("enumerate", "invalid device property"));
    }
    data[..bytes as usize / 2]
        .split(|v| *v == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf16(s).map_err(|e| failure("enumerate", e)))
        .collect()
}

fn vid_pid(id: &str) -> Option<(u16, u16)> {
    let id = id.to_ascii_uppercase();
    let mut parts = id.strip_prefix("USB\\VID_")?.split('&');
    let vid = parts.next()?;
    let pid = parts.next()?.strip_prefix("PID_")?;
    if ![vid, pid]
        .iter()
        .all(|part| part.len() == 4 && part.bytes().all(|byte| byte.is_ascii_hexdigit()))
        || parts.any(|suffix| suffix.is_empty() || suffix.contains(['\\', '/']))
    {
        return None;
    }
    Some((
        u16::from_str_radix(vid, 16).ok()?,
        u16::from_str_radix(pid, 16).ok()?,
    ))
}

pub(super) fn raster_model(hardware_ids: &[String]) -> Option<&'static DeviceInfo> {
    let mut pairs = hardware_ids.iter().filter_map(|id| vid_pid(id));
    let (vid, pid) = pairs.next()?;
    if pairs.any(|pair| pair != (vid, pid)) {
        return None;
    }
    device::find_device(vid, pid).filter(|model| {
        !model
            .flags
            .intersects(DeviceFlags::PLITE | DeviceFlags::UNSUP_RASTER)
    })
}

pub(super) fn list() -> Result<Vec<Printer>> {
    // SAFETY: fixed interface GUID and no window or enumerator restriction.
    let raw = unsafe {
        SetupDiGetClassDevsW(
            &GUID_DEVINTERFACE_USBPRINT,
            ptr::null(),
            ptr::null_mut(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    };
    if raw == -1 {
        return Err(failure("enumerate", std::io::Error::last_os_error()));
    }
    let set = DeviceSet(raw);
    let mut printers = Vec::new();
    for index in 0..4096 {
        let mut interface = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..Default::default()
        };
        // SAFETY: live set, initialized output structure.
        if unsafe {
            SetupDiEnumDeviceInterfaces(
                set.0,
                ptr::null(),
                &GUID_DEVINTERFACE_USBPRINT,
                index,
                &mut interface,
            )
        } == 0
        {
            if unsafe { GetLastError() } == ERROR_NO_MORE_ITEMS {
                return Ok(printers);
            }
            return Err(failure(
                "enumerate interface",
                std::io::Error::last_os_error(),
            ));
        }
        let mut needed = 0;
        // SAFETY: size query, no output buffer supplied.
        unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                set.0,
                &interface,
                ptr::null_mut(),
                0,
                &mut needed,
                ptr::null_mut(),
            );
        }
        let error = unsafe { GetLastError() };
        if matches!(error, ERROR_NO_SUCH_DEVINST | ERROR_DEVICE_NOT_CONNECTED) {
            continue;
        }
        if error != ERROR_INSUFFICIENT_BUFFER || !(8..=65536).contains(&needed) {
            return Err(failure("enumerate", "invalid interface length"));
        }
        let mut storage = vec![0usize; (needed as usize).div_ceil(size_of::<usize>())];
        let detail = storage
            .as_mut_ptr()
            .cast::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>();
        let mut device = SP_DEVINFO_DATA {
            cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        // SAFETY: sufficient aligned storage, structure sizes match this architecture.
        unsafe {
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
        }
        if unsafe {
            SetupDiGetDeviceInterfaceDetailW(
                set.0,
                &interface,
                detail,
                needed,
                ptr::null_mut(),
                &mut device,
            )
        } == 0
        {
            let error = std::io::Error::last_os_error();
            if matches!(
                error.raw_os_error().map(|code| code as u32),
                Some(ERROR_NO_SUCH_DEVINST | ERROR_DEVICE_NOT_CONNECTED)
            ) {
                continue;
            }
            return Err(failure("enumerate interface details", error));
        }
        let ids = property(set.0, &device, SPDRP_HARDWAREID)?;
        let Some(model) = raster_model(&ids) else {
            continue;
        };
        if !property(set.0, &device, SPDRP_SERVICE)?
            .iter()
            .any(|s| s.eq_ignore_ascii_case("usbprint"))
        {
            continue;
        }
        let mut instance = vec![0u16; 4096];
        // SAFETY: initialized device and output with stated capacity.
        if unsafe {
            SetupDiGetDeviceInstanceIdW(
                set.0,
                &device,
                instance.as_mut_ptr(),
                instance.len() as u32,
                ptr::null_mut(),
            )
        } == 0
        {
            let error = std::io::Error::last_os_error();
            if matches!(
                error.raw_os_error().map(|code| code as u32),
                Some(ERROR_NO_SUCH_DEVINST | ERROR_DEVICE_NOT_CONNECTED)
            ) {
                continue;
            }
            return Err(failure("read instance ID", error));
        }
        let offset = offset_of!(SP_DEVICE_INTERFACE_DETAIL_DATA_W, DevicePath);
        // SAFETY: UTF-16 aligned slice entirely within the allocated detail buffer.
        let path = unsafe {
            std::slice::from_raw_parts(
                storage.as_ptr().cast::<u8>().add(offset).cast::<u16>(),
                (needed as usize - offset) / 2,
            )
        };
        let decode = |units: &[u16]| -> Result<String> {
            let end = units
                .iter()
                .position(|u| *u == 0)
                .ok_or_else(|| failure("enumerate", "unterminated device string"))?;
            String::from_utf16(&units[..end]).map_err(|e| failure("enumerate", e))
        };
        printers.push(Printer {
            instance_id: decode(&instance)?,
            device_path: decode(path)?,
            model,
        });
    }
    Err(failure("enumerate", "too many interfaces"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hardware_ids_match_raster_models_and_reject_other_modes() {
        for model in device::supported_devices() {
            let ids = [
                format!(
                    "usb\\vid_{:04x}&pid_{:04x}&rev_0100&mi_00",
                    model.vid, model.pid
                ),
                format!("USB\\VID_{:04X}&PID_{:04X}", model.vid, model.pid),
            ];
            let supported = !model
                .flags
                .intersects(DeviceFlags::PLITE | DeviceFlags::UNSUP_RASTER);
            assert_eq!(raster_model(&ids), supported.then_some(model));
        }
        for id in [
            "USB\\VID_04F9&PID_20AFF",
            "USB\\VID_04F9&PID_FFFF",
            "USB\\VID_1234&PID_2061",
            "USB\\VID_04F9&PID_2061\\serial",
            "USB\\VID_04F9&PID_2061&MI_00\\serial",
            "USB\\VID_04F9&PID_+061",
            "USBPRINT\\BrotherPT-P700",
            "C:\\file",
            "\\\\?\\usb#vid_04f9&pid_20af",
        ] {
            assert_eq!(raster_model(&[id.into()]), None);
        }
        assert_eq!(
            raster_model(&[
                "USB\\VID_04F9&PID_2061".into(),
                "USB\\VID_04F9&PID_2064".into()
            ]),
            None
        );
    }
}
