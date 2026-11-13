//! seqplan: how many isolates fit on a sequencing run at a target coverage, before sequencing.
//!
//! Bases per isolate are genome size × coverage, padded by a barcode-imbalance margin so the
//! least-covered sample still reaches the target. A run's output comes from the vendor's
//! specification (Illumina) or reported yields (nanopore), times the fraction usable for samples
//! (PhiX spike-in, undetermined reads, trimming). Every kit figure and genome size carries its
//! source in `data/`, which is compiled in. No prices are built in: give `--run-price`.

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use serde_json::{Value, json};

const KITS: &str = include_str!("../data/kits.tsv");
const ORGANISMS: &str = include_str!("../data/organisms.tsv");

#[derive(Parser)]
#[command(version, about = "Plan a sequencing run: isolates per run and cost per isolate at a target coverage")]
struct Cli {
    /// Kit or flow cell (see --list-kits), e.g. miseq-v3, nextseq-p1, minion-r10.4.1.
    #[arg(short, long, required_unless_present_any = ["list_kits", "list_organisms"])]
    kit: Option<String>,

    /// Read configuration, e.g. 2x150. Default: the kit's first listed one (--list-kits).
    #[arg(long)]
    reads: Option<String>,

    /// Organism preset with NCBI's expected genome size (see --list-organisms).
    #[arg(short = 'g', long, conflicts_with = "genome_size")]
    organism: Option<String>,

    /// Genome size in bp, with an optional k/M/G suffix, e.g. 5.5M.
    #[arg(short = 's', long, required_unless_present_any = ["organism", "list_kits", "list_organisms"])]
    genome_size: Option<String>,

    /// Target mean coverage per isolate (×).
    #[arg(short, long, default_value_t = 50.0)]
    coverage: f64,

    /// Fraction of the run's output that ends up in sample reads (after PhiX, undetermined reads
    /// and trimming). Default: 0.8 for Illumina; 1.0 for nanopore, whose yields are reported ones.
    #[arg(long)]
    usable: Option<f64>,

    /// Barcode-imbalance margin: each isolate is given this many times its bases, so a sample
    /// that gets 1/margin of the average share still reaches the target coverage.
    #[arg(long, default_value_t = 1.3)]
    imbalance: f64,

    /// Number of isolates to sequence: also report runs needed and cost per isolate for them.
    #[arg(short = 'n', long)]
    samples: Option<u64>,

    /// Price of one run (flow cell plus reagents and library prep you want counted), in any
    /// currency. seqplan has no built-in prices.
    #[arg(long)]
    run_price: Option<f64>,

    /// Currency label for --run-price, e.g. USD or IDR.
    #[arg(long, default_value = "")]
    currency: String,

    /// Output format.
    #[arg(short, long, value_enum, default_value_t = Format::Text)]
    format: Format,

    /// List the kits with their output and source, then exit.
    #[arg(long)]
    list_kits: bool,

    /// List the organism presets with their genome size and source, then exit.
    #[arg(long)]
    list_organisms: bool,
}

#[derive(Clone, Copy, PartialEq, ValueEnum)]
enum Format {
    Text,
    Json,
}

#[derive(Clone, Debug)]
struct Kit {
    id: String,
    platform: String,
    name: String,
    reads: String,
    gb_low: f64,
    gb_high: f64,
    basis: String,
    source: String,
    note: String,
}

#[derive(Clone, Debug)]
struct Organism {
    id: String,
    taxid: String,
    name: String,
    expected: u64,
    minimum: u64,
    maximum: u64,
    source: String,
}

/// Data rows of a bundled TSV: `#` lines are comments, the first other line is the header.
fn rows(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).skip(1).map(|l| l.split('\t').collect())
}

fn kits() -> Vec<Kit> {
    rows(KITS)
        .map(|c| Kit {
            id: c[0].into(),
            platform: c[1].into(),
            name: c[2].into(),
            reads: c[3].into(),
            gb_low: c[4].parse().expect("kits.tsv gb_low"),
            gb_high: c[5].parse().expect("kits.tsv gb_high"),
            basis: c[6].into(),
            source: c[7].into(),
            note: c[8].into(),
        })
        .collect()
}

