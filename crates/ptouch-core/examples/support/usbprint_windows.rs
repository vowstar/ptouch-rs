// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    process::{Command, Stdio},
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{CreateFileW, OPEN_EXISTING, ReadFile, WriteFile},
};

fn interfaces() -> io::Result<Vec<String>> {
    ptouch_core::usbprint::list()
        .map(|printers| printers.into_iter().map(|p| p.device_path).collect())
        .map_err(io::Error::other)
}

fn query_status(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Do not accept arbitrary file/device paths even in the internal worker mode.
    if !interfaces()?.iter().any(|candidate| candidate == path) {
        return Err("Selected PT-P710BT interface disappeared".into());
    }
    let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    // SAFETY: NUL-terminated path, synchronous I/O, no security attributes or template handle.
    let raw = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: CreateFileW returned an owned handle. OwnedHandle closes it after synchronous I/O completes.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    let command = ptouch_core::protocol::cmd_status_request();
    let mut written = 0;
    // SAFETY: the command and count remain live until this synchronous call returns.
    if unsafe {
        WriteFile(
            handle.as_raw_handle(),
            command.as_ptr(),
            command.len() as u32,
            &mut written,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error().into());
    }
    if written as usize != command.len() {
        return Err("Short status-request write; not retrying".into());
    }
    let mut pending = Vec::new();
    for _ in 0..64 {
        let mut bytes = [0u8; 64];
        let mut received = 0;
        // SAFETY: synchronous read into a live, writable buffer of the advertised size.
        if unsafe {
            ReadFile(
                handle.as_raw_handle(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                &mut received,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error().into());
        }
        if received == 0 || received as usize > bytes.len() {
            return Err("Invalid or empty status read".into());
        }
        pending.extend_from_slice(&bytes[..received as usize]);
        while pending.len() >= 32 {
            let status = ptouch_core::PrinterStatus::from_bytes(&pending[..32])
                .ok_or("Invalid status packet")?;
            pending.drain(..32);
            if status.print_head_mark != 0x80 || status.size != 0x20 || status.brother_code != 0x42
            {
                return Err("Unexpected Brother status header".into());
            }
            if status.status_type == 0 {
                println!(
                    "USBPRINT status received: tape={}mm, status errors={}",
                    status.media_width,
                    status.error_description()
                );
                return Ok(());
            }
        }
    }
    Err("No query response within 64 reads".into())
}

pub(super) fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args == ["--help"] {
        println!(
            "usbprint_probe [--list | --status DEVICE_PATH]\nExperimental PT-P710BT query only; no printing or driver changes."
        );
        return Ok(());
    }
    if args.len() == 2 && args[0] == "--worker" {
        return query_status(&args[1]);
    }
    if args.is_empty() || args == ["--list"] {
        let paths = interfaces()?;
        for path in &paths {
            println!("{path}");
        }
        if paths.is_empty() {
            println!("No PT-P710BT USBPRINT interfaces found");
        }
        return Ok(());
    }
    if args.len() != 2 || args[0] != "--status" {
        return Err("Use --list or --status DEVICE_PATH".into());
    }
    let path = &args[1];
    if !interfaces()?.contains(path) {
        return Err("Device path is not present; run --list again".into());
    }
    // The child owns synchronous I/O buffers. Timeout terminates that process;
    // no caller buffer is freed while a background thread still uses it.
    let mut child = Command::new(std::env::current_exe()?)
        .args(["--worker", path])
        .stdin(Stdio::null())
        .spawn()?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return if status.success() {
                Ok(())
            } else {
                Err(format!("USBPRINT probe failed: {status}").into())
            };
        }
        if started.elapsed() >= Duration::from_secs(15) {
            child.kill()?;
            child.wait()?;
            return Err("USBPRINT probe timed out after 15 seconds; child terminated".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
