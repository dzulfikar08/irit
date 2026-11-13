#!/usr/bin/env bash
# macos-app.sh BINARY DEST_DIR  ->  DEST_DIR/msascope.app (ad-hoc signed)
# Bundle id com.dzulfikar.irit.msascope, version from [workspace.package] in Cargo.toml, the icon
# from scripts/assets/msascope.svg (skipped with a warning without rsvg-convert:
# brew install librsvg). Document types let Finder open alignments and trees with msascope
# (as an alternative app, never the default owner of those extensions).
# Environment:
#   MSASCOPE_BUILD    CFBundleVersion (default: the version). The App Store needs a new one per upload.
#   MSASCOPE_MAS=1    the Mac App Store edition: its own copyright line instead of the GPL one.
#   MSASCOPE_ACKNOWLEDGEMENTS   an HTML file copied to Resources/Acknowledgements.html (the
#                     licences of msascope's dependencies, scripts/acknowledgements.sh).
# Used by scripts/build-dmg.sh (re-signs it with a Developer ID) and scripts/build-mas-pkg.sh.
set -euo pipefail
bin="${1:?usage: $0 BINARY DEST_DIR}"; dest="${2:?usage: $0 BINARY DEST_DIR}"
root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(awk '/^\[workspace.package\]/{s=1;next} /^\[/{s=0} s && /^version *=/{gsub(/.*= *"|".*/,""); print; exit}' "$root/Cargo.toml")"
[ -n "$version" ] || { echo "error: no [workspace.package] version in Cargo.toml" >&2; exit 1; }
build="${MSASCOPE_BUILD:-$version}"
if [ "${MSASCOPE_MAS:-}" = 1 ]; then
  copyright="© 2026 Dzulfikar Ats Tsauri. The source is also available under GPL-3.0-or-later."
else
  copyright="GPL-3.0-or-later. Part of irit: https://github.com/dzulfikar08/irit"
fi
app="$dest/msascope.app"
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m755 "$bin" "$app/Contents/MacOS/msascope"
if [ -n "${MSASCOPE_ACKNOWLEDGEMENTS:-}" ]; then
  install -m644 "$MSASCOPE_ACKNOWLEDGEMENTS" "$app/Contents/Resources/Acknowledgements.html"
fi

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
  <key>CFBundleVersion</key><string>${build}</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.education</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>${copyright}</string>
  <key>ITSAppUsesNonExemptEncryption</key><false/>
  <key>CFBundleDocumentTypes</key>
  <array>
    <dict>
      <key>CFBundleTypeName</key><string>Sequence alignment</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>LSItemContentTypes</key>
      <array>
        <string>com.dzulfikar.irit.fasta</string>
        <string>com.dzulfikar.irit.clustal</string>
        <string>com.dzulfikar.irit.stockholm</string>
        <string>org.gnu.gnu-zip-archive</string>
      </array>
    </dict>
    <dict>
      <key>CFBundleTypeName</key><string>Newick tree</string>
      <key>CFBundleTypeRole</key><string>Viewer</string>
      <key>LSHandlerRank</key><string>Alternate</string>
      <key>LSItemContentTypes</key><array><string>com.dzulfikar.irit.newick</string></array>
    </dict>
  </array>
  <key>UTImportedTypeDeclarations</key>
  <array>
    <dict>
      <key>UTTypeIdentifier</key><string>com.dzulfikar.irit.fasta</string>
      <key>UTTypeDescription</key><string>FASTA sequences</string>
      <key>UTTypeConformsTo</key><array><string>public.plain-text</string></array>
      <key>UTTypeTagSpecification</key><dict><key>public.filename-extension</key>
        <array><string>fa</string><string>fasta</string><string>fas</string><string>fna</string><string>faa</string><string>afa</string><string>mfa</string></array></dict>
    </dict>
    <dict>
      <key>UTTypeIdentifier</key><string>com.dzulfikar.irit.clustal</string>
      <key>UTTypeDescription</key><string>Clustal alignment</string>
      <key>UTTypeConformsTo</key><array><string>public.plain-text</string></array>
      <key>UTTypeTagSpecification</key><dict><key>public.filename-extension</key>
        <array><string>aln</string><string>clustal</string></array></dict>
    </dict>
    <dict>
      <key>UTTypeIdentifier</key><string>com.dzulfikar.irit.stockholm</string>
      <key>UTTypeDescription</key><string>Stockholm alignment</string>
      <key>UTTypeConformsTo</key><array><string>public.plain-text</string></array>
      <key>UTTypeTagSpecification</key><dict><key>public.filename-extension</key>
        <array><string>sto</string><string>stk</string><string>stockholm</string></array></dict>
    </dict>
    <dict>
      <key>UTTypeIdentifier</key><string>com.dzulfikar.irit.newick</string>
      <key>UTTypeDescription</key><string>Newick tree</string>
      <key>UTTypeConformsTo</key><array><string>public.plain-text</string></array>
      <key>UTTypeTagSpecification</key><dict><key>public.filename-extension</key>
        <array><string>nwk</string><string>newick</string><string>tree</string><string>treefile</string></array></dict>
    </dict>
  </array>
</dict>
</plist>
EOF
plutil -lint "$app/Contents/Info.plist" >/dev/null

# Ad-hoc, so the bundle launches as built; build-dmg.sh re-signs it with a Developer ID.
codesign --force --sign - "$app"
echo "==> $app ($(lipo -archs "$app/Contents/MacOS/msascope"))"
