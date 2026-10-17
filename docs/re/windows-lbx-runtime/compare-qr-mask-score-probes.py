#!/usr/bin/env python3
"""Compare Brother mask-score probe rasters with fixed-mask Rust matrices."""

from pathlib import Path

from PIL import Image

ROOT = Path(__file__).parent
CASES = [
    ("02", 3),
    ("14", 6),
    ("16", 2),
    ("20", 7),
    ("23", 6),
    ("29", 6),
    ("30", 0),
]

for probe, expected_mask in CASES:
    matrix_path = ROOT / "qr-matrices" / "mask-score-probes" / f"{probe}-mask{expected_mask}.txt"
    lines = matrix_path.read_text(encoding="ascii").splitlines()
    metadata = dict(field.split("=", 1) for field in lines[0].split())
    if (metadata.get("version"), metadata.get("ecc"), metadata.get("mask")) != (
        "4",
        "Medium",
        str(expected_mask),
    ):
        raise SystemExit(f"{probe}: unexpected candidate metadata {metadata}")
    matrix = lines[1:]
    image_path = ROOT / "runtime-renders" / f"editor-pt-p900w-qr-mask-score-probe-{probe}.bmp"
    image = Image.open(image_path).convert("L")
    pitch, origin_x, origin_y = 8, 30, 20
    compared = mismatches = 0
    for y, row in enumerate(matrix):
        for x, module in enumerate(row):
            expected_black = module == "#"
            for py in range(origin_y + y * pitch, origin_y + (y + 1) * pitch):
                for px in range(origin_x + x * pitch, origin_x + (x + 1) * pitch):
                    compared += 1
                    mismatches += (image.getpixel((px, py)) < 128) != expected_black
    if mismatches:
        raise SystemExit(f"{probe}: pixel mismatches {mismatches}/{compared}")
    print(f"{probe}: Brother mask={expected_mask}, pixel_mismatches=0/{compared}")
