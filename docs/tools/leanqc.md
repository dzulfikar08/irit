# leanqc: read quality control

`leanqc` reads each FASTQ (or FASTA) file once and reports what FastQC reports, using a fraction of
the time and memory. It writes a self-contained HTML report per file (inline SVG, no scripts, opens
offline), the full numbers as JSON, a summary table, and a table MultiQC picks up.

## Usage

```sh
leanqc reads/*.fastq.gz -o qc/ -t 4
```

| Option | Effect |
|---|---|
| `-o DIR` | Output folder (default `leanqc/`). |
| `-t N` | Files processed at once; each file uses one thread (default: up to 4). |
| `--dup-sample N` | Reads examined for duplication and overrepresented sequences (default 100,000). |
| `--no-html` | JSON and TSV only. |

Input can be plain, gzip, bzip2, xz or zstd. FASTA works; the quality modules are skipped.

Outputs in the folder:

- `<name>_leanqc.html`: the report.
- `<name>_leanqc.json`: every number behind it.
- `leanqc_summary.tsv`: one row per file, with failed and warned checks listed.
- `leanqc_mqc.tsv`: MultiQC general-statistics table (reads, GC, Q30, duplication, adapters).

## Checks

Thresholds follow FastQC's defaults, so a verdict means the same thing in both tools.

| Check | Warn | Fail |
|---|---|---|
| Per-position quality | median < Q25 or lower quartile < Q10 | median < Q20 or lower quartile < Q5 |
| Per-tile quality | a tile 5 below the all-tile mean at any position | 10 below |
| Per-read quality | most common mean quality < Q27 | < Q20 |
| Per-position base content | A/T or G/C differ by > 10% | > 20% |
| Per-read GC | > 15% of reads off a normal curve | > 30% |
| N content | > 5% N at any position | > 20% |
| Read length | lengths differ | any empty read |
| Duplication | < 80% would remain after deduplication | < 50% |
| Overrepresented sequences | any sequence > 0.1% | > 1% |
| Adapter content | any adapter in > 5% of reads | > 10% |

Adapters searched: Illumina Universal, Nextera, Illumina Small RNA 3', polyA, polyG.

How some modules work:

- **Per-read GC.** A read of *n* bases can only take *n*+1 GC values, so each read's value is spread
  over the percentage bins its range covers (±half a base), as FastQC does. Without this, 75 bp
  reads fill 76 of 101 bins and the curve looks broken. (This was a bug in the first version; it
  failed every file FastQC passed.)
- **Per-tile quality.** The tile number comes from the read name, using FastQC's rule: split the
  header on `:`; with 7 or more fields (CASAVA 1.8+) the tile is the 5th, with 5–6 fields (older
  Illumina) the 3rd. Headers without a tile skip the module. More than 2,500 tiles also skips it.
- **Duplication** is measured directly on the first 100,000 reads. FastQC extrapolates from the
  distinct sequences it tracks, so the two differ by a few points.
- Per-position charts stop at 1,000 bp; every whole-read figure still uses all bases.

## Results

See [benchmarks](../benchmarks.md#leanqc-against-fastqc). In short: 2.5 s and 16 MB against
7.0 s and 483 MB for one file, and the same verdict as FastQC on every module both tools have.

## Known gaps

- No Kmer content module (FastQC disables it by default too).
- Long-read specific views (read-length N50, quality by length) are not there yet.
- Single-threaded per file: one huge file uses one core.
