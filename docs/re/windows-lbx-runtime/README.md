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

The sample layout contains one absolute text object with `PARITY PROBE`.
Generate its target-specific archives from the repository root:

```sh
cargo run -p ptouch-cli -- convert docs/re/windows-lbx-runtime/probe.ptl \
  --output docs/re/windows-lbx-runtime/pt-e550w-export.lbx --printer pt-e550w
cargo run -p ptouch-cli -- convert docs/re/windows-lbx-runtime/probe.ptl \
  --output docs/re/windows-lbx-runtime/pt-p900w-export.lbx --printer pt-p900w
```

Build and run the probe on Windows from this directory. Change the `editor`
constant in `Program.cs` if P-touch Editor is installed elsewhere.

```powershell
dotnet publish -c Release -r win-x86 --self-contained true
.\bin\Release\net8.0\win-x86\publish\LbxProbe.exe `
  .\pt-e550w-export.lbx .\pt-p900w-export.lbx
```

Both archives generated from the simple text layout loaded successfully. The
Brother runtime reported LBX version `1.8`, generator `ptouch-rs`, PT-E550W
printer ID `26160`, PT-P900W printer ID `28464`, and media ID `261` for both.
Their SHA-256 hashes are:

| File | SHA-256 |
|---|---|
| `pt-e550w-export.lbx` | `a1ec2d58bdb7fb21dfb6ec19245e64b3a9bd09a1c093b75e11158160b7163bf4` |
| `pt-p900w-export.lbx` | `58a44b39164a75a0a9eb43f9e510b50a87bdf812e1f718c4baa1d01f463ec2e4` |

This verifies loading and target/media metadata only. It does not establish
visual rendering parity, broader object support, or print behavior.
