// SPDX-License-Identifier: GPL-3.0-or-later
//! PT-P710BT access through the existing Windows USBPRINT driver.
//!
//! Applications call [`run_worker_from_args`] before initializing their UI or CLI,
//! then [`configure_worker`] with that executable. No driver installation is performed.
mod discovery;
mod io;
mod wire;

use crate::{CancellationToken, PtouchError, Result, session::Transport};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::os::windows::process::CommandExt;
use std::{
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Mutex, OnceLock, mpsc},
    time::{Duration, Instant},
};
use wire::Frame;

const WORKER_ARG: &str = "--ptouch-usbprint-worker";
static WORKER: OnceLock<PathBuf> = OnceLock::new();

/// An enumerated PT-P710BT bound to the Windows USB printer driver.
#[derive(Clone, Debug, serde::Serialize)]
pub struct Printer {
    /// Device identity for explicit selection. It can contain a serial number.
    pub instance_id: String,
    /// Opaque SetupAPI interface path. Do not parse or construct this string.
    pub device_path: String,
}

pub(super) fn failure(stage: &str, error: impl std::fmt::Display) -> PtouchError {
    PtouchError::UsbPrint(format!("{stage}: {error}"))
}

/// Enable USBPRINT using an executable that implements [`run_worker_from_args`].
pub fn configure_worker(executable: PathBuf) -> Result<()> {
    if !executable.is_absolute() {
        return Err(failure("configure", "worker path must be absolute"));
    }
    if WORKER.get().is_some_and(|existing| existing == &executable) {
        return Ok(());
    }
    WORKER
        .set(executable)
        .map_err(|_| failure("configure", "worker already configured"))
}

/// Enumerate supported USBPRINT interfaces without opening the printer.
pub fn list() -> Result<Vec<Printer>> {
    discovery::list()
}

/// Check native opening and closing without sending printer commands.
pub fn probe(instance_id: &str) -> Result<()> {
    UsbPrintTransport::open(instance_id)?.close()
}

/// Resolve automatic selection only when PnP identifies one supported physical
/// printer. Explicit libusb bus/address selection never falls back by VID/PID.
pub(crate) fn automatic() -> Result<Option<String>> {
    if WORKER.get().is_none() {
        return Ok(None);
    }
    select_automatic(&crate::diagnostics::native_bindings()?, &list()?)
}

fn select_automatic(
    bindings: &[crate::diagnostics::DriverBinding],
    printers: &[Printer],
) -> Result<Option<String>> {
    let supported: Vec<_> = bindings
        .iter()
        .filter(|binding| {
            binding.hardware_ids.iter().any(|id| {
                crate::device::supported_devices().iter().any(|model| {
                    let prefix = format!("USB\\VID_{:04X}&PID_{:04X}", model.vid, model.pid);
                    id.to_ascii_uppercase()
                        .strip_prefix(&prefix)
                        .is_some_and(|tail| tail.is_empty() || tail.starts_with('&'))
                        && !model.flags.intersects(
                            crate::DeviceFlags::PLITE | crate::DeviceFlags::UNSUP_RASTER,
                        )
                })
            })
        })
        .collect();
    if supported.len() > 1 {
        return Err(PtouchError::AmbiguousDevice);
    }
    let Some(binding) = supported.first() else {
        return Ok(None);
    };
    if !binding
        .service
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("usbprint"))
    {
        return Ok(None);
    }
    Ok(printers
        .iter()
        .find(|p| p.instance_id.eq_ignore_ascii_case(&binding.instance_id))
        .map(|p| p.instance_id.clone()))
}

