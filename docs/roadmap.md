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
- [x] Upstream report: GTDB R232 mock database checksum ([GTDBTk#726](https://github.com/Ecogenomics/GTDBTk/issues/726)).

## Next

- [ ] **GTDB-Tk on a desktop: measure before building.** Peak memory and time for GTDB-Tk with and
      without `--scratch_dir`, and with gPlace and `--max_memory`, on a rented large-memory machine.
      See [GTDB-Tk and memory](research/gtdbtk-memory.md).
- [ ] **MSA viewer** (Rust + GPUI): open 10^5–10^6 aligned sequences smoothly on a laptop; linked
      tree panel after. See [landscape](research/landscape.md#2-viewers).
- [ ] **`dblock` additions**: deep verify (hash every file), AMRFinderPlus via `amrfinder_update`,
      GTDB-Tk split download, parallel downloads.
- [ ] **`leanqc` additions**: long-read views (N50, quality by length), multi-threaded decompression
      for single large files.

## Later

- [ ] **`subprep`**: validate NCBI/ENA submission metadata offline before upload.
- [ ] **Typed MCP tool layer** over the standard microbial genomics tools, recording every call as a
      replayable script, so an AI agent can run analyses that can be repeated without it.
- [ ] **Split GTDB-Tk**: run the light steps (marker genes, alignment, ANI sketch) on the laptop and
      send only those to a shared placement server that keeps the tree in memory. See
      [GTDB-Tk and memory](research/gtdbtk-memory.md#using-hosted-data-instead-of-downloading-it).
- [ ] **Laptop benchmark for AI agents**: can an agent with these tools produce a defensible microbial
      genomics result on an 8 GB laptop, compared with a hand-built workflow (accuracy,
      reproducibility, time, cost, human intervention)? A possible paper.
- [ ] **Instrument connection**: lab instruments run vendor software on dedicated (often older Windows)
      PCs, and labs usually connect them through a LIMS, which parses instrument exports or uses vendor
      drivers. So the practical route is the LIMS or the files instruments drop for it. Open standards exist (SiLA 2; LADS, an OPC UA companion specification for lab devices).
      A small agent that watches sequencer run folders or speaks these protocols could start QC as
      soon as a run finishes. Check real instrument support first. Note that current Rust supports
      Windows 10+; Windows 7 needs a lower-tier target.
- [ ] **Microbial interaction demo**: an interactive two-species competition model (Lotka–Volterra
      with published parameters) for outreach. Original art; a toy, not a research claim.
- [ ] **Microbial observatory**: environmental sensor time series plus periodic sequencing metadata.
      Needs field partners; long-term.
- [ ] **Documentation site** from `docs/` (mdBook).
