#!/usr/bin/env bash
# changelog-section.sh [--released] VERSION [CHANGELOG]  ->  prints the body of the "## VERSION"
# section (no heading). Headings look like "## 0.2.0 (2026-11-20)" or "## 0.2.0 (unreleased)".
# Fails if the section is missing or empty; with --released, also if it is still "(unreleased)".
# Used by release.yml for the check job and the GitHub Release notes.
set -euo pipefail
released=0
if [ "${1:-}" = --released ]; then released=1; shift; fi
version="${1:?usage: $0 [--released] VERSION [CHANGELOG]}"
version="${version#v}"
file="${2:-$(cd "$(dirname "$0")/.." && pwd)/CHANGELOG.md}"

heading="$(awk -v v="$version" '
  /^## / { h = substr($0, 4); sub(/ .*/, "", h); if (h == v) { print; exit } }
' "$file")"
if [ -z "$heading" ]; then
  echo "no '## $version' section in $file" >&2
  exit 1
fi
if [ "$released" = 1 ]; then
  case "$heading" in
    *[Uu]nreleased*)
      echo "CHANGELOG section '$heading' is marked unreleased: give it the release date first, e.g. '## $version ($(date -u +%Y-%m-%d))'" >&2
      exit 1 ;;
  esac
fi

body="$(awk -v v="$version" '
  /^## / { if (on) exit; h = substr($0, 4); sub(/ .*/, "", h); if (h == v) { on = 1; next } }
  on { print }
' "$file" | sed -e '/./,$!d')"   # drop leading blank lines
if [ -z "$(printf '%s' "$body" | tr -d '[:space:]')" ]; then
  echo "the '## $version' section in $file is empty" >&2
  exit 1
fi
printf '%s\n' "$body"
