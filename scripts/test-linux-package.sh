#!/bin/sh
# SPDX-License-Identifier: MIT
set -eu

if [ "$1" != --smoke ]; then
    package=$1
    mode=$2
    version=$3
    case "$package" in
        *.deb)
            export DEBIAN_FRONTEND=noninteractive
            apt-get -o Acquire::Retries=3 update
            apt-get -y --no-install-recommends install "$package" \
                xvfb xauth fonts-dejavu-core libgl1-mesa-dri python3 util-linux
            if [ "$mode" = wayland ]; then
                apt-get -y --no-install-recommends install weston
            fi
            ;;
        *.rpm)
            dnf -y --setopt=install_weak_deps=False install "$package" \
                xorg-x11-server-Xvfb xorg-x11-xauth dejavu-sans-fonts mesa-dri-drivers \
                mesa-libEGL python3 util-linux
            ;;
        *) echo "Unsupported package: $package" >&2; exit 1 ;;
    esac
    smoke_home=$(mktemp -d)
    trap 'rm -rf "$smoke_home"' EXIT
    chown nobody "$smoke_home"
    runuser -u nobody -- env HOME="$smoke_home" sh "$0" --smoke "$mode" "$version"
    exit
fi

mode=$2
version=$3
echo "Running packaged $version on $(uname -m), $(getconf GNU_LIBC_VERSION), $mode"
unset DISPLAY WAYLAND_DISPLAY WAYLAND_SOCKET
test "$(ptouch --version)" = "ptouch $version"
test "$(ptouch-gui --version)" = "ptouch-gui $version"
ptouch --help > /dev/null
ptouch-gui --help > /dev/null
ptouch doctor --json > "$HOME/doctor.json"
ptouch print --layout /source/examples/patch-panel-24-ports.ptl --output "$HOME/test.png"
python3 - <<'PY'
import json
import os
import platform
from pathlib import Path
import struct

home = Path(os.environ["HOME"])
report = json.loads((home / "doctor.json").read_text())
assert report["process_arch"] == platform.machine(), report
assert report["schema_version"] == 1 and not report["probe"] and not report["errors"], report
with (home / "test.png").open("rb") as png:
    assert png.read(16) == b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR"
    assert struct.unpack(">II", png.read(8)) == (3025, 76)
PY

export LIBGL_ALWAYS_SOFTWARE=1
if [ "$mode" = wayland ]; then
    XDG_RUNTIME_DIR=$(mktemp -d)
    export XDG_RUNTIME_DIR WAYLAND_DISPLAY=ptouch-smoke
    weston --backend=headless-backend.so --use-gl --socket="$WAYLAND_DISPLAY" > "$HOME/weston.log" 2>&1 &
    compositor=$!
    trap 'kill "$compositor" 2>/dev/null || true; rm -rf "$XDG_RUNTIME_DIR"' EXIT
    if ! timeout 30 sh -c 'until test -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"; do sleep 0.2; done'; then
        cat "$HOME/weston.log" >&2
        exit 1
    fi
    timeout 75 ptouch-gui --smoke-test > "$HOME/gui.log" 2>&1 || { cat "$HOME/gui.log"; exit 1; }
else
    xvfb-run -a -s '-screen 0 1280x1024x24 +extension GLX' \
        timeout 75 ptouch-gui --smoke-test > "$HOME/gui.log" 2>&1 || { cat "$HOME/gui.log"; exit 1; }
fi
grep -Fx PTOUCH_GUI_SMOKE_OK "$HOME/gui.log"