struct Worker {
    child: Child,
    // Closing the parent's last job handle also stops a worker if the parent exits.
    _job: OwnedHandle,
    requests: mpsc::SyncSender<Frame>,
    replies: mpsc::Receiver<std::io::Result<Frame>>,
    dead: bool,
}
impl Worker {
    fn spawn() -> Result<Self> {
        let executable = WORKER
            .get()
            .ok_or_else(|| failure("start", "application has not configured a USBPRINT worker"))?;
        let child = Command::new(executable)
            .arg(WORKER_ARG)
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| failure("start", e))?;
        Self::from_child(child)
    }
    fn from_child(mut child: Child) -> Result<Self> {
        let job = match child_job(&child) {
            Ok(job) => job,
            Err(error) => {
                let _ = child.kill();
                return Err(error);
            }
        };
        let mut input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        let (requests, requests_rx) = mpsc::sync_channel::<Frame>(1);
        let (replies_tx, replies) = mpsc::sync_channel(1);
        // Only owned buffers cross the thread boundary. Killing the worker closes
        // the pipes, so blocked IPC never retains a caller-owned buffer.
        std::thread::spawn(move || {
            while let Ok(frame) = requests_rx.recv() {
                let result = frame
                    .write(&mut input)
                    .and_then(|()| Frame::read(&mut output));
                let failed = result.is_err();
                if replies_tx.send(result).is_err() || failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            _job: job,
            requests,
            replies,
            dead: false,
        })
    }
    fn stop(&mut self) {
        self.dead = true;
        let _ = self.child.kill();
        // Never wait indefinitely for a kernel driver during caller cleanup.
        let _ = self.child.try_wait();
    }
    fn exchange(&mut self, frame: Frame, token: &CancellationToken) -> Result<Vec<u8>> {
        if self.dead {
            return Err(failure(
                "communicate",
                "connection closed after a previous failure",
            ));
        }
        token.check()?;
        let operation = frame.code;
        let deadline = Instant::now() + Duration::from_millis(u64::from(frame.millis) + 500);
        if let Err(error) = self.requests.try_send(frame) {
            self.stop();
            return Err(failure("send IPC request", error));
        }
        loop {
            if token.is_cancelled() {
                self.stop();
                return Err(PtouchError::Cancelled);
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                self.stop();
                return Err(failure(
                    "timeout",
                    "worker did not finish; connection closed, sent data may still print",
                ));
            };
            match self
                .replies
                .recv_timeout(remaining.min(Duration::from_millis(20)))
            {
                Ok(Ok(reply)) => match reply.code {
                    wire::OK => return Ok(reply.data),
                    wire::TIMEOUT if operation == wire::READ => return Err(PtouchError::Timeout),
                    _ => {
                        self.stop();
                        return Err(failure("transfer", String::from_utf8_lossy(&reply.data)));
                    }
                },
                Ok(Err(error)) => {
                    self.stop();
                    return Err(failure("worker IPC", error));
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.stop();
                    return Err(failure(
                        "worker",
                        "connection lost; sent data may still print",
                    ));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

fn child_job(child: &Child) -> Result<OwnedHandle> {
    use windows_sys::Win32::System::JobObjects::*;
    // SAFETY: unnamed job owned by this parent only, non-inheritable handle.
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    if raw.is_null() {
        return Err(failure(
            "create worker job",
            std::io::Error::last_os_error(),
        ));
    }
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    // SAFETY: correctly sized live limits structure and owned handles.
    if unsafe {
        SetInformationJobObject(
            job.as_raw_handle(),
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            std::mem::size_of_val(&limits) as u32,
        )
    } == 0
        || unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) } == 0
    {
        return Err(failure(
            "configure worker job",
            std::io::Error::last_os_error(),
        ));
    }
    Ok(job)
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(crate) struct UsbPrintTransport {
    worker: Mutex<Worker>,
    cancellation: CancellationToken,
}
impl UsbPrintTransport {
    pub(crate) fn open(instance_id: &str) -> Result<Self> {
        if !list()?
            .iter()
            .any(|p| p.instance_id.eq_ignore_ascii_case(instance_id))
        {
            return Err(PtouchError::DeviceNotFound);
        }
        let mut worker = Worker::spawn()?;
        worker.exchange(
            Frame {
                code: wire::OPEN,
                millis: 5000,
                data: instance_id.as_bytes().to_vec(),
            },
            &CancellationToken::default(),
        )?;
        Ok(Self {
            worker: Mutex::new(worker),
            cancellation: CancellationToken::default(),
        })
    }
    pub(crate) fn set_cancellation(&mut self, token: CancellationToken) {
        self.cancellation = token;
    }
    fn request(&self, code: u32, data: Vec<u8>, timeout: Duration) -> Result<Vec<u8>> {
        let millis = timeout.as_millis().clamp(1, 5000) as u32;
        self.worker
            .lock()
            .map_err(|_| failure("communicate", "worker lock poisoned"))?
            .exchange(Frame { code, millis, data }, &self.cancellation)
    }
}
impl Transport for UsbPrintTransport {
    fn send(&self, data: &[u8], timeout: Duration) -> Result<()> {
        if data.len() > wire::MAX_DATA {
            return Err(failure("write", "transfer too large"));
        }
        self.request(wire::WRITE, data.to_vec(), timeout)
            .map(|_| ())
    }
    fn receive(&self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        if buf.is_empty() || buf.len() > wire::MAX_DATA {
            return Err(failure("read", "invalid buffer length"));
        }
        let data = self.request(
            wire::READ,
            (buf.len() as u32).to_le_bytes().to_vec(),
            timeout,
        )?;
        if data.len() > buf.len() {
            return Err(failure("read", "worker returned excess data"));
        }
        buf[..data.len()].copy_from_slice(&data);
        Ok(data.len())
    }
    fn close(self) -> Result<()> {
        self.request(wire::CLOSE, Vec::new(), Duration::from_secs(1))
            .map(|_| ())
    }
}

/// Run the private worker entry point, returning its exit code when requested.
/// Call before logging, argument parsing, or GUI initialization.
pub fn run_worker_from_args() -> Option<i32> {
    if std::env::args_os().nth(1).as_deref() != Some(std::ffi::OsStr::new(WORKER_ARG)) {
        return None;
    }
    Some(if worker_loop().is_ok() { 0 } else { 1 })
}

fn transfer_once(
    mut bytes: Vec<u8>,
    writing: bool,
    transfer: impl FnOnce(&mut [u8]) -> std::io::Result<usize>,
) -> Result<Vec<u8>> {
    let count = transfer(&mut bytes).map_err(|error| {
        if error.kind() == std::io::ErrorKind::TimedOut {
            PtouchError::Timeout
        } else if writing {
            failure("write", format!("{error}; sent data may still print"))
        } else {
            failure("read", error)
        }
    })?;
    if count > bytes.len() || (writing && count != bytes.len()) {
        return Err(failure(
            if writing { "write" } else { "read" },
            "short or invalid transfer; not retrying, sent data may still print",
        ));
    }
    if writing {
        Ok(Vec::new())
    } else {
        bytes.truncate(count);
        Ok(bytes)
    }
}

fn worker_loop() -> std::io::Result<()> {
    let mut input = std::io::stdin().lock();
    let mut output = std::io::stdout().lock();
    let mut handle = None;
    loop {
        let frame = Frame::read(&mut input)?;
        let mut bytes = frame.data;
        let result: Result<Vec<u8>> = (|| {
            if !(1..=5000).contains(&frame.millis) {
                return Err(failure("request", "invalid timeout"));
            }
            match frame.code {
                wire::PING if handle.is_none() && bytes.is_empty() => {
                    Ok(b"PTOUCH_USBPRINT_WORKER_V1".to_vec())
                }
                wire::OPEN if handle.is_none() => {
                    let id = std::str::from_utf8(&bytes).map_err(|e| failure("open", e))?;
                    let printer = list()?
                        .into_iter()
                        .find(|p| p.instance_id.eq_ignore_ascii_case(id))
                        .ok_or(PtouchError::DeviceNotFound)?;
                    handle = Some(
                        io::open(&printer.device_path)
                            .map_err(|e| failure("open (printer may be in use)", e))?,
                    );
                    Ok(Vec::new())
                }
                wire::WRITE | wire::READ => {
                    let handle = handle
                        .as_ref()
                        .ok_or_else(|| failure("request", "printer is not open"))?;
                    let writing = frame.code == wire::WRITE;
                    if !writing {
                        let len = u32::from_le_bytes(
                            bytes
                                .as_slice()
                                .try_into()
                                .map_err(|_| failure("read", "invalid length"))?,
                        ) as usize;
                        if len == 0 || len > wire::MAX_DATA {
                            return Err(failure("read", "invalid length"));
                        }
                        bytes = vec![0; len];
                    }
                    transfer_once(bytes, writing, |bytes| {
                        io::transfer(handle, bytes, writing, frame.millis)
                    })
                }
                wire::CLOSE if bytes.is_empty() => {
                    handle.take();
                    Ok(Vec::new())
                }
                _ => Err(failure("request", "invalid worker command")),
            }
        })();
        let failed = result.is_err()
            && !(frame.code == wire::READ && matches!(result, Err(PtouchError::Timeout)));
        let (code, data) = match result {
            Ok(data) => (wire::OK, data),
            Err(PtouchError::Timeout) => (
                wire::TIMEOUT,
                b"transfer timed out; sent data may still print".to_vec(),
            ),
            Err(error) => (wire::ERROR, error.to_string().into_bytes()),
        };
        Frame {
            code,
            millis: 0,
            data,
        }
        .write(&mut output)?;
        if frame.code == wire::CLOSE || failed {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn binding(id: &str, service: &str) -> crate::diagnostics::DriverBinding {
        crate::diagnostics::DriverBinding {
            instance_id: id.into(),
            hardware_ids: vec!["USB\\VID_04F9&PID_20AF".into()],
            service: Some(service.into()),
            description: None,
        }
    }
    #[test]
    fn automatic_selection_never_guesses_between_devices_or_backends() {
        let printers = [Printer {
            instance_id: "printer-a".into(),
            device_path: "opaque-a".into(),
        }];
        assert_eq!(
            select_automatic(&[binding("printer-a", "usbprint")], &printers).unwrap(),
            Some("printer-a".into())
        );
        assert_eq!(
            select_automatic(&[binding("printer-a", "WinUSB")], &printers).unwrap(),
            None
        );
        assert_eq!(
            select_automatic(&[binding("printer-b", "usbprint")], &printers).unwrap(),
            None
        );
        assert!(matches!(
            select_automatic(
                &[
                    binding("printer-a", "usbprint"),
                    binding("printer-b", "WinUSB")
                ],
                &printers
            ),
            Err(PtouchError::AmbiguousDevice)
        ));
        assert!(matches!(
            select_automatic(
                &[
                    binding("printer-a", "usbprint"),
                    binding("printer-b", "usbprint")
                ],
                &printers
            ),
            Err(PtouchError::AmbiguousDevice)
        ));
    }
    #[test]
    fn short_and_failed_writes_never_replay_data() {
        for count in [0, 2, 4] {
            let mut calls = 0;
            assert!(
                transfer_once(vec![1, 2, 3], true, |data| {
                    calls += 1;
                    assert_eq!(data, [1, 2, 3]);
                    Ok(count)
                })
                .is_err()
            );
            assert_eq!(calls, 1);
        }
        assert!(
            transfer_once(vec![1, 2, 3], true, |_| Ok(3))
                .unwrap()
                .is_empty()
        );
        let mut calls = 0;
        assert!(matches!(
            transfer_once(vec![1, 2, 3], true, |_| {
                calls += 1;
                Err(std::io::ErrorKind::TimedOut.into())
            }),
            Err(PtouchError::Timeout)
        ));
        assert_eq!(calls, 1);
    }
    #[test]
    fn fragmented_and_empty_reads_preserve_actual_length() {
        assert_eq!(
            transfer_once(vec![0; 32], false, |data| {
                data[..2].copy_from_slice(&[0x80, 0x20]);
                Ok(2)
            })
            .unwrap(),
            [0x80, 0x20]
        );
        assert!(
            transfer_once(vec![0; 32], false, |_| Ok(0))
                .unwrap()
                .is_empty()
        );
        assert!(transfer_once(vec![0; 32], false, |_| Ok(33)).is_err());
    }
    fn sleeping_worker() -> Worker {
        let child = Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Start-Sleep -Seconds 30",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Worker::from_child(child).unwrap()
    }
    #[test]
    fn unresponsive_worker_has_a_deadline_and_cannot_be_reused() {
        let mut worker = sleeping_worker();
        let token = CancellationToken::default();
        let start = Instant::now();
        assert!(
            worker
                .exchange(
                    Frame {
                        code: wire::PING,
                        millis: 10,
                        data: Vec::new()
                    },
                    &token
                )
                .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(5));
        assert!(worker.dead);
        assert!(
            worker
                .exchange(
                    Frame {
                        code: wire::PING,
                        millis: 10,
                        data: Vec::new()
                    },
                    &token
                )
                .is_err()
        );
        assert!(worker.child.wait().unwrap().code() != Some(0));
    }
    #[test]
    fn cancellation_stops_a_blocked_worker() {
        let mut worker = sleeping_worker();
        let token = CancellationToken::default();
        let trigger = token.clone();
        let cancel = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            trigger.cancel();
        });
        let start = Instant::now();
        assert!(matches!(
            worker.exchange(
                Frame {
                    code: wire::PING,
                    millis: 5000,
                    data: Vec::new()
                },
                &token
            ),
            Err(PtouchError::Cancelled)
        ));
        assert!(start.elapsed() < Duration::from_secs(3));
        cancel.join().unwrap();
        assert!(worker.dead);
        assert!(worker.child.wait().unwrap().code() != Some(0));
    }
}
