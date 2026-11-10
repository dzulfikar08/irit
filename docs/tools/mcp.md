# irit-mcp: tools for AI agents

`irit-mcp` is a [Model Context Protocol](https://modelcontextprotocol.io) server that gives AI
agents (Claude Code, Claude Desktop and other MCP clients) the irit tools as typed tools. Each call
runs the real `sheetgen`, `leanqc`, `isotable` or `dblock` binary, and is written to a run record
with a `replay.sh` that repeats the same commands without any AI.

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

The server is a thin layer. It runs the binaries as subprocesses and does not link their code, so
the CLI stays the single source of truth for what each tool does.

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
| `--runs-dir DIR` | Where run records go (env `IRIT_RUNS_DIR`). Default `./irit-runs`. |
| `--run-id ID` | Name of this session's run folder. Default `<UTC time>-<pid>`. |
| `--root DIR` | Folder the tools may write in. Default: the server's working folder. |
| `--allow-outside-root` | Turn the root check off, so tools can write anywhere the server's user can. Not recommended. |

Tools run in the server's working folder, and relative paths in parameters are resolved there.

### Where tools may write

Every path a tool would write to must resolve inside the root: sheetgen `output` and `link_into`,
leanqc `outdir`, isotable `output` and `mqc`, `dblock.toml` and `dblock.lock` in the `project_dir`
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
| `output_unreadable` | sheetgen or isotable exited 0 but the `output` file could not be read back. |

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
or printed nothing.

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

The script warns when a tool's `--version` differs from the recorded one (a tool whose version
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
- No streamable HTTP transport, and no MCP wrappers yet for the third-party tools the
  [landscape](../research/landscape.md#3-ai-agents-for-bioinformatics) lists (fastp, Bakta,
  CheckM2, GTDB-Tk).
