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
| Full Android runtime behavior, complete editor feature inventory, exact per-transport print execution, and parity with desktop | unknown | Static decompiles establish model maps and print branches below, but no Android runtime probe or complete feature audit has been performed |

### Android static code-path findings

| Claim | Status | Evidence |
|---|---|---|
| iPrint&Label 5.3.17 has explicit `PrinterModel` to `LBXPrinter` mappings for PT-E550W and PT-P900W | observed | `android-iprintandlabel-5.3.17/jadx/sources/com/brother/ptouch/iprintandlabel/utils/PrinterUtility.java:107-140`; `EditLabelActivity.java:1454-1458, 1678-1692` |
| iPrint&Label's per-model connectivity table assigns both targets infrastructure Wi-Fi, Wireless Direct, and USB; Bluetooth and wired flags are false | observed | `printerconnect/PrinterConnectSupportInfo.java:8,16,28-40`; corroborated by `device/BrotherDevicePresets.java:10-16` |
| iPrint&Label resolves stored LBX printer IDs into SDK models, changes the LBX printer for selected/connected models, and routes LBX, image, PDF, numbering, and database-assigned raster jobs to print helpers | observed | `edit/EditLabelActivity.java:1454-1458, 1650-1712, 2231-2253`; `print/PrintTask.java:61-92, 125-139, 143-200`; `print/PrinterControl.java:111-128, 169-181` |
| iPrint&Label chooses concrete print helper classes from the selected connection item: Wi-Fi, Wireless Direct, Bluetooth, or USB | observed | `print/PrinterControl.java:111-128`; each helper delegates the shared bitmap/file/PDF operations to `PrintHelperBase` driver methods |
| iPrint&Label's LBX compatibility table assigns E550W/P900W model IDs and PT7/PT9 paper maps; it explicitly reports 180 dpi for E550W and 360 dpi for P900W in this LBX tag-writing path | observed | `compatible/LBXTagWriter.java:256-303`; this does not establish all device print modes or raster geometry |
| iPrint&Label has model-specific printer settings code paths for E550W and P900W; exact exposed and writable settings remain to be enumerated | observed / unknown | `printersetting/PrinterSettingUtility.java:68-163` includes model switch cases; full setting semantics still under review |
| iPrint&Label creates LBX objects for text, symbol, image, barcode, date/time, frame, rectangle, and polygon; barcode protocol/data and image bytes/trim fields are applied in editor code | observed | `edit/EditLabelActivity.java:1156-1168, 1236-1287`; `edit/LabelView.java:718-745` |
| Design&Print 2's `PrinterModel` DEX enum contains exactly PT-N25BT, PT-P300BT, PT-P300BTz, PT-P320BT, PT-P710BT, PT-P715E, and PT-P910BT; neither target model is in this enum | observed | APK `base.apk` SHA-256 `cd22e32e2b594931a4365fe1fbb8a5581a93a386c007e83b2dd648b44196f0ee`; class in `classes2.dex`; JADX 1.5.6 single-class fallback output. Enum member declaration recovered despite normal-mode enum restoration warning. |

### Managed desktop inventories

| Claim | Status | Evidence |
|---|---|---|
| P-touch Editor's `LbxFile.dll` is x86 managed CIL, .NET 8, assembly version 1.12.1.0; complete metadata includes 9 types and 29 methods | observed | REA `inspect_managed_artifact` Evidence `ev_98456da9293460b08ecdfa60ff46cb4e2639555ebefa27de25f7ee1f5536bc5b`; file SHA-256 `17bc62bf563c28ff9d9bed115194cfac18a8deea196912e7f76faa4478d82e6a` |
| P-touch Editor package also contains `LbxViewBase.Wpf.dll`, an x86 managed PE module; LBX view/file integration is present, but exact import/export feature behavior is not established by assembly identity alone | observed / unknown | REA `inspect_managed_artifact` Evidence `ev_eb21b5dad62779a101c564073415d3298a8da8a0d37ffc0caadc16448a75d883`; file SHA-256 `480bb7a0ac35d4ea5f22995d723de3498db03f0f49eb4e12e7ed9936367fc4ff` |
| Printer Setting Tool `DeviceSettingsTool.Command.dll` is x86 managed CIL, .NET Standard 2.0, with complete metadata of 219 types and 931 methods | observed | REA `inspect_managed_artifact` Evidence `ev_907e0d44aa9bb31fa7285edfb6eb1f97f4a1ffe78353003694bcd35c2d622e9f`; file SHA-256 `f9e0749b4c75dfd4ad4c29d910eca695f748b69188118d75c5cf21fe24c175f4` |
| P-touch Editor installation contains ten packaged CIV `.lbx` starter templates for cable, heat-shrink, patch-panel, faceplate, terminal-block and related identification labels | observed | `reference/artifacts/ptouch-editor-6.12.10/6/Resources/Templates/CIV/`; exact files and hashes in `docs/re/artifact-manifest.sha256` |
| P-touch Editor import/export workflows, barcode authoring, and model-specific print call paths were not established by the bounded managed-assembly/resource inventory | unknown | Assembly identity, references, member counts, and packaged templates are recorded above; no method-level capability evidence is asserted for these behaviors |