fn organisms() -> Vec<Organism> {
    rows(ORGANISMS)
        .map(|c| Organism {
            id: c[0].into(),
            taxid: c[1].into(),
            name: c[2].into(),
            expected: c[3].parse().expect("organisms.tsv expected"),
            minimum: c[4].parse().expect("organisms.tsv minimum"),
            maximum: c[5].parse().expect("organisms.tsv maximum"),
            source: c[6].into(),
        })
        .collect()
}

/// The kit row for `id` and read configuration (the kit's first row when none is given).
fn find_kit(id: &str, reads: Option<&str>) -> Result<Kit> {
    let all = kits();
    let of_kit: Vec<&Kit> = all.iter().filter(|k| k.id == id).collect();
    if of_kit.is_empty() {
        let mut ids: Vec<&str> = all.iter().map(|k| k.id.as_str()).collect();
        ids.dedup();
        bail!("unknown kit '{id}'; known: {}", ids.join(", "));
    }
    match reads {
        None => Ok(of_kit[0].clone()),
        Some(r) => {
            let r = r.to_ascii_lowercase().replace('×', "x");
            of_kit.iter().find(|k| k.reads == r).map(|k| (*k).clone()).with_context(|| {
                let offered: Vec<&str> = of_kit.iter().map(|k| k.reads.as_str()).collect();
                format!("{} has no {r} configuration; offered: {}", of_kit[0].name, offered.join(", "))
            })
        }
    }
}

/// `5.5M`, `5500000`, `5.5Mb`, `2800k` → bp.
fn parse_size(s: &str) -> Result<u64> {
    let t = s.trim().to_ascii_lowercase();
    let t = t.strip_suffix("bp").or_else(|| t.strip_suffix('b')).unwrap_or(&t);
    let (num, mult) = match t.chars().last() {
        Some('k') => (&t[..t.len() - 1], 1e3),
        Some('m') => (&t[..t.len() - 1], 1e6),
        Some('g') => (&t[..t.len() - 1], 1e9),
        _ => (t, 1.0),
    };
    let v: f64 = num.trim().parse().with_context(|| format!("genome size '{s}' is not a number like 5.5M"))?;
    if !(v.is_finite() && v > 0.0) {
        bail!("genome size '{s}' must be positive");
    }
    let bp = (v * mult).round();
    if bp < 1000.0 || bp > 2e10 {
        bail!("genome size '{s}' is {bp} bp; expected 1 kb to 20 Gb");
    }
    Ok(bp as u64)
}

/// Bases in one read (or read pair) of a configuration like `2x150`; None for `long`.
fn bases_per_fragment(reads: &str) -> Option<f64> {
    let (n, len) = reads.split_once('x')?;
    Some(n.parse::<f64>().ok()? * len.parse::<f64>().ok()?)
}

#[derive(Debug)]
struct Plan {
    genome_bp: u64,
    genome_from: String,
    kit: Kit,
    coverage: f64,
    usable: f64,
    imbalance: f64,
    samples: Option<u64>,
    run_price: Option<f64>,
    currency: String,
}

/// One end of the kit's output range.
struct Side {
    run_gb: f64,
    usable_gb: f64,
    isolates: u64,
    runs_for_samples: Option<u64>,
    cost_full_run: Option<f64>,
    cost_for_samples: Option<f64>,
}

impl Plan {
    fn bases_per_isolate(&self) -> f64 {
        self.genome_bp as f64 * self.coverage
    }

    fn padded_bases(&self) -> f64 {
        self.bases_per_isolate() * self.imbalance
    }

