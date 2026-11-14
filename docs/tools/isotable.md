# isotable: one row per sample

`isotable` collects the outputs of the usual QC, assembly and typing tools into one table, one row
per sample (an isolate or a metagenome-assembled genome), and grades each row against thresholds.

## Why

Pipelines keep writing their own summary scripts for this, and MultiQC, which is excellent for
browsing, aggregates per tool rather than per sample, without a pass/fail decision. See
[landscape](../research/landscape.md#1-manual-steps-worth-a-cli).

## Usage

```sh
isotable results/ --sheet samplesheet.csv -o qc_table.tsv --mqc isotable_mqc.tsv
isotable results/ --mode mag                     # MIMAG tiers from CheckM2
isotable results/ --min-n50 20000 --max-contigs 200
isotable results/ --require reads,q30,contigs,n50,completeness,kraken
isotable assembly-stats asm/*/contigs.fa         # QUAST-style statistics, one row per assembly
```

| Option | Effect |
|---|---|
| `--sheet FILE` | Sample sheet (e.g. from [`sheetgen`](sheetgen.md)) with the real sample names to join on. Recommended. Its FASTQ columns also join QC reports named after the read files (below). |
| `--min-contig N` | Count assembly statistics over contigs of N bp or more. Default 500 (QUAST's); 200 is what GenBank accepts and what many papers report. A QUAST report made at another cut-off then disagrees and the row is a `conflict`. |
| `--mode isolate\|mag` | Grading rules (below). Default `isolate`. |
| `-o FILE` | Output TSV (default stdout). |
| `--mqc FILE` | Also write a MultiQC custom-content table. |
| `--require LIST` | Measurements a sample needs before it can pass (below). `--require none` turns this off. |
| `--fastqc-grade LIST` | Failed FastQC modules (and the matching leanqc checks) that make a sample `warn`, by FastQC or leanqc name, or `all` / `none`. Default: Per base sequence quality, Per sequence quality scores, Per base N content, Adapter Content (below). |
| `--min-reads`, `--min-q30`, `--min-read-q`, `--max-contigs`, `--min-n50`, `--min-busco`, `--min-completeness`, `--max-contamination`, `--min-depth` | Thresholds. `--min-read-q` is for long reads (median read quality, default Q10: basecallers' usual pass threshold). `--min-depth` is the mean depth from read mapping (CoverM), default 20; 0 turns it off. |

## Outputs it reads

Found anywhere under the given folders, by file name:

| Tool | File | Columns |
|---|---|---|
| fastp | `*.fastp.json` (one per sample, named after it) | raw and trimmed reads, Q30 after trimming, duplication |
| leanqc | `*_leanqc.json` (one per read file) | failed checks (R1 and R2 merged) |
| FastQC, [FastQC-Rust](https://github.com/ewels/FastQC-Rust) | `*_fastqc.zip`, or an extracted `*_fastqc/` folder (`fastqc_data.txt` and `summary.txt`), one per read file | sequences, %GC, failed and warned modules (R1 and R2 merged) |
| Kraken 2 | `*kraken2*.txt`, `*.kreport`, `*.report` whose content is a Kraken 2 report | top species and its % |
| QUAST | `transposed_report.tsv` | contigs, length, N50, GC, largest contig, L50, N90, Ns per 100 kbp, contigs ≥ 1 kb |
| Shovill | `contigs.fa` with `shovill.log` beside it; the sample is the folder's name | the [assembly statistics](#assembly-statistics), counted as QUAST does (contigs of 500 bp or more), so the two agree; and Shovill's depth estimate from its log (`shovill_depth_est`) |
| Flye | `assembly.fasta` with `flye.log` beside it; the sample is the folder's name | as Shovill. Left out when the same sample has a Medaka consensus |
| Medaka | `consensus.fasta` with `consensus_probs.hdf` beside it; the sample is the folder's name | as Shovill (the polished long-read assembly) |
| BUSCO | `short_summary.*.json` | complete %, lineage |
| CheckM2 | `quality_report.tsv` | completeness, contamination |
| GTDB-Tk | `*summary.tsv`, or the ANI screen's `classify/ani_screen/*.ani_summary.tsv` (from `gtdbprep ani`, or classify_wf when the screen assigned every genome) | classification |
| mlst | `*mlst*.tsv` | scheme, sequence type |
| AMRFinderPlus | `*amrfinder*.tsv` | AMR genes |
| CoverM | `*coverm*.tsv` with a `Genome` column (`coverm genome`, methods mean and covered_fraction; irit-mcp's `coverm_genome`). A one-genome table is named after the file (`s1.coverm.tsv` → `s1`); with several genomes, each row after its genome | `mapped_depth` (mean depth from read mapping; several read sets add up), `covered_percent` (one read set only). `coverm contig` tables are skipped and listed |
| Kleborate 3 | `klebsiella_pneumo_complex_output.txt`, `klebsiella_oxytoca_complex_output.txt`, `escherichia_output.txt`; rows named by `strain` (the assembly's file name) | `kleborate_species`, `kleborate_species_match`, `kleborate_st`, `virulence_score`, `resistance_score`, `k_locus`, `o_locus`, `ybt`, `clb`, `iuc`, `iro`, `rmpadc` (`-`: not found). Full or `--trim_headers` headers |

Only columns with data appear in the output.

FastQC 0.12.x, 0.13 and FastQC-Rust write the same files, so all of them work. The zip is read in
place, without extracting it. `summary.txt` gives each module's PASS/WARN/FAIL; without it, the
`>>Module<TAB>status` lines of `fastqc_data.txt` do. The read file a report belongs to is the
`Filename` FastQC records (`ERR044595_1M_1.fastq.gz`), or the report's name when FastQC read stdin.
Columns: `fastqc_sequences` (total sequences, summed over a sample's read files), `fastqc_gc` (%GC,
weighted by sequences), `fastqc_fails` and `fastqc_warns` (module names, merged and sorted; every
failed module is listed, graded or not). They sit beside leanqc's `leanqc_fails` rather than in a
shared column, because the two tools name and judge their checks differently.

Reports of trimmed reads go to their own columns: `fastqc_trimmed_sequences`, `fastqc_trimmed_gc`,
`fastqc_trimmed_fails`, `fastqc_trimmed_warns` (and `leanqc_trimmed_fails`). A read file counts as
trimmed when its name ends in a trimmer's suffix before the extension: `.fastp`
(`S_1.fastp.fastq.gz`), Trim Galore's `_val_1`/`_val_2` (`S_1_val_1.fq.gz`), `_trimmed` or
`.trimmed`. Raw and trimmed numbers are never added together.

A FastQC zip that can't be read (truncated, not a zip, no `fastqc_data.txt` inside) is skipped and
listed at the end, like a dangling link; the sample then lacks that read file's FastQC evidence.

Kraken 2 files are recognised by their content (six
or eight tab-separated columns with a rank code), read line by line; per-read output such as
`*.kraken2.classifiedreads.txt` is skipped (and listed) instead of being read into memory.

## Assembly statistics

From an assembly FASTA (Shovill's, Flye's, Medaka's, or any file with `isotable assembly-stats`),
counted the way QUAST 5 does (`quast_libs/basic_stats.py`):

| Column | Meaning |
|---|---|
| `contigs`, `assembly_length` | Contigs of 500 bp or more (`--min-contig`), and their total length (QUAST's `# contigs`, `Total length`). |
| `n50`, `l50`, `n90` | The contig length at which the contigs, longest first, reach 50% (90%) of that length, and for L50 how many contigs it took. |
| `largest_contig` | |
| `gc_percent` | G+C over A+C+G+T of those contigs, two decimals. QUAST divides by the length without Ns; the two differ only when an assembly holds other IUPAC codes. |
| `ns_per_100kbp` | N bases per 100,000 bp (QUAST's `# N's per 100 kbp`). |
| `contigs_1kb` | Contigs of 1000 bp or more (QUAST's `# contigs (>= 1000 bp)`). |
| `contigs_all` | Every contig, whatever its length. |

**Why 500 bp.** It is QUAST's default `--min-contig`, so a QUAST report on the same assembly gives
the same numbers (and isotable doesn't see a conflict when both are present), and it is what genome
announcements usually report. Shovill keeps contigs down to 200 bp, so the count differs from
Shovill's own: on the [paper redo](../research/paper-redo-kpn.md)'s three assemblies, 71, 128 and
146 contigs in all against 38, 58 and 55 of 500 bp or more. `contigs_all` keeps the full count.

`isotable assembly-stats FILE...` prints the same statistics as a table, one row per file (plain or
gzip FASTA), or JSON with `--json`. `--min-contig N` changes the cut-off (0 counts everything);
`contigs_all` and `length_all` are always over every contig. irit-mcp's `assembly_stats` runs it.

```
assembly                                 min_contig  contigs  total_length  largest_contig  n50     l50  n90     l90  gc_percent  ns_per_100kbp  contigs_1kb  contigs_all  length_all
results/shovill/BfR-EC-20867/contigs.fa  500         38       5455238       1372303         661397  3    144094  10   57.04       0.00           33           71           5463998
```

## Depth

`shovill_depth_est` is Shovill's "Estimated sequencing depth": read bases over the genome size it
used, which is its own k-mer estimate unless `--gsize` was given. In the
[depth experiment](../research/coverage-depth.md) that estimate was 0.5-1.9 Mb for a 5.5 Mb genome
at 15-20×, so Shovill reported 58-67× for 20× of reads. `mapped_depth` is the mean depth of the
reads mapped back to the assembly (CoverM), which doesn't depend on a genome-size estimate, and
`covered_percent` the share of the assembly they cover.

Only `mapped_depth` is graded (`--min-depth`, below). When both are there and one is more than 1.5×
the other, the sample warns: report the mapped depth. On BfR-EC-20867 the two agreed (51× and 50.3×).

## Joining by sample

Each tool names things differently (`ERR044595.fastp.json`, `ERR044595-unicycler.scaffolds`,
`ERR044595_1M_1.fastq.gz`). The rules are strict, because a wrong join silently mixes two samples:

- **With `--sheet`** (recommended): a record belongs to the sheet name it equals. Otherwise it
  belongs to the longest sheet name it starts with, but only if everything after that name is a
  recognised suffix: file extensions (`.fa`, `.fastq.gz`, `.scaffolds`, `.contigs`), assembler tags
  (`-unicycler`, `-spades`, ...), subsampling tags (`_1M`, `_500k`) and, for read files only, mate,
  lane and index tags (`_R1`, `_1`, `_R1_001`, `_L001`, `_S3`). So `patient_1` does **not** join
  sample `patient`: it is reported as unmatched and gets its own row. A record that matches no
  sheet name but is named after a read file in one of the sheet's FASTQ columns (`fastq_1`,
  `fastq_2`, or any column of `.fastq`/`.fq` paths) joins that row's sample: read QC of
  `reads/SRR30733615_1.fastq.gz` joins sample BAE-1 when the sheet lists that file for BAE-1.
  Every join that isn't an exact name match is printed as a note.
- **Without a sheet**: extensions and assembler tags are stripped. A mate suffix is stripped only
  from per-file read QC (leanqc, FastQC), and only when the other mate's report is there too and no other
  tool uses the unstripped name; fastp reports, assemblies, bins and genomes keep their names. So
  `patient_1` and `patient_2` stay two samples. Subsampling tags are not stripped without a sheet,
  and neither are lane and index tags: `S_L001_R1_001` and `S_L002_R1_001` become samples `S_L001`
  and `S_L002`. **Lanes of one sample merge only with a sheet** that lists `S`.
- Trimmed read files (see above) are matched under the name of the read file they came from, so
  `S_1.fastp.fastq.gz` and `S_1_val_1.fq.gz` join sample `S` (with a sheet, or next to the other
  mate's report without one), into the `*_trimmed_*` columns.

## Several files for one sample

Each value remembers which file it came from.

- If two files give **different** values for the same field (two QUAST reports with different N50,
  say), the cell shows `CONFLICT`, the sample's status is `conflict`, and the reason names both files
  and values. Equal values from several files are fine.
- fastp: identical reports (the same numbers, e.g. a copied results folder) count once. Reports on
  different raw reads are separate runs of the sample and are pooled: reads add up, Q30 is pooled
  as total Q30 bases over total bases (fastp's `q30_bases` / `total_bases` after filtering), and
  duplication is weighted by reads. Older fastp files without base counts can't be pooled and Q30
  is left out.
- Two reports on the **same raw reads** with different results (fastp rerun with other settings,
  kept in a sibling folder) are not two runs. The sample's totals are worked out with each of them
  in turn; fields that come out differently show `CONFLICT` (the reason lists the total each report
  would give), the status is `conflict`, and a note names the reports. The sample's other runs are
  still pooled in, so two runs plus a rerun of one of them flags only the rerun pair.
- Reports count as the same raw reads when fastp's whole `before_filtering` summary (reads, bases,
  Q20/Q30 bases, GC) matches. If both reports have that summary it alone decides, so two runs
  staged under the same names (`S_1.fastq.gz`) with the same read count but different bases are
  still two runs. Only when a report lacks the summary (older fastp) do the input file names in
  fastp's recorded `command` plus the raw read count decide.
- Reports form a rerun group only if every pair in it counts as the same raw reads. If a report
  without the summary matches two others on names and read count while those two have different
  summaries, isotable can't tell which are reruns and which are separate runs: every fastp field of
  the sample shows `CONFLICT` with each report's own numbers, the status is `conflict`, and a note
  names the reports that differ. Remove the reports that don't belong and rerun.
- leanqc: failed checks from all of a sample's read files are merged.
- FastQC: reports of different read files (R1 and R2, lanes) are merged: sequences add up, %GC is
  weighted by sequences, failed and warned modules are joined. A zip and the extracted folder
  beside it (`x_fastqc.zip` and `x_fastqc/` in the same folder, as `fastqc --extract` leaves them)
  are one report and count once if they agree; if they don't, the fields that differ show
  `CONFLICT`. Any other two reports are two read files, even if FastQC recorded the same file name
  in both (`A/reads_1_fastqc` and `B/reads_1_fastqc`): they are added up and a note names them.
  Remove the copies that don't belong.

## Grading

Statuses, worst first: `conflict` (files disagree), `fail`, `incomplete` (a required measurement is
missing), `warn`, `pass`. A sample never passes on partial evidence.

**Required measurements.** By default an isolate needs reads and Q30 (fastp), contigs and N50
(QUAST, or a Shovill, Flye or Medaka assembly), and completeness (CheckM2 or BUSCO); a MAG needs CheckM2 completeness and contamination.
`--require` replaces the list with any of `reads`, `q30`, `contigs`, `n50`, `completeness`,
`contamination`, `kraken`, `gtdb`, `depth` (CoverM), `mlst`, `amr`, `kleborate`; `--require none`
turns the check off.

**Long reads.** A long-read-only isolate (Nanopore assembled without short reads) has no fastp
report. Its leanqc report has the long-read numbers instead (leanqc adds them when the median read is
over 1,000 bp, or with `--long`). isotable takes the read count (`long_reads`, with `long_read_bases`)
for `reads`, and the median read quality (`read_q_median`) for `q30`. It also shows the read N50.
Several long-read files of one sample add up their reads. Their median qualities are averaged,
weighted by reads, and noted, and there is no pooled N50.

leanqc's FastQC-style checks are made for short reads: per-position quality and base content,
per-tile quality, per-read quality and GC, and read length. On a long-read file they are not
graded, and a note lists the ones that failed. On the nanopore laptop run, a clean R10.4.1 sample
failed three of them.

**Isolate mode**:

| Check | Warn | Fail |
|---|---|---|
| Reads after trimming (all runs) | | < 100,000 |
| Q30 after trimming | < 80% | |
| Median read quality (long reads) | < Q10 | |
| leanqc | a failed graded check | |
| FastQC | a failed graded module | |
| Contigs | > 300 | > 600 |
| N50 | < 10,000 | < 5,000 |
| BUSCO complete | < 95% | < 90% |
| CheckM2 completeness | < 95% | < 90% |
| CheckM2 contamination | > 2% | > 5% |
| Mean depth from read mapping (CoverM) | < 20× | < 10× |
| Shovill's depth estimate against the mapped depth | differ by more than 1.5× | |
| Kleborate typing | `Not Tested` (the species is one the preset Kleborate ran doesn't type) | |

The depth thresholds come from the [depth experiment](../research/coverage-depth.md) on three
*K. pneumoniae*: every Table 1 call held at 15× and above, resistance genes dropped out at 10×, and
20× was the practical target given uneven pooling. They are for bacterial isolates assembled from
Illumina reads; change them with `--min-depth`.

**Which read-QC failures grade.** FastQC's thresholds were set for whole-genome shotgun libraries
of a diverse genome, and several modules fail on healthy data:
[FastQC's own documentation](https://www.bioinformatics.babraham.ac.uk/projects/fastqc/Help/3%20Analysis%20Modules/)
notes that Per base sequence content warns or fails on libraries primed with random hexamers or
fragmented with transposases (Nextera) because of the intrinsic bias in the first bases; Sequence Duplication
Levels fails on any library sequenced deeply relative to the genome size, which bacterial isolates
routinely are; Per tile sequence quality flags a bubble or a bad tile on the flow cell, not the
sample; Per sequence GC content and Kmer Content fail on unusual but real genomes or
contamination that Kraken 2 and CheckM2 judge better. So by default only these make a sample
`warn`:

| FastQC module | leanqc check | Fails when (FastQC) |
|---|---|---|
| Per base sequence quality | Per-position quality | the lower quartile at a position is < 5 or the median < 20 |
| Per sequence quality scores | Per-read quality | the most common mean read quality is < Q20 |
| Per base N content | N content | more than 20% N at a position |
| Adapter Content | Adapter content | more than 10% of reads carry adapter |

Every failed module is still listed in `fastqc_fails` (and `leanqc_fails`). `--fastqc-grade`
replaces the list with FastQC module names or leanqc check names (case doesn't matter; each name
grades both tools' matching check), `all` grades every failure, `none` none. leanqc and FastQC use
the same list so the two tools are graded alike: leanqc's base content, duplication, per-tile,
read length, GC and overrepresented checks map to the FastQC modules of the same name.

When a sample has reports of trimmed reads, only those grade (they are the reads that went on to
assembly; adapter content in the raw reads is what trimming removed); otherwise the raw reads'
reports do. The reason says which: `FastQC (trimmed reads) failed ...`.

FastQC warnings are shown in `fastqc_warns` but don't change the grade. If leanqc and FastQC both
ran on a sample, each tool's failures are listed as their own reason (`leanqc failed ...`,
`FastQC failed ...`), but they are the same kind of evidence about the same reads, so together they
still make one `warn`: the status is the worst of the checks, never a count of them. FastQC's
sequence count is shown, not graded, and doesn't satisfy `--require reads`: FastQC may have run on
raw or trimmed reads, and the read floor is about reads after trimming.

These thresholds are starting points chosen from common practice for bacterial isolates, not a
standard. 100,000 reads is a floor for "something went wrong" (about 3× coverage of a 5 Mb genome
at 150 bp), not a target. Have a domain scientist review every threshold for the organism and the
study before using the grades for decisions, and change them with the flags above.

**MAG mode** uses MIMAG tiers from CheckM2: high (> 90% complete, < 5% contamination) passes, medium
(≥ 50%, < 10%) warns, anything else fails. The tier is shown as `high*`: the full MIMAG high-quality
standard also requires the 23S, 16S and 5S rRNA genes and at least 18 tRNAs, which these tools
don't report.

### Reduced genomes (Patescibacteria / CPR)

When GTDB-Tk places a genome in Patescibacteria and its CheckM2 completeness is below 90%, a note is
added; the verdict is not changed. Evidence:

- CheckM2 is far more accurate than CheckM1 for reduced-genome lineages without a specific marker
  set (mean absolute error 6.4 against 19.7) ([CheckM2 paper](https://www.biorxiv.org/content/10.1101/2022.07.11.499243v1.full)).
- It can still underestimate: complete, single-contig Patescibacteria genomes were scored as less
  than complete ([Patescibacterium danicum, ISME Communications](https://academic.oup.com/ismecommun/article/5/1/ycae147/7904805)),
  and on curated complete endosymbiont genomes CheckM2 averaged 71% completeness.
- Studies of Patescibacteria-rich catalogues re-check these genomes with the CPR-specific marker
  set ([oral microbiome catalogue](https://www.biorxiv.org/content/10.1101/2025.03.10.642329v1.full)).

## Tested

- Real outputs from the nf-core/bacass test run (fastp, QUAST, BUSCO) plus leanqc reports: with a
  sheet, both samples joined across all four tools; thresholds flagged the leanqc failures.
- Regression tests for the joining and merging rules: `patient_1`/`patient_2` stay two samples with
  and without a sheet that lists `patient`; two QUAST reports with different N50 give `conflict`;
  a copied fastp report is counted once while two runs are pooled, and two fastp analyses of the
  same raw reads (the bacass ERR064912 report and a stricter rerun) give `conflict` instead of
  doubling the reads; two runs staged under the same names with the same read count but different
  bases are pooled, and adding a rerun of one of them flags only that pair; a 5-read fastp file
  fails and a
  sample without assembly data is `incomplete`; a per-read Kraken 2 file doesn't replace the report.
- Synthetic FastQC 0.12.1 outputs (in `isotable/testdata/fastqc/`, made up and cut down to four
  modules): R1 as a `_fastqc.zip` and R2 as an extracted folder join into one sample with and
  without a sheet, and a failed graded module makes it `warn`; a lone R1 keeps its name; a zip next
  to its own extracted folder counts once, and a disagreeing copy gives `conflict`; leanqc and
  FastQC failures on one sample give two reasons and one `warn`.
- Real FastQC 0.12.1 (`--extract`) and FastQC-Rust outputs on ERR044595 (1M read pairs) fail only
  Per base sequence content and pass by default; `--fastqc-grade all` makes them `warn`.
- Regression tests: base content, duplication and per-tile failures (FastQC and leanqc) don't
  grade by default and do with `all` or by name; `reads_1_fastqc` in two folders gives two reports,
  not one; a truncated zip is skipped and listed; fastp and Trim Galore trimmed-read reports join
  the sample, in their own columns, and grade instead of the raw reads; lanes merge only with a
  sheet.
- Assembly statistics: N50, L50, N90 and L90 of a made-up assembly worked out by hand, with 500 bp,
  501 bp and no cut-off; gzip gives the same as plain; on the paper redo's three real Shovill
  assemblies, contigs (all and ≥ 500 bp), length and N50 match the numbers in
  [the paper redo](../research/paper-redo-kpn.md), and N50 matches Kleborate's own contig stats.
  Not compared with a QUAST run (QUAST isn't installed on the test laptop).
- CoverM genome tables (as CoverM 0.8.0 wrote one on the laptop, two read sets, a `coverm contig`
  table skipped) and Kleborate 3.2.4 tables (full and trimmed headers); a sample with a mapped
  depth below 20× warns, below 10× fails, and a Shovill estimate of 61× against 18× mapped warns;
  a Kleborate row with `Not Tested` warns.
- Synthetic CheckM2 and GTDB-Tk files (in `isotable/testdata/mag/`, made up for the test) in MAG
  mode: tiers as expected, including a 92%-complete bin failing on 12.5% contamination, and the
  note on both Patescibacteria bins.

## Known gaps

- No Bracken, Mash or sylph parsers yet.
- AMRFinderPlus outputs are matched by file name; one file per sample is assumed.
- Dangling symlinks, unreadable folders and unreadable FastQC zips in the results are skipped and
  listed, not read.
- Trimmed reads are recognised by name only (`.fastp`, `_val_N`, `_trimmed`, `.trimmed`); other
  trimmers' outputs (e.g. Trimmomatic's, which the user names) count as raw reads of a different file.
- fastp reports split per mate (one fastp run per read file) would be read as two runs of a sample
  named after each file; nf-core pipelines don't produce them.
- No per-species expected genome size check.
- Mapped depth is CoverM's mean, which leaves out 75 bp at each contig end; on fragmented assemblies
  of many short contigs that can matter a little.
- Kleborate's columns are read as text; isotable doesn't grade the scores or loci.