## Preservation verification (2026-10-16)

| Claim | Status | Evidence |
|---|---|---|
| Preserved original `ptset` source, Ghidra scratchpad, Printer Setting Tool, and P-touch Editor artifacts match the tracked SHA-256 manifest | observed | `sha256sum -c docs/re/artifact-manifest.sha256` from repository root; all 1,141 entries passed |
| Preserved Android APK/APKM artifacts match their SHA-256 manifest | observed | `sha256sum -c docs/re/android-artifact-manifest.sha256` from `reference/artifacts/`; all 4 entries passed |

## Cycle 3 model and TCP implementation

| Claim | Status | Evidence |
|---|---|---|
| Integrated device table now maps PT-E550W (`04f9:2060`) to 128 pixels at 180 dpi, with PackBits and ESC-i-a raster/info flags; PT-P900W (`04f9:2085`) maps to 560 pixels at 360 dpi, with ESC-i-a raster/info flags and no PackBits default | observed in reconstruction | `crates/ptouch-core/src/device.rs`; `device::tests::target_models_match_reference_raster_profiles`; source facts cross-checked against the `nbuchwitz/ptouch` reference above |
| `PtouchDevice::open_tcp(SocketAddr, pid)` uses an explicit model ID, a five-second connect deadline, TCP_NODELAY, bounded per-operation socket timeouts, and the shared `PrinterSession` | observed in reconstruction | `crates/ptouch-core/src/transport.rs`, `transport_tcp.rs`; loopback tests in `transport/tcp` |
| TCP loopback fixtures select PT-E550W/P900W by model ID and exercise raw request/reply bytes, fragmented reads, operation timeout, cancellation, and unknown-model rejection | observed in reconstruction | `cargo test --workspace`, `ptouch-core` TCP transport tests; no physical printer was used |
| TCP transport is core-library functionality; CLI/GUI TCP selection is not wired | known gap | `crates/ptouch-core/src/transport.rs`; CLI and GUI have no network-printer selection path |

## Cycle 4 high-resolution print mode

| Claim | Status | Evidence |
|---|---|---|
| P900W and E550W enable high-resolution mode through ESC `i K` bit 6, duplicate each input raster line, and double the raster count and feed margin | implemented and tested | `crates/ptouch-core/src/protocol.rs`; exact-byte unit goldens cover PackBits E550W and uncompressed P900W jobs |
| The default 2 mm margin is represented as 14 dots at 180 dpi and 28 dots at 360 dpi, then doubled in high-resolution mode | derived from the reference conversion and tested | `crates/ptouch-core/src/session.rs` derives base dots from the model DPI; `transport/virtual_printer_test.rs` verifies E550W's 28-dot and P900W's 56-dot wire values |
| Both target high-resolution streams complete through the TCP session handshake | observed in the loopback fixture | `target_high_resolution_jobs_match_golden_bytes_through_virtual_printer`; this is a virtual-printer test, not physical-device validation |
| Draft quality is supported on the modern P900W/E550W profiles | not implemented | `PrinterSession::print_raster` continues to reject Draft on these profiles; current reference evidence confirms high-resolution mode but does not establish target draft behavior |
| Standard target print jobs match exact USB-session and TCP wire-byte goldens | tested | `session::tests::target_standard_jobs_match_usb_session_golden_bytes`; `target_standard_jobs_match_golden_bytes_through_virtual_printer` |
| Raster packing preserves source pixels at both target head widths | tested | `ptouch-render::raster::tests::bitmap_raster_round_trip_preserves_pixels_at_target_head_widths` |
| Printer-reported print errors, pre-write cancellation, and connection teardown are covered without hardware | tested in virtual sessions | `printer_error_after_print_is_returned_and_transport_tears_down`; `cancellation_before_job_write_closes_without_sending_raster_bytes` |

## Cycle 5 CLI TCP printing

