# Nanopore reads to isolate results on a laptop

Can a lab with a MinION and an ordinary laptop go from nanopore reads to the results an isolate
paper reports: assembly, completeness, sequence type, resistance genes and GTDB species? This page
measures that through `irit-mcp` (the new `flye` and `medaka` tools) on one public *Klebsiella
pneumoniae* isolate. It compares the result with the same isolate's Illumina reads and with a
published hybrid analysis.

Everything marked **measured** was run on the project laptop on 2026-11-13. **Cited** numbers come
from the sources linked next to them.

## Summary

- **The ONT-only route fits in 8 GB.** Flye then Medaka ran in a kernel-enforced 8 GB cgroup with
  swap off, and nothing was OOM-killed (`memory.events`: `max 0`, `oom_kill 0`). Flye peaked at
  3.85 GiB and Medaka at 7.36 GiB, the closest step to the cap (**measured**).
- **Time.** Flye took 22 min and Medaka 9.4 min on 6 threads, on a 12-core machine shared with
  other builds (load average 15–55). The whole run, with an Illumina comparison and all typing,
  took 74 min of wall clock (**measured**).
- **Same answers as Illumina.** The ONT-only assembly (Flye + Medaka) and the Illumina assembly
  (Shovill) agree:
  - **ST101**, all 7 alleles exact.
  - **The same 49 AMRFinderPlus elements**, with identical method, identity and coverage for every
    one. These include *blaKPC-3*, *blaOXA-232*, *blaCTX-M-15*, *armA* and the *gyrA*/*parC*
    quinolone mutations.
  - **CheckM2:** 100 % complete; contamination 0.44 % (ONT) vs 0.38 % (Illumina).
  - **Species:** `s__Klebsiella pneumoniae`, ANI 98.92 % (ONT) vs 98.96 % (Illumina).
  - **The unpolished Flye draft** gave the same ST and the same 49 elements as well.
- **Far more contiguous.** The ONT assembly has 11 contigs, including a 5.49 Mb chromosome contig
  (not closed into a circle), against 127 contigs and N50 174 kb from Illumina.
- **Matches the published analysis.** Our assemblies contain every element the study's Autocycler
  assembly of these reads has (45/45). The four extra elements show up in our Illumina assembly
  too, so they come from the newer AMRFinderPlus database, not from nanopore errors.
- **What the laptop doesn't do:** basecalling. These reads arrived already basecalled with the
  `sup` model. Basecalling is GPU work, and on a CPU it is roughly 30× slower (see
  [basecalling](#basecalling)).

## Why Flye + Medaka (cited)

- **Consensus assembly is the most accurate long-read-only method, but too heavy for a laptop.**
  Autocycler (Wick, Howden & Stinear, *Bioinformatics* 2025,
  [doi:10.1093/bioinformatics/btaf474](https://doi.org/10.1093/bioinformatics/btaf474); bioRxiv
  [10.1101/2025.05.12.653612](https://doi.org/10.1101/2025.05.12.653612)) builds a consensus from
  many input assemblies. Each Autocycler assembly typically had fewer than 10 errors, mostly
  homopolymer lengths, and fewer with R10.4.1 and `sup` basecalls.
- **The trade-off in numbers** (Nagy et al., *Microbial Genomics* 2026, 12(2):001631,
  [doi:10.1099/mgen.0.001631](https://doi.org/10.1099/mgen.0.001631)). This study compared
  assemblers on 92 Enterobacterales isolates, mostly *E. coli* and *K. pneumoniae*, with R10.4.1
  `sup` reads and Illumina reads:
  - **Autocycler + Medaka** was the most accurate long-read-only route: median 0 errors per
    genome, and 51/92 genomes error-free. Its consensus combined 20 assemblies (4 each from Canu,
    Flye, Raven, Miniasm and Hybracter).
  - **Flye** was median Q61 unpolished and Q67 after Medaka. Medaka mainly removed indels.
  - **Plasmids:** Flye recovered 56 % of the reference plasmids, the other assemblers 93–96 %.
  - **Typing:** no significant difference between long-read-only and hybrid assemblies in
    resistance, virulence or stress gene counts, or in MLST.
  - **Compute:** the whole comparison took 13,428 CPU-hours.
- **So irit starts with Flye + Medaka.** Each is one program with a pinned Bioconda version, and
  both fit 8 GB. Autocycler's 20 assemblies per isolate would take hours on a laptop, and Canu
  alone is slow. The `flye` tool returns each contig's circularity and depth, so a missing small
  plasmid is visible rather than silent.

## Data

| | |
|---|---|
| Isolate | *K. pneumoniae* AF47, carbapenemase-producing, from a rectal screening swab (NEKSUS study, England, 2024) |
| ONT | ENA [ERR15424148](https://www.ebi.ac.uk/ena/browser/view/ERR15424148): PromethION, R10.4.1, Rapid Barcoding Kit V14, Dorado `sup` methylation-aware v5.0.0 model (**cited**, Nagy et al.); 84,759 reads, 385.8 Mb (64× of the 6.05 Mb assembly), 309 MB gzipped; leanqc Q20 84.8 % of bases (**measured**) |
| Illumina | ENA [ERR15422825](https://www.ebi.ac.uk/ena/browser/view/ERR15422825): NovaSeq X, 2×150, 9.36 M reads, 730 MB gzipped |
| Published results | BioProject PRJEB93885. Per-assembler AMRFinderPlus, MLST, CheckM2, contig and accuracy tables on FigShare, [doi:10.6084/m9.figshare.29584931](https://doi.org/10.6084/m9.figshare.29584931) |

Why AF47:
- It is the second-smallest *K. pneumoniae* download in the set (1.04 GB for both read sets).
- Its ONT depth (64×) is close to the study's 60× subsample, and to what a multiplexed MinION run
  gives.
- It is a hard genome. In the study, no assembler closed its chromosome: Flye gave 13 contigs (N50
  3.03 Mb), and Autocycler 171.
- Its resistance profile (KPC-3, OXA-232, CTX-M-15, ArmA, QRDR mutations) tests whether
  long-read-only assemblies get the genes and point mutations right.

The ENA FASTQ headers are bare (`@ERR15424148.1 <uuid>`). Dorado's basecalling-model tag is gone,
so Medaka's automatic model choice and `--bacteria` can't work, and the model has to be named. We
used `r1041_e82_400bps_sup_v5.0.0`, which matches the basecaller model. The bacterial methylation
model is the alternative.

## Setup (measured)

| | |
|---|---|
| irit | `nanopore-assembly` branch, `irit-mcp` debug build, run by a scripted MCP client (one stdio session, calls in order) |
| Limits | `systemd-run --user -p MemoryMax=8G -p MemorySwapMax=0`; the client sampled the unit's `memory.current` every 0.2 s for each call's peak |
| Tools | Flye 2.9.6 (`2.9.6-b1802`), Medaka 2.2.2 (CPU, PyTorch), fastp 1.4.0, Shovill 1.4.2 (SPAdes, `depth` 100), mlst 2.35.0, AMRFinderPlus 4.2.7 (DB 2026-10-29.1, `--plus`, organism *Klebsiella_pneumoniae*), CheckM2 1.1.0 (DB v3, `lowmem`), gtdbprep 0.3.5 ANI screen (GTDB R232, `species_only`, sketches from GTDB's server) |
| Threads | 6 for Flye, Medaka, fastp, Shovill, CheckM2 and the ANI screen; 4 for AMRFinderPlus |
| Machine | Arch Linux, 12 cores, 30 GB RAM, shared with other builds (load average 15–55 during the run) |

## Steps (measured)

| Step | Wall time | Peak memory (GiB) |
|---|---|---|
| leanqc, ONT reads | 2.7 min | 0.31 |
| **flye** (`nano-hq`, `genome_size` 5.5m) | **22.2 min** | **3.85** |
| **medaka** (`r1041_e82_400bps_sup_v5.0.0`) | **9.4 min** | **7.36** |
| fastp, Illumina | 2.4 min | 2.77 |
| shovill, Illumina (SPAdes, 100×) | 16.8 min | 3.77 |
| mlst (ONT + Illumina; Flye draft) | 13 s; 9 s | 0.33 |
| amrfinder: Medaka / Flye draft / Illumina | 4.2 / 4.6 / 0.6 min | 0.39 |
| checkm2 (2 genomes) | 6.3 min | 4.56 |
| gtdbprep ANI screen (2 genomes) | 4.3 min | 1.33 |
| isotable | 31 s | 1.12 |

The whole unit peaked at 7.3 GiB with no OOM events.
- **Medaka's 7.36 GiB** is most of the cap. On a machine with 8 GB in total, lower its
  `batch_size` (default 100).
- **AMRFinderPlus took 7× longer** on the long-read assemblies than on the Illumina one. Both are
  about 6 Mb.

## Results (measured)

| | ONT only (Flye + Medaka) | Flye draft | Illumina (Shovill) | Study (cited) |
|---|---|---|---|---|
| Contigs | 11 | 11 | 127 (≥500 bp) | Flye 13; Autocycler 171 |
| Length | 6,050,636 | 6,050,890 | 5,920,807 (≥500 bp) | Flye 6,051,272; Autocycler 6,022,885 |
| Longest contig / N50 | 5,490,603 | 5,490,840 | N50 173,801 | Flye N50 3,032,324 |
| Circular contigs | 5 (139.7, 17.7, 10.8, 6.1 and 3.9 kb) | same | — | Flye 5 |
| GC % | 56.41 | | | 56 |
| MLST | ST101 (PERFECT) | ST101 | ST101 | ST101, all assemblers |
| AMRFinderPlus elements | 49 | 49 | 49 | Autocycler 45 (AMRFinderPlus 4.0.3) |
| CheckM2 completeness / contamination | 100 / 0.44 | | 100 / 0.38 | Flye 100 / 0.47; Autocycler 100 / 0.42 |
| GTDB R232 species (ANI, AF) | *K. pneumoniae* (98.92, 0.87) | | *K. pneumoniae* (98.96, 0.87) | — |

Notes:
- **The 49 elements are the same in all three of our assemblies.** Gene symbol, method (e.g.
  `ALLELEX`, `POINTX`), % identity and % coverage are identical for every one.
- **Against the study's Autocycler assembly,** ours have all 45 of its elements, plus `mrx(A)`,
  `terA`, `traT` and `ompK35_G62AfsTer1`. Those four are in our Illumina assembly too, so they
  come from the newer AMRFinderPlus and database, not from nanopore errors.
- **The study's own Flye run** reported a partial `blaOXA` and lacked the *gyrA*/*parC*/*ompK36*
  mutations that its Autocycler assembly had. On our full read set, Flye + Medaka has all of
  them.
- **Base-level accuracy** (errors per Mb against an Illumina-polished reference) was not measured
  here: irit has no read-mapping tool yet. The study measured AF47's Flye assembly at 4
  substitutions and 6 indels unpolished (Q57.8), and 0 and 0 after Medaka with all reads (Q100)
  (**cited**, `assembly_nucleotide_accuracy_cleaned.csv`).

## Basecalling

Basecalling turns the raw electrical signal into bases. It is the step before these reads, and
the laptop didn't do it.
- **On a GPU** (**cited**): Ryan Wick measured `sup@v5.2.0` at 2.69×10⁷ samples/s on one H100,
  which basecalled a 132 Gbp PromethION run in 18.75 h
  ([blog, 2025](https://rrwick.github.io/2025/05/27/dorado-v1.html)). At that rate, this isolate's
  386 Mb would take about 3–4 min. `hac` was 6–10× faster than `sup`.
- **On a CPU** (**cited**, older R9.4.1 models): Dorado `hac` took 13 h 31 min for an *E. coli*
  read set on a 16-core Xeon, and 27 min on an RTX A6000 GPU, about 30× slower
  ([Rawsamble, arXiv 2410.17801](https://arxiv.org/abs/2410.17801)). `sup` is another 6–10× slower than
  `hac` (the H100 numbers above).
- **Apple silicon:** Dorado runs on it, but no published bacterial-scale numbers were found.
  Dorado 2.0.1 added an Apple Silicon implementation for the v6.0.0 `hac` model.
- **A practical route** for a lab without a GPU: basecall `hac` live on the MinION's computer, or
  send the raw data to a sequencing service or GPU machine for `sup`. Then do everything from
  here on a laptop. Wick found `sup@v5.2.0` clearly better than `hac@v6.0.0` for assembly
  ([blog, 2026](https://rrwick.github.io/2026/06/11/dorado-v2.html)).

## Limits and open questions

- **One isolate.** AF47 is hard: its chromosome didn't close, and neither did the study's. Easier
  genomes, and isolates whose resistance genes sit on small plasmids, need testing next. Small
  plasmids are where Flye is weakest.
- **isotable has no long-read read-QC row yet.** It marks the ONT sample `incomplete` for "no read
  count / no Q30 (fastp)", and leanqc's FastQC thresholds fail long reads on per-position
  quality. A long-read mode should take reads and quality from leanqc's long-read numbers.
- **The Medaka model is downloaded on first use.** `r1041_e82_400bps_sup_v5.0.0` is not in the
  Bioconda package; Medaka fetched it into the environment during the call. The run record has
  the model name, but not the model file's hash.
- **Not measured:** Autocycler on a laptop, Medaka's `batch_size` against peak memory, and Medaka
  on GPU or Apple silicon.
