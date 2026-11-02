#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Package the Windows label editor and command-line tool in one portable ZIP."""

import argparse
from pathlib import Path
import zipfile


TARGETS = {"x86_64-pc-windows-msvc": "amd64", "aarch64-pc-windows-msvc": "arm64"}
MEMBERS = {"ptouch-gui.exe": "ptouch-gui.exe", "cli/ptouch.exe": "ptouch.exe"}


def build(directory, output, target):
    for source in MEMBERS.values():
        path = directory / source
        if not path.is_file() or path.stat().st_size == 0:
            raise ValueError(f"Missing or empty Windows executable: {path}")
    output.mkdir(parents=True, exist_ok=True)
    archive = output / f"ptouch-windows-{TARGETS[target]}.zip"
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as bundle:
        for name, source in MEMBERS.items():
            bundle.write(directory / source, name)
    return archive


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--profile", required=True, choices=("debug", "release"))
    parser.add_argument("--output", type=Path, default=Path("target/windows-package"))
    args = parser.parse_args()
    print(build(Path("target") / args.target / args.profile, args.output, args.target))
