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
image after a 90-degree Pillow rotation that retained the original canvas (and therefore cropped the rotated rectangle), plus a 4 px white border. zbarimg still
does not decode that normalized image. The rotated and padded derivative is
preserved as `runtime-renders/editor-pt-e550w-qr-decoder-normalized.png`
(SHA-256 `069a79fa7063cf0ab6c5748b219ddc7e596a1fa3c6ee905838f3ae6db226c2bc`).
ZXing-C++ reports version 3, ECC M, and data mask 7 for both the normalized
Brother raster and Rust raster. The payload is recoverable from both. The visible matrix match shows the
native-size appearance difference comes
from pixel scale and vertical cropping. Brother reports `Mode=Mixed`; the exact
mode-setting semantics and physical print behavior remain open.

Cycle 22 changed Rust readable Code 39 text to include the observed `*` start/stop sentinels. The rendered result is preserved as `runtime-renders/rust-pt-e550w-code39-cycle22.png` (581x128 px, SHA-256 `1738433fa446738390b2d40d58129eb748af0ab331d98bd4524d0ce2e36d179d`). It still decodes to `ABC-123W`. The rendered full-label dimensions are not directly comparable to Brother's object-only 482x128 image, so this does not settle geometry parity.

The saved BMP/PNG comparisons are in `runtime-renders/`. In the Code 39
comparison, Brother's raster visibly includes the `*` start/stop delimiters in
human-readable text and fills nearly the full raster width with bars; Rust's
current raster omits those delimiters and keeps quiet zones. Both output
rasters decode to the same payload and check character. The QR pattern appeared different at native size. Cycle 33 aligned each Brother
cell size to Rust's exact version 3, ECC M, mask 7 matrix: every visible QR pixel
matched. The raster is vertically offset and clipped inside the fixed 128 px
bitmap, with the clipped rows varying by cell size.

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


Cycle 42 tested a single 50-character lowercase byte payload at all four ECC
levels. The PTL fixtures and captured P900W BMPs are `qr-ecc-mask-probe-*.ptl`
and `runtime-renders/editor-pt-p900w-qr-ecc-*-cycle42.bmp`. The runtime
reported ECC L/M/Q/H and versions 3/4/5/6. ZXing decoded the L, M, and Q
captures as masks 2, 2, and 4. The H/V6 image is 388x320 px, so the bottom
rows are clipped and ZXing cannot decode the full symbol. Its intact top-left
format information still identifies ECC H and mask 4 with no BCH errors. Run
`uv run --with pillow python docs/re/windows-lbx-runtime/extract-qr-format-probes.py`
from the repository root to read the format information from all four BMPs.
This broadens the observed mask behavior across ECC levels, but does not
resolve the two Cycle 41 exceptions or identify Brother's internal algorithm.
No GUI canvas or physical print was tested.


Cycle 43 evaluates whether mask scoring includes a one-module white border.
`analyze-qr-mask-margins.py` regenerates all eight forced-mask matrices for the
seven Cycle 40 byte-only probes and the two residual cases, then compares the
libqrencode-style score with and without that border. Plain scoring matches the
Brother masks in seven of nine cases. Border scoring matches six of nine: it
selects Brother's mask 1 for the 55-byte repetitive payload, but changes two
previously correct Cycle 40 predictions and still selects mask 3 for
`ABC123abc789` where Brother selects 0. The script reports all eight masks'
N1/N2/N3/N4 component scores for both residuals. This does not support changing
the production scorer.


Cycle 44 uses the `PT_QR_MASK_OVERRIDE` in-memory control to render masks 0-7
for `ABC123abc789` (version 1/ECC M) and the 55-byte repetitive lowercase
payload (version 4/ECC M). The BMP captures are named
`runtime-renders/editor-pt-p900w-qr-mask-override-*-cycle44.bmp`. Run
`uv run --with pillow python docs/re/windows-lbx-runtime/compare-qr-all-mask-candidates.py`
to compare each image with Rust's fixed-mask matrix. All 16 captures have zero
pixel mismatches across their complete symbol grids. The payload, segmentation,
version, and ECC therefore match for every forced mask; only automatic mask
selection remains different. These are in-memory renders, with no printer
handle, settings write, or print job.


