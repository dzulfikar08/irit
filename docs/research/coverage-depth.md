# How much sequencing is enough? Depth vs the same answers

Sequencing is the expensive half of an isolate study, and its cost per isolate falls almost in
proportion to the depth each isolate needs. This experiment, run on 2026-11-13, asked a narrow
question. Take the three *K. pneumoniae* isolates from the [paper redo](paper-redo-kpn.md),
which were sequenced at 50–70×. At what depth do irit's results stop matching that paper's Table 1:
MLST type, all 37 resistance genes, GTDB species and CheckM2 quality?

**Answer: 15×.** At 15× and above, every call matched in every assembly (12/12 at 15×; 18/18 at
20× and above). At 10× resistance genes started to disappear, and at 5× nothing held. Below about
15×, Shovill's default settings also failed silently, which matters more than the depth itself
(see below). With a margin for uneven pooling, **20× is a practical target**: that's 2.5 times as
many isolates per run as 50×.

Measured on the project laptop through irit-mcp 0.3.5, with an 8 GB no-swap memory cap. Three
isolates of one species is a small sample: treat the threshold as a data point, not a rule.

## Method

- **Reads:** the paper's raw ENA reads (MiSeq 2×300 for BfR-EC-20867 and -20869, NextSeq 2×150 for
  BfR-EC-20987, all MD5-checked). Each isolate's pairs were subsampled with seqtk to 40, 30, 20, 15,
  10 and 5× of its full-depth assembly length. Extra seeds were taken at 20×, 15× and 10× to see
  sampling and assembler variation. Achieved depths matched the targets to one decimal.
- **Pipeline:** fastp → Shovill 1.4.2 (SPAdes) → mlst 2.35.0 → AMRFinderPlus 4.2.7 `--plus` →
  CheckM2 1.1.0 → `gtdbprep ani` (GTDB R232 species screen), all called through irit-mcp.
- **Two assembly arms:**
  - **default:** Shovill as the agent ran it in the paper redo.
  - **gsize:** the same reads with the genome size given (`gsize: 5.5M`).
- **Comparison:** each result against the full-depth result, which itself matched the paper's
  Table 1.

## Results

Assemblies where every Table 1 call reproduced (ST, all resistance genes of the isolate, species,
CheckM2 ≥ 99% complete):

| Depth | default arm | gsize arm |
|---|---|---|
| 40× | 3/3 | — |
| 30× | 3/3 | — |
| 20× | 9/9 | 3/3 |
| 15× | 3/3 | 9/9 |
| 10× | 0/9 | 3/9 |
| 5× | 0/3 | 0/3 |

- **20× and above** gave the same ST, all of the isolate's Table 1 resistance genes (11, 13 and 13)
  and the same species for every seed. ANI stayed within 98.92–99.18%, completeness was 100%, and
  contamination stayed at most 0.24%.
- **15×** still matched everywhere, but the default-arm assemblies were more fragmented. BfR-EC-20867
  gave 316 contigs and N50 75 kb, against 85 contigs and 475 kb at 20×. Giving `gsize` brought it back
  to 101–121 contigs.
- **10× with gsize:** MLST and species held, but resistance genes went first.
  - The two MiSeq isolates lost 2–6 of their Table 1 genes in every replicate, mostly
    mobile-element genes (aph, bla, sul, dfr).
  - Some genes came back only as less specific symbols (blaSHV, oqxB, blaCTX-M). That means
    the gene was split across contigs and AMRFinderPlus could no longer name the allele.
  - The NextSeq isolate kept all 13 genes in 3/3 replicates. Three isolates can't separate read
    length from strain, so this is an observation, not a finding.
- **5×:** assemblies of 1.6–2.3 Mb with gsize (CheckM2 37–44% complete), and nothing usable without
  it.

**Shovill's default fails silently at low depth.** Shovill estimates genome size from k-mers seen
more than 10 times. At 5–10× that estimate fell to 0.01–0.11 Mb for a 5.5 Mb genome. Shovill then
subsampled the already thin reads to that tiny size and assembled only 0.4–112 kb. irit-mcp
reported `ok`. Given `gsize`, the same reads assembled to 5.35–5.46 Mb. At 15× the estimate was
already wrong (0.5–0.7 Mb) without yet triggering the subsampling. **Below 30×, pass `gsize`.**
irit should warn about this ([#22](https://github.com/dzulfikar08/irit/issues/22)).

Other irit problems found on the way:
- [#23](https://github.com/dzulfikar08/irit/issues/23): one unsketchable genome aborts a whole
  `gtdbprep ani` batch.
- [#24](https://github.com/dzulfikar08/irit/issues/24): mlst once picked an *E. coli* scheme for a
  fragmented 10× *K. pneumoniae* assembly, with status OK.
- [#25](https://github.com/dzulfikar08/irit/issues/25): CheckM2 on 48 genomes in one call peaked at
  7.7 GiB, near the 8 GB cap.

Time on the laptop, under heavy load from other work, so these are upper bounds:
- Shovill: 3–7.5 minutes per isolate, peak 3.8 GiB.
- Every other per-isolate step: under a minute.
- CheckM2 on all 48 assemblies: 34 minutes.

## What it means for cost

With [`seqplan`](../tools/seqplan.md)'s defaults (5.5 Mb genome, 80% of the run usable for
samples, ×1.3 margin for uneven barcodes), the isolates per run are:

| Depth | MiSeq v3 2×300 | NextSeq 2000 P1 2×150 |
|---|---|---|
| 50× | 29–33 | 67 |
| 20× | 73–83 | 167 |
| 15× | 98–111 | 223 |

Sequencing cost per isolate falls by the same factor. Going from 50× to 20× makes it about 2.5
times cheaper, as long as the run has enough barcodes (`seqplan --barcodes`). Library prep is a
per-isolate cost and doesn't shrink with depth.

**Limits:**
- Three isolates, one species, Illumina only.
- One assembler (SPAdes through Shovill).
- Resistance genes on small plasmids or in repeats are the hardest case, and they are what fails
  first.

For a surveillance question ("which ST, which resistance genes?"), 20× was enough here. Studies
that need closed plasmids or SNP-level outbreak resolution need more depth, or long reads (see
[the nanopore run](nanopore-laptop.md)).

The run's tables, the subsampling script and the irit-mcp run records (`depth-2` to `depth-4`) are
kept with the run, outside this repository.
