#!/usr/bin/env python3
"""Compare Brother P900W QR rasters with candidate mixed-mode segment grids."""

from pathlib import Path

from PIL import Image

ROOT = Path(__file__).parent
CASES = [
    ("lowercase then numeric", "lower-to-numeric.txt", "lower_to_numeric"),
    ("numeric then lowercase", "numeric-to-lower.txt", "numeric_to_lower"),
    ("uppercase then numeric", "uppercase-to-numeric.txt", "upper_mixed"),
    ("byte then uppercase/numeric", "byte-to-alphanumeric.txt", "interleaved"),
    ("uppercase then lowercase/digit", "alphanumeric-to-byte.txt", "upper_lower_upper"),
]

for label, matrix_name, image_suffix in CASES:
    lines = (
        ROOT / "qr-matrices" / "segment-probes" / matrix_name
    ).read_text(encoding="ascii").splitlines()
    metadata = dict(field.split("=", 1) for field in lines[0].split())
    matrix = lines[1:]
    image = Image.open(
        ROOT / "runtime-renders" / f"editor-pt-p900w-qr-segment-probe-{image_suffix}.bmp"
    ).convert("L")
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
        f"{label}: version={metadata['version']} ecc={metadata['ecc']} "
        f"mask={metadata['mask']}, pixel_mismatches={mismatches}/{compared}"
    )
