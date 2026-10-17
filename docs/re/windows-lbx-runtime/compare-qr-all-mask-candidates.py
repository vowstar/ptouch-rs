"""Compare all forced Brother QR masks with Rust fixed-mask matrices."""
from __future__ import annotations

import subprocess
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).resolve().parents[3]
DOCS = ROOT / "docs/re/windows-lbx-runtime"
CASES = [
    ("abc123", "ABC123abc789", "manual:alphanumeric:0-6,byte:6-12", 1),
    ("repeat55", "a" * 54 + "b", "auto", 4),
]
PITCH = 8
ORIGIN_X, ORIGIN_Y = 30, 20


def rust_matrix(data: str, mode: str, version: int, mask: int) -> list[str]:
    output = subprocess.run(
        [
            "cargo",
            "run",
            "-q",
            "-p",
            "ptouch-render",
            "--example",
            "dump_qr_matrix",
            "--",
            data,
            mode,
            str(version),
            str(mask),
        ],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    expected_header = f"version={version} ecc=Medium mask={mask}"
    if not output[0].startswith(expected_header):
        raise AssertionError(f"unexpected Rust matrix header: {output[0]}")
    return output[1:]


def compare(case: str, matrix: list[str], mask: int) -> tuple[int, int]:
    image_path = (
        DOCS
        / "runtime-renders"
        / f"editor-pt-p900w-qr-mask-override-{case}-mask-{mask}-cycle44.bmp"
    )
    image = Image.open(image_path).convert("L")
    compared = mismatches = 0
    for y, row in enumerate(matrix):
        for x, module in enumerate(row):
            expected = module == "#"
            for py in range(ORIGIN_Y + y * PITCH, ORIGIN_Y + (y + 1) * PITCH):
                for px in range(ORIGIN_X + x * PITCH, ORIGIN_X + (x + 1) * PITCH):
                    compared += 1
                    mismatches += (image.getpixel((px, py)) < 128) != expected
    return mismatches, compared


def main() -> None:
    for case, data, mode, version in CASES:
        for mask in range(8):
            matrix = rust_matrix(data, mode, version, mask)
            mismatches, compared = compare(case, matrix, mask)
            print(f"{case} mask={mask}: pixel_mismatches={mismatches}/{compared}")
            if mismatches:
                raise SystemExit(1)


if __name__ == "__main__":
    main()
