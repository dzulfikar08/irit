#!/usr/bin/env bash
# build-mas-pkg.sh --build N BINARY OUT_DIR  ->  OUT_DIR/msascope.app and OUT_DIR/msascope-<version>.pkg
# The Mac App Store edition of msascope: sandboxed, signed with "Apple Distribution" (or "3rd Party
# Mac Developer Application"), the Mac App Store provisioning profile embedded, packaged with
# productbuild and signed with "3rd Party Mac Developer Installer". Upload the .pkg with
# `xcrun altool --upload-package` (msascope-mas.yml); it is not notarised.
#
#   --build N   CFBundleVersion, which must grow with every upload (e.g. 0.3.5.42)
#
# Entitlements: scripts/assets/msascope-mas.entitlements (TEAMID filled in here): the sandbox, files
# the user chooses (open/save panels, drops, Finder) and bookmarks to reopen them; no network, no
# JIT, no temporary exceptions. scripts/assets/msascope-sandbox-test.entitlements is the same
# without team ids, for an ad-hoc signed test copy (msascope.yml). No comments in those files:
# codesign's entitlements parser rejects them.
#
# Environment:
#   APPLE_TEAM_ID               the team identifier (10 characters)
#   MAS_PROFILE_PATH            the Mac App Store provisioning profile (.provisionprofile)
#   MSASCOPE_ACKNOWLEDGEMENTS   Acknowledgements.html (scripts/acknowledgements.sh); required
#   MAS_APP_IDENTITY, MAS_INSTALLER_IDENTITY   override the identities found in the keychain
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
usage="usage: $0 --build N BINARY OUT_DIR"
build="" pos=()
while [ $# -gt 0 ]; do
  case "$1" in
    --build) build="${2:?$usage}"; shift 2 ;;
    -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
    -*) echo "$usage" >&2; exit 2 ;;
    *) pos+=("$1"); shift ;;
  esac
done
[ ${#pos[@]} -eq 2 ] && [ -n "$build" ] || { echo "$usage" >&2; exit 2; }
bin="${pos[0]}" out="${pos[1]}"
[ "$(uname -s)" = Darwin ] || { echo "build-mas-pkg.sh runs on macOS only" >&2; exit 1; }
for v in APPLE_TEAM_ID MAS_PROFILE_PATH MSASCOPE_ACKNOWLEDGEMENTS; do
  [ -n "${!v:-}" ] || { echo "error: $v is not set" >&2; exit 1; }
done
[[ "$APPLE_TEAM_ID" =~ ^[A-Z0-9]{10}$ ]] || { echo "error: APPLE_TEAM_ID is not a 10-character team id" >&2; exit 1; }

find_identity() { security find-identity -v -p "$1" | awk -v want="$2" 'index($0, want) { print $2; exit }'; }
app_id="${MAS_APP_IDENTITY:-$(find_identity codesigning '"Apple Distribution')}"
app_id="${app_id:-$(find_identity codesigning '"3rd Party Mac Developer Application')}"
inst_id="${MAS_INSTALLER_IDENTITY:-$(security find-identity -v | awk 'index($0, "\"3rd Party Mac Developer Installer") { print $2; exit }')}"
[ -n "$app_id" ] || { echo "error: no Apple Distribution / 3rd Party Mac Developer Application identity" >&2; exit 1; }
[ -n "$inst_id" ] || { echo "error: no 3rd Party Mac Developer Installer identity" >&2; exit 1; }

version="$(awk '/^\[workspace.package\]/{s=1;next} /^\[/{s=0} s && /^version *=/{gsub(/.*= *"|".*/,""); print; exit}' "$root/Cargo.toml")"
mkdir -p "$out"; out="$(cd "$out" && pwd)"

MSASCOPE_MAS=1 MSASCOPE_BUILD="$build" "$root/scripts/macos-app.sh" "$bin" "$out"
app="$out/msascope.app"
cp "$MAS_PROFILE_PATH" "$app/Contents/embedded.provisionprofile"

ent="$(mktemp)"; trap 'rm -f "$ent"' EXIT
sed "s/TEAMID/$APPLE_TEAM_ID/g" "$root/scripts/assets/msascope-mas.entitlements" > "$ent"
plutil -lint "$ent" > /dev/null
xattr -cr "$app"
codesign --force --options runtime --timestamp --entitlements "$ent" --sign "$app_id" "$app"
codesign --verify --strict --verbose=2 "$app"
codesign -d --entitlements - --xml "$app" 2>/dev/null | plutil -p - | grep -q 'com.apple.security.app-sandbox' \
  || { echo "error: the signed app has no sandbox entitlement" >&2; exit 1; }

pkg="$out/msascope-$version.pkg"
rm -f "$pkg"
productbuild --component "$app" /Applications --sign "$inst_id" "$pkg"
pkgutil --check-signature "$pkg"
echo "==> $pkg (CFBundleVersion $build)"
