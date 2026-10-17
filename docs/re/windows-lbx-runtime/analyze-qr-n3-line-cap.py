"""Test a one-N3-per-line variant and find discriminating byte payloads."""
from __future__ import annotations

import importlib.util
import random
import string
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
axis = load_script(DOCS / "analyze-qr-scan-domains.py", "axis_scorer")
score = load_script(DOCS / "analyze-qr-mask-margins.py", "margin_scorer")


def candidates(data: str, mode: str = "auto", version: int = 4):
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
        horizontal = [axis.line_components(row) for row in matrix]
        vertical = [axis.line_components([matrix[y][x] for y in range(size)]) for x in range(size)]
        base = score.matrix_components(matrix)
        n3_h_capped = sum(min(40, item[1]) for item in horizontal)
        n3_v_capped = sum(min(40, item[1]) for item in vertical)
        scores.append(
            {
                "base": sum(base),
                "cap": base[0] + base[1] + n3_h_capped + n3_v_capped + base[3],
            }
        )
    return scores


def winner_margin(values: list[int]) -> tuple[int, int]:
    ordered = sorted(values)
    return values.index(ordered[0]), ordered[1] - ordered[0]


def main() -> None:
    observed = []
    for name, filename, mode, version, brother_mask in probe_list.PROBES:
        data = tomllib.loads((DOCS / filename).read_text())["elements"][0]["content"]["data"]
        table = candidates(data, mode, version)
        observed.append((name, brother_mask, table))
    for variant in ("base", "cap"):
        matches = sum(
            winner_margin([candidate[variant] for candidate in table])[0] == brother_mask
            for _, brother_mask, table in observed
        )
        residuals = [
            f"{name}=mask{winner_margin([c[variant] for c in table])[0]}"
            for name, _, table in observed
            if name in {"cycle37-55", "cycle38-upper-lower-upper"}
        ]
        print(f"controls {variant}: {matches}/19; {' '.join(residuals)}")
    changed_controls = sum(
        any(candidate["base"] != candidate["cap"] for candidate in table)
        for _, _, table in observed
    )
    print(f"controls with changed candidate scores={changed_controls}/19")

    rng = random.Random(4907)
    candidates_found = []
    changed_random = 0
    for index in range(24):
        data = "".join(rng.choice(string.ascii_lowercase) for _ in range(50))
        table = candidates(data)
        changed_random += any(candidate["base"] != candidate["cap"] for candidate in table)
        baseline = [candidate["base"] for candidate in table]
        capped = [candidate["cap"] for candidate in table]
        base_mask, base_margin = winner_margin(baseline)
        cap_mask, cap_margin = winner_margin(capped)
        if base_mask != cap_mask:
            candidates_found.append(
                (min(base_margin, cap_margin), index, data, base_mask, cap_mask, base_margin, cap_margin)
            )
    for row in sorted(candidates_found, reverse=True)[:8]:
        print(f"candidate: min_margin={row[0]} index={row[1]} baseline={row[3]}/{row[5]} cap={row[4]}/{row[6]} data={row[2]}")
    print(f"candidate score vectors changed={changed_random}/24")
    print(f"discriminators={len(candidates_found)}/24")


if __name__ == "__main__":
    main()
