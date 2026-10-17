# Windows LBX runtime probe

This probe loads LBX archives with the `LibLbx.DotNet.dll` shipped with Brother
P-touch Editor. It calls the same `LibLbx.Initialize` overload as the Editor,
then reads document metadata and objects through `LbxFactory.CreateFromPath`.
It calls `GetPrintingImage()` to capture the library's in-memory rendering. The
probe does not save an LBX, open a printer handle, or send a print job.

The tested machine was Windows 11 `ryzen`, with P-touch Editor 6.12.10 and
.NET SDK 8.0.426. The Editor's startup supplied `P-touch Editor 6.12.1.0
Windows`, locale ID 1033, and its `Fonts` directory to `LibLbx.Initialize`.
The `LibLbx.DotNet.dll` SHA-256 was
`7b892c240e7deff849895a987a449ebfeaefdc3a94a788a459c32a50dbf12539`.

The text sample contains one absolute text object with `PARITY PROBE`. The QR
sample contains one editable Model 2 QR object sized to fit 24 mm test tape.
Its offline render decoded to the original payload with `zbarimg`. Generate both target-specific
archive pairs from the repository root:

```sh
cargo run -p ptouch-cli -- convert docs/re/windows-lbx-runtime/probe.ptl \
  --output docs/re/windows-lbx-runtime/pt-e550w-export.lbx --printer pt-e550w
cargo run -p ptouch-cli -- convert docs/re/windows-lbx-runtime/probe.ptl \
  --output docs/re/windows-lbx-runtime/pt-p900w-export.lbx --printer pt-p900w
cargo run -p ptouch-cli -- convert docs/re/windows-lbx-runtime/qr-probe.ptl \
  --output docs/re/windows-lbx-runtime/pt-e550w-qr-export.lbx --printer pt-e550w
cargo run -p ptouch-cli -- convert docs/re/windows-lbx-runtime/qr-probe.ptl \
  --output docs/re/windows-lbx-runtime/pt-p900w-qr-export.lbx --printer pt-p900w
```

Build and run the probe on Windows from this directory. Change the `editor`
constant in `Program.cs` if P-touch Editor is installed elsewhere.

```powershell
dotnet publish -c Release -r win-x86 --self-contained true
.\bin\Release\net8.0\win-x86\publish\LbxProbe.exe `
  .\pt-e550w-export.lbx .\pt-p900w-export.lbx `
  .\pt-e550w-qr-export.lbx .\pt-p900w-qr-export.lbx
