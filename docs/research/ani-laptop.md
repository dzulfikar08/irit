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

- **Yes, and the result is identical.** `gtdbprep ani` screens genomes locally, fetches only the
  reference sketches skani would compare, and runs GTDB-Tk's own ANI-screen code on them. On 9
  genomes, its `ani_summary.tsv` files and unassigned-hits file were byte-identical to GTDB-Tk
  2.8.0 against the full 85 GB database. That held with the sketches read from a local
  `sketches.db` and with them range-read from GTDB's own server (**measured**, GitHub runner).
- **The local index is 1.76 GB, not 85 GB.** It holds every reference's exact marker count and a
  32-bit fingerprint of each of its 644,245,721 marker k-mers. `markers.bin` is 9.86 GB. Building
  the index took 3 min 40 s with 73 MB of RAM (**measured**).
- **But a genome needs more references than hoped.** skani's screen is loose: it keeps every
  reference that shares about 1% of the marker k-mers. On complete genomes it kept 2 to 1,090
  references (B. subtilis 23, E. coli 420, P. aeruginosa 1,090), and 1,941 for a 400 kb piece of
  E. coli. All 9 genomes together needed 3,166 of the 199,923 references: 1.6 GB of sketches
  (about 510 KB each). Every one of them is needed to reproduce GTDB-Tk's files exactly, because
  the files list all close references, not just the species call (**measured**).
