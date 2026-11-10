# Split GTDB-Tk: a design

GTDB-Tk classifies bacterial and archaeal genomes against the Genome Taxonomy Database (GTDB). In
its current form it does not run on a laptop: R232 needs about 140 GB of RAM for bacteria and about
100 GB of reference data on disk (see [GTDB-Tk and memory](gtdbtk-memory.md)). This page designs a
split version. The laptop runs the light stages. A shared server keeps the reference trees loaded
and does the placement. Genome sequences stay on the laptop.

Everything here was checked on 2026-11-10 against GTDB-Tk 2.8.0 (source commit `155d989`, 13
October 2026), the GTDB R232 file listings, skani 0.3.1 and 0.4.0, and the gPlace 0.1.1
documentation. Numbers are **measured** (run here), **cited** (with a source), or marked
**estimate**.

## Summary

- **The laptop side is small and already proven.** Gene calling, marker search and alignment need
  only the marker HMMs, masks and metadata: 77 MB unpacked, all inside GTDB's 25.1 MB *mockup*
  archive. Those folders are byte-identical to the full 56.6 GB package: their SHA-1 matches the
  hashes hard-coded in GTDB-Tk. Running `identify` + `align --skip_gtdb_refs` on a laptop gives
  the same user alignment as a full `classify_wf` (measured, identical bytes). The largest process
  peaked at 165 MB.
- **Laptop → server is about 2.5 KB per genome for placement** (one masked marker alignment row of
  5,010 amino acids for bacteria, 4,369 for archaea, gzipped). The ANI species screen is different.
  If it runs on the server, the laptop must send a skani sketch of 140–430 KB per genome (measured).
  That sketch contains real DNA fragments, so the privacy claim must say so.
- **`skani search` accepts a sketch instead of a FASTA** and gives identical ANI/AF (measured).
  GTDB-Tk's skani wrapper does not handle this case, so the server needs a small shim around it.
- **Server RAM.** GTDB-Tk with pplacer needs ~140 GB (cited), so a 192–256 GB machine. With gPlace,
  the largest R226 class tree needs under 44 GiB (cited), and `--max_memory` caps memory at any
  size, at some cost in speed. gPlace 0.1.1 placed 10,000 genomes on the **full R232 bacterial
  tree within an 8.5 GB cap** (cited).
- **Cost** (on-demand list prices, 2026-11): 64 GB machines cost about $0.52/h on AWS or GCP, or
  €276/month at Hetzner. 192–256 GB machines cost about $2.10/h, or €853/month. A scale-to-zero
  server used one hour a day costs about $16–65/month in compute (estimate).