```

Cycle 19 copied the four archived files to `C:\Users\joshd\ptouch-probe`,
verified their SHA-256 hashes, and loaded them twice with the probe. Both runs
returned PT-E550W / ID `26160` for the E550W files and PT-P900W / ID `28464`
for the P900W files, with media ID `261`, LBX version `1.8`, and generator
`ptouch-rs`. The runtime also returned those same values for files converted
from the QR archives by Cycle 18; both converted QR archives were byte-for-byte
identical to the archived originals. The Cycle 17 report of MW-170 / ID `0` /
media `138` is contradicted by these repeated hash-verified runs. Its exact
cause cannot be recovered from the preserved logs. This establishes metadata
returned by `LbxFactory.CreateFromPath`; it does not verify GUI rendering,
editable QR payload after load, or print behavior.

Cycle 21 extended the reflection probe to enumerate `GetPtObjects()`, read
barcode data/protocol and scalar settings, and capture the result of
`GetPrintingImage()` as a BMP. The generated QR archives load as one
`LibLbx.BcdObj` each with protocol `QRCODE`, the expected URL, Model 2, version
3 automatic, MBCS 932, and ECC M. The Code 39 archive loads as one
`LibLbx.BcdObj` with protocol `CODE39`, data `ABC-123`, `ShowCheckDigit=true`,
`ShowChar=true`, and centered readable text. The Code 39 output is 482x128 px
and decodes to `ABC-123W` with `zbarimg`. The raw QR output is 132x128 px; neither
`zbarimg` nor ZXing-C++ decoded it directly, while the Rust 135x128 px output
decoded to the expected URL. Cycle 23 found that ZXing-C++ decodes the Brother
image after rotating it 90 degrees and adding a 4 px white border. zbarimg still
does not decode that normalized image. The rotated and padded derivative is
preserved as `runtime-renders/editor-pt-e550w-qr-decoder-normalized.png`
(SHA-256 `069a79fa7063cf0ab6c5748b219ddc7e596a1fa3c6ee905838f3ae6db226c2bc`).
ZXing-C++ reports version 3, ECC M, and data mask 7 for both the normalized
Brother raster and Rust raster. The payload is recoverable from both, but the
matching mask does not explain their visible pattern differences. Data
segmentation/charset behavior and raster orientation/scaling remain under
investigation.

Cycle 22 changed Rust readable Code 39 text to include the observed `*` start/stop sentinels. The rendered result is preserved as `runtime-renders/rust-pt-e550w-code39-cycle22.png` (581x128 px, SHA-256 `1738433fa446738390b2d40d58129eb748af0ab331d98bd4524d0ce2e36d179d`). It still decodes to `ABC-123W`. The rendered full-label dimensions are not directly comparable to Brother's object-only 482x128 image, so this does not settle geometry parity.

The saved BMP/PNG comparisons are in `runtime-renders/`. In the Code 39
comparison, Brother's raster visibly includes the `*` start/stop delimiters in
human-readable text and fills nearly the full raster width with bars; Rust's
current raster omits those delimiters and keeps quiet zones. Both output
rasters decode to the same payload and check character. QR module patterns
also differ even though the runtime reports matching data, model, version, and
ECC fields.

| Runtime or Rust image | Dimensions | SHA-256 |
|---|---:|---|
| `runtime-renders/editor-pt-e550w-qr.bmp` | 132x128 | `8e75634c3f299c9abbad78759d79e6e8279714766640c42a2d77008ede69530a` |
| `runtime-renders/rust-pt-e550w-qr.png` | 135x128 | `ebc7d77dc7541268898495147624fef2da35e396c3673348ca857d9a5e601bc1` |
| `runtime-renders/editor-pt-e550w-code39.bmp` | 482x128 | `db8d8720d1d6c381aea5a1848479993c5f4bde9d21879196bff8f7fca26ba55a` |
| `runtime-renders/rust-pt-e550w-code39.png` | 581x128 | `b625c475baf97e121a2fd8ef7d9863942500aee7607167b2ac79472fdfe0470d` |

The archived file hashes are:

| File | SHA-256 |
|---|---|
| `qr-probe.ptl` | `e6f086dfece77f9143600053ff3bcb5a7e108b62c60575c47ee09267d28ee5e6` |
| `pt-e550w-export.lbx` | `a1ec2d58bdb7fb21dfb6ec19245e64b3a9bd09a1c093b75e11158160b7163bf4` |
| `pt-p900w-export.lbx` | `58a44b39164a75a0a9eb43f9e510b50a87bdf812e1f718c4baa1d01f463ec2e4` |
| `pt-e550w-qr-export.lbx` | `4b4b8e2d276695c64ccac063571c145ff783d6384685a9893bcc0ef7cc9ca708` |
| `pt-p900w-qr-export.lbx` | `2230fb5b8d343c3067374e670ee23cc4bd0c83b442dffc36d1eb08d8c510fbd1` |

The probe records metadata, supported public object properties, and the
in-memory raster returned by `GetPrintingImage()`. It does not establish the
Editor GUI canvas behavior or physical print output. No printer handle was
opened and no print or setting write was sent.


Cycle 25 reran the read-only object probe on Windows with the archived
`pt-e550w-qr-export.lbx`. The additional scalar properties confirm the
application's QR object selects mixed mode and automatic masking. The matching
version/ECC/mask values from Cycle 24 rule out mask selection as the sole
source of the visible pattern difference. Rust's current QR path calls
`QrSegment::make_segments()`, which returns a single numeric, alphanumeric, or
byte segment; the URL contains lowercase characters and is represented as one
byte segment. Whether Brother's `Mixed` mode splits this payload into multiple
segments remains to be established.


Cycle 26 used a second QR implementation to compare data segmentation. With
version 3, ECC M, and forced mask 7, Python `qrcode` in byte-only mode produced
the exact same 29x29 module matrix as Rust's `qrcodegen` path. Splitting the
URL into byte/alphanumeric/byte/numeric segments changed the matrix and selected
mask 5 automatically. Brother reports `Mode=Mixed` and `Mask=Auto`, but its
decoded mask is 7; this particular four-segment candidate therefore does not
match the Brother mask metadata. The exact segment boundaries and original
bitmap module mapping remain unknown.


Cycle 27 added opt-in `PT_QR_MODE_OVERRIDE` and `PT_QR_DATA_OVERRIDE`
environment controls to the Windows probe. These change the loaded `BcdObj`
property only in memory, call `SetBarcodeProperty`, and then capture
`GetPrintingImage()`; the probe never saves the modified document or opens a
printer. On the URL archive, default `Mode=Mixed` and override `Mode=Binary`
returned byte-identical 132x128 BMPs (SHA-256
`8e75634c3f299c9abbad78759d79e6e8279714766640c42a2d77008ede69530a`). On a
numeric QR archive, default `Mixed`, override `Number`, and override `Binary`
all returned byte-identical BMPs (SHA-256
`f1da64b34aaf93b125d1872b9164b1617dae6087b58dff06dca19074ac61fcb3`). On an
uppercase alphanumeric QR archive, default `Mixed`, override `Binary`, and
override `AlphabetAndNumber` all returned byte-identical BMPs (SHA-256
`4b6b2066148279fe5ea366aea7b8d119c88f333cada4a11a874f09070f54e2ea`). The
reproducible inputs are `qr-numeric-mode-probe.ptl` and
`qr-alphanumeric-mode-probe.ptl`. Changing
the numeric payload in memory to `9876543210987654` changed the returned BMP
hash to `bf0af4fdcad756c1209eb24309a80f24e680d3f97070a7e9bee33c5c6efea1ba`.
This validates that data changes reach the renderer while mode changes across
three payload classes do not alter its raster. It does not establish whether
the mode property affects other render paths or saved documents.


Cycle 29 added optional `PT_QR_CELLSIZE_OVERRIDE` and
`PT_QR_MASK_OVERRIDE` properties to the probe. Changing `CellSize` from M changes
the raster width while the height remains
128 px: XS 66 px, S 99 px, M 132 px, L 165 px, and XL 198 px. The M default
decodes under the rotation-plus-border normalization; the other tested sizes do
not.
Changing `Mask` from Auto to Mask0 changed the 132x128 raster hash to
`4db8e3aed4c4c45136412a08dff4d0fc31574b936a8bfc2d668a87211e7bbaac`; ZXing-C++
decoded the expected URL and reported data mask 0. The cell-size and mask variant BMPs are preserved in `runtime-renders/`.
This demonstrates that data, mask, and cell-size changes reach the raster path,
while tested mode changes do not.
