#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Build and check the complete macOS app without an Apple signing account."""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import plistlib
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import tomllib
import uuid
import zipfile

ROOT = Path(__file__).resolve().parents[1]
BUNDLE_ID = "io.github.vowstar.ptouch-gui"
APP_NAME = "ptouch-gui.app"
ARCHIVE_NAME = "ptouch-gui-macos-arm64.app.zip"


def version():
    return tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]


def command(arguments, report=None, name=None, check=True):
    result = subprocess.run([str(arg) for arg in arguments], stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, errors="replace", timeout=90)
    if report is not None:
        (report / f"{name}.txt").write_text(result.stdout, encoding="utf-8")
    print(result.stdout, end="")
    if check:
        result.check_returncode()
    return result


def sign(path, identifier):
    command(["codesign", "--force", "--sign", "-", "--timestamp=none",
             "--identifier", identifier, path])
    command(["codesign", "--verify", "--deep", "--strict", path])


def check_bundle(app, expected_version):
    with (app / "Contents/Info.plist").open("rb") as stream:
        info = plistlib.load(stream)
    expected = {"CFBundleIdentifier": BUNDLE_ID, "CFBundleExecutable": "ptouch-gui",
                "CFBundleShortVersionString": expected_version, "CFBundleVersion": expected_version}
    if any(info.get(key) != value for key, value in expected.items()):
        raise ValueError(f"Incorrect app metadata: expected {expected}")
    if not info.get("NSBluetoothAlwaysUsageDescription", "").strip():
        raise ValueError("Missing Bluetooth permission description")
    icon = app / "Contents/Resources/ptouch-gui.icns"
    if icon.read_bytes()[:4] != b"icns":
        raise ValueError("Missing or invalid app icon")
    executable = app / "Contents/MacOS/ptouch-gui"
    with executable.open("rb") as stream:
        header = stream.read(8)
    if header != b"\xcf\xfa\xed\xfe\x0c\x00\x00\x01" or not os.access(executable, os.X_OK):
        raise ValueError("Expected an executable ARM64 Mach-O")
    if not (app / "Contents/_CodeSignature/CodeResources").is_file():
        raise ValueError("Missing complete app resource signature")
    return executable


def check_archive(path):
    with zipfile.ZipFile(path) as archive:
        if not archive.infolist():
            raise ValueError("Empty app archive")
        for entry in archive.infolist():
            member = PurePosixPath(entry.filename)
            if (member.is_absolute() or ".." in member.parts or not member.parts
                    or member.parts[0] not in (APP_NAME, "__MACOSX")
                    or stat.S_ISLNK(entry.external_attr >> 16)):
                raise ValueError(f"Unexpected archive member: {entry.filename}")


def build(target, profile, output):
    output.mkdir(parents=True, exist_ok=False)
    binaries = ROOT / "target" / target / profile
    app = output / APP_NAME
    (app / "Contents/MacOS").mkdir(parents=True)
    (app / "Contents/Resources").mkdir()
    for name in ("ptouch", "ptouch-gui"):
        raw = output / name
        shutil.copy2(binaries / name, raw)
        raw.chmod(0o755)
        sign(raw, f"io.github.vowstar.{name}")
    shutil.copy2(binaries / "ptouch-gui", app / "Contents/MacOS/ptouch-gui")
    (app / "Contents/MacOS/ptouch-gui").chmod(0o755)
    shutil.copy2(ROOT / "data/macos/ptouch-gui.icns", app / "Contents/Resources/ptouch-gui.icns")
    with (ROOT / "data/macos/Info.plist").open("rb") as stream:
        info = plistlib.load(stream)
    for key in ("CFBundleShortVersionString", "CFBundleVersion"):
        info[key] = version()
    with (app / "Contents/Info.plist").open("wb") as stream:
        plistlib.dump(info, stream, sort_keys=False)
    sign(app, BUNDLE_ID)
    check_bundle(app, version())
    command(["ditto", "-c", "-k", "--keepParent", app, output / ARCHIVE_NAME])
    check_archive(output / ARCHIVE_NAME)
    print(f"Created {output / ARCHIVE_NAME} (ad-hoc signed, not notarized)")


def stop_owned_app(executable):
    # LaunchServices starts the app outside our child process tree. Clean up only
    # this verification's executable if its rendering check timed out.
    output = subprocess.check_output(["ps", "-axo", "pid=,comm="], text=True)
    for line in output.splitlines():
        fields = line.strip().split(None, 1)
        if len(fields) == 2 and Path(fields[1]).resolve() == executable.resolve():
            try:
                os.kill(int(fields[0]), signal.SIGTERM)
            except ProcessLookupError:
                pass