    fn side(&self, run_gb: f64) -> Side {
        let usable_bases = run_gb * 1e9 * self.usable;
        let isolates = (usable_bases / self.padded_bases()).floor() as u64;
        let runs_for_samples = self.samples.map(|n| {
            if isolates > 0 {
                n.div_ceil(isolates)
            } else {
                // One isolate needs more than a run: runs for all of them, bases pooled.
                ((n as f64 * self.padded_bases()) / usable_bases).ceil() as u64
            }
        });
        let cost_full_run = self.run_price.filter(|_| isolates > 0).map(|p| p / isolates as f64);
        let cost_for_samples = match (self.run_price, runs_for_samples, self.samples) {
            (Some(p), Some(r), Some(n)) if n > 0 => Some(p * r as f64 / n as f64),
            _ => None,
        };
        Side { run_gb, usable_gb: usable_bases / 1e9, isolates, runs_for_samples, cost_full_run, cost_for_samples }
    }

    fn json(&self) -> Value {
        let side = |s: Side| {
            json!({
                "run_gb": s.run_gb, "usable_gb": round(s.usable_gb, 3), "isolates_per_run": s.isolates,
                "runs_for_samples": s.runs_for_samples, "cost_per_isolate_full_run": s.cost_full_run.map(|c| round(c, 2)),
                "cost_per_isolate_for_samples": s.cost_for_samples.map(|c| round(c, 2)),
            })
        };
        let pairs = bases_per_fragment(&self.kit.reads).map(|b| (self.padded_bases() / b).ceil() as u64);
        json!({
            "genome_bp": self.genome_bp, "genome_size_from": self.genome_from, "coverage": self.coverage,
            "kit": {"id": self.kit.id, "name": self.kit.name, "platform": self.kit.platform, "reads": self.kit.reads,
                    "gb_low": self.kit.gb_low, "gb_high": self.kit.gb_high, "basis": self.kit.basis,
                    "source": self.kit.source, "note": self.kit.note},
            "usable_fraction": self.usable, "imbalance_margin": self.imbalance,
            "bases_per_isolate": self.bases_per_isolate().round() as u64,
            "bases_per_isolate_with_margin": self.padded_bases().round() as u64,
            "reads_per_isolate_with_margin": pairs,
            "samples": self.samples, "run_price": self.run_price, "currency": self.currency,
            "low": side(self.side(self.kit.gb_low)), "high": side(self.side(self.kit.gb_high)),
        })
    }

    fn text(&self) -> String {
        let k = &self.kit;
        let basis = match k.basis.as_str() {
            "spec" => "vendor specification",
            "max" => "vendor theoretical maximum",
            _ => "typical reported yield, not a specification",
        };
        let mut o = String::new();
        o += &format!("Genome     {} ({})\n", mb(self.genome_bp as f64), self.genome_from);
        o += &format!("Kit        {}, {} ({})\n", k.name, k.reads, k.id);
        o += &format!("Coverage   {}×\n\n", trim(self.coverage));
        let unit = if k.reads.contains('x') { format!("{} read pairs", k.reads) } else { "reads".into() };
        o += &format!("Per isolate     {} at {}×", mb(self.bases_per_isolate()), trim(self.coverage));
        o += &format!("; with the ×{} imbalance margin {}", trim(self.imbalance), mb(self.padded_bases()));
        match bases_per_fragment(&k.reads) {
            Some(b) => o += &format!(" = {} {unit}\n", millions((self.padded_bases() / b).ceil())),
            None => o += "\n",
        }
        let (lo, hi) = (self.side(k.gb_low), self.side(k.gb_high));
        let range = |a: String, b: String| if a == b { a } else { format!("{a} to {b}") };
        o += &format!("Run output      {} Gb ({basis})\n", range(trim(k.gb_low), trim(k.gb_high)));
        o += &format!("Usable          {} Gb (×{} for samples)\n", range(trim(round(lo.usable_gb, 2)), trim(round(hi.usable_gb, 2))), trim(self.usable));
        if hi.isolates == 0 {
            let per = self.padded_bases() / (k.gb_high * 1e9 * self.usable);
            o += &format!("Isolates/run    0: one isolate needs {} runs' worth of this kit\n", trim(round(per, 2)));
        } else {
            o += &format!("Isolates/run    {}\n", range(lo.isolates.to_string(), hi.isolates.to_string()));
        }
        let cur = if self.currency.is_empty() { String::new() } else { format!(" {}", self.currency) };
        if let (Some(a), Some(b)) = (lo.cost_full_run.or(hi.cost_full_run), hi.cost_full_run) {
            o += &format!("Cost/isolate    {}{cur} on a full run\n", range(money(b), money(a)));
        }
        if let Some(n) = self.samples {
            let (rl, rh) = (lo.runs_for_samples.unwrap_or(0), hi.runs_for_samples.unwrap_or(0));
            o += &format!("{:<16}{} run(s)", format!("{n} isolates"), range(rh.to_string(), rl.to_string()));
            if let (Some(a), Some(b)) = (hi.cost_for_samples, lo.cost_for_samples) {
                o += &format!(", {}{cur} per isolate", range(money(a), money(b)));
            }
            o += "\n";
        }
        o += &format!("\nSource: {}\n", k.source);
        if !k.note.is_empty() {
            o += &format!("Note: {}\n", k.note);
        }
        o
    }
}

