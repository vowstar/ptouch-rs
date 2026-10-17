"""Read ECC and mask from P900W QR BMP format bits, including clipped symbols."""
from pathlib import Path

from PIL import Image

ROOT = Path(__file__).parent / "runtime-renders"
ECC_NAMES = {0: "M", 1: "L", 2: "H", 3: "Q"}


def format_code(data: int) -> int:
    remainder = data << 10
    for bit in range(14, 9, -1):
        if remainder >> bit & 1:
            remainder ^= 0x537 << (bit - 10)
    return ((data << 10) | remainder) ^ 0x5412


def read_capture(path: Path) -> tuple[int, int, int, int]:
    image = Image.open(path).convert("L")
    pitch = 8
    origin_x, origin_y = 30, 20
    size = (image.width - 60) // pitch
    version = (size - 17) // 4
    bits = 0
    for index in range(15):
        if index <= 5:
            x, y = 8, index
        elif index == 6:
            x, y = 8, 7
        elif index == 7:
            x, y = 8, 8
        elif index == 8:
            x, y = 7, 8
        else:
            x, y = 14 - index, 8
        px = origin_x + x * pitch + pitch // 2
        py = origin_y + y * pitch + pitch // 2
        bits |= int(image.getpixel((px, py)) < 128) << index
    distance, data = min(
        ((format_code(candidate) ^ bits).bit_count(), candidate)
        for candidate in range(32)
    )
    if distance > 3:
        raise ValueError(f"{path.name}: format BCH distance {distance} exceeds 3")
    if size != 17 + 4 * version:
        raise AssertionError("invalid QR dimensions")
    return version, ECC_NAMES[data >> 3], data & 7, distance


if __name__ == "__main__":
    for capture in sorted(ROOT.glob("editor-pt-p900w-qr-ecc-*-cycle42.bmp")):
        version, ecc, mask, errors = read_capture(capture)
        print(f"{capture.name}: version={version} ecc={ecc} mask={mask} format_errors={errors}")
