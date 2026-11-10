# The ANI species check on a laptop

GTDB-Tk 2.7+ starts `classify_wf` with an ANI screen. It runs `skani search` against a
pre-sketched database of all 199,923 GTDB R232 species representatives. A genome gets the species
of its closest reference with AF >= 0.5, if the ANI reaches that reference's radius. The database
is 85.2 GB, so a laptop can't hold it. Without it, classification stops at genus.

This page asks: can a laptop screen a genome locally, fetch only the few reference sketches it
needs with HTTP range reads, and get the same result as GTDB-Tk? Checked on 2026-11-10 against
skani 0.3.1 (tag `v0.3.1`), GTDB-Tk 2.8.0 (commit `155d989`) and the R232 package. Numbers are
**measured** (run here or on a GitHub runner), **cited** (with a source), or **estimate**.

## Summary

TODO(run)

## 1. How `skani search` uses its database

Read from skani 0.3.1's source: `src/search.rs`, `src/screen.rs`, `src/sketch_db.rs`,
`src/parse.rs`, `src/types.rs`, `src/file_io.rs`.

A consolidated skani database is a folder of three bincode files (fixed-width little-endian
integers, `u64` lengths) plus `metadata.tsv`:

| File | Contents | R232 size |
|---|---|---:|
| `markers.bin` | `(SketchParams, Vec<Sketch>)`: one marker-only sketch per reference (name, contig names, marker k-mers) | 9.86 GB (cited, [placement test](gtdb-placement-test.md)) |
| `index.db` | `Vec<{file_name, offset, length}>`: where each reference sits in `sketches.db` | 40 MB (cited) |
| `sketches.db` | each reference's full `(SketchParams, Sketch)`, back to back | 75.27 GB (cited), about 376 KB per reference |

