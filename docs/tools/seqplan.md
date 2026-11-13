# seqplan: isolates per sequencing run

`seqplan` answers a question you have before sequencing: at the coverage I need, how many isolates
fit on one run of this kit, how many runs do my samples need, and what does each isolate cost?

## Why

Sequencing is the expensive half of a genome project, and the price per isolate depends mostly on
how many samples share a run. The arithmetic is simple, but it usually lives in a spreadsheet with
kit outputs copied from somewhere. `seqplan` keeps each kit's output next to the vendor page it came
from, says whether a number is a specification or a reported typical yield, and adds a margin for
uneven barcodes, which is the step people most often forget.

## Usage

```sh
seqplan --kit miseq-v3 --genome-size 5.5M --coverage 50
seqplan -k nextseq-p1 -g klebsiella-pneumoniae -c 30 -n 96 --run-price 1500 --currency USD
seqplan -k miseq-v2 --reads 2x250 -s 2.8M -c 100
seqplan -k minion-r10.4.1 -g escherichia-coli -c 40 -n 24
seqplan --list-kits
seqplan --list-organisms
seqplan -k nextseq-p2 -s 5M --format json
```

| Option | Effect |
|---|---|
| `-k, --kit ID` | Kit or flow cell (below). Required. |
| `--reads 2x150` | Read configuration. Default: the kit's first listed one (2×300 for MiSeq v3, 2×150 for most others). |
| `-g, --organism ID` | Organism preset with NCBI's expected genome size. |
| `-s, --genome-size SIZE` | Genome size in bp, with an optional k/M/G suffix (`5.5M`). Instead of `--organism`. |
| `-c, --coverage X` | Target mean coverage. Default 50. |
| `--usable F` | Fraction of the run's output that ends up in sample reads. Default 0.8 for Illumina, 1.0 for nanopore (below). |
| `--imbalance M` | Barcode-imbalance margin. Default 1.3 (below). |
| `--barcodes B` | Barcodes your library kit offers (e.g. 24 or 96 for ONT Rapid Barcoding, 96 or 384 Illumina index pairs). Caps isolates per run, and the output says when it did. |
| `-n, --samples N` | Isolates to sequence: adds runs needed and cost per isolate for them. |
| `--run-price P`, `--currency C` | Price of one run in your currency. seqplan has no built-in prices. |
| `-f, --format text\|json` | Output format. |

## How it calculates

- **Per isolate:** genome size × coverage, times the imbalance margin.
- **Per run:** the kit's output × the usable fraction.
- **Isolates per run:** usable run output ÷ bases per isolate, rounded down. Both ends of the kit's
  output range are reported.
- **Runs for N isolates:** N ÷ isolates per run, rounded up. When one isolate needs more than a
  run (a large genome on a small kit), the runs are counted from the pooled bases instead.
- **Cost per isolate:** run price ÷ isolates per run on a full run, or runs × price ÷ N for your
  N samples.

The two defaults are assumptions, not measurements. Change them to match your lab:

- **`--usable 0.8` (Illumina).** Illumina's outputs are for a PhiX control library. Real runs lose
  some output to the PhiX spike-in, reads without a recognised barcode, and adapter and quality
  trimming. Nanopore yields below come from real runs already, so they default to 1.0.
- **`--imbalance 1.3`.** Pooled libraries never split a run evenly, so each isolate is planned at
  1.3× its share. The isolate that gets 1/1.3 of the average still reaches the target. Rapid
  nanopore barcoding can be much more uneven: in the study the MinION figures come from, one
  *K. pneumoniae* library got only 10.8 Mb and was rerun on its own flow cell.

Not modelled: library prep cost and the reads needed for anything other than coverage. Barcode
set size is modelled only when you give `--barcodes`: a run that fits more isolates than you have
barcodes is limited by the barcodes.

## Example

*K. pneumoniae* (5.5 Mb) at 50×, 96 isolates, for a run price of 1,500 USD (an example figure, not a
quote):

