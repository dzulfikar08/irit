# GTDB R232 placement in 8.5 GB: a test with gPlace

Can GTDB-Tk's tree placement for GTDB R232 run in about 8.5 GB of RAM, so that a 16 GB laptop could
do it? This page tests that on a free GitHub-hosted runner (4 vCPU, 15 GiB RAM), using gPlace in
place of pplacer. It follows up [Split GTDB-Tk: a design](split-gtdb.md) and
[GTDB-Tk and memory](gtdbtk-memory.md). Run on 2026-11-10.

## Summary

- **Yes, placement fits.** GTDB-Tk 2.8.0 classified 4 bacteria and 1 archaeon on the real R232
  backbone, class-level and archaeal trees, with gPlace 0.1.1 as the placer. Every gPlace call ran
  under `--max_memory 8.5G` *and* a kernel-enforced 8.5 GiB cgroup cap with swap off. There were no
  OOM kills. The highest peak was **7.44 GiB** (class-level tree 8). The whole `classify` step took
  **6.3 minutes** on 4 vCPU.
- **The full 379,594-node bacterial tree fits too**: 4 genomes in **12.1 minutes**, peak 6.2 GiB,
  in 77 tiles. Untiled, gPlace estimates 301 GiB for it; pplacer needs ~950 GB (GTDB-Tk docs).
- **Placement needs 7.4 GB on disk, not 100 GB.** The package unpacks to 100.48 GB. Of that,
  85.17 GB is the skani ANI database and 7.94 GB is the reference MSAs, and placement uses neither.
- **What is still missing on a laptop is the ANI species screen.** Its database is 85.2 GB. Without
  it, GTDB-Tk classifies down to genus, and the species field stays empty (`s__`). The plan's
  "range-read the skani database" idea is now the main open problem.
- **Download stays expensive.** The archive is one 60.8 GB gzip stream. Getting the 7.4 GB that
  placement needs still means streaming all of it (59–65 min on the runner).

## The gPlace claim, verified

