#!/usr/bin/env python3
"""Compare Brother P900W QR rasters with payload-specific Rust candidates."""

from pathlib import Path

from PIL import Image

ROOT = Path(__file__).parent
CASES = [
    ("50-byte repetitive", "50-mask1.txt", "editor-pt-p900w-qr-mask-probe-50.bmp", 1),
    ("55-byte repetitive", "55-mask1.txt", "editor-pt-p900w-qr-mask-probe-55.bmp", 1),
    ("62-byte repetitive", "62-mask1.txt", "editor-pt-p900w-qr-mask-probe-62.bmp", 1),
    (
        "60-byte diverse with byte/numeric split",
        "diverse-byte-numeric-mask2.txt",
        "editor-pt-p900w-qr-mask-probe-diverse.bmp",
        2,
    ),
]

for label, matrix_name, image_name, expected_mask in CASES:
    matrix_lines = (
        ROOT / "qr-matrices" / "mask-probes" / matrix_name
    ).read_text(encoding="ascii").splitlines()
    metadata = dict(field.split("=", 1) for field in matrix_lines[0].split())
    if int(metadata["mask"]) != expected_mask:
        raise SystemExit(f"{label}: unexpected Rust candidate mask {metadata['mask']}")
    matrix = matrix_lines[1:]
    image = Image.open(ROOT / "runtime-renders" / image_name).convert("L")
    pitch, origin_x, origin_y = 8, 30, 20
    compared = mismatches = 0
    for y in range(image.height):
        row = (y - origin_y) // pitch
        if not 0 <= row < len(matrix):
            continue
        for x in range(image.width):
            column = (x - origin_x) // pitch
            if not 0 <= column < len(matrix):
                continue
            expected_black = matrix[row][column] == "#"
            actual_black = image.getpixel((x, y)) < 128
            compared += 1
            mismatches += actual_black != expected_black
    print(
        f"{label}: Rust mask={expected_mask}, "
        f"pixel_mismatches={mismatches}/{compared}"
    )
