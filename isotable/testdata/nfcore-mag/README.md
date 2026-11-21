nf-core/mag `GenomeBinning/bin_summary.tsv` layouts, written by the pipeline's own
`bin/combine_tables.py` (https://github.com/nf-core/mag) from made-up inputs in the formats the
pipeline passes it: a bin depths table (two samples), CheckM2 1.x `quality_report.tsv` rows, CheckM
`qa` rows, a BUSCO 6 `batch_summary.txt`, a QUAST bin summary, and nf-core/mag's GTDB-Tk summary
(`bin/summary_gtdbtk.py` columns). Completeness, contamination and classifications are those of
`../mag/` (bin.1 to bin.5, renamed MEGAHIT-MetaBAT2-sample1.1 to .5); CheckM and BUSCO values are
made up to differ from CheckM2's.

- `5.5.0/bin_summary.tsv`: combine_tables.py at tag 5.5.0 with BUSCO, CheckM, CheckM2, QUAST and
  GTDB-Tk (columns suffixed `_busco`, `_checkm`, `_checkm2`, `_quast`, `_gtdbtk`, as since 5.2.0).
- `5.0.0/bin_summary.tsv`: combine_tables.py at tag 5.0.0 with `--binqc_tool checkm2`, QUAST and
  GTDB-Tk (one bin QC tool, columns not suffixed, a pandas index column first: the layout of every
  release from 3.0 to 5.1; CheckM2 is an option from 3.3.0).

Not real genomes.
