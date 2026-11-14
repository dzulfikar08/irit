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
- [x] More `irit-mcp` wrappers (0.3.2): Shovill 1.4.2, mlst 2.35.0 and AMRFinderPlus 4.2.7, so
      the isolate path from reads to resistance genes has no step outside the record. Checked with
      the real programs on real *S. aureus* reads (ST22, *mecA*) in the real-tools workflow.
- [x] `dblock` additions (0.3.2): the AMRFinderPlus database as a pinned entry
      (`amrfinderplus-db-4.2-2026-10-29.1`: NCBI's 38 files with checksums we computed, indexed by
      `amrfinder_index` before the folder is sealed), also usable as Bakta's
      `amrfinderplus-db/latest` instead of running `amrfinder_update`.
- [x] End-to-end run on a laptop through `irit-mcp` ([write-up](research/e2e-laptop.md)):
      *B. subtilis* 168 from raw reads to the GTDB R232 species (leanqc, fastp, Shovill/SKESA,
      CheckM2, `gtdbprep ani` + GTDB-Tk `classify_wf`, isotable) under a 16 GB and an 8 GB memory
      cap; largest step 5.82 GiB, correct species (ANI 100.0). The replay reran every command with
      three unrecorded steps added; gaps filed as [#2](https://github.com/dzulfikar08/irit/issues/2)
      to [#11](https://github.com/dzulfikar08/irit/issues/11).
- [x] `seqplan` (0.4.0): isolates per sequencing run and cost per isolate at a target
      coverage, from Illumina's specifications and reported nanopore yields, each cited
      ([seqplan](tools/seqplan.md)). Next: measure how low the coverage can go for this kind of
      work (subsampling the *K. pneumoniae* reads from the paper reproduction), so a plan can use
      a measured coverage instead of a habit.
- [x] **`leanqc` long-read views** (0.3.2): N50, N90, length bins and quality by length.
- [x] **Replay gaps from the laptop run closed** (0.3.2): output folders ([#2](https://github.com/dzulfikar08/irit/issues/2)),
      naming genomes ([#3](https://github.com/dzulfikar08/irit/issues/3)), a recorded `gtdbprep ani`
      tool ([#4](https://github.com/dzulfikar08/irit/issues/4)), Shovill and leanqc outputs that change
      between runs ([#5](https://github.com/dzulfikar08/irit/issues/5), [#6](https://github.com/dzulfikar08/irit/issues/6)),
      so `replay.sh` reruns the isolate path from reads to species on its own.
- [x] **fieldsheet**: an offline phone worksheet for sample metadata ([guide](tools/fieldsheet.md),
      unreleased). Next: try it on a low-end Android phone in the field.

## Next

- [ ] **GTDB-Tk on a desktop: measure before building.** Peak memory and time for GTDB-Tk with and
      without `--scratch_dir`, and with gPlace and `--max_memory`, on a rented large-memory machine.
      See [GTDB-Tk and memory](research/gtdbtk-memory.md). GTDB plans to move GTDB-Tk to gPlace "in
      the coming weeks", placing directly on the full trees and dropping the backbone/class-level
      split, so no `--placer` option ([GTDBTk#728](https://github.com/Ecogenomics/GTDBTk/issues/728)).
      Wait for that release, then measure it; until then the `pplacer` shim stays. The placement
      mirror and `irit-mcp`'s `split/` check will need updating for the new layout.
- [ ] **GTDB placement data without the 60.8 GB download**: an unofficial, credited mirror
      (280 MB download) on GitHub Releases (`dzulfikar08/irit-data`) and the `dblock` entries
      `gtdbtk-r232-placement` and `-lite` (0.2.1); `dblock` stores identical files once as hard
      links (0.3.0), so the ten `fasttree.log` copies take 2.7 GB instead of 7.4 GB. Waiting on GTDB
      GTDB has no plans for per-folder downloads and is fine with the mirror as long as it stays
      credited and links back ([GTDBTk#727](https://github.com/Ecogenomics/GTDBTk/issues/727)).
- [ ] **The ANI species check on a laptop**: `gtdbprep ani` screens locally and range-reads only
      the reference sketches a genome needs, then runs GTDB-Tk's own ANI screen on them; identical
      output to GTDB-Tk on the full database ([write-up](research/ani-laptop.md)). The screen index
      is on irit-data (`dblock` entry `gtdbtk-r232-ani-screen`). Sketches come from an unpacked
      `sketches.db`, or, opt-in, from GTDB's archive through a seek index (2–6× the download), with
      a per-user cache; GTDB won't publish the skani folder unpacked and asked for exactly these
      two safeguards ([GTDBTk#727](https://github.com/Ecogenomics/GTDBTk/issues/727)). Built
      since (0.3.4): parallel range requests with retries (377 sketches from GTDB's server in
      474–562 s instead of 1,167–2,275 s), and the opt-in `--species-only`, which fetched 47 instead of 882
      sketches for six test genomes with the same calls (fewer unassigned-hits rows than GTDB-Tk's;
      [measurements](research/ani-laptop.md#7-parallel-requests-and-a-species-only-mode)). On five
      genomes from crowded genera (*Streptococcus*, *B. cereus*, *Burkholderia*, *Pseudomonas*,
      *Klebsiella*) `--species-only` fetched 542 instead of 2,694 references with the same calls and
      the same related references ([measurements](research/ani-laptop.md#species-only-in-crowded-genera)).
- [ ] **`msascope` after the redesign** (built in 0.3.0–0.3.2: themes, names, hover card, colour
      schemes, hidden insert and gappy columns, tree panel, selection, export, sort and filter,
      motif search, annotation tracks, reference row, remembered between sessions, and annotation
      tracks and the pinned row in figures; 0.5.0: gzip decoded on its own thread, BGZF in
      parallel): alignments larger than RAM.
- [ ] **Nanopore isolates on a laptop**: `flye` and `medaka` tools in `irit-mcp` (0.4.0). One
      carbapenemase-producing *K. pneumoniae* gave the same ST, the same 49 AMRFinderPlus elements,
      the same completeness and the same GTDB species from ONT reads alone as from Illumina, in an
      8 GB cap ([write-up](research/nanopore-laptop.md)). isotable reads long-read QC, the record
      hashes Medaka's model file, and Medaka `batch_size` 50 or 25 keeps it under 5 GiB with the
      same consensus (#21). Three more isolates with small resistance plasmids: same STs and every
      resistance element, once Flye's `--meta` caught a 5 kb plasmid default mode missed; deep reads
      subsampled with `rasusa`; irit-mcp warns about plasmids Flye closed as several copies.
      Next: a ligation-kit dataset. Read files can now be checked against ENA's MD5s (`sheetgen verify`, unreleased).
- [ ] **`dblock` additions**: GTDB-Tk split download (waits for GTDB-Tk's gPlace layout). Parallel
      downloads are built (0.5.0, `fetch --jobs`).
- [ ] **Check the `irit-mcp` wrappers on a large machine**: Bakta with the full database, SPAdes
      through Shovill (the real-tools workflow uses SKESA). GTDB-Tk `classify_wf` has now run with
      R232 data for a genome the ANI screen assigns, through the small data folder `gtdbprep ani`
      builds ([laptop run](research/e2e-laptop.md)); genomes that need tree placement still go
      through the launcher and gPlace outside irit-mcp ([#4](https://github.com/dzulfikar08/irit/issues/4)).

## Later

- [ ] **`subprep`**: validate NCBI/ENA submission metadata offline before upload.
- [ ] **Split GTDB-Tk, step 2 (parked)**: a placement service for `gtdbprep` bundles. The gPlace test
      showed a 16 GB laptop can place on its own, so this is only worth building for larger batches.
- [ ] **Laptop benchmark for AI agents**: can an agent with these tools produce a defensible microbial
      genomics result on an 8 GB laptop, compared with a hand-built workflow (accuracy,
      reproducibility, time, cost, human intervention)? A possible paper. First data point: one
      isolate through `irit-mcp` fits an 8 GB cap and gets the right species
      ([laptop run](research/e2e-laptop.md)), with the steps called by a scripted client rather than an
      agent. Second data point: an agent with only `irit-mcp` (no shell, no web) redid Table 1 of a
      December 2025 *K. pneumoniae* genome announcement on the laptop. AMR genes and MLST matched
      exactly, GTDB confirmed the species, and it found six irit bugs (#14–#19)
      ([write-up](research/paper-redo-kpn.md)). Third data point: a second paper, three *E. coli*
      and three *Salmonella* Kentucky from chickens in Bangladesh. All six STs matched, genome size
      within 0.13 %, and the agent caught twelve *blaTEM* alleles listed for one truncated gene
      ([write-up](research/paper-redo-bd.md)). Still to do: more papers and lineages, and the
      hand-built comparison.
- [ ] **Gaps the paper redo showed**: QUAST-style assembly stats (`assembly_stats`), coverage by
      read mapping (`coverm_genome`) and *Klebsiella* typing (`kleborate`) are built (0.5.0).
      Still missing (both redos): virulence genes for other species (VirulenceFinder/VFDB), plasmid
      replicons (PlasmidFinder), ResFinder, *Salmonella* serotyping (SISTR/SeqSero2), a Kraken2
      tool and clonal complexes. (Checking read files against ENA MD5s: `sheetgen verify`, unreleased.)
- [x] **How much depth is enough** ([write-up](research/coverage-depth.md)): on the paper's three
      *K. pneumoniae*, every Table 1 call held down to 15× (20× is a practical target, 2.5 times as
      many isolates per run as 50×); below that Shovill needs `gsize`, and irit-mcp warns when it didn't get one (#22).
- [x] **A MAG route for metagenomes on 8 GB** ([laptop test](research/mag-laptop.md)): MEGAHIT,
      CoverM and MetaBAT2 in irit-mcp, then CheckM2 (`lowmem`), the GTDB ANI screen and isotable's
      MAG mode. A CPR-rich groundwater sample gave 23 bins (4 `high*`, 13 medium), 8 of them
      Patescibacteria, every step within 8 GB.
- [ ] **Next for MAGs**: built (unreleased): tree placement of unassigned bins on placement-lite
      (`gtdbprep place`, with gPlace), SemiBin2 (`semibin2`: pretrained models fit 1.5 GiB, its
      self-supervised training doesn't fit 8 GB), dereplication (`galah`), one genome × sample
      abundance table (`coverm_abundance`) and co-occurrence statistics ([`cooccur`](tools/cooccur.md)).
      Still open: co-assembly within 8 GB for diverse, shallow samples; SemiBin2's multi-sample mode;
      a real multi-sample CPR series for `cooccur`.
- [ ] **Instrument connection**: lab instruments run vendor software on dedicated (often older Windows)
      PCs, and labs usually connect them through a LIMS, which parses instrument exports or uses vendor
      drivers. So the practical route is the LIMS or the files instruments drop for it. Open standards exist (SiLA 2; LADS, an OPC UA companion specification for lab devices).
      A small agent that watches sequencer run folders or speaks these protocols could start QC as
      soon as a run finishes. Check real instrument support first. Note that current Rust supports
      Windows 10+; Windows 7 needs a lower-tier target.
- [ ] **Microbial observatory**: environmental sensor time series plus periodic sequencing metadata.
      Needs field partners; long-term.
