# Evidence ledger

Distinguish observed facts (from artifacts), inferences, and unknowns. Never treat static reachability as runtime proof. Update with new evidence IDs or artifact hashes as analysis proceeds.

## Upstream base (vowstar/ptouch-rs @ 6b07748, v0.10.0)

| Claim | Status | Evidence |
|---|---|---|
| Unmodified workspace builds and passes tests | observed | Local `cargo test --workspace` rerun 2026-10-16, exit 0; 334 tests passed across workspace targets |
| PT-E550W in device table (PID 0x2060, 128px, 180dpi) but flagged `UNSUP_RASTER` | observed | `crates/ptouch-core/src/device.rs` (upstream) |
| PT-P900W absent from device table | observed | same file; required PID 0x2085 known from LGPL reference |
| No TCP/network transport; USB (rusb) + Windows usbprint + macOS BT only | observed | `crates/ptouch-core/src/transport.rs`, `session.rs` (upstream) |
| Editor, offline PNG/JPEG/... export, `.ptl` layouts, GUI smoke test exist | observed | upstream README + crates; baseline tests pass |

## Reference implementation (nbuchwitz/ptouch, LGPL-2.1-or-later)

| Claim | Status | Evidence |
|---|---|---|
| PTP900W: PID 0x2085, 560 pins, 70 B/line, 360/720 dpi, no compression default | observed | `src/ptouch/printers.py` (repo main) |
| PTE550W: PID 0x2060, 128 pins, 16 B/line, 180/360 dpi, compression default on; high-res duplicates lines + doubled margin; cut tied to compression | observed | same |
| Tape pin configs (left/print/right) for both models incl. heat-shrink 2:1 and 3:1 | observed | same |
| Raster page structure: init(100x 0x00 + ESC @), `ESC i a 01` raster mode, `ESC i z` flags 0x86 + media + width + lines LE u32, `ESC i M` (bit6 autocut, bit7 mirror), `ESC i K` (bit2 halfcut, bit3 no-chain, bit4 special-tape no-cut, bit6 hires), `ESC i d` margin LE u16, `4M 00/02` compression, `47`+len+data lines / `5A` zero line, `0C` print / `1A` print+feed | observed | `src/ptouch/printer.py` |
| TCP 9100 raw transport, TCP_NODELAY, 5s timeout, timeout-only retries (3x, 100/200ms backoff); USB write-only, partial-write continuation | observed | `src/ptouch/connection.py` |

## ptset (preserved original, commit f1055fc)

| Claim | Status | Evidence |
|---|---|---|
| Windows usbprint transport mirrors ptfilo.dll (SetupDi + CreateFileW) | observed | `reference/ptset-original/src/usb.rs` |
| 32-byte status reply parse + captured PT-P900W fixture (series 0x30, model 0x6F, 24mm laminated) | observed | `reference/ptset-original/src/status.rs` + test |
| 5 settings recovered from PT900 brdvset.exe ISettingItem vtables (slots 11-14): auto-power-off-ac/li, auto-power-on, command-mode, save-print-log | observed | `reference/ptset-original/src/settings.rs` |
| SET commands not yet verified on hardware; value scales for auto-off/log unmapped | unknown | original project memory note |
| brdvset.exe Ghidra decompile + ptfilo.dll decompile preserved | observed | `docs/re/ptset/out/*.c` |

## Windows packages (ryzen, hashes in artifact-manifest.sha256)

| Claim | Status | Evidence |
|---|---|---|
| P-touch Editor 6.12.10: native x86 PE launcher + .NET 8 WPF main assembly (1183 types, 9504 methods) | observed | REA `inspect_managed_artifact` ev_e407c85c (PtouchEditor6.Wpf.dll, .NETCoreApp 8.0) |
| Editor references LBX stack: LibLbx.DotNet (mixed C++/CLI, 2493 types), LbxFile, LbxViewBase.Wpf, lbxcore.Win32.dll, liblbx.win32.dll, plus printsdk.win32.dll, ptfilontm.dll, ptfilot.dll, ptmdamm.dll, ptmdanet.dll | observed | assembly reference list ev_e407c85c + install tree listing |
| Printer Setting Tool 1.6.0142: managed DeviceSettingsTool.Command.dll (.NET Standard 2.0) + PtFiloSharp.dll P/Invoking `..\..\ptfilo.dll` exports PTWriteFile/PTReadFile | observed | ev_907e0d44, ev_429bef3c, ev_d8db2b02 |
| DeviceSettings model modules shipped: common, PJ700, PT800, PT900, PTP710BT, QL800QL1100, RJ2000, RJ4200, RJ4235_4255, TD2030A_2125_2135 | observed | directory listing |
| PT900 module maps PT-P900 (0x71) / PT-P900W (0x6F) / PT-P950NW (0x70), series 0x30 | observed | `DeviceSettings/PT900/brdvset.ini` |
| PT800 module covers PT-E800T/E800TK/E850TKW/D800W/E800W only | observed | `DeviceSettings/PT800/brdvset.ini` |
| PT-E550W has no DeviceSettings module in this tool version | observed (negative) | module list above; E550W settings path remains an open question |

## Android apps (hashes in android-artifact-manifest.sha256)

| Claim | Status | Evidence |
|---|---|---|
| iPrint&Label 5.3.17 (versionCode 161, com.brother.ptouch.iprintandlabel) bundle downloaded from APKMirror official listing; base.apk signed by genuine `CN=Brother Industries, Ltd` cert (sha256 3b53f53b91b5d0535aa3d6a65bc1426986522ac9de28d871572dd5117f7d4d14) | observed | APK v2 signing block parse + info.json |
| Design&Print 2 2.8.1(624) (com.brother.ptouch.designandprint2) base.apk signature sha256 c5cf91978f4323eeec223536027272caffd5c63db8e846021d0b98179083f56b matches APKMirror's verified signature record for this app (generic `CN=Android` subject is the app's actual release key, not tampering) | observed | APK v2 block parse vs release-page record |
| Mobile feature/network/LBX behavior | unknown | jadx decompile in progress |

## Preservation verification (2026-10-16)

| Claim | Status | Evidence |
|---|---|---|
| Preserved original `ptset` source, Ghidra scratchpad, Printer Setting Tool, and P-touch Editor artifacts match the tracked SHA-256 manifest | observed | `sha256sum -c docs/re/artifact-manifest.sha256` from repository root; all 1,141 entries passed |
| Preserved Android APK/APKM artifacts match their SHA-256 manifest | observed | `sha256sum -c docs/re/android-artifact-manifest.sha256` from `reference/artifacts/`; all 4 entries passed |
