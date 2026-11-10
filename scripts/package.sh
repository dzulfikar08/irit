#!/usr/bin/env bash
# package.sh [--zip] OUT_PREFIX PATH...  ->  OUT_PREFIX.tar.gz (or OUT_PREFIX.zip with --zip)
# The archive holds one folder, basename(OUT_PREFIX), with the given files or folders (e.g. the
# binaries, or msaview.app) plus README.md and LICENSE from the repository root.
# Used by release.yml for every platform; scripts/install.sh unpacks the .tar.gz ones.
set -euo pipefail
zip=0
if [ "${1:-}" = --zip ]; then zip=1; shift; fi
[ $# -ge 2 ] || { echo "usage: $0 [--zip] OUT_PREFIX PATH..." >&2; exit 2; }
out="$1"; shift
root="$(cd "$(dirname "$0")/.." && pwd)"
name="$(basename "$out")"
mkdir -p "$(dirname "$out")"
out="$(cd "$(dirname "$out")" && pwd)/$name"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
stage="$work/$name"
mkdir -p "$stage"
for p in "$@"; do
  [ -e "$p" ] || { echo "error: $p does not exist" >&2; exit 1; }
  if [ -d "$p" ] && command -v ditto >/dev/null; then
    # macOS: keeps the app bundle's code signature intact.
    ditto "$p" "$stage/$(basename "$p")"
  else
    cp -R "$p" "$stage/"
  fi
done
cp "$root/README.md" "$root/LICENSE" "$stage/"

if [ "$zip" = 1 ]; then
  rm -f "$out.zip"
  if command -v 7z >/dev/null; then
    (cd "$work" && 7z a -tzip -bd "$out.zip" "$name" >/dev/null)
  else
    (cd "$work" && zip -qr "$out.zip" "$name")
  fi
  echo "==> $out.zip"
else
  # COPYFILE_DISABLE: no AppleDouble ._ files from macOS tar.
  COPYFILE_DISABLE=1 tar -C "$work" -czf "$out.tar.gz" "$name"
  echo "==> $out.tar.gz"
fi
