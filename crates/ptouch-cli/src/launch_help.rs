// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Help users find the label editor when they open the command-line tool.

pub const HELP: &str = concat!(
    "For the label editor, run ptouch gui for download instructions.\n",
    "https://github.com/vowstar/ptouch-rs/releases/tag/v",
    env!("CARGO_PKG_VERSION")
);

pub fn gui_instructions() -> String {
    let download = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "aarch64") => "Download ptouch-gui-windows-arm64.exe and open it.",
        ("windows", _) => "Download ptouch-gui-windows-amd64.exe and open it.",
        ("macos", _) => {
            "Download ptouch-gui-macos-arm64.app.zip for Apple silicon, then open ptouch-gui.app."
        }
        _ => "Run ptouch-gui, or install a package containing the label editor.",
    };
    format!(
        "{download}\nhttps://github.com/vowstar/ptouch-rs/releases/tag/v{}",
        env!("CARGO_PKG_VERSION")
    )
}

#[cfg(any(windows, test))]
fn needs_desktop_help(
    console_streams: bool,
    desktop_parent: bool,
    lone_visible_console: bool,
) -> bool {
    console_streams && (desktop_parent || lone_visible_console)
}

#[cfg(windows)]
pub fn show_for_desktop_launch() -> bool {
    use std::io::IsTerminal;
    use windows_sys::Win32::System::Console::{GetConsoleProcessList, GetConsoleWindow};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        IDYES, IsWindowVisible, MB_ICONINFORMATION, MB_YESNO, MessageBoxW,
    };

    if std::env::args_os().len() != 1 {
        return false;
    }
    let streams = std::io::stdin().is_terminal()
        && std::io::stdout().is_terminal()
        && std::io::stderr().is_terminal();
    if !streams {
        return false;
    }
    let mut processes = [0u32; 2];
    // A shared shell console must retain normal command-line behavior. For a
    // pseudoconsole the window is invisible, so also check the actual parent.
    let lone_visible = unsafe {
        GetConsoleProcessList(processes.as_mut_ptr(), processes.len() as u32) == 1
            && IsWindowVisible(GetConsoleWindow()) != 0
    };
    if !needs_desktop_help(streams, explorer_parent(), lone_visible) {
        return false;
    }
    let title = wide("P-Touch command-line tool");
    let message = wide(&format!(
        "This is the command-line tool.\n\n{}\n\nOpen the download page?",
        gui_instructions()
    ));
    // These buffers remain alive while the synchronous dialog is displayed.
    let answer = unsafe {
        MessageBoxW(
            std::ptr::null_mut(),
            message.as_ptr(),
            title.as_ptr(),
            MB_YESNO | MB_ICONINFORMATION,
        )
    };
    if answer == IDYES {
        use windows_sys::Win32::UI::Shell::ShellExecuteW;
        use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
        let url = wide(&format!(
            "https://github.com/vowstar/ptouch-rs/releases/tag/v{}",
            env!("CARGO_PKG_VERSION")
        ));
        let verb = wide("open");
        // The URL is a fixed project origin, with only the compiled-in version.
        let opened = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                url.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        } as isize;
        if opened <= 32 {
            let fallback = wide(
                "Open github.com/vowstar/ptouch-rs/releases in your browser to download the label editor.",
            );
            unsafe {
                MessageBoxW(
                    std::ptr::null_mut(),
                    fallback.as_ptr(),
                    title.as_ptr(),
                    MB_ICONINFORMATION,
                );
            }
        }
    }
    true
}

#[cfg(windows)]
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

#[cfg(windows)]
fn explorer_parent() -> bool {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
    use windows_sys::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        QueryFullProcessImageNameW,
    };

    // Check the full parent path for Explorer launches hosted in Windows
    // Terminal. A lone pseudoconsole process alone is not evidence of a double click.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut parent = 0;
        let mut found = Process32FirstW(snapshot, &mut entry);
        while found != 0 {
            if entry.th32ProcessID == GetCurrentProcessId() {
                parent = entry.th32ParentProcessID;
                break;
            }
            found = Process32NextW(snapshot, &mut entry);
        }
        CloseHandle(snapshot);
        if parent == 0 {
            return false;
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, parent);
        if process.is_null() {
            return false;
        }
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        let queried = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length);
        CloseHandle(process);
        if queried == 0 {
            return false;
        }
        let mut windows = vec![0u16; 32768];
        let size = GetWindowsDirectoryW(windows.as_mut_ptr(), windows.len() as u32) as usize;
        if size == 0 || size >= windows.len() {
            return false;
        }
        let expected = std::path::PathBuf::from(std::ffi::OsString::from_wide(&windows[..size]))
            .join("explorer.exe");
        let actual = std::ffi::OsString::from_wide(&path[..length as usize]);
        actual
            .to_string_lossy()
            .eq_ignore_ascii_case(&expected.to_string_lossy())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_help_excludes_redirection_and_shared_shells() {
        for desktop in [false, true] {
            for lone in [false, true] {
                assert!(!needs_desktop_help(false, desktop, lone));
            }
        }
        assert!(!needs_desktop_help(true, false, false));
        assert!(needs_desktop_help(true, true, false));
        assert!(needs_desktop_help(true, false, true));
    }

    #[test]
    fn editor_instructions_use_the_running_version() {
        assert!(gui_instructions().contains(env!("CARGO_PKG_VERSION")));
        assert!(gui_instructions().contains("ptouch-gui"));
    }
}
