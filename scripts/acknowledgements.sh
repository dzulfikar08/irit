#!/usr/bin/env bash
# acknowledgements.sh OUT.html: the licences of everything compiled into msascope, as one HTML page
# (shipped as Acknowledgements.html in msascope.app's Resources and next to the Linux binary;
# Help > Acknowledgements opens it).
# Crate licences come from cargo-about (msascope/about.toml, msascope/about/acknowledgements.hbs);
# the bundled Lucide icons and the Catppuccin palettes, which have no crate of their own, are
# added from gpui-kit-assets' LICENSE-LUCIDE and msascope/THIRD_PARTY.md.
# Needs cargo-about: cargo install cargo-about --locked --features cli
set -euo pipefail
out="${1:?usage: $0 OUT.html}"
root="$(cd "$(dirname "$0")/.." && pwd)"
command -v cargo-about >/dev/null || { echo "error: cargo-about missing (cargo install cargo-about --locked --features cli)" >&2; exit 1; }

mkdir -p "$(dirname "$out")"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
cargo about generate --manifest-path "$root/msascope/Cargo.toml" -c "$root/msascope/about.toml" \
  --features gui --fail "$root/msascope/about/acknowledgements.hbs" -o "$work/crates.html"

# gpui-kit-assets' manifest directory, for its LICENSE-LUCIDE.
assets_dir="$(cargo metadata --manifest-path "$root/msascope/Cargo.toml" --format-version 1 --locked |
  python3 -c 'import json,sys,os; m=json.load(sys.stdin); print(next(os.path.dirname(p["manifest_path"]) for p in m["packages"] if p["name"]=="gpui-kit-assets"))')"
lucide="$assets_dir/LICENSE-LUCIDE"
[ -f "$lucide" ] || { echo "error: $lucide missing" >&2; exit 1; }

python3 - "$work/crates.html" "$lucide" "$root/msascope/THIRD_PARTY.md" "$out" <<'EOF'
import html, re, sys
page, lucide, third, out = sys.argv[1:]
text = open(page, encoding="utf-8").read()
md = open(third, encoding="utf-8").read()
# The Catppuccin section of THIRD_PARTY.md: its licence is the quoted ("> ") block.
cat = re.search(r"## Catppuccin.*?\n((?:>.*\n)+)", md, re.S)
if not cat:
    sys.exit("error: no Catppuccin licence block in THIRD_PARTY.md")
cat_text = "\n".join(l[2:] if l.startswith("> ") else l[1:] for l in cat.group(1).splitlines())
extra = (
    "<h2>Lucide icons (ISC License)</h2>\n<p>The icons in msascope's interface, bundled by gpui-kit-assets.</p>\n"
    f"<pre>{html.escape(open(lucide, encoding='utf-8').read())}</pre>\n"
    "<h2>Catppuccin colour palettes (MIT License)</h2>\n<p>The Latte and Mocha palettes of the light and dark themes.</p>\n"
    f"<pre>{html.escape(cat_text)}</pre>\n"
)
if "<!--EXTRA-->" not in text:
    sys.exit("error: template has no <!--EXTRA--> marker")
open(out, "w", encoding="utf-8").write(text.replace("<!--EXTRA-->", extra))
EOF
echo "==> $out ($(grep -c '<h2>' "$out") licence sections)"
