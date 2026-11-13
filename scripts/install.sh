#!/bin/sh
# Install the prebuilt irit tools from GitHub Releases (no compiling, no sudo).
#   curl -fsSL https://raw.githubusercontent.com/dzulfikar08/irit/main/scripts/install.sh | sh
#   curl -fsSL .../install.sh | IRIT_VERSION=v0.2.0 sh    # a given release (default: the latest)
#   curl -fsSL .../install.sh | sh -s -- --no-gui          # command-line tools only, no msascope
#   curl -fsSL .../install.sh | sh -s -- --uninstall       # remove what this script installed
#
# Installs sheetgen, leanqc, isotable, dblock, seqplan, irit-mcp, gtdbprep and irit-gtdb-prep into
# $PREFIX/bin (PREFIX defaults to ~/.local), plus the msascope viewer:
#   macOS: $IRIT_APPS/msascope.app (default ~/Applications; signed and notarised) and a
#          $PREFIX/bin/msascope link to it.
#   Linux: $PREFIX/bin/msascope (x86_64 and aarch64; needs glibc 2.39+ and a GPU driver).
# Releases up to v0.2.1 call the viewer msaview; for those it is installed under that name.
# Installing one name removes the other if this script installed it (msaview -> msascope).
# Linux command-line tools are static (musl) and run on any distribution. Every download is
# checked against the release's SHA256SUMS. Windows: download irit-x86_64-windows.zip from
# https://github.com/dzulfikar08/irit/releases instead. Prebuilt archives in this layout exist
# from v0.2.0 on.
#
# IRIT_DOWNLOAD_BASE replaces https://github.com/dzulfikar08/irit/releases/download/<tag> (a
# mirror, or a local folder as file:///path for testing).
# The installed files are listed in $PREFIX/share/irit/installed-files; --uninstall removes
# only those, and only while they are still the files this script put there.
set -eu
repo="dzulfikar08/irit"
prefix="${PREFIX:-$HOME/.local}"
bindir="$prefix/bin"
apps="${IRIT_APPS:-$HOME/Applications}"
manifest="$prefix/share/irit/installed-files"
clis="sheetgen leanqc isotable dblock seqplan irit-mcp gtdbprep irit-gtdb-prep"
# Viewer names: msascope in releases after v0.2.1, msaview up to v0.2.1. The bundle id goes
# with the name.
viewers="msascope msaview"
app_id_of() { echo "com.dzulfikar.irit.$1"; }

usage() {
  cat <<EOF
usage: install.sh [--no-gui] [--uninstall] [-h|--help]

Installs the irit command-line tools into \$PREFIX/bin (now $bindir) and the msascope viewer
(macOS: $apps/msascope.app). IRIT_VERSION=vX.Y.Z picks a release (default: the latest).

  --no-gui     skip msascope
  --uninstall  remove the files this script installed (listed in $manifest)
  -h, --help   this help

  curl -fsSL https://raw.githubusercontent.com/${repo}/main/scripts/install.sh | sh
EOF
}

die() { echo "error: $*" >&2; exit 1; }
note() { echo "note: $*" >&2; }

uninstall=0 gui=1
for arg in "$@"; do
  case "$arg" in
    --uninstall) uninstall=1 ;;
    --no-gui) gui=0 ;;
    -h|--help) usage; exit 0 ;;
    *) usage >&2; die "unknown option: $arg" ;;
  esac
done

case "$(uname -s)" in
  Darwin) os=macos ;;
  Linux) os=linux ;;
  MINGW*|MSYS*|CYGWIN*) die "on Windows, download irit-x86_64-windows.zip from https://github.com/${repo}/releases/latest" ;;
  *) die "no prebuilt irit for $(uname -s); build it with Rust: cargo install --git https://github.com/${repo} sheetgen leanqc isotable dblock seqplan" ;;
esac

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  elif command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | cut -d' ' -f1
  else die "need sha256sum or shasum to check the downloads"
  fi
}

is_ours_app() { # path to NAME.app: ours if its Info.plist names irit's bundle id for NAME
  n="$(basename "$1" .app)"
  [ -f "$1/Contents/Info.plist" ] && grep -q "<string>$(app_id_of "$n")</string>" "$1/Contents/Info.plist"
}

is_viewer() { # NAME: msascope, msaview or their .app
  case " $viewers " in *" ${1%.app} "*) return 0 ;; esac
  return 1
}

# remove_entry KIND PATH: remove one manifest entry if it is still what this script put there.
remove_entry() {
  kind="$1" path="$2"
  name="$(basename "$path")"
  case " $clis " in
    *" $name "*) ;;
    *) is_viewer "$name" || { note "skipping $path: not an irit file"; return 0; } ;;
  esac
  if [ ! -e "$path" ] && [ ! -L "$path" ]; then return 0; fi
  case "$kind" in
    link)
      if [ -L "$path" ]; then
        case "$(readlink "$path")" in
          */"$name.app/Contents/MacOS/$name") rm -f "$path"; echo "removed $path" ;;
          *) note "kept $path: it no longer points into $name.app" ;;
        esac
      else
        note "kept $path: it was replaced by something else"
      fi ;;
    app)
      if is_ours_app "$path"; then rm -rf "$path"; echo "removed $path"
      else note "kept $path: not irit's $name"; fi ;;
    *)
      if [ -f "$path" ] && [ ! -L "$path" ] && [ "$(sha256 "$path")" = "$kind" ]; then
        rm -f "$path"; echo "removed $path"
      else
        note "kept $path: it changed since this script installed it"
      fi ;;
  esac
}

