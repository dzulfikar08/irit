//! cooccur: which genomes rise and fall together across samples, from a `coverm genome` table.
//!
//! Abundances are compositional (each sample is a share of the reads), so pairs are compared on
//! centred log-ratios (CLR) after zero replacement, with proportionality rho (Lovell et al. 2015;
//! Erb & Notredame 2016) or Spearman's correlation of CLR values. Each pair gets a sequential
//! permutation p-value (Besag & Clifford 1991) from its own seeded stream, and Benjamini-Hochberg
//! q-values over all pairs. With a GTDB-Tk taxonomy, genomes are labelled CPR or not, and the
//! summary lists CPR–non-CPR pairs that pass: candidates for host pairs, not evidence of one.

mod stats;
mod table;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use serde_json::{Value, json};
use stats::{Alternative, Method, Pair, PermTest};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use table::Table;

#[derive(Parser)]
#[command(version, about = "Co-occurrence of genome abundances across samples: CLR proportionality or Spearman, permutation p, BH q")]
struct Cli {
    /// Genome × sample table: `coverm genome` output (Genome, then `<sample> <Method>` columns), or
    /// a plain TSV with the genome in the first column and one column per sample.
    table: PathBuf,

    /// CoverM measure to use when the table has several: relative_abundance, count, mean,
    /// trimmed_mean, reads_per_base, rpkm or tpm (or the column name, e.g. "Read Count").
    #[arg(short, long)]
    measure: Option<String>,

    /// Taxonomy table: a GTDB-Tk summary (user_genome, classification) or two columns (genome,
    /// lineage). Repeat for the bacterial and archaeal summaries.
    #[arg(short, long)]
    taxonomy: Vec<PathBuf>,

    /// Lineage rank that marks a genome as CPR (exact match of one `;`-separated rank). Repeat to
    /// give several. Default: p__Patescibacteria and p__Patescibacteriota (GTDB's two names).
    #[arg(long)]
    cpr_taxon: Vec<String>,

    /// Statistic: rho (proportionality on CLR values) or spearman (rank correlation of CLR values).
    #[arg(long, value_enum, default_value_t = MethodArg::Rho)]
    method: MethodArg,

    /// Keep genomes present in at least this fraction of samples.
    #[arg(long, default_value_t = 0.5)]
    min_prevalence: f64,

    /// A genome counts as present in a sample when its value is above zero and at least this.
    #[arg(long, default_value_t = 0.0)]
    min_abundance: f64,

    /// Value that replaces zeros before the log-ratio: `min` (the smallest non-zero value in the
    /// filtered table, as the propr package does) or a number, e.g. 1 for read counts.
    #[arg(long, default_value = "min")]
    pseudocount: String,

    /// two-sided: association in either direction; greater: only positive association.
    #[arg(long, value_enum, default_value_t = AltArg::TwoSided)]
    alternative: AltArg,

    /// Pairs with q at or below this pass.
    #[arg(long, default_value_t = 0.05)]
    fdr: f64,

    /// Pairs must also reach this statistic (absolute value for two-sided), e.g. 0.6.
    #[arg(long, default_value_t = 0.0)]
    min_stat: f64,

    /// Permutations per pair at first. A pair that reaches this with fewer than --stop-after
    /// exceedances and q above --fdr is rerun with ten times as many, up to --max-permutations.
    #[arg(long, default_value_t = 10_000)]
    permutations: usize,

    /// Most permutations per pair; the smallest p a pair can get is 1/(this + 1).
    #[arg(long, default_value_t = 10_000_000)]
    max_permutations: usize,

    /// Stop a pair's permutations after this many were at least as extreme as the observed value.
    #[arg(long, default_value_t = 10)]
    stop_after: usize,

    /// Random seed: the same seed and input give the same output on any number of threads.
    #[arg(long, default_value_t = 1)]
    seed: u64,

    /// Threads (0: all available).
    #[arg(long, default_value_t = 0)]
    threads: usize,

    /// Edge list TSV (default: standard output).
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Write every tested pair, not only the pairs that pass.
    #[arg(long)]
    all_pairs: bool,

