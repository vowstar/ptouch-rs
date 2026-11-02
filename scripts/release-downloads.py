#!/usr/bin/env python3
# SPDX-License-Identifier: MIT
"""Put the label editor downloads before the generated release notes."""

import argparse
from pathlib import Path
import tomllib


def download_notes(version):
    base = f"https://github.com/vowstar/ptouch-rs/releases/download/v{version}"
    return f"""Download the label editor for your computer:

| Computer | Download |
| --- | --- |
| Windows Intel or AMD (x64) | [Label editor]({base}/ptouch-windows-amd64.zip) |
| Windows ARM or Snapdragon | [Label editor]({base}/ptouch-windows-arm64.zip) |
| Mac with Apple silicon | [Label editor]({base}/ptouch-gui-macos-arm64.app.zip) |

On Windows, extract the ZIP and open `ptouch-gui.exe`. The command-line tool is in `cli/ptouch.exe`. On Mac, extract the ZIP and open `ptouch-gui.app`. The Mac app is ad-hoc signed and not notarized. See the [Mac installation steps](https://github.com/vowstar/ptouch-rs/blob/v{version}/README.md#macos-installation).

On Linux, install the `.deb` or `.rpm` for your architecture, then open P-Touch Label Printer.

<details>
<summary>Command-line downloads</summary>

The Windows ZIPs above include the command-line tool. Separate command-line downloads are also available:

| Computer | Download |
| --- | --- |
| Windows x64 | [Command-line tool]({base}/ptouch-windows-amd64.exe) |
| Windows ARM64 | [Command-line tool]({base}/ptouch-windows-arm64.exe) |
| Mac with Apple silicon | [Command-line tool]({base}/ptouch-macos-arm64) |
| Linux x64 | [Command-line tool]({base}/ptouch-linux-amd64) |
| Linux ARM64 | [Command-line tool]({base}/ptouch-linux-arm64) |

</details>
"""


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if args.tag != f"v{version}":
        parser.error("Release tag does not match the workspace version")
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(download_notes(version), encoding="utf-8")
