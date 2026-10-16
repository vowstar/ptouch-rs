# Android/Desktop feature matrix (initial static pass)

Status: preliminary evidence inventory for Cycle 2. Android evidence below is from decompiled manifest declarations and resource strings. These establish declared entry points and exposed UI vocabulary, not successful execution or parity. Desktop claims refer to the preserved P-touch Editor install and the integrated upstream application's README. Do not infer missing functionality from the absence of a string or manifest component.

| Feature area | Design&Print 2 2.8.1 | iPrint&Label 5.3.17 | P-touch Editor 6.12.10 | Integrated `ptouch-rs` baseline |
|---|---|---|---|---|
| Label editing | Static UI labels expose text, image, clipart/symbol, template, and name-label additions. Actual editor operations need call-path review. | Static UI labels expose text, image, shape, frame, symbol, barcode, contact details, and date/time objects. Actual editor operations need call-path review. | Windows WPF editor and LBX-related assemblies are present; detailed feature inventory pending. | GUI editor supports text, images, symbols/shapes, frames, layouts, and shared rendering. |
| Barcode and QR | Barcode and barcode-printing option strings are present; implementation behavior unknown. | Barcode formats include CODE128, CODE39, EAN13, QR, and UCC/EAN-128 in resource strings. | Detailed barcode inventory pending. | No parity claim established in current README/evidence. |
| Templates and LBX exchange | `SharedFileReceiveActivity` declares LBX file open and file-share entry points; template/name-template and Label Place strings are present. | `SharedFileReceiveActivity` declares LBX/LBXS open and sharing; template and download strings are present. | LBX stack assemblies and templates are present in the installation. | `.ptl` layout templates and CSV batch substitution exist; LBX interoperability unknown. |
| Batch and data sources | Initial pass has no evidence sufficient to characterize batch/data-source behavior. | CSV/spreadsheet intent filters, database UI strings, and sequence-numbering strings are present. Runtime support and exact formats need source tracing. | Detailed inventory pending. | CSV-driven batch printing and placeholder substitution are documented. |
| Printer connectivity | Manifest declares Bluetooth scan/connect and Wi-Fi state/network permissions; USB host declaration absent. Connectivity implementation and supported models need source tracing. | Manifest declares Bluetooth, Wi-Fi/network, USB-host and NFC features/permissions; UI strings describe infrastructure Wi-Fi, Wireless Direct, Bluetooth, USB, and NFC. Exact transports and model list need source tracing. | Package includes Brother print/transport modules; supported-model and settings coverage need enumeration. | Upstream baseline supports USB and macOS Bluetooth, no TCP transport; PT-P900W absent and PT-E550W raster unsupported. |
| Media and print settings | Resource strings expose tape/media settings, length, margins, auto-cut, half-cut, cut marks, and print options. Exact protocol mapping unknown. | Media settings, printer settings, print range, numbering, chain print, and print log strings are present. Exact model-specific behavior unknown. | Printer Setting Tool and P-touch Editor package evidence are recorded in `evidence.md`; detailed settings inventory pending. | Existing USB printing, chain/multi-copy, cut/feed, and offline export features are documented. |
| Offline output | Not established in this pass. | Not established in this pass. | Not established in this pass. | PNG/JPEG/BMP/GIF/TIFF/WebP export without a connected printer is documented. |

## Evidence locations

- Design&Print 2 manifest and activities: `reference/artifacts/android-designandprint2-2.8.1/jadx/resources/AndroidManifest.xml`, especially `SharedFileReceiveActivity` and `SetPrintConfigActivity`.
- Design&Print 2 editor and print UI strings: `reference/artifacts/android-designandprint2-2.8.1/jadx/resources/res/values/strings.xml` (`edit_add_item_*`, `print_confirm_dialog_*`, `print_settings_*`, `label_place_*`).
- iPrint&Label manifest and activities: `reference/artifacts/android-iprintandlabel-5.3.17/jadx/resources/AndroidManifest.xml`, especially USB-host declaration, editor, shared-file, printer-connection, and settings activities.
- iPrint&Label editor, data, transport, and print UI strings: `reference/artifacts/android-iprintandlabel-5.3.17/jadx/resources/res/values/strings.xml` (`all_add_object_*`, `barcode_*`, `database_*`, `connect_via_*`, `chain_print`, `numbering_*`, `print_*`, `wifi_*`).
- Integrated Rust baseline: `README.md` and `docs/re/evidence.md`.
- Proprietary desktop artifact identities and package contents: `docs/re/evidence.md` and `docs/re/artifact-manifest.sha256`.

## Next evidence probes

1. Inventory app-owned packages/classes separately from AndroidX and bundled libraries; trace the editor's object creation and print entry points in each Android app.
2. Extract the complete printer model tables and transport dispatch for the two Android apps, focusing on PT-P900W and PT-E550W.
3. Enumerate P-touch Editor import/export, editing, print, and barcode features from relevant managed assemblies and resources.
4. Replace each preliminary string/manifest-only row with implementation evidence, then add explicit unknowns where decompilation or static analysis cannot establish behavior.
