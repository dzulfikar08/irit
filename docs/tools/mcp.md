# irit-mcp: tools for AI agents

`irit-mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server that gives AI
agents (Claude Code, Claude Desktop and other MCP clients) the irit tools as typed tools. Each call
runs the real `sheetgen`, `leanqc`, `isotable` or `dblock` binary, and is written to a run record
with a `replay.sh` that repeats the same commands without any AI.

It also wraps four third-party tools for the microbial isolate path, so a whole run (trim,
annotate, check quality, classify) is recorded and replayable the same way: fastp, Bakta, CheckM2
and GTDB-Tk. See [Third-party tools](#third-party-tools).

## Why

Agents for bioinformatics fail in three documented ways (see
[landscape](../research/landscape.md#3-ai-agents-for-bioinformatics)): they invent command-line
flags, they carry on after a tool has refused bad input, and they leave no record that lets a
result be reproduced without the AI. `irit-mcp` handles each one:

- **Invented flags.** Every tool has a JSON schema with `additionalProperties: false`. An unknown
  parameter is refused before anything runs, and the error lists the parameters that do exist.
  Paths must exist, enums and ranges are checked, and database ids follow dblock's own rules.
- **Carrying on after a failure.** A non-zero exit comes back as an MCP error (`isError: true`)
  with the tool's stderr and exit status, never as a result the agent could mistake for success.
- **No record.** Every call goes into a run folder: exact argv, tool version, working folder,
  SHA-256 of every input and output, times, exit status. `replay.sh` re-runs the commands and checks
  each output's hash against the record.

The server is a thin layer. It runs the binaries as subprocesses, so the CLI stays the single
source of truth for what each tool does. The one exception: it links isotable's output parsers, so
the fastp, CheckM2 and GTDB-Tk numbers it returns are read exactly as isotable grades them.

## Install and register

Build it (it is a standalone crate for now), and have the irit binaries on `PATH` or in a folder:

```sh
cargo build --release --manifest-path mcp/Cargo.toml     # mcp/target/release/irit-mcp
cargo build --release -p sheetgen -p leanqc -p isotable -p dblock
```

Claude Code (the server runs in the folder Claude Code was started in):

```sh
claude mcp add irit -- /path/to/irit-mcp --bin-dir /path/to/irit/target/release
```

Any client that takes a JSON config (Claude Desktop's `claude_desktop_config.json`, a project
`.mcp.json`, ...):

```json
{
  "mcpServers": {
    "irit": {
      "command": "/path/to/irit-mcp",
      "args": ["--bin-dir", "/path/to/irit/target/release", "--runs-dir", "/data/project/irit-runs"]
    }
  }
}
```

| Option | Effect |
|---|---|
| `--bin-dir DIR` | Folder holding the irit binaries (env `IRIT_BIN_DIR`). Default: search `PATH`. |
| `--tool-path NAME=PATH` | Where a third-party tool is (`fastp`, `bakta`, `checkm2`, `gtdbtk`). Repeatable. Default: search `PATH`. |
| `--runs-dir DIR` | Where run records go (env `IRIT_RUNS_DIR`). Default `./irit-runs`. |
| `--run-id ID` | Name of this session's run folder. Default `<UTC time>-<pid>`. |
| `--root DIR` | Folder the tools may write in. Default: the server's working folder. |
| `--allow-outside-root` | Turn the root check off, so tools can write anywhere the server's user can. Not recommended. |

Tools run in the server's working folder, and relative paths in parameters are resolved there.

### Where tools may write

Every path a tool would write to must resolve inside the root: sheetgen `output` and `link_into`,
leanqc `outdir`, isotable `output` and `mqc`, fastp `out1`, `out2`, `json` and `html`, Bakta and
CheckM2 `outdir`, GTDB-Tk `out_dir` and `scratch_dir`, `dblock.toml` and `dblock.lock` in the `project_dir`
of `dblock_add` and `dblock_fetch` (the folder and each file are checked, since dblock writes
through a symlinked `dblock.toml`), and `dblock_add`'s `store`. "Resolve" means after `..` and
symlinks, including a symlink that doesn't point anywhere yet: `qc -> /elsewhere` or `../x.csv` is
refused, a symlink to a folder inside the root is fine. Absolute paths are accepted if they land
inside the root.

leanqc writes its reports by name into `outdir`, so a symlink already in that folder
(`qc/s_leanqc.json -> /elsewhere`) would be written through: the call is refused if any symlink
in `outdir` resolves outside the root. sheetgen's `link_into` is exempt: sheetgen only creates
links there and never writes through an existing one.

The run records folder (`--runs-dir`, `./irit-runs` by default, so usually inside the root) is
never a valid target, even with `--allow-outside-root`: an `output` of
`irit-runs/<run>/replay.sh` is refused.

Inputs (FASTQ files, results folders, sheets, schemas, catalogs, `project_dir` for the read-only
dblock tools) may be anywhere the server can read.

Limits of these checks: they run before the tool starts, so a symlink created or swapped in
between (by another process on the machine) is not caught; the server guards against a
confused or misled agent, not against a local attacker racing it. Hard links (a file in the root
that shares its data with one outside) are not detected.

The dblock store that comes from the environment (`DBLOCK_STORE`, else `$XDG_DATA_HOME/dblock`,
else `~/.local/share/dblock`) or from a `store =` line already in `dblock.toml` is not checked: it
was chosen by whoever set up the machine or project, not by the agent.

Existing outputs are only replaced when the call says so:

- sheetgen `output`, isotable `output` / `mqc`: an existing file (or symlink) is refused unless the
  call passes `overwrite: true`.
- leanqc `outdir`: a folder that already has entries is refused unless `overwrite: true`; an empty
  or missing folder is fine.
- sheetgen `link_into` has no `overwrite`: sheetgen itself refuses to replace a link that points
  elsewhere.
- fastp outputs work like sheetgen `output`. Bakta `outdir`, CheckM2 `outdir` and GTDB-Tk
  `out_dir` work like leanqc `outdir`. Note that CheckM2 empties its `outdir` before it writes.
- `dblock_add` and `dblock_fetch` update `dblock.toml` / `dblock.lock` in place, as dblock does.
The transport is stdio; streamable HTTP is not built in yet.

## Tools

Optional parameters can be left out; the tool's own default then applies.

### `sheetgen`

Sample sheet from folders of FASTQ files ([sheetgen](sheetgen.md)).

| Parameter | Type | Meaning |
|---|---|---|
| `dirs` | list of folders (required) | Folders to scan recursively. |
| `output` | file | Write the sheet here (inside the root); otherwise it is only returned. |
| `overwrite` | boolean | Replace `output` if it exists. Default `false`. |
| `format` | `nfcore` \| `qiime2` | Layout when no schema is given. |
| `pipeline` | `owner/name` | Use this pipeline's `schema_input.json` from Nextflow's cache. |
| `schema` | file | Use this schema. Not together with `pipeline`. |
| `set` | object | Fixed column values, e.g. `{"strandedness": "auto"}`. |
| `link_into` | folder | Write clean symlinks there and list those. |
| `name_from_dir`, `long`, `relative`, `no_pair_numeric`, `allow_merge`, `allow_unchecked` | booleans | As the CLI flags. |

Returns `columns`, `rows` (objects, up to 1,000), `row_count` and sheetgen's notes in `stderr`.

### `leanqc`

Read QC ([leanqc](leanqc.md)).

| Parameter | Type | Meaning |
|---|---|---|
| `files` | list of files (required) | FASTQ/FASTA, plain or compressed. |
| `outdir` | folder | Default `leanqc`. Inside the root. |
| `overwrite` | boolean | Allow an `outdir` that already has files. Default `false`. |
| `threads` | 1-64 | Files processed at once. |
| `dup_distinct` | integer | Distinct sequences tracked for duplication. |
| `phred` | 33 \| 64 | Quality encoding instead of detecting it. |
| `no_html` | boolean | JSON and TSV only. |

Returns `summary` (files and counts per verdict) and per file: `verdict`, `fails` and `warns`
(check name and detail), `reads`, `mean_len`, `gc_percent`, `q30_percent`, encoding `warnings`,
and the JSON and HTML report paths. If leanqc exits 0 but no `*_leanqc.json` report appeared or
changed in `outdir`, the result is an error (`no_reports`) rather than an empty summary.

### `isotable`

One graded row per sample ([isotable](isotable.md)).

| Parameter | Type | Meaning |
|---|---|---|
| `dirs` | list of folders (required) | Results folders. |
| `sheet` | file | Sample sheet to join on (recommended). |
| `mode` | `isolate` \| `mag` | Grading rules. |
| `output`, `mqc` | files | Also write the TSV / MultiQC table (inside the root). |
| `overwrite` | boolean | Replace `output` / `mqc` if they exist. Default `false`. |
| `require` | list of `reads`, `q30`, `contigs`, `n50`, `completeness`, `contamination`, `kraken`, `gtdb`, `mlst`, `amr` | Required measurements; `[]` turns the check off. |
| `min_reads`, `max_contigs`, `min_n50` | integers | Thresholds. |
| `min_q30`, `min_busco`, `min_completeness`, `max_contamination` | 0-100 | Thresholds (%). |

Returns `counts` per status and `samples`, each with `status`, `reasons` (a list) and `values`
(the other columns).

### `dblock_*`

Reference databases ([dblock](dblock.md)). All take `project_dir` (default: the working folder)
and `catalog` (extra catalog JSON).

| Tool | Extra parameters | Returns |
|---|---|---|
| `dblock_catalog` | `tool` | `databases`: id, download, on_disk, description, licence_note |
| `dblock_add` | `ids` (required), `store` | dblock's message; dblock.toml in `outputs` |
| `dblock_plan` | | `plan` text: free space and fetch mode per database. Downloads nothing, but creates the (empty) store folder if missing, to measure its free space. |
| `dblock_fetch` | `confirm` (required, must be `true`), `ids`, `stream`, `keep_archive` | dblock's message; dblock.lock in `outputs` |
| `dblock_verify` | `deep` | `report` |
| `dblock_check` | | `report` |
| `dblock_env` | | `env` (variable → path) and `shell` (export lines) |

### Third-party tools

Four tools wrap programs irit doesn't ship. Each one is pinned to an upstream version: its
parameters are exactly the flags that version has, read from its source at that release tag.
Nothing else is offered, so an agent can't pass a flag the program doesn't know.

| MCP tool | Program | Pinned version | Flags read from | Bioconda |
|---|---|---|---|---|
| `fastp` | fastp | 1.4.0 | [`src/main.cpp`](https://github.com/OpenGene/fastp/blob/v1.4.0/src/main.cpp) | `fastp=1.4.0` |
| `bakta` | Bakta | 1.12.2 | [`bakta/utils.py`](https://github.com/oschwengers/bakta/blob/v1.12.2/bakta/utils.py), DB check from [`bakta/db.py`](https://github.com/oschwengers/bakta/blob/v1.12.2/bakta/db.py) | `bakta=1.12.2` |
| `checkm2` | CheckM2 `predict` | 1.1.0 | [`checkm2/main.py`](https://github.com/chklovski/CheckM2/blob/1.1.0/checkm2/main.py) | `checkm2=1.1.0` |
| `gtdbtk_classify_wf` | GTDB-Tk `classify_wf` | 2.8.0 | [`gtdbtk/cli.py`](https://github.com/Ecogenomics/GTDBTk/blob/2.8.0/gtdbtk/cli.py), data folders from [`config/common.py`](https://github.com/Ecogenomics/GTDBTk/blob/2.8.0/gtdbtk/config/common.py) | `gtdbtk=2.8.0` |

How they work:

- **Finding the program.** The server looks on `PATH`, or where `--tool-path NAME=PATH` says. If
  the program isn't there, the call fails with `tool_unavailable` and an install hint, e.g.
  `conda install -c conda-forge -c bioconda bakta=1.12.2`. Nothing is run.
- **Versions.** The first line of the program's `--version` goes into the run record. If the
  version isn't the pinned one, the call still runs, but its result has a `warnings` entry. Every
  result also has `version` and `checked_against`.
- **Checks before running.** Inputs must exist and look right: FASTQ for fastp, FASTA for Bakta,
  CheckM2 and GTDB-Tk (plain or gzip; bzip2, xz and zstd are refused, and so is an empty file).
  Databases are checked for the files the program itself needs (below), so a wrong path fails in
  a second instead of after an hour.
- **Databases.** Pass the path, or leave it out to use the variable the program reads (`BAKTA_DB`,
  `CHECKM2DB`, `GTDBTK_DATA_PATH`, as set when the server started). The server resolves it before
  running and records it. `dblock_env` shows the paths of databases that dblock manages.
- **Failure.** A non-zero exit is `tool_failed`, as for the irit tools. An exit status of 0
  without the files the program always writes is `missing_outputs`: it is recorded as a failed
  call and `replay.sh` doesn't rerun it.
- **Results.** Each result has a small summary parsed from the outputs (below). fastp, CheckM2 and
  GTDB-Tk outputs are read with isotable's own parsers. If the summary can't be read, the result
  is an error (`output_unreadable`), not an empty list.
- **Time.** Bakta, CheckM2 and GTDB-Tk can run for minutes to hours. The call returns when the
  program ends; an MCP client with a short request timeout may give up first.

The large databases are not hashed into the run record (they are tens of GB). Only their small
version files are: Bakta's `version.json`, GTDB-Tk's `metadata/metadata.txt`. Use `dblock_verify`
to check a database itself.

#### `fastp`

Trims and filters reads, and writes a JSON and an HTML report.

| Parameter | Type | Meaning |
|---|---|---|
| `in1` | file (required) | Read 1 FASTQ. |
| `in2` | file | Read 2, for paired-end data. |
| `out1`, `out2` | files | Trimmed reads. `out2` needs `in2` and `out1`; paired input with `out1` needs `out2`. |
| `json` | file (required) | JSON report. Name it `<sample>.fastp.json` so isotable finds it. |
| `html` | file | HTML report. Default: `json` with `.html`. |
| `overwrite` | boolean | Replace existing outputs. Default `false`. |
| `thread` | 1-16 | Worker threads. |
| `compression` | 1-9 | gzip level for `.gz` outputs. |
| `detect_adapter_for_pe`, `correction` | booleans | Paired-end only. |
| `adapter_sequence`, `adapter_sequence_r2` | A/C/G/T/N | Adapters, instead of detecting them. |
| `disable_adapter_trimming`, `trim_poly_g`, `disable_trim_poly_g`, `cut_front`, `cut_tail`, `cut_right`, `dedup`, `overrepresentation_analysis`, `phred64` | booleans | As the fastp flags. |
| `trim_front1`, `trim_tail1`, `cut_window_size` (1-1000), `cut_mean_quality` (1-36), `qualified_quality_phred`, `unqualified_percent_limit` (0-100), `n_base_limit`, `average_qual`, `length_required` | integers | As the fastp options. |
| `report_title` | text | HTML report title. |

Returns `report` (JSON and HTML paths) and `summary`: `reads_raw`, `reads_trimmed`,
`bases_trimmed`, `q30_trimmed` (%), `q30_bases` and `duplication` (%).

Not offered: `--stdin`, `--stdout`, `--interleaved_in`, `--unpaired1/2`, splitting, UMI and
merging options, and `--dont_overwrite` (the server enforces `overwrite` itself).

#### `bakta`

Annotates one genome assembly.

| Parameter | Type | Meaning |
|---|---|---|
| `genome` | file (required) | Assembly, FASTA. |
| `db` | folder | Bakta database (with `version.json`). Default `$BAKTA_DB`. |
| `outdir` | folder (required) | Output folder. |
| `overwrite` | boolean | Allow an `outdir` that has files. Default `false`. |
| `prefix` | name | Output file prefix. Default: the genome file name without its last extension, as Bakta does. |
| `genus`, `species`, `strain`, `plasmid` | text | Organism. |
| `translation_table` | 11, 4 or 25 | |
| `gram` | `positive`, `negative`, `unknown` | Sent as `+`, `-`, `?`. |
| `locus`, `locus_tag` | text | Checked with Bakta's own rules. |
| `min_contig_length`, `threads` | integers | |
| `complete`, `keep_contig_headers`, `compliant`, `meta` | booleans | As the Bakta flags. |
| `skip` | list of `trna`, `tmrna`, `rrna`, `ncrna`, `ncrna_region`, `crispr`, `cds`, `pseudo`, `sorf`, `gap`, `ori`, `filter`, `plot` | Sent as `--skip-<step>`. |

The database check follows Bakta 1.12.2's own: `version.json` with `date`, `major`, `minor` and
`type`; major version 6; and all the files Bakta opens, including `psc.dmnd` (full) or
`pscc.dmnd` (light). dblock's `bakta-db-v6-full` and `bakta-db-v6-light` pass.

The server always sends `--force` (Bakta refuses an existing folder without it), and always sends
`--prefix`, so the output names are known. Returns `summary_file` and `summary`, the numbers from
`<prefix>.txt`: `sequences` (length, count, gc, n50, ...), `annotation` (`cdss`, `trnas`, `rrnas`,
`hypotheticals`, ...) and `bakta` (software and database version).

Not offered: `--proteins`, `--hmms`, `--regions`, `--replicons`, `--prodigal-tf`, `--tmp-dir`,
`--debug`, `--verbose`.

#### `checkm2`

Completeness and contamination of genomes or MAGs (`checkm2 predict`).

| Parameter | Type | Meaning |
|---|---|---|
| `genomes` | list of files | FASTA files. Give this or `genome_dir`. |
| `genome_dir` | folder | Folder of FASTA files. |
| `extension` | text | With `genome_dir`: which files. Default `.fna`. |
| `outdir` | folder (required) | Output folder. |
| `overwrite` | boolean | Allow an `outdir` that has files. CheckM2 then **deletes** them first. |
| `db` | file or folder | The DIAMOND `.dmnd` file, or a folder with exactly one. Default `$CHECKM2DB`. |
| `threads` | integer | |
| `model` | `auto`, `general`, `specific`, `both` | `both` is `--allmodels`. |
| `lowmem` | boolean | Less RAM, slower. Use on 8-16 GB machines. |
| `ttable` | integer | Translation table, from the list CheckM2 accepts. |
| `genes`, `remove_intermediates` | booleans | As the CheckM2 flags. |

The server always sends `--force`, since CheckM2 refuses a non-empty folder without it. Returns
`report` (the `quality_report.tsv` path) and `genomes`: `genome`, `completeness`, `contamination`.

#### `gtdbtk_classify_wf`

GTDB taxonomy for the genomes in a folder.

| Parameter | Type | Meaning |
|---|---|---|
| `genome_dir` | folder (required) | One genome per file. |
| `extension` | text | Default `fna`; use `fna.gz` for gzipped files. |
| `out_dir` | folder (required) | Output folder. |
| `overwrite` | boolean | Allow an `out_dir` that has files. |
| `data_path` | folder | GTDB-Tk reference data (the release folder). Default `$GTDBTK_DATA_PATH`. Sent to GTDB-Tk as `GTDBTK_DATA_PATH` and recorded in `env`. |
| `prefix` | name | Default `gtdbtk`. |
| `cpus`, `pplacer_cpus` | integers | Use `pplacer_cpus: 1` on a laptop. |
| `scratch_dir` | folder | pplacer writes a memory-mapped file here: much less RAM, slower. |
| `full_tree`, `place_species`, `keep_intermediates`, `write_single_copy_genes`, `genes` | booleans | As the GTDB-Tk flags. |
| `force` | boolean | GTDB-Tk's `--force`: carry on when one genome fails. It does not mean overwrite. |
| `min_af` | 0-1 | Minimum alignment fraction for a species match. |
| `min_perc_aa` | 0-100 | Minimum % of amino acids in the MSA. |

The data check reads `metadata/metadata.txt` (it must have `VERSION_DATA`) and wants the folders
classify_wf reads: `markers`, `masks`, `metadata`, `mrca_red`, `pplacer`, `radii`, `split`,
`taxonomy` and `skani`.

Returns `summary_files` and `genomes`: `genome`, `gtdb` (classification) and, when GTDB-Tk filled
them, `classification_method`, `closest_genome_reference`, `closest_genome_ani`, `msa_percent`,
`warnings` and `note`. Only summaries written by this call are read, so a stale `ar53` file from an
earlier run is ignored.

Laptop options in GTDB-Tk 2.8.0, checked against [the research notes](../research/gtdbtk-memory.md):

- `--scratch_dir` and `--pplacer_cpus` exist, and are offered.
- **No `--skip_ani_screen`.** Since 2.7, classify_wf always runs the skani ANI screen first, and no
  command-line option skips it. So the placement-only data (dblock `gtdbtk-r232-placement`, no
  `skani/`) can't run classify_wf: the server refuses it and points to the launcher in
  [the placement test](../research/gtdb-placement-test.md).
- **No `--max_memory`.** That is a gPlace option. GTDB-Tk calls pplacer by name, so gPlace is used
  through a `pplacer` shim on `PATH` (see the same page), not through a GTDB-Tk flag.
- `--mash_db` went away in GTDB-Tk 2.5.0 (skani replaced Mash) and isn't offered.

#### Example: an isolate from reads to taxonomy

A session an agent might run (paths shortened):

```json
{"name": "fastp", "arguments": {"in1": "reads/s1_R1.fastq.gz", "in2": "reads/s1_R2.fastq.gz",
  "out1": "trim/s1_R1.fastq.gz", "out2": "trim/s1_R2.fastq.gz", "json": "trim/s1.fastp.json"}}
