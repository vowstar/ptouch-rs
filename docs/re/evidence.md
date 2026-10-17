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
| iPrint&Label barcode objects support CODE39, CODE128, EAN13, EAN128, and QR; properties include data, protocol, check digit, human-readable text, QR version, and ECC level | observed in source | `lbxwrapper/Barcode.java`; `edit/EditLabelActivity.java:1156-1168, 2152-2171`; QR payload rendering semantics remain delegated to the proprietary LBX/SDK implementation |
| iPrint&Label reads LBX archives by extracting `label.xml` and saves by serializing tags and re-zipping; the preserved CIV sample archives contain `label.xml` and `prop.xml` | observed in source and sample files | `compatible/LBXTagWriter.java:638-660, 756-793, 1329-1340`; ten CIV archives under `reference/artifacts/ptouch-editor-6.12.10/6/Resources/Templates/CIV/` |
| iPrint&Label's target print flow renders ordinary LBX jobs to RGB_565 PNG, sends numbering jobs as generated bitmaps, and sends database-assigned jobs from a bitmap job list | observed in source | `print/PrintTask.java:61-92, 112-139, 171-190`; database file schema and full runtime behavior remain unverified |
| iPrint&Label's PT-E550W media UI lists standard widths through 24 mm, SL 24 mm, and HS widths through 23.6 mm; PT-P900W uses a shared standard/SL/FL/HS list through 36 mm and defaults to 36 mm | observed app configuration; hardware behavior unknown | `printersetting/PrinterSettingUtility.java:343-376, 398-456`; `res/values/arrays.xml:432-498, 640-697`; do not infer every listed media type is physically validated on both targets |
| Design&Print 2's `PrinterModel` DEX enum contains exactly PT-N25BT, PT-P300BT, PT-P300BTz, PT-P320BT, PT-P710BT, PT-P715E, and PT-P910BT; neither target model is in this enum | observed | APK `base.apk` SHA-256 `cd22e32e2b594931a4365fe1fbb8a5581a93a386c007e83b2dd648b44196f0ee`; class in `classes2.dex`; JADX 1.5.6 single-class fallback output. Enum member declaration recovered despite normal-mode enum restoration warning. |

### Managed desktop inventories

| Claim | Status | Evidence |
|---|---|---|
| P-touch Editor's `LbxFile.dll` is x86 managed CIL, .NET 8, assembly version 1.12.1.0; complete metadata includes 9 types and 29 methods | observed | REA `inspect_managed_artifact` Evidence `ev_98456da9293460b08ecdfa60ff46cb4e2639555ebefa27de25f7ee1f5536bc5b`; file SHA-256 `17bc62bf563c28ff9d9bed115194cfac18a8deea196912e7f76faa4478d82e6a` |
| P-touch Editor package also contains `LbxViewBase.Wpf.dll`, an x86 managed PE module; LBX view/file integration is present, but exact import/export feature behavior is not established by assembly identity alone | observed / unknown | REA `inspect_managed_artifact` Evidence `ev_eb21b5dad62779a101c564073415d3298a8da8a0d37ffc0caadc16448a75d883`; file SHA-256 `480bb7a0ac35d4ea5f22995d723de3498db03f0f49eb4e12e7ed9936367fc4ff` |
| Printer Setting Tool `DeviceSettingsTool.Command.dll` is x86 managed CIL, .NET Standard 2.0, with complete metadata of 219 types and 931 methods | observed | REA `inspect_managed_artifact` Evidence `ev_907e0d44aa9bb31fa7285edfb6eb1f97f4a1ffe78353003694bcd35c2d622e9f`; file SHA-256 `f9e0749b4c75dfd4ad4c29d910eca695f748b69188118d75c5cf21fe24c175f4` |
| P-touch Editor installation contains ten packaged CIV `.lbx` starter templates for cable, heat-shrink, patch-panel, faceplate, terminal-block and related identification labels | observed | `reference/artifacts/ptouch-editor-6.12.10/6/Resources/Templates/CIV/`; exact files and hashes in `docs/re/artifact-manifest.sha256` |
| P-touch Editor import/export workflows, barcode authoring, and model-specific print call paths were not established by the bounded managed-assembly/resource inventory | unknown | Assembly identity, references, member counts, and packaged templates are recorded above; no method-level capability evidence is asserted for these behaviors |
| P-touch Editor `LbxFile.dll` metadata names `LoadLbxFilesAsync` and file properties for name, created date, path, thumbnail, media width, and media height | observed symbol metadata only | `LbxFile.dll` in the preserved P-touch Editor 6.12.10 package; names establish a file-library surface, not full authoring/import/export/print semantics |
| Preserved CIV LBX sample XML includes document/sheet/paper metadata and nested cable/template cell structures with text/font properties | observed in sample archives | ZIP/XML inspection of the ten exact CIV files listed in `docs/re/artifact-manifest.sha256`; feature behavior remains unknown |

## Confirmed Rust parity gaps from the inventory

| Gap | Current evidence | Scope limit |
|---|---|---|
| Rust reads/writes versioned TOML `.ptl`, imports a bounded LBX subset, and exports supported absolute text, raster-image, QR, and Code 39 objects to target-specific LBX archives through the CLI | `crates/ptouch-render/src/document.rs`, `crates/ptouch-render/src/lbx.rs`, `crates/ptouch-cli/src/main.rs`; target maps from `LBXTagWriter.java` above | Export rejects unsupported layouts; generated-file acceptance and full Brother LBX conversion fidelity remain unknown |
| Rust supports Code 39 but not Code 128, EAN13, or EAN128 objects | `crates/ptouch-render/src/barcode.rs`, `document.rs`, and `lbx.rs`; compare iPrint&Label `Barcode.java` and editor creation paths above | Code 39 supports its bounded alphabet and optional Mod 43 character; target-app visual rendering and print acceptance are not established |
| Rust implements placeholder CSV batches but does not match iPrint&Label LBX numbering or database-assigned bitmap job paths | `crates/ptouch-cli/src/main.rs`; iPrint&Label `PrintTask.java` and `PrinterOptionDialogFragment.java` above | iPrint&Label CSV/schema runtime behavior is only partly established |
| Current Rust TCP media handling covers standard width codes and does not model SL, FL, or HS media types | `crates/ptouch-core/src/tape.rs`, `session.rs`, `ptouch-gui/src/panels/sidebar.rs`; compare the target media configuration row above | App UI lists do not prove physical media compatibility or wire protocol behavior |

## Cycle 7 LBX import implementation

| Claim | Status | Evidence |
|---|---|---|
| `.lbx` files are ZIP archives with `label.xml`; this importer extracts that entry and parses the XML | implemented | `crates/ptouch-render/src/lbx.rs`; archive and XML validation errors are surfaced |
| LBX import bounds compressed archive size to 64 MiB and `label.xml` to 8 MiB | implemented | `crates/ptouch-render/src/lbx.rs`, checked in the shared path loader before allocating the archive bytes |
| Ordinary direct text objects import geometry, rotation, alignment, and nominal font size using points converted to millimeters | implemented, unit tested | `crates/ptouch-render/src/lbx.rs`; synthetic archive fixtures |
| Unsupported direct object kinds generate warnings; unsupported-only labels fail instead of opening blank | implemented, unit tested | `crates/ptouch-render/src/lbx.rs`; warning and rejection tests |
| CLI and GUI use the shared path-based `.ptl` / `.lbx` loader | implemented | `crates/ptouch-render/src/document.rs`, `crates/ptouch-cli/src/main.rs`, `crates/ptouch-gui/src/panels/toolbar.rs`, `crates/ptouch-gui/src/widgets.rs` |
| LBX paper orientation, margins, trimmed/transparency/effect image behavior, specialized nested objects, XML export, and visual parity | unknown or unsupported | No implementation or physical application comparison in this cycle |

