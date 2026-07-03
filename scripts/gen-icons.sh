#!/usr/bin/env bash
# Regenerate every raster app icon from the single SVG source.
#
# The SVG (data/io.github.vowstar.ptouch-gui.svg) is the source of truth; the
# files below are generated artifacts, committed so normal builds stay hermetic
# (no rasterizer needed to build). Run this after changing the SVG, then commit
# the result. CI verifies the committed rasters still match the SVG.
#
# Requires: rsvg-convert (librsvg), magick (ImageMagick), python3.
#
# Outputs:
#   crates/ptouch-gui/assets/icon.png     256px runtime window icon (include_bytes!)
#   data/windows/ptouch-gui.ico           multi-size Windows icon (build.rs embeds it)
#   data/macos/ptouch-gui.icns            macOS bundle icon (cargo-bundle uses it)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
svg="$root/data/io.github.vowstar.ptouch-gui.svg"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

sizes="16 32 48 64 128 256 512 1024"
for s in $sizes; do
  rsvg-convert -w "$s" -h "$s" "$svg" -o "$tmp/icon_$s.png"
done

# Runtime window icon, embedded in the binary.
install -Dm644 "$tmp/icon_256.png" "$root/crates/ptouch-gui/assets/icon.png"

# Windows .ico (multi-size).
mkdir -p "$root/data/windows"
magick "$tmp/icon_16.png" "$tmp/icon_32.png" "$tmp/icon_48.png" \
       "$tmp/icon_64.png" "$tmp/icon_128.png" "$tmp/icon_256.png" \
       "$root/data/windows/ptouch-gui.ico"

# macOS .icns. ImageMagick cannot write a valid ICNS container, so pack the PNG
# chunks directly (the element types modern macOS reads).
mkdir -p "$root/data/macos"
python3 - "$tmp" "$root/data/macos/ptouch-gui.icns" <<'PY'
import struct, sys, os
srcdir, out = sys.argv[1], sys.argv[2]
mapping = [("icp4", 16), ("icp5", 32),
           ("ic07", 128), ("ic08", 256), ("ic09", 512), ("ic10", 1024),
           ("ic11", 32), ("ic12", 64), ("ic13", 256), ("ic14", 512)]
chunks = b""
for ostype, size in mapping:
    with open(os.path.join(srcdir, f"icon_{size}.png"), "rb") as f:
        data = f.read()
    chunks += ostype.encode("ascii") + struct.pack(">I", len(data) + 8) + data
with open(out, "wb") as f:
    f.write(b"icns" + struct.pack(">I", len(chunks) + 8) + chunks)
PY

echo "Regenerated icons from $svg"