```

Then assemble (outside irit-mcp for now), then:

```json
{"name": "dblock_env", "arguments": {}}
{"name": "bakta", "arguments": {"genome": "asm/s1.fna", "outdir": "bakta/s1", "skip": ["plot"]}}
{"name": "checkm2", "arguments": {"genome_dir": "asm", "outdir": "checkm2", "lowmem": true}}
{"name": "gtdbtk_classify_wf", "arguments": {"genome_dir": "asm", "out_dir": "gtdbtk",
  "pplacer_cpus": 1, "scratch_dir": "scratch"}}
{"name": "isotable", "arguments": {"dirs": ["trim", "checkm2", "gtdbtk"]}}
```

The run's `replay.sh` then has lines like these:

```sh
want_ext_version fastp 'fastp 1.4.0'
want_ext_version gtdbtk 'gtdbtk: version 2.8.0 Copyright 2017 Pierre-Alain Chaumeil, ...'

# call 001: fastp (ok, 2026-11-10T09:12:03.120Z)
cd /data/demo || exit 1
step 001 0 ext fastp --in1=reads/s1_R1.fastq.gz --in2=reads/s1_R2.fastq.gz ...

# call 005: gtdbtk_classify_wf (ok, 2026-11-10T09:58:40.002Z)
cd /data/demo || exit 1
GTDBTK_DATA_PATH=/home/u/.local/share/dblock/gtdbtk-r232/release232; export GTDBTK_DATA_PATH
step 005 0 ext gtdbtk classify_wf --genome_dir=asm --out_dir=gtdbtk --pplacer_cpus=1 --scratch_dir=scratch
```

### `describe_run`, `list_runs`

`describe_run` (`run_id`, default the current run) returns the whole call log plus the paths of
`replay.sh` and the RO-Crate. `list_runs` lists the runs in the runs folder with call counts.

### Errors

Every tool result carries `call_id` and `run_id`. Failures have `isError: true` and an `error`
field:

| `error` | Meaning |
|---|---|
| `invalid_parameter` | Refused before running; `field` and `message` say why. Recorded, not replayed. |
| `tool_failed` | The tool ran and exited non-zero; `exit_status`, `stderr`, `stdout`. Its outputs must not be used. |
| `tool_unavailable` | The binary was not found or could not start. |
| `no_reports` | leanqc exited 0 but wrote no report; there is no QC result. |
| `missing_outputs` | A third-party tool exited 0 but didn't write the files it always writes; `message` lists them. Recorded as failed. |
| `output_unreadable` | sheetgen, isotable or a third-party tool exited 0 but its output could not be read back. |

A parameter that isn't in the schema is refused by the MCP layer itself, with the list of valid
names.

## Run records

One folder per server session, `irit-runs/<run-id>/`, rewritten after every call:

```
irit-runs/20261110T074754Z-41022/
  run.json                  every call, in order
  replay.sh                 re-runs the successful calls and checks the outputs
  ro-crate-metadata.json    the same, as a Workflow Run RO-Crate
  calls/001.stdout          each call's stdout and stderr
  calls/001.stderr