| Claim | Status | Evidence |
|---|---|---|
| CLI `info` and `print` accept `--tcp IP:PORT --pid HEX_PID`; status-capable models detect tape, while other TCP models require `--tape-width-mm WIDTH_CODE` | implemented and tested | `crates/ptouch-cli/src/main.rs`; `tcp_selection_requires_a_hex_product_id_for_info_and_print`; `tcp_cli_device_prints_to_the_selected_model` |
| CLI `info` succeeds for raw TCP models without a verified status route and reports status/media as unavailable | tested through a loopback endpoint | `tcp_info_reports_unavailable_e550w_status_without_failing`; the fixture verifies the command sends no unsupported raw status request |
| The CLI TCP path sends a complete PT-E550W print job and receives printer completion through a loopback endpoint | tested in a loopback fixture | `tcp_cli_device_prints_to_the_selected_model`; no physical printer was used |
| GUI TCP model/address selection reaches the asynchronous printer worker and completes a target print | implemented and tested through a loopback endpoint | `crates/ptouch-gui/src/panels/sidebar.rs`, `printer_worker.rs`; `tcp_target_polls_and_prints_without_blocking_the_ui_worker_contract` |

## Cycle 5 network discovery and media status

| Claim | Status | Evidence |
|---|---|---|
| GUI browses `_pdl-datastream._tcp.local.` and presents resolved IPv4 printer endpoints for selection; recognized model text maps to supported product IDs | implemented; loopback printer flow tested, discovery advertisements not exercised | `crates/ptouch-gui/src/printer_worker.rs`; `crates/ptouch-gui/src/panels/sidebar.rs`; service resolution requires valid DNS-SD records |
| Raw TCP initialization does not issue USB status requests; a tape width must be selected before raster printing when no network status route is available | implemented and tested | `crates/ptouch-core/src/session.rs`; `crates/ptouch-core/src/transport/virtual_printer_test.rs`; PT-E550W has no verified network media-status route |
| PT-P900W network status is queried using Brother SNMP OID `1.3.6.1.4.1.2435.3.3.9.1.6.1.0`; community defaults to `public` and can be overridden with `PTOUCH_SNMP_COMMUNITY` | implemented and parser/request-tested with a loopback UDP responder | `crates/ptouch-core/src/network_status.rs`; `network_status::tests::parses_p900w_media_status_from_the_brother_snmp_oid`; no physical printer was used |
| GUI locks PT-P900W tape width to the detected cassette, and enables advanced high-resolution mode only when status identifies laminated tape and black text; print refreshes and rechecks width/status before sending | implemented; parser, media gate, and print-session behavior covered separately | `crates/ptouch-gui/src/printer_worker.rs`, `state.rs`, `panels/sidebar.rs`; `crates/ptouch-core/src/session.rs`; no physical print validation |
| PT-E550W GUI requires a user-selected tape width; CLI printing uses `--tape-width-mm WIDTH_CODE`, and widths wider than 24 mm or code 21 are rejected for its 128-pixel head | implemented and tested | `crates/ptouch-gui/src/state.rs`; `crates/ptouch-gui/src/panels/sidebar.rs`; `crates/ptouch-cli/src/main.rs`; `session::tests::advanced_high_resolution_rejects_media_without_black_text_before_writing` and CLI loopback tests |
| When PT-P900W SNMP status is available but reports no cassette, the GUI blocks printing and does not offer the E550W manual fallback | implemented and tested | `crates/ptouch-gui/src/app.rs`; `app::tests::p900w_status_without_media_does_not_enable_manual_media_fallback` |
| USB status initialization remains enabled for models also supported over TCP | regression fixed and workspace-tested | `crates/ptouch-core/src/session.rs`; model-level status suppression was removed; `cargo test --workspace` passes |

## Cycle 5 verification (2026-10-17)

| Claim | Status | Evidence |
|---|---|---|
| Entire Rust workspace passes tests | observed | `cargo test --workspace`, exit 0; 367 tests passed across workspace targets |
| Entire Rust workspace passes Clippy with warnings denied | observed | `cargo clippy --workspace --all-targets --all-features -- -D warnings`, exit 0 |
| TCP discovery, status, and print behavior has been validated against physical Brother printers | unknown | Verification used loopback TCP/UDP fixtures only |

## Cycle 5 read-only Command Mode query

| Claim | Status | Evidence |
|---|---|---|
| `ptouch config get command-mode` sends the recovered PT-P900W GET bytes `1B 69 55 61 01` over an explicitly selected TCP connection | implemented and loopback-tested | `crates/ptouch-core/src/settings.rs`; `crates/ptouch-cli/src/main.rs`; no physical TCP query was performed |
| The 32-byte GET reply's byte 30 is decoded using the preserved values ESC/P (0x00), Raster (0x01), and P-touch Template (0x03); unknown values remain visible as raw hex | implemented from preserved reverse-engineering evidence | `reference/ptset-original/src/settings.rs`; `CommandMode` preserves unknown values |
| The query rejects models other than PT-P900W before sending a setting request | tested | `settings::tests::command_mode_query_rejects_non_p900w_models_before_sending` |
| Printer SET commands are implemented or transmitted by this query | no | This operation is read-only; recovered SET bytes remain unverified and are not used |