From the gPlace change log, version 0.1.1
([changelog](https://cmc-aau.github.io/gPlace/changelog.html), read 2026-11-10):

> Tiled placement starts its worker processes once per pass over the tiles (the prefilter, each
> baseball round, the refinement) instead of once per tile, and rebuilds the tiles of a pass with
> one thread pool: 10,000 genomes on the GTDB R232 bacterial tree in 370 tiles (--max_memory 8.5G,
> 64 CPUs) take 25 minutes instead of 2 h 49 min, with identical results.

Two caveats from the same docs:

- 0.1.1 is not released. PyPI (`gtdb-gplace`) and Bioconda have only 0.1.0. The
  [announcements](https://cmc-aau.github.io/gPlace/announcements.html) page lists 0.1.1 as "In
  progress". This test installed it from source at
  [`cmc-aau/gPlace@695a401`](https://github.com/cmc-aau/gPlace/commit/695a401e0b54cc1bde55a5e1498a0697899b15ea)
  (`__version__ = '0.1.1'`).
- gPlace calls itself "Work in progress": compare with pplacer before you publish results.
  `gplace check_install` passed on the runner ("the engines reproduce pplacer v1.1.alpha19's
  arithmetic bit for bit", AVX2 engine).

## What is in the R232 package

[Run 42779725442](https://github.com/dzulfikar08/irit/actions/runs/42779725442) streamed the
archive through `tar -tv` (nothing extracted) and hashed it at the same time:

- 60,806,405,195 bytes; MD5 `25a59e0352b1fd150c589f56559767d4`, matching `MD5SUM.txt`.
- 263 files, 100,482,230,310 bytes unpacked (100.48 GB, 93.58 GiB), under one top folder
  `release232/`.
- 59 min to stream and list.

The full listing is the run's `gtdbtk-r232-listing` artifact.

| Folder | Size | Files | Needed for |
|---|---:|---:|---|
| `skani/` (`database/sketches.db` 75.27 GB, `markers.bin` 9.86 GB, `index.db` 40 MB) | 85.17 GB | 5 | ANI species screen |
| `msa/` (`gtdb_r232_bac120.faa` 7.80 GB, `ar53` 0.14 GB) | 7.94 GB | 2 | only `align` without `--skip_gtdb_refs`, and de novo trees |
| `split/` | 5.68 GB | 55 | bacterial placement (default split mode) |
| ├ `split/class_level/pplacer/` (8 refpkgs, 0.60–0.66 GB each) | 5.11 GB | 40 | |
| ├ `split/backbone/pplacer/gtdbtk_package_backbone.refpkg` | 0.55 GB | 5 | |
| └ `split/*/red/`, `tree_mapping.tsv` | 15 MB | 10 | |
| `pplacer/` | 1.53 GB | 10 | archaeal placement, `--full_tree` |
| ├ `gtdb_r232_bac120.refpkg` (full tree) | 1.48 GB | 5 | `--full_tree` only |
| └ `gtdb_r232_ar53.refpkg` | 48 MB | 5 | archaea |
| `markers/` | 80 MB | 182 | `identify` |
| `taxonomy/` | 56 MB | 3 | classify |
| `mrca_red/` | 15 MB | 2 | classify |
| `radii/` | 11 MB | 1 | classify, ANI |
| `masks/`, `metadata/` | 55 KB | 3 | `align`, version |

Notes:

- **Placement data comes to 7.37 GB in total.** Bacteria in split mode plus archaea, without the
  full-tree refpkg, come to about 5.9 GB.
- **The ten refpkgs each carry a 517,985,300-byte `gtdb_r232_bac120_fasttree.log`.** That is 5.18
  GB of the 7.37 GB. The files are the same size; whether they are byte-identical was not checked.
  If they are, hard-linking them would bring placement data down to about 2.7 GB.
- **The skani database is bigger than the design page assumed.** `markers.bin` alone is 9.86 GB,
  against the 5–7 GB estimate in [split-gtdb.md](split-gtdb.md). `sketches.db` is 75.3 GB, about
  376 KB per representative.
- **R232 has 8 class-level trees.** The design page listed their number and sizes as unknown.

## The placement run

[Run 42787578728](https://github.com/dzulfikar08/irit/actions/runs/42787578728), workflow
[`gtdb-placement.yml`](https://github.com/dzulfikar08/irit/blob/exp/gtdb-placement/.github/workflows/gtdb-placement.yml)
on branch [`exp/gtdb-placement`](https://github.com/dzulfikar08/irit/tree/exp/gtdb-placement), with
helpers in [`exp/gtdb/`](https://github.com/dzulfikar08/irit/tree/exp/gtdb-placement/exp/gtdb):

1. **Free disk space** with `jlumbroso/free-disk-space` (everything on). That left 128.6 GB free on
   `/`. Swap was removed, so memory numbers aren't hidden by swapping.
2. **Stream and extract.** `curl | pigz -dc | tar -x --exclude=*/skani/* --exclude=*/msa/*`: 65
   min, 7.4 GB on disk.
3. **Install** GTDB-Tk 2.8.0 (Bioconda via micromamba, Python 3.12; pplacer 1.1.alpha19, skani
   0.4.0), then gPlace 0.1.1 from source with `pip`. The engine compiled in about a minute.
4. **Genomes** (all of them placed, because the ANI screen is skipped):

   | Genome | Why |
   |---|---|
   | `G021783475` | GTDB's own test genome (`mockup_db/test_genome_G021783475.fna.gz`) |
   | E. coli K-12 MG1655, GCF_000005845.2 | well-known control |
   | *Nanosynbacter lyticus* TM7x, GCA_000803625.1 | CPR / Patescibacteria, reduced genome (0.7 Mb) |
   | Omnitrophota bacterium, GCA_988179205.1 | released 2026-08-21, after R232's NCBI snapshot (taxdump 2025-09-07), so not in R232 |
   | *Promethearchaeum syntrophicum*, GCA_008000775.2 | Asgard archaeon (ar53 tree) |

5. **`identify` and `align --skip_gtdb_refs`**, as on the design page.
6. **`classify` with gPlace in place of pplacer.**
   - A `pplacer` script sits first on `PATH`. It turns GTDB-Tk's
     `pplacer -m WAG -j 4 -c REFPKG -o OUT.json MSA` into
     `gplace place --max_memory 8.5G <same args>`. `pplacer --version` and `guppy tog` still go to
     the real pplacer package.
   - GTDB-Tk 2.8's `classify` always runs the skani screen. No command-line option skips it, so a
     small launcher sets `options.skip_ani_screen = True` and then calls GTDB-Tk's own `main()`.
     That is the same flag `classify_wf` sets after its own ANI step.
7. **Measurement.**
   - Each command runs in its own cgroup v2 group, under `/usr/bin/time -v`.
   - The table reports `memory.peak` and the sampled peak of `anon` memory, plus max RSS.
   - Each gPlace call ran in a group with `memory.max` = 8.5 GiB (9,126,805,504 bytes) and
     `memory.swap.max` = 0. Going over the cap would therefore be an OOM kill, not a slowdown.
8. **Direct gPlace runs** placed the 4 bacterial alignments on the largest class-level tree
   (package 2) and on the full bacterial tree.

### Results

| Run | Tree (nodes, sites) | Genomes | Untiled need (gPlace estimate) | Tiles | Wall | Peak memory, cgroup | Max RSS, one process | OOM |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| `identify` | – | 5 | – | – | 28 s | 0.35 GiB | 0.13 GiB | 0 |
| `align` | – | 5 | – | – | 2 s | 0.29 GiB | 0.21 GiB | 0 |
| gPlace: ar53 | 20,242, 4,007 | 1 | 13.4 GiB | 3 | 40 s | 5.53 GiB | 5.41 GiB | 0 |
| gPlace: backbone | 12,094, 5,010 | 4 | 10.1 GiB | 2 | 33 s | 5.56 GiB | 5.43 GiB | 0 |
| gPlace: class tree 6 | 53,486, 4,779 | 2 | 41.0 GiB | 7 | 112 s | 6.98 GiB | 6.81 GiB | 0 |
| gPlace: class tree 8 | 32,446, 4,225 | 1 | 22.3 GiB | 4 | 61 s | **7.44 GiB** | 7.28 GiB | 0 |
| gPlace: class tree 1 | 50,933, 4,928 | 1 | 40.3 GiB | 7 | 104 s | 6.77 GiB | 6.59 GiB | 0 |
| `classify`, whole step (GTDB-Tk itself, without the gPlace groups) | – | 5 | – | – | 376 s | 0.68 GiB | – | 0 |
| direct: class tree 2 (largest) | 55,376, 5,010 | 4 | 44.4 GiB | 7 | 115 s | 7.40 GiB | 7.21 GiB | 0 |
| direct: full bacterial tree | 379,594, 5,010 | 4 | 301.4 GiB | 77 | 724 s | 6.20 GiB | 5.92 GiB | 0 |

How to read the memory columns:

- **"Peak memory, cgroup"** is `memory.peak` of the group: all of gPlace's processes, including
  shared memory and page cache. Use it as the RAM figure.
- **Anonymous memory stays small** (0.2–0.7 GiB) for the class-level and ar53 calls. gPlace keeps
  its likelihood vectors in memory shared with its forked workers, which the kernel counts as shmem,
  not anon. The full-tree run is the exception at 4.6 GiB of anon.
- **The GTDB-Tk Python process (0.68 GiB) waits while gPlace runs.** The two together stayed under
  about 8.2 GiB at any moment.
- **gPlace's own estimates were close but low.** It estimated a peak of 6.1–8.1 GiB per call; the
  measured peaks were 5.5–7.4 GiB. It always chose tiles. Even the backbone and ar53 trees need
  10–13 GiB untiled, so a 16 GB laptop would tile them too, unless it allowed
  `--max_memory 14G`.
- **Times come from 4 vCPU.** Most of each tiled run is the prefilter, tile by tile. The design
  page's 12-core laptop should be faster (not measured).

Raw numbers: the `gtdb-placement-results` artifact of the run (`measure/`, jplace files, GTDB-Tk
outputs).

### Classifications

| Genome | GTDB-Tk 2.8.0 + gPlace (no ANI screen) | Class tree | MSA % | RED |
|---|---|---:|---:|---:|
| E. coli K-12 MG1655 | `d__Bacteria;p__Pseudomonadota;c__Gammaproteobacteria;o__Enterobacterales;f__Enterobacteriaceae;g__Escherichia;s__` | 1 | 98.36 | 0.99819 |
| G021783475 (GTDB test genome) | `d__Bacteria;p__Thermodesulfobiota;c__Thermodesulfobiia;o__Thermodesulfobiales;f__Thermodesulfobiaceae;g__Thermodesulfobium;s__` | 6 | 91.42 | 0.99999 |
| *N. lyticus* TM7x (CPR) | `d__Bacteria;p__Patescibacteriota;c__Saccharimonadia;o__Saccharimonadales;f__Saccharimonadaceae;g__Nanosynbacter;s__` | 6 | 77.62 | 1.0 |
| Omnitrophota bacterium (2026, not in R232) | `d__Bacteria;p__Omnitrophota;c__Gorgyraeia;o__Zapsychrales;f__Fredricksoniimonadaceae;g__JABDGZ01;s__` | 8 | 84.33 | 0.98167 |
| *P. syntrophicum* (Asgard) | `d__Archaea;p__Asgardarchaeota;c__Lokiarchaeia;o__Sigyniarchaeales;f__Promethearchaeaceae;g__Promethearchaeum;s__` | ar53 | 91.71 | 0.99999 |

- **All genera match the genomes' known identities**, and GTDB-Tk logged no warnings. Every row was
  decided by classification "Rule 2" (placement topology).
- **The species field is empty** because the ANI screen was skipped on purpose. With skani, the
  four genomes that have GTDB species representatives would get `s__` names. The 2026 Omnitrophota
  genome would probably still be placed as a genus-level relative.
- **Not checked here:** whether pplacer gives the same placements. That needs a ~140 GB machine.
  gPlace's own validation reports identical output for R226 (127,343 genomes); `gplace compare`
  against pplacer on these jplace files is the follow-up.

## What it means for the split-GTDB plan

**Placement runs on a 16 GB laptop.** Every R232 tree GTDB-Tk uses, and the full bacterial tree,
placed within an enforced 8.5 GiB, in 0.5–2 minutes per tree on 4 vCPU. That answers step 2 of the
[phased plan](split-gtdb.md#phased-plan). A server is not needed for placement. The server design
on that page should stay parked.

What a laptop route still needs:

1. **Disk: about 5.9–7.4 GB** of refpkgs and metadata, possibly about 2.7 GB if the repeated
   `fasttree.log` files turn out identical. That is fine on a laptop.
2. **Download.** GTDB publishes only the 60.8 GB gzip. Options:
   - host the unpacked placement folders ourselves (R2, CC BY-SA with attribution; about
     $0.11/month for 7.4 GB);
   - ask GTDB to publish a "placement" subset, the way they publish the mockup;
   - stream the archive once with `tar -x --exclude` (this page's method). That still needs the
     full download, but only 7.4 GB of disk.

   `dblock` could offer the last as a `gtdbtk-r232-placement` entry right away.

   **Done (2026-11-10):** the placement folders are on an unofficial, unmodified mirror,
   [`dzulfikar08/irit-data` release `gtdbtk-r232-placement-v1`](https://github.com/dzulfikar08/irit-data/releases/tag/gtdbtk-r232-placement-v1)
   (12 `.tar.zst` parts, 280 MB, GTDB data under CC BY-SA 4.0), and `dblock add
   gtdbtk-r232-placement` (or `gtdbtk-r232-placement-lite`, 175 MB, without the full bacterial
   tree) fetches them and checks them against GTDB-Tk's `REF_HASHES`. See
   [dblock](../tools/dblock.md#gtdbtk-r232-placement-and--lite-gtdb-placement-data-from-a-mirror).
3. **The ANI screen (85.2 GB).** This is now the only part that doesn't fit. Without it,
   classifications stop at genus for genomes of known species. That is the case where most users
   want a species name. Routes:
   - the range-read idea in [split-gtdb.md §4](split-gtdb.md#4-can-the-ani-screen-run-from-a-sketch):
     fetch `markers.bin` (9.86 GB, more than the 5–7 GB estimated), or query it remotely, then
     range-read the few `sketches.db` entries needed;
   - a small stateless ANI service. That sends skani sketches, with the privacy caveats on the
     design page.
4. **Packaging.** gPlace 0.1.1 is not on PyPI or Bioconda yet. GTDB-Tk has no option for choosing
   the placer, so the PATH shim and the `skip_ani_screen` launcher are needed for now. Both should
   go away if GTDB-Tk adopts gPlace upstream.

**Next steps:**

- Rerun with skani on a machine with ~90 GB of free disk, to get full species calls and to time
  the screen.
- Run `gplace compare` against pplacer on a large-memory machine.
- Check whether the 10 `fasttree.log` copies are byte-identical.
- Time placement on the 12-core laptop with `--max_memory 8G` and `14G`.

## Sources

- gPlace docs: [index](https://cmc-aau.github.io/gPlace/), [change log](https://cmc-aau.github.io/gPlace/changelog.html), [large trees and memory limits](https://cmc-aau.github.io/gPlace/advanced/memory.html), [FAQ](https://cmc-aau.github.io/gPlace/faq.html), [announcements](https://cmc-aau.github.io/gPlace/announcements.html); source [cmc-aau/gPlace](https://github.com/cmc-aau/gPlace) at `695a401`; PyPI [gtdb-gplace](https://pypi.org/project/gtdb-gplace/) (0.1.0)
- GTDB-Tk 2.8.0 source at `155d989`: `gtdbtk/external/pplacer.py` (how pplacer is called), `gtdbtk/main.py` and `gtdbtk/classify.py` (`skip_ani_screen`), `gtdbtk/config/common.py` (data paths)
- GTDB R232 package: [full_package](https://data.gtdb.ecogenomic.org/releases/release232/232.0/auxillary_files/gtdbtk_package/full_package/), [mockup_db](https://data.gtdb.ecogenomic.org/releases/release232/232.0/auxillary_files/gtdbtk_package/mockup_db/) (test genome)
- Runs: [sizes, 42779725442](https://github.com/dzulfikar08/irit/actions/runs/42779725442); [placement, 42787578728](https://github.com/dzulfikar08/irit/actions/runs/42787578728)
- Genomes from NCBI Datasets: GCF_000005845.2, GCA_000803625.1, GCA_988179205.1, GCA_008000775.2