    /// Write the summary as JSON to this file (the text summary always goes to standard error).
    #[arg(long)]
    summary: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum MethodArg {
    Rho,
    Spearman,
}

#[derive(Clone, Copy, ValueEnum)]
enum AltArg {
    TwoSided,
    Greater,
}

/// The analysis settings, apart from input and output.
#[derive(Clone, Debug)]
struct Opts {
    min_prevalence: f64,
    min_abundance: f64,
    /// None: the smallest non-zero value.
    pseudocount: Option<f64>,
    test: PermTest,
    fdr: f64,
    min_stat: f64,
    seed: u64,
    threads: usize,
    cpr_taxa: Vec<String>,
}

const DEFAULT_CPR: &[&str] = &["p__Patescibacteria", "p__Patescibacteriota"];

/// Fewest samples cooccur accepts.
const MIN_SAMPLES: usize = 5;

struct Analysis {
    samples: usize,
    genomes_in: usize,
    kept: Vec<usize>,
    low_prevalence: Vec<String>,
    constant: Vec<String>,
    pseudocount: f64,
    zeros_replaced: usize,
    /// Per kept genome: (label, phylum); empty labels without a taxonomy.
    labels: Vec<(String, String)>,
    /// Per kept genome: samples where present.
    present: Vec<Vec<bool>>,
    pairs: Vec<Pair>,
    warnings: Vec<String>,
}

impl Analysis {
    fn n_both(&self, p: &Pair) -> usize {
        self.present[p.i].iter().zip(&self.present[p.j]).filter(|(a, b)| **a && **b).count()
    }

    fn passes(&self, p: &Pair, o: &Opts) -> bool {
        let s = match o.test.alternative {
            Alternative::TwoSided => p.stat.abs(),
            Alternative::Greater => p.stat,
        };
        p.q <= o.fdr && s >= o.min_stat
    }
}

fn label(lineage: Option<&String>, cpr: &[String]) -> (String, String) {
    match lineage {
        None => ("unclassified".into(), String::new()),
        Some(l) => {
            let phylum = table::phylum(l).to_string();
            let lab = if l.split(';').map(str::trim).any(|r| cpr.iter().any(|c| c == r)) {
                "CPR"
            } else if phylum.is_empty() {
                "unclassified"
            } else {
                "non-CPR"
            };
            (lab.into(), phylum)
        }
    }
}

fn analyse(t: &Table, o: &Opts, lineages: Option<&HashMap<String, String>>) -> Result<Analysis> {
    let n = t.samples.len();
    if n < MIN_SAMPLES {
        bail!("{n} samples; co-occurrence needs at least {MIN_SAMPLES} (and many more to find anything: see the guide)");
    }
    let present_row = |r: &Vec<f64>| -> Vec<bool> { r.iter().map(|&v| v > 0.0 && v >= o.min_abundance).collect() };
    let mut kept = Vec::new();
    let mut low_prevalence = Vec::new();
    for (g, r) in t.values.iter().enumerate() {
        let prev = present_row(r).iter().filter(|&&p| p).count() as f64 / n as f64;
        if prev >= o.min_prevalence && prev > 0.0 {
            kept.push(g);
        } else {
            low_prevalence.push(t.genomes[g].clone());
        }
    }
    // Values below --min-abundance count as absent, so they become zeros before replacement.
    let mut values: Vec<Vec<f64>> =
        kept.iter().map(|&g| t.values[g].iter().map(|&v| if v >= o.min_abundance { v } else { 0.0 }).collect()).collect();
    let pseudocount = match o.pseudocount {
        Some(p) => p,
        None => stats::smallest_nonzero(&values).unwrap_or(1.0),
    };
    let zeros_replaced = stats::replace_zeros(&mut values, pseudocount);
    let mut logs = stats::Logs::new(&values);
    // A genome with the same CLR value in every sample has no variance: no statistic.
    let mut constant = Vec::new();
    let varying: Vec<usize> = (0..kept.len()).filter(|&k| logs.clr_ss(k) > 1e-20).collect();
    if varying.len() < kept.len() {
        constant = (0..kept.len()).filter(|k| !varying.contains(k)).map(|k| t.genomes[kept[k]].clone()).collect();
        logs = logs.subset(&varying);
        kept = varying.iter().map(|&k| kept[k]).collect();
    }
    if kept.len() < 2 {
        bail!("{} genome(s) left after filtering (prevalence {}); need at least two", kept.len(), o.min_prevalence);
    }
    let labels: Vec<(String, String)> = match lineages {
        Some(map) => kept.iter().map(|&g| label(map.get(&table::genome_key(&t.genomes[g])), &o.cpr_taxa)).collect(),
        None => vec![(String::new(), String::new()); kept.len()],
    };
    let present: Vec<Vec<bool>> = kept.iter().map(|&g| present_row(&t.values[g])).collect();
    let pairs = stats::all_pairs(&logs, o.test, o.fdr, o.seed, o.threads);

    let mut warnings = Vec::new();
    let m = pairs.len() as f64;
    let p_min = 1.0 / (o.test.max as f64 + 1.0);
    if p_min * m > o.fdr {
        warnings.push(format!(
            "with {} pairs and at most {} permutations, a pair needs at least {:.0} pairs at the smallest p (1/{}) to reach q {}; raise --max-permutations",
            pairs.len(),
            o.test.max,
            (p_min * m / o.fdr).ceil(),
            o.test.max + 1,
            o.fdr
        ));
    }
    let cells = (values.len() * n) as f64;
    if zeros_replaced as f64 / cells > 0.2 {
        warnings.push(format!(
            "{:.0}% of the kept values were zeros replaced by {pseudocount}; shared zeros can make pairs look proportional (check n_both)",
            100.0 * zeros_replaced as f64 / cells
        ));
    }
    if n < 20 {
        warnings.push(format!("only {n} samples: with fewer than 20, only very strong associations pass (see the guide's simulation)"));
    }
    if lineages.is_some() {
        let missing = labels.iter().filter(|l| l.0 == "unclassified").count();
        if missing > 0 {
            warnings.push(format!("{missing} kept genome(s) have no phylum in the taxonomy (labelled unclassified)"));
        }
    }
    Ok(Analysis { samples: n, genomes_in: t.genomes.len(), kept, low_prevalence, constant, pseudocount, zeros_replaced, labels, present, pairs, warnings })
}

/// Four significant digits, or scientific notation below 0.001.
fn num(v: f64) -> String {
    if v == 0.0 || v.abs() >= 1e-3 { format!("{v:.4}") } else { format!("{v:.3e}") }
}

fn method_name(m: Method) -> &'static str {
    match m {
        Method::Rho => "rho",
        Method::Spearman => "spearman",
    }
}

