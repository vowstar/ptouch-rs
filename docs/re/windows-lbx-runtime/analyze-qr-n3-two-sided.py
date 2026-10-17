"""Test separate N3 penalties for both qualifying light-run sides."""
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
score = load_script(DOCS / "analyze-qr-mask-margins.py", "margin_scorer")


def n3_two_sided(line: list[bool]) -> int:
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
    total = 0
    for index, run in enumerate(runs):
        if index % 2 == 0 or index < 3 or index >= len(runs) - 2 or run % 3:
            continue
        unit = run // 3
        if not (runs[index - 2] == unit and runs[index - 1] == unit
                and runs[index + 1] == unit and runs[index + 2] == unit):
            continue
        left = index == 3 or runs[index - 3] >= 4 * unit
        right = index + 4 >= len(runs) or runs[index + 3] >= 4 * unit
        total += 40 * (int(left) + int(right))
    return total


def candidate_scores(data: str, mode: str, version: int) -> list[tuple[int, int]]:
    result = []
    for mask in range(8):
        output = subprocess.run(
            ["cargo", "run", "-q", "-p", "ptouch-render", "--example",
             "dump_qr_matrix", "--", data, mode, str(version), str(mask)],
            cwd=ROOT, check=True, capture_output=True, text=True,
        ).stdout.splitlines()
        if not output[0].startswith(f"version={version} ecc=Medium mask={mask}"):
            raise AssertionError(output[0])
        matrix = [[module == "#" for module in row] for row in output[1:]]
        components = score.matrix_components(matrix)
        size = len(matrix)
        lines = matrix + [[matrix[y][x] for y in range(size)] for x in range(size)]
        baseline = sum(components)
        alternate = baseline - components[2] + sum(n3_two_sided(line) for line in lines)
        result.append((baseline, alternate))
    return result


def winner(scores: list[int]) -> tuple[int, int]:
    ordered = sorted((score_value, mask) for mask, score_value in enumerate(scores))
    return ordered[0][1], ordered[1][0] - ordered[0][0]


def main() -> None:
    controls = []
    for name, filename, mode, version, brother_mask in probe_list.PROBES:
        data = tomllib.loads((DOCS / filename).read_text())["elements"][0]["content"]["data"]
        controls.append((name, brother_mask, candidate_scores(data, mode, version)))
    base_matches = sum(winner([row[0] for row in table])[0] == mask for _, mask, table in controls)
    alt_matches = sum(winner([row[1] for row in table])[0] == mask for _, mask, table in controls)
    print(f"controls baseline={base_matches}/19 two-sided={alt_matches}/19")
    print("residuals " + " ".join(
        f"{name}=base{winner([row[0] for row in table])[0]}/two-sided{winner([row[1] for row in table])[0]}"
        for name, _, table in controls if name in {"cycle37-55", "cycle38-upper-lower-upper"}
    ))

    rng = random.Random(5001)
    discriminators = []
    score_changes = 0
    for index in range(24):
        data = "".join(rng.choice(string.ascii_lowercase) for _ in range(50))
        table = candidate_scores(data, "auto", 4)
        score_changes += any(base != alternate for base, alternate in table)
        base, base_margin = winner([row[0] for row in table])
        alternate, alternate_margin = winner([row[1] for row in table])
        if base != alternate:
            discriminators.append((min(base_margin, alternate_margin), index, data, base,
                                   alternate, base_margin, alternate_margin))
    print(f"random score vectors changed={score_changes}/24 discriminators={len(discriminators)}/24")
    for margin, index, data, base, alternate, base_margin, alternate_margin in sorted(discriminators, reverse=True)[:8]:
        print(f"candidate index={index} min_margin={margin} baseline={base}/{base_margin} "
              f"two_sided={alternate}/{alternate_margin} data={data}")


if __name__ == "__main__":
    main()