## Cycle 8 embedded image import

| Claim | Status | Evidence |
|---|---|---|
| iPrint&Label LBX image objects use an `image:imageStyle` filename resource and include trimming and source-position metadata | observed in source | `compatible/LBXTagWriter.java:111-114, 447-480, 513-517, 756-793`; image object creation in `edit/EditLabelActivity.java:1236-1247` |
| Rust imports the named embedded resource for an untrimmed, default-effect image as a decoded physical image object | implemented, unit tested | `crates/ptouch-render/src/lbx.rs`; tests build an LBX archive with an embedded PNG, load it through `LabelDocument::from_path`, and verify rendered ink |
| Trimmed images, non-default effects, and explicit transparency are reported as skipped; missing assets are reported in the unsupported-only error | implemented | `crates/ptouch-render/src/lbx.rs`; geometry and image rendering stay within the existing `LabelObject` model |
| Image resource reads are capped at 32 MiB | implemented | `crates/ptouch-render/src/lbx.rs` |
| Preserved self-laminating CIV template is refused with the unsupported `cable` object named in the error | verified offline through CLI | `ptouch print --layout reference/.../CIV/1_NewSelfLaminating.lbx`; no printer connection was made |
| Image crop fidelity, transparency, symbols and shape objects, nested templates, and LBX export | open gaps | Image schema evidence above; no preserved non-specialized image LBX fixture or Brother renderer comparison is available |

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


## Cycle 9 LBX vector outlines and nested wrappers

| Claim | Status | Evidence |
|---|---|---|
| iPrint&Label creates LINE/POLYLINE/POLYGON and RECTANGLE/ROUNDRECTANGLE/ELLIPSE objects; its internal line widths include thin, normal, and thick | observed in source | `reference/artifacts/android-iprintandlabel-5.3.17/jadx/sources/com/brother/ptouch/iprintandlabel/edit/EditLabelActivity.java:1242-1270`, and the adjacent `lbxwrapper/PolyStyle.java`, `RectStyle.java`, and `WidthType.java` |
| A preserved P-touch Editor cable-flag CIV template stores a line under nested cable/table/cell wrappers, with a dashed 0.5 pt pen and landscape paper margins | observed in packaged fixture | `reference/artifacts/ptouch-editor-6.12.10/6/Resources/Templates/CIV/3_NewCableFlag.lbx` |
| Rust imports unfilled line, polyline, polygon, rectangle, and ellipse outlines as SVG-backed image objects; nested supported children flatten in source order with warnings | implemented, unit tested | `crates/ptouch-render/src/lbx.rs`; tests cover visible rendered pixels and nested text/line order |
| Landscape left/top margins are swapped into the printable x/y origin | implemented, unit tested | `crates/ptouch-render/src/lbx.rs`; `landscape_page_margins_are_swapped_into_printable_origin` |
| The preserved cable-flag LBX renders offline through the CLI | verified | Offline `ptouch print --layout .../CIV/3_NewCableFlag.lbx --output /tmp/ptouch-cycle9-cable-flag.png`; output inspected, no source-application visual comparison or printer connection |
| Native shape editing, symbols, fills, round rectangles, unsupported pens/arrows, exact Brother styling, LBX export, and source-app rendering parity | open gaps | Conversion intentionally uses embedded SVG-backed image objects; unsupported variants warn or skip |


## Cycle 10 PT Dingbats symbol import

| Claim | Status | Evidence |
|---|---|---|
| iPrint&Label stores symbol family and code in `draw:symbolStyle`; PT Dingbats code values map to glyphs in Unicode private-use range starting at U+F000 | observed in source | `android-iprintandlabel-5.3.17/jadx/sources/com/brother/ptouch/iprintandlabel/compatible/LBXTagWriter.java:1252-1278`, `com/brother/ptouch/lbxwrapper/Symbol.java:46-50`, `com/brother/pns/labeleditorview/labelobject/SymbolUtility.java:18-25` |
| Preserved iPrint&Label Android assets include PT Dingbats 1-4 font families with private-use glyph coverage; P-touch Editor package does not expose identically named files in its Fonts directory | observed in package | Android `jadx/resources/assets/fonts/BRDB1RT0.TTF` through `BRDB4RT0.TTF`, verified with `fc-query`; P-touch Editor `6/Fonts/` inventory |
| Rust imports PT Dingbats symbols as centered editable text with an inline font family, frame, and rotation; warns that native symbol metadata is lost and when the family is unavailable | implemented, unit tested | `crates/ptouch-render/src/lbx.rs`; synthetic LBX fixture verifies code 65 maps to U+F041, preserves its inline font through `.ptl` serialization, and renders visible pixels when run with the preserved Brother font in a temporary fontconfig directory |
| Rendering a symbol exactly matches Brother's native renderer on the target platforms | unknown | Symbol geometry is normalized by iPrint&Label's proprietary LBX core; no preserved symbol-bearing LBX fixture or app-rendered comparison is available |


## Cycle 11 QR import

