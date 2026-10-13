#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
svg="$root/assets/icon.svg"
work="$(mktemp -d)"
trap 'rm -rf "${work:?}"' EXIT

render() {
  local px="$1" png="$2"
  inkscape "$svg" --export-type=png --export-width="$px" --export-filename="$png" >/dev/null 2>&1
  pngquant --force --speed 1 --strip 64 --output "$png" "$png"
  oxipng -q -o 6 --strip safe "$png"
}

mkdir -p "$work/Tidewatch.iconset"
for size in 16 32 128 256 512; do
  for scale in 1 2; do
    name="icon_${size}x${size}"
    [ "$scale" = 2 ] && name="${name}@2x"
    render $((size * scale)) "$work/Tidewatch.iconset/${name}.png"
  done
done
iconutil -c icns "$work/Tidewatch.iconset" -o "$root/assets/Tidewatch.icns"
echo "Wrote assets/Tidewatch.icns ($(du -h "$root/assets/Tidewatch.icns" | cut -f1))"

xcode="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
car="$root/assets/Assets.car"
if DEVELOPER_DIR="$xcode" xcrun --find actool >/dev/null 2>&1; then
  DEVELOPER_DIR="$xcode" xcrun actool "$root/assets/AppIcon.icon" \
    --compile "$work" --platform macosx --minimum-deployment-target 12.0 \
    --app-icon AppIcon --output-partial-info-plist "$work/partial.plist" >/dev/null
  cp "$work/Assets.car" "$car"
  echo "Wrote $car ($(du -h "$car" | cut -f1)) from AppIcon.icon"
else
  echo "Xcode 26 not found: kept the committed $car" >&2
fi

render 512 "$root/assets/icon-512.png"
echo "Wrote assets/icon-512.png ($(du -h "$root/assets/icon-512.png" | cut -f1))"

big="$work/orig.png"
inkscape "$svg" --export-type=png --export-width=512 --export-filename="$big" >/dev/null 2>&1
magick compare -metric PSNR "$big" "$root/assets/icon-512.png" null: 2>&1 | sed 's/^/PSNR (dB): /' || true

tray_svg="$root/assets/tray.svg"
tray_big="$work/tray-orig.png"
inkscape "$tray_svg" --export-type=png --export-width=44 --export-filename="$tray_big" >/dev/null 2>&1
cp "$tray_big" "$root/assets/tray.png"
pngquant --force --speed 1 --strip 64 --output "$root/assets/tray.png" "$root/assets/tray.png"
oxipng -q -o 6 --strip safe "$root/assets/tray.png"
echo "Wrote assets/tray.png ($(du -h "$root/assets/tray.png" | cut -f1))"
magick compare -metric PSNR "$tray_big" "$root/assets/tray.png" null: 2>&1 | sed 's/^/PSNR (dB): /' || true