/// Pairs in output order: q, then the strongest statistic, then table order.
fn ordered<'a>(a: &'a Analysis, o: &Opts, all: bool) -> Vec<&'a Pair> {
    let mut v: Vec<&Pair> = a.pairs.iter().filter(|p| all || a.passes(p, o)).collect();
    v.sort_by(|x, y| x.q.total_cmp(&y.q).then(y.stat.abs().total_cmp(&x.stat.abs())).then((x.i, x.j).cmp(&(y.i, y.j))));
    v
}

fn write_edges(w: &mut dyn Write, t: &Table, a: &Analysis, o: &Opts, all: bool) -> Result<()> {
    writeln!(w, "genome_a\tgenome_b\t{}\tp\tq\tpermutations\tn_both\tlabel_a\tlabel_b\tphylum_a\tphylum_b", method_name(o.test.method))?;
    for p in ordered(a, o, all) {
        let (la, pa) = &a.labels[p.i];
        let (lb, pb) = &a.labels[p.j];
        writeln!(
            w,
            "{}\t{}\t{:.6}\t{}\t{}\t{}\t{}\t{la}\t{lb}\t{pa}\t{pb}",
            t.genomes[a.kept[p.i]],
            t.genomes[a.kept[p.j]],
            p.stat,
            num(p.p),
            num(p.q),
            p.perms,
            a.n_both(p)
        )?;
    }
    Ok(())
}

