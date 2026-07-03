// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: 2026 Huang Rui <vowstar@gmail.com>

//! Build script: embed the application icon into the Windows executable.
//!
//! On other platforms this does nothing. The icon comes from the shared SVG
//! via `scripts/gen-icons.sh`; the runtime window icon and the macOS bundle
//! icon are wired elsewhere (see `main.rs` and `Cargo.toml`).

fn main() {
    #[cfg(target_os = "windows")]
    {
        // build.rs runs with the crate root as the working directory.
        let icon = "../../data/windows/ptouch-gui.ico";
        println!("cargo:rerun-if-changed={icon}");
        let mut res = winresource::WindowsResource::new();
        res.set_icon(icon);
        if let Err(e) = res.compile() {
            // Non-fatal: a missing resource compiler should not break the build,
            // only leave the executable without its icon.
            println!("cargo:warning=failed to embed Windows icon: {e}");
        }
    }
}
