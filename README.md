# leanbio

Small, fast command-line tools for the manual steps around sequencing pipelines, built to run on
ordinary laptops. Each tool is one static Rust binary: no Java, no conda, no containers.

| Tool | What it does | Replaces | Binary |
|---|---|---|---|
| [`sheetgen`](docs/tools/sheetgen.md) | Builds a pipeline's sample sheet from a folder of FASTQ files and checks it against the pipeline's own schema | Hand-written CSVs, per-pipeline scripts | 2.4 MB |
| [`leanqc`](docs/tools/leanqc.md) | Read quality control with an offline HTML report | FastQC | 1.6 MB |
| [`isotable`](docs/tools/isotable.md) | Joins fastp, leanqc, QUAST, BUSCO, CheckM2, GTDB-Tk, Kraken 2, mlst and AMRFinderPlus outputs into one graded row per sample (isolates or MAGs) | Per-pipeline summary scripts | 1.5 MB |
| [`dblock`](docs/tools/dblock.md) | Downloads, verifies and pins reference databases (GTDB-Tk, CheckM2, SILVA, Kraken 2, Bakta, CARD) with a lock file | Each tool's own downloader | 5.1 MB |

Measured against FastQC 0.12.1 on 1 million reads: **2.5 s and 16 MB of RAM** against 7.0 s and
483 MB, with the same pass/warn/fail verdict on every check. See [benchmarks](docs/benchmarks.md).

## Install

```sh
cargo install --git https://github.com/dzulfikar08/leanbio sheetgen leanqc isotable dblock
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
the choices, and the [roadmap](docs/roadmap.md). The folder is an [mdBook](https://rust-lang.github.io/mdBook/)
source (`mdbook serve` from the repository root) so it can be published as a site.

## Licence

GPL-3.0-or-later (see [LICENSE](LICENSE)). You can use, study, change and share the tools freely;
if you distribute a modified version, it must stay under the GPL with its source available.

The databases `dblock` downloads have their own licences; `dblock catalog` flags the ones that
restrict commercial use.
