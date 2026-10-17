---
name: ptset-brother-printer-project
description: "Rust reimplementation of Brother Printer Setting Tool for the user's PT-P900W (C:\\Users\\joshd\\dev\\ptset) and where the RE evidence lives"
metadata:
  node_type: memory
  type: project
  originSessionId: 329fa747-5034-491b-a55d-4cd85b900bea
  modified: 2026-10-16T03:01:29.062Z
---

User is reverse engineering Brother Printer Setting Tool 1.6.0142 (DeviceSettings\PT900\brdvset.exe + ptfilo.dll) to build a Rust CLI `ptset` at C:\Users\joshd\dev\ptset for their PT-P900W (USB serial 000C6Z514537, series 0x30 / model 0x6F). Started 2026-10-15.

Recovered so far: transport = USBPRINT interface via SetupDi + CreateFile/WriteFile/ReadFile; settings are ISettingItem subclasses in brdvset.exe (vtable slot 11 = SET builder, 12 = GET, 13 = reply len, 14 = reply parser). GET commands verified live; SET not yet implemented. Value scales for auto power off/on and print log still needed mapping against the Brother tool UI.

**Why:** user wants a Rust replacement for the Windows-only GUI tool.
**How to apply:** Ghidra decompile output and helper scripts were in the session scratchpad (may be gone); re-run Ghidra headless (C:\Tools\ghidra_12.1.4_PUBLIC, JDK 21) if needed. Never send SET/write commands to the printer without showing the user the bytes first. Related: [[user-dev-environment]]
