// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Windows startup diagnostics for launches without a console.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone)]
pub struct StartupReport {
    path: Option<PathBuf>,
    show_dialog: bool,
    editor_initialized: Arc<AtomicBool>,
}

impl StartupReport {
    pub fn new(show_dialog: bool) -> Self {
        let initial = format!(
            "\nP-Touch {}\nArchitecture: {}\nStarting GUI",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::ARCH
        );
        #[cfg(windows)]
        let path = initialize_log(
            std::env::var_os("LOCALAPPDATA")
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .map(|path| path.join("ptouch-rs").join("startup.log")),
            std::env::temp_dir().join("ptouch-rs").join("startup.log"),
            &initial,
        );
        #[cfg(not(windows))]
        let path = {
            let _ = initial;
            None
        };
        let report = Self {
            path,
            show_dialog: show_dialog && cfg!(windows),
            editor_initialized: Default::default(),
        };
        if let Ok(executable) = std::env::current_exe() {
            report.record(&format!(
                "Process: {}\nExecutable: {}",
                std::process::id(),
                executable.display()
            ));
        }
        report
    }

    pub fn record(&self, text: &str) {
        if let Some(path) = &self.path
            && let Err(error) = append_record(path, text)
        {
            eprintln!("Cannot write startup log: {error}");
        }
    }

    pub fn editor_ready(&self) {
        self.editor_initialized.store(true, Ordering::Release);
        self.record("Editor initialized");
    }

    #[cfg(any(windows, test))]
    pub fn run_with_renderer_fallback<E: std::fmt::Display>(
        &self,
        mut attempt: impl FnMut(bool) -> Result<(), E>,
        is_renderer_error: impl FnOnce(&E) -> bool,
    ) -> Result<(), E> {
        match attempt(false) {
            Err(error)
                if !self.editor_initialized.load(Ordering::Acquire)
                    && is_renderer_error(&error) =>
            {
                self.record(&format!(
                    "Default renderer failed: {error}\nTrying Windows software rendering"
                ));
                attempt(true)
            }
            result => result,
        }
    }

    pub fn error(&self, text: &str) {
        eprintln!("P-Touch could not start: {text}");
        self.record(&format!("Startup error: {text}"));
        if self.show_dialog {
            self.dialog(text);
        }
    }

    pub fn install_panic_hook(&self) {
        if self.path.is_none() && !self.show_dialog {
            return;
        }
        let report = self.clone();
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let text = info.to_string();
            report.record(&format!("Panic: {text}"));
            previous(info);
            if report.show_dialog && std::thread::current().name() == Some("main") {
                report.dialog(&text);
            }
        }));
    }

    fn dialog(&self, text: &str) {
        #[cfg(windows)]
        {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                IDOK, MB_ICONERROR, MB_OK, MB_OKCANCEL, MessageBoxW,
            };
            let report = self.path.as_ref().filter(|path| path.is_file());
            let message = if report.is_some() {
                format!(
                    "P-Touch {} could not continue.\n\nSelect OK to show the error report, or Cancel to close.",
                    env!("CARGO_PKG_VERSION")
                )
            } else {
                format!(
                    "P-Touch {} could not continue.\n\n{text}",
                    env!("CARGO_PKG_VERSION")
                )
            };
            let wide = |text: &str| {
                text.replace('\0', " ")
                    .encode_utf16()
                    .chain(Some(0))
                    .collect::<Vec<_>>()
            };
            let title = wide("P-Touch error");
            let message = wide(&message);
            // Both strings remain allocated until this synchronous dialog closes.
            let choice = unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    message.as_ptr(),
                    title.as_ptr(),
                    (if report.is_some() { MB_OKCANCEL } else { MB_OK }) | MB_ICONERROR,
                )
            };
            if choice == IDOK
                && let Some(path) = report
            {
                let mut directory = vec![0u16; 32768];
                // Query the OS path rather than searching the download directory.
                let length = unsafe {
                    windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW(
                        directory.as_mut_ptr(),
                        directory.len() as u32,
                    )
                } as usize;
                if length > 0 && length < directory.len() {
                    use std::os::windows::ffi::OsStringExt;
                    let explorer =
                        PathBuf::from(std::ffi::OsString::from_wide(&directory[..length]))
                            .join("explorer.exe");
                    if let Err(error) = std::process::Command::new(explorer)
                        .arg("/select,")
                        .arg(path)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::null())
                        .spawn()
                    {
                        self.record(&format!("Cannot show error report: {error}"));
                    }
                }
            }
        }
        #[cfg(not(windows))]
        let _ = text;
    }
}

#[cfg(any(windows, test))]
fn initialize_log(primary: Option<PathBuf>, fallback: PathBuf, text: &str) -> Option<PathBuf> {
    for path in primary.into_iter().chain(Some(fallback)) {
        match append_record(&path, text) {
            Ok(()) => return Some(path),
            Err(error) => eprintln!("Cannot write startup log at {}: {error}", path.display()),
        }
    }
    None
}