Cycle 45 tests whether a different weight on the 2x2-block penalty N2 explains
the two residual mask choices. `analyze-qr-n2-weights.py` regenerates all eight
fixed-mask matrices for 19 captured controls and sweeps integer N2 weights 1-8
while holding N1, N3, and N4 fixed. The baseline 1x score matches 17 of 19
Brother selections; weights 2-8 match 16, 13, 13, 12, 12, 10, and 10. Neither
residual case matches at any tested weight. This rejects a simple integer N2
rescaling, not other scoring algorithms.


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
decodes only under the crop-preserving rotation-plus-border transform; the other tested sizes do
not.
Changing `Mask` from Auto to Mask0 changed the 132x128 raster hash to
`4db8e3aed4c4c45136412a08dff4d0fc31574b936a8bfc2d668a87211e7bbaac`; ZXing-C++
decoded the expected URL and reported data mask 0. The cell-size and mask variant BMPs are preserved in `runtime-renders/`.
This demonstrates that data, mask, and cell-size changes reach the raster path,
while tested mode changes do not.


Cycle 30 compared the dimensions with the QR version and enum metadata. The
CellSize enum values are XS=2, S=3, M=4, L=5, and XL=6. The observed raster
widths are exactly 33 times those values. Version 3 has a 29-module side, so
this is consistent with each enum value acting as pixels per module plus four
cell units of horizontal margin. That mapping remains an inference because the
proprietary raster API does not expose module coordinates. For L and XL, the
inferred 145 px and 174 px symbol sides exceed the fixed 128 px bitmap height;
the black bounds reach the bitmap edge, consistent with clipping. XS and S also
failed decoding despite fitting within 128 px, so their failure cause remains
unknown. No physical print result is inferred.


Cycle 31 repeated decode attempts for all five CellSize captures. ZXing-C++ was
tested with all right-angle rotations using Pillow default `expand=False`,
nearest-neighbor scaling at 1x/2x/4x, and white borders of 0/4/8/16/32/64 px
(scaled with the image). Only M decoded. Those quarter-turn transforms preserve
the old canvas and crop the rotated rectangle. A second sweep using
`expand=True` retained the full rotated image, but none of the sizes decoded,
including M. zbar decoded none of the five captures in either transform family.
The successful M decode therefore depends on the crop-preserving transform; the
role of that crop remains unexplained, and decoder failure does not establish
that XS/S/L/XL contain invalid QR matrices.


Cycles 33-34 add `compare-qr-matrix.py` and `qr-v3-mask7.matrix`. The matrix is
the Rust `qrcodegen` output for the same URL at version 3, ECC M, mask 7. Run
the comparison with Pillow installed:

```sh
uv run --with pillow python docs/re/windows-lbx-runtime/compare-qr-matrix.py
```

It reports zero pixel mismatches across all visible E550W matrix pixels and
across the entire P900W symbol. For E550W cell size n=2..6, the measured grid
origin is x=2n, y=2n-14. The visible module rows are XS 5-28, S 2-28, M 1-28,
L 0-26, and XL 0-21. The smaller sizes lose more leading rows, while L/XL also
lose trailing rows at the fixed 128 px raster edge. The P900W raster is 292x320
px with cell pitch 8 and grid origin (30,20), preserving all 29 rows. ZXing-C++
and zbar decode the P900W BMP directly as the expected URL, version 3, ECC M,
mask 7. Its BMP SHA-256 is
`7c201ed7b3b0e85703e75ba75c929424ac8c0412103bc64f652cd976b3f4f0ed`. This
establishes in-memory matrix parity for the tested E550W/P900W QR exports; GUI
canvas and physical print behavior remain unverified.


