# Changelog

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