fn round(v: f64, places: i32) -> f64 {
    let m = 10f64.powi(places);
    (v * m).round() / m
}

/// A number without trailing zeros: 50, 0.8, 13.2.
fn trim(v: f64) -> String {
    let s = format!("{:.3}", v);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn mb(bp: f64) -> String {
    if bp >= 1e9 { format!("{} Gb", trim(round(bp / 1e9, 2))) } else { format!("{} Mb", trim(round(bp / 1e6, 1))) }
}

fn millions(n: f64) -> String {
    if n >= 1e6 { format!("{} M", trim(round(n / 1e6, 2))) } else { format!("{} k", trim(round(n / 1e3, 1))) }
}

fn money(v: f64) -> String {
    if v >= 100.0 { format!("{:.0}", v) } else { format!("{:.2}", v) }
}

fn plan(cli: &Cli) -> Result<Plan> {
    let kit = find_kit(cli.kit.as_deref().context("--kit is required")?, cli.reads.as_deref())?;
    let (genome_bp, genome_from) = match (&cli.organism, &cli.genome_size) {
        (Some(id), _) => {
            let all = organisms();
            let o = all.iter().find(|o| o.id == *id || o.name.eq_ignore_ascii_case(id)).with_context(|| {
                let ids: Vec<&str> = all.iter().map(|o| o.id.as_str()).collect();
                format!("unknown organism '{id}'; presets: {} (or give --genome-size)", ids.join(", "))
            })?;
            (o.expected, format!("{}, NCBI expected size; range {}–{}", o.name, mb(o.minimum as f64), mb(o.maximum as f64)))
        }
        (None, Some(s)) => (parse_size(s)?, "given".into()),
        (None, None) => bail!("give --organism or --genome-size"),
    };
    if !(cli.coverage.is_finite() && cli.coverage > 0.0 && cli.coverage <= 10_000.0) {
        bail!("--coverage must be above 0 and at most 10000");
    }
    if !(cli.imbalance.is_finite() && (1.0..=10.0).contains(&cli.imbalance)) {
        bail!("--imbalance must be between 1 and 10");
    }
    let usable = cli.usable.unwrap_or(if kit.platform == "nanopore" { 1.0 } else { 0.8 });
    if !(usable.is_finite() && usable > 0.0 && usable <= 1.0) {
        bail!("--usable must be above 0 and at most 1");
    }
    if let Some(p) = cli.run_price
        && !(p.is_finite() && p >= 0.0)
    {
        bail!("--run-price must be a non-negative number");
    }
    if cli.samples == Some(0) {
        bail!("--samples must be at least 1");
    }
    Ok(Plan {
        genome_bp,
        genome_from,
        kit,
        coverage: cli.coverage,
        usable,
        imbalance: cli.imbalance,
        samples: cli.samples,
        run_price: cli.run_price,
        currency: cli.currency.clone(),
    })
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if cli.list_kits {
        let all = kits();
        if cli.format == Format::Json {
            let v: Vec<Value> = all
                .iter()
                .map(|k| json!({"id": k.id, "platform": k.platform, "name": k.name, "reads": k.reads, "gb_low": k.gb_low,
                                "gb_high": k.gb_high, "basis": k.basis, "source": k.source, "note": k.note}))
                .collect();
            println!("{}", serde_json::to_string_pretty(&v)?);
        } else {
            println!("kit\treads\tgb\tbasis\tname\tsource");
            for k in &all {
                let gb = if k.gb_low == k.gb_high { trim(k.gb_low) } else { format!("{}-{}", trim(k.gb_low), trim(k.gb_high)) };
                println!("{}\t{}\t{gb}\t{}\t{}\t{}", k.id, k.reads, k.basis, k.name, k.source);
            }
        }
        return Ok(());
    }
    if cli.list_organisms {
        let all = organisms();
        if cli.format == Format::Json {
            let v: Vec<Value> = all
                .iter()
                .map(|o| json!({"id": o.id, "taxid": o.taxid, "organism": o.name, "expected_bp": o.expected,
                                "minimum_bp": o.minimum, "maximum_bp": o.maximum, "source": o.source}))
                .collect();
            println!("{}", serde_json::to_string_pretty(&v)?);
        } else {
            println!("organism\texpected_bp\trange_bp\tname\tsource");
            for o in &all {
                println!("{}\t{}\t{}-{}\t{}\t{}", o.id, o.expected, o.minimum, o.maximum, o.name, o.source);
            }
        }
        return Ok(());
    }
    let p = plan(&cli)?;
    match cli.format {
        Format::Text => print!("{}", p.text()),
        Format::Json => println!("{}", serde_json::to_string_pretty(&p.json())?),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("seqplan").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn bundled_data_parses_and_every_row_has_a_source() {
        let k = kits();
        assert!(k.len() >= 30);
        for x in &k {
            assert!(x.source.starts_with("https://"), "{}", x.id);
            assert!(x.gb_low > 0.0 && x.gb_low <= x.gb_high, "{}", x.id);
            assert!(["spec", "max", "typical"].contains(&x.basis.as_str()), "{}", x.id);
            assert!(x.reads == "long" || bases_per_fragment(&x.reads).is_some(), "{}", x.id);
            assert_eq!(x.platform == "nanopore", x.basis == "typical", "{}: nanopore rows are typical yields", x.id);
        }
        let o = organisms();
        assert!(o.len() >= 10);
        for x in &o {
            assert!(x.minimum <= x.expected && x.expected <= x.maximum, "{}", x.id);
            assert!(x.source.ends_with(&x.taxid), "{}", x.id);
        }
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("5.5M").unwrap(), 5_500_000);
        assert_eq!(parse_size("5.5Mb").unwrap(), 5_500_000);
        assert_eq!(parse_size("2800k").unwrap(), 2_800_000);
        assert_eq!(parse_size("4215606").unwrap(), 4_215_606);
        assert_eq!(parse_size("3.1G").unwrap(), 3_100_000_000);
        for bad in ["", "big", "-5M", "0", "5MM", "NaN", "10"] {
            assert!(parse_size(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn kit_lookup() {
        assert_eq!(find_kit("miseq-v3", None).unwrap().reads, "2x300");
        assert_eq!(find_kit("miseq-v3", Some("2X75")).unwrap().gb_low, 3.3);
        assert_eq!(find_kit("nextseq-p1", None).unwrap().reads, "2x150");
        assert!(find_kit("miseq-v3", Some("2x150")).unwrap_err().to_string().contains("2x300, 2x75"));
        assert!(find_kit("hiseq", None).unwrap_err().to_string().contains("miseq-v3"));
    }

    #[test]
    fn kpn_on_miseq_v3_at_50x() {
        // 5.5 Mb × 50 = 275 Mb, ×1.3 = 357.5 Mb; 13.2 Gb × 0.8 = 10.56 Gb → 29; 15 Gb × 0.8 = 12 Gb → 33.
        let p = plan(&cli(&["-k", "miseq-v3", "-s", "5.5M", "-c", "50", "-n", "96", "--run-price", "1500"])).unwrap();
        let (lo, hi) = (p.side(p.kit.gb_low), p.side(p.kit.gb_high));
        assert_eq!((lo.isolates, hi.isolates), (29, 33));
        assert_eq!((lo.runs_for_samples, hi.runs_for_samples), (Some(4), Some(3)));
        assert!((lo.cost_full_run.unwrap() - 1500.0 / 29.0).abs() < 1e-9);
        assert!((hi.cost_for_samples.unwrap() - 1500.0 * 3.0 / 96.0).abs() < 1e-9);
        let j = p.json();
        assert_eq!(j["reads_per_isolate_with_margin"], 595_834); // 357.5 Mb / 600 bp per pair
        assert_eq!(j["low"]["isolates_per_run"], 29);
    }

    #[test]
    fn halving_coverage_doubles_isolates_roughly() {
        let a = plan(&cli(&["-k", "nextseq-p1", "-s", "5.5M", "-c", "50"])).unwrap();
        let b = plan(&cli(&["-k", "nextseq-p1", "-s", "5.5M", "-c", "25"])).unwrap();
        // 30 Gb × 0.8 = 24 Gb; 357.5 Mb → 67; 178.75 Mb → 134.
        assert_eq!(a.side(30.0).isolates, 67);
        assert_eq!(b.side(30.0).isolates, 134);
    }

    #[test]
    fn organism_preset_and_nanopore_defaults() {
        let p = plan(&cli(&["-k", "minion-r10.4.1", "-g", "klebsiella-pneumoniae", "-c", "40"])).unwrap();
        assert_eq!(p.genome_bp, 5_600_000);
        assert_eq!(p.usable, 1.0);
        // 5.6 Mb × 40 × 1.3 = 291.2 Mb; 5.2 Gb → 17, 7.0 Gb → 24.
        assert_eq!((p.side(5.2).isolates, p.side(7.0).isolates), (17, 24));
        assert!(p.text().contains("typical reported yield"));
        assert!(p.json()["reads_per_isolate_with_margin"].is_null());
        assert!(plan(&cli(&["-k", "miseq-v3", "-g", "Klebsiella pneumoniae"])).is_ok());
        assert!(plan(&cli(&["-k", "miseq-v3", "-g", "unicorn"])).unwrap_err().to_string().contains("--genome-size"));
    }

    #[test]
    fn more_than_a_run_per_isolate() {
        // 3.1 Gb × 30 × 1.3 = 120.9 Gb on a 0.96 Gb usable iSeq run.
        let p = plan(&cli(&["-k", "iseq-100", "-s", "3.1G", "-c", "30", "-n", "2", "--run-price", "1000"])).unwrap();
        let s = p.side(1.2);
        assert_eq!(s.isolates, 0);
        assert_eq!(s.runs_for_samples, Some(252)); // ceil(2 × 120.9 / 0.96)
        assert_eq!(s.cost_full_run, None);
        assert!(p.text().contains("runs' worth"));
    }

    #[test]
    fn rejects_bad_numbers() {
        for args in [
            &["-k", "miseq-v3", "-s", "5M", "-c", "0"][..],
            &["-k", "miseq-v3", "-s", "5M", "--imbalance", "0.5"],
            &["-k", "miseq-v3", "-s", "5M", "--usable", "1.5"],
            &["-k", "miseq-v3", "-s", "5M", "--run-price=-1"],
            &["-k", "miseq-v3", "-s", "5M", "-n", "0"],
        ] {
            assert!(plan(&cli(args)).is_err(), "{args:?}");
        }
    }
}
