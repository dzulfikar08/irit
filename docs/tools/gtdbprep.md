# gtdbprep: classify genomes without the 100 GB download

`gtdbprep` is the laptop half of a [split GTDB-Tk](../research/split-gtdb.md). It runs GTDB-Tk's own
light stages (`identify`, then `align --skip_gtdb_refs`) on your genomes, after giving them and
their contigs opaque names. It packs the results into a bundle of a few kilobytes per genome. The
heavy step, tree placement, can then run elsewhere: on a machine with the full reference package,
on a laptop with gPlace and a memory cap, or on a shared server if one is ever built.
`gtdbtk classify --align_dir <bundle>` reads the bundle unchanged.

It wraps GTDB-Tk rather than reimplementing it. Every number in the bundle comes from GTDB-Tk 2.8.0
itself, run as a subprocess.

## Why

GTDB-Tk's reference data for R232 is a 61 GB download that unpacks to about 100 GB, and bacterial
placement needs about 140 GB of RAM (see [GTDB-Tk and memory](../research/gtdbtk-memory.md)). Gene
calling, marker search and alignment need none of that. They need 77 MB of marker HMMs, masks and
metadata, and take well under a minute per genome on a laptop. Splitting there means:

- the laptop never downloads the 100 GB package;
- only a masked protein alignment and marker counts leave the laptop, never the DNA;
- the result is still GTDB-Tk's own result, as the round trip below checks.

## Usage

```sh
# Laptop: GTDB-Tk 2.8 in a conda/micromamba env, plus the 77 MB client reference data
gtdbprep prep --genomes mags/ -x fa --work prep-work --out bundle.tar.gz \
    --data r232-client/ --env ~/micromamba/envs/gtdbtk
irit-gtdb-prep ...          # the same as `gtdbprep prep ...`

# Optional: also add skani 0.3.1 sketches for a remote ANI screen (contain DNA, see Privacy)
gtdbprep prep ... --sketch --skani /path/to/skani-0.3.1

# Where the full reference data is: classify the bundle, results get the original names back
gtdbprep classify --bundle bundle.tar.gz --work prep-work --data release232/

# Testing: prep, classify from the bundle, plain classify_wf, compare genome by genome
gtdbprep verify --genomes mags/ -x fa --work rt-work --data release232/ --env ...
```