fn append_record(path: &Path, text: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(file, "{text}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn isolated_report() -> StartupReport {
        StartupReport {
            path: None,
            show_dialog: false,
            editor_initialized: Default::default(),
        }
    }

    #[test]
    fn diagnostics_preserve_prior_startup_and_unicode_errors() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logs").join("startup.log");
        append_record(&path, "P-Touch previous attempt").unwrap();
        let report = StartupReport {
            path: Some(path.clone()),
            show_dialog: false,
            editor_initialized: Default::default(),
        };
        report.error("Cannot create renderer: 字体");
        let log = fs::read_to_string(path).unwrap();
        assert!(log.contains("previous attempt"));
        assert!(log.contains("Cannot create renderer: 字体"));
    }

    #[test]
    fn unavailable_log_location_does_not_replace_the_original_error() {
        let directory = tempfile::tempdir().unwrap();
        let parent = directory.path().join("not-a-directory");
        fs::write(&parent, "existing file").unwrap();
        assert!(append_record(&parent.join("startup.log"), "error").is_err());
        let report = StartupReport {
            path: Some(parent.join("startup.log")),
            show_dialog: false,
            editor_initialized: Default::default(),
        };
        report.error("Original renderer failure");
        assert_eq!(fs::read_to_string(parent).unwrap(), "existing file");
    }

    #[test]
    fn startup_log_falls_back_without_overwriting_unavailable_primary() {
        let directory = tempfile::tempdir().unwrap();
        let blocked = directory.path().join("blocked");
        fs::write(&blocked, "existing file").unwrap();
        let fallback = directory.path().join("temporary").join("startup.log");
        assert_eq!(
            initialize_log(Some(blocked.join("startup.log")), fallback.clone(), "start"),
            Some(fallback.clone())
        );
        assert_eq!(fs::read_to_string(blocked).unwrap(), "existing file");
        assert_eq!(fs::read_to_string(fallback).unwrap(), "start\n");
    }

    #[test]
    fn startup_log_prefers_primary_and_appends_each_launch() {
        let directory = tempfile::tempdir().unwrap();
        let primary = directory.path().join("local").join("startup.log");
        let fallback = directory.path().join("temporary").join("startup.log");
        for message in ["first", "second"] {
            assert_eq!(
                initialize_log(Some(primary.clone()), fallback.clone(), message),
                Some(primary.clone())
            );
        }
        assert_eq!(fs::read_to_string(primary).unwrap(), "first\nsecond\n");
        assert!(!fallback.exists());
    }

    #[test]
    fn startup_log_handles_missing_and_unavailable_locations() {
        let directory = tempfile::tempdir().unwrap();
        let fallback = directory.path().join("startup.log");
        assert_eq!(
            initialize_log(None, fallback.clone(), "start"),
            Some(fallback.clone())
        );
        assert!(initialize_log(None, fallback.join("not-a-directory"), "start").is_none());
    }

    #[test]
    fn graphics_failure_retries_software_without_user_input() {
        let report = isolated_report();
        let mut attempts = Vec::new();
        let result = report.run_with_renderer_fallback(
            |software| {
                attempts.push(software);
                if software {
                    Ok(())
                } else {
                    Err("graphics unavailable")
                }
            },
            |_| true,
        );
        assert_eq!(result, Ok(()));
        assert_eq!(attempts, [false, true]);
    }

    #[test]
    fn healthy_startup_and_non_graphics_errors_are_not_restarted() {
        for first in [Ok(()), Err("invalid editor state")] {
            let report = isolated_report();
            let mut attempts = 0;
            let result = report.run_with_renderer_fallback(
                |_| {
                    attempts += 1;
                    first
                },
                |_| false,
            );
            assert_eq!(result, first);
            assert_eq!(attempts, 1);
        }
    }

    #[test]
    fn an_initialized_editor_is_never_recreated_after_an_error() {
        let report = isolated_report();
        let mut attempts = 0;
        let result = report.run_with_renderer_fallback(
            |_| {
                attempts += 1;
                report.editor_ready();
                Err("device lost after editing started")
            },
            |_| true,
        );
        assert_eq!(result, Err("device lost after editing started"));
        assert_eq!(attempts, 1);
    }

    #[test]
    fn failed_fallback_is_bounded_and_preserves_both_errors() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("startup.log");
        let report = StartupReport {
            path: Some(path.clone()),
            show_dialog: false,
            editor_initialized: Default::default(),
        };
        let mut attempts = 0;
        let result = report.run_with_renderer_fallback(
            |software| {
                attempts += 1;
                Err(if software {
                    "software unavailable"
                } else {
                    "hardware unavailable"
                })
            },
            |_| true,
        );
        assert_eq!(result, Err("software unavailable"));
        report.error(result.unwrap_err());
        let log = fs::read_to_string(path).unwrap();
        assert!(log.contains("hardware unavailable") && log.contains("software unavailable"));
        assert_eq!(attempts, 2);
    }
}
