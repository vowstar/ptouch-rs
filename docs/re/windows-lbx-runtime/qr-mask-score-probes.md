# QR mask score probes

Cycle 40 tested whether Brother's version 4/ECC M mask choices can be explained
by omitting or reducing qrcodegen's N3 finder-pattern penalty. Seven lowercase
ASCII strings were rendered through P-touch Editor's in-memory P900W runtime.
Each produces one byte segment, avoiding mixed-mode segmentation as a variable.
The runtime decoder reports version 4 and ECC M for every capture.

| Probe | Brother mask | Standard score winner | Winner without N3 | N1 | N2 | N3 |
|---|---:|---:|---:|---:|---:|---:|
| 02 | 3 | 2 | 3 | 329 | 270 | 840 |
| 14 | 6 | 2 | 3 | 360 | 312 | 800 |
| 16 | 2 | 2 | 6 | 327 | 273 | 800 |
| 20 | 7 | 2 | 7 | 277 | 267 | 960 |
| 23 | 6 | 6 | 2 | 331 | 327 | 720 |
| 29 | 6 | 6 | 2 | 317 | 312 | 760 |
| 30 | 0 | 2 | 3 | 357 | 330 | 800 |

N1, N2, and N3 are qrcodegen's run, 2x2 block, and finder-pattern penalties
for the Brother-selected mask. N4 was zero for each of these version 4 mask
candidates. The full captured module grid for every probe matches the Rust
fixed-mask candidate exactly, with zero mismatches across 69,696 raster
pixels. Re-run the pixel checks with:

```sh
uv run --with pillow python docs/re/windows-lbx-runtime/compare-qr-mask-score-probes.py
```

The observed masks split across the hypotheses: standard scoring predicts
three, omission of N3 predicts two, and neither predicts two. Probe 14 also
cannot be explained by any nonnegative scalar weight on N3 while holding N1
and N2 fixed. No production scoring change is justified by this sample.

The seven PTL sources and captured BMPs are preserved next to this report.
They can be replayed through the read-only LbxProbe.exe using
PT_QR_DATA_OVERRIDE. No printer was opened and no print job or printer
setting change was made.
