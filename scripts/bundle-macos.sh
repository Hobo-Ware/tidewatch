#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$root/Cargo.toml" | head -1)"
identity="${SIGN_IDENTITY:--}"
dist="$root/target/dist"
app="$dist/Tidewatch.app"
zip="$dist/tidewatch-${version}-universal.app.zip"
targets=(aarch64-apple-darwin x86_64-apple-darwin)

if [ "$(uname -s)" != "Darwin" ]; then
  echo "bundle-macos.sh builds Tidewatch.app and only runs on macOS." >&2
  exit 1
fi

for target in "${targets[@]}"; do
  if command -v rustup >/dev/null 2>&1; then
    rustup target add "$target" >/dev/null
  fi
  cargo build --release -p tidewatch --target "$target" --manifest-path "$root/Cargo.toml"
done

rm -rf "${app:?}" "${zip:?}"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create -output "$app/Contents/MacOS/tidewatch" \
  "$root/target/aarch64-apple-darwin/release/tidewatch" \
  "$root/target/x86_64-apple-darwin/release/tidewatch"
chmod +x "$app/Contents/MacOS/tidewatch"
cp "$root/assets/Tidewatch.icns" "$app/Contents/Resources/Tidewatch.icns"
cp "$root/assets/Assets.car" "$app/Contents/Resources/Assets.car"
sed "s/__VERSION__/${version}/g" "$root/packaging/Info.plist" > "$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist" >/dev/null

codesign --force --deep --sign "$identity" "$app"
codesign --verify --deep --strict "$app"

ditto -c -k --sequesterRsrc --keepParent "$app" "$zip"
echo "Built $app ($(du -sh "$app" | cut -f1))"
echo "Zipped $zip ($(du -h "$zip" | cut -f1))"
