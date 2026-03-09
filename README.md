# ptouch-rs

Rust tool for Brother P-Touch USB label printers. CLI and GUI.

![ptouch-gui screenshot](https://github.com/user-attachments/assets/b18ba04d-0526-43f8-ad40-8ca29b5cb280)

## Features

- Print text labels with custom font, size, alignment and rotation
- Print images (PNG, JPEG, GIF, BMP, TIFF, WebP, SVG, and more)
- Compose multi-element labels (text + image + cut mark + padding)
- Chain print and multi-copy support
- GUI with live preview, zoom, and drag-and-drop element reordering
- Export to PNG without a printer connected

## Supported Printers

PT-9200DX, PT-2300, PT-2420PC, PT-2450PC, PT-18R, PT-1950, PT-2700,
PT-1230PC, PT-2430PC, PT-2730, PT-H500, PT-E500, PT-E550W, PT-P700,
PT-P750W, PT-D410, PT-D450, PT-D460BT, PT-D600, PT-D610BT, PT-P710BT,
PT-E310BT, PT-E560BT and more.

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

## GUI

```sh
ptouch-gui
```

- Live label preview with zoom
- Add/edit/reorder text, images, cut marks, padding
- Free-angle text rotation with auto font sizing
- Tape width selection
- Print to connected printer or export to PNG

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

# Export to PNG (no printer needed)
ptouch print "Preview" -o label.png -w 76

# Show printer info
ptouch info

# List supported models
ptouch list
```

### Print options

| Flag | Long | Description |
|------|------|-------------|
| | `TEXT...` | Text lines (max 4) |
| `-i` | `--image` | Image file path |
| `-o` | `--output` | Export to PNG instead of printing |
| `-f` | `--font` | Font name (default: DejaVuSans) |
| `-s` | `--size` | Font size in points (auto if omitted) |
| `-m` | `--margin` | Top/bottom margin in pixels |
| `-a` | `--align` | Text alignment: left, center, right |
| `-w` | `--tape-width` | Force tape width in pixels (with `-o`) |
| `-c` | `--cut` | Add cut mark |
| `-p` | `--pad` | Add padding in pixels |
| | `--chain` | Skip final feed/cut (chained labels) |
| | `--binarize` | Binarization: auto, threshold, dither |
| | `--copies` | Number of copies |
| | `--debug` | Enable debug output |

## USB Permissions (Linux)

Copy the udev rules file:

```sh
sudo cp udev/20-usb-ptouch-permissions.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
```

## Project Structure

```
crates/
  ptouch-core/    -- USB transport, protocol, device/tape tables
  ptouch-render/  -- Bitmap, text rendering, image loading, raster
  ptouch-cli/     -- CLI binary
  ptouch-gui/     -- GUI binary (egui)
```

## License

MIT -- see [LICENSE](LICENSE).
