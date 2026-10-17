"""Sweep N1 weights and N4 integer-rounding variants over Brother controls."""
from __future__ import annotations

import importlib.util
import tomllib
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
DOCS = ROOT / "docs/re/windows-lbx-runtime"


def load_script(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


weight_data = load_script(DOCS / "analyze-qr-n2-weights.py", "n2_sweep")
scorer = load_script(DOCS / "analyze-qr-mask-margins.py", "margin_scorer")
PROBES = weight_data.PROBES


def candidates(data: str, mode: str, version: int):
    result = []
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
        n1, n2, n3, current_n4 = scorer.matrix_components(matrix)
        dark = sum(map(sum, matrix))
        area = len(matrix) ** 2
        floor_percent = dark * 100 // area
        ceil_percent = (dark * 100 + area - 1) // area
        qrcodegen_k = max(0, (abs(dark * 20 - area * 10) + area - 1) // area - 1)
        n4_options = {
            "brother_round_percent": current_n4,
            "floor_percent": abs(floor_percent - 50) // 5 * 10,
            "ceil_percent": abs(ceil_percent - 50) // 5 * 10,
            "qrcodegen_exact_ratio": qrcodegen_k * 10,
        }
        result.append((n1, n2, n3, n4_options))
    return result


def winner(scores):
    return min(range(8), key=lambda mask: scores[mask])


def matches(score_sets, observed):
    return sum(
        winner(scores) == expected
        for scores, (_, expected, _) in zip(score_sets, observed)
    )


def main() -> None:
    observed = []
    for name, filename, mode, version, brother_mask in PROBES:
        data = tomllib.loads((DOCS / filename).read_text())["elements"][0]["content"]["data"]
        observed.append((name, brother_mask, candidates(data, mode, version)))
    print(f"controls={len(observed)}")

    for weight in range(1, 9):
        scores = [
            [n1 * weight + n2 + n3 + n4["brother_round_percent"] for n1, n2, n3, n4 in candidates]
            for _, _, candidates in observed
        ]
        count = matches(scores, observed)
        residual = " ".join(
            f"{name}=mask{winner(score)}"
            for (name, _, _), score in zip(observed, scores)
            if name in {"cycle37-55", "cycle38-upper-lower-upper"}
        )
        print(f"N1_weight={weight}: matched={count}/19; {residual}")

    for variant in (
        "brother_round_percent",
        "floor_percent",
        "ceil_percent",
        "qrcodegen_exact_ratio",
    ):
        scores = [
            [n1 + n2 + n3 + n4[variant] for n1, n2, n3, n4 in candidates]
            for _, _, candidates in observed
        ]
        count = matches(scores, observed)
        residual = " ".join(
            f"{name}=mask{winner(score)}"
            for (name, _, _), score in zip(observed, scores)
            if name in {"cycle37-55", "cycle38-upper-lower-upper"}
        )
        print(f"N4={variant}: matched={count}/19; {residual}")


if __name__ == "__main__":
    main()
