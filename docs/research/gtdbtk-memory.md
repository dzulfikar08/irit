# GTDB-Tk and memory

GTDB-Tk classifies bacterial and archaeal genomes against the Genome Taxonomy Database. It is the
standard for metagenome-assembled genomes, and it no longer runs on ordinary computers. This page
records why, what already helps, and what might.

## The numbers

From GTDB-Tk's own documentation (checked against the source, commit `155d989`, October 2026):

| Version / mode | RAM |
|---|---|
| Full reference tree, GTDB R207 (2022) | ~320 GB |
| v2 class-level split ("divide and conquer"), R207 | ~55 GB |
| v2.7+, R232, bacteria | ~140 GB |
| v2.7+, R232, archaea | ~100 GB |
| v2.7+, R232, `--full_tree` | ~950 GB |

The reference data is a 61 GB download, about 100 GB unpacked
([`dblock`](../tools/dblock.md) refuses it on a disk that's too small, before downloading).

## Where the memory goes

- **pplacer**, the step that places each genome on the reference tree, holds most of it: GTDB-Tk's
  FAQ describes the pplacer process at about 150 GB. Likelihood-based placement keeps intermediate
  results for every branch of the tree in memory, so memory grows with the tree. As GTDB grows,
  the split approach that once brought it to 55 GB has climbed back to 140 GB.
- **skani**, which compares genomes to species representatives by ANI, is not the bottleneck, and is
  already written in Rust. GTDB-Tk 2.7 pre-sketches its database (198 GB down to 98 GB on disk).
- Since 2.7, every genome is compared by ANI with all species representatives first; genomes that
  match a species are classified without tree placement unless `--place_species` is set. Genomes
  from novel lineages, common in environmental samples, still need placement.

Rewriting GTDB-Tk in a faster language would not change this: the memory is data the algorithm
keeps, not interpreter overhead.

## What already exists

| Approach | Status | Effect |
|---|---|---|
| GTDB-Tk `--scratch_dir` (+ `--pplacer_cpus 1`) | In GTDB-Tk | pplacer writes its tables to a memory-mapped file instead of RAM. Slower; how low it goes has not been measured here. |
| **gPlace** | New (November 2026), "work in progress" | Drop-in pplacer replacement giving identical placements; under 44 GiB for the largest R226 class tree that needs ~104 GB with pplacer, with a `--max_memory` cap. Being packaged for Bioconda by GTDB-Tk's maintainer. Authors advise checking results with `gplace compare`. |
| SCAMPP / BSCAMPP | Published (TCBB 2022, 2025) | Place each query into a small subtree chosen around its nearest references, then run pplacer or EPA-ng on that subtree. Scales to 200,000-leaf trees with low memory; BSCAMPP limits subtrees to ~2,000 leaves. Not used by GTDB-Tk. |
| EPA-ng memory saver | Published (IEEE 2021) | Recomputes intermediate results on demand instead of storing them all: less memory, more time. GTDB-Tk's v2 paper found EPA-ng needed more memory than pplacer in their setting. |
| Distance-based placement (APPLES-2) | Published | Low memory, but GTDB-Tk's authors found it less accurate. |

## Ideas not tried yet

Compression is an open question rather than a known result:

- **Lower-precision storage.** Placement tables are kept in 64-bit floats. Storing them as 32-bit (or
  scaled 16-bit) values would halve or quarter memory; the risk is numerical underflow on large trees
  (pplacer already shows numerical problems there), so it needs testing against exact results.
- **Compressed, memory-mapped tables.** Tables written to SSD in compressed blocks and decompressed
  only for the part of the tree being searched. Fast codecs decompress at several GB/s, faster than
  SSDs read, so this may cost little time.
- **Combining the above with subtree selection** (SCAMPP-style), so only a small part of the tree is
  ever touched per genome.

Any of these would have to reproduce GTDB-Tk's classifications on a large test set (GTDB's own
genomes give one) before it means anything.

## Using hosted data instead of downloading it

Two separate problems: disk and download (61 GB archive, ~100 GB unpacked) and RAM (~140 GB).
Hosting the data elsewhere helps only the first.

- **Reading from the publisher's storage doesn't work as shipped.** GTDB is one `.tar.gz`; gzip
  can't be read from the middle, so a run can't fetch just the part it needs, and GTDB-Tk expects
  every file on local disk.
- **Unpacked, individually addressable files would help disk.** If GTDB's files were hosted unpacked
  (for example on Cloudflare R2: about $1.50/month for 100 GB, no egress fees; GTDB's CC BY-SA
  licence allows redistribution with attribution), a tool could fetch only the backbone tree and the
  class-level parts a run touches. RAM stays the same: placement still loads its tree into memory.
- **Splitting the work helps both.** GTDB-Tk's light steps (finding and aligning the 120 marker
  genes; sketching the genome for the ANI species check) can run on a laptop and produce a few kB to
  MB per genome. The heavy step (placement on the tree held in RAM) can run once on a shared server
  that keeps the tree loaded for many users, needing ~44 GB with gPlace. Genomes stay on the
  laptop; only marker alignments and sketches are sent. Someone must run that server (on-demand
  cloud machines or a host institution); serverless platforms such as Cloudflare Workers and
  Containers have memory limits far below this.

Hosted options that exist today (2026-11-10):

- [KBase GTDB-Tk app](https://kbase.us/applist/apps/kb_gtdbtk/run_kb_gtdbtk_classify_wf/release):
  free, browser-based, GTDB-Tk 2.3.2 (older than the current 2.7/2.8).
- [Galaxy Europe](https://usegalaxy.eu/root?tool_id=gtdbtk_classify_wf): GTDB-Tk 2.7.2 for R232, but
  users [report](https://help.galaxyproject.org/t/gtdb-tk-database-galaxy-version-2-6-0-galaxy0/18605)
  its database isn't loaded yet; older tool versions work.

## Plan

1. Measure first: GTDB-Tk with and without `--scratch_dir`, and with gPlace and `--max_memory`, on a
   rented large-memory machine. Record peak memory, time and agreement. This answers "can a 32 GB
   desktop run it today?" without writing new code.
2. If gPlace with a memory cap reaches desktop sizes, document the recipe and add it to `dblock`.
3. Otherwise, prototype lower-precision or compressed tables, ideally as a contribution to gPlace
   rather than a competing tool.

## Sources

- [GTDB-Tk documentation](https://ecogenomics.github.io/GTDBTk/) and [FAQ](https://ecogenomics.github.io/GTDBTk/faq.html)
- [GTDB-Tk v2 paper (Bioinformatics 2022)](https://academic.oup.com/bioinformatics/article/38/23/5315/6758240)
- [gPlace documentation](https://cmc-aau.github.io/gPlace/), [gtdb-gplace on PyPI](https://pypi.org/project/gtdb-gplace/0.1.0/), [Bioconda PR #71361](https://github.com/bioconda/bioconda-recipes/pull/71361)
- [SCAMPP (TCBB 2022)](https://doi.org/10.1109/tcbb.2022.3170386), [BSCAMPP (TCBB 2025)](https://doi.org/10.1109/tcbbio.2025.3562281)
- [Efficient Memory Management in Likelihood-based Phylogenetic Placement (IEEE 2021)](https://ieeexplore.ieee.org/document/9460701/)
- [EPA-ng](https://github.com/pierrebarbera/epa-ng)
