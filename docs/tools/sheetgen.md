# sheetgen: sample sheets

`sheetgen` scans folders of FASTQ files and writes the sample sheet a pipeline expects. Given a
pipeline's input schema it uses that pipeline's exact columns and validates every row against the
whole schema (standard JSON Schema plus nf-schema's `uniqueEntries`, `unique` and `exists`) before
you launch, so mistakes show up in seconds instead of ten minutes into a run. If the schema has a
rule sheetgen can't evaluate, it says which and stops, unless you pass `--allow-unchecked`.

`sheetgen verify` checks the read files themselves before an analysis starts: gzip and FASTQ
integrity, MD5 and SHA-256, and the MD5s published in an `md5sum` file or at ENA (see
[Checking read files](#checking-read-files-sheetgen-verify)).

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
| `--long` | Treat unpaired files as long reads (the schema's long-read column, for hybrid assembly). `_R1`/`_R2` mates still pair; bare `_1`/`_2` (and lowercase `_r1`/`_r2`) pair only with evidence they are short reads (see [Long reads](#long-reads-and-nanopore)). |
| `--relative` | Paths relative to the current folder. |
| `--link-into DIR` | Create `<sample>_<n>_R1.fastq.gz` symlinks in DIR and list those. Originals are not touched. |
| `--no-pair-numeric` | Don't pair on a bare `_1`/`_2` suffix; `x_1.fq.gz` and `x_2.fq.gz` become two single-end samples. |
| `--allow-merge` | Accept different names that become the same after cleaning (`s 1`, `s_1`); the pipeline merges them. |
| `--allow-unchecked` | Write the sheet even if the schema has rules sheetgen can't evaluate (listed as warnings). |
| `--include-undetermined` | List `Undetermined_S0_*` files as a sample instead of skipping them. |
| `--include-fail` | List Nanopore reads that failed MinKNOW's filter (`_fail_` names, `fastq_fail/` folders, Guppy `fastq_runid_*` files in `fail/`) instead of skipping them. |

## What it recognises

- bcl2fastq / BCL Convert names: `Sample_S1_L001_R1_001.fastq.gz`. Lanes become separate rows
  (nf-core pipelines merge rows that share a sample name). Index reads (`I1`, `I2`) are skipped,
  and so are `Undetermined_S0_*` files (reads not assigned to a sample) unless
  `--include-undetermined` is given.
- `R3`/`R4` files next to `R1`/`R2` of the same name (10x Genomics ATAC, UMI runs) stop the run.
  In those layouts which read holds the cDNA, cell barcode or UMI depends on the kit, so neither
  skipping R3 nor listing it as a sample is safe: use the pipeline's own input for that layout
  (nf-core/scrnaseq, a UMI-aware pipeline), or move the R3/R4 files away if you know R1/R2 are the
  reads to list.
- Lane tokens: `_L001` (three digits, Illumina) anywhere before the mate token. One or two digits
  only in the BGI/MGI shape, directly before a bare `_1`/`_2`: `_L1` (Novogene/BGI, e.g.
  `S1_EKDL01_HGV_L1_1.fq.gz`), `_L01` (MGI, e.g. `FP150002315_L01_1.fq.gz`). Anywhere else a short
  `_L1` is part of the sample name, so `strain_L1_R1` and `strain_L2_R1` are two samples. Lanes of
  one sample become rows of one sample; when short lane tokens were merged the summary lists them
  (`S1_EKDL01_HGV (L1, L2)`), so a sample really named `..._L1` is noticed. If a provider puts
  flowcell or library codes in the name that differ between files of one sample, name samples by
  folder with `--name-from-dir`.
- `_R1`/`_R2` (any case, so `_r1` too), `_1`/`_2`, `.R1`, with `.fastq`/`.fq` and `.gz`, `.bz2`,
  `.zst`.
- Two files are mates only when they are in the **same folder** and their names differ only in the
  mate token (`A_R1` / `A_R2`). `runA/A_R1` is never paired with `runB/A_R2`. Several pairs for one
  sample (lanes, or runs in different folders) become several rows.
- Errors instead of guesses: two files claiming the same mate of one pair (`A_R1.fq.gz` next to
  `A_R1.fastq.gz`), an R2 without an R1 next to it, and a sample with both single-end and paired
  rows (pipelines reject that; usually a mate is missing or misnamed).
- A pair matched only by a bare `_1`/`_2` or a lowercase `_r1`/`_r2` suffix (SRA style, e.g.
  `ERR044595_1.fastq.gz`) is listed in the summary, because `patient_1` and `patient_2`, or
  `ctrl_r1` and `ctrl_r2`, might be single-end samples or replicates. Use `--no-pair-numeric` for
  `_1`/`_2` names; rename `_r1`/`_r2` files (uppercase `_R1`/`_R2` always pair).
- When the folder also has the same name numbered otherwise, the numbers may not be mates:
  - numbered from 0 (`x_0` next to `x_1`/`x_2`): chunks. No pair is formed; every file becomes
    its own single-end sample (or long-read row), with a warning.
  - numbered past 2 only (`s_3` next to `s_1`/`s_2`, `ctrl_r3` next to `ctrl_r1`/`ctrl_r2`, or
    fasterq-dump's `SRR5_1`..`SRR5_4` for a 10x run): replicates, extra reads or mates, sheetgen
    can't tell, so it stops. `--no-pair-numeric` lists each `_1`, `_2`, `_3` file as a single-end
    sample; lowercase `_r1`/`_r2` always pair, so rename those (`_rep1`, or `_R1`/`_R2` for mates).
- Files without a mate marker are single-end (or long reads with `--long`). With
  `--no-pair-numeric --long`, a long-read-only row named like a mate (`x_1` next to `x_2`) is
  listed with a warning.
- Symlinks keep their own names, so a folder of renamed links behaves as expected. A dangling
  symlink (or an unreadable subfolder) is skipped with a warning.
- Sample names are cleaned to `[A-Za-z0-9._-]`. Two different names that become the same after
  cleaning (`s 1` and `s_1`) are an error, since the pipeline would merge them into one sample;
  `--allow-merge` accepts it.

## Long reads and Nanopore

With `--long`, files without a mate marker are long reads. A sample's long-read file goes on its
short-read row when it has one (hybrid assembly), otherwise on a row of its own.

- `_R1`/`_R2` mates pair as usual. A bare `_1`/`_2` (or lowercase `_r1`/`_r2`) pair is formed only
  with evidence that the files are short reads: a long-read file named without the suffix beside
  them (`SRR1.fastq.gz` next to `SRR1_1`/`SRR1_2`, the SRA/fetchngs hybrid layout), or the first
  10 reads of both files (plain or gzip FASTQ) all shorter than 1,000 bp. Otherwise sheetgen stops:
  rename short-read mates `_R1`/`_R2`, or pass `--no-pair-numeric` if `reads_1`/`reads_2` are
  separate long-read files. `.bz2`/`.zst` files aren't sniffed, so they need the hybrid layout or
  `_R1`/`_R2` names.
- The long-read file needs a long-read column (`LongFastQ`, `long_reads`, ...). If a sample has both
  short reads and a long-read file and the sheet has none (the default `sample,fastq_1,fastq_2`
  layout, or a short-read pipeline's schema), sheetgen stops instead of dropping the file. Use the
  hybrid pipeline's schema, e.g. `--pipeline nf-core/bacass`. Long-read-only rows without a
  long-read column go in the R1 column, with a warning.
- Oxford Nanopore MinKNOW/Guppy output (files in `barcode01/`, `unclassified/`, with
  `_pass_`/`_fail_` in the name or starting `fastq_runid_`, and files without a mate token in a
  Guppy `pass/` or `fail/` folder) is never paired: `FAX123_pass_barcode01_abc_0` and `..._1`, or
  `fastq_runid_abc_0_0` and `..._0_1`, are chunk numbers, not mates. The trailing chunk number is
  dropped from the sample name, so a run's chunks are one sample (`fastq_runid_abc_0`). Without
  `--long` such files are written as single-end short reads with a warning to add `--long`.
- Reads that failed MinKNOW's quality filter (`_fail_` in the name, anywhere under a
  `fastq_fail/` folder, or `fastq_runid_*` files under a `fail/` folder) are skipped with a note,
  so scanning a whole run folder with `--name-from-dir` doesn't mix them into `barcode01`.
  `--include-fail` keeps them; pointing sheetgen at a `fastq_fail/` folder itself keeps the files without `_fail_` in the name.
- A sample with several long-read files (MinKNOW writes one per few thousand reads) is an error
  unless the schema has a run column to tell the rows apart: nf-core/bacass takes a single
  `LongFastQ` per sample. Concatenate first (gzip files concatenate as they are):

```sh
mkdir -p merged
for d in fastq_pass/barcode*/; do cat "$d"*.fastq.gz > "merged/$(basename "$d").fastq.gz"; done
sheetgen merged/ --long --pipeline nf-core/bacass -o samplesheet.tsv
```

Each merged file is then one sample named after its barcode. For files left in their barcode
folders, `--name-from-dir` names samples `barcode01`, `barcode02`, ...

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

## Checking read files: `sheetgen verify`

A download can arrive at the right size and still be corrupt without curl reporting an error. In
this project four ENA files did; the gzip CRC error showed up only when Flye was deep into the
file. `sheetgen verify` reads each file once and checks it before anything else does:

```sh
sheetgen verify reads/                                    # every FASTQ file under reads/
sheetgen verify reads/ --md5 reads/MD5SUMS                # compare with an md5sum file
sheetgen verify ERR044595_1.fastq.gz ERR044595_2.fastq.gz --ena
sheetgen verify AHB29_ont.fastq.gz --ena --run AHB29_ont.fastq.gz=ERR15424439
sheetgen verify --sheet samplesheet.csv --json > verify.json
```

| Option | Effect |
|---|---|
| `PATH...` | Read files, or folders scanned recursively for FASTQ files (all of them, `_fail_` files included). |
| `--sheet FILE` | Also check every FASTQ path in this sample sheet (CSV, or TSV by extension or tabs). Relative paths are taken from the current folder; URLs are skipped with a note. |
| `--md5 FILE` | Expected MD5s, `md5sum` style (`<md5>  <file>` or `<md5> *<file>`) or BSD style (`MD5 (file) = <md5>`). Names are matched by path relative to the list's folder, then by file name. Repeatable. |
| `--ena` | Look up expected MD5s and sizes at ENA (network; nothing is fetched without this flag). |
| `--run FILE=ACCESSION` | Run accession for a file whose name doesn't contain one (needs `--ena`). Repeatable. |
| `--json` | Print the report as JSON on stdout (for irit-mcp and scripts). |
| `-j N` | Check N files at a time (default 1; each file uses four threads). |

What is checked, in one pass over the file:

- **gzip**: every member is decompressed to the end, so zlib checks each member's header, CRC-32
  and length. Files made of several gzip members (concatenated `cat a.gz b.gz`, or bgzip) are read
  to the last member. A file that ends inside a member, or has data after the last member that isn't
  gzip, is corrupt.
- **FASTQ**: four-line records, a header starting with `@`, a third line starting with `+`, and a
  quality string as long as the sequence (a trailing `\r` is ignored; blank lines only at the end).
  Reads and bases are counted. Multi-line (wrapped) FASTQ is reported as broken. Characters inside
  the sequence and quality lines are not checked.
- **MD5 and SHA-256** of the file as stored, computed on the same read.
- `.bz2`/`.zst` files are hashed only; their content is not checked.

Each file gets one status, and the exit status is 1 if any file isn't `ok`:

| Status | Meaning |
|---|---|
| `ok` | gzip and FASTQ fine, and the MD5 (and ENA's size, when known) match. Without `--md5` or `--ena`, a structurally fine file is `ok`. |
| `mismatch` | Structure fine, but the MD5 or size differs from the expected one. |
| `corrupt` | gzip or FASTQ problem. The report gives what was wrong, the byte offset in the file as stored (compressed bytes for gzip) and the FASTQ record and line being read. |
| `not listed` | `--md5` or `--ena` was given but has no entry for this file. |

Where a problem is reported: for plain FASTQ, the offset is the start of the bad line. For gzip,
it is where zlib stopped; a CRC-32 or length error can only be noticed at the end of a member, so
the damage is somewhere before that. Damaged deflate data often decodes to garbage that breaks the
FASTQ records first; sheetgen then keeps decompressing, and if the gzip check fails too, it reports
the gzip damage, located at the record where the reads first broke (nearer the damage), with
zlib's position in the message. Entries of an `--md5` list that weren't checked are listed in a
note (and in `listed_not_checked` in the JSON), without failing the run.

### ENA lookup

With `--ena`, the run accession (`ERR`, `SRR` or `DRR` followed by six or more digits) is taken
from the file name, or from `--run`. One request per run goes to ENA's portal API:

```
https://www.ebi.ac.uk/ena/portal/api/filereport?accession=<RUN>&result=read_run&fields=run_accession,fastq_ftp,fastq_md5,fastq_bytes&format=tsv
```

The response lists the run's FASTQ files with their MD5s and sizes. A local file is matched to one
of them by the same file name, else by mate (`x_1`, `x_R1` → ENA's `<RUN>_1.fastq.gz`, `_2`
likewise, a file without a mate token → `<RUN>.fastq.gz`), else to the run's only file. If none
fits, the file is `not listed` with a note naming ENA's files. A failed lookup (after three tries)
stops the run.

### JSON report

`--json` prints one object: `ok` (all files ok), `files` (one entry per file, in input order),
`summary` (counts per status), `listed_not_checked`, `notes`, `bytes` and `seconds`. Each file has
`path`, `status`, `bytes`, `md5`, `sha256`, `expected_md5`, `expected_bytes`, `expected_from` (the
list file, or `ENA <run> (<file>)`), `compression` (`gzip`, `plain` or `not checked`),
`gzip_members`, `uncompressed_bytes`, `reads`, `bases`, `problem` (`null`, or `what`,
`byte_offset`, `record`, `line`), `note` and `seconds`.

### Speed and memory

The file is read in 1 MiB blocks by one thread and handed to three others (MD5, SHA-256, and gzip
plus FASTQ), at most four blocks ahead, so memory stays flat whatever the file or read size.
A single-member gzip file can only be inflated on one core, which sets the speed; `-j` checks
several files at once.

Measured 2026-11-14 on a laptop with a Ryzen 5 PRO 4650U (6 cores, 12 threads) that other jobs
were using at the same time (load average 11-15). The test file was a 568 MB single-member Nanopore
`.fastq.gz` (117,455 reads, 713 Mbases), already in the page cache, and each tool was run twice back to back:

| Command | Wall time |
|---|---|
| `sheetgen verify` (gzip, FASTQ, MD5, SHA-256) | 3.3-3.5 s (163-174 MB/s), 12.5 MB peak RSS |
| `gzip -t` | 6.2-7.4 s |
| `md5sum` | 0.9 s |
| `gzip -dc \| md5sum` | 6.8-10.7 s |

Under heavier load (average 20-30) the four files of a Nanopore data set (4.46 GB together) took
55.6 s one at a time (80 MB/s, 17 MB peak RSS) and 29.6 s with `-j 4` (150 MB/s; the 2.07 GB file
sets the wall time). All four MD5s matched ENA's.

On a copy of a real 309 MB file with one byte flipped at offset 150,000,000, zlib noticed the CRC
error only at the end of the file (byte 308,990,709); `verify` reported the damage at byte
150,351,704, record 40,973, where the decompressed reads first broke. `gzip -t` reports only
"crc error". A copy cut at 200,000,000 bytes was reported as truncated at that byte.

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
- No check that R1 and R2 have the same read count when writing a sheet (that needs reading the
  files); `sheetgen verify` reports each file's read count.