| Claim | Status | Evidence |
|---|---|---|
| iPrint&Label exposes CODE39, CODE128, EAN13, EAN128, and QR barcode objects with data and protocol fields; one-dimensional objects also expose check-digit and human-readable settings | observed in source | `android-iprintandlabel-5.3.17/jadx/sources/com/brother/ptouch/lbxwrapper/Barcode.java`; `compatible/LBXTagWriter.java:661-730`; editor construction and validation in `edit/EditLabelActivity.java:1156-1168, 2152-2171` |
| Preserved iPrint&Label `base.apk` contains no `.so` native libraries or `.lbx` files; the ten preserved P-touch Editor CIV LBX templates contain no barcode objects | verified artifact inventory | APK SHA-256 `cd22e32e2b594931a4365fe1fbb8a5581a93a386c007e83b2dd648b44196f0ee`; ZIP entry inventory; `reference/artifacts/ptouch-editor-6.12.10/6/Resources/Templates/CIV/` ZIP/XML scan |
| A P-touch Editor 5.0.201-generated sample establishes generic LBX QR tags: `barcode:barcode`, `barcodeStyle protocol="QRCODE"`, `qrcodeStyle` with model/ECC/cell-size/MBCS/version fields, and `pt:data` | observed in external sample | [Alecto3-D sample `label.xml`](https://github.com/Alecto3-D/brother-p-touch-editor-format/blob/master/label.xml); metadata identifies P-touch Editor 5.0.201 Macintosh and QL-700, so this establishes a generic schema only |
| The preserved target-app inventories contain no local barcode fixture | verified artifact inventory | APK SHA-256 `cd22e32e2b594931a4365fe1fbb8a5581a93a386c007e83b2dd648b44196f0ee`; ZIP entry inventory; ten P-touch Editor CIV files scanned under `reference/artifacts/ptouch-editor-6.12.10/6/Resources/Templates/CIV/` |
| Rust imports ASCII Model 2 QR objects with supported ECC/version settings as SVG-backed images | implemented, unit tested | `crates/ptouch-render/src/lbx.rs`; importer test checks generated vector content and rendered pixels; manual CLI output decoded to `http://alecto.ca` with `zbarimg` |
| Native barcode editing, other barcode protocols, source MBCS conversion for non-ASCII payloads, and exact Brother rendering on PT-P900W/PT-E550W | open gaps | Import preserves QR appearance through generated SVG only; sample targets QL-700, and no source-app render comparison or physical print was performed |


## Cycle 12 numeric placeholder batches

| Claim | Status | Evidence |
|---|---|---|
| iPrint&Label numbering configuration includes count, start-from, and increment; selected text ranges are marked as numbering-enabled | observed in source | `reference/artifacts/android-iprintandlabel-5.3.17/jadx/sources/com/brother/ptouch/iprintandlabel/edit/EditLabelActivity.java:setTextNumber`; `object/PrintParameter.java` |
| iPrint&Label creates one rendered LBX bitmap per sequence value, restores the source number after rendering each bitmap, then repeats the sequence for configured copies | observed in source | `reference/artifacts/android-iprintandlabel-5.3.17/jadx/sources/com/brother/ptouch/iprintandlabel/print/PrintTask.java:lambda$printNumberingLbx$0,printNumberingLbx` |
| Rust CLI creates numbered layout output by replacing a declared placeholder with start/increment/count values and routes labels through existing batch output/printing | implemented, tested | `crates/ptouch-cli/src/main.rs`; regression compares all three exported images at values 7, 9, and 11 against independently rendered layouts |
| Rust reproduces iPrint&Label selected-character numbering inside native LBX text objects or its database-assigned jobs | open gaps | Current numbering is an explicit CLI placeholder workflow. Native LBX character-range encoding and database file/job schemas are not implemented |


## Cycle 13 GUI template values

| Claim | Status | Evidence |
|---|---|---|
| GUI exposes each declared layout placeholder in a Template values section and applies values to preview and print rendering only when all placeholders have values | implemented, tested | `crates/ptouch-gui/src/panels/sidebar.rs`, `state.rs`, and `app.rs`; missing values keep the original tokens visible and disable Print |
| Transient GUI values do not alter the saved layout document and are cleared when another layout is opened | implemented, tested | `AppState::preview_document` applies values to a clone; `toolbar::apply_layout` clears the transient map; regression test compares serialized source before and after preview resolution |
| GUI supports CSV or multi-label numbered placeholder printing | no | GUI now supports single-label placeholder entry; CSV and numeric sequence batches remain CLI workflows |


## Cycle 14 selected-character numbering evidence

| Claim | Status | Evidence |
|---|---|---|
| iPrint&Label marks selected text ranges for numbering and stores count, start-from, and increment print parameters | observed in source | `reference/artifacts/android-iprintandlabel-5.3.17/jadx/sources/com/brother/ptouch/iprintandlabel/edit/EditLabelActivity.java:setTextNumber`; `module/numbering/NumberingAddEntity.java`; `object/PrintParameter.java` |
| iPrint&Label generates each selected-range value, renders one LBX bitmap per value, restores the source value, and applies copies to the sequence | observed in source | `serialize/Counter.java:countUp`; `print/PrintTask.java:lambda$printNumberingLbx$0,printNumberingLbx` |
| Recovered Counter accepts ASCII letters and digits and uses Java string offsets for selected ranges | observed in source | `serialize/Counter.java:enableCountUpRegex,countUp`; Java/Kotlin string range operations use UTF-16 code-unit indices |
| Rust matches native selected-character numbering, including Unicode selection and carry behavior | open gap | No attached Android device/emulator, app-produced numbering LBX, or rendered baseline was available. The decompiled carry implementation has not been independently validated, so no native numbering semantics were inferred from it |


## Cycle 15 bounded LBX export

| Claim | Status | Evidence |
|---|---|---|
| P-touch Editor LBX templates use `label.xml` and `prop.xml` entries and the Brother `pt`, `style`, `text`, and `image` namespace families | observed in preserved app package | Ten CIV archives under `reference/artifacts/ptouch-editor-6.12.10/6/Resources/Templates/CIV/`; `10_NewTerminalBlock.lbx` and `1_NewSelfLaminating.lbx` contain both entries and establish namespaced document, sheet, paper, and object structures |
| iPrint&Label maps PT-E550W to printer ID `26160` and PT-P900W to `28464`; their LBX paths use PT7/PT9 media maps and 180/360 dpi respectively | observed in source | `reference/artifacts/android-iprintandlabel-5.3.17/jadx/sources/com/brother/ptouch/iprintandlabel/compatible/LBXTagWriter.java:180-184, 263-268, 342-375` |
| Rust CLI creates target-specific LBX ZIPs containing `label.xml`, `prop.xml`, text, embedded raster image resources, and native QR barcode objects; supported text/image objects round-trip through the Rust LBX importer | implemented, tested | `crates/ptouch-render/src/lbx.rs::export`; `crates/ptouch-cli/src/main.rs::execute_convert`; export tests verify target media IDs, escaped text, geometry, image resources, QR XML, explicit rejection, and CLI convert/reimport behavior |
| Generated Rust LBX archives render equivalently in P-touch Editor or iPrint&Label, or print correctly on target devices | unknown | Cycle 19 returned target metadata through `LbxFactory.CreateFromPath`; no GUI rendering comparison or physical print was performed |
| LBX export preserves unsupported layout semantics such as inline styles, relative positions, mirroring, and native template wrappers | not implemented | Export returns explicit errors for these structures rather than silently dropping them |

## Cycle 16 Windows LBX runtime verification

| Claim | Status | Evidence |
|---|---|---|
| P-touch Editor startup initializes its LBX runtime with its app version, locale ID, and Fonts directory before creating or loading labels | observed in managed code | `PtouchEditor6.Wpf.App.CreateShell`; `docs/re/windows-lbx-runtime/Program.cs` reproduces the call using `AssemblyInfoService.AppNameVersionOS` and locale ID 1033 |
| The bundled `LibLbx.DotNet.dll` is x86 mixed C++/CLI version 1.12.1.1 with SHA-256 `7b892c240e7deff849895a987a449ebfeaefdc3a94a788a459c32a50dbf12539`; the tested Windows installation is P-touch Editor 6.12.10 | observed | Installed file hash matched the preserved artifact; the probe initialized with `P-touch Editor 6.12.1.0 Windows` and returned `Initialize=True` |
| Rust-generated LBX archives from a one-text-object layout load in the Brother runtime and return the intended printer model and stored printer ID for both targets | confirmed by Cycle 19 retest | Two runs on Windows host `ryzen` loaded the hash-verified archived files and returned PT-E550W ID `26160` and PT-P900W ID `28464`, media ID `261`. The Cycle 17 fallback report is contradicted; the exact cause is not recoverable from preserved logs |
| Brother GUI rendering, text appearance, image behavior, and physical print acceptance of generated archives match the Rust output | unknown | The runtime probe loaded metadata only; it did not inspect the GUI canvas or send a print job |

## Cycle 17 native QR authoring

| Claim | Status | Evidence |
|---|---|---|
| `.ptl` supports an editable QR object with payload and Low/Medium/Quartile/High error correction levels | implemented, tested | `crates/ptouch-render/src/document.rs`; version 7 preserves QR fields; `document::tests::native_qr_serializes_as_version_7_and_preserves_editable_data` |
| Shared rendering generates bounded Model 2 QR bitmaps with a four-module quiet zone for preview and print | implemented, tested | `crates/ptouch-render/src/barcode.rs`; `layout::render_object`; barcode unit tests cover quiet zone and invalid/capacity-overflow payloads. CLI offline render of `qr-probe.ptl` decoded back to `https://example.test/ptouch-cycle17` with `zbarimg` |
| GUI creates and edits QR payloads and correction level | implemented | `crates/ptouch-gui/src/panels/toolbar.rs`, `state.rs`, and `panels/properties.rs`; full GUI interaction is not automated |
| CLI LBX export retains QR payload as a native `barcode:barcode` object using generic P-touch Editor schema fields | implemented, tested | `crates/ptouch-render/src/lbx.rs::export`; `lbx::tests::exports_editable_qr_as_native_lbx_barcode`; schema evidence is the external P-touch Editor 5.0.201 QL-700 sample cited in Cycle 11 |
| P-touch Editor 6.12.10's LBX factory returns target model metadata for generated QR archives | confirmed by Cycle 19 retest | Two hash-verified runs returned PT-E550W ID `26160` and PT-P900W ID `28464`, media ID `261` for the archived QR exports. The probe does not read the QR object's payload from the loaded document or test rendering |
| GUI rendering and physical print output match Brother QR rendering on PT-E550W/PT-P900W | partial evidence | Cycles 33-34 matched visible E550W modules and the full P900W version 3 ECC M mask 7 in-memory raster to Rust; GUI canvas and physical print remain unverified |

## Cycle 19 Windows runtime retest

| Claim | Status | Evidence |
|---|---|---|
| The archived text and QR LBX files return their intended target model metadata through P-touch Editor's `LbxFactory.CreateFromPath` | confirmed, repeated | On `ryzen`, `LbxProbe.exe` called `LibLbx.Initialize` successfully, then loaded all four archived files twice. SHA-256 values match the table in `docs/re/windows-lbx-runtime/README.md`; E550W files returned ID `26160`, P900W files returned ID `28464`, all returned media ID `261`, LBX version `1.8`, and generator `ptouch-rs` |
| Cycle 18 LBX import/export preserves the archived QR export byte-for-byte | confirmed | `ptouch convert` imported and re-exported both target QR archives. Each output SHA-256 exactly matched its input (`4b4b8e2d...9ca708` for E550W; `2230fb5b...10fbd1` for P900W), and the Windows runtime returned the intended model metadata for both output files |
| The Cycle 17 report of MW-170, stored ID `0`, and media ID `138` can be reproduced for the same archived files | not reproduced; cause unknown | Two Cycle 19 probe runs using the exact archived hashes returned intended target metadata. The original Cycle 17 invocation details sufficient to identify the discrepancy were not preserved |

## Cycle 21 Windows loaded-object and render inspection

| Claim | Status | Evidence |
|---|---|---|
| P-touch Editor's `GetPtObjects()` returns a native Barcode object for generated QR and Code 39 LBX archives | observed | `docs/re/windows-lbx-runtime/Program.cs`; the exact archived QR files returned `LibLbx.BcdObj` / `ClassName=Barcode`, and the generated Code 39 archive returned the same object class |
| The Brother runtime reads the generated QR payload, Model 2, version 3 auto mode, MBCS 932, and Medium ECC | observed in object property | `GetBarcodeProperty()` returned the exact QR URL and `QrcodeData` fields `Model=Model2`, `Version=3`, `VerAuto=True`, `MBCS=932`, `EccLevel=M` |
| The Brother runtime reads the generated Code 39 payload and display/check-digit options | observed in object property | `GetBarcodeProperty()` returned `Data=ABC-123`, `ShowCheckDigit=True`, `ShowChar=True`, and `CharAlign=Center` for SHA-256 `0cc382b3150075152e3c29ff4eab4fbfe99327a146fb94518440e5c1e3c40214` |
| The in-memory Brother Code 39 raster is scannable and agrees with the Rust payload/check character | observed | `GetPrintingImage()` returned 482x128 BMP; `zbarimg` decoded `ABC-123W`. The Brother raster includes visible `*` start/stop delimiters and no external quiet zones. Cycle 22 aligned Rust human-readable text to include those delimiters; raster geometry remains incomparable because Brother returns object-only output while Rust currently captures the full label |
| Brother and Rust QR rasters looked different at native size | historical observation, resolved for visible pixels | `GetPrintingImage()` returned 132x128 BMP; Cycle 33 aligned its visible module pixels to the exact Rust version 3 ECC M mask 7 matrix with zero mismatches. The Brother raster clips rows at the top in a fixed 128 px capture |
| `GetPrintingImage()` output proves physical print output or the P-touch Editor GUI canvas is equivalent | unknown | The API returned an in-memory BMP without opening a printer handle. No physical print or GUI canvas inspection was performed |

## Cycle 20 Code 39 support

| Claim | Status | Evidence |
|---|---|---|
| `.ptl` represents native Code 39 data, Mod 43 option, narrow-bar width, and readable-text option | implemented, tested | `crates/ptouch-render/src/document.rs`, `barcode.rs`; layout format version 9; serialization tests preserve all fields |
| The offline Code 39 renderer encodes start/stop characters, optional Mod 43 check character, bounded payloads, and readable text | implemented, tested | `barcode::tests::code39_adds_start_stop_and_mod43_check_character`; CLI rendered `docs/re/code39-probe.ptl` and `zbarimg` decoded `ABC-123W` |
| Supported LBX Code 39 objects import to native `.ptl` objects and export with data, check digit, bar width, and readable-text fields | implemented, tested | `lbx::tests::imports_supported_code39_as_an_editable_native_object`; `lbx::tests::exports_editable_code39_as_a_native_lbx_barcode` |
| P-touch Editor's LBX runtime returns intended E550W metadata for a Rust-exported Code 39 archive | observed, metadata only | Windows `LbxProbe.exe` loaded SHA-256 `0cc382b3150075152e3c29ff4eab4fbfe99327a146fb94518440e5c1e3c40214` and returned PT-E550W ID `26160`, media ID `261`; it did not inspect the GUI canvas, barcode data, or print output |
| Generic Code 39 LBX style attribute names match Brother-generated output | not verified against Brother output | Current fields are corroborated by the independent [`bil-lbx` serializer](https://github.com/orochi235/bil-lbx/blob/main/src/serialize.ts); no app-generated Code 39 fixture is preserved |
| Rust Code 39 raster matches P-touch Editor or iPrint&Label rendering on either target | unknown | The CLI output decoded correctly, but no app-rendered baseline or physical print comparison is available |

## Cycle 18 native QR LBX import

| Claim | Status | Evidence |
|---|---|---|
| Supported ASCII Model 2 QR objects import as native editable `.ptl` QR objects rather than SVG-backed images | implemented, tested | `crates/ptouch-render/src/lbx.rs::import_barcode`; `lbx::tests::imports_model_2_qr_from_editor_barcode_tags` asserts native payload, correction, geometry, no flattening warning, and visible rendered pixels |
| LBX QR error correction, automatic or fixed version, and physical symbol size survive import into `.ptl` and export back to LBX | implemented, tested | `lbx::tests::imports_fixed_qr_version_and_correction_as_native_fields`; `document::tests::fixed_qr_version_requires_layout_version_8`; tests assert ECC, v5, content-box size, `.ptl` v8, and exported `version="5"` / `cellSize="1.250pt"` |
| Imported LBX QR templates render equivalently to the source Brother application on PT-E550W/PT-P900W | partial in-memory evidence | A generated E550W QR archive now has exact visible matrix parity with Rust after aligning the clipped bitmap; P900W, GUI canvas, and physical print behavior remain unverified |


## Cycle 22 Code 39 readable-text parity

| Claim | Status | Evidence |
|---|---|---|
| Rust Code 39 readable text includes visible start/stop delimiters observed in Brother's in-memory render | implemented, verified | `crates/ptouch-render/src/layout.rs`; CLI output from `docs/re/code39-probe.ptl` visually shows `*ABC-123W*` and `zbarimg` still decodes `ABC-123W`; PNG SHA-256 `1738433fa446738390b2d40d58129eb748af0ab331d98bd4524d0ce2e36d179d` |
| Code 39 raster dimensions now match Brother's loaded-object raster | unknown | Brother `GetPrintingImage()` returned 482x128 object-only output; Rust CLI returned a 581x128 full-label raster, so dimensions are not directly comparable |


## Cycle 23 QR raster diagnosis

| Claim | Status | Evidence |
|---|---|---|
| Brother's captured QR raster contains the expected URL payload when normalized for decoding | observed | ZXing-C++ decodes `https://example.test/ptouch-cycle17` from a grayscale 90-degree-rotated copy with 4 px white border; artifact `docs/re/windows-lbx-runtime/runtime-renders/editor-pt-e550w-qr-decoder-normalized.png`, SHA-256 `069a79fa7063cf0ab6c5748b219ddc7e596a1fa3c6ee905838f3ae6db226c2bc` |
| Direct decoder failures establish the Brother QR raster is malformed | disproven | ZXing-C++ recovers the expected payload after the crop-preserving rotation and padding; Cycle 33 later matched every visible matrix pixel to Rust. zbarimg still does not decode the derivative |


## Cycle 24 QR mask and symbol metadata

| Claim | Status | Evidence |
|---|---|---|
| Brother and Rust QR outputs decode as version 3, ECC M, data mask 7, and the same URL | observed | ZXing-C++ `extra` reports `Version=3`, `ECLevel=M`, `DataMask=7` for both the normalized Brother BMP and the Rust PNG. The Brother derivative was rotated before decode, so its reported orientation is not evidence of the source bitmap orientation |
| QR mask choice explains the visible Brother/Rust matrix difference | ruled out as sole cause | Both decoded symbols report data mask 7; codeword/segment/charset details and pixel-level raster mapping remain unknown |


## Cycle 25 Brother QR mode properties

| Claim | Status | Evidence |
|---|---|---|
| The generated Brother QR object is configured for mixed data mode, automatic mask selection, and automatic medium cell size | observed in object property | Read-only Windows probe of `C:\Users\joshd\ptouch-probe\pt-e550w-qr-export.lbx` returned `QrcodeData.Mode=Mixed`, `Mask=Auto`, `CellSize=M`, with MBCS 932, Model2, auto version3, ECC M; output also retained PT-E550W ID 26160 |
| Rust's current QR input for the Cycle 17 URL uses one byte segment | observed from dependency implementation | `qrcodegen::QrSegment::make_segments()` returns one whole-string mode: numeric, alphanumeric, or byte; lowercase URL makes the byte branch apply |
| Brother's `Mixed` mode emits multiple segments for the Cycle 17 URL | unknown | The reflected mode setting does not reveal encoded segment boundaries or codewords |


## Cycle 26 QR segment candidate comparison

| Claim | Status | Evidence |
|---|---|---|
| Rust's version 3 ECC M byte-mode matrix with mask 7 agrees with an independent QR implementation | observed | Python `qrcode` byte-only mode and `qrcodegen` forced to version 3, ECC M, mask 7 produced identical 29x29 matrices (0 differing modules) for `https://example.test/ptouch-cycle17` |
| A four-segment byte/alphanumeric/byte/numeric encoding is equivalent to the Brother symbol | not supported by current evidence | Python `qrcode` with optimization threshold 2 selected four segments and automatic mask 5; Brother's decoded mask is 7. This rejects that exact candidate as a mask match, but other mixed segmentations remain possible |
| Brother `Mode=Mixed` specifies the exact QR segment boundaries | unknown | The reflected setting does not expose segment metadata; no Brother-generated codeword stream or source-app QR fixture is available |


## Cycle 27 in-memory QR property probe

| Claim | Status | Evidence |
|---|---|---|
| Changing the loaded QR payload in memory changes Brother `GetPrintingImage()` output | observed | On numeric QR `1234567890123456`, setting `Barcode.Data=9876543210987654` before `GetPrintingImage()` changed the BMP hash from `f1da64b34aaf93b125d1872b9164b1617dae6087b58dff06dca19074ac61fcb3` to `bf0af4fdcad756c1209eb24309a80f24e680d3f97070a7e9bee33c5c6efea1ba` |
| Changing the loaded QR `Mode` property changes `GetPrintingImage()` output for tested URL, numeric, and uppercase alphanumeric payloads | not observed | URL Mixed vs Binary output had identical SHA-256 `8e75634c3f299c9abbad78759d79e6e8279714766640c42a2d77008ede69530a`; numeric Mixed, Number, and Binary outputs shared SHA-256 `f1da64b34aaf93b125d1872b9164b1617dae6087b58dff06dca19074ac61fcb3`; uppercase alphanumeric Mixed, Binary, and AlphabetAndNumber outputs shared SHA-256 `4b6b2066148279fe5ea366aea7b8d119c88f333cada4a11a874f09070f54e2ea` |
| The QR `Mode` property is ignored across all Brother render/save paths | unknown | The probe only tested `GetPrintingImage()` on three payload classes; no modified LBX was saved and no physical print was made |


## Cycle 28 QR mode scope

| Claim | Status | Evidence |
|---|---|---|
| Brother `GetPrintingImage()` output is unchanged when the in-memory QR mode changes among Mixed, Binary, Number, and AlphabetAndNumber for tested payload classes | observed | Windows probe on URL, numeric, and uppercase alphanumeric LBX fixtures; archived inputs are `qr-numeric-mode-probe.ptl` and `qr-alphanumeric-mode-probe.ptl`; unchanged hashes are listed in the Cycle 27/28 property-probe evidence above |
| Brother's mode property has no effect on GUI previews, saved LBX files, or physical print output | unknown | Only the in-memory `GetPrintingImage()` path was exercised; no modified LBX was saved and no print operation was performed |


## Cycle 29 QR raster property effects

| Claim | Status | Evidence |
|---|---|---|
| Changing the QR cell-size property changes the in-memory raster width | observed | Windows probe changed the URL QR BMP width across XS/S/M/L/XL while height stayed 128 px: 66/99/132/165/198. Only M decoded under the crop-preserving rotation-plus-border transform; hashes and captures for XS/S/L/XL are preserved in `docs/re/windows-lbx-runtime/runtime-renders/` |
| Changing the QR mask property from Auto to Mask0 changes the raster and decoded mask | observed | Mask0 BMP SHA-256 `4db8e3aed4c4c45136412a08dff4d0fc31574b936a8bfc2d668a87211e7bbaac`; ZXing-C++ recovers the expected URL and reports `DataMask=0` |
| The default QR raster appearance difference is caused by a different module matrix | resolved for visible pixels | Cycle 33 found zero pixel mismatches after mapping the captured Brother grid to the Rust version 3 ECC M mask 7 matrix; the Brother bitmap has vertical clipping |


## Cycle 30 QR cell-size geometry

| Claim | Status | Evidence |
|---|---|---|
| Brother cell-size output width is 33 px times the enum values XS=2 through XL=6 | observed | Captured widths: 66, 99, 132, 165, 198 px; each bitmap height is 128 px |
| CellSize acts as pixels per module with two cell units of horizontal margin on each side | observed for visible pixels | Cycle 33 pixel comparison found zero mismatches on a 29-module grid with cell pitch n, origin x=2n, y=2n-14; output width is `(29 + 4) * n` |
| CellSize variants clip the same QR matrix vertically in the fixed 128 px raster | observed | Cycle 33 matched all captured pixels to the Rust matrix with zero differences; XS/S/M clip leading rows, L/XL clip leading and trailing rows. No physical print was performed |


## Cycle 31 QR cell-size decoder sweep

| Claim | Status | Evidence |
|---|---|---|
| Only the CellSize M Brother capture decoded with the tested local decoders and raster normalizations | observed | ZXing-C++ tested rotations 0/90/180/270 with Pillow `expand=False`, nearest-neighbor scales 1x/2x/4x, and white borders 0/4/8/16/32/64 px (scaled with the image); only M decoded. With `expand=True`, retaining the full rotated rectangle, none decoded, including M. zbar decoded none of the five captures in either transform family |
| XS/S/L/XL captures contain a different or malformed visible QR matrix | disproven for visible pixels | Cycle 33 found zero pixel mismatches against the Rust version 3 ECC M mask 7 matrix across every visible module pixel; the captures are truncated at the bitmap edges |


## Cycle 32 QR rotation normalization correction

| Claim | Status | Evidence |
|---|---|---|
| The saved Brother decoder derivative was created with a full, uncropped 90-degree rotation | corrected | Pillow `Image.rotate(90)` used its default `expand=False`, preserving the original rectangular canvas and cropping the rotated content; a 4 px border was then added |
| Retaining the full rotated rectangle decodes the Brother QR capture | not observed | With Pillow `expand=True`, ZXing-C++ did not decode any CellSize raster, including M, even after scaling and borders. The earlier successful M decode depends on the fixed-canvas cropped transform |
| The crop-preserving transform establishes the source image's inherent QR orientation | unknown | The transform itself rotates and crops the image, so scanner orientation from that derivative cannot establish source orientation |


## Cycle 33 Brother/Rust QR module-grid alignment

| Claim | Status | Evidence |
|---|---|---|
| Every visible Brother E550W QR matrix pixel matches Rust's version 3 ECC M mask 7 output | observed | `compare-qr-matrix.py` reports zero pixel mismatches for XS/S/M/L/XL BMPs against `qr-v3-mask7.matrix`, generated by the Rust qrcodegen path for `https://example.test/ptouch-cycle17` |
| The E550W capture clips the QR module grid vertically at its 128 px raster edges | observed | Grid origin `(x,y)=(2n,2n-14)` for cell pitch `n=2..6`; visible matrix rows are XS 5-28, S 2-28, M 1-28, L 0-26, XL 0-21 |
| The visible raster parity result establishes full symbol parity or physical print behavior | unknown | Cropped matrix rows are absent from the BMP; only the in-memory `GetPrintingImage()` E550W capture was compared. No GUI canvas or physical print validation occurred |


## Cycle 34 PT-P900W QR raster comparison

| Claim | Status | Evidence |
|---|---|---|
| Brother PT-P900W returns the expected generated QR object and target metadata | observed | Windows probe loaded SHA-256 `2230fb5b8d343c3067374e670ee23cc4bd0c83b442dffc36d1eb08d8c510fbd1`; returned PT-P900W ID 28464, media 261, Model2, version3, ECC M, mask Auto, CellSize M, mode Mixed, MBCS 932 |
| The full P900W in-memory QR raster matches the Rust version 3 ECC M mask 7 matrix pixel-for-pixel | observed | BMP 292x320, pitch 8, origin (30,20), all 29 rows visible, zero mismatches across 53,824 symbol pixels in `compare-qr-matrix.py`; BMP SHA-256 `7c201ed7b3b0e85703e75ba75c929424ac8c0412103bc64f652cd976b3f4f0ed` |
| The P900W QR bitmap decodes without normalization | observed | zbar and ZXing-C++ both decode `https://example.test/ptouch-cycle17`; ZXing reports version 3, ECC M, data mask 7 |
| In-memory QR matrix parity establishes P900W GUI canvas or physical print parity | unknown | No GUI canvas or physical print was tested |


## Cycle 35 QR payload and segmentation comparison

| Claim | Status | Evidence |
|---|---|---|
| Brother and Rust choose equivalent encoding for a 20-digit payload | unresolved | Both P900W and E550W runtime metadata report version 1; the P900W raster decodes to the expected payload. The current Rust version 1/mask 4 candidate does not align pixel-for-pixel at the assumed object origin, so exact matrix parity remains unverified |
| Brother splits a mixed uppercase/numeric payload into segments that fit version 1 | observed | For `HELLO WORLD 1234567890`, both models report version 1. Rust's default segmenter produces version 2/mask 6; an alphanumeric prefix plus numeric suffix encoded as version 1/mask 3 matches all P900W and E550W raster pixels at the measured pitch and origins |
| Brother's 60-character lowercase payload matches Rust's automatic mask choice | disproven | Both models return version 4. Rust auto selects mask 2, while ZXing-C++ reads mask 1 from the P900W BMP. Forced Rust version 4/mask 1 matches all compared E550W pixels; the full P900W comparison against this forced matrix remains to be run |
| These in-memory comparisons establish GUI or printed behavior | unknown | No GUI canvas or physical print was tested |


## Cycle 36 numeric QR correction-level and matrix alignment

| Claim | Status | Evidence |
|---|---|---|
| The Cycle 35 numeric mismatch represented a different Brother QR matrix | disproven | The original helper used qrcodegen's convenience API, which boosted the requested Medium ECC to Quartile for this payload. Rust's explicit version-range encoder with boosting disabled produces version 1/ECC M/mask 4 and matches every pixel in both Brother captures at E550W origin (8,-6), pitch 4, and P900W origin (30,20), pitch 8 |
| Rust's automatic QR path preserves the requested ECC level | corrected | `render_qr` and LBX QR import now use the automatic version range with ECC boosting disabled. Regression coverage compares a 20-digit payload against explicit Medium-level modules. Before the fix qrcodegen selected Quartile for this payload while Brother reports Medium |
| Brother's 60-character lowercase payload uses Rust's automatic mask | disproven | Both use ECC Medium and version 4, but Rust auto selects mask 2 while the P900W decode reports mask 1. Forcing Rust mask 1 matches all pixels in both models' rasters |
| These in-memory comparisons establish GUI or printed behavior | unknown | No GUI canvas or physical print was tested |


## Cycle 37 QR byte-mask and mixed-segment probes

| Claim | Status | Evidence |
|---|---|---|
| Brother mask 1 on the original 60-byte lowercase payload is a one-payload anomaly | not supported | Three further P900W byte-only version 4/ECC M payloads of 50, 55, and 62 bytes also decode as mask 1. Rust automatic mask selection returns masks 2, 2, and 0 respectively. Forcing Rust mask 1 matches every pixel in all three 324x320 rasters at 8 px/module, origin (30,20) |
| Brother always selects mask 1 for version 4/ECC M byte payloads | disproven | A diverse 60-character lowercase-plus-digit payload decodes as mask 2. Rust's default whole-byte segmentation also selects mask 2 but does not match the matrix. Encoding the lowercase prefix as bytes and the trailing digits as numeric at version 4/mask 2 matches every raster pixel |
| Rust's current default QR segmenter reproduces Brother Mixed-mode segmentation for lower-case text followed by digits | disproven | The diverse fixture's default one-byte-segment matrix differs from the Brother BMP even though version, ECC, and mask agree; the byte-prefix/numeric-suffix candidate matches exactly |
| The mask difference proves a particular alternate penalty formula | unknown | Runtime captures establish selected masks, but the exact Brother mask scoring/tie-break implementation has not been recovered |
| These in-memory comparisons establish GUI or printed behavior | unknown | No GUI canvas or physical print was tested |


## Cycle 38 Brother Mixed-mode segment boundaries

| Claim | Status | Evidence |
|---|---|---|
| Brother selects numeric and alphanumeric segments around byte-only runs for the tested short mixed payloads | observed | Five P900W version 1/ECC M rasters match candidate segment layouts pixel-for-pixel: byte+numeric for `hello 1234567890`, numeric+byte for `1234567890hello`, alphanumeric+numeric for `HELLO WORLD 1234567890`, byte+alphanumeric for `abc123DEF456`, and alphanumeric+byte for `ABC123abc789`. Decoder masks are 0, 3, 3, 3, and 0 respectively |
| Rust's current whole-string segmentation reproduces these mixed-mode matrices | disproven | For the first three payloads, Rust automatic encoding selects version 2 while Brother selects version 1. For the last two, Rust also selects version 1 but produces different matrices at the same ECC and Brother-reported mask. Candidate mixed-mode segment matrices match all 28,224 Brother pixels per P900W raster |
| The observations fully specify Brother's segment optimizer for arbitrary text | unknown | The captures establish five ASCII segmentation cases; Unicode/MBCS, tie-breaking among equal-cost segmentations, long segment count limits, and full GUI editing remain untested |


## Cycle 39 QR mixed-mode segmenter implementation

| Claim | Status | Evidence |
|---|---|---|
| Automatic QR rendering finds compact numeric/alphanumeric/byte segment combinations | observed | `render_qr("HELLO WORLD 1234567890", Medium, None)` returns a version 1-size raster (232x232 px), matching Brother's reported version. Fixed version 1 rendering also succeeds where the previous whole-string segment exceeded capacity |
| P900W LBX import accepts and renders the fixed version 1 mixed payload | observed | The new public import/render regression test loads Model 2, ECC M, version 1 from an LBX archive, preserves editable fields, and successfully renders it |
| The production automatic segmenter reproduces the tested diverse version 4 Brother matrix | observed | The checked-in P900W module reference was extracted from an in-memory runtime raster. The renderer regression test compares every module and passes for the 60-character lowercase/digit payload at version 4, ECC M, mask 2 |
| The implementation reproduces exact Brother matrices for all five short mixed payloads | not observed | Rust selects version 1 and its production matrices equal the manually segmented qrcodegen candidates, but qrcodegen selects masks 6, 4, 6, 2, and 2 while Brother reports 0, 3, 3, 3, and 0. Those candidate matrices match Brother when forced to Brother's masks; mask selection remains a separate gap |
| The segmenter establishes Unicode/MBCS and GUI/physical-print parity | unknown | Cycle 39 covers ASCII numeric, alphanumeric, and byte segments through render and LBX import. Unicode/MBCS conversion, exact tie-breaking, GUI canvas behavior, and physical output remain unverified |


## Cycle 40 QR mask-score probes

| Claim | Status | Evidence |
|---|---|---|
| Brother selects the lowest standard qrcodegen penalty for every version 4 byte payload | disproven | Seven P900W/ECC M/lowercase-only runtime captures decode at version 4 and masks 3, 6, 2, 7, 6, 6, and 0. Each full 33x33 Brother module grid matches the corresponding Rust fixed-mask matrix exactly. Standard N1+N2+N3+N4 scoring predicts masks 2, 2, 2, 2, 6, 6, and 2 |
| Brother's mask difference is explained by omitting N3 | disproven | The same seven fixed-mask matrices without N3 predict masks 3, 3, 6, 7, 2, 2, and 3. This matches only probes 02 and 20; probes 14 and 30 select neither prediction |
| A single alternate weight for N3 explains the seven masks | disproven | Probe 14's selected mask is not the minimum for any nonnegative N3 weight when the other qrcodegen components are held fixed |
| The seven captures differ because their byte segment layout is unknown | not supported | All seven payloads contain only lowercase ASCII, use version 4/ECC M, and each captured grid compares pixel-for-pixel against Rust's version 4 byte matrix at Brother's decoded mask |
| The findings establish a global Brother mask rule | unknown | The tests cover seven lowercase-only version 4 payloads and do not cover other QR versions, ECC levels, Unicode/MBCS, or GUI canvas/physical output |
| Native DLL analysis located the QR mask selector | not observed | Hopper opened the 32-bit native `lbxcore.Win32.dll` (SHA-256 `f69a0fa0fd2e37f825436170853e7efdbbe04b299011d8ec11b6c82668fcc704`). The `:qrcodeStyle` references resolve to settings serialization; no named mask-selection routine was identified |


## Cycle 41 QR mask scoring implementation

| Claim | Status | Evidence |
|---|---|---|
| Brother uses qrcodegen's exact finder-pattern penalty scan | disproven | Standard qrcodegen scoring predicts Brother's selected mask in 5 of the 19 analyzed captures |
| A libqrencode-style N3 scan better matches Brother | observed | A local implementation of the scan in libqrencode's public `mask.c` matches 17 of 19 Brother mask selections: all seven Cycle 40 version 4 byte-only probes, four of five Cycle 38 version 1 mixed probes, three of four repetitive version 4 byte probes, and the numeric version 1, URL version 3, and diverse mixed version 4 controls |
| The new Rust QR renderer reproduces all seven Cycle 40 module grids | observed | The production encoder's seven version 4 ECC M outputs compare module-for-module with the checked-in matrices extracted from Brother's P900W in-memory BMPs; each earlier BMP comparison had zero mismatches across 69,696 module pixels |
| The libqrencode-style score is a complete Brother mask rule | disproven | It predicts mask 3 for `ABC123abc789` where Brother uses 0, and mask 2 for the 55-byte repetitive lowercase payload where Brother uses 1. The cause of these two differences remains unknown |
| The implementation identifies Brother's internal QR library | unknown | Matching output is behavioral evidence only. Hopper did not identify a QR mask routine in the inspected Windows DLL |
| The new rule establishes GUI or printed parity | unknown | Validation uses the in-memory `GetPrintingImage()` path. No GUI canvas or physical printing was tested |


## Cycle 42 QR mask behavior across ECC levels

| Claim | Status | Evidence |
|---|---|---|
| Brother's selected mask follows the Cycle 41 scoring behavior across all ECC levels | observed in this sample | One 50-character lowercase byte payload was rendered at ECC L, M, Q, and H. The P900W runtime reported versions 3, 4, 5, and 6, respectively. Format bits in the captured BMPs identify masks 2, 2, 4, and 4 with zero BCH correction errors. The replayable extractor is `docs/re/windows-lbx-runtime/extract-qr-format-probes.py`. |
| The version 6/ECC H mask can be read with the ordinary image decoder | disproven | The fixed-height 320-pixel P900W raster clips the 41-module symbol's bottom rows, so ZXing does not decode the full symbol. The intact top-left format bits identify H/mask 4 exactly. |
| The Cycle 41 scoring match is a complete mask-selection rule | unknown | These four ECC cases provide cross-level observations, but do not characterize all versions, payload classes, tie behavior, or Brother's internal implementation. The two Cycle 41 exceptions remain unexplained. |
| The captures establish GUI or printed parity | unknown | All captures came from the Windows Editor's in-memory `GetPrintingImage()` method. No GUI canvas or physical printing was tested. |


## Cycle 43 QR mask scoring with a quiet margin

| Claim | Status | Evidence |
|---|---|---|
| Adding one white module around a candidate explains both residual mask cases | disproven | The replayable `docs/re/windows-lbx-runtime/analyze-qr-mask-margins.py` evaluates all eight forced masks for seven Cycle 40 byte-only controls and both residual cases. A one-module border selects Brother's mask 1 for the 55-byte repetitive string, but still selects mask 3 instead of 0 for `ABC123abc789`. |
| Border scoring improves agreement across the nine exact-mask captures | disproven | Plain libqrencode-style scoring matches seven of nine Brother selections. Border scoring matches six of nine because it also changes Cycle 40 probes 14 and 20 from their correct masks 6 and 7 to masks 2 and 2. |
| The residual score gaps are large | disproven | Without a border, the selected Brother masks are 10 points from the winner for `ABC123abc789` and 16 points for the 55-byte case. The script records all N1/N2/N3/N4 components and top-two margins for each candidate. |
| The findings identify Brother's scoring domain | unknown | The border hypothesis is not a general explanation. No production scoring change is justified by these nine controls, and Brother's internal implementation remains unidentified. |


## Cycle 44 forced-mask matrix comparison

| Claim | Status | Evidence |
|---|---|---|
| The residual cases use different encoded data at masks other than Brother's automatic choice | disproven | The Windows P900W runtime was set in memory to each mask 0-7 for `ABC123abc789` (version 1/ECC M) and the 55-byte repetitive lowercase payload (version 4/ECC M). All 16 resulting full module grids match Rust fixed-mask matrices with zero pixel mismatches. |
| Payload segmentation, version, or ECC explains the two automatic mask differences | disproven for these controls | The mixed payload matches at all eight masks using the Cycle 38 alphanumeric-prefix/byte-suffix segmentation. The repetitive payload matches at all eight as one byte segment. Runtime metadata reports the same versions and ECC as Rust. |
| Brother's automatic mask selector is the only remaining source of these two matrix differences | observed | With encoded matrices held constant across all forced masks, Brother still automatically selects mask 0 instead of the libqrencode-style mask 3 for the mixed payload, and mask 1 instead of mask 2 for the repetitive payload. |
| The evidence establishes physical print parity | unknown | Tests used in-memory `GetPrintingImage()` captures only. No printer handle, settings write, or print job was used. |


## Cycle 45 N2 penalty weight sweep

| Claim | Status | Evidence |
|---|---|---|
| A higher integer N2 weight resolves the two residual Brother mask choices | disproven | `docs/re/windows-lbx-runtime/analyze-qr-n2-weights.py` regenerated all eight candidates for 19 controls and swept N2 weights 1-8 while leaving N1/N3/N4 unchanged. The residual choices remain mismatched at every tested weight. |
| Raising N2 improves agreement with the 19 captures | disproven | Weight 1 matches 17/19. Weights 2-8 match 16, 13, 13, 12, 12, 10, and 10, respectively. |
| The sweep identifies the complete Brother mask rule | unknown | Only one component's integer weight was varied. Other scan definitions, score domains, tie-breaks, and payload families remain open. |


## Cycle 46 N1 and N4 variants

| Claim | Status | Evidence |
|---|---|---|
| Increasing the integer N1 run-penalty weight resolves the residual masks | disproven | `docs/re/windows-lbx-runtime/analyze-qr-n1-n4-variants.py` sweeps N1 weights 1-8 across the same 19 controls. Weight 1 matches 17/19; weights 2-8 match 16, 13, 12, 12, 12, 11, and 11. Neither residual choice changes to Brother's selected mask. |
| Alternative N4 dark-ratio rounding improves agreement | disproven | Nearest, floor, and ceiling integer-percent bands, and qrcodegen's exact-ratio calculation all retain the same 17/19 matches and the same two residual winners. |
| N1 weights and N4 rounding determine Brother's complete mask rule | unknown | This sweep varies only simple integer weights and ratio formulas. More general pattern scans, scoring domains, and payload families remain untested. |


## Cycle 47 horizontal and vertical scan domains

| Claim | Status | Evidence |
|---|---|---|
| Horizontal-only N1 scanning with both N3 axes resolves both residual masks | observed in the 19-case sample | `docs/re/windows-lbx-runtime/analyze-qr-scan-domains.py` predicts Brother mask 1 for the 55-byte repetitive payload and mask 0 for `ABC123abc789`, while keeping N2/N4 and both N3 axes. |
| That axis-specific scan is a better general rule | disproven | It matches 17/19, the same as the existing score. It shifts errors to Cycle 40 probe 20 (predicts 2, Brother 7) and Cycle 38's interleaved byte/alphanumeric payload (predicts 2, Brother 3). Horizontal-only and vertical-only N1/N3 each match 13/19. |
| Horizontal and vertical scoring can be discarded for all payloads | unknown | The tested candidates are broad axis exclusions and one mixed variant. Payload-conditioned scans and other scoring domains remain untested. |
