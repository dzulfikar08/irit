# Metagenome to MAGs on an 8 GB laptop, through irit-mcp

Measured on 2026-11-13 on the project laptop (12 cores, 30 GB RAM), with every analysis step run
through irit-mcp inside an 8 GB, no-swap memory cap. Numbers are **measured** unless marked
**cited**. The section [Choosing the tools](#choosing-the-tools-cited) is all cited.

## Summary

- **The route works within 8 GB.** irit-mcp now wraps MEGAHIT (assembly), CoverM (read depth per
  contig) and MetaBAT2 (binning). With CheckM2, the GTDB ANI screen and isotable's MAG mode, which
  were already there, a metagenome goes from reads to graded, classified MAGs, recorded and
  replayable. The largest step peaks were MEGAHIT at 4.65 GiB, CheckM2 at 6.2 GiB and GTDB-Tk
  placement at 5.6 GiB (cgroup `memory.peak`, page cache included).
- **One step did not fit at its defaults.** CheckM2 on 23 bins was killed by the 8 GB cap: DIAMOND
  at its default block size. With `lowmem` it peaked at 5.6 GiB and took 197 s. irit-mcp now says
  so when that happens.
- **A CPR-rich groundwater sample gave real MAGs.** One Crystal Geyser filter sample (1.96 Gb of
  2×150 reads) gave 23 bins: 4 `high*`, 13 medium and 6 low by isotable's MIMAG tiers. Eight are
  Patescibacteriota (CPR): one `high*`, six medium, one low. Two are archaea from the DPANN-rich
  part of the tree (Nanobdellota, Altiarchaeota). 20 of the 23 match a GTDB R232 species at ANI
  99.2-99.99%: GTDB already holds published MAGs from this site, so those are a check that the
  laptop route recovers the same genomes.
- **A diverse, shallow groundwater sample did not.** Hainich well H4-1 (two MiSeq runs, 1.35 Gb
  after trimming) assembled within 4.65 GiB, but into short contigs (N50 465 bp; 6.8 Mb in contigs
  of 2.5 kb or more). Only 39% of the reads mapped back. Its bins were composites (CheckM2
  contamination 23-81%), even with depth from five Hainich samples. Memory was not the limit there:
  sequencing depth against the community's diversity was.
- **Two gaps in irit.** GTDB-Tk's `classify_wf` can't place genomes on dblock's placement-lite data
  (it leaves out the reference MSA that `classify_wf` always aligns against), so the three bins the
  ANI screen couldn't assign were placed outside irit-mcp. And SemiBin2, the better binner in
  benchmarks, is not wrapped yet.

## What was run

### Data

| Sample | Reads | Notes |
|---|---|---|
| Crystal Geyser, Utah: SRA SRR1556897 (BioProject PRJNA229517) | Illumina HiSeq 2000, 2×150; 13,037,142 reads, 1.96 Gb; 9,700,226 reads and 1.45 Gb after fastp | "Crystal Geyser Metagenome CG2 sample 0.2 micron filter", collected 2009-11-08 (ENA metadata). Deep, CO2-driven groundwater studied by the Banfield lab, whose papers describe many candidate-phyla (CPR and DPANN) genomes from it (**cited**: [Emerson et al. 2016](https://doi.org/10.1111/1462-2920.12817), [Probst et al. 2016](https://doi.org/10.1111/1462-2920.13362)). |
| Hainich CZE, Germany: ENA PRJEB28738, sample PNK66-H4-1, runs ERR2809130 and ERR2809132 | Illumina MiSeq, 2×300; 8.78 M reads, 2.72 Gb; 8.06 M reads, 1.35 Gb after fastp (adapter trimming of short inserts halved the bases) | Shallow-aquifer groundwater from the Küsel lab's transect (**cited**: [Wegner et al. 2019](https://doi.org/10.1128/AEM.02346-18)). Plus one run each of PNK66-H4-2 (ERR2809139), -H5-1 (ERR2809133), -H3-2 (ERR2809136) and -H5-2 (ERR2809145), only to give the binner depth across samples. |

All read files were fetched from ENA over HTTPS and their MD5s matched ENA's.

### Versions and setup

| | |
|---|---|
| irit | branch `mag-route` (irit-mcp 0.3.5 plus the new wrappers), release build |
| Tools (Bioconda, micromamba) | fastp 1.4.0; MEGAHIT 1.2.9; CoverM 0.8.0 (strobealign 0.20.0, samtools 1.25); MetaBAT 2.18; CheckM2 1.1.0 (database v3); GTDB-Tk 2.8.0 with gPlace replacing pplacer, as in [the end-to-end run](e2e-laptop.md); gtdbprep (irit) |
| Databases | dblock: `checkm2-db-v3`, `gtdbtk-r232-ani-screen`, `gtdbtk-r232-placement-lite`, locked in each project and found through `dblock.lock` |
| Memory cap | Each irit-mcp session ran in a transient systemd user service with `MemoryMax=8G`, `MemorySwapMax=0`; each step in its own child cgroup, also capped at 8G (the driver of the end-to-end run). Peaks below are that step's `memory.peak` (page cache included) and the sampled peak of anonymous plus shared memory. |
| Threads | 6 per step |
| Load | The laptop was shared with other work. The Hainich assembly ran at a 1-minute load average of 12-68 on 12 cores; the Crystal Geyser steps at 1-23. |

## Results

### Crystal Geyser: time and memory

| Step | Wall | Peak (cgroup) | Peak (anon) | Notes |
|---|---|---|---|---|
| fastp | 58 s | 2.39 GiB | 1.24 GiB | |
| MEGAHIT, `memory_gb: 6` | 36.7 min | 4.29 GiB | 2.00 GiB | 176,334 contigs, 124.4 Mb, N50 782 bp, longest 254,943 bp; 4,842 contigs of 2.5 kb or more (29.1 Mb) |
| CoverM, metabat depth | 77 s | 4.39 GiB | 3.88 GiB | 73.3% of reads mapped back |
| MetaBAT2 | 2.6 s | 0.24 GiB | 0.24 GiB | 23 bins, 24.4 Mb |
| CheckM2, defaults | 75 s, **killed** | 7.97 GiB | 7.80 GiB | DIAMOND (block size 2.0) hit the 8 GB cap |
| CheckM2, `lowmem` | 197 s | 5.58 GiB | 2.71 GiB | |
| gtdbprep ani, `species_only` | 39 s | 2.03 GiB | 0.26 GiB | 20 of 23 bins assigned |
| GTDB-Tk placement of the other 3 (outside irit-mcp, see below) | 4.6 min | 5.59 GiB | 5.43 GiB | identify, align `--skip_gtdb_refs`, classify with gPlace at `--max_memory 6G` |
| isotable, MAG mode | 0.1 s | | | |

### Crystal Geyser: the bins

Completeness and contamination from CheckM2; tier from isotable's MAG mode (`high*` = MIMAG's
completeness and contamination thresholds for high quality, without its rRNA and tRNA check);
species from the ANI screen, or the deepest rank from tree placement.

| Bin | Mb | Contigs | N50 | Complete % | Contam. % | Tier | Phylum | GTDB species / genus | Assigned by |
|---|---|---|---|---|---|---|---|---|---|
| bin.1 | 0.45 | 99 | 4,661 | 63.04 | 2.32 | medium | Patescibacteriota | UBA1547 sp001873625 | ANI 99.96 (AF 0.9641) |
| bin.2 | 0.89 | 190 | 4,964 | 78.73 | 3.42 | medium | Patescibacteriota | CG2-30-37-29 sp001873215 | ANI 99.96 (AF 0.7348) |
| bin.3 | 2.29 | 24 | 156,245 | 99.99 | 0.04 | high* | Pseudomonadota | Ghiorsea sp002323245 | ANI 99.99 (AF 0.9948) |
| bin.4 | 0.54 | 140 | 3,803 | 25 | 0.06 | low | Pseudomonadota | Parasulfuritortus sp002782005 | ANI 99.86 (AF 0.9428) |
| bin.5 | 0.81 | 50 | 22,923 | 95.48 | 0.3 | high* | Patescibacteriota | UBA1539 sp002788135 | ANI 99.89 (AF 0.9592) |
| bin.6 | 2.22 | 249 | 11,637 | 96.54 | 2.91 | high* | Pseudomonadota | Gallionella sp002780675 | ANI 99.25 (AF 0.856) |
| bin.7 | 1.07 | 251 | 4,430 | 56.61 | 4.59 | medium | Ratteibacteria | UBA8468 sp002791075 | ANI 99.76 (AF 0.7743) |
| bin.8 | 0.72 | 101 | 7,823 | 77.24 | 0.71 | medium | Patescibacteriota | UBA1544 sp002774425 | ANI 99.91 (AF 0.958) |
| bin.9 | 0.64 | 167 | 3,559 | 51.53 | 0.1 | medium | Pseudomonadota | CG1-02-64-396 sp001872725 | ANI 99.95 (AF 0.8897) |
| bin.10 | 0.77 | 135 | 6,859 | 82.45 | 0.55 | medium | Patescibacteriota | UBA1545 sp002771035 | ANI 99.89 (AF 0.9529) |
| bin.11 | 0.84 | 227 | 3,552 | 86.88 | 41.71 | low | Patescibacteriota | UM-FILTER-39-64 sp002784365 | ANI 98.42 (AF 0.537) |
| bin.12 | 0.25 | 55 | 4,909 | 9.99 | 0.08 | low | (unclassified) | Unclassified Bacteria | placement |
| bin.13 | 1.48 | 365 | 3,914 | 53.67 | 4.5 | medium | Pseudomonadota | g. Sideroxyarcus | placement |
| bin.14 | 0.85 | 216 | 4,010 | 46.33 | 0.45 | low | Elusimicrobiota | CG1-02-37-114 sp001871125 | ANI 99.82 (AF 0.7958) |
| bin.15 | 2.11 | 181 | 18,164 | 89.23 | 0.97 | medium | Pseudomonadota | UBA1543 sp002779605 | ANI 99.95 (AF 0.9865) |
| bin.16 | 0.58 | 121 | 5,274 | 71.57 | 3.25 | medium | Patescibacteriota | g. OLB19 | placement |
| bin.17 | 1.06 | 265 | 4,121 | 42.27 | 0.45 | low | Pseudomonadota | UBA1543 sp001871195 | ANI 99.97 (AF 0.9049) |
| bin.18 | 0.45 | 104 | 4,787 | 72.81 | 2.52 | medium | Patescibacteriota | CG2-30-45-37 sp001873515 | ANI 99.53 (AF 0.7254) |
| bin.19 | 1.16 | 259 | 4,683 | 67.61 | 0 | medium | Nitrospirota | UBA1546 sp001871685 | ANI 99.87 (AF 0.8207) |
| bin.20 | 0.54 | 135 | 3,990 | 38.24 | 1.6 | low | Nanobdellota (archaea) | UBA92 sp002497285 | ANI 99.95 (AF 0.7397) |
| bin.21 | 2.28 | 74 | 48,908 | 97.24 | 0.43 | high* | Pseudomonadota | Hydrogenovibrio sp002779995 | ANI 99.91 (AF 0.9399) |
| bin.22 | 1.25 | 204 | 7,257 | 77.95 | 1.66 | medium | Altiarchaeota (archaea) | Altiarchaeum hamiconexum | ANI 99.89 (AF 0.9338) |
| bin.23 | 1.11 | 134 | 10,698 | 88.68 | 0.97 | medium | Omnitrophota | Taenaricola geysiri | ANI 99.84 (AF 0.9253) |

What this shows:

- **The CPR bins are small, as expected:** 0.45-0.89 Mb. Their completeness is likely
  underestimated: CheckM2 handles reduced genomes better than marker-gene sets, but a lineage far
  from its training data is its hardest case (**cited**, below). isotable's MAG mode adds a note to
  every Patescibacteria row saying so.
- **bin.11 is two genomes.** 86.9% complete with 41.7% contamination, and its ANI match covers only
  54% of it (AF 0.537): a bin to split or discard, which isotable grades `fail`.
- **The ANI matches are a re-derivation check, not discovery.** Several GTDB species here
  (`CG1-02-…`, `CG2-30-…`, *Taenaricola geysiri*, *Altiarchaeum hamiconexum*) are named after or
  come from the Crystal Geyser studies, so a matching laptop MAG at 99.8-99.99% ANI recovers the
  same population from the same site.

### Hainich H4-1: time and memory

| Step | Wall | Peak (cgroup) | Peak (anon) | Notes |
|---|---|---|---|---|
| fastp, per run | 58-82 s | 2.19-2.49 GiB | 1.29-1.30 GiB | |
| MEGAHIT, `memory_gb: 6`, both runs | 2 h 43 min | 4.65 GiB | 4.05 GiB | at load 12-68; 272,672 contigs, 132.7 Mb, N50 465 bp, longest 25,510 bp |
| CoverM, the two runs | 123 s | 2.99 GiB | 2.18 GiB | 38.5% and 40.0% of reads mapped |
| CoverM, five samples | 221 s | 2.75 GiB | 2.33 GiB | H4-1 38.5%; H4-2 8.8%; H5-1 17.5%; H3-2 7.2%; H5-2 12.3% |
| MetaBAT2 (4 variants) | 1-5 s | ≤0.21 GiB | | 1-3 bins each |
| CheckM2 | 191-204 s | 6.09-6.21 GiB | 4.69-5.24 GiB | |

The bins:

| Depth from | `min_contig` | Bins | CheckM2 (complete / contamination %) |
|---|---|---|---|
| H4-1 only (its two runs) | 2,500 | 1 bin, 6.19 Mb | not run |
| H4-1 only | 1,500 | 2 bins: 0.37 Mb, 10.16 Mb | 17.8 / 0.0; 100.0 / 80.7 |
| five samples | 2,500 | 2 bins: 4.24 Mb, 3.38 Mb | not run |
| five samples | 1,500 | 3 bins: 4.07, 6.92, 0.37 Mb | 79.9 / 22.7; 92.9 / 39.3; 18.6 / 0.0 |

None is a usable MAG. The assembly is the limit: with an N50 of 465 bp and most reads not mapping
back, most of the community was sequenced too thinly to assemble, and the few long contigs pool
into composite bins. Sequencing much deeper or co-assembling many samples is the usual answer
for such a community; the second needs more memory and is the next thing to measure.

### Reproducibility

- **MEGAHIT:** two identical runs in the real-tools workflow gave the same contig count, length and
  N50 (92 contigs, 3,384,504 bp, N50 86,866 on a two-genome mock), but not byte-identical contigs.
  Its outputs are recorded as volatile, as Shovill's are, and replay compares the assembly's stats.
- **CoverM:** three runs on the same assembly and reads gave byte-identical depth tables (6
  threads).
- **MetaBAT2:** with irit-mcp's default seed of 1, a second run on the Crystal Geyser data gave
  byte-identical bins and `bin.BinMembers.txt`; `bin.BinInfo.txt` differed only by the output
  folder named in it. MetaBAT2's own default seed (0) draws a random one.

## What changed in irit

- `megahit`, `coverm_contig` and `metabat2` in irit-mcp, pinned and documented in
  [irit-mcp](../tools/mcp.md#megahit), with stand-in tests and a two-genome mock metagenome in the
  real-tools workflow.
- MEGAHIT's memory defaults to 4 GB, passed in bytes. MEGAHIT's own default is 90% of the
  machine's RAM, which ignores a memory cap. MetaBAT2's seed defaults to 1.
- A failed CheckM2 whose DIAMOND child was SIGKILLed now returns a `hint` to use `lowmem`; any tool
  killed by a signal gets a general out-of-memory hint.

## Open questions

- **Placement on placement-lite.** `gtdbtk_classify_wf` always aligns against GTDB's reference MSA
  (`msa/gtdb_r232_bac120.faa`), which placement-lite leaves out, so a bin the ANI screen can't
  assign fails. Either add `msa/` to the dblock entry, or give irit-mcp the identify → align
  `--skip_gtdb_refs` → classify route the end-to-end run used.
- **SemiBin2.** Its pretrained single-sample models may fit 8 GB and bin better than MetaBAT2.
- **Co-assembly.** How large a co-assembly fits MEGAHIT under 8 GB, and whether it rescues
  diverse, shallow samples like Hainich H4-1.
- **Dereplication** (galah) once several samples are binned separately.

## Choosing the tools (cited)

Everything in this section comes from the papers and benchmarks linked; nothing here was measured
on the laptop.

| Step | Chosen | Why | Not chosen |
|---|---|---|---|
| Assembly | MEGAHIT 1.2.9 | In benchmarks it uses far less memory than metaSPAdes: 8.7 times less on average in [Zhang et al. 2023](https://academic.oup.com/bib/article/24/2/bbad087/7077274), and 11 GB against metaSPAdes' 157 GB on one mid-sized dataset in [van der Walt et al. 2017](https://link.springer.com/article/10.1186/s12864-017-3918-9). Its memory cap (`-m`) is the one knob a laptop needs ([Li et al. 2015](https://doi.org/10.1093/bioinformatics/btv033)). | metaSPAdes ([Nurk et al. 2017](https://doi.org/10.1101/gr.213959.116)): often better contigs, but its memory needs rule out 8 GB for anything but small samples. |
| Depth | CoverM 0.8.0, `contig --methods metabat` | One call maps reads (strobealign by default) and writes MetaBAT's depth table from the streamed alignments, without a BAM file on disk ([Aroney et al. 2025](https://doi.org/10.1093/bioinformatics/btaf147)). | MetaBAT's own `jgi_summarize_bam_contig_depths`: needs a sorted BAM from a separate mapper and samtools first. |
| Binning | MetaBAT 2.18 | The lowest peak memory of the binners in a 2025 benchmark, about 2 GB ([Briefings in Bioinformatics 2025](https://academic.oup.com/bib/article/26/6/bbaf617/8339791)); maintained, one C++ program ([Kang et al. 2019](https://doi.org/10.7717/peerj.7359)). | SemiBin2 ([Pan et al. 2023](https://doi.org/10.1093/bioinformatics/btad209)) and COMEBin recover more high-quality bins in that benchmark, but SemiBin2 peaked at 24-37 GB there. SemiBin's pretrained single-sample models are much lighter (4.4 GB peak in [the SemiBin1 paper](https://www.nature.com/articles/s41467-022-29843-y)). |
| Dereplication | none yet | One sample gives one set of bins: nothing to dereplicate. | dRep ([Olm et al. 2017](https://doi.org/10.1038/ismej.2017.126)) or galah (skani-based, [Shaw and Yu 2023](https://doi.org/10.1038/s41592-023-02018-3)) once several samples are binned separately. |
| Quality | CheckM2 1.1.0 (already in irit-mcp) | Machine-learning models that don't depend on lineage-specific marker sets ([Chklovski et al. 2023](https://doi.org/10.1038/s41592-023-01940-w)). That matters for reduced genomes such as Patescibacteria (CPR), which lack single-copy genes the older marker-set approach expects: CheckM 1 rated one complete, single-scaffold CPR genome 70.1% complete ([Chen et al. 2020](https://www.biorxiv.org/content/10.1101/808410v2.full.pdf)), and groundwater studies have rechecked CPR MAGs with CheckM2 ([npj Biofilms and Microbiomes 2024](https://www.nature.com/articles/s41522-024-00615-4)). The CheckM2 authors note that a genome from a new phylum is still its hardest case. | CheckM 1 with the 43-marker CPR set of [Brown et al. 2015](https://doi.org/10.1038/nature14486). |
| Tiers | MIMAG ([Bowers et al. 2017](https://doi.org/10.1038/nbt.3893)) | High quality: >90% complete, <5% contamination, plus 23S, 16S and 5S rRNA and at least 18 tRNAs; medium: ≥50% and <10%; low: <50%, <10%. isotable's MAG mode tiers on completeness and contamination only, so its "high" is `high*`, and a bin at 10% contamination or more is `low`. | |
| Taxonomy | gtdbprep ani + GTDB-Tk 2.8.0 on GTDB R232 (already in irit-mcp) | The laptop route of [the end-to-end run](e2e-laptop.md): the ANI screen first, then tree placement for bins the screen can't assign. | |
