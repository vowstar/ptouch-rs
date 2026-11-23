#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check the ABI contract for portable GNU/Linux release binaries."""

import argparse
import os
from pathlib import Path
import re
import subprocess


GLIBC_BASELINE = (2, 17)
TARGETS = {
    "x86_64-unknown-linux-gnu": (62, "/lib64/ld-linux-x86-64.so.2"),
    "aarch64-unknown-linux-gnu": (183, "/lib/ld-linux-aarch64.so.1"),
}


def check_elf(path, target):
    machine, interpreter = TARGETS[target]
    with path.open("rb") as binary:
        header = binary.read(20)
    if header[:6] != b"\x7fELF\x02\x01" or int.from_bytes(header[18:20], "little") != machine:
        raise ValueError(f"{path}: expected little-endian ELF64 for {target}")

    report = subprocess.check_output(
        ["readelf", "-W", "-l", "-d", "-V", str(path)],
        text=True, env={**os.environ, "LC_ALL": "C"},
    )
    if re.findall(r"Requesting program interpreter: ([^\]]+)", report) != [interpreter]:
        raise ValueError(f"{path}: expected interpreter {interpreter}")
    if "Shared library: [libudev.so.1]" not in report:
        raise ValueError(f"{path}: missing the libudev USB backend")
    if re.search(r"\((RPATH|RUNPATH|RELR)\)", report):
        raise ValueError(f"{path}: unexpected runtime search path or packed relative relocations")

    needs = report.partition("Version needs section")[2]
    versions = set()
    for name in re.findall(r"\bName: (GLIBC_\S+)", needs):
        match = re.fullmatch(r"GLIBC_(\d+(?:\.\d+)+)", name)
        if not match:
            raise ValueError(f"{path}: unsupported glibc version namespace {name}")
        versions.add(tuple(int(part) for part in match[1].split(".")))
    if not versions:
        raise ValueError(f"{path}: no glibc version needs found")
    newest = max(versions)
    version = ".".join(map(str, newest))
    if newest > GLIBC_BASELINE:
        raise ValueError(f"{path}: requires GLIBC_{version}, above 2.17")
    print(f"{path}: {target}, maximum GLIBC_{version}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("binaries", type=Path, nargs="+")
    args = parser.parse_args()
    try:
        for binary in args.binaries:
            check_elf(binary, args.target)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"{error}\n")
