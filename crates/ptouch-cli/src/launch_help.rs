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
        ("windows", "aarch64") => "Extract ptouch-windows-arm64.zip and open ptouch-gui.exe.",
        ("windows", _) => "Extract ptouch-windows-amd64.zip and open ptouch-gui.exe.",
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_instructions_use_the_running_version() {
        assert!(gui_instructions().contains(env!("CARGO_PKG_VERSION")));
        assert!(gui_instructions().contains("ptouch-gui"));
    }
}
