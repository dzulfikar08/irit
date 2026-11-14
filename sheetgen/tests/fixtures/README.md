# Test fixtures

Copies of real nf-core sample-sheet schemas (`assets/schema_input.json`), used by sheetgen's tests.
They are unmodified and come from the nf-core repositories, which are MIT-licensed
(© the nf-core community and the respective pipeline authors).

| File | Source |
|---|---|
| `bacass-2.7.0.schema_input.json` | nf-core/bacass 2.7.0 (Nextflow asset cache, commit `ce30c3a`) |
| `rnaseq-3.27.0.schema_input.json` | https://raw.githubusercontent.com/nf-core/rnaseq/3.27.0/assets/schema_input.json |
| `mag-5.5.0.schema_input.json` | https://raw.githubusercontent.com/nf-core/mag/5.5.0/assets/schema_input.json |
| `taxprofiler-2.0.1.schema_input.json` | https://raw.githubusercontent.com/nf-core/taxprofiler/2.0.1/assets/schema_input.json |
| `fetchngs-1.13.0.schema_input.json` | https://raw.githubusercontent.com/nf-core/fetchngs/1.13.0/assets/schema_input.json |

Downloaded 2026-11-10.

## ENA file reports

Saved responses of ENA's portal API, used by the `sheetgen verify` tests (no network in tests):
`https://www.ebi.ac.uk/ena/portal/api/filereport?accession=<RUN>&result=read_run&fields=run_accession,fastq_ftp,fastq_md5,fastq_bytes&format=tsv`.

| File | Run |
|---|---|
| `ena-filereport-ERR044595.tsv` | ERR044595 (paired: `_1`, `_2`) |
| `ena-filereport-ERR15424439.tsv` | ERR15424439 (one Nanopore file) |

Downloaded 2026-11-14.
