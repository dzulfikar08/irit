# isotable: one row per sample

`isotable` collects the outputs of the usual QC, assembly and typing tools into one table, one row
per sample (an isolate or a metagenome-assembled genome), and grades each row against thresholds.

## Why

Pipelines keep writing their own summary scripts for this, and MultiQC, which is excellent for
browsing, aggregates per tool rather than per sample, without a pass/fail decision. See
[landscape](../research/landscape.md#1-manual-steps-worth-a-cli).

## Usage

```sh
isotable results/ --sheet samplesheet.csv -o qc_table.tsv --mqc isotable_mqc.tsv
isotable results/ --mode mag                     # MIMAG tiers from CheckM2
isotable results/ --min-n50 20000 --max-contigs 200
isotable results/ --require reads,q30,contigs,n50,completeness,kraken
```

| Option | Effect |
|---|---|
| `--sheet FILE` | Sample sheet (e.g. from [`sheetgen`](sheetgen.md)) with the real sample names to join on. Recommended. |
| `--mode isolate\|mag` | Grading rules (below). Default `isolate`. |
| `-o FILE` | Output TSV (default stdout). |
| `--mqc FILE` | Also write a MultiQC custom-content table. |
| `--require LIST` | Measurements a sample needs before it can pass (below). `--require none` turns this off. |
| `--min-reads`, `--min-q30`, `--max-contigs`, `--min-n50`, `--min-busco`, `--min-completeness`, `--max-contamination` | Thresholds. |

## Outputs it reads

Found anywhere under the given folders, by file name:

| Tool | File | Columns |
|---|---|---|
| fastp | `*.fastp.json` (one per sample, named after it) | raw and trimmed reads, Q30 after trimming, duplication |
| leanqc | `*_leanqc.json` (one per read file) | failed checks (R1 and R2 merged) |
| Kraken 2 | `*kraken2*.txt`, `*.kreport`, `*.report` whose content is a Kraken 2 report | top species and its % |
| QUAST | `transposed_report.tsv` | contigs, length, N50, GC |
| BUSCO | `short_summary.*.json` | complete %, lineage |
| CheckM2 | `quality_report.tsv` | completeness, contamination |
| GTDB-Tk | `*summary.tsv` | classification |
| mlst | `*mlst*.tsv` | scheme, sequence type |
| AMRFinderPlus | `*amrfinder*.tsv` | AMR genes |

Only columns with data appear in the output. Kraken 2 files are recognised by their content (six
or eight tab-separated columns with a rank code), read line by line; per-read output such as
`*.kraken2.classifiedreads.txt` is skipped (and listed) instead of being read into memory.

## Joining by sample

Each tool names things differently (`ERR044595.fastp.json`, `ERR044595-unicycler.scaffolds`,
`ERR044595_1M_1.fastq.gz`). The rules are strict, because a wrong join silently mixes two samples:

- **With `--sheet`** (recommended): a record belongs to the sheet name it equals. Otherwise it
  belongs to the longest sheet name it starts with, but only if everything after that name is a
  recognised suffix: file extensions (`.fa`, `.fastq.gz`, `.scaffolds`, `.contigs`), assembler tags
  (`-unicycler`, `-spades`, ...), subsampling tags (`_1M`, `_500k`) and, for read files only, mate,
  lane and index tags (`_R1`, `_1`, `_R1_001`, `_L001`, `_S3`). So `patient_1` does **not** join
  sample `patient`: it is reported as unmatched and gets its own row. Every join that isn't an exact
  name match is printed as a note.
- **Without a sheet**: extensions and assembler tags are stripped. A mate suffix is stripped only
  from per-file read QC (leanqc), and only when the other mate's report is there too and no other
  tool uses the unstripped name; fastp reports, assemblies, bins and genomes keep their names. So
  `patient_1` and `patient_2` stay two samples. Subsampling tags are not stripped without a sheet.

## Several files for one sample

Each value remembers which file it came from.

- If two files give **different** values for the same field (two QUAST reports with different N50,
  say), the cell shows `CONFLICT`, the sample's status is `conflict`, and the reason names both files
  and values. Equal values from several files are fine.
- fastp: identical reports (the same numbers, e.g. a copied results folder) count once. Two reports
  on the **same raw reads** with different results (fastp rerun with other settings, kept in a
  sibling folder) are a `conflict`, not two runs: the fields that differ show `CONFLICT` and a note
  names both files. Reports count as the same raw reads when fastp's whole `before_filtering`
  summary matches, or when the input file names in fastp's recorded `command` and the raw read
  count both match. Only reports on different raw reads are pooled as separate runs of the sample:
  reads add up, Q30 is pooled as total Q30 bases over total bases (fastp's `q30_bases` /
  `total_bases` after filtering), and duplication is weighted by reads. Older fastp files without
  base counts can't be pooled and Q30 is left out.
- leanqc: failed checks from all of a sample's read files are merged.

## Grading

Statuses, worst first: `conflict` (files disagree), `fail`, `incomplete` (a required measurement is
missing), `warn`, `pass`. A sample never passes on partial evidence.

**Required measurements.** By default an isolate needs reads and Q30 (fastp), contigs and N50
(QUAST), and completeness (CheckM2 or BUSCO); a MAG needs CheckM2 completeness and contamination.
`--require` replaces the list with any of `reads`, `q30`, `contigs`, `n50`, `completeness`,
`contamination`, `kraken`, `gtdb`, `mlst`, `amr`; `--require none` turns the check off.

Long-read-only isolates (Nanopore assembled without short reads) have no fastp report, so under the
isolate defaults every sample is `incomplete`. Ask for what such a run does produce, e.g.:

```sh
isotable results/ --require contigs,n50,completeness
```

**Isolate mode**:

| Check | Warn | Fail |
|---|---|---|
| Reads after trimming (all runs) | | < 100,000 |
| Q30 after trimming | < 80% | |
| leanqc | any failed check | |
| Contigs | > 300 | > 600 |
| N50 | < 10,000 | < 5,000 |
| BUSCO complete | < 95% | < 90% |
| CheckM2 completeness | < 95% | < 90% |
| CheckM2 contamination | > 2% | > 5% |

These thresholds are starting points chosen from common practice for bacterial isolates, not a
standard. 100,000 reads is a floor for "something went wrong" (about 3× coverage of a 5 Mb genome
at 150 bp), not a target. Have a domain scientist review every threshold for the organism and the
study before using the grades for decisions, and change them with the flags above.

**MAG mode** uses MIMAG tiers from CheckM2: high (> 90% complete, < 5% contamination) passes, medium
(≥ 50%, < 10%) warns, anything else fails. The tier is shown as `high*`: the full MIMAG high-quality
standard also requires the 23S, 16S and 5S rRNA genes and at least 18 tRNAs, which these tools
don't report.

### Reduced genomes (Patescibacteria / CPR)

When GTDB-Tk places a genome in Patescibacteria and its CheckM2 completeness is below 90%, a note is
added; the verdict is not changed. Evidence:

- CheckM2 is far more accurate than CheckM1 for reduced-genome lineages without a specific marker
  set (mean absolute error 6.4 against 19.7) ([CheckM2 paper](https://www.biorxiv.org/content/10.1101/2022.07.11.499243v1.full)).
- It can still underestimate: complete, single-contig Patescibacteria genomes were scored as less
  than complete ([Patescibacterium danicum, ISME Communications](https://academic.oup.com/ismecommun/article/5/1/ycae147/7904805)),
  and on curated complete endosymbiont genomes CheckM2 averaged 71% completeness.
- Studies of Patescibacteria-rich catalogues re-check these genomes with the CPR-specific marker
  set ([oral microbiome catalogue](https://www.biorxiv.org/content/10.1101/2025.03.10.642329v1.full)).

## Tested

- Real outputs from the nf-core/bacass test run (fastp, QUAST, BUSCO) plus leanqc reports: with a
  sheet, both samples joined across all four tools; thresholds flagged the leanqc failures.
- Regression tests for the joining and merging rules: `patient_1`/`patient_2` stay two samples with
  and without a sheet that lists `patient`; two QUAST reports with different N50 give `conflict`;
  a copied fastp report is counted once while two runs are pooled, and two fastp analyses of the
  same raw reads (the bacass ERR064912 report and a stricter rerun) give `conflict` instead of
  doubling the reads; a 5-read fastp file fails and a
  sample without assembly data is `incomplete`; a per-read Kraken 2 file doesn't replace the report.
- Synthetic CheckM2 and GTDB-Tk files (in `isotable/testdata/mag/`, made up for the test) in MAG
  mode: tiers as expected, including a 92%-complete bin failing on 12.5% contamination, and the
  note on both Patescibacteria bins.

## Known gaps

- No Bracken, Mash or sylph parsers yet.
- AMRFinderPlus outputs are matched by file name; one file per sample is assumed.
- Dangling symlinks and unreadable folders in the results are skipped and listed, not read.
- fastp reports split per mate (one fastp run per read file) would be read as two runs of a sample
  named after each file; nf-core pipelines don't produce them.
- No per-species expected genome size check.
