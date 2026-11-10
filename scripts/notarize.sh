#!/usr/bin/env bash
# notarize.sh FILE  ->  submits FILE (.zip, .dmg or .pkg) to Apple's notary service and waits.
# Fails, printing the notary log, unless the submission is Accepted. Does not staple: the
# caller staples what can be stapled (DMGs and apps; bare command-line binaries can't be).
# Environment: ASC_KEY_PATH, ASC_KEY_ID, ASC_ISSUER_ID (App Store Connect API key;
# scripts/ci-keychain.sh setup writes ASC_KEY_PATH in CI).
set -euo pipefail
file="${1:?usage: $0 FILE}"
for v in ASC_KEY_PATH ASC_KEY_ID ASC_ISSUER_ID; do
  [ -n "${!v:-}" ] || { echo "error: notarising needs $v" >&2; exit 1; }
done
[ -f "$ASC_KEY_PATH" ] || { echo "error: ASC_KEY_PATH is not a file" >&2; exit 1; }
[ -f "$file" ] || { echo "error: $file does not exist" >&2; exit 1; }

result="$(mktemp)"
trap 'rm -f "$result"' EXIT
# notarytool exits 0 for a finished but rejected submission, so read the status.
xcrun notarytool submit "$file" --key "$ASC_KEY_PATH" --key-id "$ASC_KEY_ID" \
  --issuer "$ASC_ISSUER_ID" --wait --timeout 1h --output-format json > "$result" || true
cat "$result"; echo
status="$(plutil -extract status raw -o - "$result" 2>/dev/null || true)"
sub_id="$(plutil -extract id raw -o - "$result" 2>/dev/null || true)"
if [ "$status" != Accepted ]; then
  echo "error: notarisation of $(basename "$file"): status '${status:-unknown}'" >&2
  if [ -n "$sub_id" ]; then
    xcrun notarytool log "$sub_id" --key "$ASC_KEY_PATH" --key-id "$ASC_KEY_ID" \
      --issuer "$ASC_ISSUER_ID" >&2 || true
  fi
  exit 1
fi
echo "==> $(basename "$file") notarised (submission $sub_id)"
