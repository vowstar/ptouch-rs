# ptouch-rs

Rust tool for Brother P-Touch USB label printers. CLI and GUI.

![ptouch-gui screenshot](https://github.com/user-attachments/assets/b18ba04d-0526-43f8-ad40-8ca29b5cb280)

## Features

- Print text labels with custom font, size, alignment and rotation
- Print images (PNG, JPEG, GIF, BMP, TIFF, WebP, SVG, and more)
- Compose multi-element labels (text + image + cut mark + padding)
- Save and reload designs as self-contained `.ptl` layout files (images
  embedded), then print them from the GUI or CLI
- Template layouts with `{{name}}` placeholders and batch-print from a CSV
- Chain print and multi-copy support
- GUI with live preview, zoom, and drag-and-drop element reordering
- Export to image (PNG, JPEG, BMP, GIF, TIFF, WebP) without a printer connected
- Feed and cut tape without printing

## Supported Printers

PT-9200DX, PT-2300, PT-2420PC, PT-9500PC, PT-9700PC, PT-2450PC, PT-18R,
PT-1950, PT-2700, PT-1230PC, PT-2430PC, PT-2730, PT-H500, PT-E500, PT-E550W,
PT-P700, PT-P750W, PT-D410, PT-D450, PT-D460BT, PT-D600, PT-D610BT,
PT-P710BT, PT-E310BT, PT-E560BT and more.

Tape widths: 3.5mm, 6mm, 9mm, 12mm, 18mm, 24mm, 36mm.

## Building

Requires Rust stable toolchain.

**Linux** (libusb + udev):

```sh
sudo apt install libusb-1.0-0-dev libudev-dev   # Debian/Ubuntu
sudo pacman -S libusb                            # Arch
sudo emerge dev-libs/libusb                      # Gentoo
cargo build --release --workspace
```

**macOS / Windows**: no extra dependencies.

```sh
cargo build --release --workspace
```

Binaries: `target/release/ptouch` (CLI), `target/release/ptouch-gui` (GUI).

libusb is compiled in statically (`rusb` vendored), so the binaries carry no
external libusb dependency.

## Prebuilt Packages

Each release publishes ready-to-use downloads on the
[releases page](https://github.com/vowstar/ptouch-rs/releases):

- **Linux**: `.deb` and `.rpm` that install the CLI, the GUI, the desktop entry,
  the icon, and the udev rule (the post-install step reloads udev), plus the raw
  binaries.
- **macOS**: `ptouch-gui-macos-arm64.app.zip` (a `.app` bundle with the icon)
  and the raw `ptouch` CLI binary. The app is unsigned, so on first launch
  right-click the app and choose Open, or run
  `xattr -dr com.apple.quarantine ptouch-gui.app`.
- **Windows**: `ptouch.exe` and `ptouch-gui.exe` (the icon is embedded).

## GUI

```sh
ptouch-gui
```

- Live label preview with zoom
- Add/edit/reorder text, images, cut marks, padding
- Free-angle text rotation with auto font sizing
- Mirror the whole label or a single element (horizontal/vertical)
- Tape width selection
- Save/Open layout (`.ptl`) with images embedded for portability
- Print to connected printer or export to image file
- Feed and cut tape without printing

## CLI Usage

```sh
# Print text
ptouch print "Hello World"

# Multi-line text
ptouch print "Line 1" "Line 2"

# Print with options
ptouch print "Label" -f "DejaVu Sans" -s 32 -a center

# Print image (PNG, JPEG, BMP, SVG, etc.)
ptouch print -i logo.png

# Text + image + cut mark
ptouch print "Name" -i photo.png -c

# Mirror the whole label left-right (e.g. clear tape read from the back)
ptouch print "MIRROR" --flip-h

# Export to image file (no printer needed, format from extension)
ptouch print "Preview" -o label.png -w 76
ptouch print "Preview" -o label.bmp -w 76

# Print a layout designed in the GUI (images are embedded in the file)
ptouch print --layout label.ptl

# Render a layout to an image without a printer (uses the saved tape width)
ptouch print --layout label.ptl -o label.png

# Show printer info
ptouch info

# List supported models
ptouch list
```

### Layout templates and batch printing

Text in a layout may contain `{{name}}` placeholders. Fill them per print, or
drive a batch from a CSV file.

```sh
# See which placeholders a layout declares
ptouch print --layout badge.ptl --list-vars

# Fill placeholders for a single label
ptouch print --layout badge.ptl --set name=Alice --set id=A001

# One label per CSV row; the header row names the placeholders
ptouch print --layout badge.ptl --csv people.csv

# Batch to image files instead of a printer ({n} is the row number)
ptouch print --layout badge.ptl --csv people.csv -o 'badge-{n}.png'

# CSV from stdin, with a constant value applied to every row
cat people.csv | ptouch print --layout badge.ptl --csv - --set dept=Eng
```

### Print options

| Flag | Long | Description |
|------|------|-------------|
| | `TEXT...` | Text lines (max 4) |
| `-l` | `--layout` | Print a saved layout file (.ptl); overrides content flags |
| | `--set` | Set a layout placeholder, `KEY=VALUE` (repeatable) |
| | `--csv` | Batch-print one label per CSV row (`-` for stdin) |
| | `--list-vars` | List the placeholders a layout declares, then exit |
| | `--allow-missing` | Render placeholders with no value as blank |
| `-i` | `--image` | Image file path |
| `-o` | `--output` | Export to image file instead of printing |
| `-f` | `--font` | Font name (default: DejaVuSans) |
| `-s` | `--size` | Font size in points (auto if omitted) |
| `-m` | `--margin` | Top/bottom margin in pixels |
| `-a` | `--align` | Text alignment: left, center, right |
| `-w` | `--tape-width` | Force tape width in pixels (with `-o`) |
| `-c` | `--cut` | Add cut mark |
| `-p` | `--pad` | Add padding in pixels |
| | `--flip-h` | Mirror the whole label left-right (horizontal) |
| | `--flip-v` | Mirror the whole label top-bottom (vertical) |
| | `--chain` | Skip final feed/cut (chained labels) |
| | `--precut` | Cut before the label |
| | `--binarize` | Binarization: auto, threshold, dither |
| | `--copies` | Number of copies |
| | `--timeout` | Printer timeout in seconds |
| | `--debug` | Enable debug output |

## USB Permissions (Linux)

The `.deb`/`.rpm` install and load this rule for you. To do it manually, copy
the udev rules file:

```sh
sudo cp data/udev/20-usb-ptouch-permissions.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
```

## Desktop Integration (Linux)

The `.deb`/`.rpm` already install these. To do it manually, install the desktop
entry and icon so `ptouch-gui` appears in your application menu:

```sh
sudo install -Dm644 data/io.github.vowstar.ptouch-gui.desktop \
  /usr/share/applications/io.github.vowstar.ptouch-gui.desktop
sudo install -Dm644 data/io.github.vowstar.ptouch-gui.svg \
  /usr/share/icons/hicolor/scalable/apps/io.github.vowstar.ptouch-gui.svg
sudo gtk-update-icon-cache -f /usr/share/icons/hicolor 2>/dev/null || true
```

## Application Icon

`data/io.github.vowstar.ptouch-gui.svg` is the single source of truth. The
raster forms are generated from it by `scripts/gen-icons.sh` (needs
`rsvg-convert`, `magick`, and `python3`) and committed so normal builds need no
rasterizer:

- `crates/ptouch-gui/assets/icon.png` embedded as the runtime window icon
- `data/windows/ptouch-gui.ico` embedded into the `.exe` by `build.rs`
- `data/macos/ptouch-gui.icns` used by `cargo bundle` for the macOS `.app`

Regenerate after editing the SVG, then commit the result. CI checks that the
committed rasters still match the SVG.

## USB Driver (Windows)

Communication goes through libusb, which on Windows can only reach a device
that uses the WinUSB driver. Out of the box Windows binds the printer to its
own driver (and the official Brother driver does the same), so `ptouch info`
reports `Device not found` until you switch it. See issue
[#4](https://github.com/vowstar/ptouch-rs/issues/4).

1. Download [Zadig](https://zadig.akeo.ie/).
2. Plug in the printer, then choose `Options > List All Devices`.
3. Select your printer in the list (Brother VID `04F9`).
4. Pick `WinUSB` as the target driver and click `Replace Driver`.
5. Run `ptouch info` again.

After this the normal Brother software no longer sees the printer. Undo it any
time by uninstalling or rolling back the driver in Device Manager.

## USB Driver (macOS)

No driver replacement is needed. Install libusb and it works directly:

```sh
brew install libusb
```

If claiming the device fails with a busy or access error, make sure the
printer is not added as a print queue in System Settings.

## Project Structure

```
crates/
  ptouch-core/    -- USB transport, protocol, device/tape tables
  ptouch-render/  -- Bitmap, text rendering, image loading, raster
  ptouch-cli/     -- CLI binary
  ptouch-gui/     -- GUI binary (egui)
```

## License

This project's printer protocol and device layer is derived from
[ptouch-print](https://git.familie-radermacher.ch/linux/ptouch-print.git) by
Dominic Radermacher and the ptouch-print contributors, which is licensed under
the GPLv3. Thanks to them for the reverse engineering that made this possible.

Because of that, the project as a whole and the distributed `ptouch` and
`ptouch-gui` binaries are licensed **GPL-3.0-or-later** (see [LICENSE](LICENSE)).

Per-directory licensing (each source file carries an SPDX header):

| Path | License | Notes |
|------|---------|-------|
| `crates/ptouch-core/` | GPL-3.0-or-later | Device table, flags, status protocol, command construction. Derived from ptouch-print. |
| `crates/ptouch-render/src/raster.rs` | GPL-3.0-or-later | Raster bit-packing. Derived from ptouch-print. |
| `crates/ptouch-render/` (other files) | MIT | Bitmap, text, image loading, composition. Original work. |
| `crates/ptouch-cli/` | MIT | Original work. |
| `crates/ptouch-gui/` | MIT | Original work. |

The MIT-licensed files are reusable on their own under the MIT license (see
[LICENSE-MIT](LICENSE-MIT)). Any program that links `ptouch-core`, including the
binaries in this repository, is covered by the GPLv3. See [NOTICE](NOTICE) for
attribution details.
