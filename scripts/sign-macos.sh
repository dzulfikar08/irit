#!/usr/bin/env bash
# sign-macos.sh [--notarize] BINARY...  ->  signs command-line binaries in place with the
# "Developer ID Application" identity (hardened runtime, secure timestamp, identifier
# com.dzulfikar.irit.<name>) and checks each with codesign --verify --strict.
#
#   --notarize   also zip the signed binaries and notarise the zip (scripts/notarize.sh).
#                Bare binaries can't be stapled; Gatekeeper looks the ticket up online.
#
# Environment:
#   IRIT_SIGN_IDENTITY   codesign identity (name or SHA-1); default: the first valid
#                        "Developer ID Application" identity in the keychain search list
#   ASC_KEY_PATH, ASC_KEY_ID, ASC_ISSUER_ID   for --notarize (see scripts/notarize.sh)
# msascope.app is signed by scripts/build-dmg.sh instead.
set -euo pipefail
notarize=0
if [ "${1:-}" = --notarize ]; then notarize=1; shift; fi
[ $# -gt 0 ] || { echo "usage: $0 [--notarize] BINARY..." >&2; exit 2; }
[ "$(uname -s)" = Darwin ] || { echo "sign-macos.sh runs on macOS only" >&2; exit 1; }
root="$(cd "$(dirname "$0")/.." && pwd)"

identity="${IRIT_SIGN_IDENTITY:-$(security find-identity -v -p codesigning | awk '/"Developer ID Application/ { print $2; exit }')}"
[ -n "$identity" ] || { echo "error: no valid \"Developer ID Application\" identity in the keychain (set IRIT_SIGN_IDENTITY or import one)" >&2; exit 1; }

for b in "$@"; do
  [ -f "$b" ] || { echo "error: $b does not exist" >&2; exit 1; }
  xattr -c "$b" 2>/dev/null || true
  codesign --force --options runtime --timestamp \
    --identifier "com.dzulfikar.irit.$(basename "$b")" --sign "$identity" "$b"
  codesign --verify --strict --verbose=2 "$b"
  codesign --display --verbose=2 "$b" 2>&1 | grep -E '^(Identifier|Authority=Developer ID Application|TeamIdentifier|Runtime Version|Timestamp)=' || true
done

if [ "$notarize" = 1 ]; then
  work="$(mktemp -d)"
  trap 'rm -rf "$work"' EXIT
  mkdir "$work/irit"
  for b in "$@"; do ditto "$b" "$work/irit/$(basename "$b")"; done
  ditto -c -k --keepParent "$work/irit" "$work/irit-clis.zip"
  "$root/scripts/notarize.sh" "$work/irit-clis.zip"
fi
