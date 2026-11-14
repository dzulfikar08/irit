# Changelog

## Unreleased

### irit-mcp
- **`assembly_stats`**: QUAST-style assembly statistics (contigs, total length, largest contig, N50,
  L50, N90, L90, GC, Ns per 100 kbp, contigs >= 1 kb; contigs of 500 bp or more, QUAST's default),
  computed by isotable and recorded like any call.
- **`coverm_genome`** (CoverM 0.8.0): an isolate's depth by read mapping, and the fraction of the
  assembly covered. Warns when Shovill's depth estimate differs by more than 1.5×.
- **`kleborate`** (Kleborate 3.2.4): *Klebsiella* typing (ST, virulence and resistance scores,
  ybt/clb/iuc/iro/rmpADC, K/O loci with Kaptive's confidence). Its bundled databases are hashed into
  the record; warns about assemblies with no row or outside the KpSC.
- A tool whose bundled data (mlst's schemes, Kleborate's databases) can't be found now gets a
  warning that the record identifies it by version only.

### isotable
- **`isotable assembly-stats`**, and largest contig, L50, N90, Ns per 100 kbp, contigs >= 1 kb and
  all contigs as table columns (from assemblies and QUAST reports).
- Reads CoverM genome and Kleborate 3 tables. Shows Shovill's depth estimate next to the mapped
  depth; `--min-depth` (default 20) grades the mapped depth (warn below it, fail below half), and a
  warning says when the two differ by more than 1.5×. New requirements `depth` and `kleborate`.

### dblock
- **Downloads side by side**: `fetch --jobs N` (default 4) downloads the parts or files of an entry,
  and several databases, under one shared limit; `--jobs 1` fetches one thing at a time as before.
  Measured: placement-lite (11 parts, 175 MB) 133 s → 68 s, AMRFinderPlus (38 files, 126 MB)
  414 s → 161 s. When a part fails, the parts still downloading stop and keep their partial files
  for the next run; the error names the part that failed. Progress stays on one live line.

## 0.4.0 (2026-11-14)

New: `seqplan` (plan a sequencing run), long-read isolates (`flye`, `medaka`) and metagenomes to
MAGs (`megahit`, `coverm_contig`, `metabat2`) in irit-mcp, and msascope's menu bar and Mac App Store
preparation. Fixes from an AI agent reproducing Neumann et al. 2025 (*K. pneumoniae* from alfalfa
sprouts, MRA e01270-25) through irit-mcp alone (#14-#19), and from the depth experiment and the
nanopore run (#21-#25).

### irit-mcp
- **A used run folder is refused** (#14): irit-mcp won't start when `<runs-dir>/<run-id>/` already
  holds a record. Reusing `--run-id` restarted call numbering and overwrote `run.json`, `replay.sh`,
  the RO-Crate and `calls/NNN.*`, losing the earlier session's record.
- **Databases fall back to `dblock.lock`** (#16): with no path given and the variable unset,
  `gtdbtk_classify_wf`, `gtdbprep_ani` (data and `index`, now optional), `checkm2`, `bakta` and
  `amrfinder` use the one database of that kind locked in the working folder, and `warnings` names
  it; two candidates are refused.
- **Warnings from stderr** (#18): CheckM2's scikit-learn `InconsistentVersionWarning` and
  AMRFinderPlus's `WARNING:` lines go into `warnings`. AMRFinderPlus from an unactivated conda
  environment gets `CONDA_PREFIX` set to it.
- **`describe_run` summarises by default** (#19): tool, status, version, error, warnings and outputs
  per call; `detail: true` for the full records (170 kB for 26 calls), `call_id` for one.
- `amrfinder` names Bakta's trimmed AMRFinderPlus copy instead of calling it an incomplete download;
  `replay.sh` says where a `gtdbprep ani` call's sketches come from on another machine (#19).

### isotable
- **GTDB from the ANI screen** (#15): `classify/ani_screen/*.ani_summary.tsv` (`reference_taxonomy`)
  counts as the GTDB classification, so a run where the screen assigned every genome no longer
  comes out `incomplete` under `--require gtdb`.
- **GC for Shovill assemblies** (#15), over the same contigs of 500 bp or more as the contig count,
  length and N50 (QUAST's rule); the requirement help names the cutoff. On the paper's isolates:
  57.04 / 57.15 / 57.35 against the paper's 57.06 / 57.15 / 57.35.

### dblock
- **No download without the prepare program** (#17): `fetch` refuses a database whose `prepare`
  program (`amrfinder_index`) isn't on `PATH` before downloading it, instead of after; `plan` says
  it won't fetch, and notes databases whose tool isn't installed.

### gtdbprep
- The missing-sketch error says how much would be fetched and suggests `--species-only` (#19).
### Fixes from the depth experiment and the nanopore run (#21-#25)
- **irit-mcp warns when Shovill's genome-size estimate collapses** (#22). At 5-10× Shovill
  estimated 0.01-0.11 Mb for a 5.5 Mb genome, subsampled to it and assembled a fragment with
  exit 0. The server reads `shovill.log` and warns in three cases:
  - Shovill subsampled to a tiny size.
  - The assembly is over twice the estimate. The warning gives the real depth over the assembly,
    since Shovill's own figure is then too high (60× reported at 20×).
  - `gsize` was given and the assembly is under 70% of it.

  A separate warning comes when the depth over the assembly is under 15×.
- **`gtdbprep ani` screens the rest when skani can't sketch a genome** (#23). Such a genome, e.g. a
  407 bp assembly, is listed as not screened (`-` markers; `screened: false` in irit-mcp) with a
  warning. Before, it failed the whole batch.
- **mlst: a warning when it chose the scheme itself and found no ST** (#24). A fragmented
  *K. pneumoniae* got `ecoli_achtman_4`.
- **CheckM2: a warning above 20 genomes per call** (#25). 48 peaked at 7.7 GiB on an 8 GB laptop.
- **Medaka's model file is recorded** (#21). Medaka downloads a named model at run time, so the
  server hashes the file Medaka logged into the call's inputs, returns it as `model_file`, and
  `replay.sh` notes it.
  `batch_size` 50 peaked at 4.78 GiB and 25 at 3.66 GiB on the 6 Mb nanopore genome, against 7.36 GiB
  at Medaka's default 100. All three gave the same consensus and the smaller sizes were no slower,
  so the docs and the tool description recommend 50 or 25 on 8 GB machines.
- **isotable reads long-read QC** (#21). For a nanopore-only isolate, leanqc's long-read numbers
  stand in for fastp:
  - `long_reads` and `long_read_bases` count as reads, and `read_q_median` counts as quality
    (warn below `--min-read-q`, default Q10). `read_n50` is shown.
  - leanqc's short-read checks (per-position, per-tile, per-read quality and GC, read length) are
    not graded for long-read files, and a note lists them.
  - The nanopore run's ONT sample now passes instead of being `incomplete`.

### seqplan (new)
- **Plan a sequencing run before sequencing**: bases and read pairs per isolate at a target
  coverage, isolates per run, runs for N samples, and cost per isolate from a run price you give
  (no built-in prices). It pads each isolate by a barcode-imbalance margin (default 1.3) and counts
  only the usable part of a run (default 0.8 for Illumina). `--barcodes` caps isolates per run at
  the library kit's barcode count. Kits: MiSeq v2/v3/Micro/Nano, iSeq 100,
  NextSeq 500/550 mid/high, NextSeq 1000/2000 P1-P4, NovaSeq X 1.5B/10B/25B (Illumina
  specifications), plus MinION and Flongle R10.4.1 (reported typical yields, marked as such). There
  are 16 organism presets with NCBI's expected genome size. Every figure carries its source URL
  (`seqplan/data/`, `--list-kits`, `--list-organisms`). Example: *K. pneumoniae* 5.5 Mb at 50× fits
  29-33 isolates on a MiSeq v3 run and 67 on a NextSeq 2000 P1; at 25×, 59-67 and 134.
- In the release archives (all platforms, including Windows) and the installer.

### irit-mcp
- **`seqplan` tool**: the same plan as JSON, so an agent answers from cited kit figures; it is told
  never to invent a run price.

### irit-mcp: metagenomes to MAGs
- **`megahit`, `coverm_contig` and `metabat2`**: MEGAHIT 1.2.9 assembly, CoverM 0.8.0 per-contig
  depth (`methods: ["metabat"]` writes MetaBAT2's depth table) and MetaBAT 2.18 binning, pinned and
  replayable like the isolate tools. Then `checkm2` on the bins, `gtdbprep_ani` and isotable's MAG
  mode. Measured on the laptop in [Metagenome to MAGs in 8 GB](docs/research/mag-laptop.md): a
  Crystal Geyser groundwater sample gave 23 bins (4 `high*`, 13 medium, 8 of them
  Patescibacteria), every step within an 8 GB cap.
- MEGAHIT's memory defaults to 4 GB (MEGAHIT's own default, 90% of the machine, ignores memory
  caps); MetaBAT2's seed defaults to 1, so bins repeat. MetaBAT2's version comes from `--help`,
  in the record and in `replay.sh`.
- A failed tool gets a `hint` when it ran out of memory: CheckM2's SIGKILLed DIAMOND step (use
  `lowmem`), or any tool killed by a signal.
- The real-tools workflow runs the three on a two-genome mock metagenome, twice, and replays them.

### msascope
- **Menu bar** (macOS) with About, Acknowledgements, Hide, Quit, Open, Open Sample Alignment,
  Export, Close File, Close Window, Copy Selection and Select All. Quit, Open, Close Window, Export,
  Copy and Select All are key bindings (⌘ on macOS, Ctrl elsewhere), so the menus show them.
- **Closing the window**: on macOS msascope keeps running and a click on its Dock icon opens a new
  window; elsewhere closing the last window quits. Before, it kept running without a window.
- **Finder opens files with msascope** (Open With, a drop on the Dock icon): alignments and trees,
  registered as an alternative app, not the default one.
- **Acknowledgements**: the licences of every dependency compiled into msascope, plus the Lucide
  icons and Catppuccin palettes, generated with cargo-about and shipped as `Acknowledgements.html`
  in msascope.app and the Linux archives. Neither shipped them before.
- **Mac App Store edition, prepared**: it runs in the App Sandbox (security-scoped bookmarks for
  recent files, **File ▸ Allow Access to Folder…** for a tree next to the alignment, a clear message
  for a file it may not read); `scripts/build-mas-pkg.sh` and the `msascope-mas.yml` workflow build,
  validate and upload it. CI checks that msascope's dependencies stay permissively licensed
  (`msascope/deny.toml`) and that a sandboxed copy starts and can't read a file nobody chose.
### irit-mcp
- **`flye` and `medaka` tools: nanopore (and PacBio) isolate assembly.** Flye 2.9.6 (`read_type`
  `nano-hq` for R10.4.1 hac/sup reads, the default) and Medaka 2.2.2 (`medaka_consensus`), pinned
  and checked like Shovill: flags read from the release sources, inputs and `outdir` validated,
  `link_as` for CheckM2, GTDB-Tk and `gtdbprep_ani`, the assembly's contig count, length and N50
  recorded for replay. `flye` also returns each contig's depth and circularity. On one
  *K. pneumoniae* isolate, ONT reads alone gave the same ST, the same 49 AMRFinderPlus elements,
  the same CheckM2 completeness and the same GTDB species as Illumina, in an 8 GB cap: Flye 22 min
  (3.85 GiB), Medaka 9.4 min (7.36 GiB) ([write-up](docs/research/nanopore-laptop.md)).

### isotable
- Reads Flye's `assembly.fasta` (with `flye.log` beside it) and Medaka's `consensus.fasta` (with
  `consensus_probs.hdf` beside it) for contigs, length, N50 and GC; when a sample has both, the
  polished one counts and the draft is listed as skipped.

## 0.3.5 (2026-11-12)

### irit-mcp
- **`gtdbprep_ani` tool** (#4): `gtdbprep ani` through the server, recorded and replayable.
  `source` stays opt-in; the sketch cache, `$GTDBTK_DATA_PATH` and `$GTDBPREP_ENV` are resolved
  when the server starts and written into the command. It returns each genome's species,
  reference, ANI and AF, and `<work>/gtdbtk_data`, which `gtdbtk_classify_wf` takes as `data_path`
  (its skani-missing message now says so). On *B. subtilis* 168 with a warm cache: 15 s, then
  classify_wf 4.8 s, same species; the replay matched all 32 outputs. Since that folder is now an
  output of a recorded call, the run record shows where classify_wf's `GTDBTK_DATA_PATH` came from
  (#8).
- **GTDB-Tk's data is accounted for or flagged** (#8): a `skani/` database of up to 512 MB (the one
  `gtdbprep_ani` builds is a few MB) is hashed into `gtdbtk_classify_wf`'s inputs under its real
  path, and the call warns when `data_path` is neither in a dblock store nor written by an earlier
  call of the run, since the record can't then say what the folder held.
- **Missing output folders are created** (#2): `trim/s1_R1.fastq.gz` or an `outdir` of `asm/s1`
  no longer needs `trim/` or `asm/` to exist. Only folders of checked outputs are created (so inside
  the root), a file in the way is refused, the record lists them as `dirs`, and `replay.sh` runs
  `mkdir -p` on them, so a replay works from a clean folder.
- **Shovill `link_as`** (#3): e.g. `genomes/s1.fna` becomes a relative link to
  `<outdir>/contigs.fa`, made before Shovill runs, recorded as the call's `links` and recreated by
  `replay.sh` (`ln -sfn`), so CheckM2, GTDB-Tk and `gtdbprep_ani` name the genome after the
  sample and replay works without hand-made copies.
- **Every Shovill output is volatile** (#5): `contigs.fa` headers carry the date, and SKESA 2.5.1
  with 8 CPUs gave different assemblies from identical reads, as SPAdes did. `replay.sh` checks
  that they exist; the contig count, length and N50 are in the result and in isotable's row.
- **Replay explains outputs made from a volatile one** (#5). The contig count, length and N50
  are now recorded with the Shovill call (`assembly`), and `replay.sh` prints the replayed
  assembly's beside them. Before each call it checks the inputs an earlier call wrote; when one
  differs (a new assembly), that call's different outputs (CheckM2's report, isotable's row) are
  reported as following it instead of failing the replay. Anything else that differs still fails.

### isotable
- **Reads Shovill's `contigs.fa`** (#7) when `shovill.log` is beside it, as the sample named after
  its folder: contigs, length and N50, counted as QUAST does (contigs of 500 bp or more), so a
  QUAST report on the same assembly agrees. The default isolate requirements (contigs, N50) can
  now be met without QUAST, also through irit-mcp.

### leanqc
- **Reports are byte-identical between runs** (#6): `remaining_if_deduplicated_percent` summed
  over duplication levels in a hash map's order, so its last digits could change from run to run
  and `replay.sh`'s hash check of the JSON report failed. The levels are now summed in order.
  A test runs leanqc with 1 to 4 threads, eight times, on reads that push the duplication
  estimate past its tracking limit, and compares the JSON byte for byte (it fails on the old code).

### gtdbprep
- `irit-gtdb-prep --version` (and `-V`) works (#9); it said "unexpected argument". The release
  workflow now checks its version like the other binaries.

### dblock
- `checkm2-db-v3` is 3.08 GB on disk (measured), not ~3.5 GB (#11).
- A database folder moved or linked in from another store is reused with its `.dblock.toml`
  marker rewritten to its new place; the marker kept the old store's paths (#10). `fetch` now says
  what reuse checked (names and sizes) and that `verify --deep` re-hashes. Markers are written to
  a temporary file and renamed, so a marker hard-linked from the other store (a folder copied with
  `cp -al`, as on the laptop run) is replaced in this store only, not edited in both.

## 0.3.4 (2026-11-12)

### msascope
- **The reference row is remembered per file**: the reference and its options (pinned, ruler by
  reference, compare with it) are saved in the settings file for the last 50 files and restored
  when the same file (same size and modification time) is opened again and a row of that name is
  there. Stale or damaged entries are ignored.
- **Figures include the annotation tracks and the pinned reference row** shown in the window, in
  PNG and SVG (SVG keeps track characters and labels as text). The figure's ruler counts the
  reference's residues when the view's does.

### leanqc
- **Long-read views** (ONT, PacBio), for files whose median read is over 1,000 bp or with the new
  `--long`: total bases, N50/N90, mean and median length, longest read, reads and bases per
  log-scale length bin; each read's quality from its mean error probability, its distribution,
  mean and median, reads (and bases) at or above Q10 and Q20, and mean read quality by length. In
  the HTML report, the JSON (`long_reads`, documented) and the terminal summary. The FastQC checks
  and the JSON of short-read files are unchanged. N50 and the other figures match an independent
  calculation on a public MinION run and a 1.5 GB synthetic file; peak memory stayed at 14–23 MB.

### gtdbprep
- **`ani` fetches sketches in parallel**: `--fetch-jobs N` range requests in flight (default 4,
  at most 16; `1` is the old one-at-a-time behaviour). Failed requests (connection errors, cut-off
  bodies, 408, 429, 5xx) are retried up to 5 times with growing, jittered waits and `Retry-After`;
  a request with no answer for 60 s, or a body slower than 32 KiB/s, is cut off and retried
  instead of waiting up to 10 minutes. Each sketch goes to the cache as soon as it is checked.
  From GTDB's server, 377 sketches (1.23 GB) took 474–562 s instead of 1,167–2,275 s; for a few
  dozen sketches the connection's variation was as large as the gain.
- **`ani --species-only`** (opt-in) fetches only the references whose shared marker count shows
  they could reach ANI 95 (the smallest species radius) at AF 0.5, with a wide margin. GTDB-Tk's
  call is the same: identical on six test genomes, with 47 instead of 882 sketches fetched
  (147 MB instead of 2.8 GB; *M. tuberculosis* 1 instead of 377). The unassigned-hits file of an
  unassigned genome, and possibly the "other related references" column, list fewer references
  than GTDB-Tk's.
- `ani` writes `<work>/screen.tsv`: every screened-in reference per genome with its shared marker
  count and whether it was used. With `--species-only`, the summary table gains a `used` column.

## 0.3.3 (2026-11-12)

### irit-mcp
- **mlst's PubMLST data is in the run record.** After locating `mlst`, the server asks it where its
  data is (`mlst --help` prints the resolved `--datadir` and `--blastdb`, so `$MLST_DBDIR` and
  wrapper scripts are followed; if it prints none, `db/` next to the real program is used) and
  hashes `VERSION.txt`, the scheme map, the allele BLAST database and every scheme's profile table
  (177 inputs per call for Bioconda's mlst 2.35.0). `$MLST_DBDIR` is recorded and exported by
  `replay.sh`, and the mlst result gives `database_version` (the data's date, 2025-12-29). Two
  builds with different scheme snapshots no longer look the same in the record.
- **Shovill with SPAdes: `contigs.fa` is volatile.** SPAdes 3.15.5 (Shovill's default assembler)
  gave different `contigs.fa` bytes in two identical runs (same length, same ST), so `replay.sh`
  only checks it exists; the hash is still recorded. SKESA's contigs stay hash-checked.
- Checked against the real tools on a GitHub runner
  ([run 43203640701](https://github.com/dzulfikar08/irit/actions/runs/43203640701)): Shovill with
  SPAdes and its default read correction, stitching and Pilon (ST22 again), and
  `dblock_verify` with `deep` over the four databases, including the 162 files of the indexed
  AMRFinderPlus folder.

## 0.3.2 (2026-11-12)

### msascope
- **Annotation tracks**: Stockholm `#=GC` lines of every tag are shown under the track header,
  aligned to the columns and following hidden columns: `SS_cons` as helix and strand bands, RNA
  base pairs and coil, `PP_cons` as bars, other tags (`seq_cons`, `RF`, …) as text. The Tracks
  menu switches each one; the hover card over a track gives its character in words. `#=GR`
  lines are read too: the hover card over a cell lists the row's values, and the reference row's
  lines can be shown as tracks. Lines of the wrong length (or for a sequence that isn't in the
  file) are left out with a warning in the status bar and the terminal; at most 64 MiB of `#=GR`
  lines are kept.
- **Reference row**: set from the Tracks or Sort menu, by right-clicking a name, or with `m` (the
  cursor's row); marked "ref" in the name panel. It can be pinned under the tracks, the ruler can
  count its residues (from its `/start-end`), and Differences, the identity track, the density
  view, the overview and figures compare with it instead of the consensus. It stays with its
  sequence through sorting, filtering and tree order.
- **Cheaper track header**: the consensus, identity and gaps tracks (and the new ones) are drawn
  as cached image tiles instead of quads every frame; on a 20,000 × 10,000 alignment the paint
  median at the fit zoom went from 2.7–4.6 ms to 0.2 ms, at 1 px per cell from 3.9 ms to
  0.3–0.4 ms.
- `--stats` prints the annotation read and any lines left out.
- The `--bench` peak RSS difference reported for 0.3.1 was measured again: 0.3.0 and 0.3.1 peak
  within 2 MiB of each other, at the end of loading.

### dblock
- `gtdbtk-r232-mockup` is fetched again: GTDB corrected the stale MD5 in its `MD5SUM.txt`
  ([GTDBTk#726](https://github.com/Ecogenomics/GTDBTk/issues/726)), and the entry now uses it.
  `gtdbtk-r232-client` keeps the same file and now cites the published MD5.
- **`amrfinderplus-db-4.2-2026-10-29.1`**: AMRFinderPlus's database, release 2026-10-29.1 (format
  4.2), from NCBI's FTP: the 38 files `amrfinder_update` 4.2.7 downloads, with MD5 and SHA-256 we
  computed (NCBI publishes none). 126 MB download, 256 MB once indexed.
- Multi-part entries can be plain files (`"format": "file"`): each part is kept under its name.
- `prepare`: a fixed step run on the downloaded files before the folder is sealed. The only one is
  `amrfinder_index` (needs AMRFinderPlus on `PATH`); the lock records the program and its version
  (`prepared`).
- The Bakta entries' notes now say how to give Bakta this database as `amrfinderplus-db/latest`
  through a folder of links, instead of running `amrfinder_update` into the dblock folder.

### gtdbprep
- **`gtdbprep ani` no longer reads GTDB's server unless asked.** `--source` has no default:
  pass an unpacked `sketches.db` (path or URL), or `--source gtdb` to range-read GTDB's archive,
  as GTDB's maintainers asked ([GTDBTk#727](https://github.com/Ecogenomics/GTDBTk/issues/727)).
  A run whose sketches are all cached needs no `--source`.
- Fetched sketches go to one cache per user (`$XDG_CACHE_HOME/irit/gtdbprep-ani`, or
  `~/.cache/irit/gtdbprep-ani`) instead of `<work>/cache`, so a sketch is fetched once per machine,
  not once per work folder. `--cache` still overrides it. Runs sharing a cache no longer write
  through the same temporary file.

### irit-mcp
- **`shovill`** (Shovill 1.4.2, the current Bioconda release): paired-end Illumina assembly with
  SPAdes, SKESA, MEGAHIT or Velvet. Only Shovill 1.4.2's flags (`--opts`, which goes into a shell
  command, and `--tmpdir` are left out); reads checked as FASTQ, `ram` at least 8 GB as Shovill
  requires, `gsize`, `namefmt` and `kmers` checked the way Shovill parses them, an `outdir` whose
  path a shell would split refused. Returns contig count, total length, N50, longest contig and GC.
- **`mlst`** (mlst 2.35.0): sequence type from the PubMLST schemes bundled with mlst. `outfile` must
  be named so isotable finds it (`<sample>.mlst.tsv`), and with one assembly its sample part becomes
  mlst's `--label`, so Shovill's `contigs.fa` joins to the right sample.
- **`amrfinder`** (AMRFinderPlus 4.2.7): resistance genes and point mutations from an assembly,
  proteins, or both with a GFF (Bakta's with `annotation_format: bakta`). The database is checked
  like amrfinder does (files, indexes, format not newer than 4.2.7, data at least 2025-09-22.2,
  `organism` one the database knows) and its `version.txt` is hashed into the record; `--update` is
  not offered. `output` must be named `<sample>.amrfinder.tsv` (or contain `amrfinder`) for isotable.
- `AMRFINDER_DB` is the default database, as for `amrfinder` itself; `--tool-path` takes the three
  new names.
- The real-tools workflow also runs fastp → Shovill → mlst → AMRFinderPlus on real
  *Staphylococcus aureus* reads (ERR044595, nf-core/bacass's test data) and AMRFinderPlus on
  Bakta's proteins, then isotable over all of it, twice, then `replay.sh`.

### isotable
- `parse::mlst` and `parse::amrfinder` are public, for irit-mcp's summaries. No change in what
  isotable reads.

## 0.3.1 (2026-11-11)

### msascope
- **Loading progress and Cancel**: the loading window shows a progress bar of the bytes read
  (compressed bytes for gzip), the sequences read so far, both passes of Clustal and Stockholm,
  and the steps after reading; the status bar repeats it. Cancel or Escape stops the load and
  frees what it read; the file that was open stays open. Opening another file cancels a load.
  Clustal and Stockholm files are now opened twice instead of three times.
- **PNG and SVG export** (Export menu): the view, or the selection (rows × shown columns), with
  names and ruler optional. SVG has the residue letters, names and ruler as text and is limited
  to 200,000 cells (larger regions get a message to export a PNG); PNG at 1×, 2× or 4×, cells
  drawn like the screen's tiles and text with resvg, at most 16,384 px a side and 100 megapixels.
- **Sorting and filtering rows**: Sort menu by name, identity to the consensus or to a reference
  row (set from the cursor), gap share or length, reversible; filter bar (Ctrl+L) by name or
  accession (substring or `/regex/`), minimum identity and maximum gap share, and "Show only this
  clade" from the tree; "N of M rows shown" and Clear. Filters keep tree order; a sort turns the
  tree panel off until tree order is chosen again. Copy and export take the rows shown.
- **Go to `name:position`** (Ctrl+G): a residue in the sequence's own numbering (from its
  `/start-end`), by name, accession or unique prefix, with messages for ambiguous or missing
  names and positions outside the sequence; a name alone selects its row.
- **Motif search** (Find box in Residues mode, Ctrl+Shift+F): plain letters (IUPAC codes for
  nucleotides), PROSITE patterns (`C-x(2,4)-C`) or `/regex/`, over the ungapped sequences so
  matches span gaps; run in the background, matches highlighted over the shown columns, Enter
  or F3 for the next one, a count in the status bar.
- **Tree menu** (right-click a node): collapse or expand, collapse to a depth, expand all, select
  the clade's rows, show only the clade, ladderize, reroot on the branch above the node.
- **Recent files**: the last 10 in the settings file; the empty window lists five.
- `--stats --motif PATTERN` and `--stats --sort KEY` time a motif search and a sort and filter.
- Ctrl+G, Ctrl+F and Ctrl+L also work while a text box has the focus.

## 0.3.0 (2026-11-11)

### msascope
- **msaview is now msascope**; the name MSAView belongs to an existing viewer, GMOD's. The binary,
  `msascope.app` (bundle id `com.dzulfikar.irit.msascope`), `msascope.dmg` and the
  `msascope-<arch>-linux.tar.gz` archives take the new name. The installer replaces an `msaview`
  it installed earlier with `msascope`, and still installs `msaview` from releases up to 0.2.1.
- **Colour schemes**: Clustal X with its per-column rules (Jalview's table), percent identity,
  hydrophobicity (Kyte–Doolittle), nucleotide and differences from the consensus, each with a
  "by conservation" fade, in light and dark. Letters on every fill keep at least 4.5:1 contrast
  (a test). Chosen from the toolbar's colour menu; `s` cycles, `d` differences, `c` conservation.
  Residues a scheme leaves unfilled are drawn as muted letters on the background, and gaps are
  the background with a faint dot, also in the dark theme.
- **Low zoom shows density, not sampled cells**: below one pixel per cell each pixel takes the
  majority residue and the share of differing cells of the cells it covers (from the pyramid or
  the cells, up to 32 samples). "Fit all" on Pfam PF00042 full (32,112 × 698) now shows conserved
  columns as stripes instead of noise.
- **Overview panel** on the right: the whole alignment, the visible area as a box (click or drag
  to pan), the visible range and the scheme's legend. Toggle in the toolbar.
- **Hover card** next to the pointer: sequence name, accession (`#=GS AC`) and range, residue
  with its position in the ungapped sequence (from the name's `/start-end`), column in the
  alignment's numbering and among the shown columns, consensus, identity and gaps. It replaces
  the hover text in the status bar.
- **Insert and gappy columns can be hidden** (toolbar, `i` and `g`). Insert columns are now kept
  at load: the columns where every row has lower case or `.`, or `#=GC RF` when the file has it.
  Residues are still stored as upper case, at 1 bit per column extra. Gappy means at least 25, 50
  (default), 75, 90 or 100% gaps. The ruler keeps the original column numbers and marks hidden
  runs; go-to takes original numbers. Inserts are hidden by default when a file has them.
- **Selection**: click names for rows (Ctrl adds or removes, Shift extends, drag extends), click
  or drag on the track header for columns, Shift+click or Shift+drag in the alignment for a
  block; plain drag still pans. Ctrl+A selects all, Escape clears. Selected rows show in the name
  panel, columns in the ruler, blocks with an accent wash and outline; the status bar gives the
  size.
- **Copy and FASTA export**: Ctrl+C copies the selection as FASTA, Ctrl+Shift+C as residues only;
  the Export menu writes the selection or all rows to a FASTA file. Hidden columns are left out
  unless "Include hidden columns" is on; "Leave out gaps" writes ungapped sequences. Rows come in
  the order shown. Above 64 MiB the copy goes to a file.
- **Cell cursor**: arrows move it (Shift extends a block, Ctrl jumps over runs of residues),
  Page Up/Down and Home/End move it by a page or to the row's ends; Alt+arrows pan as the arrows
  did before. The cursor shows in the ruler and the name panel, and the hover card follows it.
- **Drag-and-drop**: drop an alignment, a tree, or both on the window. Open… also takes trees.
- **Newick tree panel**: from `--tree`, Open or a drop, Stockholm `#=GF NH`, or a sibling
  `.nwk`/`.newick`/`.tree`/`.treefile` with the alignment's stem. Rows follow the tree (Sort:
  tree / file, `t`); clicking a node collapses its clade into one row showing the clade
  consensus; the status bar counts collapsed clades and reports unmatched leaves and sequences.
  The parser handles quoted labels, branch lengths, comments and NHX, and internal labels, without
  recursion. A synthetic 230,000-leaf tree reads and parses in 0.1–0.5 s and matches in
  0.3–0.8 s on the test laptop, in the background.
- The ruler's marks for hidden columns are now thin ticks, merged when closer than 6 px; the hover
  card over the ruler lists the hidden columns next to the column.
- `msagen --tree TREE.nwk` writes a tree matching the generated alignment.
- Toolbar icons that were not in the bundled icon set (the columns button) now use bundled ones,
  so they no longer show blank.

### dblock
- **Identical files inside a database are hard-linked** after unpacking, so they take disk space
  once. Files are matched by size, permissions and the SHA-256 dblock already records, so nothing
  is read twice. `gtdbtk-r232-placement` takes 2.7 GB on disk instead of 7.4 GB, and
  `gtdbtk-r232-placement-lite` 1.7 GB instead of 5.9 GB (measured with `du` on Linux).
  `verify`, `verify --deep` and GTDB-Tk folder hashes give the same results. The marker and lock
  entry record `linked_files` and `linked_bytes`. If the file system refuses hard links the copies
  are kept, with a note. `fetch --no-dedupe` keeps copies.
- `gtdbtk-r232-ani-screen` (2.1 GB download, 2.3 GB unpacked): the screen index for `gtdbprep ani`,
  from a new unofficial irit-data release derived from GTDB R232 (CC BY-SA 4.0).

### irit-mcp
- **Third-party tools for the microbial isolate path**: `fastp`, `bakta`, `checkm2` and
  `gtdbtk_classify_wf`. Each one offers exactly the flags of a pinned upstream version (fastp 1.4.0,
  Bakta 1.12.2, CheckM2 1.1.0, GTDB-Tk 2.8.0), read from that release's source. The calls go into
  the same run record and `replay.sh` as the irit tools, so a whole run from reads to taxonomy can
  be replayed without the AI.
  - The program is found on `PATH` or through the new `--tool-path NAME=PATH`. If it's missing,
    the call fails with a Bioconda install hint.
  - Inputs are checked before running (FASTQ or FASTA, plain or gzip, not empty). So are the
    databases: Bakta's `version.json` and files, CheckM2's `.dmnd`, and GTDB-Tk's data folders,
    including `skani/`, which `classify_wf` always needs, and the split-tree files unless
    `full_tree` is set.
  - The program's `--version` is recorded. A version other than the pinned one adds a warning to
    the result; `replay.sh` warns on a mismatch.
  - Exit 0 without the files the program always writes is an error (`missing_outputs`), recorded
    as a failed call.
  - Results carry small summaries: fastp totals, Bakta feature counts, CheckM2 completeness and
    contamination, and GTDB-Tk classifications. fastp, CheckM2 and GTDB-Tk files are read with
    isotable's parsers.
- `version` in run records is now the first line of `--version`, as the docs said.

### isotable
- The output parsers are now also a library (`isotable::parse`), so irit-mcp reads tool outputs the
  same way isotable grades them. The command-line tool is unchanged.

### gtdbprep
- **`gtdbprep ani`: GTDB-Tk's ANI species screen without the 85 GB skani database.** The genomes
  are screened locally against a compact index of every R232 reference's marker k-mers. Only the
  references skani would compare in full are fetched, by HTTP range requests, and GTDB-Tk's own
  `ANIScreener` then runs on just those. Sketches can come from GTDB's own `.tar.gz` (through a
  gzip seek index, the default), an unpacked `sketches.db` (local or hosted), or one split into
  parts. On test genomes the output was byte-identical to GTDB-Tk against the full database. See
  [the research page](docs/research/ani-laptop.md).
- `gtdbprep ani-index build | gz-scan | gz-attach` make the screen index and the seek index from
  a full skani database and GTDB's archive (maintainers).
- **Reading sketches from GTDB's archive is a stopgap** (about 5.7 times the sketch bytes, and load
  on GTDB's server): keep a `--cache`. Peak memory 1.9 GB for 9 genomes; GTDB-Tk on the full
  database takes 14.7 GB.

## 0.2.1 (2026-11-10)

### Fixed
- **macOS: the command-line tools aborted at launch** with "Library not loaded:
  /opt/homebrew/opt/xz/lib/liblzma.5.dylib" (or "different Team IDs" where Homebrew's xz is
  installed). The 0.2.0 builds linked the build machine's Homebrew liblzma, which a signed binary
  can't load and most Macs don't have. xz support is now built in, and the release fails if a macOS
  binary links anything outside the system libraries.

### msaview
- **Try a sample** button: opens a synthetic 2,000 × 3,000 nucleotide alignment, so there is
  something to look at before you have a file.

### dblock
- `tar.zst` archives (long-window ones too), in both download and streaming modes.
- Multi-part entries: several archives (`parts`, each with its own MD5 and optional SHA-256)
  unpacked into one database folder that takes its final name only after every part passes; each
  part resumes on its own.
- `gtdbtk-r232-placement` (280 MB download, 7.4 GB unpacked) and `gtdbtk-r232-placement-lite`
  (175 MB, 5.9 GB, no full bacterial tree): GTDB-Tk R232's placement folders from an unofficial,
  unmodified mirror of GTDB's data (CC BY-SA 4.0), checked against GTDB-Tk's `REF_HASHES`. No ANI
  database, so classification stops at genus.

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
