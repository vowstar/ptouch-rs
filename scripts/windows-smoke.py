#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check Windows executable architecture, subsystem, icons, and native startup."""

import argparse
import ctypes
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import sys
import tempfile
import time


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


def pe_imports(path):
    """Read DLL imports without relying on a compiler toolchain on PATH."""
    data = path.read_bytes()
    pe = struct.unpack_from("<I", data, 0x3C)[0]
    section_count = struct.unpack_from("<H", data, pe + 6)[0]
    optional = pe + 24
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]

    def offset(rva):
        for index in range(section_count):
            section = optional + optional_size + index * 40
            size, address, raw_size, raw = struct.unpack_from("<IIII", data, section + 8)
            if address <= rva < address + min(size, raw_size):
                return raw + rva - address
        raise ValueError(f"{path}: unmapped import RVA {rva:#x}")

    import_rva = struct.unpack_from("<I", data, optional + 112 + 8)[0]
    if not import_rva:
        return set()
    cursor = offset(import_rva)
    result = set()
    while any(data[cursor:cursor + 20]):
        name = offset(struct.unpack_from("<I", data, cursor + 12)[0])
        end = data.index(b"\0", name)
        result.add(data[name:end].decode("ascii").lower())
        cursor += 20
    return result


def owned_window(pid, title):
    """Find a visible window owned by the exact process under test."""
    from ctypes import wintypes
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
    user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user32.IsWindowVisible.argtypes = [wintypes.HWND]
    user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
    found = []

    @callback_type
    def visit(window, _):
        owner = wintypes.DWORD()
        user32.GetWindowThreadProcessId(window, ctypes.byref(owner))
        if owner.value == pid and user32.IsWindowVisible(window):
            text = ctypes.create_unicode_buffer(1024)
            user32.GetWindowTextW(window, text, len(text))
            if text.value.startswith(title):
                found.append(window)
        return True

    user32.EnumWindows(visit, 0)
    return found[0] if found else None


def close_window(window):
    from ctypes import wintypes
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
    if not user32.PostMessageW(window, 0x0010, 0, 0):  # WM_CLOSE
        raise ctypes.WinError(ctypes.get_last_error())


def check_gui_startup(gui, report_dir, name, arguments, expect_failure=False, interactive=False):
    profile = report_dir / name
    profile.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(LOCALAPPDATA=str(profile / "local"), APPDATA=str(profile / "roaming"), RUST_LOG="error")
    env.pop("WGPU_BACKEND", None)
    if expect_failure:
        # A browser-only backend is unavailable to this native Windows binary.
        env["WGPU_BACKEND"] = "webgpu"
    elif "--software-renderer" in arguments:
        # Explicit recovery also overrides a stale backend override.
        env["WGPU_BACKEND"] = "webgpu"
    stdout_path, stderr_path = profile / "stdout.txt", profile / "stderr.txt"
    with stdout_path.open("w") as stdout, stderr_path.open("w") as stderr:
        process = subprocess.Popen([str(gui), *arguments], cwd=gui.parent, env=env,
                                   stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr)
        try:
            if interactive:
                title = "P-Touch error" if expect_failure else "P-Touch Label Printer"
                deadline = time.monotonic() + 40
                window = None
                while time.monotonic() < deadline and process.poll() is None:
                    window = owned_window(process.pid, title)
                    if window:
                        break
                    time.sleep(0.1)
                if window is None:
                    raise RuntimeError(f"{name}: visible window not found, exit={process.poll()}")
                if not expect_failure:
                    # Exercise an ordinary startup, including its printer worker.
                    time.sleep(2)
                    if process.poll() is not None:
                        raise RuntimeError(f"{name}: window closed immediately")
                close_window(window)
            process.wait(timeout=60)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=10)
    output, errors = stdout_path.read_text(errors="replace"), stderr_path.read_text(errors="replace")
    log_path = profile / "local" / "ptouch-rs" / "startup.log"
    log = log_path.read_text(encoding="utf-8") if log_path.is_file() else ""
    if expect_failure:
        if process.returncode == 0 or not any(marker in log for marker in ["Startup error:", "Panic:"]):
            raise RuntimeError(f"{name}: failed startup did not retain its error\n{errors}\n{log}")
    elif process.returncode != 0 or "Editor initialized" not in log or "Renderer: Dx12" not in log:
        raise RuntimeError(f"{name}: startup failed, exit={process.returncode}\n{errors}\n{log}")
    elif not interactive and "PTOUCH_GUI_SMOKE_OK" not in output:
        raise RuntimeError(f"{name}: renderer did not finish its frames\n{errors}\n{log}")
    if "--software-renderer" in arguments and "Adapter type: Cpu" not in log:
        raise RuntimeError(f"{name}: software renderer was not used\n{log}")
    print(f"{name}: passed")


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
    if "opengl32.dll" in pe_imports(gui):
        raise RuntimeError("Windows GUI still imports OpenGL")
    report_dir = (directory / "windows-smoke").resolve()
    suffix = "arm64" if args.target.startswith("aarch64") else "amd64"
    # Only the distributed executables go into this folder. In particular, no
    # CI-only graphics DLL from the build directory can make these checks pass.
    with tempfile.TemporaryDirectory(prefix="ptouch-启动-") as temporary:
        clean = Path(temporary)
        staged_gui = clean / f"ptouch-gui-windows-{suffix}.exe"
        shutil.copy2(gui, staged_gui)
        check_gui_startup(staged_gui, report_dir, "default-renderer", ["--offline", "--smoke-test"])
        check_gui_startup(staged_gui, report_dir, "software-renderer", ["--offline", "--smoke-test", "--software-renderer"])
        check_gui_startup(staged_gui, report_dir, "normal-launch", [], interactive=True)
        check_gui_startup(staged_gui, report_dir, "failed-startup", ["--offline", "--smoke-test"], expect_failure=True)
        check_gui_startup(staged_gui, report_dir, "visible-startup-error", ["--offline"], expect_failure=True, interactive=True)
    print("PE architecture, subsystem, icon, CLI startup, clean-folder GUI rendering and startup errors passed")
