# cooccur: genomes that rise and fall together

`cooccur` reads a genome × sample abundance table, such as `coverm genome` writes for MAGs mapped
across many samples, and tests every pair of genomes for co-occurrence. Abundances are
compositional, so it compares log-ratios, not raw percentages. Each pair gets a permutation
p-value and a Benjamini-Hochberg q-value. With a GTDB-Tk taxonomy, genomes are labelled CPR
(Patescibacteria) or not, and the summary lists the CPR–non-CPR pairs that pass.

## Why

Most CPR bacteria (phylum Patescibacteria) have reduced genomes and are thought to depend on other
microbes, often as epibionts ([Castelle et al. 2018](https://doi.org/10.1038/s41579-018-0071-2)).
The first cultivated one, a Saccharibacteria (TM7) strain from the human mouth, grows attached to
an *Actinomyces* host ([He et al. 2015](https://doi.org/10.1073/pnas.1419038112)). Most CPR have
never been grown, so candidate hosts are often proposed from indirect evidence. One such line is a
non-CPR genome whose abundance tracks the CPR genome across many samples. `cooccur` does that step
for MAGs from a metagenome series, with methods made for relative data.

## What co-occurrence can and cannot show

- **Association, not interaction.** Two genomes can rise together because one feeds the other,
  because both like the same conditions (depth, oxygen, season), or because both respond to a third
  organism. Co-occurrence networks are hypotheses to test, not interaction maps ([Faust & Raes
  2012](https://doi.org/10.1038/nrmicro2832)). A CPR–host pair needs other evidence, such as
  microscopy (FISH), co-culture, or genomic signs of dependence.
- **It needs many samples.** In the simulation below, a strong planted association
  (log-abundance correlation 0.83) was found in 6% of cases with 10 samples, 80% with 20 and 100%
  with 40. Weaker associations need more. With fewer than 20 samples cooccur warns.
- **Abundances are relative.** Each sample is a share of the reads, so if one genome blooms, every
  other genome's share falls even if nothing else changed. Correlating percentages directly gives
  spurious pairs ([Lovell et al. 2015](https://doi.org/10.1371/journal.pcbi.1004075)). cooccur works
  on centred log-ratios (CLR), which do not change when a sample is scaled (sequencing depth,
  closure to 100%). The CLR still couples genomes slightly: independent genomes have a mean CLR
  correlation of about −1/(k − 1) for k genomes (measured below: −0.14 for 8 genomes, −0.02 for
  50). The permutation test includes this coupling.
- **Sequencing depth and detection.** A genome missing from a sample may be absent or just below
  detection. Zeros have no logarithm, so they are replaced by a pseudocount. When two genomes are
  both absent from the same samples, the replacement values make them look proportional. The
  `n_both` column counts the samples where both are present; treat pairs with low `n_both` with
  caution.
- **One pair at a time.** rho and Spearman test pairs. They do not separate direct from indirect
  associations, as conditional methods try to do.

## Usage

```sh
coverm genome --genome-fasta-directory bins -x fa -1 *_R1.fastq.gz -2 *_R2.fastq.gz \
  -m relative_abundance -o abundance.tsv
cooccur abundance.tsv -t gtdbtk.bac120.summary.tsv -t gtdbtk.ar53.summary.tsv -o edges.tsv
cooccur abundance.tsv --alternative greater --min-stat 0.6 --summary summary.json -o edges.tsv
cooccur counts.tsv --measure count --pseudocount 1 --method spearman --all-pairs -o all.tsv
```

| Option | Effect |
|---|---|
| `TABLE` | `coverm genome` output (`Genome`, then `<sample> <Method>` columns), or a plain TSV with the genome in the first column and one column per sample. CoverM's `unmapped` row is dropped. |
| `-m, --measure M` | Which CoverM measure to use when the table has several: `relative_abundance`, `count`, `mean`, `trimmed_mean`, `reads_per_base`, `rpkm` or `tpm` (or the column name, e.g. `"Read Count"`). Covered fraction, covered bases, variance and length are refused. |
| `-t, --taxonomy FILE` | GTDB-Tk summary (`user_genome`, `classification`) or a two-column table (genome, lineage). Repeat for bacterial and archaeal summaries. Names match with or without `.fa`/`.fna`/`.fasta`. |
| `--cpr-taxon RANK` | Lineage rank that marks a genome as CPR (exact match of one `;`-separated rank). Repeatable. Default: `p__Patescibacteria` and `p__Patescibacteriota` (GTDB R232 uses the second). Give `p__Nanoarchaeota` or another group to look at its partners instead. |
| `--method rho\|spearman` | Proportionality rho on CLR values (default), or Spearman's correlation of CLR values. |
| `--min-prevalence F` | Keep genomes present in at least this fraction of samples. Default 0.5. |
| `--min-abundance X` | A genome counts as present when its value is above zero and at least X; smaller values count as zeros. Default 0. |
| `--pseudocount min\|X` | Value that replaces zeros. `min` (default): the smallest non-zero value of the filtered table, as propr does. A number, e.g. `1` for read counts. |
| `--alternative two-sided\|greater` | Association in either direction (default), or only positive. |
| `--fdr Q` | Pairs with q ≤ Q pass. Default 0.05. |
| `--min-stat S` | Pairs must also reach this statistic (absolute value when two-sided). Default 0. |
| `--permutations N`, `--max-permutations M` | Permutations per pair at first (10,000), and the most a pair can be extended to (10,000,000). Below. |
| `--stop-after H` | Stop a pair after H permutations at least as extreme as the observed value. Default 10. |
| `--seed S` | Random seed. Default 1. The same seed and input give the same output on any number of threads. |
| `--threads N` | Threads (default: all). |
| `-o, --output FILE` | Edge list (default: standard output). |
| `--all-pairs` | Write every tested pair, not only those that pass. |
| `--summary FILE` | The summary as JSON. A text summary always goes to standard error. |

## How it calculates

1. **Filter.** Genomes present in fewer than `--min-prevalence` of the samples are dropped. A genome
   whose CLR value is the same in every sample is dropped too: it has no variance. At least 5
   samples are required.
2. **Zeros.** Zeros (and values below `--min-abundance`) are replaced by the pseudocount.
3. **CLR.** For each sample, the log of each genome's value minus the mean log of all kept genomes
   in that sample: the centred log-ratio ([Aitchison 1986](https://doi.org/10.1007/978-94-009-4109-0)).
4. **Statistic.** Proportionality
   rho = 1 − var(clr *a* − clr *b*) / (var(clr *a*) + var(clr *b*))
   ([Lovell et al. 2015](https://doi.org/10.1371/journal.pcbi.1004075); with the CLR as reference,
   [Erb & Notredame 2016](https://doi.org/10.1007/s12064-015-0220-8)). It is 1 when *a* and *b*
   keep the same ratio in every sample, 0 when unrelated, and negative when one rises as the other
   falls. With `--method spearman`, Spearman's rank correlation of the CLR values, ties averaged.
   SparCC ([Friedman & Alm 2012](https://doi.org/10.1371/journal.pcbi.1002687)) is not implemented.
5. **Permutation p-value.** For each pair, *b*'s log-ratio to the geometric mean of the other genomes
   is shuffled across samples, both CLRs are recomputed (moving *b* also moves each sample's mean),
   and the statistic is computed again. Two earlier designs gave too many small p-values and were
   dropped (measurements below): shuffling *b*'s CLR values leaves out the CLR's own coupling, and
   shuffling *b*'s log share carries one sample's total into another.
   The count is sequential ([Besag & Clifford 1991](https://doi.org/10.1093/biomet/78.2.301)): a pair
   stops after `--stop-after` (h = 10) exceedances, with p = h / l after l permutations, or at the
   cap, with p = (g + 1) / (cap + 1) for g < h exceedances. Unrelated pairs stop after a few dozen
   permutations, and the work goes to the pairs with small p.
6. **Growing the cap.** With m pairs, the smallest p a pair can get is 1 / (cap + 1), and its q is at
   least that times m divided by its rank. For 500 genomes (124,750 pairs) at 10,000 permutations, a
   pair at the cap could only pass q 0.05 if about 250 pairs were at the cap. So pairs that reach the cap
   with q above `--fdr` are rerun with ten times the cap, up to `--max-permutations`. Each pair has its
   own random stream (SplitMix64, [Steele et al. 2014](https://doi.org/10.1145/2660193.2660195)),
   seeded from `--seed` and the pair, so a rerun repeats its first permutations and continues, and the
   output does not depend on the thread count. If even `--max-permutations` cannot let a pair pass,
   cooccur warns.
7. **q-values.** Benjamini-Hochberg over all tested pairs
   ([Benjamini & Hochberg 1995](https://doi.org/10.1111/j.2517-6161.1995.tb02031.x)). A pair passes
   at q ≤ `--fdr` and |statistic| ≥ `--min-stat` (statistic ≥ `--min-stat` with `greater`).

rho, and its permutation p, do not change when a genome is multiplied by a constant or a sample is
scaled, as long as no zeros are involved (unit test `rho_ignores_per_genome_and_per_sample_scale`).
So CoverM's relative abundance, read count and mean depth give nearly the same rho: on the CoverM
table below, rho for bin.1–bin.2 was 0.998226, 0.998252 and 0.998225. Genome length is a constant
per genome; the unmapped share is a constant per sample.

## Output

The edge list has one row per pair that passes (every pair with `--all-pairs`), sorted by q and then
by the strength of the statistic:

| Column | Meaning |
|---|---|
| `genome_a`, `genome_b` | The pair, in table order. |
| `rho` or `spearman` | The statistic. |
| `p`, `q` | Permutation p-value and Benjamini-Hochberg q-value. |
| `permutations` | Permutations this pair used. |
| `n_both` | Samples where both genomes are present (raw value above zero and at least `--min-abundance`). |
| `label_a`, `label_b` | `CPR`, `non-CPR` or `unclassified` (no lineage or no phylum), with a taxonomy; empty without one. |
| `phylum_a`, `phylum_b` | The `p__` rank of each lineage. |

The summary (standard error, and `--summary` as JSON) gives the samples, genomes kept and dropped,
the pseudocount and how many zeros it replaced, the settings, pairs tested and passing (positive and
negative), label counts, and every CPR–non-CPR pair that passes, CPR genome first.

## Example

A table from CoverM 0.8.0 (`coverm genome -m relative_abundance count mean
--min-covered-fraction 0`). The reads were simulated: four random 30 kb genomes, six samples of
4,000 reads, with bin.2 getting twice bin.1's share in every sample. Labels come from a GTDB-Tk-style
summary that lists bin.1 to bin.3:

```
$ cooccur coverm.tsv -m count -t taxonomy.tsv -o edges.tsv
cooccur: 6 samples, 4 of 4 genomes kept (prevalence >= 0.5), 0 zeros replaced by 172.0000
rho two-sided, 6 pairs tested, 3 pass q <= 0.05 (1 positive, 2 negative)
labels: 1 CPR, 2 non-CPR, 1 unclassified
CPR–non-CPR pairs passing: 2 (1 positive, 1 negative)
  bin.1	bin.2	p__Pseudomonadota	0.9983	q 0.0102	both present in 6
  bin.1	bin.3	p__Bacillota	-0.8832	q 0.0244	both present in 6
warning: only 6 samples: with fewer than 20, only very strong associations pass (see the guide's simulation)
warning: 1 kept genome(s) have no phylum in the taxonomy (labelled unclassified)
```

bin.3's share fell as bin.1's rose in this design, hence the negative pair. Six samples are far too
few for real data; the example only shows the format.

## Tested

**Against propr and R.** On a 25 × 30 count table with 62 zeros, generated in R, rho for all 300
pairs matched `propr(counts, metric = "rho", ivar = "clr")` from the R package propr 5.1.8
([Quinn et al. 2017](https://doi.org/10.1038/s41598-017-16520-0)). The largest difference was
5.0 × 10⁻⁷, the rounding of cooccur's six printed decimals. Both replace zeros with the smallest
non-zero value. Spearman matched R's `cor(clr, method = "spearman")` to the same precision.
Benjamini-Hochberg matches R's `p.adjust(p, "BH")` (unit test).

**Permutation p-values.** For five pairs of that table, R with 100,000 plain permutations of the CLR
values gave p = 0.0022, 0.0042, 0.0071, 0.0081 and 0.0248. cooccur's sequential p-values (stopping
at 10 exceedances) gave 0.0037, 0.0045, 0.0102, 0.0081 and 0.0169. With h = 10, a p-value has about
30% relative error; use `--stop-after 100` for tighter values. The R check permuted CLR values, the
earlier design of step 5, so this compares the sequential count, not the null.

**Calibration.** Tables simulated by `cooccur/sim/simulate.py` have independent log-normal
genomes, closed to 100% per sample, with 40 samples. With 2,000 permutations and no early stop
(`cooccur/sim/calibrate.sh`), the share of independent pairs with small p:

| Genomes | Statistic | Pairs | p ≤ 0.01 | p ≤ 0.05 | Mean statistic |
|---|---|---|---|---|---|
| 8 | rho | 8,400 | 1.08% | 4.87% | −0.139 |
| 15 | rho | 31,500 | 0.97% | 4.97% | −0.070 |
| 50 | rho | 49,000 | 0.98% | 5.03% | −0.020 |
| 8 | spearman | 8,400 | 0.96% | 4.93% | −0.133 |
| 50 | spearman | 49,000 | 1.04% | 4.96% | −0.019 |

Shuffling CLR values instead gave 13.9% at p ≤ 0.05 for 8 genomes and 7.0% for 15. Shuffling log
shares was nominal at 0.05 but too heavy in the tail: 0.127% at p ≤ 0.001 (50 genomes, one-sided,
73,500 pairs, 100,000 permutations). The scheme used now gave 0.124% at p ≤ 0.001, 0.031% at
2.5 × 10⁻⁴ and 0.011% at 10⁻⁴ in the same test. Those are 91, 23 and 8 pairs where 73.5, 18.4 and
7.4 were expected. No permutation null is exact for relative data; this one is close.

**Planted pairs.** 50 genomes, 5 of the 25 genome pairs made proportional (genome 2*i* + 1 = 3 ×
genome 2*i* × log-normal noise), 50 simulated tables per row, defaults (`cooccur/sim/power.sh`).
Noise sd 0.3 gives a log-abundance correlation of 0.98; noise sd 1.0 gives 0.83.

| Noise | Samples | Planted pairs found | False pairs | Tables with a false pair | Mean false discovery proportion |
|---|---|---|---|---|---|
| 0.3 | 10 | 239 of 250 | 19 | 13 of 50 | 5.7% |
| 0.3 | 20 | 250 of 250 | 14 | 12 of 50 | 4.5% |
| 0.3 | 40 | 250 of 250 | 12 | 9 of 50 | 3.6% |
| 0.3 | 60 | 250 of 250 | 20 | 13 of 50 | 5.9% |
| 1.0 | 10 | 16 of 250 | 6 | 5 of 50 | 8.7% |
| 1.0 | 20 | 199 of 250 | 15 | 13 of 50 | 5.9% |
| 1.0 | 40 | 250 of 250 | 14 | 11 of 50 | 4.2% |
| 1.0 | 60 | 250 of 250 | 17 | 13 of 50 | 5.3% |
| 1.0, `--alternative greater` | 20 / 40 / 60 | 217 / 250 / 250 of 250 | 15 / 13 / 13 | 11 / 11 / 11 of 50 | 5.0% / 4.1% / 4.1% |
| none planted | 20 / 40 / 60 | – | 5 / 2 / 1 | 3 / 2 / 1 of 50 | – |

So the false discovery proportion stays near the 5% asked for, a little above in some rows; with
50 tables per row the estimates vary by about ±2 percentage points. Most false pairs are negative,
and several were two planted pairs tested against each other (four pairs from one chance event).
For host candidates, `--alternative greater` drops negative pairs.

**Unit and CLI tests** (`cargo test -p cooccur`): CoverM headers with one or several measures, the
`unmapped` row, plain tables, refused measures and malformed rows; GTDB-Tk and two-column
taxonomies; CLR and rho against values worked by hand; tie-averaged ranks; scale invariance; a
simulated proportional pair found, with at most one false pair among 435; the same output on 1 and
4 threads; and the binary run on the CoverM 0.8.0 table above.

## Speed and memory

A simulated table of 500 genomes × 60 samples (124,750 pairs, 20 planted pairs) on a laptop with a
Ryzen 5 PRO 4650U (6 cores, 12 threads), shared with other jobs:

| Run | Wall time | CPU time | Peak memory | Result |
|---|---|---|---|---|
| rho, 12 threads | 5.6–6.4 s (3 runs) | 19.5 s | 18 MiB | 21 pairs pass: the 20 planted (rho 0.97–0.99, 10⁶ permutations each) and one false (rho 0.57) |
| rho, 1 thread | 22.6 s | 17.9 s | 20 MiB | same output, byte for byte |
| spearman, 12 threads | 59 s | 217 s | 19 MiB | the 20 planted pairs |

Spearman re-ranks both genomes for every permutation, hence the extra time. Memory grows with the
number of pairs (roughly 100 bytes each while running) and the table, not with permutations. The release binary is
1.0 MB.

## Known gaps

- No SparCC, no conditional (partial) association, no time-series or repeated-measures structure:
  samples are treated as exchangeable.
- The permutation null is approximate for relative data (measured above).
- One table at a time. Several sequencing runs of one sample should be merged before, or
  `coverm genome` given them as one sample.
