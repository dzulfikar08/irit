# A second paper redo: *E. coli* and *Salmonella* Kentucky from chickens in Bangladesh

On 2026-11-14, the same kind of agent as in the [first paper redo](paper-redo-kpn.md) got another
genome paper, its raw reads and irit-mcp, with no shell, no web access and no other analysis
tools. This time there were two species and six isolates, and irit-mcp had the tools the first
redo asked for: QUAST-style assembly statistics (`assembly_stats`) and depth from read mapping
(`coverm_genome`). Everything ran on the project laptop.

**Result:**
- **Matched:** all six sequence types (every allele exact), genome size within 0.13 %, GC within
  0.01 points, and every acquired resistance gene in Table 1 apart from one gene and some allele
  names that come from using a different tool.
- **Not checkable:** clonal complex, plasmid replicons and serovar; irit has no tool for them.
- **Problems the agent found in the paper:**
  - Twelve *blaTEM* alleles listed for one isolate where the assembly has one truncated *blaTEM*.
  - Read counts and coverage that don't follow from the deposited reads.
  - Two isolates reported resistant to ceftriaxone and ertapenem with no gene that explains it.
- **irit problems:** the agent reported twelve. Two were real irit bugs and are fixed; the rest
  were setup on our side, missing tools, or upstream.

## The paper

