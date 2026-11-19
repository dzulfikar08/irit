# Introduction

irit is a set of small tools for the parts of sequencing analysis that pipelines leave to the
researcher: building sample sheets and checking read files, read quality control, planning
sequencing runs, getting the right reference databases onto disk, joining results into one row per
sample, GTDB classification without the 100 GB download, and which genomes rise and fall together
across samples. An MCP server lets AI agents run all of it with a replayable record, and two
companions cover the rest: msascope, a viewer for very large alignments, and fieldsheet, an offline
phone worksheet for sample metadata. The command-line tools are written in Rust, ship as single
binaries, and are meant to run on the laptop a student actually owns, not only on a cluster.

The name is the Indonesian and Javanese word for frugal: getting the most out of modest resources.
The project was called leanbio for its first day, until it turned out a biologics manufacturer
already uses that name.

## Why

Two things came out of the research behind this project ([landscape](research/landscape.md)):

1. **The heavy science is mostly solved; the glue isn't.** Assemblers, classifiers and annotators
   are mature. What still costs researchers hours is the manual work around them: writing sample
   sheets by hand, fetching and matching database versions, collecting QC into one table.
2. **The standard tooling assumes big machines.** Running the nf-core/bacass test profile on a
   laptop needed only 3.1 GB of RAM, but pulled about 16 GB of container images to produce 78 MB
   of results ([setup notes](lab/setup.md)). FastQC alone used 483 MB of RAM for one small file.
   GTDB-Tk now needs about 140 GB of RAM ([details](research/gtdbtk-memory.md)); irit's
   [`gtdbprep`](tools/gtdbprep.md) gets the same species calls and tree placements in under 8 GB.

So each tool here does one manual step, does it on modest hardware, and is checked against the
established tool it replaces or feeds.

## Principles

- **Match the reference, then beat it on resources.** `leanqc` reproduces FastQC's verdicts before
  claiming to be faster. `sheetgen`'s output is validated by running the real pipeline.
- **Refuse rather than guess.** `sheetgen` stops on values the pipeline would reject; `dblock`
  deletes a download whose checksum doesn't match and refuses databases that won't fit on disk.
- **Record what was used.** `dblock.lock` and the JSON outputs are there so an analysis can be
  repeated exactly.
- **Measured claims only.** Every number in these docs comes from a run described alongside it.

## Status

Early. The tools work and are tested on real data (see [benchmarks](benchmarks.md)), but
they have one test machine so far. Real-data runs on that laptop, each in an 8 GB memory cap: an AI
agent redoing two published genome papers through irit-mcp alone ([first](research/paper-redo-kpn.md),
[second](research/paper-redo-bd.md)), [nanopore isolates](research/nanopore-laptop.md),
[metagenome to MAGs](research/mag-laptop.md), [how much sequencing depth is
enough](research/coverage-depth.md), [reads to GTDB species](research/e2e-laptop.md) and
[co-occurrence on real oral samples](research/cooccur-real.md). The [roadmap](roadmap.md) lists what comes next.