```

A call in `run.json` (from a real session, with paths shortened and lists trimmed):

```json
{
  "id": 2,
  "mcp_tool": "leanqc",
  "status": "ok",
  "params": {"files": ["reads/s1_R1.fastq", "reads/s1_R2.fastq"], "outdir": "qc", "no_html": true},
  "program": "/opt/irit/bin/leanqc",
  "argv": ["leanqc", "--outdir=qc", "--no-html", "--", "reads/s1_R1.fastq", "reads/s1_R2.fastq"],
  "version": "leanqc 0.1.0",
  "cwd": "/data/demo",
  "started": "2026-11-10T07:47:54.881Z",
  "ended": "2026-11-10T07:47:54.900Z",
  "exit_status": 0,
  "inputs": [
    {"path": "/data/demo/reads/s1_R1.fastq", "size": 21890, "sha256": "0d434ae6..."}
  ],
  "outputs": [
    {"path": "/data/demo/qc/s1_R1_leanqc.json", "size": 12991, "sha256": "fbc19cfa..."}
  ],
  "stdout": "calls/002.stdout",
  "stderr": "calls/002.stderr",
  "env": {"DBLOCK_STORE": "/home/u/.local/share/dblock"}
}
```

`version` is the first line the tool printed for `--version`; it is left out when `--version` failed
or printed nothing. Third-party calls also have `"external": true`.

`status` is `ok`, `failed` (non-zero exit), `rejected` (invalid parameters, with `error`) or
`error` (binary missing). Inputs are the files the call reads; a folder input (sheetgen, isotable)
is listed file by file. Outputs are the files under the call's output paths that are new or changed
after it ran. "Changed" compares size, modification time, inode number and inode change time (on
Unix), and the SHA-256 of files under 1 MB, so a rewrite with the same size inside the same
timestamp tick is still seen. An output path that is itself a symlink (`qc -> /scratch/qc`) is
followed; symlinks inside it (sheetgen's `link_into`) are recorded with `symlink_to`.

Calls whose output paths overlap (the same path, or one inside the other) run one after the other,
so each call's outputs are only its own; other calls run in parallel.

#### Environment

`env` holds the environment that decided where the tool looked or wrote, as the resolved value the
server computed with the tool's own rules. The server passes exactly these values to the tool, so
the record and the run agree, and `replay.sh` exports them before the call:

| Tool | Recorded | Resolved from |
|---|---|---|
| `dblock_*` (except catalog) | `DBLOCK_STORE` | `$DBLOCK_STORE`, else `$XDG_DATA_HOME/dblock`, else `$HOME/.local/share/dblock` |
| `sheetgen` with `pipeline` | `NXF_HOME` | `$NXF_HOME`, else `$HOME/.nextflow` |
| `gtdbtk_classify_wf` | `GTDBTK_DATA_PATH` | `data_path`, else `$GTDBTK_DATA_PATH` |

Bakta and CheckM2 get their database as a flag (`--db`, `--database_path`), resolved the same way
from `BAKTA_DB` and `CHECKM2DB`, so it is in `argv`.

With `pipeline`, the `schema_input.json` sheetgen will use (same lookup as sheetgen: the
`assets/<name>` checkout, else the newest clone) is also hashed into `inputs`.

Not captured: `PATH` and other variables, locale, the user, the machine; a `store =` line in
`dblock.toml` (it wins over `DBLOCK_STORE`, and dblock.toml is hashed as an input instead); `~/`
in such a line, which dblock expands with the replaying user's `HOME`; the contents of the dblock
store and of the Nextflow cache beyond the one schema file.

Options are always written as `--name=value` and positional arguments after `--`, so a file name
starting with `-` can't be read as a flag.

### Replaying

```sh
sh irit-runs/<run-id>/replay.sh                            # binaries from PATH
IRIT_BIN_DIR=/opt/irit/bin sh irit-runs/<run-id>/replay.sh # or from a folder
```

Third-party tools (fastp, bakta, checkm2, gtdbtk) are always taken from `PATH`, so activate their
conda environment first. `IRIT_BIN_DIR` applies to the irit tools only.

Some third-party outputs differ between two identical runs. They are recorded with
`"volatile": true` and their hash, and `replay.sh` only checks that they exist (`check_exists`).
Every other output is checked by hash. The lists come from running each tool twice with the same
inputs and paths in the real-tools workflow (below) and comparing every file:

| Tool | Volatile outputs | Why they differ |
|---|---|---|
| fastp | `.html` | The HTML report; the JSON report is identical. |
| bakta | `.log`, `.gbff`, `.embl`, `.json`, `.svg` | Run times and dates (GenBank `LOCUS` date; in the JSON only `run.start` and `run.end`). |
| checkm2 | `checkm2.log` | Timestamps. |
| gtdbtk | `.log`, `gtdbtk.json`, `.gz` | Timestamps (in `gtdbtk.json` only each step's start and end); gzip headers carry a time. |

The script warns when a tool's `--version` (its first line) differs from the recorded one (a tool whose version
couldn't be read in the run gets one "version unknown" line instead), exports each call's recorded
`env`, re-runs each successful call in its original folder (stdout and stderr go to `replay/NNN.*` beside the script), then checks
the SHA-256 of every output (the last version, if several calls wrote the same file), and of stdout
where stdout is the result (a sheet or table not written to a file, `dblock catalog`, `dblock env`).
It exits 0 only if every command exits as it did and every hash matches. Failed and rejected calls
appear as comments.

A successful `dblock_fetch` is not repeated by default, since it would download the databases
again: the script prints that it skipped it, the exact command is in a comment, and the check of the
`dblock.lock` it wrote is skipped too. Run with `IRIT_REPLAY_FETCH=1` to fetch again:

```sh
IRIT_REPLAY_FETCH=1 sh irit-runs/<run-id>/replay.sh
```

Everything taken from a call (arguments, paths, versions, env values) is single-quoted, and every
comment line is written through one helper that escapes newlines, carriage returns and other
control characters, so a value can't end a comment and start a command. The server also refuses
control characters in every path and text parameter, so they never reach a tool's argv. An
excerpt:

```sh
want_version leanqc 'leanqc 0.1.0'

