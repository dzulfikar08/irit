# Benchmarks

All runs on one machine: AMD Ryzen 5 PRO 4650U laptop (6 cores / 12 threads), 30 GB RAM, NVMe SSD,
Arch Linux, 2026-11-10. Test data: the four 1M-read-pair subsets of ERR064912 and ERR044595
(75 bp Illumina, HiSeq) that nf-core/bacass uses for its test profile.

## leanqc against FastQC

FastQC 0.12.1 ran from the BioContainers image that nf-core uses
(`quay.io/biocontainers/fastqc:0.12.1--hdfd78af_0`). Peak memory for FastQC is the container's
cgroup `memory.peak`; for leanqc it is the process's maximum resident set size. FastQC wall time
includes about 0.5 s of container start.

| | FastQC | leanqc |
|---|---|---|
| 1 file: wall time | 7.0 s | 2.5 s |
| 1 file: peak memory | 483 MB | 16 MB |
| 4 files, 4 threads: wall time | 10.0 s | 3.2 s |
| 4 files, 4 threads: peak memory | 1,350 MB | 52 MB |
| Install size | 941 MB image (or a Java runtime + FastQC) | 1.6 MB binary |

Inside the bacass pipeline itself, FastQC used 2.3–3.1 GB per file (Nextflow trace, with
pipeline defaults).

### Verdicts

| File | Module | FastQC | leanqc |
|---|---|---|---|
| ERR044595_1M_1 | Per-position base content | fail | fail |
| | Overrepresented sequences | warn | warn |
| ERR044595_1M_2 | Per-position base content | fail | fail |
| ERR064912_1M_1 | Per-tile quality | fail | fail |
| | Per-position base content | warn | warn |
| | N content | warn | warn |
| ERR064912_1M_2 | Per-tile quality | fail | fail |
| | Per-position base content | warn | warn |
| all four | every other module | pass | pass |

The same comparison runs on GitHub-hosted runners in the [validation workflow](https://github.com/dzulfikar08/irit/actions/workflows/validate.yml)
(weekly and on every change to a tool). First run, 2026-11-10, ubuntu-latest (4 vCPU, 16 GB):
40 module verdicts compared across the four files, 0 differ. 4 files with 4 threads: leanqc 2.95 s
wall and 52 MB peak; FastQC 1,200 MB peak (container cgroup).

Total sequences and %GC agree. Total deduplicated percentage differs by up to 3 points
(FastQC 90.2–94.9%, leanqc 93.5–94.9%) because the two estimate it differently; both pass.

The first version of leanqc failed the per-read GC check on all four files; the cause (integer GC
binning) and the fix are described in [leanqc](tools/leanqc.md#checks).

## nf-core/bacass baseline

`nextflow run nf-core/bacass -profile test,docker` (bacass 2.7.0, Nextflow 26.10.5), processes
capped at 8 CPUs and 12 GB. Two isolates, short-read assembly and annotation.

| Step | Time | Peak RAM |
|---|---|---|
| fastp | 15–28 s | 1.2 GB |
| FastQC (raw and trimmed) | 9–14 s each | 2.3–3.1 GB |
| Unicycler | 11–14 min | 2.0–3.1 GB |
| Prokka | 3.3 min | 0.27 GB |
| BUSCO | 35–45 s | 0.1–0.24 GB |
| QUAST | 7 s | 0.15 GB |
| MultiQC | 1.8 min | 0.18 GB |

About 32 minutes in total. Peak RAM 3.1 GB: an 8 GB laptop can run it. Disk is the real cost:
about 16 GB of container images (Prokka 5.2 GB, BUSCO 4.7 GB, QUAST 3.6 GB, Unicycler 1.6 GB,
FastQC 0.9 GB) for 78 MB of results.

## sheetgen

Validated by running nf-core/bacass 2.7.0 on a generated sheet (`-profile test,docker -stub-run`):
31 of 31 steps completed, including the pipeline merging two lanes of one sample. Details in
[sheetgen](tools/sheetgen.md#tested).

## dblock

Real downloads from GTDB, Amazon S3 (Kraken 2 indexes), SILVA and CARD. See
[dblock](tools/dblock.md#tested) for each test. Speeds were limited by the network (0.4–5 MB/s
from Indonesia), not by dblock.
