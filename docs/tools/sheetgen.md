# sheetgen: sample sheets

`sheetgen` scans folders of FASTQ files and writes the sample sheet a pipeline expects. Given a
pipeline's input schema it uses that pipeline's exact columns and checks every value against the
schema's rules before you launch, so mistakes show up in seconds instead of ten minutes into a run.

## Why

Every nf-core pipeline ships its own small script for this, none of them read subfolders, and none
read the pipeline's schema. QIIME 2's forum has a steady stream of manifest errors (commas for tabs,
relative paths, duplicate IDs). See [landscape](../research/landscape.md#1-manual-steps-worth-a-cli).

## Usage

```sh
sheetgen data/ -o samplesheet.csv                            # sample,fastq_1,fastq_2
sheetgen data/ --pipeline nf-core/bacass -o samplesheet.tsv  # bacass's own columns
sheetgen data/ --schema path/to/schema_input.json --set strandedness=auto
sheetgen data/ -f qiime2 -o manifest.tsv                     # QIIME 2 manifest
sheetgen data/ --pipeline nf-core/bacass --link-into links/  # clean, space-free symlinks
```

| Option | Effect |
|---|---|
| `-o FILE` | Output file. `.tsv` writes tabs, anything else commas. Default: stdout. |
| `-f nfcore\|qiime2` | Layout when no schema is given. |
| `--pipeline NAME` | Find `assets/schema_input.json` for a pipeline Nextflow has pulled (`$NXF_HOME`, default `~/.nextflow`). |
| `--schema FILE` | Use this schema instead. |
| `--set COL=VALUE` | Fixed value for a column the files can't tell us, such as `strandedness`. Repeatable. |
| `--name-from-dir` | Name samples after the folder holding the files. |
| `--long` | Treat unpaired files as long reads (the schema's long-read column, for hybrid assembly). |
| `--relative` | Paths relative to the current folder. |
| `--link-into DIR` | Create `<sample>_<n>_R1.fastq.gz` symlinks in DIR and list those. Originals are not touched. |

## What it recognises

- bcl2fastq / BCL Convert names: `Sample_S1_L001_R1_001.fastq.gz`. Lanes become separate rows
  (nf-core pipelines merge rows that share a sample name). Index reads (`I1`, `I2`) are skipped.
- `_R1`/`_R2`, `_1`/`_2`, `.R1`, with `.fastq`/`.fq` and `.gz`, `.bz2`, `.zst`.
- Files without a mate marker are single-end (or long reads with `--long`).
- Symlinks keep their own names, so a folder of renamed links behaves as expected.
- Sample names are cleaned to `[A-Za-z0-9._-]`; two names that collide after cleaning are reported.

## Schema handling

From `schema_input.json` it reads the column order, which column is the sample (the `meta: ["sample"]`
tag or a name like `sample`/`ID`), the read columns, required columns, `enum` choices and `pattern`
rules (including `anyOf`). Blanks are written as `NA` when the schema's patterns accept `NA`, as
bacass's do. Values that break a rule stop the run with the row, column and the pipeline's own
error message, plus a hint (`--set ...` for missing required columns, `--link-into` for paths with
spaces).

## Tested

On a deliberately messy folder (two lanes of one sample, SRA-style `_1/_2` names, a sample and
folder with spaces, a lone R1, an index read):

- the bacass schema check rejected the paths with spaces, as bacass would;
- with `--link-into`, nf-core/bacass 2.7.0 accepted the generated sheet and ran all 31 steps in
  `-stub-run`, merging the two lanes itself;
- the QIIME 2 layout refused the multi-lane sample with an explanation instead of writing an
  invalid manifest.

Bugs found and fixed while testing: symlinked files were resolved to their targets (losing their
names), columns came out alphabetically instead of in schema order, and an R2-only sample crashed
the QIIME 2 writer.

## Known gaps

- Schema patterns using look-around are skipped (noted on stderr); Rust's regex engine lacks it.
- YAML sample sheets (supported by nf-schema) are not written yet.
- No check that R1 and R2 have the same read count (that needs reading the files).
