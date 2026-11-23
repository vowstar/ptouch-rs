#!/usr/bin/env bash
# SPDX-License-Identifier: MIT
set -euo pipefail

target=${1:?supply the GNU Linux Rust target}
zig_version=0.15.2
zigbuild_version=0.24.1
export RUSTUP_TOOLCHAIN=1.100.0

case "$target:$(uname -m)" in
  x86_64-unknown-linux-gnu:x86_64)
    zig_arch=x86_64
    platform=linux/amd64
    zig_sha=02aa270f183da276e5b5920b1dac44a63f1a49e55050ebde3aecc9eb82f93239
    zigbuild_sha=32581c8e13f27a807bb244aaaac870522f5b51263c766cf77b1a99771ce9fa5c
    sdk=centos@sha256:dead07b4d8ed7e29e98de0f4504d87e8880d4347859d839686a31da35a3b532f
    udev_sha=6b9431f01189f5affc2881f465111989d6e94e5b18f68c508778c5af56ccae69
    ;;
  aarch64-unknown-linux-gnu:aarch64)
    zig_arch=aarch64
    platform=linux/arm64
    zig_sha=958ed7d1e00d0ea76590d27666efbf7a932281b3d7ba0c6b01b0ff26498f667f
    zigbuild_sha=70bb4c9cc71a6d16211b9079996e3cd588671053312adeff4d043499ed887601
    sdk=centos@sha256:73f11afcbb50d8bc70eab9f0850b3fa30e61a419bc48cf426e63527d14a8373b
    udev_sha=05b5c76ec9822b0931b98c5496c5223b825c06d5d9942fadcfa248012ce50e6e
    ;;
  *) echo "Unsupported native build for $target" >&2; exit 1 ;;
esac

work=$(mktemp -d "${RUNNER_TEMP:-/tmp}/ptouch-linux-sdk.XXXXXXXX")
sdk_container=
cleanup() {
  if [[ -n "$sdk_container" ]]; then docker rm "$sdk_container" >/dev/null; fi
  rm -rf "$work"
}
trap cleanup EXIT

verify() {
  printf '%s  %s\n' "$1" "$2" | sha256sum --check --status
}

download() {
  curl --fail --silent --show-error --location --retry 3 "$1" --output "$2"
  verify "$3" "$2"
}

zig_dist="zig-$zig_arch-linux-$zig_version"
download "https://ziglang.org/download/$zig_version/$zig_dist.tar.xz" "$work/zig.tar.xz" "$zig_sha"
download "https://github.com/rust-cross/cargo-zigbuild/releases/download/v$zigbuild_version/cargo-zigbuild-$target.tar.xz" \
  "$work/zigbuild.tar.xz" "$zigbuild_sha"
mkdir -p "$work/zigbuild" "$work/udev/include" "$work/udev/lib/pkgconfig"
tar -xJf "$work/zig.tar.xz" -C "$work"
tar -xJf "$work/zigbuild.tar.xz" --strip-components=1 -C "$work/zigbuild"
export PATH="$work/$zig_dist:$work/zigbuild:$PATH"
export CARGO_ZIGBUILD_CACHE_DIR="$work/zig-cache"

# The SDK supplies a link-time input only. The binary uses the host libudev.
sdk_container=$(docker create --platform "$platform" "$sdk")
docker cp -L "$sdk_container:/usr/lib64/libudev.so.1" "$work/udev/lib/libudev.so.1"
verify "$udev_sha" "$work/udev/lib/libudev.so.1"
ln -s libudev.so.1 "$work/udev/lib/libudev.so"
download https://raw.githubusercontent.com/systemd/systemd/v219/src/libudev/libudev.h \
  "$work/udev/include/libudev.h" ef01cfb9b9751df0900ac75911873531a8cc39ce80734bd6dc88c29d18a1fe51
cat > "$work/udev/lib/pkgconfig/libudev.pc" <<EOF
Name: libudev
Description: CentOS 7 release link input
Version: 219
Libs: -L$work/udev/lib -ludev
Cflags: -I$work/udev/include
EOF
unset PKG_CONFIG_PATH PKG_CONFIG_SYSROOT_DIR LIBUDEV_NO_PKG_CONFIG
export PKG_CONFIG_LIBDIR="$work/udev/lib/pkgconfig"
pkg-config --modversion libudev
rustup toolchain install "$RUSTUP_TOOLCHAIN" --profile minimal --target "$target"
rustc --version
zig version
printf 'cargo-zigbuild %s; libudev SDK %s\n' "$zigbuild_version" "$sdk"
cargo zigbuild --release --workspace --locked --target "$target.2.17"
