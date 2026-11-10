# Roadmap

What's done, what's next, and what's further out. Items move up as evidence comes in.

## Done

- [x] `sheetgen`: schema-driven sample sheets, validated by an nf-core/bacass stub run.
- [x] `leanqc`: FastQC-equivalent QC including per-tile quality; verdicts match FastQC 0.12.1 on the four bacass test files (40 of 40) and on a synthetic corpus (88 of 90, two deliberate encoding differences).
- [x] `dblock`: database catalog, disk planning, resumable and streaming fetch, lock file, verify,
      tool-version check.
- [x] Baseline of nf-core/bacass on a laptop ([setup](lab/setup.md), [benchmarks](benchmarks.md)).
- [x] `isotable`: one graded row per sample from nine tools' outputs, isolate and MAG (MIMAG) modes,
      with a note for reduced-genome lineages.
- [x] v0.1.0 release with static binaries for Linux (x86_64, ARM64), macOS (Intel, Apple Silicon)
      and Windows, built by GitHub Actions on each tag.
- [x] Upstream reports: GTDB R232 mock database checksum ([GTDBTk#726](https://github.com/Ecogenomics/GTDBTk/issues/726));
      FastQC accepting a FASTQ cut off mid-line ([FastQC#221](https://github.com/s-andrews/FastQC/issues/221), fix in
      [FastQC#222](https://github.com/s-andrews/FastQC/pull/222)).
- [x] Independent reviews of every tool, with fixes and regression tests (sample-name joins, pairing
      rules, FastQC fidelity, database integrity, MCP security).
- [x] `msaview` first version: 100,000 × 30,000 alignments on a laptop (FASTA/gzip, Clustal,
      Stockholm; overview pyramid, consensus and identity tracks, search, go-to; `--type` override).
- [x] `irit-mcp`: typed MCP tools for the irit CLIs, with run records and a `replay.sh` that reruns
      a session without the AI; outputs confined to a root folder.
- [x] Split GTDB-Tk, step 1: the [design](research/split-gtdb.md) and the laptop client
      [`gtdbprep`](tools/gtdbprep.md); its bundle classifies identically to `classify_wf` on test genomes.
- [x] Documentation site (Astro + Starlight) at https://dzulfikar08.github.io/irit/, and an
      interactive competition demo.

## Next

- [ ] **GTDB-Tk on a desktop: measure before building.** Peak memory and time for GTDB-Tk with and
      without `--scratch_dir`, and with gPlace and `--max_memory`, on a rented large-memory machine.
      See [GTDB-Tk and memory](research/gtdbtk-memory.md).
- [ ] **gPlace under an 8.5 GB cap** on GitHub runners (in progress): can GTDB placement run on a
      16 GB machine? Decides whether a placement server is needed at all.
- [ ] **`msaview` additions**: linked tree panel, multi-threaded gzip, alignments larger than RAM,
      macOS/Windows testing, desktop packaging.
- [ ] **`leanqc` vs FastQC-Rust benchmark** on a GitHub runner (time and memory, same files), and
      `isotable` support for FastQC/FastQC-Rust output.
- [ ] **`dblock` for `gtdbprep`**: keep only selected folders of an archive, and verify folder hashes
      the way GTDB-Tk does, so the 77 MB client data comes with a lock entry.
- [ ] **`dblock` additions**: AMRFinderPlus via `amrfinder_update`, GTDB-Tk split download, parallel
      downloads.
- [ ] **`leanqc` additions**: long-read views (N50, quality by length).

## Later

- [ ] **`subprep`**: validate NCBI/ENA submission metadata offline before upload.
- [ ] **MCP wrappers for external tools** (fastp, Bakta, CheckM2, GTDB-Tk) in `irit-mcp`, so a whole
      microbial genomics run is recorded and replayable.
- [ ] **Split GTDB-Tk, step 2**: a placement service for `gtdbprep` bundles, only if the gPlace test
      shows laptops can't place on their own. See [the design](research/split-gtdb.md).
- [ ] **Laptop benchmark for AI agents**: can an agent with these tools produce a defensible microbial
      genomics result on an 8 GB laptop, compared with a hand-built workflow (accuracy,
      reproducibility, time, cost, human intervention)? A possible paper.
- [ ] **Instrument connection**: lab instruments run vendor software on dedicated (often older Windows)
      PCs, and labs usually connect them through a LIMS, which parses instrument exports or uses vendor
      drivers. So the practical route is the LIMS or the files instruments drop for it. Open standards exist (SiLA 2; LADS, an OPC UA companion specification for lab devices).
      A small agent that watches sequencer run folders or speaks these protocols could start QC as
      soon as a run finishes. Check real instrument support first. Note that current Rust supports
      Windows 10+; Windows 7 needs a lower-tier target.
- [ ] **Microbial observatory**: environmental sensor time series plus periodic sequencing metadata.
      Needs field partners; long-term.
