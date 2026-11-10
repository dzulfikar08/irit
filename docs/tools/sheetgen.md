# sheetgen: sample sheets

`sheetgen` scans folders of FASTQ files and writes the sample sheet a pipeline expects. Given a
pipeline's input schema it uses that pipeline's exact columns and validates every row against the
whole schema (standard JSON Schema plus nf-schema's `uniqueEntries`, `unique` and `exists`) before
you launch, so mistakes show up in seconds instead of ten minutes into a run. If the schema has a
rule sheetgen can't evaluate, it says which and stops, unless you pass `--allow-unchecked`.

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
| `--no-pair-numeric` | Don't pair on a bare `_1`/`_2` suffix; `x_1.fq.gz` and `x_2.fq.gz` become two single-end samples. |
| `--allow-merge` | Accept different names that become the same after cleaning (`s 1`, `s_1`); the pipeline merges them. |
| `--allow-unchecked` | Write the sheet even if the schema has rules sheetgen can't evaluate (listed as warnings). |

## What it recognises

- bcl2fastq / BCL Convert names: `Sample_S1_L001_R1_001.fastq.gz`. Lanes become separate rows
  (nf-core pipelines merge rows that share a sample name). Index reads (`I1`, `I2`) are skipped.
- `_R1`/`_R2`, `_1`/`_2`, `.R1`, with `.fastq`/`.fq` and `.gz`, `.bz2`, `.zst`.
- Two files are mates only when they are in the **same folder** and their names differ only in the
  mate token (`A_R1` / `A_R2`). `runA/A_R1` is never paired with `runB/A_R2`. Several pairs for one
  sample (lanes, or runs in different folders) become several rows.
- Errors instead of guesses: two files claiming the same mate of one pair (`A_R1.fq.gz` next to
  `A_R1.fastq.gz`), an R2 without an R1 next to it, and a sample with both single-end and paired
  rows (pipelines reject that; usually a mate is missing or misnamed).
- A pair matched only by a bare `_1`/`_2` suffix (SRA style, e.g. `ERR044595_1.fastq.gz`) is listed
  in the summary, because `patient_1` and `patient_2` might be two single-end samples. Use
  `--no-pair-numeric` in that case.
- Files without a mate marker are single-end (or long reads with `--long`).
- Symlinks keep their own names, so a folder of renamed links behaves as expected.
- Sample names are cleaned to `[A-Za-z0-9._-]`. Two different names that become the same after
  cleaning (`s 1` and `s_1`) are an error, since the pipeline would merge them into one sample;
  `--allow-merge` accepts it.

## Schema handling

From `schema_input.json` it takes the column order, which column is the sample (the `meta: ["id"]`
or `["sample"]` tag, or a name like `sample`/`ID`), and the read columns (`fastq_1`, `R1`,
`short_reads_1`, `LongFastQ`, `long_reads`, ...). A `run`/`run_accession` column is filled with
1, 2, ... per sample, so lanes stay distinct under `uniqueEntries: ["sample", "run"]`.

Each row is then checked the way nf-schema checks it:

- Blank cells are left out of the row (so `required` and `dependentRequired` see them as missing),
  and values are cast to the column's declared `type` (`"1"` becomes `1` in an integer column,
  `true`/`false` in a boolean one); a value that doesn't cast fails the `type` check.
- The row is validated against the full schema with the
  [`jsonschema`](https://crates.io/crates/jsonschema) crate: `type`, `enum`, `pattern` (ECMA-style,
  look-around included), `anyOf`/`oneOf`/`allOf`/`not`/`if`, `required`, `dependentRequired`,
  ranges, lengths, `$ref`.
- nf-schema's own keywords are checked by sheetgen: `uniqueEntries` (on the array or in a top-level
  `allOf`; rows with none of the listed columns filled are skipped), `unique` on a column (`true`,
  or a list of columns to group by), and `exists` (the path must exist, as a file for
  `format: file-path` or a folder for `directory-path`, relative to the current folder).
- Any other keyword or `format` is listed as unchecked and stops the run; `--allow-unchecked` writes
  the sheet anyway and leaves those rules to the pipeline. Annotations (`meta`, `errorMessage`,
  `description`, `mimetype`, ...) are ignored.

Blanks are written as `NA` only when a column's pattern lists `NA` as an explicit alternative
(bacass's `^(...|NA)$`); a pattern that merely happens to accept the letters `NA` (`^\S+$`) gets an
empty cell. Rule violations stop the run with the row, column and the pipeline's own error message,
plus a hint (`--set ...` for required or `dependentRequired` columns, `--link-into` for paths with
spaces). A schema whose items aren't a table (nf-core/fetchngs takes a list of accessions) is
refused.

Regression tests run against the real `schema_input.json` of nf-core/bacass 2.7.0, rnaseq 3.27.0,
mag 5.5.0, taxprofiler 2.0.1 and fetchngs 1.13.0 (copies in `sheetgen/tests/fixtures/`).

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

- nf-schema runs patterns on the JVM; sheetgen uses ECMA-style regexes (fancy-regex). They agree on
  what nf-core schemas use, but exotic Java-only syntax could be judged differently.
- Type casting approximates nf-schema's handling of CSV/TSV cells (which turns numbers and
  booleans into typed values before validating): a cell is cast to the first declared type it fits,
  trying integer, number, boolean, then string. It has not been compared cell by cell with
  nf-schema's own code.
- The `schema` keyword (validate a file's contents against another schema) is not supported and is
  reported as unchecked.
- YAML sample sheets (supported by nf-schema) are not written yet.
- No check that R1 and R2 have the same read count (that needs reading the files).
