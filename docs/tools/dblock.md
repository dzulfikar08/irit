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
dblock verify                      # same file names and sizes as locked? (seconds)
dblock verify --deep               # same contents? re-hashes every file (minutes to hours)
dblock check                       # do installed tool versions accept these databases?
eval "$(dblock env)"               # export GTDBTK_DATA_PATH, CHECKM2DB, BAKTA_DB ...
dblock remove card-4.0.2           # drop from the project; files stay in the store
dblock remove card-4.0.2 --delete-data --force   # also delete the folder from the shared store
```

Global options: `-C DIR` (project folder), `--catalog FILE` (extra or overriding catalog entries,
same JSON format as the built-in one).

## The store

Databases live in one store shared by all projects: `store = "..."` in `dblock.toml`, else
`$DBLOCK_STORE`, else `~/.local/share/dblock`. Each database gets a folder named after its id with a
`.dblock.toml` marker, so a second project that wants the same database reuses it instantly.
Before reusing a folder, dblock checks its file names and sizes against the marker; a folder that
changed, has no marker, or holds a different release is reported and left alone, never reused or
overwritten.

Catalog ids become folder names, so they may only use `A-Z a-z 0-9 . _ -`, must not start with `.`
and must not contain `..`. MD5s are accepted in either case.

Two `dblock fetch` runs (from two projects, or two terminals) can share a store safely: each
database has a lock file, `<store>/.lock-<id>`, held while it is planned, downloaded and moved into
place. The second run waits, then reuses what the first one fetched. `remove --delete-data` takes
the same lock before deleting and checks the folder again once it has it, so it never deletes a
folder while a fetch is moving it into place, nor one that was replaced while it waited. The lock
only serialises the two runs: dblock doesn't track which projects use a database, so a project that
already uses the folder loses it (which is why deleting needs `--force`). Empty temporary folders
(`<store>/.tmp/<id>/`) are removed after each fetch and delete.

`remove --delete-data` deletes only `<store>/<id>` itself, and only when it is a real folder with a
marker for that id; any other path written in a lock file is refused. Because the store is shared,
it prints what it would delete and stops unless `--force` is given.

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

Either way, files are unpacked into a uniquely named temporary folder under `<store>/.tmp/<id>/`
(one folder per database, so cleaning up after a crashed `gtdbtk-r232` fetch never touches an
unpack of `gtdbtk-r232-mockup` running at the same time). Once the checksum matches,
dblock records every file's size and SHA-256 (in `.dblock.sha256`; symlinks inside the database are
recorded with their target and not followed, so retargeting one shows up in `verify`) and writes the marker inside that
folder, and only then renames it to its final name. A half-finished or corrupt database is never
mistaken for a good one, and a crash can't leave a folder under the final name without its marker.
Hashing happens after unpacking, so it reads the unpacked data once more (a few minutes for
GTDB-Tk's 100 GB on an SSD). Archive
entries that would land outside that folder (absolute paths, `..`) are refused. A checksum mismatch
is never retried against a mirror: a mirror serving the same bad file wouldn't fix it.

### Formats

`tar.gz`, `tar.bz2`, `tar.xz`, `tar.zst`, or `file` (kept as downloaded). All archive formats work
in both modes. `tar.zst` archives made with a long window (`zstd --long=N`) decode without extra
settings: dblock raises libzstd's decoder window limit from its default 2^27 (128 MiB) to 2^31
(2^30 on 32-bit systems). The limit is only a ceiling; the decoder allocates the window the archive
declares.

### Multi-part entries (`parts`)

An entry can be several archives that unpack into one database folder:

```json
{
  "id": "gtdbtk-r232-placement-lite",
  "format": "tar.zst",
  "base_url": "https://github.com/dzulfikar08/irit-data/releases/download/gtdbtk-r232-placement-v1",
  "parts": [
    {"file": "gtdbtk-r232-core.tar.zst", "size": 21376819,
     "md5": "d68a374abc0946aaec5ad59606754b8d", "sha256": "419925c6..."},
    {"url": "https://elsewhere.example/other.tar.zst", "size": 1, "md5": "..."}
  ],
  ...
}
```

Each part gives `url`, or `file` (appended to `base_url`); `size`; `md5`; and optionally
`sha256`, which is then checked as well. The entry has no `urls`. Its `size` is the sum of the
parts, and its `md5` is the MD5 of the parts' `MD5SUMS` lines (`<md5>  <file>\n`, in catalog
order); both are filled in when left out, and refused if given and wrong. That combined MD5 is
what the marker and lock entry record as `archive_md5`, so a different set of parts is a
different release. The lock entry also lists every part (`[[database.part]]`: file, URL, bytes,
MD5, SHA-256).

The whole entry is one database: one folder, one lock, one marker, one manifest and SHA-256 list,
and `verify_dirs` checked once over the combined folder. In download mode every part is
downloaded and checked before any is unpacked. Each part resumes on its own, and a part that is
already complete is re-read from disk instead of downloaded again, so a fetch that fails on part 7
keeps parts 1–6 under `.partial/` for the next run. A part whose checksum doesn't match is
deleted. In stream mode the parts are streamed one after another into the same temporary folder.
Either way the folder takes its final name only after every part has passed, and the folder
hashes match; any failure keeps nothing in the store. `plan` counts the sum of the parts'
sizes, and `extracted` (or `kept_size`) is for the whole entry.

Parts are unpacked in order and may share folders, but a file may appear in only one part. A
read-only folder entry in one part would stop a later part from writing into it; the GTDB mirror
archives hold no folder entries.

### Keeping part of an archive (`keep`)

A catalog entry can list `keep`: paths inside the archive (e.g. `release232_mockup/markers`).
Only entries under them are written; everything else is read through and thrown away, so the
archive MD5 is still checked over the whole file, in both modes. Matching is by path component
(`markers` keeps `markers/x`, not `markers2/x`), and a leading `./` in the archive is ignored. The
manifest and `.dblock.sha256` cover only the kept files. A `keep` path that matches nothing fails
the fetch. `plan` and `catalog` use `kept_size` (bytes kept) when the entry gives it, else
`extracted`.

Folders are unpacked the way tar does it: their files first, then each folder's own entry (and
mode) deepest first, so a read-only folder in the archive (`markers/` at mode 0555) still gets its
files and keeps its mode. Links must stay inside the kept folders: a symlink whose target (resolved
from where the link sits) is outside every `keep` path, absolute, or above the archive root fails
the fetch, since it would be left dangling (and GTDB-Tk's `sha1_dir` would then fail on it). A hard
link to a file outside the kept folders fails the same way. Add the target's folder to `keep`.
When `keep` is set, every `verify_dirs` folder must be inside a `keep` path; the catalog is refused
otherwise.

### GTDB-Tk folder hashes (`verify_dirs`)

`verify_dirs` maps folders inside the database to the hash GTDB-Tk itself expects for them
(`REF_HASHES` in `gtdbtk/config/common.py`). dblock computes it the way GTDB-Tk 2.8.0's
`sha1_dir` (`gtdbtk/tools.py`) does: walk the folder as `os.walk` would (links to folders not
entered, links to files read through), sort the full file paths as strings, and take one SHA-1
over all file contents in that order (file names are not hashed). The test suite checks this port
against GTDB-Tk's own Python function when a GTDB-Tk environment is available.

The hashes are checked after unpacking, before the folder takes its final name: a mismatch fails
the fetch and nothing is kept. They are recorded in the marker and lock entry, and `verify --deep`
checks them again. Unlike the SHA-256 list, which dblock writes itself at fetch time, these come
from the publisher, so they also vouch for what was fetched.

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
contents_sha256 = "..."
manifest_format = 2
```

