#!/usr/bin/env bash
# CI only (macOS): import the Developer ID Application identity into a temporary keychain and
# write the App Store Connect API key to disk, for scripts/sign-macos.sh and
# scripts/build-dmg.sh --notarize (release.yml).
#
#   scripts/ci-keychain.sh setup       # appends ASC_KEY_PATH to $GITHUB_ENV
#   scripts/ci-keychain.sh setup-mas   # Mac App Store: also MAS_PROFILE_PATH (msascope-mas.yml)
#   scripts/ci-keychain.sh cleanup     # run in an `if: always()` step
#
# setup reads DEVELOPER_ID_P12_BASE64, DEVELOPER_ID_P12_PASSWORD, ASC_KEY_ID and ASC_KEY_BASE64;
# setup-mas reads MAS_APP_P12_BASE64, MAS_INSTALLER_P12_BASE64, MAS_P12_PASSWORD,
# MAS_PROVISIONPROFILE_BASE64, ASC_KEY_ID and ASC_KEY_BASE64 (GitHub environment `release`).
# Never echoes them.
set -euo pipefail

tmp="${RUNNER_TEMP:?RUNNER_TEMP not set (CI only)}"
kc="$tmp/irit-release.keychain-db"
key_dir="$tmp/asc"

# The App Store Connect API key, for notarytool and altool (which also looks in
# ~/.appstoreconnect/private_keys).
write_asc_key() {
  mkdir -p "$key_dir"
  key="$key_dir/AuthKey_${ASC_KEY_ID}.p8"
  printf '%s' "$ASC_KEY_BASE64" | base64 --decode > "$key"
  echo "ASC_KEY_PATH=$key" >> "$GITHUB_ENV"
}

# A temporary keychain, searched first, with the given base64 p12 files imported.
make_keychain() {  # password p12_base64...
  local pass="$1"; shift
  kcpass="$(uuidgen)"
  security create-keychain -p "$kcpass" "$kc"
  security set-keychain-settings -lut 21600 "$kc"
  security unlock-keychain -p "$kcpass" "$kc"
  local i=0 p12
  for b64 in "$@"; do
    p12="$tmp/import-$i.p12"; i=$((i + 1))
    printf '%s' "$b64" | base64 --decode > "$p12"
    security import "$p12" -k "$kc" -f pkcs12 -P "$pass" \
      -T /usr/bin/codesign -T /usr/bin/security -T /usr/bin/productbuild > /dev/null
    rm -f "$p12"
  done
  security set-key-partition-list -S apple-tool:,apple:,codesign: -s -k "$kcpass" "$kc" > /dev/null
  existing=()
  while IFS= read -r line; do
    line="${line#"${line%%[![:space:]]*}"}"; line="${line%\"}"; line="${line#\"}"
    [ -n "$line" ] && existing+=("$line")
  done < <(security list-keychains -d user)
  security list-keychains -d user -s "$kc" ${existing[@]+"${existing[@]}"}
  # Identity names only (no hashes): proves the import worked.
  security find-identity -v "$kc" | sed -n 's/^ *[0-9]*) [0-9A-F]* //p'
}

case "${1:-}" in
setup)
  umask 077
  for v in DEVELOPER_ID_P12_BASE64 DEVELOPER_ID_P12_PASSWORD ASC_KEY_ID ASC_KEY_BASE64; do
    [ -n "${!v:-}" ] || { echo "::error::$v is not set (secrets of the 'release' environment)" >&2; exit 1; }
  done
  write_asc_key
  make_keychain "$DEVELOPER_ID_P12_PASSWORD" "$DEVELOPER_ID_P12_BASE64"
  security find-identity -v -p codesigning "$kc" | grep -q '"Developer ID Application' \
    || { echo "::error::no Developer ID Application identity in DEVELOPER_ID_P12_BASE64" >&2; exit 1; }
  ;;
setup-mas)
  umask 077
  for v in MAS_APP_P12_BASE64 MAS_INSTALLER_P12_BASE64 MAS_P12_PASSWORD MAS_PROVISIONPROFILE_BASE64 ASC_KEY_ID ASC_KEY_BASE64; do
    [ -n "${!v:-}" ] || { echo "::error::$v is not set (secrets of the 'release' environment)" >&2; exit 1; }
  done
  write_asc_key
  make_keychain "$MAS_P12_PASSWORD" "$MAS_APP_P12_BASE64" "$MAS_INSTALLER_P12_BASE64"
  security find-identity -v -p codesigning "$kc" | grep -Eq '"(Apple Distribution|3rd Party Mac Developer Application)' \
    || { echo "::error::no Apple Distribution identity in MAS_APP_P12_BASE64" >&2; exit 1; }
  security find-identity -v "$kc" | grep -q '"3rd Party Mac Developer Installer' \
    || { echo "::error::no 3rd Party Mac Developer Installer identity in MAS_INSTALLER_P12_BASE64" >&2; exit 1; }
  profile="$tmp/msascope.provisionprofile"
  printf '%s' "$MAS_PROVISIONPROFILE_BASE64" | base64 --decode > "$profile"
  echo "MAS_PROFILE_PATH=$profile" >> "$GITHUB_ENV"
  ;;
cleanup)
  rm -rf "$key_dir" "$tmp"/import-*.p12 "$tmp/msascope.provisionprofile"
  security delete-keychain "$kc" 2>/dev/null || true
  ;;
*)
  echo "usage: $0 setup | setup-mas | cleanup" >&2; exit 1 ;;
esac
