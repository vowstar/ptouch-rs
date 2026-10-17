"""Compare libqrencode-style QR mask scores with one white border module."""
from __future__ import annotations

import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
DOCS = ROOT / "docs/re/windows-lbx-runtime"
PROBES = [
    ("cycle40-02", "qr-mask-score-probe-02.ptl", "auto", 4, 3),
    ("cycle40-14", "qr-mask-score-probe-14.ptl", "auto", 4, 6),
    ("cycle40-16", "qr-mask-score-probe-16.ptl", "auto", 4, 2),
    ("cycle40-20", "qr-mask-score-probe-20.ptl", "auto", 4, 7),
    ("cycle40-23", "qr-mask-score-probe-23.ptl", "auto", 4, 6),
    ("cycle40-29", "qr-mask-score-probe-29.ptl", "auto", 4, 6),
    ("cycle40-30", "qr-mask-score-probe-30.ptl", "auto", 4, 0),
    (
        "cycle38-abc123abc789",
        "qr-segment-probe-upper_lower_upper.ptl",
        "manual:alphanumeric:0-6,byte:6-12",
        1,
        0,
    ),
    ("cycle37-repeat55", "qr-mask-probe-55.ptl", "auto", 4, 1),
]


def matrix_components(matrix: list[list[bool]]) -> tuple[int, int, int, int]:
    size = len(matrix)

    def line_score(line: list[bool]) -> int:
        runs = [-1] if line[0] else []
        current = line[0]
        length = 1
        for module in line[1:]:
            if module == current:
                length += 1
            else:
                runs.append(length)
                current = module
                length = 1
        runs.append(length)
        n1 = 0
        n3 = 0
        for index, run in enumerate(runs):
            if run >= 5:
                n1 += 3 + run - 5
            if index % 2 == 0 or index < 3 or index >= len(runs) - 2 or run % 3:
                continue
            unit = run // 3
            if not (
                runs[index - 2] == unit
                and runs[index - 1] == unit
                and runs[index + 1] == unit
                and runs[index + 2] == unit
            ):
                continue
            if index == 3 or runs[index - 3] >= 4 * unit:
                n3 += 40
            elif index + 4 >= len(runs) or runs[index + 3] >= 4 * unit:
                n3 += 40
        return n1, n3

    n1 = 0
    n3 = 0
    for line in matrix + [[matrix[y][x] for y in range(size)] for x in range(size)]:
        line_n1, line_n3 = line_score(line)
        n1 += line_n1
        n3 += line_n3
    n2 = 0
    for y in range(size - 1):
        for x in range(size - 1):
            module = matrix[y][x]
            if (
                matrix[y][x + 1] == module
                and matrix[y + 1][x] == module
                and matrix[y + 1][x + 1] == module
            ):
                n2 += 3
    dark = sum(map(sum, matrix))
    rounded_percent = (200 * dark + size * size) // (size * size) // 2
    n4 = abs(rounded_percent - 50) // 5 * 10
    return n1, n2, n3, n4


def score_candidates(
    data: str, mode: str, version: int, border: int
) -> list[tuple[int, int, int, int]]:
    scores = []
    for mask in range(8):
        result = subprocess.run(
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
        )
        rows = result.stdout.splitlines()[1:]
        matrix = [[module == "#" for module in row] for row in rows]
        if border:
            width = len(matrix) + 2 * border
            bordered = [[False] * width for _ in range(width)]
            for y, row in enumerate(matrix):
                bordered[y + border][border : border + len(row)] = row
            matrix = bordered
        scores.append(matrix_components(matrix))
    return scores


def winner_and_margin(scores: list[tuple[int, int, int, int]]) -> tuple[int, int]:
    ranked = sorted((sum(components), mask) for mask, components in enumerate(scores))
    return ranked[0][1], ranked[1][0] - ranked[0][0]


def main() -> None:
    for name, filename, mode, version, brother_mask in PROBES:
        layout = tomllib.loads((DOCS / filename).read_text())
        data = layout["elements"][0]["content"]["data"]
        plain = score_candidates(data, mode, version, 0)
        bordered = score_candidates(data, mode, version, 1)
        plain_winner, plain_margin = winner_and_margin(plain)
        border_winner, border_margin = winner_and_margin(bordered)
        print(
            f"{name}: Brother={brother_mask} plain={plain_winner} "
            f"(margin {plain_margin}) border1={border_winner} "
            f"(margin {border_margin})"
        )
        if name in {"cycle38-abc123abc789", "cycle37-repeat55"}:
            for border_name, scores in (("plain", plain), ("border1", bordered)):
                details = ", ".join(
                    f"{mask}:{'/'.join(map(str, components))}={sum(components)}"
                    for mask, components in enumerate(scores)
                )
                print(f"  {border_name} N1/N2/N3/N4 by mask: {details}")


if __name__ == "__main__":
    main()
