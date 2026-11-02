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
import threading
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


def check_cli_launches(cli, report_dir):
    """A missing command must not block scripts or an existing shell."""
    report_dir.mkdir(parents=True, exist_ok=True)
    options = dict(stdin=subprocess.DEVNULL, capture_output=True, text=True, timeout=15)
    for name, arguments, shell in (
        ("redirected", [str(cli)], False),
        ("cmd", [str(cli)], True),
        ("powershell", ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
                        "& '" + str(cli).replace("'", "''") + "'; exit $LASTEXITCODE"], False),
    ):
        result = subprocess.run(arguments, shell=shell, **options)
        (report_dir / f"{name}.txt").write_text(result.stdout + result.stderr, encoding="utf-8")
        if result.returncode != 2 or "Usage:" not in result.stdout + result.stderr:
            raise RuntimeError(f"{name}: no-argument CLI must show help and exit 2")
    instructions = subprocess.check_output([str(cli), "gui"], text=True, timeout=15)
    suffix = "arm64" if "arm64" in cli.name else "amd64"
    if f"ptouch-gui-windows-{suffix}.exe" not in instructions:
        raise RuntimeError("GUI instructions point to the wrong executable")
    # Create a real standalone console without inheriting the CI runner's pipes.
    # This is the classic Explorer double-click case.
    from ctypes import wintypes
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)

    class StartupInfo(ctypes.Structure):
        _fields_ = [("cb", wintypes.DWORD), ("reserved", wintypes.LPWSTR),
                    ("desktop", wintypes.LPWSTR), ("title", wintypes.LPWSTR),
                    ("x", wintypes.DWORD), ("y", wintypes.DWORD),
                    ("width", wintypes.DWORD), ("height", wintypes.DWORD),
                    ("x_chars", wintypes.DWORD), ("y_chars", wintypes.DWORD),
                    ("fill", wintypes.DWORD), ("flags", wintypes.DWORD),
                    ("show", wintypes.WORD), ("reserved_size", wintypes.WORD),
                    ("reserved_bytes", ctypes.c_void_p), ("stdin", wintypes.HANDLE),
                    ("stdout", wintypes.HANDLE), ("stderr", wintypes.HANDLE)]

    class ProcessInfo(ctypes.Structure):
        _fields_ = [("process", wintypes.HANDLE), ("thread", wintypes.HANDLE),
                    ("pid", wintypes.DWORD), ("tid", wintypes.DWORD)]

    kernel32.CreateProcessW.argtypes = [wintypes.LPCWSTR, wintypes.LPWSTR, ctypes.c_void_p,
                                      ctypes.c_void_p, wintypes.BOOL, wintypes.DWORD,
                                      ctypes.c_void_p, wintypes.LPCWSTR,
                                      ctypes.POINTER(StartupInfo), ctypes.POINTER(ProcessInfo)]
    kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
    kernel32.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    kernel32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    check_pseudoconsole_cli(cli, kernel32, StartupInfo, ProcessInfo, report_dir)
    startup, process = StartupInfo(), ProcessInfo()
    startup.cb = ctypes.sizeof(startup)
    command = ctypes.create_unicode_buffer(subprocess.list2cmdline([str(cli)]))
    if not kernel32.CreateProcessW(str(cli), command, None, None, False, 0x10,
                                  None, str(cli.parent), ctypes.byref(startup), ctypes.byref(process)):
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        deadline = time.monotonic() + 20
        window = None
        while time.monotonic() < deadline:
            window = owned_window(process.pid, "P-Touch command-line tool")
            if window or kernel32.WaitForSingleObject(process.process, 0) == 0:
                break
            time.sleep(0.1)
        if window is None:
            raise RuntimeError("Standalone CLI launch did not display the editor download guide")
        capture_guide(window, report_dir)
        # Choosing No must close just the guide, without opening a browser.
        user32 = ctypes.WinDLL("user32", use_last_error=True)
        user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
        user32.PostMessageW(window, 0x0111, 7, 0)  # WM_COMMAND, IDNO
        if kernel32.WaitForSingleObject(process.process, 10000) != 0:
            raise RuntimeError("CLI download guide did not close")
        code = wintypes.DWORD()
        if not kernel32.GetExitCodeProcess(process.process, ctypes.byref(code)) or code.value != 2:
            raise RuntimeError("Desktop guidance changed the missing-command exit status")
        (report_dir / "desktop.txt").write_text("Download guide displayed and closed, exit 2.\n")
    finally:
        if kernel32.WaitForSingleObject(process.process, 0) != 0:
            kernel32.TerminateProcess(process.process, 1)
            kernel32.WaitForSingleObject(process.process, 10000)
        kernel32.CloseHandle(process.thread)
        kernel32.CloseHandle(process.process)