```console
$ seqplan -k miseq-v3 -s 5.5M -c 50 -n 96 --run-price 1500 --currency USD
Genome     5.5 Mb (given)
Kit        MiSeq Reagent Kit v3, 2x300 (miseq-v3)
Coverage   50×

Per isolate     275 Mb at 50×; with the ×1.3 imbalance margin 357.5 Mb = 595.8 k 2x300 read pairs
Run output      13.2 to 15 Gb (vendor specification)
Usable          10.56 to 12 Gb (×0.8 for samples)
Isolates/run    29 to 33
Cost/isolate    45.45 to 51.72 USD on a full run
96 isolates     3 to 4 run(s), 46.88 to 62.50 USD per isolate

Source: https://www.illumina.com/systems/sequencing-platforms/miseq/specifications.html
Note: 22-25 M reads passing filter
```

At 25× the same run fits 59 to 67 isolates. On a NextSeq 2000 P1 (2×150, 30 Gb) it fits 67 at 50×
and 134 at 25×. Halving the coverage roughly halves the sequencing cost per isolate. Whether 25× is
enough for your analysis is a separate question, and one that should be measured.

## Kits

Output per run (one flow cell), checked 2026-11-13. `seqplan --list-kits` prints the same table
from the data compiled into the binary (`seqplan/data/kits.tsv`).

