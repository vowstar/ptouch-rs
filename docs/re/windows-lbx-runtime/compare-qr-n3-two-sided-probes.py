"""Compare Brother's in-memory captures for two-sided N3 probes with Rust."""
from __future__ import annotations

import importlib.util
import subprocess
import tomllib
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
DOCS = ROOT / "docs/re/windows-lbx-runtime"
spec = importlib.util.spec_from_file_location(
    "qr_format_reader", DOCS / "extract-qr-format-probes.py"
)
format_reader = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(format_reader)


def main() -> None:
    for index in (4, 14, 19):
        suffix = f"{index:02}"
        source = DOCS / f"qr-n3-two-sided-{suffix}.ptl"
        data = tomllib.loads(source.read_text())["elements"][0]["content"]["data"]
        capture = DOCS / "runtime-renders" / f"editor-pt-p900w-qr-n3-two-sided-{suffix}-cycle50.bmp"
        version, ecc, mask, errors = format_reader.read_capture(capture)
        if (version, ecc) != (4, "M") or errors > 3:
            raise AssertionError(f"unexpected QR format: {version}/{ecc}/mask{mask}, errors={errors}")
        output = subprocess.run(
            ["cargo", "run", "-q", "-p", "ptouch-render", "--example",
             "dump_qr_matrix", "--", data, "auto", str(version), str(mask)],
            cwd=ROOT, check=True, capture_output=True, text=True,
        ).stdout.splitlines()
        if not output[0].startswith(f"version=4 ecc=Medium mask={mask}"):
            raise AssertionError(output[0])
        matrix = output[1:]
        image = Image.open(capture).convert("L")
        compared = mismatches = 0
        for y, row in enumerate(matrix):
            for x, module in enumerate(row):
                expected = module == "#"
                for py in range(20 + y * 8, 20 + (y + 1) * 8):
                    for px in range(30 + x * 8, 30 + (x + 1) * 8):
                        compared += 1
                        mismatches += (image.getpixel((px, py)) < 128) != expected
        print(f"probe{suffix}: version={version} ecc={ecc} mask={mask} grid_mismatches={mismatches}/{compared}")
        if mismatches:
            raise SystemExit(1)


if __name__ == "__main__":
    main()
