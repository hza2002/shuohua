#!/bin/bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

target="aarch64-apple-darwin"
deployment_target="15.0"
dist_dir="${DIST_DIR:-dist}"
cargo_bin="${CARGO:-cargo}"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
name="shuo-v${version}-${target}"

if [[ "$(uname -m)" != "arm64" ]]; then
    echo "ERROR: distribution builds require an Apple Silicon host" >&2
    exit 1
fi

sdk_major="$(xcrun --sdk macosx --show-sdk-version | cut -d. -f1)"
if [[ "$sdk_major" -lt 26 ]]; then
    echo "ERROR: distribution builds require the Xcode 26 SDK or newer" >&2
    exit 1
fi

build_root="$(mktemp -d "${TMPDIR:-/tmp}/shuo-dist-build.XXXXXX")"
pkg_config_root="$(mktemp -d "${TMPDIR:-/tmp}/shuo-empty-pkg-config.XXXXXX")"
cleanup() {
    rm -rf "$build_root" "$pkg_config_root"
}
trap cleanup EXIT

env \
    -u ORT_LIB_LOCATION \
    -u ORT_LIB_PROFILE \
    -u ORT_PREFER_DYNAMIC_LINK \
    -u ORT_SKIP_DOWNLOAD \
    -u ORT_CXX_STDLIB \
    -u CXXSTDLIB \
    "PKG_CONFIG_PATH=$pkg_config_root" \
    "PKG_CONFIG_LIBDIR=$pkg_config_root" \
    "PKG_CONFIG_SYSROOT_DIR=" \
    "PKG_CONFIG_PATH_aarch64_apple_darwin=$pkg_config_root" \
    "PKG_CONFIG_LIBDIR_aarch64_apple_darwin=$pkg_config_root" \
    "PKG_CONFIG_SYSROOT_DIR_aarch64_apple_darwin=" \
    "PKG_CONFIG_PATH_aarch64-apple-darwin=$pkg_config_root" \
    "PKG_CONFIG_LIBDIR_aarch64-apple-darwin=$pkg_config_root" \
    "PKG_CONFIG_SYSROOT_DIR_aarch64-apple-darwin=" \
    "CARGO_TARGET_DIR=$build_root" \
    "MACOSX_DEPLOYMENT_TARGET=$deployment_target" \
    "$cargo_bin" build --release --locked --target "$target"

rm -rf \
    "$dist_dir/$name" \
    "$dist_dir/$name.tar.gz" \
    "$dist_dir/$name.tar.gz.sha256"
mkdir -p "$dist_dir/$name"
cp "$build_root/$target/release/shuo" "$dist_dir/$name/"
cp LICENSE README.md README.en.md "$dist_dir/$name/"
tar -C "$dist_dir" -czf "$dist_dir/$name.tar.gz" "$name"
(
    cd "$dist_dir"
    shasum -a 256 "$name.tar.gz" > "$name.tar.gz.sha256"
)

scripts/verify-macos-dist.sh "$dist_dir/$name.tar.gz" "$build_root"
