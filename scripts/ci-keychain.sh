#!/usr/bin/env bash
# CI only (macOS): import the Developer ID Application identity into a temporary keychain and
# write the App Store Connect API key to disk, for scripts/sign-macos.sh and
# scripts/build-dmg.sh --notarize (release.yml).
#
#   scripts/ci-keychain.sh setup     # appends ASC_KEY_PATH to $GITHUB_ENV
#   scripts/ci-keychain.sh cleanup   # run in an `if: always()` step
#
# Reads DEVELOPER_ID_P12_BASE64, DEVELOPER_ID_P12_PASSWORD, ASC_KEY_ID and ASC_KEY_BASE64
# from the environment (GitHub environment `release`). Never echoes them.
set -euo pipefail

tmp="${RUNNER_TEMP:?RUNNER_TEMP not set (CI only)}"
kc="$tmp/irit-release.keychain-db"
key_dir="$tmp/asc"

case "${1:-}" in
setup)
  umask 077
  for v in DEVELOPER_ID_P12_BASE64 DEVELOPER_ID_P12_PASSWORD ASC_KEY_ID ASC_KEY_BASE64; do
    [ -n "${!v:-}" ] || { echo "::error::$v is not set (secrets of the 'release' environment)" >&2; exit 1; }
  done
  mkdir -p "$key_dir"
  key="$key_dir/AuthKey_${ASC_KEY_ID}.p8"
  printf '%s' "$ASC_KEY_BASE64" | base64 --decode > "$key"
  echo "ASC_KEY_PATH=$key" >> "$GITHUB_ENV"

  kcpass="$(uuidgen)"
  security create-keychain -p "$kcpass" "$kc"
  security set-keychain-settings -lut 21600 "$kc"
  security unlock-keychain -p "$kcpass" "$kc"
  p12="$tmp/developer-id.p12"
  printf '%s' "$DEVELOPER_ID_P12_BASE64" | base64 --decode > "$p12"
  security import "$p12" -k "$kc" -f pkcs12 -P "$DEVELOPER_ID_P12_PASSWORD" \
    -T /usr/bin/codesign -T /usr/bin/security > /dev/null
  rm -f "$p12"
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$kcpass" "$kc" > /dev/null
  # Search the new keychain first, keeping the existing ones.
  existing=()
  while IFS= read -r line; do
    line="${line#"${line%%[![:space:]]*}"}"; line="${line%\"}"; line="${line#\"}"
    [ -n "$line" ] && existing+=("$line")
  done < <(security list-keychains -d user)
  security list-keychains -d user -s "$kc" ${existing[@]+"${existing[@]}"}
  # Identity names only (no hashes): proves the import worked.
  security find-identity -v -p codesigning "$kc" | sed -n 's/^ *[0-9]*) [0-9A-F]* //p'
  security find-identity -v -p codesigning "$kc" | grep -q '"Developer ID Application' \
    || { echo "::error::no Developer ID Application identity in DEVELOPER_ID_P12_BASE64" >&2; exit 1; }
  ;;
cleanup)
  rm -rf "$key_dir" "$tmp/developer-id.p12"
  security delete-keychain "$kc" 2>/dev/null || true
  ;;
*)
  echo "usage: $0 setup | cleanup" >&2; exit 1 ;;
esac
