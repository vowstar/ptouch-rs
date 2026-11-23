#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

export BUILD_PROFILE="${BUILD_PROFILE:-release}"
export VERSION
VERSION="$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"])')"
case "${RUST_TARGET}:${PACKAGE_ARCH}:${RPM_ARCH}" in
  x86_64-unknown-linux-gnu:amd64:x86_64|aarch64-unknown-linux-gnu:arm64:aarch64) ;;
  *) echo "Inconsistent Linux target/package architecture" >&2; exit 1 ;;
esac
binary_dir="target/${RUST_TARGET}/${BUILD_PROFILE}"
python3 scripts/linux-elf.py --target "$RUST_TARGET" "$binary_dir/ptouch" "$binary_dir/ptouch-gui"
go install github.com/goreleaser/nfpm/v2/cmd/nfpm@v2.47.0
nfpm="$(go env GOPATH)/bin/nfpm"
mkdir -p dist
deb="$PWD/dist/ptouch_${VERSION}_${PACKAGE_ARCH}.deb"
rpm="$PWD/dist/ptouch-${VERSION}.${RPM_ARCH}.rpm"
"$nfpm" package -f packaging/nfpm.yaml -p deb -t "$deb"
"$nfpm" package -f packaging/nfpm.yaml -p rpm -t "$rpm"
test "$(dpkg-deb --field "$deb" Architecture)" = "$PACKAGE_ARCH"
test "$(rpm -qp --queryformat '%{ARCH}' "$rpm")" = "$RPM_ARCH"
dpkg-deb --field "$deb" Depends | grep -F 'libc6 (>= 2.17)'
rpm -qp --requires "$rpm" | grep -Fx 'libc.so.6(GLIBC_2.17)(64bit)'
extracted="$(mktemp -d)"
trap 'rm -rf "$extracted"' EXIT
mkdir "$extracted/deb" "$extracted/rpm"
dpkg-deb --extract "$deb" "$extracted/deb"
(cd "$extracted/rpm" && rpm2cpio "$rpm" | cpio --quiet -id --no-absolute-filenames)
for format in deb rpm; do
  payload="$extracted/$format/usr"
  for binary in ptouch ptouch-gui; do
    test -x "$payload/bin/$binary"
    cmp "$binary_dir/$binary" "$payload/bin/$binary"
  done
  cmp data/io.github.vowstar.ptouch-gui.desktop "$payload/share/applications/io.github.vowstar.ptouch-gui.desktop"
  cmp data/io.github.vowstar.ptouch-gui.svg "$payload/share/icons/hicolor/scalable/apps/io.github.vowstar.ptouch-gui.svg"
  cmp data/udev/20-usb-ptouch-permissions.rules "$payload/lib/udev/rules.d/20-usb-ptouch-permissions.rules"
done
