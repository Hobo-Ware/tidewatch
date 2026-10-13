#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"

if [ "$(uname -s)" != "Linux" ] || [ "$(uname -m)" != "x86_64" ]; then
  echo "package-linux.sh builds the x86_64 Linux tarball and AppImage and only runs on x86_64 Linux." >&2
  exit 1
fi

version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
dist="$root/target/dist"
name="tidewatch-${version}-x86_64-linux"
stage="$dist/$name"
appdir="$dist/Tidewatch.AppDir"
appimage="$dist/Tidewatch-${version}-x86_64.AppImage"
appimagetool="${APPIMAGETOOL:-appimagetool}"

cargo build --release -p tidewatch --manifest-path "$root/Cargo.toml"
binary="$root/target/release/tidewatch"

rm -rf "${stage:?}" "${appdir:?}" "${dist:?}/$name.tar.gz" "${appimage:?}"
mkdir -p "$stage"
install -m 755 "$binary" "$stage/tidewatch"
install -m 644 "$root/README.md" "$root/LICENSE" "$stage/"
install -m 644 "$root/packaging/tidewatch.desktop" "$stage/tidewatch.desktop"
install -m 644 "$root/assets/icon-512.png" "$stage/tidewatch.png"
tar -C "$dist" -czf "$dist/$name.tar.gz" "$name"
echo "Built $dist/$name.tar.gz ($(du -h "$dist/$name.tar.gz" | cut -f1))"

if ! command -v "$appimagetool" >/dev/null 2>&1; then
  echo "Skipped the AppImage: '$appimagetool' not found. Download appimagetool or set APPIMAGETOOL to its path." >&2
  exit 0
fi

mkdir -p "$appdir/usr/bin"
install -m 755 "$binary" "$appdir/usr/bin/tidewatch"
install -m 644 "$root/packaging/tidewatch.desktop" "$appdir/tidewatch.desktop"
install -m 644 "$root/assets/icon-512.png" "$appdir/tidewatch.png"
ln -s usr/bin/tidewatch "$appdir/AppRun"
ARCH=x86_64 APPIMAGE_EXTRACT_AND_RUN=1 "$appimagetool" "$appdir" "$appimage"
echo "Built $appimage ($(du -h "$appimage" | cut -f1))"