/// The summary: settings, counts, and the CPR–non-CPR pairs that pass (CPR genome first).
fn summary(t: &Table, a: &Analysis, o: &Opts, lineages: bool) -> Value {
    let passing: Vec<&Pair> = ordered(a, o, false);
    let positive = passing.iter().filter(|p| p.stat > 0.0).count();
    let mut counts = serde_json::Map::new();
    if lineages {
        for l in ["CPR", "non-CPR", "unclassified"] {
            counts.insert(l.into(), json!(a.labels.iter().filter(|x| x.0 == l).count()));
        }
    }
    let cpr_pairs: Vec<Value> = passing
        .iter()
        .filter_map(|p| {
            let (x, y) = match (a.labels[p.i].0.as_str(), a.labels[p.j].0.as_str()) {
                ("CPR", "non-CPR") => (p.i, p.j),
                ("non-CPR", "CPR") => (p.j, p.i),
                _ => return None,
            };
            Some(json!({
                "cpr": t.genomes[a.kept[x]], "partner": t.genomes[a.kept[y]], "partner_phylum": a.labels[y].1,
                "statistic": p.stat, "p": p.p, "q": p.q, "n_both": a.n_both(p),
            }))
        })
        .collect();
    json!({
        "tool": "cooccur",
        "version": env!("CARGO_PKG_VERSION"),
        "measure": t.measure,
        "samples": a.samples,
        "genomes_in_table": a.genomes_in,
        "genomes_kept": a.kept.len(),
        "dropped_low_prevalence": a.low_prevalence,
        "dropped_constant": a.constant,
        "min_prevalence": o.min_prevalence,
        "min_abundance": o.min_abundance,
        "pseudocount": a.pseudocount,
        "pseudocount_rule": if o.pseudocount.is_some() { "given" } else { "smallest non-zero value" },
        "zeros_replaced": a.zeros_replaced,
        "method": method_name(o.test.method),
        "alternative": match o.test.alternative { Alternative::TwoSided => "two-sided", Alternative::Greater => "greater" },
        "permutations": o.test.start,
        "max_permutations": o.test.max,
        "stop_after": o.test.stop_after,
        "seed": o.seed,
        "fdr": o.fdr,
        "min_stat": o.min_stat,
        "pairs_tested": a.pairs.len(),
        "pairs_passing": passing.len(),
        "passing_positive": positive,
        "passing_negative": passing.len() - positive,
        "labels": if lineages { Value::Object(counts) } else { Value::Null },
        "cpr_pairs_passing": if lineages { json!(cpr_pairs) } else { Value::Null },
        "warnings": a.warnings,
    })
}

fn print_summary(s: &Value) {
    let g = |k: &str| s[k].to_string();
    eprintln!(
        "cooccur: {} samples, {} of {} genomes kept (prevalence >= {}), {} zeros replaced by {}",
        g("samples"),
        g("genomes_kept"),
        g("genomes_in_table"),
        g("min_prevalence"),
        g("zeros_replaced"),
        num(s["pseudocount"].as_f64().unwrap_or(0.0))
    );
    eprintln!(
        "{} {}, {} pairs tested, {} pass q <= {} ({} positive, {} negative)",
        s["method"].as_str().unwrap_or(""),
        s["alternative"].as_str().unwrap_or(""),
        g("pairs_tested"),
        g("pairs_passing"),
        g("fdr"),
        g("passing_positive"),
        g("passing_negative")
    );
    if let Some(l) = s["labels"].as_object() {
        let parts: Vec<String> = l.iter().map(|(k, v)| format!("{v} {k}")).collect();
        eprintln!("labels: {}", parts.join(", "));
        let cp = s["cpr_pairs_passing"].as_array().map_or(&[][..], |v| &v[..]);
        let pos = cp.iter().filter(|p| p["statistic"].as_f64().is_some_and(|v| v > 0.0)).count();
        eprintln!("CPR–non-CPR pairs passing: {} ({pos} positive, {} negative)", cp.len(), cp.len() - pos);
        for p in cp.iter().take(30) {
            eprintln!(
                "  {}\t{}\t{}\t{:.4}\tq {}\tboth present in {}",
                p["cpr"].as_str().unwrap_or(""),
                p["partner"].as_str().unwrap_or(""),
                p["partner_phylum"].as_str().unwrap_or(""),
                p["statistic"].as_f64().unwrap_or(f64::NAN),
                num(p["q"].as_f64().unwrap_or(f64::NAN)),
                p["n_both"]
            );
        }
        if cp.len() > 30 {
            eprintln!("  ... {} more in the edge list", cp.len() - 30);
        }
    }
    for w in s["warnings"].as_array().into_iter().flatten() {
        eprintln!("warning: {}", w.as_str().unwrap_or(""));
    }
}