# call 002: leanqc (ok, 2026-11-10T07:47:54.881Z)
cd /data/demo || exit 1
step 002 0 irit leanqc --outdir=qc --no-html -- reads/s1_R1.fastq reads/s1_R2.fastq

# call 005: leanqc (rejected, 2026-11-10T07:47:54.946Z)
# not run: phred: 40 is not 33 or 64

# outputs
check fbc19cfa4e3a31daabbaec0ffda5e39ae9614937132f8f9e7cabfcdfc8d7a735 /data/demo/qc/s1_R1_leanqc.json
```

### RO-Crate

`ro-crate-metadata.json` follows the [Process Run Crate](https://w3id.org/ro/wfrun/process/0.6)
profile 0.6 (RO-Crate 1.3): one `CreateAction` per call that ran, with the tool as a
`SoftwareApplication` (`softwareVersion`), inputs as `object`, outputs as `result`, start and end
times, and `FailedActionStatus` for failed calls. The root Dataset has a `description` and a
`license` (CC0 1.0, for the run record itself; the data files keep their own terms). `run.json`,
`replay.sh` and every `calls/NNN.stdout` / `.stderr` are listed in `hasPart`.
The data files are not copied in; they are referenced by `file://` URI, so the crate describes the
run but does not carry the data.

## Safety

