#!/usr/bin/env bash
# macos-app.sh BINARY DEST_DIR  ->  DEST_DIR/msascope.app (ad-hoc signed)
# Bundle id com.dzulfikar.irit.msascope, version from [workspace.package] in Cargo.toml, the icon
# from scripts/assets/msascope.svg (skipped with a warning without rsvg-convert:
# brew install librsvg). No document types: msascope takes the file as an argument and doesn't
# handle Finder's open-file events yet.
# Used by scripts/build-dmg.sh, which re-signs the app with a Developer ID.
set -euo pipefail
bin="${1:?usage: $0 BINARY DEST_DIR}"; dest="${2:?usage: $0 BINARY DEST_DIR}"
root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(awk '/^\[workspace.package\]/{s=1;next} /^\[/{s=0} s && /^version *=/{gsub(/.*= *"|".*/,""); print; exit}' "$root/Cargo.toml")"
[ -n "$version" ] || { echo "error: no [workspace.package] version in Cargo.toml" >&2; exit 1; }
app="$dest/msascope.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m755 "$bin" "$app/Contents/MacOS/msascope"

icon_keys=""
if command -v rsvg-convert >/dev/null && command -v iconutil >/dev/null; then
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  iconset="$work/msascope.iconset"
  mkdir -p "$iconset"
  for s in 16 32 128 256 512; do
    rsvg-convert -w "$s" -h "$s" "$root/scripts/assets/msascope.svg" -o "$iconset/icon_${s}x${s}.png"
    rsvg-convert -w $((s * 2)) -h $((s * 2)) "$root/scripts/assets/msascope.svg" -o "$iconset/icon_${s}x${s}@2x.png"
  done
  iconutil -c icns "$iconset" -o "$app/Contents/Resources/msascope.icns"
  icon_keys="<key>CFBundleIconFile</key><string>msascope</string>"
else
  echo "::warning::rsvg-convert or iconutil missing: msascope.app has no icon" >&2
fi

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>msascope</string>
  <key>CFBundleDisplayName</key><string>msascope</string>
  <key>CFBundleIdentifier</key><string>com.dzulfikar.irit.msascope</string>
  <key>CFBundleExecutable</key><string>msascope</string>
  ${icon_keys}
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>${version}</string>
  <key>CFBundleVersion</key><string>${version}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.education</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>GPL-3.0-or-later. Part of irit: https://github.com/dzulfikar08/irit</string>
</dict>
</plist>
EOF
plutil -lint "$app/Contents/Info.plist" >/dev/null

# Ad-hoc, so the bundle launches as built; build-dmg.sh re-signs it with a Developer ID.
codesign --force --sign - "$app"
echo "==> $app ($(lipo -archs "$app/Contents/MacOS/msascope"))"
