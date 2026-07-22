#!/bin/sh
# Rasterise packaging/icon.svg into the icon set the Tauri bundler expects.
# Needs rsvg-convert (librsvg) and, for the .ico, ImageMagick.
set -eu
here=$(dirname "$0")
out="$here/../crates/fretwire-tauri/icons"
mkdir -p "$out"
for size in 32 128 256 512; do
  rsvg-convert -w "$size" -h "$size" "$here/icon.svg" -o "$out/${size}x${size}.png"
done
# Tauri's conventional names: 128x128@2x is the 256px render; icon.png is the largest.
mv "$out/256x256.png" "$out/128x128@2x.png"
cp "$out/512x512.png" "$out/icon.png"
# Windows/macOS bundles want these; harmless to generate on Linux.
magick "$out/32x32.png" "$out/128x128.png" "$out/512x512.png" "$out/icon.ico" 2>/dev/null || \
  echo "note: ImageMagick not found — skipped icon.ico (Linux bundles don't need it)"
echo "wrote icons to $out"
