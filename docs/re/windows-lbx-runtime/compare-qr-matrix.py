#!/usr/bin/env python3
"""Compare Brother QR raster pixels against a known version 3 module grid."""

from pathlib import Path

from PIL import Image

ROOT = Path(__file__).parent
MATRIX_PATH = ROOT / "qr-v3-mask7.matrix"
CASES = [
    ("E550W XS", 2, 4, -10, "runtime-renders/editor-pt-e550w-qr-cellsize-xs.bmp"),
    ("E550W S", 3, 6, -8, "runtime-renders/editor-pt-e550w-qr-cellsize-s.bmp"),
    ("E550W M", 4, 8, -6, "runtime-renders/editor-pt-e550w-qr.bmp"),
    ("E550W L", 5, 10, -4, "runtime-renders/editor-pt-e550w-qr-cellsize-l.bmp"),
    ("E550W XL", 6, 12, -2, "runtime-renders/editor-pt-e550w-qr-cellsize-xl.bmp"),
    ("P900W M", 8, 30, 20, "runtime-renders/editor-pt-p900w-qr.bmp"),
]

matrix = MATRIX_PATH.read_text(encoding="ascii").splitlines()
if len(matrix) != 29 or any(len(row) != 29 for row in matrix):
    raise SystemExit("reference matrix must be 29 rows of 29 modules")

for label, cell, origin_x, origin_y, relative_path in CASES:
    image = Image.open(ROOT / relative_path).convert("L")
    compared = mismatches = 0
    visible_rows = set()
    for y in range(image.height):
        row = (y - origin_y) // cell
        if not 0 <= row < 29:
            continue
        for x in range(image.width):
            column = (x - origin_x) // cell
            if not 0 <= column < 29:
                continue
            expected_black = matrix[row][column] == "#"
            actual_black = image.getpixel((x, y)) < 128
            compared += 1
            mismatches += actual_black != expected_black
            visible_rows.add(row)
    print(
        f"{label}: origin=({origin_x},{origin_y}), "
        f"visible_rows={min(visible_rows)}..{max(visible_rows)}, "
        f"pixel_mismatches={mismatches}/{compared}"
    )
