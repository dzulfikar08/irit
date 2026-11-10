#!/usr/bin/env bash
# macos-app.sh BINARY DEST_DIR  ->  DEST_DIR/msaview.app (ad-hoc signed)
# Bundle id com.dzulfikar.irit.msaview, version from [workspace.package] in Cargo.toml, the icon
# from scripts/assets/msaview.svg (skipped with a warning without rsvg-convert:
# brew install librsvg). No document types: msaview takes the file as an argument and doesn't
# handle Finder's open-file events yet.
# Used by scripts/build-dmg.sh, which re-signs the app with a Developer ID.
set -euo pipefail
bin="${1:?usage: $0 BINARY DEST_DIR}"; dest="${2:?usage: $0 BINARY DEST_DIR}"
root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(awk '/^\[workspace.package\]/{s=1;next} /^\[/{s=0} s && /^version *=/{gsub(/.*= *"|".*/,""); print; exit}' "$root/Cargo.toml")"
[ -n "$version" ] || { echo "error: no [workspace.package] version in Cargo.toml" >&2; exit 1; }
app="$dest/msaview.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m755 "$bin" "$app/Contents/MacOS/msaview"

icon_keys=""
if command -v rsvg-convert >/dev/null && command -v iconutil >/dev/null; then
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  iconset="$work/msaview.iconset"
  mkdir -p "$iconset"
  for s in 16 32 128 256 512; do
    rsvg-convert -w "$s" -h "$s" "$root/scripts/assets/msaview.svg" -o "$iconset/icon_${s}x${s}.png"
    rsvg-convert -w $((s * 2)) -h $((s * 2)) "$root/scripts/assets/msaview.svg" -o "$iconset/icon_${s}x${s}@2x.png"
  done
  iconutil -c icns "$iconset" -o "$app/Contents/Resources/msaview.icns"
  icon_keys="<key>CFBundleIconFile</key><string>msaview</string>"
else
  echo "::warning::rsvg-convert or iconutil missing: msaview.app has no icon" >&2
fi

cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>msaview</string>
  <key>CFBundleDisplayName</key><string>msaview</string>
  <key>CFBundleIdentifier</key><string>com.dzulfikar.irit.msaview</string>
  <key>CFBundleExecutable</key><string>msaview</string>
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
echo "==> $app ($(lipo -archs "$app/Contents/MacOS/msaview"))"
