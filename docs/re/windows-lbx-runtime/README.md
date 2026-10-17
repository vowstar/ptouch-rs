# Windows LBX runtime probe

This probe loads LBX archives with the `LibLbx.DotNet.dll` shipped with Brother
P-touch Editor. It calls the same `LibLbx.Initialize` overload as the Editor,
then reads document metadata through `LbxFactory.CreateFromPath`. It does not
save files, open a printer handle, or print.

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

The archived file hashes are:

| File | SHA-256 |
|---|---|
| `qr-probe.ptl` | `e6f086dfece77f9143600053ff3bcb5a7e108b62c60575c47ee09267d28ee5e6` |
| `pt-e550w-export.lbx` | `a1ec2d58bdb7fb21dfb6ec19245e64b3a9bd09a1c093b75e11158160b7163bf4` |
| `pt-p900w-export.lbx` | `58a44b39164a75a0a9eb43f9e510b50a87bdf812e1f718c4baa1d01f463ec2e4` |
| `pt-e550w-qr-export.lbx` | `4b4b8e2d276695c64ccac063571c145ff783d6384685a9893bcc0ef7cc9ca708` |
| `pt-p900w-qr-export.lbx` | `2230fb5b8d343c3067374e670ee23cc4bd0c83b442dffc36d1eb08d8c510fbd1` |

The probe only records metadata returned by `LbxFactory.CreateFromPath`. It does
not establish visual rendering parity, broader object support, or print
behavior. No printer handle was opened and no print or setting write was sent.
