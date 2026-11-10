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
| 1 file: wall time | 7.0 s | 1.5 s |
| 1 file: peak memory | 483 MB | 13 MB |
| 4 files, 4 threads: wall time | 10.0 s | 1.8 s |
| 4 files, 4 threads: peak memory | 1,350 MB | 41 MB |
| Install size | 941 MB image (or a Java runtime + FastQC) | 1.6 MB binary |

The leanqc figures were re-measured after the FastQC-parity changes of 2026-11-10 (median of three
warm runs; the single file is ERR044595_1M_1); the FastQC figures are from the first measurement
the same day. Version 0.1 of leanqc took 2.5 s / 16 MB and 3.2 s / 52 MB.

Inside the bacass pipeline itself, FastQC used 2.3–3.1 GB per file (Nextflow trace, with
pipeline defaults).

### Verdicts

What is compared: for each of the four files, the verdict of the ten FastQC modules that grade data
(per base sequence quality, per tile sequence quality, per sequence quality scores, per base
sequence content, per sequence GC content, per base N content, sequence length distribution,
sequence duplication levels, overrepresented sequences, adapter content) against leanqc's
matching check: 40 verdicts. FastQC's Basic Statistics (no verdict) and Kmer Content (off by
default) are not compared. `scripts/compare_fastqc.py` does the comparison.

Result after the 2026-11-10 changes: **40 of 40 agree**.

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

(That runner result was for leanqc 0.1; the workflow now also runs the synthetic corpus below.)

Total sequences and %GC agree, and so does the total deduplicated percentage (90.20, 91.00, 94.91
and 93.27%) since leanqc uses FastQC's duplication estimate; version 0.1 was up to 3 points off.

The first version of leanqc failed the per-read GC check on all four files; the cause (integer GC
binning) and the fix are described in [leanqc](tools/leanqc.md#checks).

### Synthetic corpus

The bacass files are all 75 bp Illumina reads with good quality, so they can't show how leanqc
behaves on anything else. `scripts/make_fastq_corpus.py` writes fourteen small files from fixed
seeds (5 MB in all, in `leanqc/tests/corpus/`), each aimed at one behaviour:

| File | What it exercises | FastQC 0.12.1 | leanqc |
|---|---|---|---|
| `varied_lengths` | 4,000 reads of 35–250 bp (position groups, GC truncation) | length warn, base content warn | same |
| `hiq_phred33` | Phred+33 with every quality Q31–Q40 (`@`–`I`) | per-base and per-read quality **fail** (read as Phred+64) | **pass** (read as Phred+33): deliberate |
| `phred64` | genuine Phred+64 (`B`–`h`) | pass | pass, Phred+64 detected |
| `adapters` | 15% of reads run into the Illumina adapter, 3% polyG tails | adapter fail | same |
| `odd_headers` | tile headers, then names with no tile, odd colons, spaces | per-tile dropped | same |
| `long_reads` | 150 reads of 5–6 kb, Q40 for 1 kb then Q2 | per-base and per-read quality fail, 4 warns | same |
| `n_rich` | position 10: 60% N, rest 70% A / 30% T | base content fail, N fail, GC warn | same |
| `gc_bimodal` | 70% of reads at 35% GC, 30% at 65% | GC fail | same |
| `duplicates` | 40% of reads are copies of 20 sequences | duplication warn, overrepresented fail, 2 warns | same |
| `low_quality_tail` | quality falling to ~Q7 over the last 30 of 150 bases | per-base quality fail | same |
| `truncated_gzip` | gzip stream cut in half | "Failed to process file", no report, **exit 0** | error, exit 1 |
| `truncated_record` | last record has no quality line | "Failed to process file", no report, exit 0 | error, exit 1 |
| `bad_tile` | 16,000 CASAVA reads over 8 tiles; tile 1103 drops to Q12 over positions 40–75 (per-tile module and its every-10th-read sampling after 10,000) | pending | per-tile fail, rest pass |
| `ultra_long` | 120 reads of 105–110 kb, Q35 except Q2 from position 104,001 (position groups past 100 kb) | pending | per-base quality fail; base content, length and overrepresented warn |

Over the ten readable files, 90 module verdicts are compared and 88 agree; the two that differ
are the deliberate encoding rule ([leanqc](tools/leanqc.md#quality-encoding)). Writing the corpus
also surfaced two FastQC behaviours leanqc did not copy before: per-tile quality switches off at
the first header without a tile number, and it counts tiles far *above* average too; leanqc now
does both.

FastQC's verdicts are recorded in `leanqc/tests/corpus/fastqc_verdicts.tsv` by
`scripts/fastqc_corpus.sh` (Docker), and `cargo test` checks leanqc against them, allowing only the
differences listed with reasons in `expected_differences.tsv`. The validation workflow re-records
FastQC's verdicts for every corpus file on every run and fails if any recorded verdict changed.

"Pending" files were added on a machine without Docker: their FastQC verdicts haven't been
recorded yet. `pending_verdicts.tsv` holds leanqc's own verdicts for them, which `cargo test` holds
leanqc to. The workflow records FastQC's verdicts for them on GitHub's runner, lists them in the run
summary, and runs the corpus test against them, so a disagreement fails the run; the re-recorded
`fastqc_verdicts.tsv` is in the run's `qc-reports` artifact, to be committed (and the pending rows
removed).

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
