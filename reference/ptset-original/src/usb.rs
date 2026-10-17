//! Raw USB transport to Brother printers via the usbprint.sys device interface,
//! mirroring what ptfilo.dll does: enumerate GUID_DEVINTERFACE_USBPRINT with SetupDi,
//! open the interface path with CreateFileW, then WriteFile/ReadFile.

use anyhow::{Context, Result, bail};
use std::{thread, time::Duration};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA,
    SP_DEVICE_INTERFACE_DETAIL_DATA_W, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    ReadFile, WriteFile,
};
use windows::core::{GUID, HSTRING, PCWSTR};

const GUID_DEVINTERFACE_USBPRINT: GUID = GUID::from_u128(0x28d78fad_5a12_11d1_ae5b_0000f803a8c2);
const BROTHER_VID: &str = "vid_04f9";

/// Device interface paths of present USB printers.
pub fn list_interfaces() -> Result<Vec<String>> {
    let mut paths = Vec::new();
    unsafe {
        let set = SetupDiGetClassDevsW(
            Some(&GUID_DEVINTERFACE_USBPRINT),
            PCWSTR::null(),
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
        .context("SetupDiGetClassDevsW")?;
        for index in 0.. {
            let mut iface = SP_DEVICE_INTERFACE_DATA {
                cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..Default::default()
            };
            if SetupDiEnumDeviceInterfaces(set, None, &GUID_DEVINTERFACE_USBPRINT, index, &mut iface)
                .is_err()
            {
                break;
            }
            let mut required = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(set, &iface, None, 0, Some(&mut required), None);
            // u32-aligned buffer large enough for the variable-length detail struct.
            let mut buf = vec![0u32; (required as usize).div_ceil(4)];
            let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
            if SetupDiGetDeviceInterfaceDetailW(set, &iface, Some(detail), required, None, None).is_ok()
            {
                let path = PCWSTR((*detail).DevicePath.as_ptr()).to_string()?;
                paths.push(path);
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
    }
    Ok(paths)
}

pub struct Printer {
    handle: HANDLE,
    pub path: String,
}

impl Printer {
    /// Open the first Brother USB printer, or the interface whose path contains `serial`.
    pub fn open(serial: Option<&str>) -> Result<Self> {
        let path = list_interfaces()?
            .into_iter()
            .find(|p| {
                let p = p.to_lowercase();
                p.contains(BROTHER_VID) && serial.is_none_or(|s| p.contains(&s.to_lowercase()))
            })
            .context("no Brother USB printer found (is it powered on and connected?)")?;
        let handle = unsafe {
            CreateFileW(
                &HSTRING::from(path.as_str()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        }
        .with_context(|| format!("CreateFileW {path}"))?;
        Ok(Self { handle, path })
    }

    pub fn write(&self, data: &[u8]) -> Result<()> {
        let mut written = 0u32;
        unsafe { WriteFile(self.handle, Some(data), Some(&mut written), None) }.context("WriteFile")?;
        if written as usize != data.len() {
            bail!("short write: {written} of {} bytes", data.len());
        }
        Ok(())
    }

    /// Read up to `buf.len()` bytes, polling until data arrives or `timeout` elapses.
    pub fn read(&self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        let step = Duration::from_millis(50);
        let mut waited = Duration::ZERO;
        loop {
            let mut read = 0u32;
            unsafe { ReadFile(self.handle, Some(buf), Some(&mut read), None) }.context("ReadFile")?;
            if read > 0 || waited >= timeout {
                return Ok(read as usize);
            }
            thread::sleep(step);
            waited += step;
        }
    }
}

impl Drop for Printer {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}