Options shared by all commands: `--data` (or `GTDBTK_DATA_PATH`), `--env` (or `GTDBPREP_ENV`, a
conda prefix whose `bin/` goes first on `PATH`), `--cpus` (default 4). `--full-tree true|false` on
`classify` and `verify` defaults to false, or to true when the reference data has an empty `split/`
folder (GTDB's mockup database).

`prep` writes into `--work`:

| Path | What | Leaves the laptop? |
|---|---|---|
| `renamed/<id>.fna` | genomes with opaque names (sequence bytes unchanged) | no |
| `mapping.tsv` | opaque ID → GTDB-Tk genome ID, source path, source SHA-256, contig count, and why an input was left out (if it was) | **no, never** |
| `contigs/<id>.tsv` | opaque contig name → original header | no |
| `gtdbtk/` | GTDB-Tk's full `identify`/`align` output (genes, HMM hits) | no |
| `logs/` | one log per command | no |
| `bundle/` | the bundle (also copied to `--out`) | **yes, this is what you send** |

On Unix the work folder is made owner-only (mode 0700), and `mapping.tsv` and `contigs/*.tsv` are
written with mode 0600.

Input files that are empty or not FASTA (no `>` header, or sequence before the first one) are left
out with a warning instead of stopping the run. `mapping.tsv` lists them with opaque ID `-` and the
reason; `prep` prints them; the manifest records only how many there were (`inputs_excluded`), not
their names. `classify` repeats the warning, and `verify` reports them as `excluded` (not as
differences) and runs `classify_wf` on the remaining inputs only.

**Spaces in paths.** GTDB-Tk 2.8.0 refuses genome paths that contain a space ("unsupported by
downstream applications"), and it reads the renamed genomes from the work folder, so `prep` and
`classify` refuse a work folder with a space in its path before doing anything. The input folder
may have spaces: only gtdbprep reads it, and `verify` gives `classify_wf` a folder of links to the
inputs inside the work folder. A `--data` path with a space is untested.

`--out` takes a folder, a `.tar`, or a `.tar.gz`. It is checked before anything runs: it must not
exist yet (an empty folder is fine) and its parent folder must exist.

**What is reproducible.** Tar files have fixed metadata (mtime 0, mode 644, owner 0, sorted
entries), and every gzip file in the bundle is written with a fixed header (no file name, mtime 0;
GTDB-Tk's own `.gz` files are decompressed and recompressed so its header, which names the source
file and the time it ran, does not leave the laptop). So the same bundle folder always gives the
same `.tar` bytes. Two `prep` runs on the same genomes do not give the same bundle: the opaque IDs
are random and the manifest records `started` and `created` times. `.tar.gz` bytes also depend on
the gzip compressor (flate2 and its backend version), so a different gtdbprep build can compress
the same `.tar` to different bytes; compare `.tar` files or the manifest's `files` hashes instead.

## Bundle format (`gtdbprep-bundle/1`)

The bundle uses GTDB-Tk's own folder layout and file names (prefix `gtdbtk`), so it is a valid
`--align_dir`:

```
manifest.json
identify/gtdbtk.bac120.markers_summary.tsv     marker counts and lists per genome
identify/gtdbtk.ar53.markers_summary.tsv
identify/gtdbtk.translation_table_summary.tsv  genetic code per genome (11 or 4)
identify/gtdbtk.failed_genomes.tsv             Prodigal failures (often empty)
align/gtdbtk.bac120.user_msa.fasta.gz          masked alignment, 5,010 columns
align/gtdbtk.ar53.user_msa.fasta.gz            masked alignment, 4,369 columns (if any archaea)
align/gtdbtk.{bac120,ar53}.filtered.tsv        genomes below 10% of columns
align/intermediate_results/gtdbtk.{align,identify}.failed.tsv   (when GTDB-Tk writes them)
sketches/<id>.fna.sketch                       only with --sketch
```

`manifest.json` records:

- `format`, `client` (name, version), `started` and `created` (RFC 3339, UTC);
- `gtdb_release` (from `metadata/metadata.txt`) and `tools`: the version line each tool printed
  (`gtdbtk`, `prodigal`, `hmmer`, and `skani` with `--sketch`). A line containing `/` or `\` (a
  Python traceback naming a path, say) is not kept: `reported` is then just the parsed version;
- `commands`: the exact GTDB-Tk and skani argument lists, with local paths replaced by `<work>` and
  `<GTDBTK_DATA_PATH>`;
- `reference`: for `markers/`, `masks/`, `metadata/` and `taxonomy/`, GTDB-Tk's own folder hash
  (`sha1_dir`) next to the `REF_HASHES` value GTDB-Tk 2.8.0 expects for that release, plus a
  SHA-256 over the folder's `sha256sum` listing, and per-file SHA-256 for every folder except
  `markers/` (182 files, covered by the listing hash). `prep` refuses data whose markers, masks or
  metadata don't match `REF_HASHES` (`--allow-ref-mismatch` overrides). See
  [Reference data](#reference-data) for `taxonomy/`;
- `genomes`: per opaque ID, the SHA-256 of the renamed FASTA, contig count, domain call,
  single-copy marker counts, genetic code, which MSA holds it, and why GTDB-Tk excluded it if it did;
- `inputs_excluded`: how many input files `prep` left out (empty or not FASTA), without names;
- `sketches`: skani version and parameters (c = 125, marker c = 1000, k = 15) and the warning text;
- `files`: size and SHA-256 of every other file in the bundle. `classify` rejects a bundle whose
  files don't match, and checks again after GTDB-Tk has run that nothing was changed;
- `privacy`: the statements below.

Genomes are listed in the order of their random IDs, so the bundle doesn't reveal the original
file order.

## Privacy

**Leaves the laptop (the bundle):** for each genome, a masked alignment of 120 bacterial (or 53
archaeal) marker proteins (about 5,000 amino acids); marker counts and the list of which markers
were found; the genetic code; the domain call; the contig count; the SHA-256 of the renamed FASTA.
Also the tool versions, reference-data hashes, and the number of inputs left out. The contig count
is a mild identifying feature: with the genome size class it can help match a bundle to a known
assembly. The bundle's own `privacy` text says so.

**Stays on the laptop:** DNA sequences (except the k-mer samples in sketches, if you add them),
genes and proteins outside the markers, file names, contig names, folder paths, and the mapping
from opaque IDs back to your genomes.

**Leak scan.** After the manifest is written, `prep` searches every bundle file, `manifest.json`
included, decompressed, for: the original genome IDs, file names and contig headers (whole header
and first word); the work, data and input folders and every input file path, each as given, made
absolute, and with symlinks resolved; `$HOME` in the same three forms; and, as whole words only,
`$USER`, `$LOGNAME` and the host name. With `--out`, it scans what it wrote again (for a `.tar` or
`.tar.gz`, the tar headers and every entry, decompressed). On any hit it deletes the bundle (or the
`--out` copy) and fails, so a bundle that fails the scan cannot be sent by mistake.

Names and paths shorter than 6 characters (user and host names: shorter than 3) are not scanned
for, because they would match by chance inside alignments and marker lists. They cannot reach the
bundle by construction: GTDB-Tk only ever sees the renamed files (`<random id>.fna`, contigs
`<id>_c<n>`), so none of its outputs can contain an original name; the bundle files are GTDB-Tk's
outputs plus a manifest that gtdbprep builds from opaque IDs, counts and hashes; and the commands
recorded in it have local paths replaced by `<work>` and `<GTDBTK_DATA_PATH>`. The scan is a
second check of that, not the mechanism.

**What the alignment still says.** It identifies the organism; that is its purpose. For a novel
lineage it contains partial sequences of conserved housekeeping proteins, which may matter before
publication. The FASTA hash lets someone who already has the same genome confirm it is in the
bundle; it reveals nothing to anyone else.

**Sketches are not anonymous.** With `--sketch`, the bundle also carries a skani sketch per genome.
It holds about 1 in 125 of the genome's 15-mers with their positions and 1 in 1,000 of its 21-mers:
a sparse but real sample of the DNA. Names inside it are opaque (gtdbprep sketches the renamed
files), but the k-mers remain. `prep` prints this warning and writes it into the manifest. Don't
send sketches for unpublished or sensitive genomes. The design describes a route that keeps the ANI
screen local ([split GTDB-Tk, section 4](../research/split-gtdb.md#4-can-the-ani-screen-run-from-a-sketch)).

## How `classify` matches `classify_wf`

`classify_wf` runs the ANI screen first and never runs `identify`/`align` on genomes it assigns to
a species, so their `msa_percent` and `translation_table` are `N/A`. A plain `gtdbtk classify`
given a bundle that contains every genome fills those two fields in instead. That was the only
difference in the first round trip. `gtdbprep classify` therefore:

1. runs GTDB-Tk's own ANI screen (`ANIScreener.run_aniscreen`, because GTDB-Tk 2.8 has no
   `ani_screen` subcommand) on the renamed genomes. It runs under the Python that runs `gtdbtk`:
   the environment's `python3` with `--env`, otherwise the interpreter named in the shebang of the
   `gtdbtk` script on `PATH` (conda's `#!/bin/sh` + `exec` form included). The call refuses any
   GTDB-Tk version other than 2.8.0, since the API is internal, and refuses genome IDs it was not
   given; gtdbprep then checks every assigned ID is in the bundle's manifest;
2. stages a copy of the bundle without the assigned genomes (rows removed from every TSV, records
   from every MSA), which is what `classify_wf`'s align folder looks like;
3. runs `gtdbtk classify --genome_dir <renamed> --align_dir <staged> [--full_tree]`;
4. maps opaque IDs back to the original GTDB-Tk genome IDs (`replay/summary.original_names.tsv`).

A placement server would do the same in steps 1 and 2, with the ANI screen fed by sketches.

All subprocesses run with `LC_ALL=C`. Under some host locales, Bioconda's pplacer aborts in glibc's
locale loader (`_nl_intern_locale_data` assertion), and the C locale changes no output.

## Reference data

`prep` reads `markers/`, `masks/` and `metadata/`. Their SHA-1 equals GTDB-Tk's `REF_HASHES`
for R232 in GTDB's 25 MB mockup archive (checked by `prep` on every run). GTDB-Tk also opens
`taxonomy/gtdb_taxonomy.tsv`, but only to refuse input genome IDs equal to a GTDB accession.
Opaque IDs (`g` + 16 hex digits) never are, so the mockup's 5-line stub works. gtdbprep records
its hash and notes the `REF_HASHES` mismatch without stopping. These four folders total 77 MB. With
only them on disk, `prep` produced byte-identical alignment rows to a run against the whole
mockup. `dblock add gtdbtk-r232-client && dblock fetch` fetches exactly this subset and checks it
against `REF_HASHES` (see [dblock](dblock.md#gtdbtk-r232-client-and-its-md5)); pass the path
`dblock env` prints to `--data`.

`classify` and `verify` need the full package (or the mockup for tests), as GTDB-Tk does.

## Tested

2026-11-10, GTDB-Tk 2.8.0 (Bioconda: Prodigal 2.6.3, HMMER 3.4, pplacer 1.1.alpha19, skani 0.4.0
for GTDB-Tk's own ANI screen), skani 0.3.1 built from its tag for sketches, R232 mockup database
(md5 `59512f60fa48010647977460bb855666`, see [GTDBTk#726](https://github.com/Ecogenomics/GTDBTk/issues/726)),
4 threads on a 12-core Linux laptop. Genomes: GTDB's test genome G021783475 (1.91 Mb, 138 contigs),
E. coli K-12 MG1655 (4.64 Mb) and P. aeruginosa PAO1 (6.26 Mb), gzipped.

**Round trip (`gtdbprep verify`): 3/3 genomes identical to `classify_wf`.** The merged
`summary.tsv` is identical line for line once rows are sorted. The test genome is assigned by the
ANI screen, and the other two are placed with pplacer (on the mockup's 5-genome tree, so the
classifications themselves are not meaningful). The renamed genomes' alignment rows are
byte-identical to `classify_wf`'s own user MSA. The whole `verify` (prep, classify from the
bundle, `classify_wf`) took 78 s wall.

| Bundle part | Bytes (3 genomes) | Per genome |
|---|---|---|
| `bac120.user_msa.fasta.gz` | 7,499 | 2.5 KB |
| marker summaries (bac120 + ar53, plain text) | 5,782 | 1.9 KB |
| translation table, filtered/failed lists | 63 | <0.1 KB |
| `manifest.json` | 5,794–6,343 | ~0.3 KB per genome entry + ~5 KB fixed |
| **folder total, no sketches** | 19,197–19,687 | 6.4–6.6 KB |
| **`.tar.gz`, no sketches** | 11,201 | 3.7 KB |
| skani 0.3.1 sketches | 205,666 / 491,686 / 699,402 | 1.9 / 4.6 / 6.3 Mb genome |

Tests: 37 unit tests (renaming, GTDB-Tk genome IDs, gzip input, non-FASTA inputs, leak scan of
folders and tarballs, user/host/path needles, manifest round trip and tamper detection, SHA-256
vectors, GTDB-Tk's `sha1_dir` order including symlinks, tar reproducibility, gzip header rewrite,
file modes, `--out` checks, summary comparison, dropping ANI-assigned genomes, the ANI-screen
interpreter, version and ID checks against a stand-in `gtdbtk` Python package). One of them runs
the whole `prep` against a stand-in `gtdbtk` shell script, with spaces in the input, data and
environment paths, a refused work folder with a space, two bad
inputs, and a local name planted where only the manifest would carry it. One integration test,
ignored by default, runs `verify` on GTDB's test genome folder, copied into a folder whose name has
a space and joined by an empty input file. It checks
every genome is identical, that the empty input is reported as left out, that no original name is
in the `.tar.gz`, and that `classify` from the `.tar.gz` gives `classify_wf`'s rows:

```sh
GTDBPREP_ENV=/path/to/gtdbtk-env GTDBTK_DATA_PATH=/path/to/release232_mockup \
GTDBPREP_TEST_GENOMES=/folder/with/test_genome_G021783475.fna.gz \
GTDBPREP_SKANI=/path/to/skani-0.3.1 \
cargo test --release -- --ignored
```

## Known gaps

- **Mockup only.** The round trip ran against GTDB's mockup database (5 reference genomes, full
  tree). The real R232 package (split backbone and class-level trees) is untested: that needs the
  ~100 GB package on a large-memory machine (the [phased plan](../research/split-gtdb.md#phased-plan),
  step 2).
- **Three bacterial genomes.** No archaea, no translation table 4, no genomes below 10% alignment,
  no Prodigal failures, no batch where every genome is ANI-assigned, no borderline bac/ar calls.
- **The ANI screen still needs the genomes.** `classify` screens the renamed FASTA files locally,
  so it runs where the genomes and the skani database both are. Sketches are produced and pinned to
  skani 0.3.1, but nothing consumes them yet: the server-side shim that maps sketch queries back to
  opaque IDs (design, section 4) is not built.
- **`--place_species` is not supported** (its `classify_wf` behaviour differs and is untested).
- **The ANI-screen step calls GTDB-Tk's internal Python API** (`ANIScreener.run_aniscreen`), which
  may change in a later GTDB-Tk release. The call is pinned to GTDB-Tk 2.8.0: `classify` refuses
  other versions until the API has been checked against them.
- **Placement-side provenance** (pplacer/gPlace version, full-package hash) is not written next to
  the results yet; only the laptop side has a manifest.
- **Marker summaries are plain text** inside a folder bundle (1.9 KB per genome); use `.tar.gz`
  to send them compressed.
- **Linux and macOS only** (random IDs come from `/dev/urandom`; GTDB-Tk doesn't run on Windows).
- **Tool sources.** Bioconda installs skani 0.4.0 with GTDB-Tk 2.8.0; `--sketch` refuses anything
  but 0.3.1, which has to be built from its tag.
- **Reference download.** GTDB's mockup archive currently fails its published MD5
  ([GTDBTk#726](https://github.com/Ecogenomics/GTDBTk/issues/726)), so `dblock` refuses
  `gtdbtk-r232-mockup`; the `gtdbtk-r232-client` entry uses the observed MD5 and checks the
  extracted folders against `REF_HASHES` instead.
