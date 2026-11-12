# leanqc: read quality control

`leanqc` reads each FASTQ (or FASTA) file once and runs the ten FastQC modules that grade data
(listed under [Checks](#checks)), using a fraction of the time and memory. It writes a
self-contained HTML report per file (inline SVG, no scripts, opens offline), the full numbers as
JSON, a summary table, and a table MultiQC picks up. What it does not do is listed under
[Known gaps](#known-gaps).

## Usage

```sh
leanqc reads/*.fastq.gz -o qc/ -t 4
```

| Option | Effect |
|---|---|
| `-o DIR` | Output folder (default `leanqc/`). |
| `-t N` | Files processed at once; each file uses one thread (default: up to 4). |
| `--phred 33\|64` | Quality encoding, instead of detecting it (see [Quality encoding](#quality-encoding)). |
| `--dup-distinct N` | Distinct sequences tracked for duplication and overrepresented sequences (default 100,000, as FastQC). |
| `--no-html` | JSON and TSV only. |
| `--long` | Long-read views for every file (see [Long reads](#long-reads-ont-pacbio)); without it, only for files whose median read is over 1,000 bp. |

Input can be plain, gzip, bzip2, xz or zstd. FASTA works; the quality modules are skipped. A
truncated file (a gzip stream that stops early, or a last record without its quality line) is an
error, and leanqc exits non-zero.

Outputs in the folder:

- `<name>_leanqc.html`: the report.
- `<name>_leanqc.json`: every number behind it.
- `leanqc_summary.tsv`: one row per file, with failed and warned checks listed.
- `leanqc_mqc.tsv`: MultiQC general-statistics table (reads, GC, Q30, duplication, adapters).

## Checks

Thresholds are FastQC 0.12.1's defaults (its `Configuration/limits.txt`), so a verdict means the
same thing in both tools.

| Check (FastQC module) | Warn | Fail |
|---|---|---|
| Per-position quality (Per base sequence quality) | median < Q25 or lower quartile < Q10 in a position group | median < Q20 or lower quartile < Q5 |
| Per-tile quality (Per tile sequence quality) | a tile's mean differs from the all-tile mean by > 5 in a position group | > 10 |
| Per-read quality (Per sequence quality scores) | most common mean read quality ≤ Q27 | ≤ Q20 |
| Per-position base content (Per base sequence content) | A/T or G/C differ by > 10 points | > 20 |
| Per-read GC (Per sequence GC content) | > 15% of reads off the fitted normal curve | > 30% |
| N content (Per base N content) | > 5% N in a position group | > 20% |
| Read length (Sequence Length Distribution) | lengths differ | any empty read |
| Duplication (Sequence Duplication Levels) | < 70% of reads would remain after deduplication | < 50% |
| Overrepresented sequences | any sequence > 0.1% of reads | > 1% |
| Adapter content | any adapter > 5% of reads in a position group | > 10% |

Adapters searched (FastQC's default list): Illumina Universal, Illumina Small RNA 3', Illumina
Small RNA 5', Nextera, polyA, polyG.

How the modules work (ported from FastQC 0.12.1: several functions in `leanqc/src/stats.rs` are
translations of FastQC's Java code, each marked with the FastQC class and method it ports; see
[credit](#credit)):

- **Position groups.** Per-position modules are judged on FastQC's linear groups: single positions
  up to 9, then equal-width groups chosen so there are fewer than 75 (reads up to 75 bp are not
  grouped). Quality percentiles are FastQC's: each position's percentile, averaged over the group,
  using only positions covered by more than 100 reads.
- **Long reads.** Positions 1–999 are kept one by one; beyond that leanqc keeps 10 bp slots up to
  100 kb and 1,000 bp slots after, so memory stays small. FastQC's group boundaries for reads that
  long fall on multiples of 20 (and of 2,000 above 100 kb), so slots line up with groups and the
  verdicts cover the whole read; a 5 kb read set with Q2 after the first 1,000 bases fails, as it
  should. Two approximations beyond 999 bp: a slot's quality percentile comes from its pooled
  counts rather than the mean of each position's percentile, and an adapter starting inside a slot
  is counted from the slot's start.
- **Base content and N.** A, C, G and T are percentages of A+C+G+T at those positions (Ns and other
  codes left out); N content is N over all bases, as in FastQC.
- **Per-read GC.** FastQC's GC model: each read's GC is counted over its first 100 × ⌊length/100⌋
  bases (1,000 × ⌊length/1,000⌋ above 1 kb), Ns included in the length, and spread over the
  percentage bins its value can round to. The normal curve is centred on the mode (the average of
  the bins within 10% of the peak), with the spread measured around that mode. (Version 0.1 put
  each read in one bin, which failed every file FastQC passed, then fitted the curve around the
  mean.)
- **Duplication and overrepresented sequences.** As FastQC: the first 100,000 distinct sequences
  (first 50 bases of longer reads) are tracked and counted through the whole file; the remaining
  percentage is extrapolated from those counts with FastQC's correction for sequences first seen
  after tracking stopped, and overrepresented sequences are percentages of all reads. On the four
  bacass test files this gives FastQC's figures to two decimals. (Version 0.1 counted the first
  100,000 reads only and differed by up to 3 points.)
- **Per-tile quality.** The tile number comes from the read name, using FastQC's rule: split the
  header on `:`; with 7 or more fields (CASAVA 1.8+) the tile is the 5th, with 5–6 fields (older
  Illumina) the 3rd. The first read decides the field; a later read without a number there turns
  the module off, as in FastQC. Like FastQC it uses all of the first 10,000 reads and every 10th
  after, the first 1,000 positions, and the largest difference from the all-tile mean in either
  direction. More than 2,500 tiles also turns it off.
- **Per-read quality.** The most common mean read quality; ties go to the lower score.

## Long reads (ONT, PacBio)

When a file's median read is longer than 1,000 bp (or with `--long`), the report gets a **Long
reads** section and the JSON a `long_reads` object; the terminal line gets a second line with the
same summary. Files with shorter reads are reported exactly as before: their JSON has no
`long_reads` field.

- **Lengths**: total bases, N50 and N90, mean and median length, longest read, and reads and bases
  per length bin on a log scale (10 bins per factor of ten: 1,000–1,258 bp, 1,259–1,584 bp, …).
  N50 is the length L such that reads of length L or more hold at least half of all bases (N90:
  90%). These come from the exact length histogram the FastQC modules keep anyway, so they are
  exact.
- **Read quality**: each read's quality is the Phred score of the mean of its bases' error
  probabilities, −10·log10(mean(10^(−Q/10))), which is how basecallers report a read's mean
  quality; the mean of the scores (what FastQC's per-read module uses) is higher, by about 7 points
  for a read of half Q10 and half Q30 bases. Reported: the distribution (by whole Phred score),
  the mean and median (median to 0.1, rounded down), reads at or above Q10 and Q20 with their share
  of reads and of bases, and the mean read quality per length bin (quality against length).
  Qualities are read as Phred+33; with Phred+64 qualities the quality views are left out with a
  warning.

The ten FastQC checks are unchanged for long reads, so their verdicts still match FastQC's (the
corpus file of 5–6 kb reads is compared with FastQC as before). Read them with this in mind:

- **Read length** warns whenever lengths differ, which they always do for long reads.
- **Per-read quality** uses the mean of the scores and FastQC's Q20/Q27 limits; the read-quality
  view above is the one to use for nanopore data.
- **Per-position modules** use FastQC's position groups (fewer than 75, about 2,000 bp wide for
  reads up to 130 kb); groups near the longest reads are covered by few reads, and quality
  percentiles are only computed where more than 100 reads cover a position, as in FastQC.
- **Per-tile quality** needs Illumina read names with a tile field; nanopore (`@<uuid> runid=…`)
  and PacBio (`@m64011_…/1/ccs`) names have none, so it is skipped with that reason, as FastQC
  drops the module.
- **Duplication and overrepresented sequences** look at the first 50 bases of each read, as FastQC
  does; on nanopore reads that are not trimmed, adapter and barcode sequence at the read start
  shows up here.

JSON fields under `long_reads`:

| Field | Meaning |
|---|---|
| `detected` | `"auto"` (median read above 1,000 bp) or `"--long"` |
| `reads`, `bases` | reads (empty ones included) and bases |
| `mean_length`, `median_length` | bp; the median of an even count is the mean of the two middle reads |
| `n50`, `n90` | bp; `null` when there are no bases |
| `longest` | bp |
| `length_bins[]` | `min`, `max` (bp, inclusive), `reads`, `bases`, `mean_read_quality` (`null` without qualities) |
| `read_quality` | `null` for FASTA or Phred+64; else the fields below |
| `read_quality.reads` | reads with qualities (empty reads have none) |
| `read_quality.mean`, `.median` | Phred, of the per-read qualities |
| `read_quality.distribution` | `[Q, reads]` pairs: reads with quality from Q up to Q+1 |
| `read_quality.q10_reads`, `.q10_percent`, `.q10_bases_percent` | reads with quality ≥ Q10, as a count, a percent of reads, and their bases as a percent of all bases |
| `read_quality.q20_reads`, `.q20_percent`, `.q20_bases_percent` | the same for Q20 |

Memory doesn't grow with the file: the long-read tables have a fixed size (90 length bins, read
quality in 0.1 steps), and the length histogram has one entry per distinct read length.

Measured on this project's laptop on 2026-11-12 (shared with another long job; leanqc 0.3.2 from
main against this version, alternating runs, `--no-html`):

| File | main | this version | Peak RSS (both) |
|---|---|---|---|
| Synthetic, 1.5 GB FASTQ, 90,010 reads, log-normal lengths (median 5 kb, up to 300 kb) | 9.3 s (median of 10) | 7.1 s (median of 10) | 22–23 MB |
| Same generator, first 0.3 GB | | 1.4 s | 15 MB |
| ENA DRR1060986 (MinION, *Salmonella* Dublin), 148 MB gzipped, 13,144 reads | 3.2 s (3 runs) | 2.6 s (3 runs) | 14 MB |
| Synthetic short reads, 1 GB, 150 bp | 5.3 s (4 runs) | 3.9 s (4 runs) | 13 MB |

This version also adds work (an error-probability sum per base), and the faster times were not
traced to a cause (code generation is the likely one); take them as "not slower". The JSON for the
short-read file is identical to main's, and for the long-read files identical apart from the new
`long_reads` object. The synthetic file comes from `scripts/make_long_fastq.py`.

Checked against an independent Python calculation (reading the FASTQ and computing every figure
from the reads):

| | DRR1060986: Python | leanqc | Synthetic: Python | leanqc |
|---|---|---|---|---|
| Reads / bases | 13,144 / 157,701,283 | same | 90,010 / 749,352,750 | same |
| Mean / median length | 11,997.97 / 7,588.5 | same | 8,325.22 / 5,018 | same |
| Longest | 127,760 | same | 300,000 | same |
| N50 / N90 | 20,227 / 5,363 | same | 13,751 / 3,799 | same |
| Mean / median read quality | Q19.874 / Q20.329 | Q19.87 / Q20.3 | Q13.262 / Q12.886 | Q13.26 / Q12.8 |
| Reads ≥ Q10 / ≥ Q20 | 13,144 / 7,208 | same | 58,630 / 13,348 | same |

### Quality encoding

leanqc reads qualities as Phred+64 only when the lowest character is `@` (64) or above **and** the
highest is above `J` (74, the highest Phred+33 value Illumina 1.8+ writes) but no higher than `i`
(105, Q41 in Phred+64, the top Illumina 1.3–1.7 wrote). FastQC uses only the first condition, so
two kinds of Phred+33 file are misread by it as Phred+64:

- excellent short reads (every base Q31–Q40, characters `@` to `I`): FastQC reads Q0–Q9 and fails
  its quality modules;
- high-quality long reads such as PacBio HiFi, with qualities spread over `@` to `~` (Q31–Q93):
  read as Phred+64, about half the bases drop below Q30.

leanqc reads both as Phred+33 and adds a note to the report saying why; `--phred 64` overrides it.
This is the one deliberate difference from FastQC's verdicts.

## Results

See [benchmarks](../benchmarks.md#leanqc-against-fastqc). In short: on a 1-million-read bacass
test file, 1.5 s and 13 MB against FastQC's 7.0 s and 483 MB. The ten graded modules give the
same verdict as FastQC 0.12.1 on all four bacass test files (40 of 40) and on a synthetic corpus
of ten more files (88 of 90; the two differences are the encoding rule above), and both tools
refuse the two truncated files in the corpus. Two newer corpus files (a bad flowcell tile, and
reads over 100 kb) are checked against leanqc's own recorded verdicts until the validation
workflow records FastQC's.

## leanqc and FastQC-Rust

[FastQC-Rust](https://github.com/ewels/FastQC-Rust) is Phil Ewels' faithful Rust rewrite of FastQC:
its `fastqc_data.txt` and `summary.txt` are byte-identical to FastQC's, it has every module, and it
exists to prototype improvements for FastQC itself. If you need FastQC's exact output, use it (or
FastQC). leanqc is irit's own QC: the same verdicts on the files compared, a smaller footprint, and
it refuses truncated input that FastQC and FastQC-Rust accept.

Measured on this project's laptop on 2026-11-10 (four 1M-read bacass test files, two runs each,
FastQC-Rust 1.1.1; leanqc before its switch to the zlib-rs decoder):

| | FastQC-Rust 1.1.1 | leanqc |
|---|---|---|
| 1 file, 1 thread | 1.2 s, 32 MB | 1.5 s, 13 MB |
| 4 files, 4 threads | 1.6 s, 105 MB | 1.7 s, 42 MB |
| Verdicts (40 compared) | identical to FastQC | identical to FastQC |
| Output | FastQC's own files, byte for byte | leanqc's own HTML/JSON/TSV |
| Truncated plain FASTQ | accepted (as FastQC) | refused |

Most of the remaining time difference was the gzip decoder: FastQC-Rust uses zlib-rs, and leanqc
used miniz_oxide. leanqc now uses zlib-rs too (about 12% faster on one file, identical output).

The same comparison on a GitHub runner (ubuntu-latest, median of 3, leanqc with zlib-rs; the
`qc-speed` job of the validation workflow, 2026-11-10):

| | FastQC 0.12.1 (Docker) | FastQC-Rust 1.1.1 | leanqc |
|---|---|---|---|
| 1 file, 1 thread | 5.43 s, 306 MB | 1.24 s, 31 MB | 1.34 s, 13 MB |
| 4 files, 4 threads | 9.50 s, 1,204 MB | 2.05 s, 104 MB | 1.93 s, 41 MB |

FastQC's time includes about 0.5 s of container start; its memory is the container's cgroup peak.

## Known gaps

- No Kmer content module (FastQC switches it off by default too).
- "Possible source" for overrepresented sequences checks only the six adapters above; FastQC
  compares against its contaminant list (151 sequences in 0.12.1). The verdict doesn't depend on
  it.
- FastQC's options for grouping (`--nogroup`, `--expgroup`), custom `limits.txt`, adapter and
  contaminant files are not supported.
- The long-read views have no verdicts of their own (no thresholds are applied to N50 or read
  quality).
- Single-threaded per file: one huge file uses one core.

## Credit

leanqc's checks are ported from [FastQC](https://www.bioinformatics.babraham.ac.uk/projects/fastqc/)
0.12.1, Copyright 2010-17 Simon Andrews, Babraham Bioinformatics, released under the GNU GPL
version 3 or any later version. leanqc (GPL-3.0-or-later) includes code translated from FastQC's
`BaseGroup`, `GCModel`, `OverRepresentedSeqs`, `DuplicationLevel`, `PhredEncoding`,
`QualityCount`, `PerBaseQualityScores`, `PerSequenceQualityScores`, `PerSequenceGCContent`,
`AdapterContent` and `PerTileQualityScores`, and uses FastQC's default adapter list
(`adapter_list.txt`) and thresholds (`limits.txt`). The encoding rule is deliberately changed (see
[quality encoding](#quality-encoding)); everything else aims to give FastQC's verdicts.
