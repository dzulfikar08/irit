# Roadmap

What's done, what's next, and what's further out. Items move up as evidence comes in.

## Done

- [x] `sheetgen`: schema-driven sample sheets, validated by an nf-core/bacass stub run.
- [x] `leanqc`: FastQC-equivalent QC including per-tile quality; verdicts match FastQC on test data.
- [x] `dblock`: database catalog, disk planning, resumable and streaming fetch, lock file, verify,
      tool-version check.
- [x] Baseline of nf-core/bacass on a laptop ([setup](lab/setup.md), [benchmarks](benchmarks.md)).
- [x] Upstream report: GTDB R232 mock database checksum ([details](research/upstream-reports.md)).

## Next

- [ ] **Release binaries for every platform**: Linux (x86_64 and ARM64, statically linked so old
      distributions work), macOS (Intel and Apple Silicon), Windows. A single binary only helps if
      people can download one for their machine.
- [ ] **GTDB-Tk on a desktop: measure before building.** Peak memory and time for GTDB-Tk with and
      without `--scratch_dir`, and with gPlace and `--max_memory`, on a rented large-memory machine.
      See [GTDB-Tk and memory](research/gtdbtk-memory.md).
- [ ] **`isotable`**: one row per isolate or genome (reads, species, assembly stats, completeness,
      contamination, AMR) with pass/fail thresholds, as TSV and MultiQC table. For metagenome-
      assembled genomes, flag lineages known to score low on completeness because their genomes are
      reduced (for example Patescibacteria / CPR), after checking current literature on how CheckM2
      handles them.
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
- [ ] **Laptop benchmark for AI agents**: can an agent with these tools produce a defensible microbial
      genomics result on an 8 GB laptop, compared with a hand-built workflow (accuracy,
      reproducibility, time, cost, human intervention)? A possible paper.
- [ ] **Instrument connection**: lab instruments run vendor software on dedicated (often older Windows)
      PCs. Open standards exist (SiLA 2; LADS, an OPC UA companion specification for lab devices).
      A small agent that watches sequencer run folders or speaks these protocols could start QC as
      soon as a run finishes. Check real instrument support first. Note that current Rust supports
      Windows 10+; Windows 7 needs a lower-tier target.
- [ ] **Microbial interaction demo**: an interactive two-species competition model (Lotka–Volterra
      with published parameters) for outreach. Original art; a toy, not a research claim.
- [ ] **Microbial observatory**: environmental sensor time series plus periodic sequencing metadata.
      Needs field partners; long-term.
- [ ] **Documentation site** from `docs/` (mdBook).
