#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check Windows executable architecture, subsystem, icons, and native startup."""

import argparse
from contextlib import contextmanager
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


def check_window_size(window):
    from ctypes import wintypes
    user32 = ctypes.WinDLL("user32", use_last_error=True)
    user32.GetClientRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
    user32.GetDpiForWindow.argtypes = [wintypes.HWND]
    user32.GetDpiForWindow.restype = wintypes.UINT
    rect = wintypes.RECT()
    if not user32.GetClientRect(window, ctypes.byref(rect)):
        raise ctypes.WinError(ctypes.get_last_error())
    dpi = user32.GetDpiForWindow(window)
    if not dpi:
        raise RuntimeError("Window DPI is unavailable")
    width = (rect.right - rect.left) * 96 / dpi
    height = (rect.bottom - rect.top) * 96 / dpi
    if width < 799 or height < 499:
        raise RuntimeError(f"GUI opened too small: {width:.1f} by {height:.1f} logical pixels")
    print(f"Usable GUI window: {width:.1f} by {height:.1f} logical pixels")


@contextmanager
def isolated_settings(profile):
    """Swap only this application's settings; eframe ignores the APPDATA override."""
    if os.name != "nt":
        yield
        return
    from ctypes import wintypes
    shell32 = ctypes.WinDLL("shell32", use_last_error=True)
    shell32.SHGetFolderPathW.argtypes = [wintypes.HWND, ctypes.c_int, wintypes.HANDLE,
                                       wintypes.DWORD, wintypes.LPWSTR]
    shell32.SHGetFolderPathW.restype = wintypes.LONG
    directory = ctypes.create_unicode_buffer(32768)
    if shell32.SHGetFolderPathW(None, 0x1A, None, 0, directory) != 0:  # CSIDL_APPDATA
        raise RuntimeError("Cannot locate native roaming application data")
    relative = Path("io.github.vowstar.ptouch-gui") / "data" / "app.ron"
    actual = Path(directory.value) / relative
    saved = profile / "roaming" / relative
    original = actual.read_bytes() if actual.is_file() else None
    actual.parent.mkdir(parents=True, exist_ok=True)
    try:
        if saved.is_file():
            actual.write_bytes(saved.read_bytes())
        else:
            actual.unlink(missing_ok=True)
        yield
    finally:
        if actual.is_file():
            saved.parent.mkdir(parents=True, exist_ok=True)
            saved.write_bytes(actual.read_bytes())
        if original is None:
            actual.unlink(missing_ok=True)
        else:
            actual.write_bytes(original)


def check_gui_startup(gui, report_dir, name, arguments, force_primary_failure=False,
                      interactive=False, profile=None, hold_seconds=5):
    run_dir = report_dir / name
    run_dir.mkdir(parents=True, exist_ok=True)
    profile = profile or run_dir / "profile"
    for folder in ("local", "roaming", "temporary"):
        (profile / folder).mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update(LOCALAPPDATA=str(profile / "local"), APPDATA=str(profile / "roaming"),
               TEMP=str(profile / "temporary"), TMP=str(profile / "temporary"), RUST_LOG="error")
    env.pop("WGPU_BACKEND", None)
    if force_primary_failure:
        # Fail the first renderer. The ordinary launcher must recover itself.
        env["WGPU_BACKEND"] = "webgpu"
    logs = [profile / folder / "ptouch-rs" / "startup.log" for folder in ("local", "temporary")]
    offsets = {path: path.stat().st_size if path.is_file() else 0 for path in logs}
    stdout_path, stderr_path = run_dir / "stdout.txt", run_dir / "stderr.txt"
    failure = None
    with isolated_settings(profile), stdout_path.open("w") as stdout, stderr_path.open("w") as stderr:
        process = subprocess.Popen([str(gui), *arguments], cwd=gui.parent, env=env,
                                   stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr)
        try:
            if interactive:
                title = "P-Touch Label Printer"
                deadline = time.monotonic() + 40
                window = None
                while time.monotonic() < deadline and process.poll() is None:
                    window = owned_window(process.pid, title)
                    if window:
                        break
                    time.sleep(0.1)
                if window is None:
                    raise RuntimeError(f"{name}: visible window not found, exit={process.poll()}")
                # Exercise an ordinary startup, including its printer worker.
                time.sleep(hold_seconds)
                if process.poll() is not None:
                    raise RuntimeError(f"{name}: window closed immediately")
                check_window_size(window)
                close_window(window)
            process.wait(timeout=60)
        except Exception as error:
            failure = error
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=10)
    output, errors = stdout_path.read_text(errors="replace"), stderr_path.read_text(errors="replace")
    log = "\n".join(path.read_bytes()[offsets[path]:].decode("utf-8", errors="replace")
                    for path in logs if path.is_file())
    (run_dir / "startup.log").write_text(log, encoding="utf-8")
    result = {"executable": str(gui), "exit_code": process.returncode,
              "exit_hex": f"0x{process.returncode & 0xffffffff:08X}",
              "fresh_log": bool(log), "failure": str(failure) if failure else None}
    (run_dir / "result.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    if failure or process.returncode != 0 or "Editor initialized" not in log or "Renderer: Dx12" not in log:
        raise RuntimeError(f"{name}: startup failed, exit={result['exit_hex']}, {failure}\n{errors}\n{log}")
    elif not interactive and "PTOUCH_GUI_SMOKE_OK" not in output:
        raise RuntimeError(f"{name}: renderer did not finish its frames\n{errors}\n{log}")
    if force_primary_failure and ("Adapter type: Cpu" not in log or "Trying Windows software rendering" not in log):
        raise RuntimeError(f"{name}: automatic software fallback was not used\n{log}")
    print(f"{name}: passed")
    return result


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
        check_gui_startup(staged_gui, report_dir, "normal-launch", [], interactive=True)
        check_gui_startup(staged_gui, report_dir, "automatic-fallback", [], force_primary_failure=True, interactive=True)
    print("PE architecture, subsystem, icon, CLI startup, clean-folder GUI and automatic fallback passed")
