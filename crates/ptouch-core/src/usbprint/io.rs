// SPDX-License-Identifier: GPL-3.0-or-later
use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_IO_PENDING, ERROR_NOT_FOUND, ERROR_OPERATION_ABORTED, GENERIC_READ, GENERIC_WRITE,
        INVALID_HANDLE_VALUE, WAIT_TIMEOUT,
    },
    Storage::FileSystem::{CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING, ReadFile, WriteFile},
    System::{
        IO::{CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED},
        Threading::CreateEventW,
    },
};

pub(super) fn open(path: &str) -> io::Result<OwnedHandle> {
    let path: Vec<u16> = path.encode_utf16().chain(Some(0)).collect();
    // SAFETY: terminated path from SetupAPI. Exclusive access prevents interleaved jobs.
    let handle = unsafe {
        CreateFileW(
            path.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            0,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: this function owns the newly created handle.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle) })
}

/// Runs only in the disposable worker. Storage stays alive until I/O completion,
/// including cancellation acknowledgement. The parent enforces the outer deadline.
pub(super) fn transfer(
    handle: &OwnedHandle,
    data: &mut [u8],
    write: bool,
    millis: u32,
) -> io::Result<usize> {
    // SAFETY: manual reset event, no shared name, default security.
    let event = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
    if event.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: event is newly owned.
    let event = unsafe { OwnedHandle::from_raw_handle(event) };
    let mut overlapped = OVERLAPPED {
        hEvent: event.as_raw_handle(),
        ..Default::default()
    };
    // SAFETY: live buffer and OVERLAPPED remain in scope through completion below.
    let success = unsafe {
        if write {
            WriteFile(
                handle.as_raw_handle(),
                data.as_ptr(),
                data.len() as u32,
                ptr::null_mut(),
                &mut overlapped,
            )
        } else {
            ReadFile(
                handle.as_raw_handle(),
                data.as_mut_ptr(),
                data.len() as u32,
                ptr::null_mut(),
                &mut overlapped,
            )
        }
    };
    if success == 0 && io::Error::last_os_error().raw_os_error() != Some(ERROR_IO_PENDING as i32) {
        return Err(io::Error::last_os_error());
    }
    let mut count = 0;
    // SAFETY: submitted operation, live output and OVERLAPPED.
    if unsafe { GetOverlappedResultEx(handle.as_raw_handle(), &overlapped, &mut count, millis, 0) }
        != 0
    {
        return Ok(count as usize);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(WAIT_TIMEOUT as i32) {
        return Err(error);
    }
    // SAFETY: cancel only this request. Cancellation is not completion.
    let cancelled = unsafe { CancelIoEx(handle.as_raw_handle(), &overlapped) };
    let cancel_error = if cancelled == 0 {
        Some(io::Error::last_os_error())
    } else {
        None
    };
    // SAFETY: wait for the system to release buffer/OVERLAPPED. If a driver never
    // acknowledges, the parent terminates this worker, not the caller's process.
    let completed =
        unsafe { GetOverlappedResult(handle.as_raw_handle(), &overlapped, &mut count, 1) };
    if completed != 0 {
        return Ok(count as usize);
    } // completion raced with timeout
    let completion_error = io::Error::last_os_error();
    if let Some(error) = cancel_error
        && error.raw_os_error() != Some(ERROR_NOT_FOUND as i32)
    {
        return Err(error);
    }
    if completion_error.raw_os_error() != Some(ERROR_OPERATION_ABORTED as i32) {
        return Err(completion_error);
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "USBPRINT transfer timed out",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows_sys::Win32::{
        Foundation::ERROR_PIPE_CONNECTED,
        Storage::FileSystem::PIPE_ACCESS_DUPLEX,
        System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
        },
    };

    #[test]
    fn native_io_roundtrip_and_timeout_recovery() {
        let name = format!(r"\\.\pipe\ptouch-usbprint-test-{}", std::process::id());
        let wide: Vec<u16> = name.encode_utf16().chain(Some(0)).collect();
        // SAFETY: terminated unique pipe name and initialized arguments.
        let server = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                4096,
                4096,
                0,
                ptr::null(),
            )
        };
        assert_ne!(server, INVALID_HANDLE_VALUE);
        let server = unsafe { OwnedHandle::from_raw_handle(server) };
        let client = open(&name).unwrap();
        let mut ov = OVERLAPPED::default();
        let connected = unsafe { ConnectNamedPipe(server.as_raw_handle(), &mut ov) };
        assert!(
            connected != 0
                || io::Error::last_os_error().raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32)
        );
        let mut bytes = [0; 32];
        assert_eq!(
            transfer(&server, &mut bytes, false, 10).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(transfer(&client, &mut [1, 2, 3], true, 1000).unwrap(), 3);
        assert_eq!(transfer(&server, &mut bytes, false, 1000).unwrap(), 3);
        assert_eq!(&bytes[..3], &[1, 2, 3]);
    }
}