def check_pseudoconsole_cli(cli, kernel32, startup_type, process_type, report_dir):
    """A lone ConPTY client, as used by modern terminals, must not show a dialog."""
    from ctypes import wintypes

    class Coord(ctypes.Structure):
        _fields_ = [("x", ctypes.c_short), ("y", ctypes.c_short)]

    class ExtendedStartup(ctypes.Structure):
        _fields_ = [("startup", startup_type), ("attributes", ctypes.c_void_p)]

    kernel32.CreatePipe.argtypes = [ctypes.POINTER(wintypes.HANDLE), ctypes.POINTER(wintypes.HANDLE), ctypes.c_void_p, wintypes.DWORD]
    kernel32.CreatePseudoConsole.argtypes = [Coord, wintypes.HANDLE, wintypes.HANDLE, wintypes.DWORD, ctypes.POINTER(wintypes.HANDLE)]
    kernel32.CreatePseudoConsole.restype = wintypes.LONG
    kernel32.ClosePseudoConsole.argtypes = [wintypes.HANDLE]
    kernel32.ClosePseudoConsole.restype = None
    kernel32.InitializeProcThreadAttributeList.argtypes = [ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(ctypes.c_size_t)]
    kernel32.UpdateProcThreadAttribute.argtypes = [ctypes.c_void_p, wintypes.DWORD, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_void_p]
    kernel32.DeleteProcThreadAttributeList.argtypes = [ctypes.c_void_p]
    kernel32.ReadFile.argtypes = [wintypes.HANDLE, ctypes.c_void_p, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD), ctypes.c_void_p]
    input_read, input_write, output_read, output_write, console = [wintypes.HANDLE() for _ in range(5)]
    process = process_type()
    attributes = None
    initialized = False
    reader = None
    chunks = []

    def drain():
        buffer = ctypes.create_string_buffer(4096)
        count = wintypes.DWORD()
        while kernel32.ReadFile(output_read, buffer, len(buffer), ctypes.byref(count), None) and count.value:
            chunks.append(buffer.raw[:count.value])

    try:
        if not kernel32.CreatePipe(ctypes.byref(input_read), ctypes.byref(input_write), None, 0):
            raise ctypes.WinError(ctypes.get_last_error())
        if not kernel32.CreatePipe(ctypes.byref(output_read), ctypes.byref(output_write), None, 0):
            raise ctypes.WinError(ctypes.get_last_error())
        status = kernel32.CreatePseudoConsole(Coord(100, 40), input_read, output_write, 0, ctypes.byref(console))
        if status != 0:
            raise RuntimeError(f"Cannot create ConPTY: HRESULT {status & 0xffffffff:#x}")
        size = ctypes.c_size_t()
        kernel32.InitializeProcThreadAttributeList(None, 1, 0, ctypes.byref(size))
        attributes = ctypes.create_string_buffer(size.value)
        if not kernel32.InitializeProcThreadAttributeList(attributes, 1, 0, ctypes.byref(size)):
            raise ctypes.WinError(ctypes.get_last_error())
        initialized = True
        if not kernel32.UpdateProcThreadAttribute(attributes, 0, 0x00020016, console, ctypes.sizeof(console), None, None):
            raise ctypes.WinError(ctypes.get_last_error())
        startup = ExtendedStartup()
        startup.startup.cb = ctypes.sizeof(startup)
        startup.attributes = ctypes.cast(attributes, ctypes.c_void_p)
        reader = threading.Thread(target=drain, daemon=True)
        reader.start()
        command = ctypes.create_unicode_buffer(subprocess.list2cmdline([str(cli)]))
        if not kernel32.CreateProcessW(str(cli), command, None, None, False, 0x00080000, None,
                                      str(cli.parent), ctypes.cast(ctypes.byref(startup), ctypes.POINTER(startup_type)), ctypes.byref(process)):
            raise ctypes.WinError(ctypes.get_last_error())
        if kernel32.WaitForSingleObject(process.process, 15000) != 0:
            raise RuntimeError("ConPTY CLI blocked instead of returning help")
        code = wintypes.DWORD()
        if not kernel32.GetExitCodeProcess(process.process, ctypes.byref(code)) or code.value != 2:
            raise RuntimeError("ConPTY changed the missing-command exit status")
    finally:
        if process.process:
            if kernel32.WaitForSingleObject(process.process, 0) != 0:
                kernel32.TerminateProcess(process.process, 1)
                kernel32.WaitForSingleObject(process.process, 10000)
            kernel32.CloseHandle(process.thread)
            kernel32.CloseHandle(process.process)
        for handle in (input_read, input_write, output_write):
            if handle:
                kernel32.CloseHandle(handle)
        if console:
            kernel32.ClosePseudoConsole(console)
        if reader:
            reader.join(timeout=5)
        if output_read:
            kernel32.CloseHandle(output_read)
        if initialized:
            kernel32.DeleteProcThreadAttributeList(attributes)
    output = b"".join(chunks).decode("utf-8", errors="replace")
    (report_dir / "conpty.txt").write_text(output, encoding="utf-8")
    if "Usage:" not in output:
        raise RuntimeError("ConPTY did not receive command-line help")


