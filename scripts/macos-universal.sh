#!/usr/bin/env bash
# macos-universal.sh OUT_DIR PACKAGE... -- BINARY...  ->  OUT_DIR/BINARY (arm64 + x86_64)
# Builds the packages for aarch64-apple-darwin and x86_64-apple-darwin (release profile, the
# committed Cargo.lock) and joins each named binary with lipo. Needs Xcode and both Rust targets:
#   rustup target add aarch64-apple-darwin x86_64-apple-darwin
# Example: scripts/macos-universal.sh dist/universal sheetgen msaview -- sheetgen msaview
set -euo pipefail
usage="usage: $0 OUT_DIR PACKAGE... -- BINARY..."
[ "$(uname -s)" = Darwin ] || { echo "macos-universal.sh runs on macOS only" >&2; exit 1; }
out="${1:?$usage}"; shift
pkgs=()
while [ $# -gt 0 ] && [ "$1" != -- ]; do pkgs+=(-p "$1"); shift; done
[ "${1:-}" = -- ] || { echo "$usage" >&2; exit 2; }
shift
[ ${#pkgs[@]} -gt 0 ] && [ $# -gt 0 ] || { echo "$usage" >&2; exit 2; }
root="$(cd "$(dirname "$0")/.." && pwd)"

# Rust's x86_64 default is 10.12; match msaview.app's LSMinimumSystemVersion for both slices.
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
for target in aarch64-apple-darwin x86_64-apple-darwin; do
  (cd "$root" && cargo build --release --locked --target "$target" "${pkgs[@]}")
done

mkdir -p "$out"
for b in "$@"; do
  lipo -create -output "$out/$b" \
    "$root/target/aarch64-apple-darwin/release/$b" "$root/target/x86_64-apple-darwin/release/$b"
  archs="$(lipo -archs "$out/$b")"
  case "$archs" in *arm64*x86_64*|*x86_64*arm64*) ;; *) echo "error: $b is not universal: $archs" >&2; exit 1 ;; esac
  echo "$out/$b: $archs"
done