`manifest_md5` fingerprints the unpacked folder's file names and sizes, so `dblock verify` notices
files added, removed, truncated or grown in seconds, without re-reading 100 GB. It does **not**
notice an edit that keeps a file's size. `contents_sha256` is the SHA-256 of the per-file SHA-256
list taken at fetch time; `dblock verify --deep` re-hashes every file, compares, and names the files
that changed. Databases fetched by dblock 0.1 have no `contents_sha256`; `verify --deep` reports
them as unchecked until they are fetched again. A database that can't be read (a file without read
permission, say) is reported as `ERROR` with the reason, and the others are still checked; `verify`
exits non-zero if any database is missing, changed, unchecked or unreadable. Commit `dblock.lock` with the analysis.

`manifest_format = 2` means symlinks are part of both fingerprints. Markers and lock entries
without it come from dblock 0.1.0, which left symlinks out; dblock checks those the old way, so a
database with symlinks fetched by 0.1.0 is still reused and verifies as before. The first `dblock
fetch` that reuses such a database upgrades its marker (and its `.dblock.sha256` list, by adding
the symlink lines to the recorded sums, without re-hashing) and that project's lock entry; other
projects' lock entries keep verifying the old way until they run `dblock fetch`.

`dblock env` single-quotes paths for the shell (a `'` in a path is written as `'\''`).

## Catalog

Every URL, size and checksum in the built-in catalog was checked against the publisher on
2026-11-10 (HTTP 200, `Content-Length`, published MD5 files or the Zenodo API).

