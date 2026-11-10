# msaview: large alignment viewer

`msaview` is a desktop viewer for multiple sequence alignments that stays smooth at sizes where the
usual viewers give up. 100,000 sequences × 30,000 columns loads in about 5 seconds into 1.6 GB of
RAM. The whole alignment then pans at the display's 60 frames per second, from the full overview
down to individual letters.

## Why

Jalview and AliView (Java) and SeaView show a few hundred rows comfortably, fail silently when they
run out of memory, and stall somewhere between 10^4 and 10^5 sequences. Surveillance alignments
(SARS-CoV-2, influenza, mpox) are now 10^5–10^6 genomes of ~30 kb. No native desktop viewer handles
that size; a WebGL viewer (Alignment Viewer 2.0) reported 10–100× gains from drawing on the GPU, so
`msaview` draws with GPUI, Zed's GPU-accelerated UI framework. See
[landscape](../research/landscape.md#2-viewers).

## Usage

```sh
msaview alignment.fasta          # or .fa.gz, .aln (Clustal), .sto (Stockholm)
msaview                          # then Open… (Ctrl+O)
msaview alignment.fasta --stats  # load, print sizes and timings, exit (no window)
msaview silk.fasta --type protein # residue type given instead of guessed: dna, rna or protein
```

`--type` also applies to files opened later with Open….

| Input | Action |
|---|---|
| Wheel / trackpad | Scroll (Shift+wheel: horizontal) |
| Drag | Pan |
| Ctrl+wheel | Zoom at the pointer (Ctrl+Shift: columns only, Ctrl+Alt: rows only) |
| `+` / `-` / `0` | Zoom in / out / fit the whole alignment |
| Arrows, Page Up/Down, Home/End | Move; Ctrl+Home/End jumps to the corners |
| Ctrl+G | Go to a column (`15000`) or row and column (`250:15000`) |
| Ctrl+F | Find sequences by name (substring, case-insensitive); Enter / Shift+Enter for next/previous |
| `d` | Colour mode: residues, or differences from the column consensus |
| Click | Select a row; hovering shows row, column, residue, consensus and identity in the status bar |

Zoom goes from the whole alignment (fit to the window, columns and rows scaled separately) to
about 32 pixels per cell in steps of √2. Letters appear once cells are at least 8 × 10 pixels.

The window shows, from the top: a toolbar, the position ruler, a consensus track (most common
residue per column) and an identity track (bar height = share of sequences carrying the consensus
residue; dark green ≥ 90%, light green ≥ 50%, amber below). The sequence names stay in a column on
the left while the alignment scrolls; when rows are too thin to label, it shows the visible row
range and a marker of where the view sits in the whole alignment.

Colours: nucleotides A green, C blue, G orange, T/U red, ambiguity codes grey; amino acids by
Clustal X groups (hydrophobic blue, positive red, negative magenta, polar green, plus C, G, P and
aromatic H/Y). The **differences** mode fades every residue that matches its column's consensus and
keeps the rest in full colour, which is what you want for alignments of near-identical genomes.

## Formats

| Format | Detected by | Notes |
|---|---|---|
| Aligned FASTA | first line starts with `>` | Wrapped or unwrapped, `\r\n` line ends, gzip (`.gz`, detected by content) |
| Clustal | `CLUSTAL`, `MUSCLE` or `PROBCONS` header | Consensus lines and trailing residue counts are ignored |
| Stockholm | `# STOCKHOLM` header | First alignment in the file; `#=GF/GS/GR/GC` markup is skipped; blocks must be separated by a blank line |

A UTF-8 byte order mark and blank lines before the header are ignored.

Every sequence must have the same number of columns; a file that isn't aligned is refused with the
name of the first sequence that differs. In Clustal and Stockholm, a name that appears twice in one
block is refused with that name and its line; that is also what happens to a Stockholm or Clustal
file whose blocks have no blank line between them (the repeated names look like duplicates), so add
the blank lines. `-`, `.` and `~` are gaps; lower case is shown as
upper case, so Stockholm insert columns (lower case and `.`) look the same as match columns and
gaps. `?` is read as N (nucleotides) or X (protein). In nucleotides, hard-masked `X` and `*` are
kept as written.

The residue type comes from the residues (gaps excluded) of the whole file for Clustal and
Stockholm, and from the first 64 K residues across the first rows for FASTA. It is nucleotides when
no residue is a letter only an amino acid can have (E, F, I, L, P, Q, …) and at least 90% of the
residues are A, C, G, T, U or N; otherwise protein. `X`, `*` and `?` count neither way, so
hard-masked DNA stays DNA, and a sequence made only of them is protein. The 90% share matters
because many real proteins use only letters that are also IUPAC nucleotide codes (silk fibroin
`GAGAGS…`, glycine-rich and cysteine-rich peptides); they fall far below it, while DNA with
ambiguity codes stays far above it. Short or low-complexity sequences can still be read the wrong
way: an antifreeze glycoprotein `AATAATAAT…` is letter for letter the (AAT)n DNA repeat. Give
`--type protein` (or `dna`, `rna`) when the guess is wrong; with `--type`, a residue that doesn't
fit the type is reported instead of switching type.

Nucleotides are shown as DNA (T), RNA (U), or, when both T and U occur, as DNA/RNA with T and U kept
apart. If a later FASTA row needs a wider type (a U in a DNA alignment, an `X` or `*`, or an
amino-acid-only letter), the rows read so far are converted; in the one case where that would lose
information (a `?` already stored as N, which must become X), the file is read again from the
start.

## Design

The core (parsing, storage, summaries, view geometry, tile drawing) is a plain Rust library with
unit tests and no GUI dependency; the GPUI code only handles windows, input and painting.

**Storage.** One small code per residue, row-major. Nucleotides use 16 codes (gap plus the 15 IUPAC
letters) packed two per byte, so a 30 kb genome takes 15 kB; amino acids take one byte. Rows are
stored in blocks of 1,024 so the matrix can grow while a file streams in, without reallocating
(and briefly doubling) gigabytes. Lower case is not kept and `.` becomes `-`. An alignment that has
both T and U needs a 17th code, and one with `X` or `*` needs codes beyond the 16, so those are
stored at one byte per residue like protein (no residue is rewritten to fit the 4-bit codes).

**Loading.** FASTA is read in 64 MB batches; the records in each batch are encoded into their rows
in parallel. Plain and gzip input take the same path and memory stays at the packed matrix plus one
batch. (The first version memory-mapped the file instead: parsing was 1 s faster, but the mapped
3 GB file counted towards resident memory and peaked at 4.3 GB.) If the type widens after rows
were packed, the packed rows are converted to one byte per residue, briefly holding the old and the
new matrix. Clustal and Stockholm are read twice, line
by line: the first pass collects names, row lengths and the residue counts that decide the type,
the second encodes each line straight into its row, so memory is the packed matrix plus the names
(gzip is decompressed twice). That path is single-threaded.

**Column summaries.** One pass counts every residue per column (consensus, identity, gap share).
The work is split by blocks of 1,024 columns so each thread's counters (128 KB) stay in cache, and
each block keeps only the three results per column (9 bytes) once counted;
split by rows instead, it took 15 s rather than 1 s on the 100,000 × 30,000 test.

**Pyramid.** For zoomed-out views, level *k* summarises blocks of 8·2^k × 8·2^k cells with the most
common code and the share of the block's cells (gaps included, so a gappy block looks calmer)
holding a residue that differs from its column's consensus; N and X never count as differences.
Level 0 is counted from the cells. Each higher level combines 2 × 2 blocks of the one below, each
weighted by the number of cells it covers, so the short blocks at the right and bottom edges don't
count as much as full ones: the difference share stays exact up to rounding, while the most common
code is the child majority with the most weight, an approximation of the true majority. Ties go
to a residue rather than the gap, then to the lower code. For 100,000 ×
30,000 the pyramid takes 119 MiB next to the 1,430 MiB matrix.

**Rendering.** GPUI does not expose custom shaders, so the alignment area is drawn as image tiles
of 256 × 256 device pixels. A tile is rasterised once on the CPU (in parallel across tiles, one
pixel = one cell or one pyramid block, picking the coarsest pyramid level that is still at least
one block per pixel) and then composited by GPUI's GPU sprite atlas; tiles are cached per zoom
level and colour mode, and evicted (and freed from the GPU atlas) when far off screen. No UI element
is created per cell. When letters are visible, each distinct letter is shaped once per frame and
painted per visible cell. Tracks are drawn as runs of equal-colour quads, one entry per pixel
column.

## Measured

Machine: AMD Ryzen 5 PRO 4650U (6 cores, 12 threads), 30 GB RAM, Arch Linux (kernel 7.2.11),
Hyprland 0.56 on Wayland, built-in 1920 × 1080 display at 60 Hz with 1.25 scaling, integrated
Radeon graphics. Rust 1.99.1, release build (thin LTO). Test data from `msagen` (below) in `/tmp`,
which is RAM-backed (tmpfs) here, so these are warm-cache numbers; reading from a disk adds the
time to read the file.

Gzip loading is limited by decompression, which runs on one thread; the parsed batches are still
encoded in parallel. The gzip row is a single run; four loads of the plain file (one with
`--stats`, three inside the viewer) took 5.1–5.4 s.

Release binaries: `msaview` 36 MB (most of it GPUI and the bundled icon set), `msagen` 0.4 MB.

Peak resident memory is the process's own high-water mark (`VmHWM`), also checked from outside
with `getrusage` on the child process; MiB = 2^20 bytes.

| Alignment | File | Load (parse + columns + pyramid) | Peak RSS, no window (`--stats`) | Peak RSS, viewer |
|---|---|---|---|---|
| 1,000 × 1,000 DNA | 1.0 MB | under 0.01 s | 9 MiB | 126 MiB |
| 100,000 × 30,000 DNA | 3.0 GB | 5.1 s (2.1 + 1.1 + 1.7) | 1,567 MiB | 1,746 MiB |
| 100,000 × 30,000 DNA, gzip | 1.18 GB | 17.1 s (13.8 + 1.2 + 1.9) | 1,572 MiB | not measured |

The viewer column is the high-water mark just after loading for the small file, and over the whole
`--bench` session (four zoom levels of panning) for the large one.

**Frame smoothness.** `msaview FILE --bench` opens the window and pans continuously, by 1/12 of
the view width and 1/24 of its height every frame for 240 frames, at four zoom levels; it records
the time between frames and the time spent in the alignment's paint callback. On the 100,000 ×
30,000 alignment:

| Zoom | Frame interval median / p95 / max | Frames over 25 ms | Paint median / p95 | Tiles drawn |
|---|---|---|---|---|
| Whole alignment (fit) | 16.7 / 19.4 / 66.0 ms | 2 of 240 | 7.0 / 12.0 ms | 28 |
| 1 px per cell | 16.7 / 18.9 / 35.3 ms | 1 | 5.4 / 13.5 ms | 853 |
| 4 px per cell | 16.7 / 17.0 / 17.2 ms | 0 | 1.8 / 3.2 ms | 853 |
| Letters (11 × 16 px) | 16.6 / 20.6 / 25.9 ms | 1 | 6.8 / 15.3 ms | 853 |

16.7 ms is the display's refresh interval, so the median frame is never late. The occasional slow
frame comes when the pan crosses into a new row of tiles and several are rasterised at once, or
when the view switches zoom (the 66 ms maximum is the first frame of the overview phase). These
are measurements from an automated pan, not a judgement by eye with a real mouse. At the fit zoom the whole alignment is on screen, so the pan has nothing to reveal: that row
measures redrawing, not tile drawing. One machine and one display so far; 120 Hz displays and
weaker GPUs are untested.

## Test data

`msagen ROWS COLS OUT [--protein] [--seed N]` writes a synthetic alignment (OUT ending in `.gz` is
gzipped; ROWS and COLS must be at least 1, and write errors end it with a non-zero exit): a random reference, 32 clades with shared mutations, and per sequence a few
substitutions, an occasional deletion, an N run and ragged ends, like a viral surveillance
alignment. It writes the 3 GB, 100,000 × 30,000 file in under 2 seconds.

```sh
msagen 1000 1000 /tmp/small.fa
msagen 100000 30000 /tmp/big.fa
```

Generated files are for testing only and are not kept in the repository.

## Building

`msaview` needs a desktop build environment (GPUI); the other leanbio tools do not. On
Debian/Ubuntu, install `pkg-config libfontconfig-dev libxkbcommon-dev libxkbcommon-x11-dev` first.
This list comes from the build scripts and linked libraries, and is not yet confirmed on a clean
Ubuntu install. macOS and Windows should need nothing extra, but neither has been built yet. The core library and `msagen` build without GPUI:
`cargo test --no-default-features`.

It uses GPUI through Longbridge's `gpui-kit` 0.9.1, which pins a GPUI snapshot (`gpui-pre`
0.3.12); the `gpui` crate on crates.io is from October 2025. Versions are pinned exactly because the
API changes often.

## Known gaps

- Single machine tested (Linux, Wayland). macOS and Windows builds are untested.
- Mouse wheel, drag and the Open… dialog were not exercised by an automated test; zoom keys, colour
  mode, go-to and search were (driven through the keyboard while taking screenshots).
- The whole alignment must fit in RAM: about half a byte per nucleotide cell, so 10^6 × 30 kb needs
  about 15 GB. A memory-mapped packed cache file would lift that.
- Gzip input loads about 3× slower than plain FASTA because decompression is single-threaded.
- Loading blocks the view (a "Loading…" status, no progress bar) and cannot be cancelled.
- Zoom steps are √2 apart, so "fit" can leave up to ~30% of the window empty.
- Upper pyramid levels approximate the majority (area-weighted majority of majorities).
- Clustal and Stockholm load on one thread and read the file twice; fine for typical sizes, slower
  than FASTA for very large files.
- No editing, selection of columns, export, tree panel, sequence reordering or sorting by
  similarity. A linked tree panel is planned next ([roadmap](../roadmap.md)).
- Lower-case residues (often used for unaligned or masked regions) are shown as upper case, and `.`
  as `-`: Stockholm insert states are not told apart from match states.