# --- uninstall ------------------------------------------------------------------------------
if [ "$uninstall" = 1 ]; then
  if [ ! -f "$manifest" ]; then
    echo "Nothing to remove: no irit installed by this script under $prefix ($manifest not found)."
    exit 0
  fi
  # Each line: "<sha256> <path>" for a file, "link <path>" for the viewer's link, "app <path>".
  while read -r kind path; do
    [ -n "$path" ] || continue
    remove_entry "$kind" "$path"
  done < "$manifest"
  rm -f "$manifest"
  rmdir "$prefix/share/irit" "$prefix/share" 2>/dev/null || true
  echo "irit uninstalled."
  exit 0
fi

# --- install --------------------------------------------------------------------------------
command -v curl >/dev/null 2>&1 || die "curl is needed"
command -v tar >/dev/null 2>&1 || die "tar is needed"
case "$(uname -m)" in
  aarch64|arm64) arch=aarch64 ;;
  x86_64|amd64) arch=x86_64 ;;
  *) die "no prebuilt irit for $(uname -m) $os; build it with Rust: cargo install --git https://github.com/${repo} sheetgen leanqc isotable dblock seqplan" ;;
esac
if [ "$os" = macos ]; then
  cli_asset="irit-universal-macos.tar.gz"
  gui_suffix="universal-macos.tar.gz"
else
  cli_asset="irit-${arch}-linux.tar.gz"
  gui_suffix="${arch}-linux.tar.gz"
fi

tag="${IRIT_VERSION:-}"
case "$tag" in
  ""|latest)
    # github.com/OWNER/REPO/releases/latest redirects to .../releases/tag/<latest tag>.
    url="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "https://github.com/${repo}/releases/latest")" \
      || die "could not reach github.com to find the latest irit release"
    case "$url" in
      */releases/tag/*) tag="${url##*/releases/tag/}" ;;
      *) die "irit has no published release yet (see https://github.com/${repo}/releases)" ;;
    esac ;;
  v[0-9]*) ;;
  [0-9]*) tag="v$tag" ;;
  *) die "IRIT_VERSION must be a release tag like v0.2.0 (got '$tag')" ;;
esac
base="${IRIT_DOWNLOAD_BASE:-https://github.com/${repo}/releases/download/${tag}}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
trap 'exit 1' INT TERM

echo "irit $tag for $os ($arch)"
if ! curl -fsL -o "$tmp/SHA256SUMS" "$base/SHA256SUMS"; then
  if ! curl -fsIL -o /dev/null "https://github.com/${repo}/releases/tag/${tag}"; then
    die "no irit release '$tag' (see https://github.com/${repo}/releases)"
  fi
  echo "error: irit $tag was released before this installer existed: it has no SHA256SUMS or" >&2
  echo "       ${cli_asset}. Releases from v0.2.0 on have them. For $tag, download an archive" >&2
  echo "       by hand from https://github.com/${repo}/releases/tag/${tag}, or build it with Rust:" >&2
  echo "       cargo install --git https://github.com/${repo} --tag ${tag} sheetgen leanqc isotable dblock seqplan" >&2
  exit 1
fi

checksum_of() { awk -v f="$1" '$2 == f || $2 == "*" f { print $1; exit }' "$tmp/SHA256SUMS"; }
# listed FILE PATH: is PATH the path of a "<kind> <path>" line in FILE?
listed() { [ -f "$1" ] && awk -v p="$2" '{ k = $1; sub(/^[^ ]+ /, ""); if ($0 == p) f = 1 } END { exit !f }' "$1"; }

