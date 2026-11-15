# cooccur on real data: a known CPR–host pair

[`cooccur`](../tools/cooccur.md) had been checked on simulations and against propr. This page
tests it on real data with a positive control. The best-known CPR–host pair is the Saccharibacteria
TM7x ("*Candidatus* Nanosynbacter lyticus"), which grows attached to *Schaalia odontolytica*
(formerly *Actinomyces odontolyticus*) strain XH001 from the human mouth
([He et al. 2015](https://doi.org/10.1073/pnas.1419038112)). In culture, 13 of 27 *Actinomyces* strains
tested, XH001 included, could host TM7x ([Utter et al. 2020](https://doi.org/10.1038/s41396-020-00736-6)).
The question: in many oral metagenomes, does cooccur put Saccharibacteria together with
*A. odontolyticus*, or with *Actinomyces* in general?

Run on 2026-11-15 with cooccur from irit 0.6.0 (commit `07b274c`). Numbers are **measured** here
unless a source is given.

## Summary

- **The positive control is not recovered.** In 128 supragingival plaque and 136 tongue dorsum
  metagenomes from the Human Microbiome Project, neither Saccharibacteria profile has a positive
  association with *A. odontolyticus* or any other *Actinomyces*. In every run *A. odontolyticus*
  ranks in the lower half of the CPR genome's partners (61st to 111th of 112–121), with rho
  between −0.11 and −0.35.
  On the full plaque table the negative association passes q ≤ 0.05 (rho −0.26 and −0.25 for the
  two Saccharibacteria profiles). At genus level, Saccharibacteria against *Actinomyces* gives rho
  −0.37 in plaque and −0.24 on the tongue, both passing.
- **The partners that do pass are anaerobes.** The Saccharibacteria profiles' top positive partners in
  plaque are *Prevotella*, *Leptotrichia*, *Selenomonas*, *Centipeda* and *Treponema* (rho 0.34–0.49).
  These organisms tend to be found in older, more anaerobic plaque, so the pattern looks like plaque
  ecology rather than host dependence (interpretation).
- **It's not a computing error.** An independent Python recomputation gave the same rho and
  Spearman values for the control pair to all six printed decimals. Without the log-ratio
  transform, the plain Spearman correlation of the percentages is also near zero (+0.04 to +0.05 in
  plaque). No measure tried gives a positive association.
- **The negative control is close to nominal.** With each genome's values shuffled across samples
  ([section 4](#4-negative-control)), 4.86–5.00% of null pairs had p ≤ 0.05 and 0.95–1.03% had p ≤ 0.01,
  and 50 shuffled tables per run gave 4 or 5 false pairs passing q ≤ 0.05 in total.
- **Real oral tables are densely associated.** 28–44% of species pairs pass q ≤ 0.05 (50–56% at
  genus level), against 0.002% in the shuffles. With 90–136 samples, cooccur gives small
  q-values to weak associations too: the control pair passed at rho −0.26. On data like this,
  significance alone does not pick out a host. Ranking by rho, with `--min-stat`, is needed, and
  even that did not work here.

## 1. Data

**Source.** curatedMetagenomicData ([Pasolli et al. 2017](https://doi.org/10.1038/nmeth.4468)),
ExperimentHub resource `EH2491`, "20190423.HMP_2012.metaphlan_bugs_list.oralcavity": MetaPhlAn2
([Truong et al. 2015](https://doi.org/10.1038/nmeth.3589)) relative abundances of the 415 oral
metagenomes of the Human Microbiome Project's 2012 data set, with per-sample metadata (subject,
body subsite, read count). URL:
`https://mghp.osn.xsede.org/bir190004-bucket01/ExperimentHub/curatedMetagenomicData/20190423/20190423.HMP_2012.metaphlan_bugs_list.oralcavity.rda`
(708,007 bytes, MD5 `9e7575da931c5612d20143f108a712aa`, which matched the server's ETag; status
"Public" in the ExperimentHub metadata of 2026-11-13). curatedMetagenomicData is under the
Artistic-2.0 licence.

The file is an R `ExpressionSet`. R isn't installed here, so the matrix (1,988 clades × 415 samples)
and the sample table were read with the Python package `rdata` (`eset2tsv.py`, kept with the
results). The 415 samples come from 91 people: 128 supragingival plaque, 136 tongue dorsum, 119
buccal mucosa and 32 from six other subsites.

**Why this table.** Three other public tables were checked, and none of them works:

| Table | Saccharibacteria? | Note |
|---|---|---|
| HMP DACC `HMSMCP/HMP.ab.txt` (MetaPhlAn 1, 2012), on the HMP AWS bucket | No | Has *A. odontolyticus*, no TM7. |
| curatedMetagenomicData 3, `EH5584` "2021-03-31.HMP_2012.relative_abundance" (MetaPhlAn 3, 748 samples, all sites) | No | No *Saccharibacteria* phylum among its 18 phyla. |
| Shaiber et al. 2020 Table S07 ([paper](https://doi.org/10.1186/s13059-020-02195-w), figshare, CC BY 4.0) | Yes, 50+ TM7 genomes in 481 HMP oral samples | Coverage of the TM7 genomes only. No host genomes in the same table. |

The MetaPhlAn2 profiles have two Saccharibacteria species: the single-cell genomes TM7b and TM7c
([Marcy et al. 2007](https://doi.org/10.1073/pnas.0704662104)). There is no TM7x entry. Reads of
other oral Saccharibacteria, TM7x among them, may hit these two marker sets. So TM7b and TM7c here
stand for "Saccharibacteria close to these genomes", not for one strain. Presence (subset of the
table above):

| Subsite | Samples | TM7c present | TM7c mean | *A. odontolyticus* present | its mean | *Actinomyces* (genus) mean |
|---|---|---|---|---|---|---|
| supragingival plaque | 128 | 107 | 1.03% | 120 | 0.16% | 9.4% |
| tongue dorsum | 136 | 124 | 0.03% | 136 | 0.54% | 8.7% |
| buccal mucosa | 119 | 48 | 0.10% | 104 | 0.22% | 2.8% |

Plaque, where Saccharibacteria are most abundant, is the main test. Tongue is the second one.

## 2. Commands

Inputs (scripts in `bioinfo-lab/cooccur-real/scripts/`): `make_inputs.py` writes one species ×
sample table per subsite (MetaPhlAn rows with `s__` and no `t__`, species absent from every sample
dropped) and a two-column taxonomy (species, MetaPhlAn lineage with `;` between ranks). With
`--one-per-subject` it keeps each person's first sample only. HMP sampled some people twice, and
cooccur assumes exchangeable samples.

```sh
python3 make_inputs.py oral_mpa2.tsv oral_meta.tsv supragingival_plaque supragingival_plaque
cooccur supragingival_plaque.tsv -t supragingival_plaque.tax.tsv \
  --cpr-taxon p__Candidatus_Saccharibacteria --method rho --min-prevalence 0.5 --fdr 0.05 \
  --all-pairs --summary supragingival_plaque.rho.json -o supragingival_plaque.rho.all.tsv
# the same with --method spearman, for tongue_dorsum, for the one-per-subject tables,
# with --min-prevalence 0.8, and for genus tables (rows with g__ and no s__)
python3 rank.py supragingival_plaque.rho.all.tsv     # partner ranks of each CPR genome
```

MetaPhlAn's percentages go in as they are. cooccur's statistic does not depend on per-sample
scale, so the missing closure (MetaPhlAn sums to 100% over all ranks, not over the kept species)
doesn't matter. Each run took under a second for rho and a few seconds for Spearman.

## 3. Results

The plaque summary (rho, all 128 samples):

```
cooccur: 128 samples, 114 of 389 genomes kept (prevalence >= 0.5), 2830 zeros replaced by 4.000e-5
rho two-sided, 6441 pairs tested, 2469 pass q <= 0.05 (1160 positive, 1309 negative)
labels: 2 CPR, 112 non-CPR, 0 unclassified
CPR–non-CPR pairs passing: 125 (50 positive, 75 negative)
```

*A. odontolyticus* as a partner of each Saccharibacteria profile: its rank among all that genome's
partners (1 = most positive), the statistic and q. For genus tables, the partner is the genus
*Actinomyces*.

| Table | Samples | Method | Pairs passing | TM7b – *A. odontolyticus* | TM7c – *A. odontolyticus* |
|---|---|---|---|---|---|
| plaque | 128 | rho | 2,469 of 6,441 (38%) | 95 of 113, −0.26, q 0.009, passes | 88 of 113, −0.25, q 0.02, passes |
| plaque | 128 | spearman | 2,759 (43%) | 75 of 113, −0.24, q 0.02, passes | 73 of 113, −0.23, q 0.02, passes |
| plaque, one per person | 88 | rho | 1,761 of 6,328 (28%) | 85 of 112, −0.20, q 0.13 | 69 of 112, −0.16, q 0.20 |
| plaque, one per person | 88 | spearman | 2,060 (33%) | 63 of 112, −0.16, q 0.26 | 61 of 112, −0.11, q 0.35 |
| plaque, prevalence ≥ 0.8 | 128 | rho | 614 of 1,770 (35%) | (TM7b dropped) | 41 of 59, −0.19, q 0.07 |
| plaque, 17 low-diversity samples dropped | 111 | rho | 2,215 of 7,140 (31%) | 93 of 119, −0.21, q 0.07 | 84 of 119, −0.17, q 0.12 |
| tongue | 136 | rho | 3,123 of 7,260 (43%) | 106 of 120, −0.26, q 0.004, passes | 100 of 120, −0.19, q 0.06 |
| tongue | 136 | spearman | 3,204 (44%) | 111 of 120, −0.35, q 0.001, passes | 111 of 120, −0.20, q 0.055 |
| tongue, one per person | 90 | rho | 2,226 of 7,381 (30%) | 99 of 121, −0.21, q 0.09 | 96 of 121, −0.15, q 0.24 |
| tongue, one per person | 90 | spearman | 2,459 (33%) | 105 of 121, −0.28, q 0.03, passes | 82 of 121, −0.12, q 0.44 |
| plaque, genus | 128 | rho | 496 of 990 (50%) | Saccharibacteria – *Actinomyces*: 40 of 44, −0.37, q 0.0004, passes | |
| plaque, genus | 128 | spearman | 558 (56%) | 39 of 44, −0.45, q 0.0004, passes | |
| tongue, genus | 136 | rho | 705 of 1,378 (51%) | 43 of 52, −0.24, q 0.006, passes | |
| tongue, genus | 136 | spearman | 759 (55%) | 39 of 52, −0.22, q 0.03, passes | |

Every other *Actinomyces* species, and *Propionibacterium propionicum*, also ranks low against the
Saccharibacteria profiles in plaque: ranks 63–107 of 113 for TM7b, all with rho ≤ −0.08. On the
tongue, TM7c's positive partners include *A. johnsonii*, *A. naeslundii* and *A. oris* (rho
0.21–0.28, ranks 5, 7 and 15 of 120, passing). TM7b's are not (rho −0.05 to 0.14), and neither is
*A. odontolyticus*. With the two profiles disagreeing and the signal only on the tongue, this is
weak evidence at most.

TM7b and TM7c are each other's strongest partner (rho 0.84 in plaque, 0.52 on the tongue). That is
expected for two marker sets of related genomes, and it is the one pair here a reader could predict.

Top positive partners in plaque (rho, all 128 samples):

| TM7b | rho | TM7c | rho |
|---|---|---|---|
| *Leptotrichia buccalis* | 0.43 | *Prevotella saccharolytica* | 0.49 |
| *Prevotella saccharolytica* | 0.42 | *Treponema socranskii* | 0.43 |
| *Prevotella maculosa* | 0.41 | *Prevotella maculosa* | 0.41 |
| *Centipeda periodontii* | 0.40 | *Selenomonas infelix* | 0.41 |
| *Selenomonas artemidis* | 0.38 | *Mitsuokella* (unclassified) | 0.41 |

**Recomputed independently.** `check_pair.py` (plain Python: prevalence filter, minimum non-zero
pseudocount, CLR, rho, tie-averaged Spearman) gave rho −0.249734 and Spearman −0.230830 for
TM7c – *A. odontolyticus* in plaque, and −0.258052 and −0.242961 for TM7b. cooccur printed the
same values.

## 4. Negative control

Each genome's row was shuffled across samples independently (`shuffle.py`, seeds 1–50). That breaks
every association but keeps each genome's values and prevalence. cooccur then ran with the same
settings (`--seed` = shuffle seed, `--all-pairs`):

| Table | Method | Null pairs (50 tables) | p ≤ 0.05 | p ≤ 0.01 | Pairs passing q ≤ 0.05 | Tables with any |
|---|---|---|---|---|---|---|
| plaque | rho | 322,050 | 5.00% | 1.03% | 5 | 5 of 50 |
| plaque | spearman | 322,050 | 4.86% | 0.95% | 4 | 2 of 50 |
| tongue | rho | 363,000 | 4.90% | 1.00% | 5 | 4 of 50 |
| tongue | spearman | 363,000 | 4.88% | 0.97% | 4 | 3 of 50 |

With every pair null, Benjamini-Hochberg at 0.05 lets through at least one false pair in at most
about 5% of tables. Here it was 4–10% of tables (2 to 5 of 50; with 50 tables the binomial range
is wide), with one or two false pairs when there were any. The real tables, at 28–56% passing, are
nowhere near this null. The pass rate measures real structure in the data, not a broken test.

## 5. What this shows and what it doesn't

**On real oral profiles, cooccur doesn't find the cultured host.** The control pair ranks near the
bottom, and the genus-level association is significantly negative. Some reasons, none of them tested
here:

- **The relationship is not a proportional one.** TM7x is a parasite. In culture, permissive hosts
  grow, crash and recover after infection ([Utter et al. 2020](https://doi.org/10.1038/s41396-020-00736-6)),
  and the host evolves lower susceptibility within weeks ([Bor et al. 2018](https://doi.org/10.1073/pnas.1810625115)).
  Such dynamics need not leave a positive correlation in one snapshot per person.
- **Hosts are a few strains, and the profile is a species or genus.** Of 27 *Actinomyces* strains, 13
  hosted TM7x. A MetaPhlAn species sums permissive and non-permissive strains, and the
  Saccharibacteria profiles mix several Saccharibacteria. A host signal could be diluted by both.
- **Habitat dominates.** The strongest axis in plaque runs from *Actinomyces*/*Rothia*-rich plaque to
  plaque rich in anaerobes. The Saccharibacteria profiles sit with the anaerobes, *Actinomyces* on the
  other side (interpretation of the partner lists). Co-occurrence measures shared habitat first
  ([Faust & Raes 2012](https://doi.org/10.1038/nrmicro2832)).
- **The CLR reference follows the zeros.** In the plaque table, the per-sample mean log, which every
  CLR value is measured against, had a Spearman correlation of −0.95 with the number of zeros in
  the sample. Zero counts ranged from 2 to 113 of 114 species. One sample was 100% *Rothia
  dentocariosa*. The zeros track sequencing depth (Spearman −0.47 with read count) and community
  type. TM7c is absent mostly from low-diversity samples (Spearman −0.64 between its abundance and
  the sample's zero count); *A. odontolyticus* is present almost everywhere (−0.10). That turns a
  near-zero association of the percentages (+0.05) into a negative CLR one (−0.25). Dropping the 17
  samples with ≥ 40 zeros, or keeping only species in ≥ 80% of samples, weakened the control pair
  to rho −0.17 to −0.21, no longer passing. It was still not positive.

**What the test doesn't cover.**

- Association, not interaction, as the [tool page](../tools/cooccur.md) says. A positive pair would
  not have proven a host either.
- One body site at a time, one cohort (healthy US adults, 2009–2011), one or two samples per
  person. No time series, so lagged parasite–host dynamics are invisible.
- Profiles, not MAGs. MetaPhlAn2 has no TM7x entry. Its two Saccharibacteria species are 2007
  single-cell genomes, and its *A. odontolyticus* is a species, not the XH001 strain. A MAG-based
  table (all genomes mapped across the same samples, as in [Shaiber et al.
  2020](https://doi.org/10.1186/s13059-020-02195-w) for TM7 only) would test strains, at the cost
  of mapping 400+ metagenomes.
- MetaPhlAn relative abundances are already relative, and cooccur's CLR handles that: rho is the same
  whether a sample sums to 100% or not. What the CLR cannot fix is zeros. A MetaPhlAn zero means "no
  marker reads", and how often that happens depends on depth, so the pseudocount carries depth
  into every CLR value. `n_both` shows how many samples each pair rests on (TM7c – *A. odontolyticus*:
  101 of 128).

## 6. Notes for cooccur

No bug was found, and cooccur's code was not changed. Two things came up:

- **q printed as `0.0500` for pairs that don't pass.** With `--all-pairs`, q has four significant
  digits, so a pair at q = 0.05003 shows as `0.0500` and is not in the pass count (e.g. 1,762 rows
  show q ≤ 0.0500 where the summary says 1,761 pass). A `pass` column, or more digits, would make the
  file unambiguous.
- **A zero diagnostic would help.** On real profiles, the zeros dominated the CLR reference. A
  summary line with the per-sample zero range, and a warning when it is wide, would flag this. Also
  `--min-prevalence 0.8` as a suggested setting for MetaPhlAn tables.

## 7. Files

Kept in `bioinfo-lab/cooccur-real/` (not in the repo):
`oral_mpa2.tsv.gz` (the 415-sample MetaPhlAn2 table as TSV) and `oral_meta.tsv` (sample table),
both from `EH2491` (Artistic-2.0); the per-subsite inputs, taxonomies, `--all-pairs` edge lists,
summaries and logs of every run above; and the scripts (`eset2tsv.py`, `make_inputs.py`, `rank.py`,
`summarise.py`, `check_pair.py`, `shuffle.py`).
