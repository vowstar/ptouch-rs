#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Assemble the complete release before publishing any asset."""

import argparse
import hashlib
from pathlib import Path
import shutil
import tomllib


def asset_map(version):
    assets = {}
    for suffix in ("linux-amd64", "linux-arm64", "windows-amd64", "windows-arm64", "macos-arm64"):
        extension = ".exe" if suffix.startswith("windows-") else ""
        for binary in ("ptouch", "ptouch-gui"):
            assets[f"{binary}-{suffix}/{binary}{extension}"] = f"{binary}-{suffix}{extension}"
    assets["ptouch-gui-macos-arm64-app/ptouch-gui-macos-arm64.app.zip"] = "ptouch-gui-macos-arm64.app.zip"
    for arch in ("amd64", "arm64"):
        assets[f"ptouch-windows-{arch}-portable/ptouch-windows-{arch}.zip"] = f"ptouch-windows-{arch}.zip"
    for arch in ("amd64", "arm64"):
        assets[f"usbprint-probe-windows-{arch}/usbprint_probe.exe"] = f"usbprint-probe-windows-{arch}.exe"
    for arch, rpm_arch in (("amd64", "x86_64"), ("arm64", "aarch64")):
        for filename in (f"ptouch_{version}_{arch}.deb", f"ptouch-{version}.{rpm_arch}.rpm"):
            assets[f"ptouch-linux-{arch}-packages/{filename}"] = filename
    return assets


def assemble(artifacts, output, version):
    assets = asset_map(version)
    for source in assets:
        path = artifacts / source
        if not path.is_file() or path.stat().st_size == 0:
            raise ValueError(f"Missing or empty release artifact: {source}")
    # A fresh destination prevents old files from silently joining a release.
    output.mkdir(exist_ok=False)
    checksums = []
    for source, name in sorted(assets.items()):
        destination = output / name
        shutil.copy2(artifacts / source, destination)
        with destination.open("rb") as stream:
            digest = hashlib.file_digest(stream, "sha256").hexdigest()
        checksums.append(f"{digest}  {name}\n")
    (output / "SHA256SUMS").write_text("".join(checksums), encoding="ascii")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--artifacts", type=Path, default=Path("artifacts"))
    parser.add_argument("--output", type=Path, default=Path("release"))
    parser.add_argument("--check-version")
    args = parser.parse_args()
    manifest = Path(__file__).resolve().parents[1] / "Cargo.toml"
    version = tomllib.loads(manifest.read_text())["workspace"]["package"]["version"]
    if args.check_version is not None:
        if args.check_version != f"v{version}":
            parser.error(f"Tag {args.check_version!r} does not match Cargo version v{version}")
    else:
        assemble(args.artifacts, args.output, version)