| id | Download | Unpacked | Notes |
|---|---|---|---|
| `gtdbtk-r232` | 60.8 GB | ~100 GB | GTDB-Tk ≥ 2.7.0 (2.7.1+ recommended). Needs ~140 GB RAM to run: see [GTDB-Tk and memory](../research/gtdbtk-memory.md). |
| `gtdbtk-r232-client` | 26 MB | 80 MB | Client data for [gtdbprep](gtdbprep.md): `markers/`, `masks/`, `metadata/`, `taxonomy/` from the mockup archive. **Can't classify.** MD5 observed, not published; see below. |
| `gtdbtk-r232-placement` | 280 MB (12 parts) | 7.4 GB | **Unofficial mirror** of GTDB-Tk's placement folders; no ANI database or MSAs, so classification stops at genus. See below. |
| `gtdbtk-r232-placement-lite` | 175 MB (11 parts) | 5.9 GB | Same without the full bacterial tree (no `--full_tree`). |
| `gtdbtk-r232-mockup` | 26 MB | ~120 MB | Test database. Currently refused: the served file doesn't match GTDB's checksum ([GTDBTk#726](https://github.com/Ecogenomics/GTDBTk/issues/726)). For gtdbprep use `gtdbtk-r232-client` (below). |
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

### `gtdbtk-r232-client` and its MD5

gtdbprep's laptop half runs only GTDB-Tk's `identify` and `align` stages, which read four folders
of GTDB's R232 mockup archive. This entry keeps just those (188 files, 79.8 MB) and checks
`markers/`, `masks/` and `metadata/` against GTDB-Tk 2.8.0's `REF_HASHES` for R232:

| Folder | `sha1_dir` |
|---|---|
| `markers/` | `163f542c3f0a40f59df45d453aa235b39aa96e27` |
| `masks/` | `84c7f7a17adc134db4161b15db10ae58389a90c1` |
| `metadata/` | `1febdba7d2513a8f43c409423aee83ed99df7a78` |

`taxonomy/` is the mockup's 5-line stub and has no `REF_HASHES` value; GTDB-Tk reads it only to
refuse genome IDs equal to GTDB accessions. The entry exports no `GTDBTK_DATA_PATH`, so it can't
shadow a full `gtdbtk-r232` in the same project: `dblock env` prints its path as a comment, and
you pass it to `gtdbprep prep --data`. `classify` and `classify_wf` need `gtdbtk-r232`.

GTDB's published MD5 for this archive (`2c710f35...`) does not match the file it serves
([GTDBTk#726](https://github.com/Ecogenomics/GTDBTk/issues/726)), which is why `gtdbtk-r232-mockup`
refuses it. This entry uses the MD5 we observed instead (`59512f60fa48010647977460bb855666`), and
its `md5_source` says so. That is acceptable here, and not for the mockup, because the MD5 is not
the check that matters: GTDB-Tk publishes the `REF_HASHES` values in its own source, independently
of the served file; they cover all but 1.7 kB of the kept bytes; and GTDB-Tk applies the same check
itself. The observed MD5 pins the exact file (so the lock records which one) and covers the
`taxonomy/` stub. If GTDB replaces the file, the MD5 stops matching and the fetch is refused until
the catalog is updated. To fetch a corrected file sooner, copy the built-in entry into a JSON file
(a one-element list), change `md5` and `md5_source`, and pass it with `--catalog`; an entry with
the same id there replaces the built-in one.

### `gtdbtk-r232-placement` and `-lite`: GTDB placement data from a mirror

GTDB-Tk's tree placement needs 7.4 GB of the 100 GB R232 package; the rest is the skani ANI
database (85 GB) and the reference MSAs (8 GB), see
[GTDB R232 placement in 8.5 GB](../research/gtdb-placement-test.md). GTDB serves the package only
as one 60.8 GB gzip stream, so these entries fetch the placement folders from an **unofficial
mirror**: [`dzulfikar08/irit-data`, release `gtdbtk-r232-placement-v1`](https://github.com/dzulfikar08/irit-data/releases/tag/gtdbtk-r232-placement-v1),
12 `.tar.zst` archives (zstd -19, `--long=27`) that unpack into one `release232/` folder.

The data is GTDB's, unmodified: **Genome Taxonomy Database (GTDB), https://gtdb.ecogenomic.org,
CC BY-SA 4.0**, taken from
[`gtdbtk_r232_data.tar.gz`](https://data.gtdb.ecogenomic.org/releases/release232/232.0/auxillary_files/gtdbtk_package/full_package/gtdbtk_r232_data.tar.gz).
The mirror is not run or endorsed by GTDB; cite GTDB and GTDB-Tk for results. The release also
has `SHA256SUMS`, `MD5SUMS`, `MANIFEST.tsv` (every file's SHA-256) and `FOLDER_HASHES.tsv`.

Trust: the parts' MD5 and SHA-256 come from the mirror, which GTDB doesn't publish for these
archives, so they only show that the download is the file the mirror published. What ties the
contents to GTDB is `verify_dirs`: `markers`, `masks`, `metadata`, `mrca_red`, `pplacer`, `radii`
and `taxonomy` must match GTDB-Tk 2.8.0's own `REF_HASHES`, the same check `gtdbtk check_install`
does. `split/` has no `REF_HASHES` value; its hash (`c91ff434...`) is the mirror's
`FOLDER_HASHES.tsv` value, recomputed by dblock with the same `sha1_dir`, so it catches damage
but is not independent of the mirror.

| | `gtdbtk-r232-placement` | `gtdbtk-r232-placement-lite` |
|---|---|---|
| Parts | 12 | 11 (no `gtdbtk-r232-pplacer-bac120-full-tree.tar.zst`) |
| Download | 279,636,147 bytes | 175,495,212 bytes |
| Unpacked | 7,368,346,876 bytes | 5,887,238,603 bytes (251 files) |
| `GTDBTK_DATA_PATH` | `<store>/gtdbtk-r232-placement/release232` | `.../gtdbtk-r232-placement-lite/release232` |
| Folder hashes checked | 8 (7 against `REF_HASHES`, plus `split`) | 7 (`pplacer` left out) |
| `classify --full_tree` | yes | no |

The lite entry leaves out the full 379,594-node bacterial tree (1.48 GB unpacked), which only
`--full_tree` uses; the default split-tree run (backbone, then the 8 class-level trees) and the
archaeal tree work. Its `pplacer/` then holds only `gtdb_r232_ar53.refpkg`, and GTDB-Tk's
`pplacer` hash covers both trees, so it can't match: `pplacer` is not in the lite entry's
`verify_dirs` (the archaeal tree is covered by its part's MD5 and SHA-256 only). For the same
reason `gtdbtk check_install` reports `pplacer` as wrong on a lite folder.

What these can't do: without `skani/` there is no ANI species screen, so classification stops at
genus (species stays `s__`), and GTDB-Tk 2.8's `classify` has no option to skip the screen; the
research page shows the small launcher it used, and how gPlace stood in for pplacer with every
placement step under an 8.5 GiB cap. Compat: GTDB-Tk ≥ 2.7.0.

Not in the catalog yet: the AMRFinderPlus database (a folder of files without checksums; use
`amrfinder_update`).

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
| `gtdbtk-r232-client` from the real 26 MB archive (served locally), both modes | 188 files, 79,826,896 bytes kept; all three folder hashes match, and GTDB-Tk 2.8.0's own `sha1_dir` gives the same values on the result |
| One byte of a `masks/` file changed, then `verify --deep` | Reported as a folder hash mismatch for `masks/` |
| `gtdbtk-r232-placement-lite` from the GitHub mirror (download mode) | 11 parts, 175 MB in 2 min 5 s; every part's MD5 and SHA-256 ok; 251 files, 5,887,238,603 bytes; all 7 folder hashes match; `verify --deep` ok; every file's SHA-256 matches the mirror's `MANIFEST.tsv` |
| `pplacer/` of the full entry (bac120 part unpacked next to the ar53 refpkg) | `4e8ffa11...`, GTDB-Tk 2.8.0's `REF_HASHES` value (hashed with a Python re-typing of `sha1_dir`, not by a dblock fetch of the full entry) |

The test suite also runs fetches against a local HTTP server: two fetches of one database at
once (one download, the other waits and reuses it), an MD5 written in upper case, a leftover partial
download larger than the file, a database folder without a marker, a same-size edit (caught only by
the SHA-256 list), and a `glob:` entry that matches two files (an error, not a silent fallback to the
folder), `keep` in both modes (only listed folders written, skipped entries still checksummed, a
`keep` path that matches nothing, a read-only kept folder, symlinks and hard links out of the kept
folders refused), `verify_dirs` passing and failing (nothing kept on mismatch) and outside `keep`
(catalog refused), `verify --deep` reporting an unreadable database and going on to the next,
`tar.zst` archives made with a 2^28 window (which a default zstd decoder refuses) in both modes,
and multi-part entries: three parts into one folder in both modes, a corrupt part (same size, one
byte changed; nothing kept, with MD5 alone and with MD5 + SHA-256), resuming after it (a complete
part re-read from disk, a partial one resumed with a `Range` request), a folder hash mismatch
across parts, and the catalog's checks on parts.
`sha1_dir` is cross-checked against GTDB-Tk's Python function when `GTDBTK_PYTHON` (or
`/tmp/mm/gtdbtk/bin/python`) has GTDB-Tk installed; otherwise that test is skipped.

Measured: Kraken 2 viral unpacks to 1.23 GB, not the 0.6 GB its index page lists; the catalog uses
the measured figure.

## Known gaps

- Plain `verify` checks names and sizes only; use `verify --deep` to check contents.
- Locking uses OS file locks (`flock` / `LockFileEx`), which some network file systems don't honour.
- Tool version rules exist only where the publisher states one (none for Kraken 2 or CARD).
- No parallel downloads; one database at a time.
