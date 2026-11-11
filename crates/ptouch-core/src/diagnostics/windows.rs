// SPDX-License-Identifier: GPL-3.0-or-later
//! Read-only PnP enumeration. No driver installation or registry writes.

use super::{DoctorReport, DriverBinding};
use std::{io, mem::size_of, ptr};
use windows_sys::Win32::{
    Devices::DeviceAndDriverInstallation::*,
    Foundation::{ERROR_INVALID_DATA, ERROR_NO_MORE_ITEMS, GetLastError},
    System::Threading::{GetCurrentProcess, IsWow64Process2},
};

struct DeviceSet(HDEVINFO);
impl Drop for DeviceSet {
    fn drop(&mut self) {
        // SAFETY: the set was created by SetupDiGetClassDevsW and is owned here.
        unsafe {
            SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

fn property(
    set: &DeviceSet,
    device: &SP_DEVINFO_DATA,
    name: u32,
) -> io::Result<Option<Vec<String>>> {
    // Registry strings are UTF-16. Keep alignment and cap allocation at 64 KiB.
    let mut data = vec![0u16; 32768];
    let mut bytes = 0;
    let mut kind = 0;
    // SAFETY: all buffers have the advertised size and device belongs to set.
    let ok = unsafe {
        SetupDiGetDeviceRegistryPropertyW(
            set.0,
            device,
            name,
            &mut kind,
            data.as_mut_ptr().cast(),
            (data.len() * 2) as u32,
            &mut bytes,
        )
    };
    if ok == 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_INVALID_DATA as i32) {
            return Ok(None);
        }
        return Err(error);
    }
    if ![1, 7].contains(&kind) || bytes as usize > data.len() * 2 || !bytes.is_multiple_of(2) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Invalid PnP string property",
        ));
    }
    Ok(Some(
        data[..bytes as usize / 2]
            .split(|value| *value == 0)
            .filter(|part| !part.is_empty())
            .map(String::from_utf16_lossy)
            .collect(),
    ))
}

fn composite_parent(device: &SP_DEVINFO_DATA, hardware_ids: &[String]) -> Option<String> {
    if !hardware_ids.iter().any(|id| {
        id.split('&')
            .any(|part| part.to_ascii_uppercase().starts_with("MI_"))
    }) {
        return None;
    }
    let mut parent = 0;
    let mut id = [0u16; MAX_DEVICE_ID_LEN as usize];
    // SAFETY: the devnode came from this local PnP enumeration. Both output
    // pointers refer to writable buffers of the advertised size.
    let result = unsafe {
        if CM_Get_Parent(&mut parent, device.DevInst, 0) != CR_SUCCESS {
            return None;
        }
        CM_Get_Device_IDW(parent, id.as_mut_ptr(), id.len() as u32, 0)
    };
    if result != CR_SUCCESS {
        return None;
    }
    let end = id.iter().position(|unit| *unit == 0)?;
    String::from_utf16(&id[..end]).ok()
}

pub(super) fn inspect(report: &mut DoctorReport) {
    let mut process = 0;
    let mut native = 0;
    // SAFETY: GetCurrentProcess returns a borrowed pseudo-handle; both outputs are valid.
    if unsafe { IsWow64Process2(GetCurrentProcess(), &mut process, &mut native) } != 0 {
        report.native_arch = Some(match native {
            0xAA64 => "aarch64".into(),
            0x8664 => "x86_64".into(),
            0x014c => "x86".into(),
            value => format!("machine_{value:04x}"),
        });
    } else {
        report.errors.push(format!(
            "Native architecture: {}",
            io::Error::last_os_error()
        ));
    }
    let enumerator: Vec<u16> = "USB\0".encode_utf16().collect();
    // SAFETY: enumerator is NUL-terminated; optional pointers are null.
    let raw = unsafe {
        SetupDiGetClassDevsW(
            ptr::null(),
            enumerator.as_ptr(),
            ptr::null_mut(),
            DIGCF_ALLCLASSES | DIGCF_PRESENT,
        )
    };
    if raw == -1 {
        report.errors.push(format!(
            "Windows PnP enumeration: {}",
            io::Error::last_os_error()
        ));
        return;
    }
    let set = DeviceSet(raw);
    for index in 0..4096 {
        let mut device = SP_DEVINFO_DATA {
            cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
            ..Default::default()
        };
        // SAFETY: initialized size field and a live device information set.
        if unsafe { SetupDiEnumDeviceInfo(set.0, index, &mut device) } == 0 {
            if unsafe { GetLastError() } != ERROR_NO_MORE_ITEMS {
                report.errors.push(format!(
                    "Windows PnP device: {}",
                    io::Error::last_os_error()
                ));
            }
            return;
        }
        let ids = match property(&set, &device, SPDRP_HARDWAREID) {
            Ok(Some(ids)) => ids,
            Ok(None) => continue,
            Err(error) => {
                report.errors.push(format!("PnP hardware IDs: {error}"));
                continue;
            }
        };
        if !ids
            .iter()
            .any(|id| id.to_ascii_uppercase().starts_with("USB\\VID_04F9&"))
        {
            continue;
        }
        let mut instance = vec![0u16; 4096];
        // SAFETY: output is aligned and contains the advertised number of UTF-16 units.
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
            report
                .errors
                .push(format!("PnP instance ID: {}", io::Error::last_os_error()));
            continue;
        }
        let instance_id =
            String::from_utf16_lossy(instance.split(|unit| *unit == 0).next().unwrap_or_default());
        let mut text = |key| match property(&set, &device, key) {
            Ok(value) => value.and_then(|list| list.into_iter().next()),
            Err(error) => {
                report
                    .errors
                    .push(format!("PnP property {key} for {instance_id}: {error}"));
                None
            }
        };
        let service = text(SPDRP_SERVICE);
        let description = text(SPDRP_DEVICEDESC);
        let parent_instance_id = composite_parent(&device, &ids);
        report.windows_bindings.push(DriverBinding {
            instance_id,
            hardware_ids: ids,
            service,
            description,
            parent_instance_id,
        });
    }
    report
        .errors
        .push("Windows PnP enumeration exceeded 4096 devices".into());
}
