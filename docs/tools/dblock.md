# dblock: reference databases

`dblock` gets reference databases onto disk and records exactly which ones an analysis used. A
project lists what it needs in `dblock.toml`; `dblock fetch` checks free space, downloads (resuming
if interrupted), verifies the publisher's checksum, unpacks into a shared store, and writes
`dblock.lock`.

## Why

Each tool has its own downloader, none record what they fetched, and mismatches are silent: a
newer Kraken 2 taxonomy with an older Kraken 2 gives empty Bracken output, Bakta accepts only one
database schema per release, and checksum failures happen. On a laptop the bigger problem is space:
GTDB-Tk's data is a 61 GB download that unpacks to about 100 GB.

## Usage

```sh
dblock catalog                     # what's available, with sizes and licence flags
dblock add checkm2-db-v3 kraken2-standard-8gb-20261022
dblock plan                        # will it fit, and how will it be fetched?
dblock fetch                       # download, verify, unpack; writes dblock.lock
dblock verify                      # are the files still exactly as locked?
dblock check                       # do installed tool versions accept these databases?
eval "$(dblock env)"               # export GTDBTK_DATA_PATH, CHECKM2DB, BAKTA_DB ...
dblock remove card-4.0.2           # drop from the project (add --delete-data to delete files)
```

Global options: `-C DIR` (project folder), `--catalog FILE` (extra or overriding catalog entries,
same JSON format as the built-in one).

## The store

Databases live in one store shared by all projects: `store = "..."` in `dblock.toml`, else
`$DBLOCK_STORE`, else `~/.local/share/dblock`. Each database gets a folder named after its id with a
`.dblock.toml` marker, so a second project that wants the same database reuses it instantly.

## How fetching works

`dblock plan` and `dblock fetch` choose one of two modes per database:

- **Download, then unpack** (default when there's room for both): the archive is saved under
  `.partial/`, resumable after a broken connection (the bytes already on disk are re-hashed so the
  checksum still covers the whole file), verified, then unpacked.
- **Stream** (automatic when only the unpacked size fits, or `--stream`): unpacking happens while
  downloading and the archive never touches the disk. Not resumable. The checksum is still computed
  over the whole stream, and nothing is kept if it doesn't match.

If neither fits, it stops before downloading and says how much space is needed. A 2 GB reserve is
always kept free.

Either way, files are unpacked into a temporary folder that takes its final name only after the
checksum matches, so a half-finished or corrupt database is never mistaken for a good one. Archive
entries that would land outside that folder (absolute paths, `..`) are refused. A checksum mismatch
is never retried against a mirror: a mirror serving the same bad file wouldn't fix it.

## The lock file

```toml
[[database]]
id = "kraken2-viral-20261022"
version = "2026-10-22"
url = "https://genome-idx.s3.amazonaws.com/kraken/k2_viral_20261022.tar.gz"
archive_md5 = "2c8c97686c45474d1d82cf940b4d9893"
fetched = "2026-11-10T06:01:23Z"
data_path = ".../kraken2-viral-20261022"
files = 16
bytes = 1230959768
manifest_md5 = "5ba2e1f3f4d99b3b0a0ff77b16dd98bc"
```

`manifest_md5` fingerprints the unpacked folder (every relative path and size), so `dblock verify`
notices files added, removed or truncated without re-reading 100 GB. Commit `dblock.lock` with the
analysis.

## Catalog

Every URL, size and checksum in the built-in catalog was checked against the publisher on
2026-11-10 (HTTP 200, `Content-Length`, published MD5 files or the Zenodo API).

| id | Download | Unpacked | Notes |
|---|---|---|---|
| `gtdbtk-r232` | 60.8 GB | ~100 GB | GTDB-Tk ≥ 2.7.0 (2.7.1+ recommended). Needs ~140 GB RAM to run: see [GTDB-Tk and memory](../research/gtdbtk-memory.md). |
| `gtdbtk-r232-mockup` | 26 MB | ~120 MB | Test database. Currently refused: the served file doesn't match GTDB's checksum ([GTDBTk#726](https://github.com/Ecogenomics/GTDBTk/issues/726)). |
| `checkm2-db-v3` | 1.7 GB | ~3.5 GB | CheckM2 ≥ 1.1.0 only. |
| `silva-144-qiime2-full` | 274 MB | same | QIIME 2 ≥ 2026.4. |
| `silva-144-qiime2-v4` | 79 MB | same | 515F/806R amplicons only. |
| `kraken2-standard-8gb-20261022` | 5.9 GB | 7.5 GB | Runs in 8 GB RAM. |
| `kraken2-standard-16gb-20261022` | 12.0 GB | 14.9 GB | |
| `kraken2-pluspf-8gb-20261022` | 5.9 GB | 7.5 GB | Adds protozoa and fungi. |
| `kraken2-viral-20261022` | 590 MB | 1.23 GB | Good for tests. |
| `bakta-db-v6-full` | 31.9 GB | 84 GB | Bakta 1.11–1.x (schema 6). Run `amrfinder_update` after. |
| `bakta-db-v6-light` | 1.3 GB | 3.9 GB | Same. |
| `card-4.0.2` | 4 MB | 50 MB | **Non-commercial licence**; checksum computed by us (CARD publishes none). |

Not in the catalog yet: the AMRFinderPlus database (a folder of files without checksums; use
`amrfinder_update`), GTDB-Tk's 12-part split download.

## Tested

Against the real servers on 2026-11-10:

| Test | Result |
|---|---|
| `plan` with GTDB-Tk on a disk with 23 GB free | Refused before downloading: needs ~100 GB |
| Fetch Kraken 2 viral, CARD, SILVA V4 | All checksums matched; lock file written |
| `--stream` with `.tar.gz` (GTDB mock) and `.tar.bz2` (CARD) | Unpacked while downloading, checksum ok, no archive on disk |
| Download killed after 6 s, then fetch again | Resumed at 783 kB; final checksum ok |
| Catalog entry with a wrong checksum | Rejected, nothing kept |
| `verify`, `env`, `check` | As expected |
| GTDB mock database with GTDB's published checksum | **Rejected: real upstream mismatch** (see [report](../research/upstream-reports.md)) |

Measured: Kraken 2 viral unpacks to 1.23 GB, not the 0.6 GB its index page lists; the catalog uses
the measured figure.

## Known gaps

- `verify` checks names and sizes, not contents. A deep mode that hashes every file is planned.
- Tool version rules exist only where the publisher states one (none for Kraken 2 or CARD).
- No parallel downloads; one database at a time.
