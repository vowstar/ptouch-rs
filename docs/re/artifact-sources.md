# Preserved artifact sources

These files are for local reverse engineering only. Brother binaries, resources, and other proprietary assets are excluded from git and must not be redistributed.

| Artifact | Original source | Local analysis path |
|---|---|---|
| P-touch Editor 6.12.10 | `C:\Program Files (x86)\Brother\P-touch Editor\6\` on `ryzen` | `reference/artifacts/ptouch-editor-6.12.10/6/` |
| Printer Setting Tool 1.6.0142 | `C:\Program Files (x86)\Brother\LabelMobile Printer\Printer Setting Tool\` on `ryzen` | `reference/artifacts/printer-setting-tool-1.6.0142/Printer Setting Tool/` |
| `ptset` Ghidra scratchpad | `C:\Users\joshd\AppData\Local\Temp\claude\C--Users-joshd\329fa747-5034-491b-a55d-4cd85b900bea\scratchpad\re\` | `reference/artifacts/ptset-re-scratchpad/re/` |
| Original `ptset` source | `C:\Users\joshd\dev\ptset\` on `ryzen`, excluding `target/` | `reference/ptset-original/` |

`docs/re/artifact-manifest.sha256` records SHA-256 hashes for 1,141 preserved artifact files. Textual decompiles and analysis scripts are mirrored into `docs/re/ptset/` for tracked reference.

The upstream application baseline is `vowstar/ptouch-rs` commit `6b07748` (`0.10.0`). Its unmodified workspace test suite passed locally on 2026-10-16 before integration changes.
