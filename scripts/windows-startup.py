#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Exercise published Windows GUIs and a release candidate with persisted settings."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import runpy
import shutil
import struct
import tempfile
import urllib.request


SMOKE = runpy.run_path(str(Path(__file__).with_name("windows-smoke.py")))
CHECK_GUI = SMOKE["check_gui_startup"]
APP_ID = "io.github.vowstar.ptouch-gui"
REFERENCE_TAGS = ("v0.15.1", "v0.15.2")


def download_release(tag, suffix, directory):
    """Use the published manifest, not an unchecked cached executable."""
    name = f"ptouch-gui-windows-{suffix}.exe"
    root = f"https://github.com/vowstar/ptouch-rs/releases/download/{tag}"
    manifest = urllib.request.urlopen(f"{root}/SHA256SUMS", timeout=60).read().decode("ascii")
    matches = [parts[0] for line in manifest.splitlines()
               if len(parts := line.split()) == 2 and parts[1].lstrip("*") == name]
    if len(matches) != 1 or not re.fullmatch(r"[0-9a-f]{64}", matches[0]):
        raise RuntimeError(f"{tag}: missing or invalid checksum for {name}")
    destination = directory / tag / name
    destination.parent.mkdir(parents=True, exist_ok=True)
    if not destination.is_file() or hashlib.sha256(destination.read_bytes()).hexdigest() != matches[0]:
        with urllib.request.urlopen(f"{root}/{name}", timeout=120) as response:
            data = response.read(100 * 1024 * 1024 + 1)
        if len(data) > 100 * 1024 * 1024 or hashlib.sha256(data).hexdigest() != matches[0]:
            raise RuntimeError(f"{tag}: published executable checksum mismatch")
        destination.write_bytes(data)
    return destination


def check_stack_reserve(gui):
    data = gui.read_bytes()
    optional = struct.unpack_from("<I", data, 0x3C)[0] + 24
    reserve, commit = struct.unpack_from("<QQ", data, optional + 72)
    if reserve < 8 * 1024 * 1024 or not 0 < commit <= 65536:
        raise RuntimeError(f"Unexpected GUI stack reserve/commit: {reserve}/{commit}")
    print(f"GUI stack reserve: {reserve} bytes; initial commit: {commit} bytes")


def write_window_settings(profile, width, height, maximized=False):
    directory = profile / "roaming" / APP_ID / "data"
    directory.mkdir(parents=True, exist_ok=True)
    value = (f"(inner_position_pixels:None,outer_position_pixels:None,"
             f"inner_size_points:Some((x:{width}.0,y:{height}.0)),"
             f"fullscreen:false,maximized:{str(maximized).lower()})")
    # RON string maps accept the same quoted strings as this JSON encoding.
    (directory / "app.ron").write_text(json.dumps({"window": value}), encoding="utf-8")


def check_references(suffix, report, downloads):
    profile = report / "reference-profile"
    for tag in REFERENCE_TAGS:
        gui = download_release(tag, suffix, downloads)
        SMOKE["check_pe"](gui, 0xAA64 if suffix == "arm64" else 0x8664,
                           require_icon=True, subsystem=2)
        for attempt in range(2):
            CHECK_GUI(gui, report, f"published-{tag}-{attempt}", [],
                      interactive=True, profile=profile)


def check_candidate(gui, suffix, report, downloads):
    SMOKE["check_pe"](gui, 0xAA64 if suffix == "arm64" else 0x8664,
                       require_icon=True, subsystem=2)
    check_stack_reserve(gui)
    # A clean folder contains only what is distributed, without build-time DLLs.
    with tempfile.TemporaryDirectory(prefix="ptouch-启动-") as temporary:
        staged = Path(temporary) / f"ptouch-gui-windows-{suffix}.exe"
        shutil.copy2(gui, staged)
        profile = report / "upgrade-profile"
        reference = download_release(REFERENCE_TAGS[0], suffix, downloads)
        CHECK_GUI(reference, report, "upgrade-reference", [], interactive=True, profile=profile)
        settings = profile / "roaming" / APP_ID / "data" / "app.ron"
        if not settings.is_file():
            raise RuntimeError("Reference GUI did not persist settings")
        for attempt in range(3):
            CHECK_GUI(staged, report, f"upgrade-candidate-{attempt}", [],
                      interactive=True, profile=profile)
        CHECK_GUI(staged, report, "candidate-fallback", [], force_primary_failure=True,
                  interactive=True, profile=profile)
        for name, width, height, maximized in (
            ("tiny", 64, 64, False), ("large", 20000, 20000, False),
            ("maximized", 1200, 700, True),
        ):
            geometry = report / f"{name}-profile"
            write_window_settings(geometry, width, height, maximized)
            CHECK_GUI(staged, report, name, [], interactive=True, profile=geometry)
        malformed = report / "malformed-profile"
        write_window_settings(malformed, 1200, 700)
        (malformed / "roaming" / APP_ID / "data" / "app.ron").write_text("{broken", encoding="utf-8")
        CHECK_GUI(staged, report, "malformed-settings", [], interactive=True, profile=malformed)
        blocked = report / "blocked-profile"
        (blocked / "local").mkdir(parents=True, exist_ok=True)
        obstruction = blocked / "local" / "ptouch-rs"
        obstruction.write_text("existing file", encoding="utf-8")
        CHECK_GUI(staged, report, "blocked-primary-log", [], interactive=True, profile=blocked)
        if obstruction.read_text(encoding="utf-8") != "existing file":
            raise RuntimeError("Fallback logging changed the unavailable primary path")
        if not (blocked / "temporary" / "ptouch-rs" / "startup.log").is_file():
            raise RuntimeError("Fallback startup report was not written")
        SMOKE["check_usbprint_worker"](staged)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True,
                        choices=("x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"))
    parser.add_argument("--references-only", action="store_true")
    args = parser.parse_args()
    directory = (Path("target") / args.target / "release").resolve()
    report = directory / "startup-regression"
    downloads = report / "downloads"
    report.mkdir(parents=True, exist_ok=True)
    suffix = "arm64" if args.target.startswith("aarch64") else "amd64"
    if args.references_only:
        check_references(suffix, report, downloads)
    else:
        check_candidate(directory / "ptouch-gui.exe", suffix, report, downloads)
    print("Windows persisted-profile startup checks passed")
