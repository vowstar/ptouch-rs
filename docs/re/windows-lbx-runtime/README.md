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

The first Cycle 16 run recorded target IDs and media metadata for the simple
text pair. Repeating the probe on the same machine and the same archived text
files in Cycle 17 returned the Editor's default Brother MW-170 model, stored
printer ID `0`, and media ID `138`. The QR pair returned the same fallback.
The current run also reported LBX version `1.10` and generator
`P-touch Editor 6.12.1.0 Windows` for all four files. This does not prove target
metadata acceptance or QR-object preservation. The discrepancy between the
two runs is unresolved.

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
