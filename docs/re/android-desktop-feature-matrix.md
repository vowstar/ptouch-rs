# Android/Desktop feature matrix (Cycle 2 evidence pass)

Status: Cycle 2 static inventory baseline with Cycle 5 implementation, Cycle 6 target-specific evidence, Cycle 7 text import, Cycle 8 embedded image import, Cycle 9 vector-outline import, Cycle 10 PT Dingbats symbol import, Cycle 11 QR import, Cycle 12 CLI numeric placeholder batches, and Cycle 13 GUI placeholder values. Android evidence includes manifest declarations and decompiled code paths. P-touch Editor and Printer Setting Tool evidence is limited to package/assembly inventories and packaged resources. Static evidence does not establish runtime execution or full feature parity. Unknowns below are not claims of feature absence.

| Feature area | Design&Print 2 2.8.1 | iPrint&Label 5.3.17 | P-touch Editor 6.12.10 | Printer Setting Tool 1.6.0142 | Integrated `ptouch-rs` current state after Cycle 10 |
|---|---|---|---|---|---|
| Label editing | UI strings expose text, image, clipart/symbol, template, and name-label additions. Its printer catalog is Cube-family only; JADX enum fallback lists no target model. | Decompiled `EditLabelActivity`/`LabelView` create TEXT, SYMBOL, IMAGE, BARCODE, DATETIME, FRAME, RECT, and POLY LBX objects. Barcode protocol/data and image bitmap/crop fields are set in app code. | Main WPF and LBX assemblies are identified. `LbxFile.dll` is x86 .NET 8 CIL (9 types, 29 methods); specific editing semantics were not recovered in this pass. | `DeviceSettingsTool.Command.dll` is a managed command assembly, not a label editor. | GUI editor supports text, images, symbols/shapes, frames, layouts, shared rendering, and static QR images imported from LBX. Native barcode creation/editing controls are not implemented. |
| Barcode and QR | Barcode-related UI strings present; implementation not traced. | Editor creates CODE39, CODE128, EAN13, EAN128, and QR objects with data, check-digit, human-readable, QR version, and error-correction settings. | Barcode behavior is unknown from inspected package inventory; do not infer absence. | Barcode authoring is outside the setting-tool module inventory inspected here. | Imports a bounded ASCII Model 2 QR subset as an SVG-backed image; other protocols remain unsupported. Generic XML fields come from a P-touch Editor 5.0.201 QL-700 sample. Exact PT-P900W/PT-E550W rendering parity is unknown; preserved local fixtures contain no barcode LBX. |
| Templates and LBX exchange | `SharedFileReceiveActivity` declares LBX open/share entry points; template/name-template and Label Place UI strings present. | `.lbx` is read as a ZIP containing `label.xml`; preserved starter samples also contain `prop.xml`. The writer maps both targets to model-specific paper maps. PrintTask renders ordinary LBX jobs through a bitmap PNG path. | `LbxFile.dll` and `LbxViewBase.Wpf.dll` are packaged. Ten CIV templates include cable, heat-shrink, patch-panel, terminal-block, and related designs. Import/export and print semantics remain unknown. | PT900 model module includes `brdvset.ini`; E550W has no separate model module in this package version. | Cycles 7-10 import ordinary text, PT Dingbats symbols, untrimmed embedded images, and selected unfilled vector outlines from LBX in CLI and GUI. Symbols become inline-font text glyphs and require a matching Brother font; shapes become SVG-backed image objects. Nested supported objects flatten with warnings. Barcode objects, trimmed/effect-adjusted images, fills, and LBX export remain gaps. |
| Batch and data sources | Not characterized in this pass. | Numbering changes LBX objects by configured start and increment values, then creates bitmap jobs for configured count and copies. Database-assigned LBX jobs use a prepared bitmap job list. UI declares CSV transfer, but exact accepted schemas and runtime behavior remain open. | Batch/data-source behavior is unknown from inspected inventory. | Not characterized in this pass. | GUI single-label placeholder values, CLI numeric placeholder batches, and CLI CSV row printing are implemented. Native selected-character numbering inside LBX text and database-assigned jobs are not matched. |
| Target model coverage | DEX enum has exactly PT-N25BT, PT-P300BT, PT-P300BTz, PT-P320BT, PT-P710BT, PT-P715E, and PT-P910BT; neither target is present in this catalog. | Both targets are explicit SDK/LBX models; model-to-LBX mapping and stored-ID resolution are present in code. | Exact P900W/E550W print support remains unverified. | PT-P900W is mapped by PT900 module as model 0x6F, series 0x30. No E550W DeviceSettings module found in this version. | PT-E550W and PT-P900W models have USB and explicit raw TCP print paths; model raster profiles and high-resolution jobs are covered by virtual-printer tests. |
| Printer connectivity | Bluetooth and Wi-Fi permissions declared; no USB-host declaration or target model in this catalog. | Both targets map to infrastructure Wi-Fi, Wireless Direct, and USB. Bluetooth and wired flags are false. Print-control selects Wi-Fi, Wireless Direct, Bluetooth, or USB helper classes by connection item. | Brother print/transport modules are packaged; target-specific dispatch is unverified. | Shared `ptfilo.dll`/P/Invoke declarations and model modules are packaged; no E550W module. | USB and raw TCP for the targets; GUI DNS-SD discovery is available. macOS Bluetooth support exists for other supported models; target-model Bluetooth behavior is not established. |
| Media and print settings | UI strings expose tape/media, length, margins, auto-cut, half-cut, cut marks, and print options; protocol mapping unknown. | E550W media UI lists standard widths through 24 mm, SL 24 mm, and HS 5.8-23.6 mm. P900W uses a shared standard/SL/FL/HS list through 36 mm. The P900W and E550W defaults are 36 mm and 24 mm respectively. Physical compatibility and writable setting semantics remain unverified. | Print-related assemblies are present; full device/settings semantics not inventoried. | PT900 module covers PT-P900/P900W/P950NW; PT800 covers E800/E850/D800 family but not E550W. Five `ptset` settings are separately recovered for P900W, with SET writes unverified. | USB and TCP use standard tape-width codes; the GUI detects PT-P900W media with SNMP and offers manual PT-E550W selection. SL, FL, and HS media types are not modeled. P900W Command Mode is read-only; SET writes remain unverified and disabled. |
| Offline output | Not established in this pass. | Not established in this pass. | Not established in this pass. | Not applicable to this settings tool. | PNG/JPEG/BMP/GIF/TIFF/WebP export without a connected printer. |