Amena B, Islam M, Nizami TA, Barua H, Biswas PK. *Draft genome sequences of multidrug-resistant
Escherichia coli and Salmonella enterica serovar Kentucky isolates from chicken samples in
Bangladesh.* Microbiology Resource Announcements 15(3):e01429-25, published 9 February 2026.
[doi:10.1128/mra.01429-25](https://doi.org/10.1128/mra.01429-25), CC BY 4.0. Reads: SRA
SRR30733610–SRR30733615 (BioProject PRJNA1162444).

Why this paper:
- **Different from the first redo:** two species instead of one, and a lab setting close to
  Indonesia's (poultry farms and live bird markets).
- **Data:** Illumina NextSeq 2000 reads, 1.6 GB gzipped for all six isolates.
- **Methods:** FastQC, fastp, Shovill (SPAdes), QUAST, Kraken2, ResFinder, PlasmidFinder and
  PubMLST, run on Galaxy. irit-mcp wraps some of these and has alternatives for others.
- **Table 1** has concrete values per isolate to check: ST, clonal complex, genome size, GC,
  contigs, N50, reads, coverage, resistance genes and plasmid replicons.

## Setup

| | |
|---|---|
| Agent | Claude Code, headless (`claude -p`), default model (Claude Opus 5.5), built-in tools limited to Read/Write/Edit/Glob/Grep. The only MCP server was irit-mcp. |
| irit | Built from main at `861d753` (after 0.4.0: `assembly_stats`, `coverm_genome`, `kleborate`) |
| Tools | fastp 1.4.0, Shovill 1.4.2 (SPAdes 3.15.5), mlst 2.35.0, AMRFinderPlus 4.2.7, CheckM2 1.1.0, CoverM 0.8.0, via Bioconda environments |
| Databases | dblock: `amrfinderplus-db-4.2-2026-10-29.1`, `checkm2-db-v3`, `gtdbtk-r232-ani-screen`, `gtdbtk-r232-placement-lite`, all reused from the shared store |
| Limits | irit-mcp and every tool it started ran in an 8 GB, no-swap cgroup. The machine has 12 cores and was shared with nanopore runs and builds. |
| Brief | The first redo's brief, adapted: check every Table 1 value, report species and quality, and say what looks wrong in the data or the paper. |

The session took 106 minutes and 52 turns, and cost US$5.43 at API prices. Compute steps, from
the run record (`started`/`ended` of each call):

| Step | Time |
|---|---|
| leanqc, all 12 read files | 28 s |
| fastp, per isolate | 11–24 s |
| Shovill (SPAdes), per isolate | 7.7–20.8 min |
| mlst, per isolate | 7–9 s |
| AMRFinderPlus `--plus`, per isolate | 23–46 s |
| CheckM2, all six | 10 min |
| CoverM genome, per isolate | 20–83 s |

Two setup mistakes on our side limited the run:
- **sheetgen:** its binary wasn't copied into irit-mcp's bin folder, so the agent wrote the sample
  sheet by hand.
- **Species check:** the brief said "no web access" and didn't say irit's own tools may read from
  GTDB's server. So the agent didn't let `gtdbprep_ani` fetch the 23 reference sketches it needed
  (75 MB). The first redo hit the same wording; the brief now has to say it.

The species check was run afterwards, outside the agent (below). Kleborate wasn't installed;
these aren't *Klebsiella*, so it wouldn't have helped.

## Results against Table 1

The agent's full table is in its report; this is the summary.

| Row | Verdict | Detail |
|---|---|---|
| ST | 6 of 6 match | *E. coli* ST1589, ST2705, ST1196 (Achtman scheme) and *Salmonella* ST198 ×3, every allele exact |
| Clonal complex | not checked | mlst reports no clonal complex |
| Genome size | 5 match, 1 close | Within 0.13 %, but only when counting contigs of 200 bp or more (below) |
| GC | 6 of 6 match | Within 0.01 points |
| Contigs | 5 close, 1 differs | 6–13 % fewer for five isolates at ≥ 200 bp; BAS-3 has 25 % more |
| N50 | 2 match, 1 close, 3 differ | BAE-3 and BAS-1 within 4 bp; others −26 % to +48 % |
| Reads | 6 differ | The paper's counts are 92.9–94.2 % of the read pairs in SRA |
| Coverage | 6 differ | The paper's values are 0.82–0.90 times our mapped depth |
| Resistance genes | 1 match, 5 close | Below |
| Plasmid replicons | not checked | irit has no replicon typing (PlasmidFinder) |

- **Contig cut-off:** the paper doesn't say which minimum contig length Table 1 uses. The agent
  ran `assembly_stats` at both 500 bp (QUAST's default) and 200 bp (what GenBank accepts).
  - At 200 bp, the genome sizes agree to 43–6,711 bp; at 500 bp the gaps are about ten times
    larger.
  - So Table 1's size and contig count most likely describe the deposited 200 bp assemblies.
    Its GC fits the 500 bp numbers slightly better.
- **Reads and coverage:**
  - **Reads:** the paper's counts are a steady 93–94 % of the pairs in SRA, below even our
    post-fastp counts (99.3–99.6 %).
  - **Coverage:** our mapped depth (CoverM) and Shovill's own estimate agree within 3 %. The
    paper's coverage is 10–18 % lower, and its lower read counts explain only about 6 % of that.
  - The paper doesn't say how it counted either.
- **Resistance genes** were found with a different tool (AMRFinderPlus instead of ResFinder), so
  some differences are naming:
  - **Missing:** *aac(6')-Iaa*, a chromosomal *Salmonella* gene, in all three *Salmonella*
    isolates. ResFinder reports it; this AMRFinderPlus database doesn't, with or without the
    organism option.
  - **Names:** TEM-1A/1B differ only by silent changes, so AMRFinderPlus, which works on
    proteins, reports *blaTEM-1*. For BAE-2, AMRFinderPlus calls *aac(3)-IIe* where the paper has
    *aac(3)-IIa*, and *aadA2* where it has *aadA17*.
  - **Partial hits:** several genes sit at contig ends, for example *tet(A)* in BAE-3, found as a
    59 % fragment. A cut-off on gene coverage would explain why ResFinder didn't list it.

### Species, afterwards

The species check the agent couldn't run was run afterwards with `gtdbprep ani --species-only`
(GTDB R232) on the agent's six assemblies. It fetched 23 sketches (75 MB) in 30 s.

| Isolate | Closest GTDB reference (ANI, AF) | GTDB R232 species |
|---|---|---|
| BAE-1 | GCA_047199055.1 (96.91, 0.85) | *ECMA0423 sp047199055* |
| BAE-2 | GCF_003697165.2 (96.88, 0.78) | *Escherichia coli* |
| BAE-3 | GCA_047199055.1 (96.83, 0.87) | *ECMA0423 sp047199055* |
| BAS-1, BAS-2, BAS-3 | GCF_000006945.2 (98.42–98.61) | *Salmonella enterica* |

**Two of the three "*E. coli*" get a placeholder genus name in GTDB R232.**
- **Where they land:** BAE-1 and BAE-3 are closest to a reference GTDB R232 files under the
  placeholder genus *ECMA0423*. The *E. coli* species representative is the next hit, at 96.6.
- **Not a new genus:** GTDB R232 splits the *E. coli*/*Shigella* complex into several species with
  placeholder genus names. The *E. coli* K-12 test genome in the
  [ANI write-up](ani-laptop.md) lands on another of them (GCA_047199095.1, *G047199095*).
- **What to report:** by the usual clinical definition all three are *E. coli*, as the paper says.
  A GTDB name in a report on *E. coli* needs that explanation next to it.

## What the agent found in the paper

1. **BAS-3's twelve *blaTEM* alleles.**
   - **What Table 1 says:** TEM-1B, -30, -70, -104, -148, -176, -198, -207, -217, -220, -230 and
     -234.
   - **What the assembly has:** one *blaTEM*, cut off at a contig end (62.6 % coverage, 100 %
     identity).
   - **Why:** a fragment matches many TEM alleles equally well, and ResFinder lists every tie. The
     table should say one partial *blaTEM*, not twelve β-lactamases.
2. **Read counts and coverage** don't follow from the deposited reads, and the paper doesn't say
   how it counted them (above).
3. **Phenotype without a genotype:**
   - BAS-1 and BAS-2 are reported resistant to ceftriaxone and ertapenem. Neither the paper's list
     nor AMRFinderPlus finds an ESBL, AmpC or carbapenemase gene in them, only *blaTEM-1*.
   - BAE-2 (ertapenem resistant, with only *blaDHA-1*) is similar.
   - The cause could be porin or efflux changes that neither tool reports, or the susceptibility
     test itself.
4. **Allele calls at contig ends:**
   - BAE-2 has several genes as partial copies at contig ends: *aac(3)-IId* 53.5 %, *aadA2* 65.4 %
     and *tet(A)* 74.4 %.
   - In short-read drafts, gene presence is reliable but allele-level names for such genes are not.

The agent also judged that BAE-3's larger assembly (5.77 Mb) is accessory DNA rather than a
second organism. CheckM2 contamination is 0.61 %, and the paper lists the most plasmid replicons
for this isolate.

## What it says about irit

**Fixed after the run:**
- **isotable ignored the sheet's read files.** The sheet listed `reads/SRR30733615_1.fastq.gz` for
  BAE-1, yet each leanqc report got a row of its own (12 spurious "incomplete" rows). With
  `--sheet`, a report named after a read file in the sheet's FASTQ columns now joins that sample.
  On the agent's outputs the table now has exactly the six samples.
- **Contig cut-off:** isotable always counted contigs of 500 bp or more, so 200 bp statistics
  couldn't be compared in the table. It now has `--min-contig` (irit-mcp `min_contig`).
- **Output names:** irit-mcp refused AMRFinderPlus, mlst and CoverM outputs not named
  `<sample>.<tool>.tsv`. That blocked a side run that shouldn't join the table. Other names are now
  allowed, with a warning that isotable won't read them.
- **`dblock_env` in a new folder:** it errors until a project exists. Its description now says to
  call `dblock_add` first.

**Setup and environment, not irit code:**
- **sheetgen missing and the species-check wording:** above.
- **CheckM2:** the Bioconda environment has scikit-learn 1.9.1 against models saved with 1.6.1,
  and irit-mcp passes on scikit-learn's warning. The results look plausible (100 % complete for
  all six), but the environment should pin scikit-learn.
- **AMRFinderPlus** warns on every call that `$CONDA_PREFIX` isn't set; harmless when the database
  is given.
- **Shovill 1.4.2:** it needs at least 8 GB for `--ram`, the same as the cap. It worked here.

**Gaps, now on the roadmap:**
- **Plasmid replicon typing** (PlasmidFinder or similar) and **ResFinder**. A whole Table 1 row
  couldn't be checked, and the resistance comparison had to cross tools.
- ***Salmonella* serotyping** (SISTR or SeqSero2), and **a Kraken2 tool**. dblock can already
  fetch Kraken2 databases, but irit-mcp can't run Kraken2.
- **Clonal complex:** mlst doesn't report it. A lookup table from PubMLST would cover it.
- **Read checksums:** the agent couldn't check the reads' MD5s. During this work, four ONT
  downloads (two files, twice each) arrived corrupt with no download error; Flye only noticed
  partway through ([nanopore write-up](nanopore-laptop.md)). irit has no way to check read files against
  ENA/SRA checksums.

## Files

The brief, the agent's transcript, its report, the run record and `replay.sh` are kept in the
project's lab folder, not in the repository. The paper is CC BY 4.0; its Table 1 is summarised here
and not copied.
