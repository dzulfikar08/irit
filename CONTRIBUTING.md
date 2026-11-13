# Contributing to irit

Issues and pull requests are welcome: https://github.com/dzulfikar08/irit

## Building and testing

`cargo test --workspace` runs the tests of every tool except msascope's GUI; msascope needs the
system libraries listed in [its guide](docs/tools/msascope.md#building). `cargo clippy --all-targets
-- -D warnings` and `cargo fmt --check` must pass.

## Licence of contributions

irit is GPL-3.0-or-later, and contributions are accepted under that licence.

**msascope is the exception.** Besides the GPL source here, its author also publishes a Mac App
Store edition under a separate licence, which is only possible while they hold the copyright in
all of msascope's code. So a pull request that changes `msascope/` (or the design-system files it
shares with Kantor, listed in `msascope/THIRD_PARTY.md`) can only be merged with a signed agreement
that lets the author publish your change under other licences too, as well as under the GPL. The
pull request will say how to sign it. Changes to the other tools don't need it.

New dependencies of msascope must be permissively licensed (MIT, Apache-2.0, BSD, ISC, Zlib and
the like): `msascope/deny.toml` lists them and CI checks it.
