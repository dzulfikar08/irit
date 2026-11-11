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
- [x] `msascope` first version: 100,000 × 30,000 alignments on a laptop (FASTA/gzip, Clustal,
      Stockholm; overview pyramid, consensus and identity tracks, search, go-to; `--type` override).
- [x] `irit-mcp`: typed MCP tools for the irit CLIs, with run records and a `replay.sh` that reruns
      a session without the AI; outputs confined to a root folder.
- [x] Split GTDB-Tk, step 1: the [design](research/split-gtdb.md) and the laptop client
      [`gtdbprep`](tools/gtdbprep.md); its bundle classifies identically to `classify_wf` on test genomes.
- [x] Documentation site (Astro + Starlight) at https://dzulfikar08.github.io/irit/, and an
      interactive competition demo.
- [x] GTDB placement under an enforced 8.5 GiB memory cap on a 16 GB GitHub runner, with gPlace
      ([write-up](research/gtdb-placement-test.md)): a shared placement server isn't needed.
- [x] gPlace gives the same placements as pplacer (59/59 genomes) and runs down to `--max_memory`
      1.9G (class-level tree) / 5.2G (full bacterial tree) ([write-up](research/gplace-checks.md)).
- [x] irit 0.2.0: signed, notarised macOS builds and a curl installer.
- [x] `leanqc` vs FastQC-Rust benchmark on a GitHub runner; `isotable` reads FastQC/FastQC-Rust output.
- [x] `dblock` keeps selected folders of an archive and verifies GTDB-Tk folder hashes
      (`gtdbtk-r232-client` for `gtdbprep`).
- [x] `irit-mcp` wraps fastp 1.4.0, Bakta 1.12.2, CheckM2 1.1.0 and GTDB-Tk 2.8.0 `classify_wf`, with
      only the flags of those versions, input and database checks, and the same run records and
      `replay.sh` (0.3.0). Tested with stand-ins for the programs.

## Next

- [ ] **GTDB-Tk on a desktop: measure before building.** Peak memory and time for GTDB-Tk with and
      without `--scratch_dir`, and with gPlace and `--max_memory`, on a rented large-memory machine.
      See [GTDB-Tk and memory](research/gtdbtk-memory.md).
- [ ] **GTDB placement data without the 60.8 GB download**: an unofficial, credited mirror
      (280 MB download) on GitHub Releases (`dzulfikar08/irit-data`) and the `dblock` entries
      `gtdbtk-r232-placement` and `-lite` (0.2.1); `dblock` stores identical files once as hard
      links (0.3.0), so the ten `fasttree.log` copies take 2.7 GB instead of 7.4 GB. Waiting on GTDB
      for official per-folder downloads ([GTDBTk#727](https://github.com/Ecogenomics/GTDBTk/issues/727)).
- [ ] **The ANI species check on a laptop**: `gtdbprep ani` screens locally and range-reads only
      the reference sketches a genome needs, then runs GTDB-Tk's own ANI screen on them; identical
      output to GTDB-Tk on the full database ([write-up](research/ani-laptop.md)). The screen index
      is on irit-data (`dblock` entry `gtdbtk-r232-ani-screen`). Sketches come from GTDB's archive
      through a seek index for now (a stopgap: 2–6× the download); waiting on GTDB for the skani
      folder unpacked ([GTDBTk#727](https://github.com/Ecogenomics/GTDBTk/issues/727)). Next:
      parallel range requests; an opt-in "species call only" flag that fetches only references
      that can reach their radius (far fewer downloads, but fewer "other related references" rows
      than GTDB-Tk's).
- [ ] **`msascope` redesign** ([mockups](tools/msascope.md#planned-redesign)): themes, sequence names
      and a readable default zoom are built; next the hover card, colour schemes, hiding insert and gappy columns, a tree
      panel, selection and export. Later: multi-threaded gzip, alignments larger than RAM.
- [ ] **`dblock` additions**: AMRFinderPlus via `amrfinder_update`, GTDB-Tk split download, parallel
      downloads.
- [ ] **`leanqc` additions**: long-read views (N50, quality by length).
- [ ] **More `irit-mcp` wrappers**: an assembler (e.g. SPAdes or Shovill), mlst and AMRFinderPlus, so
      the isolate path has no step outside the record. Check the four wrappers against the real
      tools and databases on a large machine.

## Later

- [ ] **`subprep`**: validate NCBI/ENA submission metadata offline before upload.
- [ ] **Split GTDB-Tk, step 2 (parked)**: a placement service for `gtdbprep` bundles. The gPlace test
      showed a 16 GB laptop can place on its own, so this is only worth building for larger batches.
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