- **No shell.** The binaries are started with an argument vector; nothing passes through a
  shell. `replay.sh` single-quotes every value and escapes comments (see Replaying); control
  characters are refused in parameters.
- **Writes stay in the root** and don't replace existing files without `overwrite: true` (see
  [Where tools may write](#where-tools-may-write)).
- **Downloads need confirmation.** `dblock_fetch` refuses unless `confirm: true`, and its
  description tells the agent to show the user `dblock_plan` first. dblock itself still refuses
  databases that won't fit on disk.
- **No deleting tools.** `dblock remove` is not exposed.
- **Stdin is closed** for every tool, so nothing can wait for input.

## Tested

- Unit tests: parameter checks and argument vectors for every tool (missing paths, bad enums,
  out-of-range numbers, `schema` with `pipeline`, unconfirmed fetch, unsafe database ids, unknown
  parameters, control characters, writes outside the root through `..`, absolute paths and
  symlinks, overwriting), resolved environment, shell quoting, `replay.sh` generation, output
  detection (symlinked output folders, same-size rewrites), serialising overlapping calls,
  `run.json` and RO-Crate writing, and parsing of the tables and dblock output.
- A `replay.sh` built from hostile values (newlines, carriage returns, quotes, `$(...)`, backticks,
  leading dashes) in every field it is generated from passes `sh -n`, and running it with stub
  tools creates nothing but its own `replay/` folder.
- An integration test starts `irit-mcp` over stdio, speaks raw JSON-RPC (initialize, tools/list,
  tools/call), runs `leanqc` on a generated FASTQ pair, checks the run record against the files on
  disk, deletes the reports, runs `replay.sh` and gets the same SHA-256 for every output. It then
  changes an input and checks that `replay.sh` fails and names the file that differs. A second one
  replays the review's cases: leanqc into a symlinked `outdir`, a second leanqc refused without
  `overwrite`, sheetgen refused for a file outside the root and for a newline in `set`, and the
  replay of that session. No network.
- Third-party tools: unit tests map parameters to argv for each tool and check the refusals
  (wrong file type, gzip and empty inputs, contradictory flags, out-of-range values, database
  folders missing a file or of the wrong version, GTDB-Tk data without `skani/`, flags that don't
  exist in the pinned version such as `skip_ani_screen` and `max_memory`). An integration test
  (Unix only) puts shell-script stand-ins for fastp, bakta, checkm2 and gtdbtk on `PATH`. Each
  stand-in refuses any flag the wrapper isn't allowed to send. The test runs the isolate path
  over stdio, checks the summaries, a failed run, an exit 0 without outputs, a version warning and
  the run record, then deletes the outputs, replays and gets the same hashes. A second test checks
  the install hint when a tool is missing.
- With the real programs: the opt-in workflow
  [`mcp-real-tools.yml`](https://github.com/dzulfikar08/irit/blob/main/.github/workflows/mcp-real-tools.yml)
  installs fastp 1.4.0, Bakta 1.12.2, CheckM2 1.1.0 and GTDB-Tk 2.8.0 from Bioconda (one
  environment each), fetches the Bakta light and CheckM2 databases through `dblock_add` and
  `dblock_fetch`, and runs the four tools through irit-mcp on *Mycoplasma genitalium* G37 (reads
  simulated from it for fastp) and on GTDB's mock database and test genome for GTDB-Tk. It runs
  each tool twice to find the volatile outputs, deletes the outputs and runs `replay.sh`. It only
  runs by hand: `gh workflow run mcp-real-tools.yml`. First result (2026-11-10): all calls
  succeeded; fastp 40,000 reads, Bakta 522 CDSs / 36 tRNAs / 3 rRNAs, CheckM2 98.9% complete and
  0.08% contaminated, GTDB-Tk classified the mock test genome with ANI 100. `replay.sh` then re-ran 11 commands and all
  41 outputs matched ([run](https://github.com/dzulfikar08/irit/actions/runs/42850479255)).
- By hand: a session with sheetgen → leanqc → isotable → dblock_catalog replayed with all 7 outputs
  matching; dblock_add, plan, verify, env and check against a local project.

## Known gaps

- `dblock_fetch` has not been run through the server against the real servers (dblock's own tests
  cover fetching).
- An unknown parameter is refused by the MCP SDK before the server sees the call, so that refusal
  isn't in the run record (refusals of known parameters are).
- Inputs are hashed in full before each call, so a call on large FASTQ folders reads them twice.
- `replay.sh` writes to the original paths; there is no option to replay into another folder.
- The root check runs before the tool starts; a symlink swapped in between could still redirect a
  write (the server doesn't sandbox the tools).
- Calls with overlapping outputs wait for each other, but a call may still read files another call
  is writing.
- The RO-Crate does not include the data, and has no `agent` (who ran it) yet.
- No streamable HTTP transport.
- The real programs are only checked by the opt-in workflow, on a small genome and GTDB's mock
  database: GTDB-Tk has not been run through irit-mcp with the full R232 data.
- Bakta: the AMRFinderPlus data inside the Bakta v6 databases (`amrfinderplus-db/`) may be older
  than the `amrfinder` Bioconda installs with Bakta 1.12.2, and Bakta then fails with "amrfinder
  error". Run `amrfinder_update --force_update --database <BAKTA_DB>/amrfinderplus-db` once (as
  Bakta's message says). This changes the dblock-managed folder, so `dblock_verify` will then
  report it as changed.
- CheckM2 1.1.0 from Bioconda can fail to load TensorFlow on newer glibc ("cannot enable
  executable stack"). Setting `GLIBC_TUNABLES=glibc.rtld.execstack=2` for CheckM2 works around it
  (the workflow does this in a wrapper script).
- No wrapper yet for an assembler, mlst or AMRFinderPlus, so the isolate path still has a step
  outside the record.
- A long Bakta or GTDB-Tk call blocks until it ends; there is no progress reporting or
  cancellation beyond the client closing the connection.