- **The view on building it.** Build the laptop client first: it is needed whatever happens next.
  **Do not build the server yet.** gPlace's memory cap may make placement possible on the laptop
  itself. The remaining blocker would then be disk, which hosted reference files and HTTP range
  reads can solve without anyone running a server. Measure that route first (see
  [Phased plan](#phased-plan)).

## 1. How `classify_wf` is staged (GTDB-Tk 2.8.0, R232)

Read from `gtdbtk/main.py` (`parse_options`, `classify_wf` branch), `markers.py`, `classify.py`,
`ani_screen.py`, `external/skani.py` and `config/common.py`.

```
genomes (FASTA)
   │
   ├─(1) ani_screen ── skani search vs pre-sketched DB of all 199,923 species reps
   │        └─ genome with ANI ≥ rep's radius and AF ≥ 0.5 → species assigned, DONE
   │           (unless --place_species)
   │
   ├─(2) identify ──── Prodigal (tables 4 and 11) → HMMER hmmsearch vs 120 bac + 53 ar markers
   │        └─ domain call: % of bac120 vs ar53 single-copy markers found
   │
   ├─(3) align ─────── hmmalign each marker → concatenate (41,084 / 13,540 cols)
   │        └─ apply canonical mask → 5,010 (bac) / 4,369 (ar) cols; drop if <10% filled
   │
   └─(4) classify
            ├─ archaea:  pplacer on the single ar53 tree
            └─ bacteria: pplacer on the backbone tree → class → pplacer on that
                         class-level tree → RED values + topology + ANI results → taxonomy
```

Stage order changed in 2.7: the ANI screen now runs **first**, against a single pre-sketched skani
database of all GTDB representatives. Genomes it assigns to a species skip `identify`, `align` and
placement, unless `--place_species` is given. `--skip_ani_screen` and `--skani_sketch_dir` were
removed. In 2.8, `classify` and `classify_wf` apply the same species rule (closest reference that
passes the AF threshold, checked against its own ANI radius). 2.8 also writes the skani hits of
unassigned genomes to `classify/ani_screen/<prefix>.ani_screen_unassigned_hits.tsv.gz`, which
`classify` reads back for its notes and warnings.

| Stage | Inputs | Outputs | Reference files used (R232 package) | Tools | Resources |
|---|---|---|---|---|---|
| 1 `ani_screen` | genome FASTA | `ani_summary.tsv` per domain, unassigned hits `.tsv.gz` | `skani/database/` (sketches.db, index.db, markers.bin), `radii/gtdb_radii.tsv` (11 MB, 199,923 rows), `taxonomy/gtdb_taxonomy.tsv` | skani `search -d <db> --ql <list> --short-header` (default preset, c = 125, k = 15) | skani DB ~90+ GB on disk (estimate, see note); RAM: all marker sketches loaded, full sketches memory-mapped |
| 2 `identify` | genome FASTA | `*.faa` genes, `markers_summary.tsv` per domain, `translation_table_summary.tsv` | `markers/` (77 MB: `tigrfam/tigrfam.hmm` 152 models + pressed files, `pfam/Pfam-A.hmm` 16 models, `individual_hmms/`) | Prodigal 2.6.3 (via GTDB-Tk's biolib wrapper, both tables 4 and 11), HMMER 3.x `hmmsearch` | measured: 36 s wall / 67 s CPU for 2 genomes (4.6 and 6.3 Mb) on 4 threads; largest process 165 MB |
| 3 `align` | identify folder | `<prefix>.bac120.user_msa.fasta.gz` (and `ar53`), `filtered.tsv` | `markers/*/individual_hmms`, `masks/gtdb_r232_bac120.mask` (41,084 chars, 5,010 kept), `masks/gtdb_r232_ar53.mask` (13,540, 4,369 kept); `msa/` only to write the combined ref+user MSA, not needed with `--skip_gtdb_refs` | HMMER `hmmalign` | measured: 0.9 s, 48 MB for 2 genomes |
| 4a classify, archaea | user MSA (ar53) + ANI results | `ar53.summary.tsv`, `ar53.classify.tree` | `pplacer/gtdb_r232_ar53.refpkg`, `mrca_red/gtdbtk_r232_ar53.tsv`, taxonomy, radii, metadata (RED thresholds) | pplacer 1.1.alpha19 (`-m WAG`), guppy `tog` | ~100 GB RAM (cited, GTDB-Tk docs) |
| 4b classify, bacteria (split, default) | user MSA (bac120) + ANI results | `bac120.summary.tsv`, backbone + class-level trees, `tree_mapping.tsv` | `split/backbone/pplacer/gtdbtk_package_backbone.refpkg`, `split/class_level/pplacer/gtdbtk.package.{n}.refpkg`, `split/*/red/*.tsv`, `split/class_level/tree_mapping.tsv` | pplacer, guppy | ~140 GB RAM (cited); `--full_tree` ~950 GB |

Notes:

- **Per-folder sizes inside the package are not published.** GTDB's listing gives only the archive
  (56.63 GB) and GTDB-Tk's docs say ~100 GB unpacked. skani sketches measured here are 219–699 KB
  per genome (1.9–6.3 Mb genomes). Times 199,923 representatives, that suggests the skani database
  is most of the 100 GB (**estimate**). GTDB-Tk's 2.7 changelog says the pre-sketched database cut
  "the database storage footprint from 198 GB down to 98 GB".
- **The mockup DB contains the real `markers/`, `masks/`, `radii/` and `metadata/` folders.** Their
  SHA-1 (GTDB-Tk's own `sha1_dir` algorithm) equals GTDB-Tk's `REF_HASHES` for R232:
  `163f542c…` (markers), `84c7f7a1…` (masks), `546c1769…` (radii), `1febdba7…` (metadata). Its
  `pplacer/`, `skani/` and `msa/` folders hold 5 dummy genomes, and it has no `split/` folder.
- **Version pins don't hold in practice.** GTDB-Tk's 2.6.0 changelog fixes skani at 0.3.1 and
  pplacer at 1.1.alpha19. The Bioconda recipe for 2.8.0 requires only `skani >=0.3.0`, and a fresh
  install today gets skani **0.4.0** (measured). 0.4.0 writes a new, incompatible sketch database
  format (`.db2`) but still reads 0.3.x databases. Results agreed on the test genome here. The split
  protocol must pin these versions itself.
- **Problem sizes.** R232 has 199,923 species representatives. The R232 trees have 189,801
  bacterial and 10,122 archaeal leaves (counted here from `bac120_r232.tree.gz` and
  `ar53_r232.tree.gz`). The backbone and class-level trees ship only inside the package; their
  number and sizes for R232 are not known without downloading it.

## 2. What the laptop needs

| Item | Size | Source |
|---|---|---|
| `markers/` (TIGRFAM + Pfam HMMs, pressed and individual) | 77 MB unpacked | inside GTDB's `r232_mockup.tar.gz` (25.1 MB) |
| `masks/` (bac120, ar53) | 54 KB | same; also published as `bac120_msa_mask_r232.txt` / `ar53_msa_mask_r232.txt` (identical, checked) |
| `metadata/metadata.txt` (RED thresholds, version) | 333 B | same |
| Prodigal 2.6.3, HMMER 3.4 | binaries | Bioconda (later: native ports, see risks) |
| skani 0.3.1 (only if the laptop sketches) | 2.7 MB static binary | built from the v0.3.1 tag (3 min here) |

**Distribution through `dblock`.** `dblock` already lists `gtdbtk-r232-mockup`. A new
`gtdbtk-r232-client` entry would fetch the same archive and keep only `markers/`, `masks/` and
`metadata/`. It would then check those folders against GTDB-Tk's `REF_HASHES`, not just the
archive checksum, because GTDB labels the mockup "for testing" and could change it (see
[GTDBTk#726](https://github.com/Ecogenomics/GTDBTk/issues/726)). The cleaner long-term source is
an unpacked copy on R2 (see section 7), or asking GTDB to publish a "client" subset.

## 3. Payload sizes

Measured with GTDB-Tk 2.8.0 on E. coli K-12 (4.64 Mb) and P. aeruginosa PAO1 (6.26 Mb), and with
skani 0.3.1 on those plus GTDB's test genome (1.91 Mb) and GTDB-Tk's `genome_1` (2.97 Mb).

**Laptop → server, per genome**

| Artefact | Raw | Compressed | Notes |
|---|---|---|---|
| Masked bac120 MSA row | 5,010 B + header | ~2.4 KB gzip (4,809 B for 2 genomes); 3,694 B zstd -19 for 2 | identical to `classify_wf`'s own file |
| Masked ar53 MSA row | 4,369 B + header | ~2 KB (estimate) | |
| Marker summary + translation table + domain percentages | ~1.3 KB | <1 KB | used by `classify` for `msa_percent`, `translation_table`, multi-copy and domain warnings |
| skani 0.3.1 sketch (only for a server-side ANI screen) | 219 KB (1.9 Mb), 316 KB (3.0 Mb), 492 KB (4.6 Mb), 699 KB (6.3 Mb) | zstd -19: 143 / 195 / 314 / 430 KB | sketch params must match the DB (c = 125, k = 15, marker c = 1000) |

So a typical 3–5 Mb genome costs about **4 KB** if only placement is remote, or about **200–320
KB** if the ANI screen is remote too.

**Server → laptop, per genome**

| Artefact | Size | Notes |
|---|---|---|
| `summary.tsv` row | 0.3–0.6 KB (1,416 B for header + 3 rows, measured) | the result; same columns as GTDB-Tk |
| jplace placements | ~0.4 KB (846 B for 2 genomes on the mock tree, measured) | up to 7 edges per genome on the real trees |
| Classify tree with the genome inserted (optional) | MBs per class tree (**estimate**: the full R232 bac120 tree is 9.0 MB of Newick) | the reference trees are public; the laptop can fetch them once and graft locally from jplace |

## 4. Can the ANI screen run from a sketch?

**Yes, technically.** In skani 0.3.1, `skani search` treats queries as sketches when every query
file name contains `.sketch` (`src/parse.rs`). It then loads them with `sketches_from_sketch` and
only warns if the sketch parameters differ from the database's. Tested here against the mockup
database: a FASTA query and the same genome's `--separate-sketches` sketch gave identical rows
(ANI 100.00 / AF 99.93 and 94.76 / 80.52 / 84.57).

Three caveats:

1. **GTDB-Tk's wrapper maps results back by query path** (`reverse_ql.get(parsed.query_file)` in
   `external/skani.py`). With sketch queries, skani reports the file name stored *inside* the
   sketch, so the lookup returns nothing. The server needs a shim: either name the stored file
   after the opaque genome ID, or run `skani search` itself and write GTDB-Tk's
   `ani_summary.tsv` and unassigned-hits files. GTDB-Tk's 2.8 `classify` reads both back.
2. **A sketch is not anonymous.** The struct (`src/types.rs`) stores the original file name, every
   contig header, contig lengths, ~1/125 of the canonical 15-mers *as raw k-mers with positions*,
   and ~1/1000 of the canonical 21-mers. The test sketch contained headers like
   `MAG: bacterium isolate BP4.bin65 NODE_10248_length_9430_cov_1.48`. For a 4.6 Mb genome that is
   roughly 37,000 positioned 15-mers, about 0.5 Mb of sequence (**estimate**). The client must
   rename the file and contigs before sketching. Even then, the k-mers remain.
3. **Format drift.** skani 0.4.0 sketch databases cannot be read by 0.3.x. Client and server must
   agree on the skani version, and the server must reject mismatched sketches.

**An alternative that keeps sequence data on the laptop.** The consolidated skani database has an
index of (offset, length) for each sketch (`index.db`, `src/sketch_db.rs`). The 199,923 reference
sketches are public GTDB data. With an unpacked copy on object storage, the laptop could:

1. download `markers.bin` once (~5–7 GB, **estimate** from 26–35 KB of markers per genome here), or
   send only its own ~1/1000 marker k-mers to a stateless lookup;
2. pick candidate references exactly as `skani search` screens them;
3. fetch only those reference sketches with HTTP range requests (0.2–0.7 MB each);
4. run skani locally. ANI is computed pairwise, so the result should be identical (to be verified).

This keeps the ANI screen fully local. It needs no server, only static files.

## 5. Server design

```
 LAPTOP (irit client)                              SERVER (one per GTDB release × GTDB-Tk version)
 ─────────────────────                             ────────────────────────────────────────────────
 genome.fna ──► rename to opaque id                 /v1/info  ◄── versions, hashes, limits
           ──► skani sketch (opt.)  ─┐
           ──► prodigal → hmmsearch  │  manifest.json  ┌──► ANI screen (skani search, shim)
           ──► hmmalign → mask       ├─────────────────┤        │ species found → result
               ↓                     │  msa.fasta.gz   │        ▼
           marker summary, tln table ┘  sketch.zst     ├──► backbone placement (pplacer|gPlace)
                                                       │        │ class n
           summary.tsv ◄── map ids back ◄──────────────┤        ▼
           (+ jplace, provenance)        results.json  └──► class-level tree n (load on demand)
                                                                │
 genome sequences never leave ◄──────────────────────────── RED + topology → taxonomy
```

**Data held by the server.** The full R232 package unpacked (~100 GB on local NVMe; the laptop's
`markers/` aren't needed there). In RAM:

| Component | pplacer | gPlace |
|---|---|---|
| Largest bacterial class-level tree | ~104 GB (R226, cited by gPlace) | <44 GiB (R226, cited); R232 not published; scaling by GTDB-Tk's own 140 GB → ~60 GB (**estimate**) |
| Bacterial split overall (GTDB-Tk figure) | ~140 GB (R232, cited) | not published for R232 |
| Archaeal tree | ~100 GB (cited) | ~14 GiB from gPlace's formula 2 × internal nodes × sites × 168 B (10,121 × 4,369) (**estimate**) |
| Full R232 bacterial tree | ~950 GB (cited) | untiled ~300 GiB from the same formula (**estimate**); tiled: placed within 8.5 GB `--max_memory` (cited) |
| skani screen | marker sketches in RAM, ~5–7 GB (**estimate**); full sketches memory-mapped from disk | same |

**Lazy class trees.** GTDB-Tk already loads trees one at a time: the backbone, then each class-level
tree that received genomes, in sequence. If every class tree stayed resident, total memory would
approach the whole-tree cache, because the class trees partition the tree. So the server should
keep the backbone hot and load class trees on demand, ordered by queue depth. With gPlace, all
queries on a reference package share one likelihood cache, and "a query's result does not depend
on the other queries" (gPlace docs). That makes batching safe in principle. pplacer pre-masks
columns using all queries in a run, so batch-independence must be tested, not assumed (section 6).

**Batching and concurrency.** Jobs are queued per tree: for example, flush a class tree when it has
N genomes waiting or after T minutes. gPlace found 32–40 workers fastest on a 64-core machine,
with memory barely growing with workers (cited). pplacer forks workers that share memory
copy-on-write (GTDB-Tk FAQ). ANI-screen jobs are independent and can run between placement
batches.

**API shape.** Plain HTTP + JSON, with binary parts as multipart (curl-able; no gRPC toolchain
needed in the Rust client):

- `GET /v1/info`: GTDB release (`r232`), package MD5 (`25a59e0352b1fd150c589f56559767d4`), the
  `REF_HASHES` of each folder, GTDB-Tk version, skani/pplacer/gPlace versions, accepted client
  versions, limits.
- `POST /v1/jobs`: `manifest.json` (client version; laptop tool versions; markers and masks hashes;
  per genome: opaque ID, domain counts, marker summary, translation table, `--place_species`) plus
  `bac120.msa.fasta.gz`, `ar53.msa.fasta.gz`, and optional sketches. If the release, GTDB-Tk
  version or hashes don't match, the server answers `409` with what it does accept.
- `GET /v1/jobs/{id}`: status, then results: `summary.tsv` rows, jplace, and provenance.
- Results are deleted after a fixed retention period (for example 7 days), and inputs as soon as the
  job finishes.

**Auth and limits.** API keys (sign-in with GitHub or ORCID), per-key quotas (genomes per day,
upload size), a global queue cap, and refusal of uploads that aren't alignments or sketches of the
expected shape (length 5,010 / 4,369 columns, amino-acid alphabet).

**Provenance in every result.** GTDB release and package MD5; `REF_HASHES` of the server's data;
GTDB-Tk version and the equivalent command (`gtdbtk classify_wf` with defaults, or with
`--place_species`); versions of skani, pplacer or gPlace (and gPlace engine variant), Prodigal and
HMMER (reported by the client); client version; job ID; timestamps. Outputs use GTDB-Tk's own file
formats, so `isotable` and other tools read them unchanged.

**Privacy statement (draft).**
- *Placement only:* the server receives 120 (bacteria) or 53 (archaea) concatenated, masked
  marker-protein alignments per genome, about 5,000 amino acids, plus marker counts and the genetic
  code. It receives no DNA, no contig names and no file names. The alignment reveals what the
  organism is (that is its purpose). For a novel lineage it reveals partial sequences of conserved
  housekeeping proteins, which may matter before publication.
- *With a server-side ANI screen:* the server also receives a skani sketch, which contains about one
  in 125 of the genome's 15-mers with positions and one in 1,000 of its 21-mers. That is a sparse
  but real sample of the DNA sequence. The client strips names before sketching. Users with
  unpublished or sensitive genomes should use the local ANI route (section 4).

## 6. Proving results equal GTDB-Tk's

The test: for the same genomes, the split pipeline's `summary.tsv` must equal `gtdbtk classify_wf`'s,
field by field (classification, closest references, ANI, AF, placement reference, method, note,
other related references, `msa_percent`, translation table, RED value, warnings). jplace
placements must match too. Only paths and timestamps may differ.

1. **Laptop stages (done on 2 genomes, extend).** `identify` + `align --skip_gtdb_refs` gave a
   user MSA byte-identical to `classify_wf`'s. Repeat on thousands of genomes, including
   translation table 4 (Mycoplasma), genomes below 10% alignment, genomes with no markers, failed
   Prodigal, and borderline bacteria/archaea calls (`bac_ar_diff`).
2. **ANI equality.** For sketch queries vs FASTA queries, compare all skani rows (done for one
   genome on the mock DB). Then on the real DB, run every test genome both ways.
3. **End to end on a rented machine** with the real R232 package, the same GTDB-Tk 2.8.0, skani
   0.3.1 and pplacer 1.1.alpha19:
   - GTDB's test genome (`test_genome_G021783475.fna.gz`) and `gtdbtk test` genomes;
   - ~1,000 GTDB representatives with `--place_species` (exercises every class tree);
   - public MAGs not in R232: genomes deposited after the R232 cutoff, plus novel-lineage MAGs.
     Include Patescibacteria/CPR, DPANN and Asgard archaea, whose reduced genomes stress marker
     completeness and placement.
   - GlobDB r226 genomes, which gPlace's validation already used (127,343 genomes), as the large
     set.
4. **Placement backend.** gPlace vs pplacer on the same queries (`gplace compare`). gPlace reports
   100% identical output on 127,343 genomes for R226 (cited); repeat for R232 and for tiled
   placement.
5. **Batch independence.** Place genome X alone, then inside batches of 10, 1,000 and 10,000 other
   genomes. Results must be identical, with both pplacer and gPlace.
6. **Release gate.** Any non-identical row blocks a server release. Keep the comparison as a CI
   job that runs on each GTDB-Tk or GTDB release.

## 7. Hosting and cost

On-demand Linux list prices, checked 2026-11-10 (third-party trackers that agree with each other;
the AWS and GCP prices are not from the providers' own pages, so recheck before committing):

| Machine | vCPU | RAM | Price | Fits |
|---|---|---|---|---|
| AWS r7i.2xlarge (us-east-1) | 8 | 64 GiB | $0.5292/h (~$386/month); spot ~$0.195/h | gPlace (class trees, or capped) |
| GCP n2-highmem-8 (us-central1) | 8 | 64 GB | $0.5241/h (~$383/month); spot $0.3144/h | gPlace |
| Hetzner Cloud CCX43 (DE/FI) | 16 dedicated | 64 GB | €275.99/month (+€0.50 IPv4), 360 GB NVMe | gPlace; always on |
| AWS r7i.8xlarge | 32 | 256 GiB | $2.1168/h (~$1,545/month); spot ~$0.49–0.65/h | pplacer (140 GB) |
| GCP n2-highmem-32 | 32 | 256 GB | $2.0962/h; spot $1.2575/h | pplacer |
| Hetzner Cloud CCX63 | 48 dedicated | 192 GB | €853.49/month (€1.3678/h), 960 GB NVMe | pplacer; always on |

Hetzner raised CCX prices twice in 2026 (1 April and 15 June, about 2.1–2.3× in total), so older
quotes are stale.

**Scale to zero (estimates).** Keep the unpacked package on a persistent volume or a stopped
machine's disk. Start a 64 GB machine when the queue is non-empty or once a day, and stop it when
the queue is empty. One hour a day costs ~$16/month on r7i.2xlarge on demand, or ~$65/month on
r7i.8xlarge, plus storage for ~100 GB (not priced here). Start-up time (boot, page in the tree
caches) is unmeasured. Users would wait minutes to a day, which is acceptable for an
asynchronous job API.

**Reference files on Cloudflare R2.** R2 Standard costs $0.015 per GB-month with free egress. Class
B reads are $0.36 per million; the free tier is 10 GB-month and 10 M reads. A ~100 GB unpacked
GTDB-Tk package costs ~$1.50/month. GTDB data is CC BY-SA, so redistribution needs attribution
and the same licence. Range reads on `sketches.db` give per-genome access (section 4).

**Why serverless can't place.** A Cloudflare Worker isolate is capped at 128 MB of memory.
Cloudflare Containers top out at 4 vCPU / 12 GiB (`standard-4` or custom; the default can be raised
on request). That is far below 44–140 GB untiled. A capped gPlace run might fit 12 GiB, since the
changelog shows 8.5 GB working on the full R232 tree. But the container would still need the
reference package (20 GB container disk limit) and the run time is unmeasured. Treat this as an
experiment, not a plan.

**Existing hosted options and how this differs.**
- [KBase GTDB-Tk app](https://kbase.us/applist/apps/kb_gtdbtk/run_kb_gtdbtk_classify_wf/release):
  free, but runs GTDB-Tk 2.3.2 (older release). Genomes are uploaded to KBase.
- [Galaxy Europe](https://usegalaxy.eu/root?tool_id=gtdbtk_classify_wf): GTDB-Tk 2.7.2 for R232,
  with user reports that the R232 database is not loaded yet. Genomes are uploaded.
- The split design uploads no genomes (alignments only, optionally sketches). It runs the current
  GTDB-Tk version and returns provenance for comparison with a local run. It does not provide
  KBase's or Galaxy's broader workflow environment.

## 8. Risks, open questions, and what would kill it

**What would make the server not worth building**

- **Placement may fit on a laptop.** gPlace's `--max_memory` tiles the tree with identical results.
  Its 0.1.1 changelog reports 10,000 genomes on the full R232 bacterial tree within an 8.5 GB cap
  (25 minutes on 64 CPUs). If a 16 GB laptop can place a few genomes on a class tree in minutes,
  the remaining barrier is disk. Disk is solved by fetching only the refpkgs needed (an unknown
  size, probably a few GB, **estimate**) and the ANI screen through range reads (section 4). In
  that case a server adds operating cost and a privacy story without much gain.
- **GTDB-Tk itself adopts gPlace**, making a 64 GB workstation enough. gPlace's author maintains
  GTDB-Tk and is packaging it for Bioconda. Many labs have 64 GB machines; laptops usually don't.
- **No one to run it.** A shared server needs an owner, uptime, abuse handling and money. Without a
  host institution or grant, an always-on 192 GB machine at €853/month is not sustainable.

**Technical risks**

- Version coupling: results depend on the GTDB-Tk version (2.8 changed species assignments), skani
  version and format, and pplacer or gPlace version. The server must run exactly one combination
  per endpoint, and the client must refuse mismatches.
- Reimplementing GTDB-Tk's classify logic (RED, topology rules, warnings) on the server would
  diverge silently. The server should **call GTDB-Tk's own code** with a thin shim for skani
  sketches, never a rewrite.
- Laptop tools: Prodigal and HMMER come from Bioconda. Native Rust ports would need byte-level
  equality tests before they replace them. Prodigal took ~25 s per genome here (two translation
  tables).
- gPlace is labelled "work in progress" by its authors.
- Unknowns that need the real package: the number and sizes of R232 class-level trees and refpkgs,
  the real `skani/database` size, and server start-up time.

## Phased plan

1. **Laptop client prototype (buildable now, no server).** An `irit gtdb prep` command wraps
   GTDB-Tk's own `identify` + `align --skip_gtdb_refs`. It renames genomes and contigs to opaque
   IDs and writes a manifest (tool versions, markers and masks hashes, marker summary, translation
   table, domain percentages) plus the gzipped MSAs. Optionally it writes skani 0.3.1 sketches.
   Reference data: a `dblock` entry `gtdbtk-r232-client` (markers, masks, metadata, verified
   against `REF_HASHES`). Test against the mockup DB: the prepped artefacts fed to a local
   `gtdbtk classify` (via the skani shim) must reproduce `classify_wf`'s `summary.tsv` exactly.
   All of this runs on a laptop today.
2. **Measure the no-server route.** On one rented large-memory machine (the roadmap's existing
   "measure first" item): unpack R232, record per-folder sizes, class tree and refpkg sizes, and
   run the validation of section 6. Then copy the needed refpkgs to a laptop and time gPlace with
   `--max_memory 8G/16G` for 1, 10 and 100 genomes.
3. **Decide.** If laptop placement takes minutes, build the local route instead: hosted unpacked
   files on R2, range-read ANI screen, refpkgs fetched on demand, gPlace with a memory cap. If it
   takes hours or fails, build the server.
4. **Server (only if 3 says so).** A batch-queue service around GTDB-Tk 2.8 + gPlace on a 64 GB
   machine, scaled to zero, with the API, provenance and retention above. Release it only when the
   section 6 validation passes, and rerun that validation for every GTDB or GTDB-Tk release.

## Sources

- GTDB-Tk source, commit `155d989` (2.8.0): [github.com/Ecogenomics/GTDBTk](https://github.com/Ecogenomics/GTDBTk) (`gtdbtk/main.py`, `markers.py`, `classify.py`, `ani_screen.py`, `external/skani.py`, `config/common.py`, `tools.py`)
- GTDB-Tk docs: [installing (hardware, reference data)](https://ecogenomics.github.io/GTDBTk/installing/index.html), [FAQ](https://ecogenomics.github.io/GTDBTk/faq.html), [change log](https://ecogenomics.github.io/GTDBTk/changelog.html), [announcements](https://ecogenomics.github.io/GTDBTk/announcements.html)
- GTDB R232 files: [release232/232.0](https://data.gtdb.ecogenomic.org/releases/release232/232.0/), [auxillary_files](https://data.gtdb.ecogenomic.org/releases/release232/232.0/auxillary_files/), [gtdbtk_package](https://data.gtdb.ecogenomic.org/releases/release232/232.0/auxillary_files/gtdbtk_package/) (full 56.63 GB, split parts, mockup 25.1 MB)
- Bioconda GTDB-Tk recipe: [meta.yaml](https://github.com/bioconda/bioconda-recipes/blob/master/recipes/gtdbtk/meta.yaml)
- skani: [github.com/bluenote-1577/skani](https://github.com/bluenote-1577/skani) (v0.3.1 tag: `src/search.rs`, `src/parse.rs`, `src/types.rs`, `src/sketch_db.rs`, `src/seeding.rs`); Shaw & Yu, [Nature Methods 2023](https://www.nature.com/articles/s41592-023-02018-3)
- gPlace: [docs](https://cmc-aau.github.io/gPlace/), [large trees and memory limits](https://cmc-aau.github.io/gPlace/advanced/memory.html), [validation](https://cmc-aau.github.io/gPlace/validation.html), [FAQ](https://cmc-aau.github.io/gPlace/faq.html), [how it works](https://cmc-aau.github.io/gPlace/how_it_works.html), [change log](https://cmc-aau.github.io/gPlace/changelog.html)
- Pricing (checked 2026-11-10): AWS r7i via [Vantage](https://instances.vantage.sh/aws/ec2/r7i.2xlarge), [Holori](https://calculator.holori.com/aws/ec2/r7i.8xlarge), [cloudprice.net](https://cloudprice.net/aws/ec2/instances/r7i.8xlarge); GCP via [Economize](https://www.economize.cloud/resources/gcp/pricing/compute-engine/n2-highmem-8/), [gcloud-compute.com](https://gcloud-compute.com/n2-highmem-8.html), [Google spot pricing](https://cloud.google.com/spot-vms/pricing); Hetzner via [hetzner.com](https://www.hetzner.com/cloud/general-purpose/), [trackvps](https://trackvps.com/hetzner-cloud-pricing), [webhosting.today](https://webhosting.today/2026/06/18/hetzners-price-increases-reached-209-the-30-headline-applied-to-a-different-tier/)
- Cloudflare: [R2 pricing](https://developers.cloudflare.com/r2/pricing/), [Workers limits](https://developers.cloudflare.com/workers/platform/limits/), [Containers limits](https://developers.cloudflare.com/containers/platform-details/limits/)
- Hosted GTDB-Tk: [KBase app](https://kbase.us/applist/apps/kb_gtdbtk/run_kb_gtdbtk_classify_wf/release), [Galaxy Europe](https://usegalaxy.eu/root?tool_id=gtdbtk_classify_wf), [Galaxy help thread](https://help.galaxyproject.org/t/gtdb-tk-database-galaxy-version-2-6-0-galaxy0/18605)
- Measurements on this page: GTDB-Tk 2.8.0 (Bioconda, with skani 0.4.0, pplacer 1.1.alpha19, Prodigal 2.6.3, HMMER 3.4) on the R232 mockup DB; skani 0.3.1 built from source; genomes E. coli K-12 MG1655 (GCF_000005845.2), P. aeruginosa PAO1 (GCF_000006765.1), GTDB's test genome G021783475, GTDB-Tk's `tests/data/genomes/genome_1.fna`; 4 threads on a 12-core Linux laptop.

## Status

Step 1 is built: [`gtdbprep`](../tools/gtdbprep.md). Two findings from building it:

- Besides `markers/`, `masks/` and `metadata/`, GTDB-Tk also opens `taxonomy/gtdb_taxonomy.tsv`
  during identify/align, only to reject genome IDs that clash with GTDB accessions. With random IDs
  the mock database's small stub is enough (77 MB in total for the client).
- To match `classify_wf`, the placement side must leave out genomes the ANI screen already assigned;
  otherwise their `msa_percent` and `translation_table` fields differ.