## Cycle 3 implementation evidence (historical)

- `ptouch-core` registers PT-E550W at USB PID `0x2060` with a 128-pixel, 180 dpi raster profile and PackBits, raster-mode-A, and info-command flags; it registers PT-P900W at PID `0x2085` with a 560-pixel, 360 dpi profile, raster-mode-A, and info-command flags. Both are covered by `device::tests::target_models_match_reference_raster_profiles`.
- `PtouchDevice::open_tcp(SocketAddr, product_id)` connects explicitly to a supported model. The loopback fixture exercises both target IDs, raw byte send/receive, partial reads, cancellation, timeout handling, and rejection of an unknown product ID.
- At the end of Cycle 3, TCP was core-library only; Cycles 5 and later wired CLI and GUI network selection and discovery.

## Cycle 4 branch updates

- `PrintQuality::HighRes` is supported for PT-E550W and PT-P900W via ESC `i K` bit 6. The print job duplicates each input raster line, reports the doubled raster line count, and doubles the 2 mm default margin at the model's base resolution: 28 dots at 180 dpi for E550W and 56 dots at 360 dpi for P900W.
- The PT-E550W retains PackBits compression, which is required for cutting; PT-P900W remains uncompressed by default. USB-session and loopback TCP tests pin standard-job bytes; target high-resolution bytes are pinned in unit and TCP virtual-printer tests. Raster bitmaps round-trip at both target head widths, and virtual-printer error, cancellation, and teardown paths are covered. No physical printer was used.
- Draft quality remains available only on legacy-hires profiles. CLI and GUI now accept an explicit TCP address and model selection; both routes are exercised against loopback printer fixtures.

