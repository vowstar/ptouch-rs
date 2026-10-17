"""Sweep integer N2 weights against 19 captured Brother QR masks."""
from __future__ import annotations

import importlib.util
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
DOCS = ROOT / "docs/re/windows-lbx-runtime"
SCORER_PATH = DOCS / "analyze-qr-mask-margins.py"
spec = importlib.util.spec_from_file_location("qr_mask_margins", SCORER_PATH)
scorer = importlib.util.module_from_spec(spec)
assert spec.loader is not None
spec.loader.exec_module(scorer)

PROBES = [
    ("cycle40-02", "qr-mask-score-probe-02.ptl", "auto", 4, 3),
    ("cycle40-14", "qr-mask-score-probe-14.ptl", "auto", 4, 6),
    ("cycle40-16", "qr-mask-score-probe-16.ptl", "auto", 4, 2),
    ("cycle40-20", "qr-mask-score-probe-20.ptl", "auto", 4, 7),
    ("cycle40-23", "qr-mask-score-probe-23.ptl", "auto", 4, 6),
    ("cycle40-29", "qr-mask-score-probe-29.ptl", "auto", 4, 6),
    ("cycle40-30", "qr-mask-score-probe-30.ptl", "auto", 4, 0),
    ("cycle37-50", "qr-mask-probe-50.ptl", "auto", 4, 1),
    ("cycle37-55", "qr-mask-probe-55.ptl", "auto", 4, 1),
    ("cycle37-62", "qr-mask-probe-62.ptl", "auto", 4, 1),
    (
        "cycle37-diverse",
        "qr-mask-probe-diverse.ptl",
        "manual:byte:0-52,numeric:52-60",
        4,
        2,
    ),
    ("cycle35-lowercase60", "qr-v4-byte.ptl", "auto", 4, 1),
    (
        "cycle38-lower-to-numeric",
        "qr-segment-probe-lower_to_numeric.ptl",
        "manual:byte:0-6,numeric:6-16",
        1,
        0,
    ),
    (
        "cycle38-numeric-to-lower",
        "qr-segment-probe-numeric_to_lower.ptl",
        "manual:numeric:0-10,byte:10-15",
        1,
        3,
    ),
    (
        "cycle38-upper-mixed",
        "qr-segment-probe-upper_mixed.ptl",
        "manual:alphanumeric:0-12,numeric:12-22",
        1,
        3,
    ),
    (
        "cycle38-interleaved",
        "qr-segment-probe-interleaved.ptl",
        "manual:byte:0-3,alphanumeric:3-12",
        1,
        3,
    ),
    (
        "cycle38-upper-lower-upper",
        "qr-segment-probe-upper_lower_upper.ptl",
        "manual:alphanumeric:0-6,byte:6-12",
        1,
        0,
    ),
    ("numeric-v1", "qr-v1-numeric.ptl", "auto", 1, 4),
    ("url-v3", "qr-probe.ptl", "auto", 3, 7),
]


def main() -> None:
    observations = []
    for name, filename, mode, version, brother_mask in PROBES:
        layout = tomllib.loads((DOCS / filename).read_text())
        data = layout["elements"][0]["content"]["data"]
        components = scorer.score_candidates(data, mode, version, border=0)
        observations.append((name, brother_mask, components))

    print(f"controls={len(observations)}")
    for weight in range(1, 9):
        matches = 0
        residuals = []
        for name, brother_mask, components in observations:
            totals = [n1 + weight * n2 + n3 + n4 for n1, n2, n3, n4 in components]
            winner = min(range(8), key=totals.__getitem__)
            matches += winner == brother_mask
            if name in {"cycle38-upper-lower-upper", "cycle37-55"}:
                residuals.append(f"{name}=mask{winner}")
        print(f"N2_weight={weight}: matched={matches}/19; {' '.join(residuals)}")


if __name__ == "__main__":
    main()
