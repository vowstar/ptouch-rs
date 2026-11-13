#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Check GUI command-line responses with display variables removed."""

import os
from pathlib import Path
import subprocess
import tomllib


def check_gui_arguments(executable):
    environment = os.environ.copy()
    for key in ("DISPLAY", "WAYLAND_DISPLAY", "WAYLAND_SOCKET"):
        environment.pop(key, None)
    version = tomllib.loads((Path(__file__).resolve().parents[1] / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    cases = [
        (["--help"], 0, "Usage: ptouch-gui [OPTIONS]"),
        (["-h"], 0, "Usage: ptouch-gui [OPTIONS]"),
        (["--offline", "--help"], 0, "Usage: ptouch-gui [OPTIONS]"),
        (["--smoke-test", "--help"], 0, "Usage: ptouch-gui [OPTIONS]"),
        (["--version"], 0, f"ptouch-gui {version}\n"),
        (["-V"], 0, f"ptouch-gui {version}\n"),
        (["--unknown-option"], 2, "unexpected argument '--unknown-option'"),
        (["unexpected.ptl"], 2, "unexpected argument 'unexpected.ptl'"),
    ]
    for arguments, expected_code, expected_text in cases:
        result = subprocess.run([str(executable), *arguments], env=environment,
                                stdin=subprocess.DEVNULL, capture_output=True,
                                text=True, timeout=15)
        output = result.stdout if expected_code == 0 else result.stderr
        if result.returncode != expected_code or expected_text not in output:
            raise RuntimeError(f"GUI arguments {arguments}: exit {result.returncode}\n{result.stdout}\n{result.stderr}")
        if expected_code == 0 and result.stderr:
            raise RuntimeError(f"GUI arguments {arguments} initialized diagnostics: {result.stderr}")
        if "--help" in arguments or "-h" in arguments:
            for option in ("--offline", "--smoke-test", "--help", "--version"):
                if option not in result.stdout:
                    raise RuntimeError(f"GUI help omits {option}")
        print(f"GUI {' '.join(arguments)}: passed")
