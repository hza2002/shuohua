#!/usr/bin/env bash
set -euo pipefail

ICON_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SRC="$ICON_DIR/shuohua-icon.svg"
MACOS_DIR="$ICON_DIR/macos"

PNG_1024="$ICON_DIR/shuohua-icon-1024.png"
ICNS="$MACOS_DIR/shuohua.icns"
TMP_DIR=""

cleanup() {
  if [[ -n "$TMP_DIR" && -d "$TMP_DIR" ]]; then
    rm -rf "$TMP_DIR"
  fi
}
trap cleanup EXIT

render_png() {
  local size="$1"
  local out="$2"

  if command -v rsvg-convert >/dev/null 2>&1; then
    rsvg-convert -w "$size" -h "$size" "$SRC" -o "$out"
  elif command -v resvg >/dev/null 2>&1; then
    resvg -w "$size" -h "$size" "$SRC" "$out"
  elif command -v magick >/dev/null 2>&1; then
    magick -background none -resize "${size}x${size}" "$SRC" "$out"
  else
    echo "error: install rsvg-convert, resvg, or ImageMagick to render SVG assets" >&2
    exit 1
  fi
}

mkdir -p "$MACOS_DIR"

render_png 1024 "$PNG_1024"

TMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/shuohua-icon.XXXXXX")"
ICONSET="$TMP_DIR/shuohua.iconset"
mkdir -p "$ICONSET"

render_png 16 "$ICONSET/icon_16x16.png"
render_png 32 "$ICONSET/icon_16x16@2x.png"
render_png 32 "$ICONSET/icon_32x32.png"
render_png 64 "$ICONSET/icon_32x32@2x.png"
render_png 128 "$ICONSET/icon_128x128.png"
render_png 256 "$ICONSET/icon_128x128@2x.png"
render_png 256 "$ICONSET/icon_256x256.png"
render_png 512 "$ICONSET/icon_256x256@2x.png"
render_png 512 "$ICONSET/icon_512x512.png"
render_png 1024 "$ICONSET/icon_512x512@2x.png"

if command -v iconutil >/dev/null 2>&1; then
  iconutil -c icns "$ICONSET" -o "$ICNS"
else
  echo "warning: iconutil not found; skipped .icns generation" >&2
fi

echo "wrote $PNG_1024"
if [[ -f "$ICNS" ]]; then
  echo "wrote $ICNS"
fi
