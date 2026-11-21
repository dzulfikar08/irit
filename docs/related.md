# Related projects

irit is small and leans on other people's work. This page lists what each irit tool overlaps with,
what it builds on, and where an existing project is the better choice. Versions and dates were
checked on 2026-11-21.

## Use these instead when they fit better

| If you need | Consider | Instead of / next to |
|---|---|---|
| A full metagenome-to-MAG pipeline on a server or cluster | [nf-core/mag](https://nf-co.re/mag) | irit-mcp's step-by-step MAG route. isotable reads nf-core/mag's bin summary, so its MIMAG tiers work on nf-core/mag results too. |
| A full bacterial isolate pipeline | [nf-core/bacass](https://nf-co.re/bacass), [Bactopia](https://bactopia.github.io/) | irit-mcp's isolate route; sheetgen writes their sample sheets. |
| FastQC-identical reports | [FastQC](https://www.bioinformatics.babraham.ac.uk/projects/fastqc/), [falco](https://github.com/smithlabcode/falco), [FastQC-Rust](https://github.com/ewels/FastQC-Rust) | leanqc, which follows FastQC's modules but is not byte-identical. |
| One report over many tools' outputs | [MultiQC](https://multiqc.info/) | leanqc writes a MultiQC table; isotable grades samples rather than plotting them. |
| AMR results from many tools in one format | [hAMRonization](https://github.com/pha4ge/hAMRonization) | isotable reads hAMRonization's output, so any of its supported AMR tools works. |
| Field data collection by a team, with syncing | [ODK Collect](https://getodk.org/), [KoboToolbox](https://www.kobotoolbox.org/) | fieldsheet, which is one phone, offline, with no server. |
| Compositional association statistics in R | [propr](https://github.com/tpq/propr), [SpiecEasi](https://github.com/zdk123/SpiecEasi), [FastSpar](https://github.com/scwatts/fastspar) | cooccur, whose rho matches propr's ([cooccur](tools/cooccur.md)). |
| Recorded, re-runnable workflows | [Nextflow](https://www.nextflow.io/) with [nf-prov](https://github.com/nextflow-io/nf-prov), [Snakemake](https://snakemake.github.io/) | irit-mcp's run record and `replay.sh`, which record tool calls an AI agent made one at a time. |
| Bioinformatics tools for AI agents | [BioContextAI registry](https://biocontext.ai/registry), [Galaxy MCP](https://github.com/galaxyproject/galaxy-mcp) | irit-mcp, which runs local tools with pinned flags and keeps a replayable record. |

## What irit builds on

- **Rust libraries:** [needletail](https://github.com/onecodex/needletail) (FASTQ/FASTA reading in
  leanqc), [rmcp](https://github.com/modelcontextprotocol/rust-sdk) (irit-mcp's MCP server),
  [jsonschema](https://github.com/Stranger6667/jsonschema) (sheetgen), [GPUI](https://www.gpui.rs/)
  (msascope), flate2 with zlib-rs, clap, serde, rayon and others listed in each `Cargo.toml`.
- **The tools irit-mcp runs:** fastp, MEGAHIT, Shovill, Flye, Medaka, Rasusa, CoverM, MetaBAT 2,
  SemiBin2, galah, CheckM2, GTDB-Tk, gPlace, skani, AMRFinderPlus, mlst, Kleborate and Bakta. Each
  is installed separately and keeps its own licence; [irit-mcp](tools/mcp.md) lists the version
  each wrapper was checked against. Cite them in papers, not only irit.
- **Data:** GTDB, CheckM2's database, AMRFinderPlus's database, PubMLST schemes and others that
  [dblock](tools/dblock.md) fetches, each with its own licence.

## The wider Rust bioinformatics ecosystem

- [rust-bio](https://github.com/rust-bio/rust-bio): algorithms and data structures (alignment,
  FM-index, pattern matching) and readers for common formats. irit doesn't use it today: its file
  reading is covered by needletail and small parsers of tool outputs.
- [noodles](https://github.com/zaeleus/noodles): pure-Rust readers for BAM, CRAM, VCF, BGZF and
  others; the choice if irit ever reads alignments directly.
- Rust tools worth knowing: [sylph](https://github.com/bluenote-1577/sylph) (fast profiling),
  [skani](https://github.com/bluenote-1577/skani) (ANI), [Deacon](https://github.com/bede/deacon)
  (host read removal), [chopper](https://github.com/wdecoster/chopper) and
  [nanoq](https://github.com/esteinig/nanoq) (long-read QC), [Autocycler](https://github.com/rrwick/Autocycler).

## Lists of tools

- [Awesome-Bioinformatics](https://github.com/danielecook/Awesome-Bioinformatics): a curated list
  by topic.
- [bio.tools](https://bio.tools/): the ELIXIR registry of bioinformatics software.
- [Bioconda](https://bioconda.github.io/): where most of the tools above are installed from.