## Cycle 5 branch updates

- CLI and GUI can select a raw TCP endpoint and supported model for printing. The CLI requires `--tape-width-mm` for models without network status. GUI browses `_pdl-datastream._tcp.local.` and maps recognized DNS-SD model text to supported product IDs; discovery advertisements themselves have not been exercised in tests.
- Raw TCP does not send the USB `ESC i S` status request. PT-E550W tape width is selected in the GUI or passed as `--tape-width-mm WIDTH_CODE` in the CLI; widths incompatible with its 128-pixel head are rejected. PT-P900W media status uses Brother SNMP OID `1.3.6.1.4.1.2435.3.3.9.1.6.1.0`; the community defaults to `public` and can be set with `PTOUCH_SNMP_COMMUNITY`.
- PT-P900W width selection follows the detected cassette, and advanced high-resolution is offered only when status confirms laminated tape and black text. Printing refreshes media status and refuses a changed or absent cassette. PT-E550W has no verified automatic tape-detection route. Discovery, SNMP decoding, and print routes use loopback fixtures; no physical printer was used.
- CLI `info` reports raw-TCP status as unavailable for models without a verified network status route. A P900W SNMP response that reports no tape does not fall back to manual selection.
- `ptouch config get command-mode` provides a read-only PT-P900W query over explicit TCP. Its GET bytes and reply-byte mapping come from the preserved `ptset` evidence; no physical TCP query or SET operation was performed.
- Verification on 2026-10-17: `cargo fmt --all -- --check`, `cargo test --workspace` (367 passed), and `cargo clippy --workspace --all-targets --all-features -- -D warnings` all passed. TCP discovery advertisements and physical network behavior remain unverified.

## Cycle 6 target workflow inventory

- iPrint&Label's target media UI uses a dedicated PT-E550W list containing 3.5, 6, 9, 12, 18, and 24 mm tape, SL 24 mm, and HS 5.8, 8.8, 11.7, 17.7, and 23.6 mm. Its PT-P900W capability route uses the shared P900/P950/E800/D800/E850 list containing standard widths through 36 mm, SL 24/36 mm, FL 21 x 45 mm, and HS widths through 23.6 mm. The app chooses default PT_24_MM for E550W and PT_36_MM for P900W. These are app configuration paths; physical acceptance by each model is not established.
- P-touch Editor 6.12.10 `LbxFile.dll` managed metadata exposes `LoadLbxFilesAsync` and properties for filename, creation date, path, thumbnail path, media width, and media height. This establishes file-library metadata surfaces, not full editor import/export or print behavior.
- The ten preserved CIV `.lbx` sample archives each contain `label.xml` and `prop.xml`. Their XML includes document/sheet/paper metadata and template structures such as cable, self-lamination, heat-shrink, patch-panel, faceplate, punch-down, and breaker-box cells with nested text properties. The Rust importer currently skips these specialized wrappers.
- Highest-ranked confirmed implementation gaps are LBX template preservation and export, native barcode editing and non-QR barcode import, iPrint&Label numbering/database assignment, and specialty-media semantics. P-touch Editor method-level behavior and full iPrint&Label runtime behavior remain unknown.

## Cycle 7 LBX text import

- `ptouch-render::lbx::import` reads `label.xml` from an LBX ZIP archive and converts ordinary text objects into physical text objects, preserving point geometry, rotation, horizontal alignment, and nominal font size.
- `LabelDocument::from_path` is shared by CLI and GUI. The GUI file picker accepts `.ptl` and `.lbx`; CLI layout input detects LBX by extension.
- Specialized objects, including cable, table, drawing, unsupported barcode protocols, and image objects, are skipped with explicit warnings. An archive with no supported objects returns an error rather than an empty label.
- Archive size is capped at 64 MiB and `label.xml` at 8 MiB before XML parsing.
- This importer is a first compatibility slice, not general LBX parity. Image resources, symbols, most barcode semantics, special media, paper orientation and margins, XML save/export, and runtime comparison against Brother applications remain open.

