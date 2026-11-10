# Installing and verifying downloads

## The installer (Linux and macOS)

```sh
curl -fsSL https://raw.githubusercontent.com/dzulfikar08/irit/main/scripts/install.sh | sh
```

It downloads the latest release from GitHub, checks each download against the release's
`SHA256SUMS`, and installs without sudo:

- the command-line tools (`sheetgen`, `leanqc`, `isotable`, `dblock`, `irit-mcp`, `gtdbprep`,
  `irit-gtdb-prep`) into `~/.local/bin`;
- msaview: on macOS `~/Applications/msaview.app` plus an `msaview` command in `~/.local/bin`; on
  Linux `~/.local/bin/msaview`.

If `~/.local/bin` is not on your `PATH`, the installer prints the line to add to your shell's
startup file. Options:

| | |
|---|---|
| `IRIT_VERSION=v0.2.0` | install that release instead of the latest (in front of `sh`: `... \| IRIT_VERSION=v0.2.0 sh`) |
| `PREFIX=/opt/irit` | install into `/opt/irit/bin` instead of `~/.local/bin` |
| `IRIT_APPS=/Applications` | macOS: where msaview.app goes (default `~/Applications`) |
| `sh -s -- --no-gui` | command-line tools only, no msaview |
| `sh -s -- --uninstall` | remove what the installer put there |

The installer lists what it installed in `~/.local/share/irit/installed-files` (under `PREFIX`).
`--uninstall` removes only those files, and leaves any that changed since (a `sheetgen` you built
yourself into the same folder, say).

The installer works for releases from 0.2.0 on. For 0.1.0, download an archive by hand from its
[release page](https://github.com/dzulfikar08/irit/releases/tag/v0.1.0).

## Downloads

Every [release](https://github.com/dzulfikar08/irit/releases) has these files, each under a
versioned name (`irit-0.2.0-x86_64-linux.tar.gz`) and a fixed one. The fixed names always
point at the latest release, as
`https://github.com/dzulfikar08/irit/releases/latest/download/<name>`:

| File | What | Signed |
|---|---|---|
| `irit-x86_64-linux.tar.gz`, `irit-aarch64-linux.tar.gz` | command-line tools, statically linked (any distribution) | no (Linux has no equivalent; check SHA256SUMS) |
| `irit-universal-macos.tar.gz` | command-line tools for Apple Silicon and Intel Macs | Developer ID, notarised |
| `msaview.dmg` | msaview.app for Apple Silicon and Intel Macs | Developer ID, notarised, ticket stapled |
| `msaview-universal-macos.tar.gz` | the same msaview.app as a tarball (used by the installer) | as above |
| `msaview-x86_64-linux.tar.gz`, `msaview-aarch64-linux.tar.gz` | msaview for Linux; needs glibc 2.39 or newer (Ubuntu 24.04, Debian 13, Fedora 40 or later) | no |
| `irit-x86_64-windows.zip` | `sheetgen`, `leanqc`, `isotable`, `dblock` for Windows | **no**: SmartScreen may warn the first time |
| `SHA256SUMS` | SHA-256 of every file above | |

`irit-mcp` and `gtdbprep` use Unix-only APIs, so the Windows archive doesn't have them. Or
build from source with Rust:

```sh
cargo install --git https://github.com/dzulfikar08/irit sheetgen leanqc isotable dblock irit-mcp gtdbprep
```

## Verifying a download

### Checksums (any system)

Download `SHA256SUMS` from the same release into the folder with your downloads, then:

```sh
sha256sum -c SHA256SUMS --ignore-missing            # Linux
shasum -a 256 -c SHA256SUMS --ignore-missing        # macOS
```

Each file you downloaded should be reported `OK`. On Windows (PowerShell):
`Get-FileHash irit-x86_64-windows.zip` and compare with its line in `SHA256SUMS`.

The checksums come from the same GitHub release as the files, so they catch a broken or
swapped download, not a compromised release. The macOS signatures below are independent of
GitHub.

### macOS signatures and notarisation

The macOS builds are signed with a Developer ID Application certificate, with the hardened
runtime and a secure timestamp, and notarised by Apple. To check them yourself:

```sh
# The DMG: signed, notarised, ticket stapled.
spctl --assess --type open --context context:primary-signature --verbose=2 msaview-*.dmg
xcrun stapler validate msaview-*.dmg

# The app (in the DMG, or after installing).
codesign --verify --strict --verbose=2 ~/Applications/msaview.app
spctl --assess --type execute --verbose=2 ~/Applications/msaview.app
#   -> accepted   source=Notarized Developer ID

# A command-line tool.
codesign --verify --strict --verbose=2 ~/.local/bin/leanqc
codesign --display --verbose=2 ~/.local/bin/leanqc 2>&1 | grep -E 'Authority|TeamIdentifier|Runtime|Timestamp'
```

`codesign --display` should show `Authority=Developer ID Application: ...`, a `TeamIdentifier`,
the hardened runtime (`Runtime Version`) and a `Timestamp`. The command-line tools are notarised
too, but a ticket can't be stapled to a bare binary: macOS looks it up online the first time a
quarantined copy runs (archives fetched with `curl`, as the installer does, aren't quarantined).

## How releases are built

[`.github/workflows/release.yml`](../.github/workflows/release.yml) builds every archive on
GitHub's runners when a `v*` tag is pushed: the Linux tools with `cross` (musl), msaview on Ubuntu
24.04, the macOS binaries for both architectures joined with `lipo`, then signed, notarised and
checked (`scripts/sign-macos.sh`, `scripts/build-dmg.sh`). The signing keys live in a GitHub
environment that only the main branch and release tags can use.
