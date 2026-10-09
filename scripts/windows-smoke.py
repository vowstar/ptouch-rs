#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check Windows executable architecture, subsystem, icons, and native startup."""

import argparse
import json
from pathlib import Path
import struct
import subprocess
import sys


def check_pe(path, machine, require_icon=False, subsystem=3):
    data = path.read_bytes()
    if data[:2] != b"MZ":
        raise ValueError(f"{path}: not a PE executable")
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    if data[pe:pe + 4] != b"PE\0\0":
        raise ValueError(f"{path}: invalid PE signature")
    actual, sections = struct.unpack_from("<HH", data, pe + 4)
    if actual != machine:
        raise ValueError(f"{path}: machine {actual:#x}, expected {machine:#x}")
    optional = pe + 24
    if struct.unpack_from("<H", data, optional)[0] != 0x20B:
        raise ValueError(f"{path}: expected PE32+")
    actual_subsystem = struct.unpack_from("<H", data, optional + 68)[0]
    if actual_subsystem != subsystem:
        raise ValueError(f"{path}: subsystem {actual_subsystem}, expected {subsystem}")
    if not require_icon:
        return
    resource_rva = struct.unpack_from("<I", data, optional + 112 + 2 * 8)[0]
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    for index in range(sections):
        section = optional + optional_size + index * 40
        size, rva, raw_size, offset = struct.unpack_from("<IIII", data, section + 8)
        if rva <= resource_rva < rva + max(size, raw_size):
            root = offset + resource_rva - rva
            named, ids = struct.unpack_from("<HH", data, root + 12)
            for entry in range(named + ids):
                kind = struct.unpack_from("<I", data, root + 16 + entry * 8)[0]
                if kind == 14:  # RT_GROUP_ICON
                    return
    raise ValueError(f"{path}: no embedded group icon")


def check_usbprint_worker(executable):
    def frame(code, payload=b"", millis=1000):
        return struct.pack("<III", code, millis, len(payload)) + payload

    # Match the application's hidden worker launch, including redirected handles.
    options = dict(capture_output=True, timeout=10, creationflags=subprocess.CREATE_NO_WINDOW)
    # Both application binaries must enter IPC mode before initializing their UI.
    result = subprocess.run([str(executable), "--ptouch-usbprint-worker"],
                            input=frame(5) + frame(4), **options)
    result.check_returncode()
    expected = frame(0, b"PTOUCH_USBPRINT_WORKER_V1", 0) + frame(0, millis=0)
    if result.stdout != expected:
        raise RuntimeError(f"Invalid USBPRINT worker handshake: {result.stdout!r}")
    # Oversized requests must fail without allocating or accessing hardware.
    result = subprocess.run([str(executable), "--ptouch-usbprint-worker"],
                            input=struct.pack("<III", 1, 1000, 0xFFFFFFFF), **options)
    if result.returncode == 0:
        raise RuntimeError("USBPRINT worker accepted an oversized request")
    # A filename is not a device selector, even in the private worker protocol.
    result = subprocess.run([str(executable), "--ptouch-usbprint-worker"],
                            input=frame(1, b"C:\\Windows\\win.ini"), **options)
    result.check_returncode()
    if len(result.stdout) < 12 or struct.unpack_from("<I", result.stdout)[0] != 2:
        raise RuntimeError("USBPRINT worker accepted an arbitrary file path")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=("x86_64-pc-windows-msvc", "aarch64-pc-windows-msvc"))
    parser.add_argument("--profile", required=True, choices=("debug", "release"))
    parser.add_argument("--probe", action="store_true", help="Also validate the experimental USBPRINT executable")
    args = parser.parse_args()
    directory = Path("target") / args.target / args.profile
    machine = 0xAA64 if args.target.startswith("aarch64") else 0x8664
    cli, gui = directory / "ptouch.exe", directory / "ptouch-gui.exe"
    check_pe(cli, machine)
    check_pe(gui, machine, require_icon=True, subsystem=2)
    if args.probe:
        probe = directory / "examples" / "usbprint_probe.exe"
        check_pe(probe, machine)
        for option in ("--help", "--list"):
            subprocess.run([str(probe), option], check=True, timeout=30)
    for executable in (cli, gui):
        check_usbprint_worker(executable)
    for option in ("--version", "--help"):
        subprocess.run([str(cli), option], check=True, timeout=30)
    report = json.loads(subprocess.check_output([str(cli), "doctor", "--json"], timeout=30, text=True))
    if report["schema_version"] != 1 or report["probe"]:
        raise RuntimeError("Invalid read-only doctor report")
    if report["process_arch"] != ("aarch64" if args.target.startswith("aarch64") else "x86_64"):
        raise RuntimeError("Doctor architecture disagrees with the build target")
    result = subprocess.run([str(gui), "--smoke-test"], timeout=60, capture_output=True, text=True)
    print(result.stdout, end="")
    print(result.stderr, end="", file=sys.stderr)
    result.check_returncode()
    if "PTOUCH_GUI_SMOKE_OK" not in result.stdout:
        raise RuntimeError(f"GUI did not complete its rendering check: {result.stdout}\n{result.stderr}")
    print("PE architecture, subsystem, icon, CLI startup, and GUI rendering passed")