## Cycle 8 embedded image import

- Direct `image:image` objects load their `image:imageStyle fileName` resource from the LBX ZIP into an editable physical image object.
- Untrimmed images with default effects and no explicit transparency are decoded, embedded in `.ptl` documents, and rendered by the shared renderer. A round-trip path test verifies the shared file loader and a raster test verifies visible ink.
- Trimmed images, non-default brightness/contrast/effects, explicit transparency, missing resources, and image formats rejected by the shared image decoder do not silently disappear. Trimming/effect cases return explicit warnings; malformed or unsupported image data returns an error.
- Image resource reads are capped at 32 MiB. Crop fidelity, transparent-image handling, shapes, barcodes, specialized CIV templates, and LBX export remain open.

## Evidence locations

- Design&Print 2 manifest and activities: `reference/artifacts/android-designandprint2-2.8.1/jadx/resources/AndroidManifest.xml`, especially `SharedFileReceiveActivity` and `SetPrintConfigActivity`.
- Design&Print 2 model catalog: APK `base.apk`, SHA-256 `cd22e32e2b594931a4365fe1fbb8a5581a93a386c007e83b2dd648b44196f0ee`, class in `classes2.dex`; single-class JADX 1.5.6 fallback lists enum fields despite normal-mode enum restoration warning. Evidence details are in `docs/re/evidence.md`.
- Design&Print 2 editor and print UI strings: `reference/artifacts/android-designandprint2-2.8.1/jadx/resources/res/values/strings.xml` (`edit_add_item_*`, `print_confirm_dialog_*`, `print_settings_*`, `label_place_*`).
- iPrint&Label manifest and activities: `reference/artifacts/android-iprintandlabel-5.3.17/jadx/resources/AndroidManifest.xml`, especially USB-host declaration, editor, shared-file, printer-connection, and settings activities.
- iPrint&Label editor, data, transport, and print UI strings: `reference/artifacts/android-iprintandlabel-5.3.17/jadx/resources/res/values/strings.xml` (`all_add_object_*`, `barcode_*`, `database_*`, `connect_via_*`, `chain_print`, `numbering_*`, `print_*`, `wifi_*`).
- iPrint&Label target routing: `device/presets/Device.java` (model switch, transport predicates and printer code), `device/BrotherDevicePresets.java`, and `printerconnect/PrinterConnectSupportInfo.java`.
- iPrint&Label model-to-LBX mapping and print dispatch: `utils/PrinterUtility.java` (lines 107-155), `edit/EditLabelActivity.java` (printer selection, object creation, stored-ID mapping and template model selection), `print/PrinterControl.java` (lines 169-181), and `print/PrintTask.java` (LBX/image/PDF/database/numbering branches at lines 125-139 and 165-190).
- iPrint&Label editor object creation: `edit/EditLabelActivity.java` (lines 1156-1168, 1176-1222, 1225-1287) and `edit/LabelView.java` (lines 718-745).
- iPrint&Label LBX archive conversion and target media mappings: `compatible/LBXTagWriter.java` (lines 256-303, 756-805, 1329-1340) and `printersetting/PrinterSettingUtility.java` (lines 343-376, 398-456); media labels and indexes: `res/values/arrays.xml` (lines 432-498, 640-697).
- iPrint&Label barcode encoding options: `lbxwrapper/Barcode.java` (`CodeProtocol`, QR version, and ECC fields); editor assignment of data, protocol, check digit, and human-readable setting: `edit/EditLabelActivity.java` (lines 1156-1168, 2152-2171).
- iPrint&Label numbering and database print dispatch: `print/PrintTask.java` (lines 61-92, 112-139, 171-190); numbering values and database assignment originate in `edit/PrinterOptionDialogFragment.java` and `object/PrintParameter.java`.
- iPrint&Label connection-specific print helper choice: `print/PrinterControl.java` (lines 111-128). LBX model IDs, paper-map selection, and reported model DPI: `compatible/LBXTagWriter.java` (lines 256-303).
- Managed assembly identities: `LbxFile.dll` and Printer Setting Tool `DeviceSettingsTool.Command.dll` metadata are recorded in `docs/re/evidence.md` with REA Evidence IDs and SHA-256.
- P-touch Editor package structure, managed assembly references, and bounded feature unknowns: `docs/re/evidence.md`; packaged sample templates are under `reference/artifacts/ptouch-editor-6.12.10/6/Resources/Templates/CIV/`.
- Printer Setting Tool model-module and recovered `ptset` configuration findings: `docs/re/evidence.md` and `docs/re/ptset-original.md`.
- Integrated Rust baseline: `README.md` and `docs/re/evidence.md`.
- Proprietary desktop artifact identities and package contents: `docs/re/evidence.md` and `docs/re/artifact-manifest.sha256`.
- P-touch Editor packaged CIV templates: `reference/artifacts/ptouch-editor-6.12.10/6/Resources/Templates/CIV/` (ten exact filenames are recorded in the preserved artifact manifest).