def verify(archive, report):
    report.mkdir(parents=True, exist_ok=False)
    check_archive(archive)
    result = {"archive": archive.name, "version": version(), "signing": "ad-hoc",
              "notarized": False, "sha256": hashlib.sha256(archive.read_bytes()).hexdigest()}
    (report / "system.txt").write_text(subprocess.check_output(["sw_vers"], text=True))
    with tempfile.TemporaryDirectory(prefix="ptouch install 空格 ") as directory:
        unpacked = Path(directory)
        command(["ditto", "-x", "-k", archive, unpacked])
        app = unpacked / APP_NAME
        executable = check_bundle(app, version())
        command(["codesign", "--verify", "--deep", "--strict", "--verbose=2", app], report, "signature")
        identity = command(["codesign", "--display", "--verbose=4", app], report, "identity").stdout
        if "Signature=adhoc" not in identity or f"Identifier={BUNDLE_ID}" not in identity:
            raise ValueError("Expected complete ad-hoc app signature")
        # Assess an untouched copy before any rendering or negative tests. Do not
        # let a successful local launch or changed resources contaminate policy evidence.
        policy_app = unpacked / "policy" / APP_NAME
        shutil.copytree(app, policy_app)
        quarantine = f"0083;{int(time.time()):x};ptouch-ci;{uuid.uuid4()}"
        command(["xattr", "-w", "com.apple.quarantine", quarantine, policy_app])
        assessment = command(["syspolicy_check", "distribution", policy_app], report, "gatekeeper", check=False)
        result["gatekeeper_exit_code"] = assessment.returncode
        result["gatekeeper_accepted"] = assessment.returncode == 0
        result["gatekeeper_internal_error"] = "Internal Xprotect Error" in assessment.stdout
        with (report / "policy-log.txt").open("w") as stream:
            try:
                subprocess.run(["log", "show", "--last", "1m", "--style", "compact", "--info",
                                "--predicate", "process == 'syspolicyd'"],
                               stdout=stream, stderr=subprocess.STDOUT, timeout=15, check=False)
            except subprocess.TimeoutExpired:
                stream.write("Policy log collection timed out.\n")
        direct = command([executable, "--offline", "--smoke-test"], report, "renderer")
        if "PTOUCH_GUI_SMOKE_OK" not in direct.stdout:
            raise RuntimeError("Packaged GUI did not render its frames")
        stdout, stderr = report / "launch.stdout.txt", report / "launch.stderr.txt"
        try:
            command(["open", "-W", "-n", "--stdout", stdout, "--stderr", stderr,
                     "-a", app, "--args", "--offline", "--smoke-test"], report, "launch")
            if not stdout.is_file() or "PTOUCH_GUI_SMOKE_OK" not in stdout.read_text():
                raise RuntimeError("LaunchServices did not finish the GUI rendering check")
        finally:
            stop_owned_app(executable)
        # Confirm that changing a signed resource really invalidates this app.
        changed_app = unpacked / "changed" / APP_NAME
        shutil.copytree(app, changed_app)
        icon = changed_app / "Contents/Resources/ptouch-gui.icns"
        original = icon.read_bytes()
        icon.write_bytes(original + b"changed")
        try:
            damaged = command(["codesign", "--verify", "--deep", "--strict", changed_app],
                              report, "changed-resource", check=False)
            if damaged.returncode == 0:
                raise RuntimeError("Changed resource still passed signature verification")
        finally:
            icon.write_bytes(original)
        command(["codesign", "--verify", "--deep", "--strict", app])
        result["renderer"] = result["launch_services"] = result["changed_resource_rejected"] = True
        result["gatekeeper_note"] = "Ad-hoc signing does not establish Developer ID or notarization."
    (report / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print("Final ZIP signature and packaged GUI rendering passed. This app is not notarized.")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    package = commands.add_parser("build")
    package.add_argument("--target", default="aarch64-apple-darwin", choices=("aarch64-apple-darwin",))
    package.add_argument("--profile", choices=("debug", "release"), required=True)
    package.add_argument("--output", type=Path, required=True)
    validation = commands.add_parser("verify")
    validation.add_argument("archive", type=Path)
    validation.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("Native macOS is required for signing and app startup checks")
    if args.command == "build":
        build(args.target, args.profile, args.output.resolve())
    else:
        verify(args.archive.resolve(), args.report.resolve())