fn opts(cli: &Cli) -> Result<Opts> {
    if !(0.0..=1.0).contains(&cli.min_prevalence) {
        bail!("--min-prevalence is a fraction from 0 to 1");
    }
    if !(cli.fdr > 0.0 && cli.fdr <= 1.0) {
        bail!("--fdr must be above 0 and at most 1");
    }
    if cli.permutations == 0 || cli.max_permutations == 0 || cli.stop_after == 0 {
        bail!("--permutations, --max-permutations and --stop-after must be at least 1");
    }
    let pseudocount = match cli.pseudocount.as_str() {
        "min" => None,
        s => {
            let v: f64 = s.parse().with_context(|| format!("--pseudocount '{s}' is neither `min` nor a number"))?;
            if !(v.is_finite() && v > 0.0) {
                bail!("--pseudocount must be above 0");
            }
            Some(v)
        }
    };
    let threads = match cli.threads {
        0 => std::thread::available_parallelism().map_or(1, |n| n.get()),
        t => t,
    };
    Ok(Opts {
        min_prevalence: cli.min_prevalence,
        min_abundance: cli.min_abundance,
        pseudocount,
        test: PermTest {
            method: match cli.method {
                MethodArg::Rho => Method::Rho,
                MethodArg::Spearman => Method::Spearman,
            },
            start: cli.permutations.min(cli.max_permutations),
            max: cli.max_permutations,
            stop_after: cli.stop_after,
            alternative: match cli.alternative {
                AltArg::TwoSided => Alternative::TwoSided,
                AltArg::Greater => Alternative::Greater,
            },
        },
        fdr: cli.fdr,
        min_stat: cli.min_stat,
        seed: cli.seed,
        threads,
        cpr_taxa: if cli.cpr_taxon.is_empty() { DEFAULT_CPR.iter().map(|s| s.to_string()).collect() } else { cli.cpr_taxon.clone() },
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let o = opts(&cli)?;
    let t = table::read_table(&cli.table, cli.measure.as_deref())?;
    let lineages = if cli.taxonomy.is_empty() { None } else { Some(table::read_taxonomy(&cli.taxonomy)?) };
    let a = analyse(&t, &o, lineages.as_ref())?;
    match &cli.output {
        Some(p) => {
            let mut f = std::io::BufWriter::new(std::fs::File::create(p).with_context(|| format!("creating {}", p.display()))?);
            write_edges(&mut f, &t, &a, &o, cli.all_pairs)?;
            f.flush()?;
        }
        None => {
            let mut out = std::io::BufWriter::new(std::io::stdout().lock());
            write_edges(&mut out, &t, &a, &o, cli.all_pairs)?;
            out.flush()?;
        }
    }
    let s = summary(&t, &a, &o, lineages.is_some());
    if let Some(p) = &cli.summary {
        std::fs::write(p, serde_json::to_string_pretty(&s)? + "\n").with_context(|| format!("writing {}", p.display()))?;
    }
    print_summary(&s);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use stats::Rng;

    /// Standard normal from two uniforms (Box-Muller).
    fn normal(r: &mut Rng) -> f64 {
        let u1 = ((r.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
        let u2 = ((r.next_u64() >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }

    fn opts_default() -> Opts {
        Opts {
            min_prevalence: 0.5,
            min_abundance: 0.0,
            pseudocount: None,
            test: PermTest { method: Method::Rho, start: 10_000, max: 10_000_000, stop_after: 10, alternative: Alternative::TwoSided },
            fdr: 0.05,
            min_stat: 0.0,
            seed: 1,
            threads: 4,
            cpr_taxa: DEFAULT_CPR.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// `k` genomes × `n` samples of relative abundance (%): independent log-normal abundances,
    /// except genome 1 = 3 × genome 0 × log-normal noise (sd `noise` on the log scale), so the pair
    /// (0, 1) is proportional up to noise. Values below `detection` (% of the sample) become zeros.
    fn simulate(k: usize, n: usize, noise: f64, detection: f64, seed: u64) -> Table {
        let mut r = Rng::new(seed);
        let mut raw: Vec<Vec<f64>> = (0..k).map(|_| (0..n).map(|_| (1.5 * normal(&mut r)).exp()).collect()).collect();
        for s in 0..n {
            raw[1][s] = 3.0 * raw[0][s] * (noise * normal(&mut r)).exp();
        }
        for s in 0..n {
            let total: f64 = raw.iter().map(|g| g[s]).sum();
            for g in raw.iter_mut() {
                g[s] *= 100.0 / total;
                if g[s] < detection {
                    g[s] = 0.0;
                }
            }
        }
        Table {
            genomes: (0..k).map(|g| format!("g{g}")).collect(),
            samples: (0..n).map(|s| format!("s{s}")).collect(),
            values: raw,
            measure: Some("Relative Abundance (%)".into()),
        }
    }

    #[test]
    fn simulated_proportional_pair_is_found_and_independent_ones_are_not() {
        let t = simulate(30, 40, 0.2, 0.05, 11);
        let o = opts_default();
        let a = analyse(&t, &o, None).unwrap();
        assert!(a.zeros_replaced > 0, "the simulation should exercise zero replacement");
        let pass: Vec<(usize, usize)> = a.pairs.iter().filter(|p| a.passes(p, &o)).map(|p| (a.kept[p.i], a.kept[p.j])).collect();
        assert!(pass.contains(&(0, 1)), "the proportional pair must pass: {pass:?}");
        let p01 = a.pairs.iter().find(|p| (a.kept[p.i], a.kept[p.j]) == (0, 1)).unwrap();
        assert!(p01.stat > 0.9, "rho {}", p01.stat);
        // at FDR 0.05 with one true pair, more than one false pair would be unexpected
        assert!(pass.len() <= 2, "independent pairs passed: {pass:?}");
    }

    #[test]
    fn rho_ignores_per_genome_and_per_sample_scale() {
        // CLR rho and its permutation p are unchanged when a genome is multiplied by a constant
        // (e.g. genome length: read count vs depth) or a sample by a constant (sequencing depth),
        // as long as there are no zeros.
        let t = simulate(8, 20, 0.3, 0.0, 5);
        let mut u = t.clone();
        for (g, row) in u.values.iter_mut().enumerate() {
            for (s, v) in row.iter_mut().enumerate() {
                *v *= (g + 1) as f64 * 1000.0 * (1.0 + s as f64 / 7.0);
            }
        }
        let mut o = opts_default();
        o.test.start = 500;
        o.test.max = 500;
        let a = analyse(&t, &o, None).unwrap();
        let b = analyse(&u, &o, None).unwrap();
        for (x, y) in a.pairs.iter().zip(&b.pairs) {
            assert!((x.stat - y.stat).abs() < 1e-10);
            // the permutation null is scale-free too: same permutations, same p
            assert_eq!((x.p, x.perms), (y.p, y.perms));
        }
    }

    #[test]
    fn labels_and_summary() {
        let t = simulate(4, 12, 0.1, 0.0, 9);
        let mut lin = HashMap::new();
        lin.insert("g0".to_string(), "d__Bacteria;p__Patescibacteriota;c__Paceibacteria".to_string());
        lin.insert("g1".to_string(), "d__Bacteria;p__Pseudomonadota".to_string());
        lin.insert("g2".to_string(), "Unclassified Bacteria".to_string());
        let o = opts_default();
        let a = analyse(&t, &o, Some(&lin)).unwrap();
        assert_eq!(a.labels[0], ("CPR".into(), "p__Patescibacteriota".into()));
        assert_eq!(a.labels[1].0, "non-CPR");
        assert_eq!(a.labels[2].0, "unclassified");
        assert_eq!(a.labels[3].0, "unclassified");
        let s = summary(&t, &a, &o, true);
        assert_eq!(s["labels"]["CPR"], 1);
        let cp = s["cpr_pairs_passing"].as_array().unwrap();
        assert_eq!(cp.len(), 1, "{s}");
        assert_eq!((cp[0]["cpr"].as_str(), cp[0]["partner"].as_str()), (Some("g0"), Some("g1")));
        assert!(a.warnings.iter().any(|w| w.starts_with("only 12 samples")));
        let mut buf = Vec::new();
        write_edges(&mut buf, &t, &a, &o, true).unwrap();
        let text = String::from_utf8(buf).unwrap();
        assert_eq!(text.lines().count(), 7);
        assert!(text.lines().nth(1).unwrap().starts_with("g0\tg1\t"), "{text}");
    }

    #[test]
    fn filtering_and_too_few_samples() {
        let mut t = simulate(5, 10, 0.1, 0.0, 2);
        for v in t.values[4].iter_mut().take(6) {
            *v = 0.0;
        }
        let mut o = opts_default();
        o.test.start = 100;
        o.test.max = 100;
        let a = analyse(&t, &o, None).unwrap();
        assert_eq!(a.low_prevalence, ["g4"]);
        assert_eq!(a.kept, [0, 1, 2, 3]);
        let small = simulate(5, 4, 0.1, 0.0, 2);
        assert!(analyse(&small, &o, None).is_err());
    }
}
