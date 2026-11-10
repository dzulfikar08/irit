#!/usr/bin/env bash
# build-dmg.sh [options] BINARY OUT_DIR  ->  OUT_DIR/msaview.app and OUT_DIR/<name>.dmg
# BINARY is the msaview executable, normally the universal one from scripts/macos-universal.sh.
#
#   --name NAME     DMG file name without .dmg (default msaview-<version>)
#   --sign          sign the app and the DMG with the "Developer ID Application" identity
#                   (hardened runtime, secure timestamp); fails if there is none
#   --notarize      also notarise the DMG (scripts/notarize.sh), staple the ticket to the DMG
#                   and the app, and check both with spctl (implies --sign)
#
# Without --sign the app stays ad-hoc signed: it runs where it was built, but a downloaded copy
# is blocked by Gatekeeper. release.yml uses --notarize.
#
# Environment:
#   IRIT_SIGN_IDENTITY   codesign identity (name or SHA-1); default: the first valid
#                        "Developer ID Application" identity in the keychain search list
#   ASC_KEY_PATH, ASC_KEY_ID, ASC_ISSUER_ID   for --notarize (see scripts/notarize.sh)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
usage="usage: $0 [--name NAME] [--sign] [--notarize] BINARY OUT_DIR"
name="" sign=0 notarize=0 pos=()
while [ $# -gt 0 ]; do
  case "$1" in
    --name) name="${2:?$usage}"; shift 2 ;;
    --sign) sign=1; shift ;;
    --notarize) sign=1; notarize=1; shift ;;
    -h|--help) sed -n '2,18p' "$0"; exit 0 ;;
    -*) echo "$usage" >&2; exit 2 ;;
    *) pos+=("$1"); shift ;;
  esac
done
[ ${#pos[@]} -eq 2 ] || { echo "$usage" >&2; exit 2; }
bin="${pos[0]}" out="${pos[1]}"
[ "$(uname -s)" = Darwin ] || { echo "build-dmg.sh runs on macOS only" >&2; exit 1; }
[ -f "$bin" ] || { echo "error: $bin does not exist" >&2; exit 1; }

version="$(awk '/^\[workspace.package\]/{s=1;next} /^\[/{s=0} s && /^version *=/{gsub(/.*= *"|".*/,""); print; exit}' "$root/Cargo.toml")"
name="${name:-msaview-$version}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
dmg="$out/$name.dmg"

# Fail early when signing or notarising can't work.
identity=""
if [ "$sign" = 1 ]; then
  identity="${IRIT_SIGN_IDENTITY:-$(security find-identity -v -p codesigning | awk '/"Developer ID Application/ { print $2; exit }')}"
  [ -n "$identity" ] || { echo "error: --sign: no valid \"Developer ID Application\" identity in the keychain (set IRIT_SIGN_IDENTITY or import one)" >&2; exit 1; }
fi
if [ "$notarize" = 1 ]; then
  for v in ASC_KEY_PATH ASC_KEY_ID ASC_ISSUER_ID; do
    [ -n "${!v:-}" ] || { echo "error: --notarize needs $v" >&2; exit 1; }
  done
fi

# 1. msaview.app (ad-hoc signed by macos-app.sh).
"$root/scripts/macos-app.sh" "$bin" "$out"
app="$out/msaview.app"

# 2. Developer ID signature: hardened runtime (required for notarisation), secure timestamp.
#    One executable, no nested code and no entitlements (GPUI's Metal shaders compiled at run
#    time are not JIT), so no --deep and no --entitlements.
if [ "$sign" = 1 ]; then
  xattr -cr "$app"
  codesign --force --options runtime --timestamp --sign "$identity" "$app"
  codesign --verify --strict --verbose=2 "$app"
  codesign --display --verbose=2 "$app" 2>&1 | grep -E '^(Identifier|TeamIdentifier|Runtime Version|Timestamp)=' || true
else
  echo "::warning::msaview.app is ad-hoc signed (no Developer ID): Gatekeeper blocks a downloaded copy." >&2
fi

# 3. DMG: the app and a link to /Applications, so it is dragged across.
staging="$(mktemp -d)"
trap 'rm -rf "$staging"' EXIT
ditto "$app" "$staging/msaview.app"
ln -s /Applications "$staging/Applications"
rm -f "$dmg"
# hdiutil can guess the size too small ("No space left on device"): give it the folder size
# plus 20% and 20 MB. On CI runners it sometimes fails with "Resource busy"; retry.
size_mb=$(( $(du -sm "$staging" | cut -f1) * 12 / 10 + 20 ))
for attempt in 1 2 3 4 5; do
  if hdiutil create -volname msaview -srcfolder "$staging" -fs HFS+ -size "${size_mb}m" \
      -format ULMO -ov "$dmg"; then
    break
  fi
  [ "$attempt" -lt 5 ] || { echo "error: hdiutil create failed" >&2; exit 1; }
  echo "hdiutil create failed (attempt $attempt), retrying" >&2
  sleep $((attempt * 5))
done
hdiutil verify "$dmg"

if [ "$sign" = 1 ]; then
  codesign --force --timestamp --sign "$identity" "$dmg"
  codesign --verify --strict --verbose=2 "$dmg"
fi

# 4. Notarise, staple, assess.
if [ "$notarize" = 1 ]; then
  "$root/scripts/notarize.sh" "$dmg"
  xcrun stapler staple "$dmg"
  xcrun stapler validate "$dmg"
  spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
  # The DMG's ticket covers the app inside it too: staple it to msaview.app as well, for the
  # tarball scripts/install.sh unpacks (Gatekeeper checks it online anyway if this fails).
  if xcrun stapler staple "$app"; then
    spctl --assess --type execute --verbose=2 "$app"
  else
    echo "::warning::could not staple the ticket to msaview.app; Gatekeeper will check it online" >&2
  fi
fi

shasum -a 256 "$dmg"
echo "==> $dmg ($(if [ "$notarize" = 1 ]; then echo "Developer ID, notarised"; elif [ "$sign" = 1 ]; then echo "Developer ID, not notarised"; else echo "ad-hoc signed, not notarised"; fi))"