def capture_guide(window, report_dir):
    """Capture only the owned download guide for visual review."""
    path = str(report_dir / "download-guide.png").replace("'", "''")
    script = """Add-Type -AssemblyName System.Drawing
Add-Type 'using System; using System.Runtime.InteropServices; public class CaptureGuide {
[DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out Rect rect);
public struct Rect { public int Left, Top, Right, Bottom; }
}'
$rect = New-Object CaptureGuide+Rect
[void][CaptureGuide]::GetWindowRect([IntPtr]WINDOW_HANDLE, [ref]$rect)
$image = New-Object System.Drawing.Bitmap -ArgumentList ($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top)
$graphics = [System.Drawing.Graphics]::FromImage($image)
$dc = $graphics.GetHdc()
try { if (-not [CaptureGuide]::PrintWindow([IntPtr]WINDOW_HANDLE, $dc, 2)) { throw 'PrintWindow failed' } }
finally { $graphics.ReleaseHdc($dc) }
$image.Save('IMAGE_PATH', [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose()
$image.Dispose()
""".replace("WINDOW_HANDLE", str(window)).replace("IMAGE_PATH", path)
    subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script], check=True, timeout=20)


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
        staged_cli = clean / f"ptouch-windows-{suffix}.exe"
        shutil.copy2(gui, staged_gui)
        shutil.copy2(cli, staged_cli)
        check_cli_launches(staged_cli, report_dir / "cli-launches")
        check_gui_startup(staged_gui, report_dir, "default-renderer", ["--offline", "--smoke-test"])
        check_gui_startup(staged_gui, report_dir, "normal-launch", [], interactive=True)
        check_gui_startup(staged_gui, report_dir, "automatic-fallback", [], force_primary_failure=True, interactive=True)
    print("PE architecture, subsystem, icon, CLI startup, clean-folder GUI and automatic fallback passed")
