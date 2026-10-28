// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Windows startup diagnostics for launches without a console.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct StartupReport {
    path: Option<PathBuf>,
    show_dialog: bool,
}

impl StartupReport {
    pub fn new(show_dialog: bool) -> Self {
        #[cfg(windows)]
        let path = std::env::var_os("LOCALAPPDATA")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .map(|path| path.join("ptouch-rs").join("startup.log"));
        #[cfg(not(windows))]
        let path = None;
        let report = Self {
            path,
            show_dialog: show_dialog && cfg!(windows),
        };
        report.record(&format!(
            "\nP-Touch {}\nArchitecture: {}\nStarting GUI",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::ARCH
        ));
        report
    }

    pub fn record(&self, text: &str) {
        if let Some(path) = &self.path
            && let Err(error) = append_record(path, text)
        {
            eprintln!("Cannot write startup log: {error}");
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
            use windows_sys::Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW};
            let location = self
                .path
                .as_ref()
                .filter(|path| path.is_file())
                .map(|path| format!("\n\nStartup log: {}", path.display()))
                .unwrap_or_default();
            let message = format!(
                "P-Touch {} encountered an error.\n\n{text}{location}\n\nFor a graphics error, you can try starting with --software-renderer.",
                env!("CARGO_PKG_VERSION")
            );
            let wide = |text: &str| {
                text.replace('\0', " ")
                    .encode_utf16()
                    .chain(Some(0))
                    .collect::<Vec<_>>()
            };
            let title = wide("P-Touch error");
            let message = wide(&message);
            // Both strings remain allocated until this synchronous dialog closes.
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    message.as_ptr(),
                    title.as_ptr(),
                    MB_OK | MB_ICONERROR,
                );
            }
        }
        #[cfg(not(windows))]
        let _ = text;
    }
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

    #[test]
    fn diagnostics_preserve_prior_startup_and_unicode_errors() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("logs").join("startup.log");
        append_record(&path, "P-Touch previous attempt").unwrap();
        let report = StartupReport {
            path: Some(path.clone()),
            show_dialog: false,
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
        };
        report.error("Original renderer failure");
        assert_eq!(fs::read_to_string(parent).unwrap(), "existing file");
    }
}