Cycle 35 compares three payload families on both target runtimes. The mixed uppercase/numeric payload `HELLO WORLD 1234567890` is version 1 in Brother, while Rust's default segmentation chooses version 2. An explicit alphanumeric-prefix/numeric-suffix version 1 mask 3 matrix matches every pixel in both models' captured rasters. A 60-character lowercase payload is version 4 in both; the P900W decoder reports mask 1 while Rust auto selects mask 2, and forcing Rust mask 1 matches all visible E550W pixels. The numeric payload is version 1 on both and decodes on P900W, but its exact module-grid alignment is unresolved. Fixture LBX files, extracted BMPs, and candidate matrices are in this directory. No printer was opened and nothing was printed.

Cycle 36 corrected the Cycle 35 numeric comparison. The matrix dump helper had used qrcodegen's convenience encoder, which silently boosted the requested Medium ECC to Quartile for the 20-digit payload. With ECC explicitly kept at Medium, Rust's version 1/mask 4 matrix matches every Brother pixel for E550W (pitch 4, origin 8,-6) and P900W (pitch 8, origin 30,20). Production automatic-version rendering and LBX import now disable ECC boosting. The long lowercase version 4 case remains a mask-selection difference: Brother uses mask 1 while Rust automatic selection uses mask 2.


Cycle 37 tests more P900W version 4/ECC M payloads. Three repetitive byte-only strings (50, 55, and 62 bytes) all decode as mask 1, while Rust automatic mask selection returns 2, 2, and 0. Forcing mask 1 matches every raster pixel. A diverse 60-character lowercase/digit payload decodes as mask 2; Rust's default whole-byte segment uses the same mask but differs in modules. A byte-prefix/numeric-suffix candidate at mask 2 matches every pixel, which is evidence Brother's Mixed mode splits trailing digits. The comparison script is `compare-qr-mask-probes.py`. The exact Brother mask scoring rule remains unknown.

Cycle 38 extends the mixed-mode probes with five short strings. P900W reports version 1/ECC M for each. Candidate segment layouts reproduce every pixel with these boundaries: bytes `hello ` then numeric `1234567890`; numeric `1234567890` then bytes `hello`; alphanumeric `HELLO WORLD ` then numeric `1234567890`; bytes `abc` then alphanumeric `123DEF456`; and alphanumeric `ABC123` then bytes `abc789`. Decoder masks are 0, 3, 3, 3, and 0. Rust's current whole-string path needs version 2 for the first three, and yields different version 1 grids for the last two. Run `uv run --with pillow python docs/re/windows-lbx-runtime/compare-qr-segment-probes.py` to compare the checked-in candidate matrices and captures. This evidence does not yet prove the segment optimizer for arbitrary inputs.

Cycle 39 implements a shared version-aware segmenter for numeric, alphanumeric, and byte runs. It uses each QR version group's mode count widths when selecting segments, keeps the requested ECC level, and is used by both QR rendering and LBX import. Public renderer tests now select version 1 for the observed mixed strings, including fixed version 1; the LBX import/render test also accepts the fixed version 1 payload. A P900W-captured version 4/ECC M/mask 2 module grid is checked in as a renderer golden and matches the 60-character mixed byte/numeric payload exactly. The short version 1 payloads still use different Rust-selected masks from Brother, so their final matrices differ until mask parity is addressed.

Cycle 40 tests whether the remaining mask mismatch comes from Brother omitting
qrcodegen's N3 finder-pattern penalty. Seven lowercase-only version 4/ECC M
captures match the Rust matrix exactly at Brother's decoded mask, holding byte
encoding constant. Standard scoring predicts Brother's mask in three cases,
omitting N3 predicts two, and neither predicts two. A single N3 weight change
does not explain the data, so production scoring remains unchanged. The
per-probe scores and comparison command are in qr-mask-score-probes.md.
A Hopper inspection of the native lbxcore.Win32.dll identified QR style
settings serialization but did not locate the mask selector.

Cycle 41 compares alternative mask implementations with the exact captured
matrices. A libqrencode-style score selects the Brother mask for 17 of 19
analyzed captures and reproduces every module in the seven Cycle 40 version 4
byte-only grids. Two cases remain different: `ABC123abc789` and the
55-character repetitive lowercase payload. The Rust renderer now uses this
score by default. Validation covers the library's in-memory raster only; it
does not establish GUI canvas or physical print behavior. The scoring source
comparison is recorded in qr-mask-score-probes.md.