| Id | Kit | Read configurations (output) | Basis | Source |
|---|---|---|---|---|
| `miseq-v3` | MiSeq Reagent Kit v3 | 2×300 (13.2–15 Gb), 2×75 (3.3–3.8 Gb) | spec | [Illumina](https://www.illumina.com/systems/sequencing-platforms/miseq/specifications.html) |
| `miseq-v2` | MiSeq Reagent Kit v2 | 2×150 (4.5–5.1 Gb), 2×250 (7.5–8.5 Gb), 2×25 (0.75–0.85 Gb) | spec | [Illumina](https://www.illumina.com/systems/sequencing-platforms/miseq/specifications.html) |
| `miseq-v2-micro` | MiSeq Reagent Micro Kit v2 | 2×150 (1.2 Gb) | spec | [Illumina](https://www.illumina.com/systems/sequencing-platforms/miseq/specifications.html) |
| `miseq-v2-nano` | MiSeq Reagent Nano Kit v2 | 2×150 (0.3 Gb), 2×250 (0.5 Gb) | spec | [Illumina](https://www.illumina.com/systems/sequencing-platforms/miseq/specifications.html) |
| `iseq-100` | iSeq 100 i1 Reagent v2 | 2×150 (1.2 Gb), 2×75 (0.6 Gb) | spec | [Illumina](https://www.illumina.com/systems/sequencing-platforms/iseq/specifications.html) |
| `nextseq-550-mid` | NextSeq 500/550 Mid Output Kit v2.5 | 2×150 (32.5–39 Gb), 2×75 (16.25–19.5 Gb) | spec | [Illumina](https://www.illumina.com/systems/sequencing-platforms/nextseq/specifications.html) |
| `nextseq-550-high` | NextSeq 500/550 High Output Kit v2.5 | 2×150 (100–120 Gb), 2×75 (50–60 Gb) | spec | [Illumina](https://www.illumina.com/systems/sequencing-platforms/nextseq/specifications.html) |
| `nextseq-p1` | NextSeq 1000/2000 P1 (XLEAP-SBS) | 2×150 (30 Gb), 2×300 (60 Gb), 2×50 (10 Gb) | spec | [Illumina](https://www.illumina.com/systems/sequencing-platforms/nextseq-1000-2000/specifications.html) |
| `nextseq-p2` | NextSeq 1000/2000 P2 (XLEAP-SBS) | 2×150 (120 Gb), 2×300 (240 Gb), 2×100, 2×50 | spec | [Illumina](https://www.illumina.com/systems/sequencing-platforms/nextseq-1000-2000/specifications.html) |
| `nextseq-p3` | NextSeq 2000 P3 (XLEAP-SBS) | 2×150 (360 Gb), 2×100, 2×50 | spec | [Illumina](https://www.illumina.com/systems/sequencing-platforms/nextseq-1000-2000/specifications.html) |
| `nextseq-p4` | NextSeq 2000 P4 (XLEAP-SBS) | 2×150 (540 Gb), 2×100, 2×50 | spec | [Illumina](https://www.illumina.com/systems/sequencing-platforms/nextseq-1000-2000/specifications.html) |
| `novaseq-x-1.5b` | NovaSeq X 1.5B | 2×150 (500–716 Gb), 2×300, 2×100, 2×50 | spec | [Illumina](https://www.illumina.com/systems/sequencing-platforms/novaseq-x-plus/specifications.html) |
| `novaseq-x-10b` | NovaSeq X 10B | 2×150 (3–4 Tb), 2×100, 2×50 | spec | [Illumina](https://www.illumina.com/systems/sequencing-platforms/novaseq-x-plus/specifications.html) |
| `novaseq-x-25b` | NovaSeq X 25B | 2×150 (8–10.5 Tb), 2×100, 2×50 | spec | [Illumina](https://www.illumina.com/systems/sequencing-platforms/novaseq-x-plus/specifications.html) |
| `minion-r10.4.1` | MinION flow cell R10.4.1, rapid barcoding | long reads (5.2–7.0 Gb) | **typical** | [Microbial Genomics 2024](https://pmc.ncbi.nlm.nih.gov/articles/PMC11170131/) |
| `minion-r10.4.1-core` | MinION flow cell R10.4.1, core facility | long reads (10–30 Gb) | **typical** | [UGA GGBC](https://dna.uga.edu/oxford-nanopore) |
| `flongle-r10.4.1` | Flongle flow cell R10.4.1 | long reads (1.8–2.6 Gb) | **typical** | [University of Iowa](https://humangenetics.medicine.uiowa.edu/node/86) |

- **Illumina** figures are the vendor's specifications for a PhiX control library at supported
  cluster densities. For NovaSeq X, Illumina doesn't guarantee the upper end. The NextSeq 500/550
  kits are listed on Illumina's NextSeq 550 page. The NextSeq 1000/2000 figures are for XLEAP-SBS
  chemistry. P3 and P4 run on the NextSeq 2000 only.
- **Nanopore** yields depend on the library, the run time and the sample far more than Illumina's
  do, so `seqplan` uses reported typical ranges, not ONT's theoretical maxima.
  - The MinION rapid-barcoding range comes from 72-hour runs of bacterial isolates in a published
    study.
  - The core-facility MinION and Flongle ranges are what sequencing cores state they expect.
  - Oxford Nanopore's own figures are theoretical maxima: up to 50 Gb for a MinION flow cell over
    72 h ([price list](https://store.nanoporetech.com/priceList.html)), and up to 2.8 Gb for a
    Flongle ([specification](https://nanoporetech.com/document/requirements/flongle-spec)).
  - If your own runs yield less than the listed range, scale it down with `--usable` (your
    yield ÷ the listed yield).

## Organism presets

Genome sizes from NCBI's
[expected-genome-size service](https://api.ncbi.nlm.nih.gov/genome/v0/expected_genome_size?species_taxid=573)
(the size NCBI checks submitted assemblies against), fetched 2026-11-13. Any other organism: give
`--genome-size`.

| Id | Expected size | NCBI range |
|---|---|---|
| `klebsiella-pneumoniae` | 5.6 Mb | 4.5–6.7 Mb |
| `escherichia-coli` | 5.15 Mb | 4.0–6.3 Mb |
| `salmonella-enterica` | 5.05 Mb | 4.1–6.0 Mb |
| `staphylococcus-aureus` | 2.8 Mb | 2.2–3.4 Mb |
| `pseudomonas-aeruginosa` | 6.475 Mb | 5.0–7.95 Mb |
| `acinetobacter-baumannii` | 3.95 Mb | 3.2–4.7 Mb |
| `enterococcus-faecium` | 2.95 Mb | 2.0–3.9 Mb |
| `enterococcus-faecalis` | 2.95 Mb | 2.0–3.9 Mb |
| `streptococcus-pneumoniae` | 2.095 Mb | 1.57–2.62 Mb |
| `mycobacterium-tuberculosis` | 4.075 Mb | 3.15–5.0 Mb |
| `vibrio-cholerae` | 4.225 Mb | 3.6–4.85 Mb |
| `neisseria-gonorrhoeae` | 2.225 Mb | 1.95–2.5 Mb |
| `helicobacter-pylori` | 1.62 Mb | 1.29–1.94 Mb |
| `campylobacter-jejuni` | 1.985 Mb | 1.37–2.6 Mb |
| `listeria-monocytogenes` | 3.05 Mb | 2.5–3.6 Mb |
| `bacillus-subtilis` | 4.12 Mb | 3.3–4.95 Mb |

## Through irit-mcp

The [`seqplan` tool](mcp.md#seqplan) takes the same options and returns the plan as JSON, so an AI
agent can answer "how many isolates per run" from the cited figures instead of from memory. It
never fills in a price the user didn't give.