What `skani search -d <db> --ql <list>` (GTDB-Tk's call, default preset) does:

1. **Loads all of `markers.bin`** into memory: every reference's set of marker k-mers. Markers
   are canonical 21-mers sampled at about 1 in 1,000 positions (`marker_c = 1000`).
2. **Sketches each query** with the database's parameters (c = 125, k = 15, marker c = 1000).
3. **Screens every reference, one pair at a time.** A reference passes when the query and the
   reference share at least `max(1, floor(0.8^21 × min(|Q|, |R|)))` marker k-mers, where `|Q|`
   and `|R|` are the two marker set sizes (`check_markers_quickly`, screen identity 0.80 by
   default). With more than 50 query files, skani builds an inverted index instead and requires
   strictly more than that number (`screen_refs_indices`). That passes a subset of the same
   references, so any candidate set built with `>=` covers both.
4. **For each passing reference `j`**, reads entry `j` of `index.db` and memory-maps that slice
   of `sketches.db` (`SketchDbReader::get_sketch`). The positions in `markers.bin` and `index.db`
   line up.
5. **Computes ANI and AF for that one pair** (`chain_seeds`), and keeps it if ANI > 0.5. Nothing
   in this step looks at other references.

So the result for a query depends only on which references pass step 3 and on those references'
own sketches. A database that holds only those references gives the same rows. We checked this
on GTDB's mockup database (5 references): skani 0.3.1 searched a database rebuilt from subsets of
it, and the rows for the references it held were byte-identical to the full database's
(**measured**).

**One shortcut that keeps skani's own code in charge.** A rebuilt `markers.bin` may hold the
*full* sketches instead of marker-only copies. `skani search` reads only the marker k-mers and the
name from it. So the client builds the small database by concatenating the records it fetched,
without decoding or re-encoding any sketch (**measured** on the mockup: identical rows).

**What GTDB-Tk adds** (`gtdbtk/ani_screen.py`, `external/skani.py`, `ani_rep.py`): it parses the
skani rows, takes the closest reference with AF >= 0.5, checks that reference's radius
(`radii/gtdb_radii.tsv`), and writes `classify/ani_screen/<prefix>.{bac120,ar53}.ani_summary.tsv`
(with the other references that pass AF >= 0.5, closest first, at most 50) and
`<prefix>.ani_screen_unassigned_hits.tsv.gz` (every hit of genomes it could not assign; `classify`
reads it back for its notes). The client runs this same code (`ANIScreener.run_aniscreen`), with
the small database in place of the full one. Nothing about the species rule is rewritten.

## 2. What the client needs locally: the screen

The screen needs, for every reference, its marker set size and which of the query's markers it
has. That information is spread across all 199,923 references, so the client needs all of it in
some form. Options:

| Option | Download | Exact? |
|---|---|---|
| `markers.bin` as is | 9.86 GB (cited) | yes, but it also carries every contig name |
| **32-bit fingerprints of each reference's markers, Rice-coded, plus exact set sizes** (built here) | TODO(run) | yes: a fingerprint never misses a shared marker, so the count can only be too high, and every reference skani keeps is kept. A few extra may pass; skani screens them again on the small database |
| a subsample of markers (e.g. 1 in 10) | about a tenth | no: only very likely to keep every reference |
| range-reading posting lists per query marker | a small directory; then about one request per query marker (3,000–6,000 for a typical genome) | yes, but thousands of requests per genome |

The fingerprint index (`gtdbprep ani-index build`) is one sequential pass over `markers.bin`,
`index.db` and `sketches.db`. Per reference it stores a 41-byte row in `refs.bin` (record offset
and length, exact marker count, where its fingerprints are, and the first 16 bytes of the
record's SHA-256) and its sorted fingerprints in `markers.idx`. Screening reads `markers.idx` once
from front to back, for any number of genomes at once.

TODO(run): sizes, markers per reference, screen time and memory.

## 3. Where the full sketches can come from

The client then needs the full sketch of each reference that passed (about 376 KB on average).

**GTDB's server supports range requests, but serves only the archive.** `data.gtdb.ecogenomic.org`
(nginx) answers `HEAD` on `gtdbtk_r232_data.tar.gz` with `accept-ranges: bytes`, returns 206 for a
single range and `multipart/byteranges` for several (**measured**, about 0.9 s per request from
Indonesia). But the package is one 60.8 GB gzip stream. Gzip can't be entered in the middle
without the 32 KiB of output before that point.

**A seek index makes GTDB's own archive readable by range.** zlib's `zran.c` example shows how:
decompress the stream once and note "checkpoints" at deflate block boundaries (compressed byte and
bit position, uncompressed position, and the 32 KiB window before it). From a checkpoint, a raw
inflater primed with those bits and that window carries on. To read one sketch, fetch the
compressed bytes from the checkpoint before it to the checkpoint after it and decompress. The cost
is one window (32 KiB) per checkpoint in the index, and on average half a checkpoint spacing of
extra download per fetch. `gtdbprep ani-index gz-scan` builds this index while the archive streams
past (it can share the one download with `tar -x`). It records checkpoints only across
`skani/database/sketches.db`.

TODO(run): sizes by spacing, download per genome.

**Or re-host `sketches.db` unpacked.** GitHub Release assets support single range requests (206,
**measured**; `release-assets.githubusercontent.com`, Azure Blob behind a 302 redirect, about
0.12 s per request) but not multi-range ones (501, **measured**). Assets must be under 2 GiB, so
75.27 GB means about 38 parts. The client handles numbered parts (`--source
'<url>/sketches.db.{part}' --part-size N`). Cloudflare R2 would cost about $1.13/month for 75 GB
(R2 Standard at $0.015 per GB-month, free egress; cited in [split-gtdb.md](split-gtdb.md#7-hosting-and-cost)).
Each fetch is then exactly one sketch.

**Or GTDB could publish the folder unpacked**, as asked for the placement folders in
[GTDBTk#727](https://github.com/Ecogenomics/GTDBTk/issues/727). Their server already supports
range requests, so the client would read sketches straight from the source with no seek index.

## 4. Verification on a GitHub runner

TODO(run)

## 5. What it costs a laptop

TODO(run)

## 6. Licences and credit

- **GTDB data** (the skani database, radii, taxonomy) is CC BY-SA 4.0 (Parks et al., the Genome
  Taxonomy Database, <https://gtdb.ecogenomic.org>). The screen index and the seek index are
  derived from it, so they are CC BY-SA 4.0 too and must credit GTDB if they are published. The
  sketches the client fetches are GTDB's, unmodified.
- **skani** (Shaw and Yu, *Nature Methods* 2023, MIT licence) computes every ANI and AF. The
  client runs the skani 0.3.1 binary; it reimplements only skani's file layout and the screen
  threshold, to choose which references to fetch.
- **GTDB-Tk** (Chaumeil et al.; GPL-3.0) applies the species rule and writes the output. The client
  calls its own `ANIScreener`.
- **zran** (Mark Adler, zlib licence) is the method behind the seek index; the inflater is zlib-rs
  (`libz-rs-sys`, zlib licence).

## Sources

- skani 0.3.1: [bluenote-1577/skani](https://github.com/bluenote-1577/skani) at tag `v0.3.1`: `src/search.rs` (`search`), `src/screen.rs` (`check_markers_quickly`, `screen_refs_indices`), `src/sketch_db.rs` (`SketchDbWriter`, `SketchDbReader`), `src/sketch.rs` (writing `markers.bin`), `src/types.rs` (`Sketch`, `get_markers_only`), `src/params.rs` (`SEARCH_ANI_CUTOFF_DEFAULT`, `K_MARKER_DNA`, `FULL_INDEX_THRESH`), `src/parse.rs` (`parse_params_search`)
- GTDB-Tk 2.8.0 at `155d989`: `gtdbtk/ani_screen.py`, `gtdbtk/ani_rep.py` (`ANISummaryFile`, `SkaniRawHitsFile`), `gtdbtk/external/skani.py` (`run_all_vs_all`), `gtdbtk/classify.py` (`formatnote`, `parse_radius_file`), `gtdbtk/main.py` (`ani_screen`, `_genomes_to_process`)
- zlib `examples/zran.c` (Mark Adler): [github.com/madler/zlib](https://github.com/madler/zlib/blob/develop/examples/zran.c)
- GTDB R232 package: [full_package](https://data.gtdb.ecogenomic.org/releases/release232/232.0/auxillary_files/gtdbtk_package/full_package/), [mockup_db](https://data.gtdb.ecogenomic.org/releases/release232/232.0/auxillary_files/gtdbtk_package/mockup_db/)
- Earlier pages: [split-gtdb.md](split-gtdb.md), [gtdb-placement-test.md](gtdb-placement-test.md), [gplace-checks.md](gplace-checks.md)
