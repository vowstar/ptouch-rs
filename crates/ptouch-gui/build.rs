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
        // Leave headroom for GUI initialization on the Windows main thread.
        // Reserve address space only; keep the linker's normal initial commit.
        if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
            println!("cargo:rustc-link-arg-bin=ptouch-gui=/STACK:8388608");
        }
        // build.rs runs with the crate root as the working directory.
        let icon = "../../data/windows/ptouch-gui.ico";
        println!("cargo:rerun-if-changed={icon}");
        let mut res = winresource::WindowsResource::new();
        res.set_icon(icon);
        res.compile().expect("failed to embed Windows icon");
    }
}
