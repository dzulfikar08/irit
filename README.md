# irit

*irit* (Indonesian and Javanese): frugal, economical with what you have.

Small, fast tools for the manual steps around sequencing pipelines, built to run on ordinary laptops.
Each command-line tool is one static Rust binary: no Java, no conda, no containers.

| Tool | What it does | Replaces | Binary |
|---|---|---|---|
| [`sheetgen`](docs/tools/sheetgen.md) | Builds a pipeline's sample sheet from a folder of FASTQ files and checks it against the pipeline's own schema | Hand-written CSVs, per-pipeline scripts | 6.4 MB |
| [`leanqc`](docs/tools/leanqc.md) | Read quality control with an offline HTML report | FastQC | 1.6 MB |
| [`isotable`](docs/tools/isotable.md) | Joins fastp, leanqc, QUAST, BUSCO, CheckM2, GTDB-Tk, Kraken 2, mlst and AMRFinderPlus outputs into one graded row per sample (isolates or MAGs) | Per-pipeline summary scripts | 2.6 MB |
| [`dblock`](docs/tools/dblock.md) | Downloads, verifies and pins reference databases (GTDB-Tk, CheckM2, SILVA, Kraken 2, Bakta, CARD) with a lock file | Each tool's own downloader | 4.9 MB |
| [`seqplan`](docs/tools/seqplan.md) | Plans a sequencing run before you sequence: isolates per run and cost per isolate at a target coverage, from Illumina's specifications and reported nanopore yields | Spreadsheets and vendor calculators | — |
| [`irit-mcp`](docs/tools/mcp.md) | Gives AI agents (any MCP client) the irit tools as typed, checked tools, and records every call with a `replay.sh` that reruns it without the AI | Agents calling CLIs through a free-form shell | 3.1 MB |
| [`gtdbprep`](docs/tools/gtdbprep.md) | Laptop half of a split GTDB-Tk: renames genomes, runs identify + align, and writes a ~4 KB-per-genome bundle for placement elsewhere | Downloading the 100 GB GTDB-Tk package | 1.6 MB |
| [`msascope`](docs/tools/msascope.md) | GPU-drawn desktop viewer for very large multiple sequence alignments (100,000 × 30,000 tested) | Jalview, AliView, SeaView | desktop app |

Measured against FastQC 0.12.1 on a 1-million-read nf-core/bacass test file: **1.5 s and 13 MB of
RAM** against 7.0 s and 483 MB. On the four bacass test files, leanqc gives FastQC's verdict for all
40 graded module results (ten modules × four files); on a synthetic corpus of ten more files, 88 of
90 agree, and the two differences are a deliberate fix to FastQC's quality-encoding guess. See
[benchmarks](docs/benchmarks.md).

If you need FastQC's exact output, byte for byte, use [FastQC-Rust](https://github.com/ewels/FastQC-Rust)
(Phil Ewels' faithful Rust rewrite) or FastQC itself. leanqc is irit's built-in, smallest-footprint
QC; see [how they compare](docs/tools/leanqc.md#leanqc-and-fastqc-rust).

## Install

Linux and macOS, no sudo (installs into `~/.local/bin`; on macOS msascope goes to `~/Applications`):

```sh
curl -fsSL https://raw.githubusercontent.com/dzulfikar08/irit/main/scripts/install.sh | sh
```

`sh -s -- --no-gui` skips msascope, `sh -s -- --uninstall` removes what it installed, and
`IRIT_VERSION=v0.2.0` in front of `sh` picks a release.

Or download from the [latest release](https://github.com/dzulfikar08/irit/releases/latest):

- **macOS** (Apple Silicon and Intel): `msascope.dmg` and `irit-universal-macos.tar.gz`, signed with a
  Developer ID and notarised by Apple.
- **Linux** (x86_64, ARM64): `irit-<arch>-linux.tar.gz`, statically linked, runs on old distributions
  too; `msascope-<arch>-linux.tar.gz` needs glibc 2.39 or newer.
- **Windows** (x86_64): `irit-x86_64-windows.zip` with sheetgen, leanqc, isotable, dblock and seqplan. Not
  code-signed, so SmartScreen may warn the first time.

Check downloads against the release's `SHA256SUMS`; see
[installing and verifying downloads](docs/install.md) for that and for checking the macOS signatures.

Or build with Rust:

```sh
cargo install --git https://github.com/dzulfikar08/irit sheetgen leanqc isotable dblock seqplan irit-mcp gtdbprep
```

or build from a checkout with `cargo build --release` (binaries in `target/release/`).

## Quick start

```sh
# 1. sample sheet for nf-core/bacass, using the pipeline's own column rules
sheetgen raw_reads/ --pipeline nf-core/bacass -o samplesheet.tsv

# 2. quality control, one HTML report per file
leanqc raw_reads/*.fastq.gz -o qc/

# 3. one graded row per sample from all the tool outputs
isotable results/ --sheet samplesheet.tsv -o qc_table.tsv

# 4. reference databases, checked for free disk space and pinned in dblock.lock
dblock add checkm2-db-v3 kraken2-standard-8gb-20261022
dblock plan
dblock fetch
eval "$(dblock env)"
```

## Documentation

Everything is in [`docs/`](docs/introduction.md): tool guides, benchmarks, the research behind
the choices, and the [roadmap](docs/roadmap.md). The documentation site in `site/` is built from
that folder.

## People

- **Dzulfikar Ats Tsauri** ([@dzulfikar08](https://github.com/dzulfikar08)): builds the tools.
- **Naufal Rafif Ramadhan** ([@Melchior132112](https://github.com/Melchior132112)), microbiologist:
  scientific validation (QC thresholds, biological assumptions, testing on real data).

## Acknowledgements

- **[FastQC](https://www.bioinformatics.babraham.ac.uk/projects/fastqc/)** 0.12.1, Copyright
  2010-17 Simon Andrews, Babraham Bioinformatics (GPL v3 or later). leanqc includes code
  translated from FastQC (position groups, GC model, duplication estimate, per-sequence GC fit,
  per-tile quality, adapter content, encoding detection) and uses its default adapter list and
  thresholds; see [leanqc](docs/tools/leanqc.md#credit). Each ported function in
  `leanqc/src/stats.rs` names the FastQC class it comes from.
- **[nf-core](https://nf-co.re/)** (MIT): sheetgen reads pipelines' `schema_input.json` and follows
  [nf-schema](https://github.com/nextflow-io/nf-schema)'s rules for sample sheets; copies of five
  nf-core schemas are test fixtures ([sources](sheetgen/tests/fixtures/README.md)). The validation
  workflow uses nf-core/bacass and its [test datasets](https://github.com/nf-core/test-datasets).
- isotable reads the output formats of fastp, QUAST, BUSCO, CheckM2, GTDB-Tk, Kraken 2, mlst and
  AMRFinderPlus, and grades MAGs by the MIMAG standard (Bowers et al., *Nature Biotechnology*
  35, 725-731, 2017). No code from those tools is included.
- dblock's catalog points at databases published by their authors (GTDB, CheckM2, SILVA, Kraken 2
  index builds, Bakta, CARD); each entry records its source and licence.

## Licence

GPL-3.0-or-later (see [LICENSE](LICENSE)). You can use, study, change and share the tools freely;
if you distribute a modified version, it must stay under the GPL with its source available.

The databases `dblock` downloads have their own licences; `dblock catalog` flags the ones that
restrict commercial use.
