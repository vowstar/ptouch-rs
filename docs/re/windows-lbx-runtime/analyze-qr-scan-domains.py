"""Compare horizontal and vertical QR mask-penalty scan domains."""
from __future__ import annotations

import importlib.util
import subprocess
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
DOCS = ROOT / "docs/re/windows-lbx-runtime"


def load_script(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


probe_list = load_script(DOCS / "analyze-qr-n2-weights.py", "n2_sweep")
score_module = load_script(DOCS / "analyze-qr-mask-margins.py", "margin_scorer")
PROBES = probe_list.PROBES


def line_components(line: list[bool]) -> tuple[int, int]:
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


def candidate_scores(data: str, mode: str, version: int):
    scores = []
    for mask in range(8):
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
        if not output[0].startswith(f"version={version} ecc=Medium mask={mask}"):
            raise AssertionError(output[0])
        matrix = [[module == "#" for module in row] for row in output[1:]]
        size = len(matrix)
        horizontal = [line_components(row) for row in matrix]
        vertical = [line_components([matrix[y][x] for y in range(size)]) for x in range(size)]
        n2_n4 = score_module.matrix_components(matrix)
        scores.append(
            {
                "n1_h": sum(item[0] for item in horizontal),
                "n3_h": sum(item[1] for item in horizontal),
                "n1_v": sum(item[0] for item in vertical),
                "n3_v": sum(item[1] for item in vertical),
                "n2": n2_n4[1],
                "n4": n2_n4[3],
            }
        )
    return scores


def main() -> None:
    observations = []
    for name, filename, mode, version, brother_mask in PROBES:
        data = tomllib.loads((DOCS / filename).read_text())["elements"][0]["content"]["data"]
        observations.append((name, brother_mask, candidate_scores(data, mode, version)))

    variants = {
        "both_axes": lambda c: c["n1_h"] + c["n1_v"] + c["n2"] + c["n3_h"] + c["n3_v"] + c["n4"],
        "horizontal_only": lambda c: c["n1_h"] + c["n2"] + c["n3_h"] + c["n4"],
        "vertical_only": lambda c: c["n1_v"] + c["n2"] + c["n3_v"] + c["n4"],
        "horizontal_n1_both_n3": lambda c: c["n1_h"] + c["n2"] + c["n3_h"] + c["n3_v"] + c["n4"],
        "vertical_n1_both_n3": lambda c: c["n1_v"] + c["n2"] + c["n3_h"] + c["n3_v"] + c["n4"],
        "both_n1_horizontal_n3": lambda c: c["n1_h"] + c["n1_v"] + c["n2"] + c["n3_h"] + c["n4"],
        "both_n1_vertical_n3": lambda c: c["n1_h"] + c["n1_v"] + c["n2"] + c["n3_v"] + c["n4"],
    }
    for label, function in variants.items():
        matches = 0
        residuals = []
        mismatches = []
        for name, brother_mask, components in observations:
            totals = [function(candidate) for candidate in components]
            winner = min(range(8), key=totals.__getitem__)
            matches += winner == brother_mask
            if winner != brother_mask:
                mismatches.append(f"{name}=mask{winner}/Brother{brother_mask}")
            if name in {"cycle37-55", "cycle38-upper-lower-upper"}:
                residuals.append(f"{name}=mask{winner}")
        print(
            f"{label}: matched={matches}/19; {' '.join(residuals)}; "
            f"mismatches={' '.join(mismatches)}"
        )


if __name__ == "__main__":
    main()
