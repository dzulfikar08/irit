# gPlace checks: same placements as pplacer, and how low --max_memory goes

Two follow-ups to [GTDB R232 placement in 8.5 GB](gtdb-placement-test.md), run on free GitHub-hosted
runners (4 vCPU, 16 GB RAM, swap off) on 2026-11-10:

1. **Accuracy.** Do real pplacer and gPlace give the same placements on the R232 trees that fit a
   16 GB runner?
2. **Memory caps.** How low can gPlace's `--max_memory` go on a class-level tree and on the full
   bacterial tree, under a kernel cap of the same size, and what does it cost in time?

## Summary

- **gPlace matched pplacer exactly.** On the ar53 tree (21 archaea) and the bacterial backbone
  (38 bacteria), every genome got the same edge list and every printed value was equal: likelihood,
  LWR, distal and pendant length. `gplace compare` reported "All values identical: 100%", and so did
  an independent per-genome check. That holds for gPlace untiled (backbone) and tiled at
  `--max_memory 8.5G` (both trees).
- **Plain pplacer does not fit 14 GiB on either tree.** It was OOM-killed while caching the
  reference tree, at 13.7 GiB of anonymous memory, on ar53 and on the backbone. With
  `--mmap-file` (what GTDB-Tk's `--scratch_dir` passes) it finished under the same 14 GiB cap.
  pplacer then took **694 s and 424 s**. gPlace at 8.5G took **30 s and 21 s**.
- **Class tree 8: the floor is 2 GiB.** Placements were identical at every cap down to `1.9G`
  (39 tiles). At `1.5G`, gPlace stops with a clear message ("needs `--max_memory` 1.9G or more").
  Smaller caps were not slower on 4 vCPU: 54 s at 8.5G, 41 s at 2G.
- **Full bacterial tree (379,594 nodes): the floor is about 5.2 GiB.** `5.2G` worked: 360 tiles,
  608 s, peak 4.77 GiB, placements identical to 8.5G. Below that, the floor is set by loading the
  reference package, not by the tiles. Before gPlace checks the limit, its main process already
  holds about 4.4–4.5 GiB of anonymous memory (4.8 GiB RSS):
  - with a kernel cap at or below 4.5 GiB, it is **OOM-killed** (SIGKILL) within 2–7 s;
  - with more room it exits cleanly, saying it needs 5.1G.
- **Practical minimum on a laptop:** with `--max_memory 6G`, every tree tested here fits in about
  5 GiB, with identical results. This is the number to plan a 16 GB laptop around. The full tree
  needs at least about 5.2G. A class-level tree with ~40 genomes goes down to 2G.
- **The ten `gtdb_r232_bac120_fasttree.log` copies are byte-identical** (MD5
  `4d17189d3aaeadf3f9b494255aa9d1d4`). Hard-linking them cuts the placement data from 7.37 GB to
  about 2.7 GB.

## Methods

Workflow [`gplace-checks.yml`](https://github.com/dzulfikar08/irit/blob/exp/gplace-checks/.github/workflows/gplace-checks.yml)
on branch [`exp/gplace-checks`](https://github.com/dzulfikar08/irit/tree/exp/gplace-checks)
(created from `main` at `ea1a788`), with helpers in
[`exp/gplace/`](https://github.com/dzulfikar08/irit/tree/exp/gplace-checks/exp/gplace):
`genomes.tsv`, `measure.sh` (the earlier cgroup measurer, plus a sampled sum of RSS) and
`jcheck.py` (an independent jplace comparison that doesn't use any gPlace code).

1. **Data.** The irit-data mirror (`gtdbtk-r232-placement-v1`) was not published, so the prep job
   streamed the 60.8 GB R232 archive as before (`curl | pigz -dc | tar -x`, without `skani/` and
   `msa/`), in 47 min this time. It then passed four sets of refpkgs to the other jobs as
   artifacts: ar53 plus backbone, class tree 8 (`gtdbtk.package.8`), and the full bac120 tree.
   Later runs reused those artifacts instead of streaming again.
2. **Query alignments.** GTDB-Tk 2.8.0 `identify` and `align --skip_gtdb_refs` on 59 genomes. The
   genomes are listed below. All 59 passed: 38 went to bac120 and 21 to ar53. The
   uncompressed user MSAs were the input to both placers.
3. **Placers.**
   - pplacer 1.1.alpha19 (`v1.1.alpha19-0-g807f6f3`, Bioconda build `h9ee0642_2`, the version
     GTDB-Tk 2.8.0 installs).
   - gPlace 0.1.1 from source at
     [`cmc-aau/gPlace@695a401`](https://github.com/cmc-aau/gPlace/commit/695a401e0b54cc1bde55a5e1498a0697899b15ea).
     That was still the newest commit on 2026-11-10. PyPI and Bioconda only have 0.1.0.
     `gplace check_install` passed.
4. **Same call as GTDB-Tk.** Both tools ran
   `pplacer -m WAG -j 4 -c REFPKG -o OUT.jplace MSA`, the argument list in GTDB-Tk 2.8.0
   `gtdbtk/external/pplacer.py`. With `--scratch_dir`, GTDB-Tk adds `--mmap-file FILE`. gPlace
   takes the same arguments, plus `--max_memory` where it is stated.
5. **Memory.** Each command ran in its own cgroup v2 group, with `memory.max` set to the cap and
   `memory.swap.max` = 0. Over the cap means an OOM kill, not swapping.
   - The tables report the cgroup's `memory.peak`, the OOM kill count and wall time.
   - **Use `memory.peak` for gPlace.** Its workers share their vectors in shared memory, so a
     sum of per-process RSS counts them several times (e.g. 29 GB summed for a 7.2 GiB peak). The
     cgroup's `anon` counter, on the other hand, leaves them out.
   - For pplacer in `--mmap-file` mode, `memory.peak` includes the mapped scratch file. That
     memory is page cache, which the kernel can reclaim.
6. **Comparison.** `gplace compare --jplace_a A --jplace_b B`, plus `jcheck.py`. The check compares
   the best edge, LWR, distal and pendant lengths and logL per genome, and compares whole rows for
   "identical". A row only counts as "same" within |ΔLWR| ≤ 1e-4 and Δlength ≤ 1e-5. No such rows
   were needed.

### Genomes

GTDB's test genome `G021783475`, plus 58 public genomes from NCBI. The NCBI genomes were picked with
the Datasets API (one reference genome per phylum where possible, highest CheckM completeness)
and include the 4 NCBI genomes of the earlier test. Bacteria (38):

| Accession | Organism | Lineage (NCBI) |
|---|---|---|
| G021783475 | GTDB test genome (*Thermodesulfobium*) | Thermodesulfobiota |
| GCF_000005845.2 | *Escherichia coli* K-12 MG1655 | Pseudomonadota |
| GCF_019048245.1 | *Enterobacter hormaechei* | Pseudomonadota |
| GCF_006094375.1 | *Staphylococcus epidermidis* | Bacillota |
| GCF_000062885.1 | *Saccharopolyspora erythraea* NRRL 2338 | Actinomycetota |
| GCF_019930665.1 | *Bacteroides nordii* | Bacteroidota |
| GCF_000317475.1 | *Phormidium nigroviride* PCC 7112 | Cyanobacteriota |
| GCF_000199675.1 | *Anaerolinea thermophila* UNI-1 | Chloroflexota |
| GCF_000217655.1 | *Treponema paraluiscuniculi* | Spirochaetota |
| GCF_000092105.1 | *Planctopirus limnophila* | Planctomycetota |
| GCF_000019665.1 | *Methylacidiphilum infernorum* V4 | Verrucomicrobiota |
| GCF_000020685.1 | *Deinococcus deserti* | Deinococcota |
| GCF_000230655.2 | *Thermotoga maritima* MSB8 | Thermotogota |
| GCF_000025605.1 | *Thermocrinis albus* | Aquificota |
| GCF_001687475.2 | *Campylobacter hepaticus* | Campylobacterota |
| GCF_001457555.1 | *Fusobacterium polymorphum* | Fusobacteriota |
| GCF_040235295.1 | *Chlamydia abortus* | Chlamydiota |
| GCF_000179915.2 | *Terriglobus saanensis* | Acidobacteriota |
| GCF_000020985.1 | *Thermodesulfovibrio yellowstonii* | Nitrospirota |
| GCF_000008305.1 | *Mesoplasma florum* L1 | Mycoplasmatota |
| GCF_000012925.1 | *Geobacter metallireducens* | Thermodesulfobacteriota |
| GCF_000022145.1 | *Anaeromyxobacter dehalogenans* | Myxococcota |
| GCF_000695095.2 | *Gemmatimonas phototrophica* | Gemmatimonadota |
| GCF_002355835.1 | *Ca.* Endomicrobiellum trichonymphae | Elusimicrobiota |
| GCF_000172975.1 | *Dethiosulfovibrio peptidovorans* | Synergistota |
| GCF_002300465.1 | *Fibrobacter intestinalis* | Fibrobacterota |
| GCF_000020965.1 | *Dictyoglomus thermophilum* | Dictyoglomota |
| GCF_000284335.1 | *Caldisericum exile* | Caldisericota |
| GCF_000025725.1 | *Denitrovibrio acetiphilus* | Deferribacterota |
| GCF_021057185.1 | *Ca.* Absconditicoccus praedator | CPR (Absconditabacteria) |
| GCF_000803625.1 | *Ca.* Nanosynbacter lyticus TM7x | CPR (Saccharimonadia) |
| GCA_988217875.1 | *Ca.* Microgenomatia bacterium (MAG, 2026) | CPR (Microgenomatota) |
| GCA_963663305.1 | *Ca.* Altimarinota bacterium (MAG) | CPR (Gracilibacteria) |
| GCF_000513475.1 | *Ca.* Babela massiliensis | Dependentiae |
| GCF_004353895.1 | *Ca.* Syntrophosphaera thermopropionivorans | Cloacimonadota |
| GCA_900465355.1 | *Bipolaricaulis sanaerobius* | Bipolaricaulota |
| GCA_985559455.1 | Margulisiibacteriota bacterium (MAG, 2026) | Margulisiibacteriota |
| GCA_988179205.1 | Omnitrophota bacterium (MAG, 2026, not in R232) | Omnitrophota |

Archaea (21):

| Accession | Organism | Lineage (NCBI) |
|---|---|---|
| GCF_000091665.1 | *Methanocaldococcus jannaschii* | Methanobacteriota |
| GCF_000016525.1 | *Methanobrevibacter smithii* | Methanobacteriota |
| GCF_000092185.1 | *Thermosphaera aggregans* | Thermoproteota |
| GCF_000019605.1 | *Ca.* Korarchaeum cryptofilum OPF8 | Korarchaeota |
| GCF_000018465.1 | *Nitrosopumilus maritimus* SCM1 | Nitrososphaerota |
| GCF_000008265.1 | *Picrophilus oshimae* | Thermoplasmatota |
| GCA_986311735.1 | Halobacteriota archaeon (MAG, 2026) | Halobacteriota |
| GCA_988083985.1 | Bathyarchaeota archaeon (MAG, 2026) | Bathyarchaeota |
| GCA_060611055.1 | *Hadarchaeum* sp. (2026) | Hadarchaeota |
| GCA_008000775.2 | *Promethearchaeum syntrophicum* | Asgard (Lokiarchaeia) |
| GCA_021513715.1 | *Ca.* Heimdallarchaeum endolithica | Asgard |
| GCA_020348985.1 | Thorarchaeota archaeon (MAG) | Asgard |
| GCF_023169545.1 | *Nanobdella aerobiophila* | DPANN (Nanoarchaeota) |
| GCF_002214165.1 | *Ca.* Mancarchaeum acidiphilum | DPANN (Micrarchaeota) |
| GCA_061245525.1 | Aenigmatarchaeia archaeon (MAG, 2026) | DPANN |
| GCA_963717515.1 | Woesearchaeota archaeon (MAG) | DPANN |
| GCA_026016245.1 | Pacearchaeota archaeon (MAG) | DPANN |
| GCA_002780595.1 | *Ca.* Huberarchaeum crystalense | DPANN |
| GCA_964527545.1 | Undinarchaeota archaeon (MAG) | DPANN |
| GCA_016699555.1 | *Ca.* Iainarchaeum sp. (MAG) | DPANN |
| GCA_988140495.1 | Altiarchaeota archaeon (MAG, 2026) | Altiarchaeota |

For two lineages the Datasets query returned the wrong kind of genome, so they were queried more
specifically:

- "Candidatus Korarchaeota" returned *Thermosphaera*, so Korarchaeum was queried by genus.
- "Methanobacteriota" returned *Haloferax*, which was dropped. Two methanogens were added by
  species name instead.

The prep log records each genome's download status, and all 58 downloaded.

## Experiment 1: pplacer vs gPlace

[Run 42814890542](https://github.com/dzulfikar08/irit/actions/runs/42814890542), job `accuracy`.
A first attempt in [run 42805650529](https://github.com/dzulfikar08/irit/actions/runs/42805650529)
gave the same pplacer OOM, then stopped because of a `bash -e` bug in the workflow.

| Run | Tree | Queries | Wall | Peak, cgroup `memory.peak` | Anonymous peak | OOM kill | Cap |
|---|---|---:|---:|---:|---:|---:|---:|
| pplacer (GTDB-Tk default) | ar53 (20,242 nodes) | 21 | killed at 44 s | 14.0 GiB (cap) | 13.65 GiB | 1 | 14 GiB |
| pplacer `--mmap-file` | ar53 | 21 | **694 s** | 14.0 GiB, mostly the mapped scratch file | 1.04 GiB | 0 | 14 GiB |
| gPlace untiled | ar53 | 21 | killed at 10 s (gPlace estimates 14.5 GiB) | 14.0 GiB | – | 1 | 14 GiB |
| gPlace `--max_memory 8.5G` (3 tiles) | ar53 | 21 | **30 s** | 6.12 GiB | – | 0 | 8.5 GiB |
| pplacer (GTDB-Tk default) | backbone (12,094 nodes) | 38 | killed at 33 s | 14.0 GiB (cap) | 13.72 GiB | 1 | 14 GiB |
| pplacer `--mmap-file` | backbone | 38 | **424 s** | 14.0 GiB, mostly the mapped scratch file | 0.74 GiB | 0 | 14 GiB |
| gPlace untiled | backbone | 38 | **15 s** | 9.80 GiB | – | 0 | 14 GiB |
| gPlace `--max_memory 8.5G` (2 tiles) | backbone | 38 | **21 s** | 5.57 GiB | – | 0 | 8.5 GiB |

| Comparison (A = pplacer) | Genomes | Best edge same | Edge list same | All values identical | Largest Δ on best placement |
|---|---:|---:|---:|---:|---|
| ar53: gPlace 8.5G | 21 | 21 | 21 | **21** | 0 (logL, LWR, distal, pendant) |
| backbone: gPlace untiled | 38 | 38 | 38 | **38** | 0 |
| backbone: gPlace 8.5G | 38 | 38 | 38 | **38** | 0 |

- **Per-genome detail.** `jcheck.py` gave "identical" for every genome. All 59 best placements have
  LWR 1. The per-genome tables are in the `accuracy-results` artifact (`jcheck_*.md`).
- **The pplacer reference ran in `--mmap-file` mode.** Plain pplacer did not fit a 14 GiB cap on
  either tree, so the brief's "skip it" rule would have left nothing to compare. GTDB-Tk supports
  this mode itself (`--scratch_dir`), and it changes where the likelihood vectors are stored, not
  how they are calculated. pplacer's true resident need in default mode is above 14 GiB on both
  trees. A 16 GB machine would not run either one without `--scratch_dir`.
- **Not done:** class-level and full-tree comparisons against pplacer. They need about 60–140 GB
  in default mode, as briefed. With `--mmap-file` they might run on a runner but would be slow;
  that is untested.

## Experiment 2: memory-cap sweep

Every row is `gplace place --max_memory CAP -m WAG -j 4 -c REFPKG -o OUT MSA` on the same 38
bacterial genomes, in a cgroup with `memory.max` = CAP (same bytes) and swap off. The
comparisons are against the 8.5G output on the same tree.

### Class-level tree 8 (32,446 nodes; gPlace's untiled estimate is 26.3 GiB)

[Run 42805650529](https://github.com/dzulfikar08/irit/actions/runs/42805650529) (one runner, run in
sequence) and, for 1.9G, [run 42814890542](https://github.com/dzulfikar08/irit/actions/runs/42814890542).

| Cap | Tiles (edges per tile) | gPlace's estimated peak | Peak, cgroup | Wall | OOM | vs 8.5G |
|---|---:|---:|---:|---:|---:|---|
| 8.5G | 4 (9,284) | 7.8 GiB | 7.24 GiB | 53.8 s | 0 | reference |
| 6G | 7 (6,785) | 5.6 GiB | 5.01 GiB | 49.4 s | 0 | 38/38 identical |
| 4G | 12 (3,672) | 3.8 GiB | 3.16 GiB | 46.4 s | 0 | 38/38 identical |
| 3G | 16 (2,429) | 2.8 GiB | 2.18 GiB | 42.5 s | 0 | 38/38 identical |
| 2G | 32 (1,164) | 1.9 GiB | 1.23 GiB | 40.8 s | 0 | 38/38 identical |
| 1.9G (second runner; 8.5G took 86.3 s there) | 39 (1,026) | 1.8 GiB | 1.14 GiB | 68.6 s | 0 | 38/38 identical |
| 1.5G | – | – | 0.43 GiB | 0.8 s | 0 | refused: "needs --max_memory 1.9G or more" |

### Full bacterial tree (379,594 nodes; gPlace's untiled estimate is 301.5 GiB)

Each cap ran on its own runner, so wall times include some differences between machines.
Runs: [42805650529](https://github.com/dzulfikar08/irit/actions/runs/42805650529) (8.5G, 6G, and
4G–1.5G), [42814890542](https://github.com/dzulfikar08/irit/actions/runs/42814890542) (5G, 4.5G, and
4G with a loose kernel cap), [42816548489](https://github.com/dzulfikar08/irit/actions/runs/42816548489)
(5.5G, 5.2G).

| Cap | Tiles (edges per tile) | Peak, cgroup | Wall | OOM | vs 8.5G |
|---|---:|---:|---:|---:|---|
| 8.5G | 77 (5,860) | 6.21 GiB | 840 s | 0 | reference |
| 6G | 181 (2,602) | 4.77 GiB | 643 s | 0 | 38/38 identical |
| 5.5G | 243 (1,880) | 4.77 GiB | 632 s | 0 | 38/38 identical |
| **5.2G** | **360 (1,277)** | 4.77 GiB | 608 s | 0 | **38/38 identical** |
| 5G | – | 4.77 GiB | 9.7 s | 0 | refused cleanly: "needs --max_memory 5.1G or more" |
| 4G, kernel cap 14 GiB | – | 4.77 GiB (4.40 GiB anonymous) | 9.5 s | 0 | refused cleanly, same message |
| 4.5G | – | 4.50 GiB (cap) | 7.1 s | **1** | SIGKILL while loading |
| 4G | – | 4.00 GiB (cap) | 7.0 s | **1** | SIGKILL while loading |
| 3G | – | 3.00 GiB (cap) | 4.3 s | **1** | SIGKILL while loading |
| 2G | – | 2.00 GiB (cap) | 2.4 s | **1** | SIGKILL while loading |
| 1.5G | – | 1.50 GiB (cap) | 2.1 s | **1** | SIGKILL while loading |

- **Floor:** 5.2G works, and 5.1G is gPlace's own minimum for 38 queries. On every cap from 5.2G to
  6G the peak is the same 4.77 GiB. That is the loading step (one process at 4.8 GiB RSS) and
  not the tiles. At 8.5G the tiles are larger, and the tiles set the peak (6.2 GiB).
- **Time:** going down to the floor did not make placement slower: 840 s at 8.5G, 608 s at 5.2G.
  On class tree 8, a single runner shows the same thing from 8.5G down to 2G. gPlace's docs give
  "tiled runs take about 1.7–2.5× longer" than untiled. Between tile sizes, smaller tiles were
  slightly faster here, with 4 vCPU and 38 queries. That may differ with many queries or many
  CPUs, where the docs' 10,000-genome numbers come from.

## What it means for the laptop plan

- **Placement accuracy is no longer a caveat.** On the two R232 trees where pplacer could be run,
  gPlace was identical to pplacer for 59 genomes from about 45 phylum-level lineages. Those include CPR, DPANN and Asgard
  genomes, and MAGs released after R232. Together with gPlace's own R226 validation, it is
  reasonable to use gPlace for GTDB-Tk placement on a laptop.
- **Set `--max_memory 6G` in the shim.** That kept every tree in this test and the earlier one at
  about 5 GiB, leaving room on a 16 GB laptop for the rest of GTDB-Tk and the desktop. (ar53 and
  the backbone were only measured at 8.5G, with peaks of 6.1 and 5.6 GiB. A lower cap tiles them
  further.)
- **Do not set the cap below about 5.2G when the full tree may be used.** A cgroup or container
  limit under about 4.8 GiB kills the run without gPlace's helpful message.
- **Placement data shrinks to about 2.7 GB:** the fasttree.log copies are identical, so the mirror
  can store one copy and link it into each refpkg.

## Possible upstream report (not sent)

Two things are worth sharing with the gPlace authors (cmc-aau/gPlace): the confirmation itself,
and the loading-memory behaviour on the full bacterial tree. A draft is in the report to the
user, in the conversation, and is not posted.

Reproduction for the second point:

- gPlace `695a401` (0.1.1), Python 3.12, ubuntu-latest runner (4 vCPU, 16 GB, swap off);
- refpkg `release232/pplacer/gtdb_r232_bac120.refpkg` from `gtdbtk_r232_data.tar.gz`;
- 38-genome bac120 user MSA from `gtdbtk align --skip_gtdb_refs` (artifact `msa` of run 42805650529);
- `gplace place --max_memory 4G -m WAG -j 4 -c gtdb_r232_bac120.refpkg -o out.jplace bac120.fasta`
  in a cgroup v2 group with `memory.max` = 4 GiB and `memory.swap.max` = 0: OOM-killed after
  7 s, last log line "Read 38 sequences". With `memory.max` = 14 GiB, the same command exits
  after 9.5 s with "needs --max_memory 5.1G or more". By then the cgroup's peak is 4.77 GiB, with
  4.40 GiB anonymous, and the main process's max RSS is 4.79 GiB.

## Sources

- Runs: [42805650529](https://github.com/dzulfikar08/irit/actions/runs/42805650529) (prep, class
  tree 8 sweep, full tree 8.5G and 6G), [42814890542](https://github.com/dzulfikar08/irit/actions/runs/42814890542)
  (accuracy, class 1.9G, full 5G, 4.5G, 4G), [42816548489](https://github.com/dzulfikar08/irit/actions/runs/42816548489)
  (full 5.5G and 5.2G). Branch commits `ac868ed`, `8f86020`, `e8bbb08`.
- gPlace: [docs](https://cmc-aau.github.io/gPlace/), [large trees and memory limits](https://cmc-aau.github.io/gPlace/advanced/memory.html), source at `695a401`.
- GTDB-Tk 2.8.0 `gtdbtk/external/pplacer.py` (pplacer arguments, `--mmap-file` from `--scratch_dir`).
- GTDB R232 [full_package](https://data.gtdb.ecogenomic.org/releases/release232/232.0/auxillary_files/gtdbtk_package/full_package/) and [mockup_db](https://data.gtdb.ecogenomic.org/releases/release232/232.0/auxillary_files/gtdbtk_package/mockup_db/).
- Genomes: NCBI Datasets v2, accessions above.
