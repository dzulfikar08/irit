# Changelog

## 0.2.0 (2026-11-10)

The project is now called **irit** (it was leanbio for its first day; a biologics manufacturer
already uses that name). Every tool went through independent code reviews; each fix has a
regression test. Several changes make the tools stricter: they stop with an error where 0.1.0
guessed.

### New
- **irit-mcp**: an MCP server that gives AI agents the irit tools as typed, checked tools. Every
  session writes a run record (`run.json`, an RO-Crate) and a `replay.sh` that reruns it without the
  AI. Outputs are confined to a root folder; downloads need `confirm: true`.
- **gtdbprep**: the laptop half of a split GTDB-Tk. Renames genomes, runs GTDB-Tk's identify and
  align, and writes a small bundle (about 4 KB per genome) that classifies identically to
  `classify_wf` on test genomes. Needs 77 MB of reference data instead of 100 GB.
- **msaview**: a GPU-drawn desktop viewer for very large multiple sequence alignments (FASTA, gzip,
  Clustal, Stockholm). In the releases as `msaview.dmg` for macOS and as Linux x86_64/aarch64
  tarballs.
- **Documentation site**: https://dzulfikar08.github.io/irit/, with an interactive two-species
  competition demo.
- **Validation on GitHub runners**: leanqc against FastQC on real and synthetic data, sheetgen
  against a real nf-core/bacass run, dblock against the live database servers, and a speed
  comparison with FastQC-Rust. Runs weekly and on every change.

### Releases and installing
- **Installer**: `curl -fsSL https://raw.githubusercontent.com/dzulfikar08/irit/main/scripts/install.sh | sh`
  installs the prebuilt tools (and msaview) into `~/.local` without sudo, checks every download
  against `SHA256SUMS`, and `--uninstall` removes exactly what it installed.
- **Signed macOS builds**: the command-line tools are universal binaries (Apple Silicon and Intel),
  signed with a Developer ID and notarised; msaview is a signed, notarised and stapled `msaview.app`
  in a DMG.
- The Linux and macOS archives now include `irit-mcp`, `gtdbprep` and `irit-gtdb-prep`. Windows builds
  (sheetgen, leanqc, isotable, dblock) are not code-signed.
- Every release has `SHA256SUMS` and fixed asset names (`irit-x86_64-linux.tar.gz`, `msaview.dmg`,
  ...) for `releases/latest/download/` links; see [installing and verifying downloads](docs/install.md).

### sheetgen
- Rows are validated against the pipeline's whole schema (the `jsonschema` crate plus nf-schema's
  `uniqueEntries`, `unique` and `exists`). Rules it can't check stop the run unless
  `--allow-unchecked`.
- Mates pair only within one folder. Errors instead of guesses for: two files for one mate, an R2
  without its R1, names that collide after cleaning (`--allow-merge` to accept), ambiguous numbered
  files (`s_1`, `s_2`, `s_3`), 10x R3/R4 next to R1/R2, and long-read files named like mates without
  evidence they are short reads.
- Nanopore: MinKNOW `barcodeNN/`, `_pass_`/`_fail_` and Guppy `fastq_runid_` files are never paired;
  fail reads are skipped (`--include-fail`). Long reads are never silently dropped.
- BGI/MGI/Novogene lanes (`_L1_1`), lowercase `_r1`/`_r2`; `Undetermined_S0_*` skipped
  (`--include-undetermined`); dangling symlinks skipped with a warning.
- New flags: `--no-pair-numeric`, `--allow-merge`, `--allow-unchecked`, `--include-fail`,
  `--include-undetermined`. The binary grew to 6.4 MB (JSON Schema validation).

### leanqc
- Matches FastQC 0.12.1's methods across the whole read (positions beyond 1,000 bp are grouped as
  FastQC groups them), including GC, duplication extrapolation and per-tile quality. Portions are
  ported from FastQC and credited.
- Quality encoding: Phred+64 only when the highest character is between `J` and `i`; `--phred 33|64`
  to override.
- Truncated files are an error (FastQC accepts plain FASTQ cut off mid-line:
  [FastQC#221](https://github.com/s-andrews/FastQC/issues/221)).
- gzip decoding uses zlib-rs: about 12% faster on a 1M-read file, identical output.
- **JSON changes**: positions are groups (`pos`/`end`), adapters have `positions`/`percent`, new
  duplication fields and `warnings`; `positions_capped_at` is gone. `--dup-sample` is replaced by
  `--dup-distinct`.

### isotable
- New statuses `incomplete` (required evidence missing; `--require`, `--min-reads`) and `conflict`
  (two files disagree, shown as `CONFLICT` cells).
- Different samples are never merged: mate suffixes are stripped only from read-level records, and
  prefix matches against a sample sheet need a known suffix.
- fastp: two reports of the same raw reads are a conflict; separate runs are pooled, with Q30 from
  base counts.
- Kraken 2 reports are recognised by content; per-read outputs are ignored.

### dblock
- `verify --deep` hashes every file (SHA-256 recorded at fetch). A changed store folder is not
  reused.
- Locking: concurrent fetches of one database download it once; `remove` waits for the lock and
  checks again. `remove --delete-data` needs `--force` and only deletes marked store folders.
- Uppercase MD5s accepted; catalog ids restricted to plain names; symlinks recorded in manifests
  (0.1.0 markers are accepted and upgraded).

### Upstream
- [GTDBTk#726](https://github.com/Ecogenomics/GTDBTk/issues/726): GTDB R232 mock database doesn't
  match its published MD5.
- [FastQC#221](https://github.com/s-andrews/FastQC/issues/221) / [#222](https://github.com/s-andrews/FastQC/pull/222):
  FASTQ cut off mid-line accepted; fix with a test.

### Licence
- GPL-3.0-or-later (0.1.0 was MIT OR Apache-2.0).

## 0.1.0 (2026-11-10)

First release: sheetgen, leanqc, isotable and dblock.
