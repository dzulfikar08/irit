# An AI agent redoes a 2025 paper with irit: three *K. pneumoniae* from alfalfa sprouts

On 2026-11-13, a Claude Code agent was given a recently published genome paper, the paper's raw
reads, and irit-mcp. It had no shell, no web access and no other analysis tools. Its task was to
check every value in the paper's Table 1 from the raw reads. Everything ran on the project laptop.

**Result:** every resistance gene and allele in Table 1 and every MLST type match, for all three
isolates. Assembly size, GC and Q30 are within 0.3 %. Read counts, contig counts and coverage
differ, for reasons the agent explained. GTDB R232 confirms the species, which the paper had
identified by MALDI-TOF only. Along the way the agent found six irit problems
([#14](https://github.com/dzulfikar08/irit/issues/14)–[#19](https://github.com/dzulfikar08/irit/issues/19)),
one of them serious. All six are fixed in the release after 0.3.5.

## The paper

Neumann T, Seemann A, Heß D, Schuh E, Projahn M. *Draft genome sequences of three multidrug-resistant
Klebsiella pneumoniae strains isolated from alfalfa sprouts in Germany.* Microbiology Resource
Announcements 15(2):e01270-25, published 22 December 2025.
[doi:10.1128/mra.01270-25](https://doi.org/10.1128/mra.01270-25), CC BY 4.0. Reads: SRA
SRR35254815–SRR35254817 (BioProject PRJNA1315124).

Why this paper:
- It is recent, open access, and its reads are public: Illumina MiSeq and NextSeq, 650 MB
  gzipped in total.
- Its methods are fastp, Shovill (SPAdes), QUAST, tseemann/mlst and NCBI AMRFinder, inside the
  BfR pipelines AQUAMIS 1.4.2 and BakCharak 3.1.6. irit-mcp wraps the same tools, in newer
  versions.
- Table 1 holds concrete numbers to check for each isolate. Only the virulence row needs tools
  irit doesn't have (VirulenceFinder/VFDB, Kleborate-style loci).

## Setup

| | |
|---|---|
| Agent | Claude Code 2.1.317, headless (`claude -p`), default model, built-in tools limited to Read/Write/Edit/Glob/Grep. The only MCP server was irit-mcp. |
| irit | v0.3.5 release binaries; irit-mcp started with `--root` set to the agent's folder |
| Tools | fastp 1.4.0, Shovill 1.4.2 (SPAdes 3.15.5), mlst 2.35.0, AMRFinderPlus 4.2.7, CheckM2 1.1.0, GTDB-Tk 2.8.0 (skani 0.3.1), via Bioconda environments |
| Databases | dblock: `checkm2-db-v3`, `gtdbtk-r232-ani-screen`, `gtdbtk-r232-placement-lite` (reused from the [laptop end-to-end run](e2e-laptop.md)), and `amrfinderplus-db-4.2-2026-10-29.1`, which the agent fetched |
| Limits | irit-mcp and every tool it started ran in an 8 GB, no-swap cgroup (`systemd-run --scope -p MemoryMax=8G -p MemorySwapMax=0`). The machine has 12 cores and was shared with other work. |
| Brief | Reproduce Table 1 row by row, and give a verdict and a reason for each value. Every number must come from a tool output; nothing from memory or the paper. Report irit problems. |

The agent wrote the report itself, in two sessions:
- **Session 1, 62 min, 46 turns.** It did QC, assembly, MLST and CheckM2. It could not run
  AMRFinderPlus or the GTDB screen. Two setup mistakes on our side caused that:
  `amrfinder_index` was missing from irit-mcp's PATH, and the brief didn't allow the GTDB
  server range reads that the species screen needs on a cold cache.
- **Session 2, 7 min, 14 turns.** After both were fixed, the same session was resumed. It fetched
  the AMR database, then ran AMRFinderPlus and the species screen.

The two sessions cost about US$4.50 at API prices. Compute steps, from the run records:

| Step | Time |
|---|---|
| fastp, per isolate | ~20 s |
| Shovill (SPAdes), per isolate | 7.5–11 min |
| mlst, per isolate | ~5 s |
| CheckM2, all three | ~5 min |
| AMRFinderPlus `--plus`, per isolate | ~33 s |
| gtdbprep ANI screen, all three (`species_only`, 5 sketches / 17 MB fetched) | ~35 s |

## Results against Table 1

Each value below was checked by hand against the output files.

| Table 1 row | BfR-EC-20867 (paper → ours) | BfR-EC-20869 | BfR-EC-20987 | Verdict |
|---|---|---|---|---|
| Reads (after trimming) | 1,370,298 → 1,399,916 | 1,938,111 → 1,978,008 | 2,271,682 → 2,335,252 | ours 2–3 % higher |
| Contigs | 75 → 71 (all) / 38 (≥500 bp) | 111 → 128 / 58 | 77 → 146 / 55 | differ |
| Q30 fraction | 0.933 → 0.931 | 0.941 → 0.940 | 0.877 → 0.874 | close |
| Coverage | 49.0 → 50.4 | 68.4 → 70.3 | 60.9 → 63.1 | ours ~3 % higher (bases / length) |
| N50 (bp) | 474,715 → 661,397 | 350,263 → 364,313 | 304,234 → 304,235 | 1 of 3 equal |
| Length (bp) | 5,459,041 → 5,463,998 | 5,525,422 → 5,528,095 | 5,442,817 → 5,454,508 | within 0.21 % |
| GC (%) | 57.06 → 57.04 | 57.15 → 57.13 | 57.35 → 57.34 | close |
| MLST | ST716 → ST716 | ST219 → ST219 | ST189 → ST189 | **match** (7/7 exact alleles) |
| Aminoglycosides | aph(3'')-Ib, aph(6)-Id | aadA2, aph(3'')-Ib, aph(3')-Ia, aph(6)-Id | aadA2, aph(3'')-Ib, aph(6)-Id | **match** |
| Beta-lactams | blaCTX-M-15, blaSHV-27, blaTEM-1 | blaCTX-M-15, blaSHV-1 | blaCTX-M-15, blaSHV-80 | **match** (exact alleles) |
| Sulfonamides | sul2 | sul1, sul2 | sul1, sul2 | **match** |
| Quinolones | oqxA, oqxB19, qnrB1 | oqxA10, oqxB5, qnrS1 | oqxA, oqxB, qnrS1 | **match** |
| Trimethoprim | dfrA14 | dfrA12 | dfrA12 | **match** |
| Fosfomycin | fosA | fosA | fosA | **match** |
| Tetracycline | none | none | tet(A) | **match** |
| Virulence | 8 markers | 8 markers | 7 markers | ybt only (present in 20867/20869, absent in 20987, as in the paper); the rest can't be checked with irit |

AMRFinderPlus found some genes Table 1 doesn't list: the chromosomal efflux gene emrD in all three
isolates, and mph(A)/mrx(A) (macrolides) in 20869 and 20987. Table 1 has no macrolide row.

**Species and quality.** GTDB R232 places all three in `s__Klebsiella pneumoniae` (closest
reference GCF_000742135.1, ANI 98.97–99.14 %, AF 0.86–0.87). The next nearest species, *K.
africana*, is at ANI ≤95.18 %. CheckM2 rates all three 100 % complete, with 0.08–0.21 %
contamination.

**Why some values differ.** These are the agent's explanations, which we agree with. None could be
tested without the paper's exact pipeline.
- **Reads:** our raw counts are already above the paper's. Paper ÷ ours is 0.97–0.98 for all three
  isolates, which points to a stricter read filter in AQUAMIS than fastp 1.4.0's defaults, rather
  than different data. The ENA MD5s matched. **Coverage** follows the same factor.
- **Contigs and N50:** we used Shovill 1.4.2, the paper used 1.1.0. No single length cutoff fits
  all three isolates, so the assemblies themselves differ. SPAdes splits contigs at repeats
  differently with small changes in reads or version. BfR-EC-20987's 150 bp NextSeq reads gave
  the most extra short contigs, and its N50 contig is the same as the paper's to within 1 bp.

## What it says about irit

**irit makes the computational half cheap. It doesn't make sequencing cheap.** The reads came off
an Illumina sequencer, and that wet-lab half (DNA extraction, library prep, the machine and its
reagents) is still the expensive part. Everything after the read files, this paper's whole Table
1 except virulence, ran on a laptop.

- **The tool coverage fits a real paper.** Every step of a typical isolate announcement, except
  virulence typing, ran through irit-mcp on a laptop in about 40 minutes of compute. It fit in
  8 GB.
- **An agent with only irit's tools reported honestly.** It left the rows it couldn't measure as
  "not checked". It used no gene names from the paper or from memory, and it separated our setup
  mistakes from irit's bugs once told about them. Reading the paper first did not make it copy
  values: every value in its table traces to a file.
- **What got in its way** (now issues, all fixed after 0.3.5):
  - [#14](https://github.com/dzulfikar08/irit/issues/14): resuming with the same `--run-id`
    overwrote the first session's run record, and irit-mcp said nothing. The first session's
    record survives only because `describe_run`'s output had been saved.
  - [#15](https://github.com/dzulfikar08/irit/issues/15): isotable doesn't read the ANI screen's
    species call, and doesn't report GC for Shovill assemblies.
  - [#16](https://github.com/dzulfikar08/irit/issues/16): database paths had to be copied from
    `dblock_env` by hand instead of being taken from the project's lock.
  - [#17](https://github.com/dzulfikar08/irit/issues/17): dblock checks that the AMRFinderPlus
    indexer is installed only *after* downloading. It also doesn't say that Bakta is missing
    before a 1.3 GB fetch.
  - [#18](https://github.com/dzulfikar08/irit/issues/18): CheckM2's scikit-learn version warning
    and AMRFinderPlus's CONDA_PREFIX warning aren't reported.
  - [#19](https://github.com/dzulfikar08/irit/issues/19): `describe_run` output is too large for
    an MCP client, plus three smaller messages.
- **Gaps that are features, not bugs:** irit has no QUAST, no read-mapping coverage tool, and no
  virulence or Kleborate-style typing. Those three would cover the rest of this paper's Table 1.

The agent's own report, the brief, the launcher and both transcripts are kept with the run
(outside this repository).