# fetch ASSET DIR: download, check against SHA256SUMS, unpack into DIR; prints the folder inside.
# Callers check that SHA256SUMS lists ASSET first.
fetch() {
  want="$(checksum_of "$1")"
  echo "Downloading $base/$1" >&2
  curl -fL --progress-bar -o "$tmp/$1" "$base/$1" || die "download of $1 failed"
  got="$(sha256 "$tmp/$1")"
  [ "$got" = "$want" ] || die "checksum mismatch for $1 (expected $want, got $got): not installing"
  mkdir -p "$2"
  tar -C "$2" -xzf "$tmp/$1"
  for d in "$2"/*/; do [ -d "$d" ] && { printf '%s\n' "${d%/}"; return 0; }; done
  die "$1 holds no folder"
}

# put SRC DEST: copy into place through a temporary name, so a running copy isn't disturbed.
put() {
  if [ -e "$2" ] && ! listed "$manifest" "$2"; then note "replacing $2 (not installed by this script)"; fi
  cp "$1" "$2.irit-new"
  chmod 755 "$2.irit-new"
  mv -f "$2.irit-new" "$2"
  printf '%s %s\n' "$(sha256 "$2")" "$2" >> "$tmp/new-manifest"
}

: > "$tmp/new-manifest"
mkdir -p "$bindir" "$(dirname "$manifest")"

[ -n "$(checksum_of "$cli_asset")" ] || die "irit $tag has no build for $os $arch (no $cli_asset in https://github.com/${repo}/releases/tag/${tag})"
cli_dir="$(fetch "$cli_asset" "$tmp/cli")" || exit 1
installed=""
for b in $clis; do
  if [ -f "$cli_dir/$b" ]; then put "$cli_dir/$b" "$bindir/$b"; installed="$installed $b"
  else note "$cli_asset has no $b; skipped"; fi
done

gui_done=""
gui_name=""
if [ "$gui" = 1 ]; then
  # The release's viewer: msascope, or msaview in releases up to v0.2.1.
  for v in $viewers; do
    if [ -n "$(checksum_of "$v-$gui_suffix")" ]; then gui_name="$v"; break; fi
  done
  if [ -n "$gui_name" ]; then
    g="$gui_name"
    gui_asset="$g-$gui_suffix"
    gui_dir="$(fetch "$gui_asset" "$tmp/gui")" || exit 1
    if [ "$os" = macos ]; then
      [ -d "$gui_dir/$g.app" ] || die "$gui_asset holds no $g.app"
      if [ -e "$apps/$g.app" ] && ! is_ours_app "$apps/$g.app"; then
        note "$apps/$g.app is not irit's $g: left alone, $g not installed"
      else
        mkdir -p "$apps"
        rm -rf "$apps/$g.app"
        if command -v ditto >/dev/null 2>&1; then ditto "$gui_dir/$g.app" "$apps/$g.app"
        else cp -R "$gui_dir/$g.app" "$apps/$g.app"; fi
        printf 'app %s\n' "$apps/$g.app" >> "$tmp/new-manifest"
        if [ -e "$bindir/$g" ] && [ ! -L "$bindir/$g" ]; then note "replacing $bindir/$g with a link to the app"; fi
        ln -sf "$apps/$g.app/Contents/MacOS/$g" "$bindir/$g"
        printf 'link %s\n' "$bindir/$g" >> "$tmp/new-manifest"
        gui_done="$apps/$g.app (command: $g)"
      fi
    else
      [ -f "$gui_dir/$g" ] || die "$gui_asset holds no $g"
      put "$gui_dir/$g" "$bindir/$g"
      gui_done="$bindir/$g"
      if ! "$bindir/$g" --help >/dev/null 2>&1; then
        note "$g was installed but does not start on this system. It needs glibc 2.39 or newer"
        note "(Ubuntu 24.04, Debian 13, Fedora 40 or later) and libxkbcommon, fontconfig and Vulkan."
      fi
    fi
  else
    note "irit $tag has no viewer build for $os $arch (msascope-$gui_suffix); installed the command-line tools only"
  fi
fi

# The viewer under its other name (msaview, before the rename), if this script installed it.
if [ -n "$gui_done" ] && [ -f "$manifest" ]; then
  while read -r kind path; do
    [ -n "$path" ] || continue
    n="$(basename "$path")"
    if is_viewer "$n" && [ "${n%.app}" != "$gui_name" ]; then remove_entry "$kind" "$path"; fi
  done < "$manifest"
fi

# Keep entries from earlier installs that weren't replaced now (e.g. msascope before --no-gui).
if [ -f "$manifest" ]; then
  while read -r kind path; do
    [ -n "$path" ] || continue
    listed "$tmp/new-manifest" "$path" && continue
    if [ -e "$path" ] || [ -L "$path" ]; then printf '%s %s\n' "$kind" "$path" >> "$tmp/new-manifest"; fi
  done < "$manifest"
fi
cp "$tmp/new-manifest" "$manifest"

echo
echo "Installed irit $tag into $bindir:$installed"
[ -z "$gui_done" ] || echo "$gui_name: $gui_done"
"$bindir/leanqc" --version 2>/dev/null || true

case ":$PATH:" in
  *":$bindir:"*) ;;
  *)
    echo
    echo "$bindir is not on your PATH, so the commands won't be found yet. Add it:"
    case "$(basename "${SHELL:-sh}")" in
      zsh)  echo "  echo 'export PATH=\"$bindir:\$PATH\"' >> ~/.zshrc && exec zsh" ;;
      bash) if [ "$os" = macos ]; then rc=.bash_profile; else rc=.bashrc; fi
            echo "  echo 'export PATH=\"$bindir:\$PATH\"' >> ~/$rc && exec bash" ;;
      fish) echo "  fish_add_path $bindir" ;;
      *)    echo "  export PATH=\"$bindir:\$PATH\"   (in your shell's startup file)" ;;
    esac ;;
esac
echo "Uninstall: curl -fsSL https://raw.githubusercontent.com/${repo}/main/scripts/install.sh | sh -s -- --uninstall"
