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
```

| Option | Effect |
|---|---|
| `--sheet FILE` | Sample sheet (e.g. from [`sheetgen`](sheetgen.md)) with the real sample names to join on. Recommended. |
| `--mode isolate\|mag` | Grading rules (below). Default `isolate`. |
| `-o FILE` | Output TSV (default stdout). |
| `--mqc FILE` | Also write a MultiQC custom-content table. |
| `--min-q30`, `--max-contigs`, `--min-n50`, `--min-busco`, `--min-completeness`, `--max-contamination` | Thresholds. |

## Outputs it reads

Found anywhere under the given folders, by file name:

| Tool | File | Columns |
|---|---|---|
| fastp | `*.fastp.json` | raw and trimmed reads, Q30 after trimming, duplication |
| leanqc | `*_leanqc.json` | failed checks (R1 and R2 merged) |
| Kraken 2 | `*kraken2*.report.txt`, `*.kreport` | top species and its % |
| QUAST | `transposed_report.tsv` | contigs, length, N50, GC |
| BUSCO | `short_summary.*.json` | complete %, lineage |
| CheckM2 | `quality_report.tsv` | completeness, contamination |
| GTDB-Tk | `*summary.tsv` | classification |
| mlst | `*mlst*.tsv` | scheme, sequence type |
| AMRFinderPlus | `*amrfinder*.tsv` | AMR genes |

Only columns with data appear in the output.

## Joining by sample

Each tool names things differently (`ERR044595.fastp.json`, `ERR044595-unicycler.scaffolds`,
`ERR044595_1M_1.fastq.gz`). With `--sheet`, a record belongs to the longest sample name it equals or
starts with, followed by a separator. Without a sheet, file extensions, assembler tags
(`-unicycler`, `-spades`, ...) and read-mate suffixes (`_R1`, `_1`) are stripped, which works for
most layouts but not all: files that carry extra tags (such as a subsampling tag) get their own row.
Records that match no sheet name are reported.

## Grading

**Isolate mode** (defaults are common conventions for bacterial isolates, not a standard):

| Check | Warn | Fail |
|---|---|---|
| Q30 after trimming | < 80% | |
| leanqc | any failed check | |
| Contigs | > 300 | > 600 |
| N50 | < 10,000 | < 5,000 |
| BUSCO complete | < 95% | < 90% |
| CheckM2 completeness | < 95% | < 90% |
| CheckM2 contamination | > 2% | > 5% |

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
- Synthetic CheckM2 and GTDB-Tk files (in `isotable/testdata/mag/`, made up for the test) in MAG
  mode: tiers as expected, including a 92%-complete bin failing on 12.5% contamination, and the
  note on both Patescibacteria bins.

## Known gaps

- No Bracken, Mash or sylph parsers yet.
- AMRFinderPlus outputs are matched by file name; one file per sample is assumed.
- No per-species expected genome size check.