## Next evidence probes

1. Obtain target-app barcode fixtures and compare QR plus one-dimensional barcode output against Brother rendering before expanding import or adding native authoring.
2. Compare preserved template and image fixtures against iPrint&Label or P-touch Editor rendering when an app-rendered baseline is available.
3. Model nested CIV wrapper semantics and retain unsupported source metadata before claiming those templates round-trip; add barcode/data fixtures as the object model can preserve them.
4. Obtain focused managed-member analysis of P-touch Editor import/export and print paths; keep physical media compatibility, Android runtime behavior, and unsupported setting semantics unknown until stronger evidence is available.


## Cycle 9 vector outlines and nested templates

- Imports unfilled line, polyline, polygon, rectangle, and ellipse outlines as SVG-backed image objects, preserving order among supported children. This does not preserve native shape editing.
- Recursively flattens supported child objects from nested wrappers and warns that wrapper-specific behavior is lost. The preserved cable-flag sample renders offline, but has not been compared against P-touch Editor or iPrint&Label output.
- Landscape left/top page margins are applied with the axis swap used by the preserved sample. Unsupported fills, round rectangles, non-default arrows, and pen variants remain explicit gaps.


## Cycle 10 PT Dingbats symbols

- Recognizes `draw:symbolStyle` with a PT Dingbats family and documented code range, maps each code to U+F000 plus the stored value, and preserves the family as an inline text style. Symbol frames and rotation use the ordinary text-object path.
- The shared path loader makes imported glyphs available to both CLI and GUI. Import warns that the native symbol object is converted to text and warns when the requested family lacks the requested glyph or is absent.
- Android source and bundled font metadata confirm the mapping and font family names. Exact Brother glyph fitting remains unknown because no preserved symbol-bearing LBX or source-app rendering baseline is available.

## Cycle 11 QR import

- Imports supported ASCII Model 2 QR objects from LBX as SVG-backed images. Native barcode editing is not retained, other protocols are skipped, and exact P900W/E550W rendering parity remains unknown.
- Generic QR XML fields are evidenced by an external P-touch Editor 5.0.201 QL-700 sample. Local preserved app/template inventories contain no barcode LBX fixture.

## Cycle 12 numeric placeholder batches

- `ptouch print --layout FILE --number-var NAME` emits one label per value, with `--number-start`, `--number-increment`, and `--number-count`; `--copies` applies through the existing batch printer path.
- Values replace a declared text placeholder in a cloned layout for each render, leaving the saved source file unchanged. Exports require `{n}` in the output path.
- iPrint&Label source confirms separate count/start/increment settings and per-label value progression, then renders a bitmap job list and applies copies across that sequence: `PrintParameter.java`, `EditLabelActivity.java:setTextNumber`, and `PrintTask.java:printNumberingLbx`.
- This is a CLI placeholder sequence. Native LBX selected-character numbering, its serialization, GUI authoring, and database-assigned jobs remain open.