- **GTDB's server supports range requests, but serves one 60.8 GB gzip stream.** A seek index
  (checkpoints with 32 KiB windows, after zlib's `zran.c`) makes it readable by range. The cost is
  the index size and extra download per sketch: with a checkpoint every 1 MiB, a 2.29 GB index and
  3.3 GB downloaded for the 1.6 GB of sketches; every 4 MiB, 0.35 GB (zstd) and 9.3 GB; every 16
  MiB, 0.09 GB and 26.3 GB (**measured**).
- **Re-hosting the unpacked `sketches.db` (75.3 GB) would cut the download to the sketches
  themselves.** It was decided not to re-host it.
- **Memory.** GTDB-Tk's own screen on the full database peaked at 14.7 GB of RAM (skani 0.3.1) and
  15.4 GB (skani 0.4.0). The client peaked at 1.9 GB for the same 9 genomes (**measured**).
- **Published.** The screen index is on irit-data,
  [`gtdbtk-r232-ani-screen-v1`](https://github.com/dzulfikar08/irit-data/releases/tag/gtdbtk-r232-ani-screen-v1)
  (2.1 GB, CC BY-SA 4.0), as the `dblock` entry `gtdbtk-r232-ani-screen`. Reading sketches from
  GTDB's archive is opt-in (`--source gtdb`), at GTDB's request, and fetched sketches are cached
  per user.
- **skani 0.4.0 gave the same files as 0.3.1** on these genomes, against the same 0.3.1 database.
- **Later additions** ([section 7](#7-parallel-requests-and-a-species-only-mode)): parallel range
  requests, which cut 377 sketches (1.23 GB) from GTDB's server from 1,167–2,275 s to 474–562 s, and the
  opt-in `--species-only`. On six genomes it fetched 47 of the 882 screened-in references with
  the same species calls, but lists fewer unassigned hits than GTDB-Tk.

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

**The small database is written in skani's own format.** Its `sketches.db` is the fetched records,
unchanged. Its `markers.bin` holds each record in marker-only form (skani's `get_markers_only`:
the same bytes without seeds, seed positions and contig lengths). On the mockup these were
byte-identical to GTDB's own `markers.bin` entries (**measured**). A first version put the full
sketches in `markers.bin` instead. That gave the same rows, but skani loads `markers.bin` whole,
so 3,166 references took 5.0 GB of RAM.

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
| **32-bit fingerprints of each reference's markers, Rice-coded, plus exact set sizes** (built here) | 1.76 GB (**measured**) | yes: a fingerprint never misses a shared marker, so the count can only be too high, and every reference skani keeps is kept. A few extra may pass; skani screens them again on the small database |
| a subsample of markers (e.g. 1 in 10) | about a tenth | no: only very likely to keep every reference |
| range-reading posting lists per query marker | a small directory; then about one request per query marker (3,000–6,000 for a typical genome) | yes, but thousands of requests per genome |

The fingerprint index (`gtdbprep ani-index build`) is one sequential pass over `markers.bin`,
`index.db` and `sketches.db`. Per reference it stores a 41-byte row in `refs.bin` (record offset
and length, exact marker count, where its fingerprints are, and the first 16 bytes of the
record's SHA-256) and its sorted fingerprints in `markers.idx`. Screening reads `markers.idx` once
from front to back, for any number of genomes at once.

Measured on R232 (GitHub runner, 4 vCPU):

| | |
|---|---:|
| references | 199,923 |
| marker k-mers | 644,245,721 (3,222 per reference on average, at most 15,955) |
| `markers.idx` | 1,748,824,109 bytes (21.7 bits per marker; zstd does not shrink it) |
| `refs.bin` | 8,196,843 bytes (5,703,776 with zstd -19) |
| build time, peak RAM | 3 min 40 s, 73 MB |
| screening 9 genomes | 4.6–9.5 s |

The fingerprints are close to the information they hold: 644 million markers at 32 bits each,
sorted per reference, need about 21 bits each. Fewer fingerprint bits would make small genomes
pull in many references by chance (their threshold is 1–3 shared markers).

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

R232 has one gzip member; `sketches.db` starts 3,072 bytes into the tar stream and is
75,270,701,831 bytes. `gz-scan` ran alongside the download and extraction (the whole stream took
57 min on the runner, with 2.2 GB peak RAM for the scanner). It kept 69,767 checkpoints at 1 MiB.

| Checkpoint every | `gz.idx` | with zstd -19 | Downloaded for the 9 genomes (1.62 GB of sketches) | Requests | Fetch time from the runner |
|---|---:|---:|---:|---:|---:|
| 1 MiB | 2.29 GB | about 1.4 GB (estimate) | 3.29 GB (2.0×) | 2,979 | 13.3 min |
| 4 MiB | 0.57 GB | 0.35 GB | 9.28 GB (5.7×) | 2,601 | 13.2 min |
| 16 MiB | 0.14 GB | 0.09 GB | 26.3 GB (16×) | 1,470 | 28.7 min |

All **measured**, except the 1 MiB compressed size, scaled from the other two. Requests went one
at a time, from a US runner to GTDB's server in Australia, so time is mostly latency at 1 MiB and
bandwidth at 16 MiB. Parallel requests were added later; see [section 7](#7-parallel-requests-and-a-species-only-mode).

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

Workflow [`ani-laptop.yml`](https://github.com/dzulfikar08/irit/blob/ani-laptop/.github/workflows/ani-laptop.yml),
[run 42860431958](https://github.com/dzulfikar08/irit/actions/runs/42860431958) (4 vCPU, 16 GB RAM,
12 GB swap):

1. Stream GTDB's archive once (MD5 `25a59e0352b1fd150c589f56559767d4`, as published). Extract
   `skani/`, `radii/`, `taxonomy/` and `metadata/`, and build the seek index from the same bytes.
2. Build the screen index.
3. **Reference:** GTDB-Tk 2.8.0's `ANIScreener.run_aniscreen` (what `classify_wf` calls first),
   against the full database, with skani 0.3.1 (as the database was made) and with skani 0.4.0
   (what Bioconda installs with GTDB-Tk 2.8.0).
4. **Client:** `gtdbprep ani` with the local `sketches.db` as the source, then with GTDB's server
   through seek indexes of 4, 16 and 1 MiB, each with an empty cache, and once more with a warm cache.
5. Compare `ani_summary.tsv` files byte for byte, and the unassigned-hits file row by row (GTDB-Tk
   writes those rows in set order).

| Genome | Size | Markers | References screened in | GTDB-Tk's call (all runs agree) |
|---|---:|---:|---:|---|
| GTDB's test genome G021783475 | 1.91 Mb, 138 contigs | 1,858 | 9 | GCA_021783475.1, ANI 100.0 |
| B. subtilis 168 | 4.22 Mb | 4,107 | 23 | GCF_000009045.1, 100.0 |
| E. coli K-12 MG1655 | 4.64 Mb | 4,419 | 420 | GCA_047199095.1 (*s__G047199095 sp047199095*), 97.14 |
| E. coli K-12, first 400 kb | 0.40 Mb | 385 | 1,941 | GCA_047199095.1, 96.78 |
| P. aeruginosa PAO1 | 6.26 Mb | 6,187 | 1,090 | GCF_001457615.1, 99.32 |
| M. genitalium G37 (code 4) | 0.58 Mb | 542 | 2 | GCF_000027325.1, 100.0 |
| N. lyticus TM7x (CPR) | 0.71 Mb | 710 | 176 | GCF_000803625.1, 100.0 |
| P. syntrophicum (Asgard) | 4.32 Mb | 4,355 | 15 | GCF_008000775.2, 100.0 (ar53) |
| Omnitrophota bacterium (2026, not in R232) | 2.49 Mb | 2,404 | 6 | not assigned; no skani hits |

**Every client run gave byte-identical `gtdbtk.bac120.ani_summary.tsv` (7 rows) and
`gtdbtk.ar53.ani_summary.tsv` (1 row) to the reference, and the same unassigned-hits file.**
That includes the "other related references" column, which lists up to 50 close references per
genome. skani 0.4.0 also gave identical files.

Notes:

- GTDB-Tk itself calls E. coli K-12 *s__G047199095 sp047199095* in R232 (ANI 97.14, AF 0.97),
  with *s__Escherichia coli* (GCF_003697165.2) second at 96.92. That is GTDB's result, not the
  client's; it is reported here only because it looks surprising.
- The unassigned-hits file was empty here: the one unassigned genome had no skani hits. Its
  content path is untested on real data.
- skani's other code path (more than 50 genomes: an inverted index with a strict `>`) was tested
  on the mockup only (51 genomes, identical rows). The runner step for it did not run.
- That run used the first version of the small database (full sketches in `markers.bin`). [Run
  42936739604](https://github.com/dzulfikar08/irit/actions/runs/42936739604) repeated the check
  with the marker-only version: identical files again from the local `sketches.db` and from GTDB's
  server (4 MiB seek index: 9.28 GB in 2,601 requests, 11.9 min), peak RAM 1.86 GB instead of
  5.0 GB.
- **More than 50 genomes on the full database did not fit the runner.** With 51 genomes, skani
  builds an inverted index of all 644 million marker k-mers. GTDB-Tk's own screen was killed
  inside a 12 GB + 6 GB swap memory cap, so there was nothing to compare with. The client ran the
  same 51 genomes in 33 s with 2.0 GB. That path is checked on the mockup only (identical rows).
  An earlier attempt without a cap took the runner down.
- [An earlier run](https://github.com/dzulfikar08/irit/actions/runs/42839315591) filled the
  runner's disk (85 GB database, swap, and several copies of the fetched sketches) and lost its logs.

## 5. What it costs a laptop

| | Full GTDB-Tk ANI screen | `gtdbprep ani` |
|---|---:|---:|
| One-time download | 60.8 GB archive (85.2 GB unpacked skani database) | 1.76 GB screen index + seek index (0.35 GB at 4 MiB) + 0.06 GB taxonomy and radii (in the placement mirror) |
| Disk | 85.2 GB | about 2.1 GB, plus the sketch cache (about 0.5 MB per reference used) |
| RAM | 14.7 GB for 9 genomes; more than 18 GB for 51 (measured) | 1.9 GB for 9 genomes, 2.0 GB for 51 (measured) |
| Per genome | nothing | 2–1,941 sketches here; about 0.5 MB each from an unpacked copy, 2–6× that through GTDB's archive |
| Time for the 9 genomes | 1 min 46 s – 2 min 33 s | 20–24 s with a local source; 12–13 min from GTDB's server (one request at a time); 15–18 s warm |

The per-genome cost depends on the lineage. Well-sampled genera (Pseudomonas, the Escherichia
group) and small or partial genomes pull in hundreds to thousands of references. A cache shared
between runs helps a lab that works on the same taxa: the second run here fetched nothing.

**A cheaper mode, for the species call only.** The species call only needs references that can
reach their radius (ANI >= 95% and AF >= 0.5). A much stricter screen fetches far fewer
references, but the unassigned-hits file, and possibly the "other related references" column,
then list fewer rows than GTDB-Tk's. It is now built as the opt-in `--species-only`
([section 7](#7-parallel-requests-and-a-species-only-mode)).

## 6. What was decided

1. **The screen index is published** on irit-data, release
   [`gtdbtk-r232-ani-screen-v1`](https://github.com/dzulfikar08/irit-data/releases/tag/gtdbtk-r232-ani-screen-v1):
   `gtdbtk-r232-ani-screen-core.tar.zst` (5.7 MB), `-markers.tar.zst` (1.75 GB) and
   `-gzindex.tar.zst` (0.35 GB, the 4 MiB seek index), with `SHA256SUMS` and `MD5SUMS`. `dblock`
   fetches it as `gtdbtk-r232-ani-screen`. It was built on a runner
   ([run 42936739602](https://github.com/dzulfikar08/irit/actions/runs/42936739602)) with
   `exp/ani/pack.sh`.
2. **Reading sketches from GTDB's archive is opt-in.** It downloads about 5.7 times the sketch bytes
   and loads GTDB's server. GTDB was asked to publish `skani/database/` unpacked; they have no
   plans to, and asked instead that reading from their server be opt-in and that a cache be on by
   default ([GTDBTk#727](https://github.com/Ecogenomics/GTDBTk/issues/727)). Now
   `--source` has no default (`--source gtdb` reads GTDB's archive), and fetched sketches go to one
   cache per user (`~/.cache/irit/gtdbprep-ani`), so no sketch is fetched twice on a machine. A
   local or split `sketches.db` works with `--source`; `sketches.db` is not re-hosted.
3. **A "species call only" flag** (fetch only references that can reach their radius) was
   planned as an opt-in for later; it is now `--species-only` (section 7).

## 7. Parallel requests and a species-only mode

Added after the first version (2026-11-12): `--fetch-jobs` (parallel range requests, default 4)
and `--species-only`. Measured on a 12-core laptop in Indonesia on a home connection, reading
from GTDB's server in Australia through the 4 MiB seek index (`--source gtdb`), each run with its
own empty cache. All numbers are **measured**. Test genomes: a *B. subtilis* 168 assembly
(Shovill/SKESA, from ENA run DRR000852), and from NCBI *S. aureus* NCTC 8325 (GCF_000013425.1),
*M. jannaschii* DSM 2661 (GCF_000091665.1, archaeon), *M. tuberculosis* H37Rv (GCF_000195955.2) and
*E. coli* K-12 MG1655 (GCF_000005845.2).

### Parallel range requests

The client plans the same requests as before (one per sketch, or one per group of sketches near
one checkpoint), then runs them from a pool of workers with at most `--fetch-jobs` in flight.
Each sketch is checked against the index and written to the cache as it arrives. That keeps
memory bounded (at most about twice `--fetch-jobs` requests' bytes are held at once), and an
interrupted run keeps what it fetched. Failed requests are retried with growing waits and
jitter, `Retry-After` is honoured, and a stalled request is cut off and retried.

Fetch phase only (screen and GTDB-Tk excluded), sequential (irit 0.3.2) against 4 parallel
requests. Every run of a genome fetched the same bytes in the same number of requests, and
wherever both were run, the outputs matched: byte-identical `ani_summary.tsv` files and the same
unassigned-hits rows. The fetch time includes writing the small database (well under a second).

| Genome | Sketches | Downloaded | Requests | One at a time (s) | 4 in parallel (s) |
|---|---:|---:|---:|---|---|
| M. tuberculosis H37Rv | 377 | 1,225.6 MB | 370 | 2,275 (with a 10-minute stall), 1,167 | 474, 562 |
| E. coli K-12 | 420 | 1,324.9 MB | 406 | not run | 502 |
| S. aureus NCTC 8325 | 28 | 86.0 MB | 28 | 288, 69, 55, 53, 44 (median 55) | 47, 42, 38, 30, 29 (median 38) |
| B. subtilis 168 | 21 | 60.8 MB | 21 | 71, 50 | 25, 185 |
| M. jannaschii | 19 | 60.4 MB | 19 | 34, 50 | 31, 43, 59, 164 |

What this shows:

- **For hundreds of sketches, parallel requests were 2 to 5 times faster.** In two pairs of runs
  on *M. tuberculosis*, one request at a time took 2,275 s (including a stall, below) and 1,167 s
  (1.05 MB/s), four at a time 474 s and 562 s (2.2–2.6 MB/s). Neither parallel run retried
  anything.
- **For a few dozen sketches, the connection's own variation is as large as the effect.** Runs
  alternating between the two on *S. aureus* favour parallel requests (median 38 s against 55 s),
  but two parallel runs on the other small genomes took 3–4 times as long as the sequential runs
  before them. Those two ran before retries were logged, so whether they retried is not known;
  the later runs logged no retries. More runs at different times of day would be needed to say
  more.
- **Stalls.** The first sequential *M. tuberculosis* run sat for about 10 minutes on a
  connection that delivered nothing, until irit 0.3.2's 600 s request timeout fired. One
  sequential *S. aureus* run took 288 s instead of about 55 s. The new client cuts off a request
  that gets no answer for 60 s, or whose body arrives slower than 32 KiB/s, and retries it.
- `--fetch-jobs 8` was not faster than 4 here (b168: 36 and 57 s; *S. aureus* 36 s). The cap is
  16 requests, to keep the load on GTDB's server modest; reading from GTDB stays opt-in.

### Species call only (`--species-only`)

GTDB-Tk's rule (`ANIScreener.sort_skani_ani_screen`, 2.8.0) takes the closest hit with
AF >= 0.5 (max of the two directions), ordered by ANI then AF. It assigns that hit's species only
if the ANI reaches that reference's own radius. No radius in R232 is below 95.0. So a reference
whose ANI is below 95 can never give the call: if it is the closest AF-passing hit, every other
hit is below 95 too, and the genome is unassigned either way. Any set of references that keeps
all hits with ANI >= 95 therefore gives the same call. Dropping references by *their own* radius
would not be safe: a closest hit at 96.5 with a 97 radius leaves the genome unassigned, and
removing it would assign the next one.

The client can't know the ANI before fetching, but the screen counts shared marker k-mers.
For a pair with ANI `a` over an aligned fraction `f` of the smaller genome, a 21-mer marker
survives with probability at least `a^21` (more if the differences cluster). So at least about
`f × a^21` of the smaller marker set is shared. `--species-only` keeps a reference if the shared
count is at least the binomial expectation at ANI 93 (2 points below the smallest radius, read
from `radii/gtdb_radii.tsv`) and AF 0.5, less 4 standard deviations. That is 10.9% of the smaller
set (356 of 4,000 markers), against skani's 0.9%. The fingerprint count can only be too high,
which errs the same way.

Checked on the five genomes above plus a sixth with no species within reach: the *B. subtilis*
assembly with 6% of its bases changed at random (`bmut`; closest reference GCF_000009045.1 at
ANI 93.45, so unassigned). Every species-only run used an empty cache:

| Genome | Sketches (full → species-only) | Downloaded | Requests | Fetch (s) | Call | GTDB-Tk's files |
|---|---|---|---|---|---|---|
| M. tuberculosis | 377 → 1 | 1,225.6 → 2.9 MB | 370 → 1 | 474 → 24 | same (GCF_000195955.2, 100.0) | `ani_summary.tsv` byte-identical |
| E. coli K-12 | 420 → 18 | 1,324.9 → 60.9 MB | 406 → 18 | 502 → 36 | same (GCA_047199095.1, 97.14) | byte-identical, all 15 other related references |
| S. aureus | 28 → 4 | 86.0 → 11.5 MB | 28 → 4 | 29–47 → 36 | same (GCF_001027105.1, 99.86) | byte-identical |
| B. subtilis 168 | 21 → 14 | 60.8 → 40.7 MB | 21 → 14 | 25–185 → 19 | same (GCF_000009045.1, 100.0) | byte-identical, all 13 other related references |
| M. jannaschii | 19 → 8 | 60.4 → 25.6 MB | 19 → 8 | 31–164 → 14 | same (GCF_000091665.1, 100.0) | byte-identical |
| bmut (unassigned) | 17 → 2 | 49.2 → 6.0 MB | 17 → 2 | 34 → 38 | same (unassigned) | unassigned hits: 2 rows instead of 14 |
| **All six** | **882 → 47** | **2,807 → 147 MB** | | | **6 of 6 the same** | |

Fetch times on the small genomes swing with the connection, as above; the drop in bytes and
requests is the reliable figure.

- **E. coli is the near-tie case.** Three references are within their radius: GCA_047199095.1
  at 97.14, *s__Escherichia coli* GCF_003697165.2 at 96.92 and GCA_048288465.1 at 95.20. All
  three were kept, and the call is GTDB-Tk's.
- **The margin is wide.** skani's ANI and AF for all 882 screened-in pairs, against the rule: no
  pair with ANI >= 95 and AF >= 0.5 was dropped, and the one closest to the threshold
  (E. coli vs GCA_048288465.1, ANI 95.20, AF 0.81) shares 1,760 markers where the rule asks for
  378, 4.7 times as many. Every dropped reference had ANI 87.1 or less, or no skani hit at all.
  These pairs are a test fixture (`gtdbprep/src/ani/testdata/species_only_hits.tsv`), and a unit
  test replays GTDB-Tk's rule on them.
- **What is missing compared with GTDB-Tk.** The unassigned-hits file of a genome the screen
  doesn't assign lists only the fetched references (bmut: 2 of 14 rows). GTDB-Tk's `classify`
  reads that file for its notes on placed genomes (closest reference, near misses), so those notes
  can be shorter; the placement and classification don't use it. The "other related references"
  column can lose references the rule drops (here, below ANI 87.1); none were lost on these
  genomes, whose related references are all at ANI 87.8 or more. Genomes in genera with many
  moderately related species may lose some.
- **It is not GTDB-Tk-identical output**, so it stays opt-in. The default still fetches every
  reference skani's screen keeps.

## 8. Licences and credit

- **GTDB data** (the skani database, radii, taxonomy) is CC BY-SA 4.0 (Parks et al., the Genome
  Taxonomy Database, <https://gtdb.ecogenomic.org>). The screen index and the seek index are
  derived from it, so they are CC BY-SA 4.0 too; the irit-data release says so and credits GTDB. The
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
