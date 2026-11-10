//! isotable: one row per sample from the outputs of the usual QC, assembly and typing tools.
//!
//! Point it at a results folder; it finds fastp, leanqc, QUAST, BUSCO, CheckM2, GTDB-Tk,
//! Kraken 2, mlst and AMRFinderPlus outputs by name, joins them by sample, and grades each
//! sample against thresholds you can change.

mod parse;

use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};
use parse::{Record, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "One QC row per sample from fastp, leanqc, QUAST, BUSCO, CheckM2, GTDB-Tk, Kraken 2, mlst and AMRFinderPlus outputs")]
struct Cli {
    /// Folders to search (recursively) for tool outputs.
    #[arg(required = true)]
    dirs: Vec<PathBuf>,

    /// Sample sheet (CSV/TSV, e.g. from sheetgen) giving the real sample names to join on.
    #[arg(long)]
    sheet: Option<PathBuf>,

    /// Isolates (single-genome assemblies) or MAGs (metagenome-assembled genomes).
    #[arg(long, value_enum, default_value_t = Mode::Isolate)]
    mode: Mode,

    /// Output TSV (default: stdout).
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Also write a MultiQC custom-content table here.
    #[arg(long)]
    mqc: Option<PathBuf>,

    #[command(flatten)]
    limits: Limits,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Mode {
    Isolate,
    Mag,
}

/// Thresholds. Defaults are common conventions for bacterial isolates, not a standard;
/// change them to suit the organism and the study.
#[derive(clap::Args)]
struct Limits {
    /// Warn below this percentage of Q30 bases after trimming.
    #[arg(long, default_value_t = 80.0)]
    min_q30: f64,
    /// Warn above this many contigs; fail above twice it.
    #[arg(long, default_value_t = 300)]
    max_contigs: u64,
    /// Warn below this N50; fail below half of it.
    #[arg(long, default_value_t = 10_000)]
    min_n50: u64,
    /// Warn below this BUSCO complete %; fail 5 points lower.
    #[arg(long, default_value_t = 95.0)]
    min_busco: f64,
    /// Isolate mode: warn below this CheckM2 completeness; fail 5 points lower.
    #[arg(long, default_value_t = 95.0)]
    min_completeness: f64,
    /// Isolate mode: warn above this CheckM2 contamination; fail above 5.
    #[arg(long, default_value_t = 2.0)]
    max_contamination: f64,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Status {
    Pass,
    Warn,
    Fail,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Warn => "warn",
            Status::Fail => "fail",
        }
    }
}

/// Columns in output order: (key, header).
const COLUMNS: &[(&str, &str)] = &[
    ("reads_raw", "reads_raw"),
    ("reads_trimmed", "reads_trimmed"),
    ("q30_trimmed", "q30_trimmed"),
    ("duplication", "duplication"),
    ("leanqc_fails", "leanqc_fails"),
    ("kraken_top", "kraken_top_species"),
    ("kraken_top_pct", "kraken_top_pct"),
    ("contigs", "contigs"),
    ("length", "assembly_length"),
    ("n50", "n50"),
    ("gc", "gc_percent"),
    ("busco_complete", "busco_complete"),
    ("busco_lineage", "busco_lineage"),
    ("completeness", "checkm2_completeness"),
    ("contamination", "checkm2_contamination"),
    ("mimag", "mimag_tier"),
    ("gtdb", "gtdb_classification"),
    ("mlst_scheme", "mlst_scheme"),
    ("mlst_st", "mlst_st"),
    ("amr_genes", "amr_genes"),
];

fn main() -> Result<()> {
    let cli = Cli::parse();
    let sheet = match &cli.sheet {
        Some(p) => parse::sheet_samples(p)?,
        None => Vec::new(),
    };
    let (records, seen) = parse::scan(&cli.dirs)?;
    if records.is_empty() {
        bail!("no recognised tool outputs under the given folders");
    }

    // Join by sample.
    let mut rows: BTreeMap<String, BTreeMap<&'static str, Value>> = BTreeMap::new();
    let mut unmatched = BTreeSet::new();
    for r in records {
        let sample = parse::match_sample(&r.raw_name, &sheet);
        if !sheet.is_empty() && !sheet.contains(&sample) {
            unmatched.insert(r.raw_name.clone());
        }
        let row = rows.entry(sample).or_default();
        merge(row, r);
    }
    for s in &sheet {
        rows.entry(s.clone()).or_default();
    }

    if cli.mode == Mode::Mag {
        for row in rows.values_mut() {
            if let (Some(c), Some(x)) = (num(row, "completeness"), num(row, "contamination")) {
                row.insert("mimag", Value::Text(mimag(c, x).into()));
            }
        }
    }

    let present: Vec<_> = COLUMNS.iter().filter(|(k, _)| rows.values().any(|r| r.contains_key(k))).collect();
    let mut out = String::from("sample\tstatus\treasons");
    for (_, h) in &present {
        out.push('\t');
        out.push_str(h);
    }
    out.push('\n');
    let mut mqc = String::from("# id: 'isotable'\n# section_name: 'isotable'\n# plot_type: 'table'\nSample\tstatus");
    for (_, h) in &present {
        mqc.push('\t');
        mqc.push_str(h);
    }
    mqc.push('\n');

    let mut counts = [0usize; 3];
    for (sample, row) in &rows {
        let (status, reasons) = grade(row, &cli.limits, cli.mode);
        counts[status as usize] += 1;
        let cells: Vec<String> = present.iter().map(|(k, _)| row.get(k).map(Value::show).unwrap_or_default()).collect();
        out.push_str(&format!("{sample}\t{}\t{}\t{}\n", status.name(), reasons.join("; "), cells.join("\t")));
        mqc.push_str(&format!("{sample}\t{}\t{}\n", status.name(), cells.join("\t")));
    }

    match &cli.output {
        Some(p) => std::fs::write(p, &out)?,
        None => print!("{out}"),
    }
    if let Some(p) = &cli.mqc {
        std::fs::write(p, mqc)?;
    }
    eprintln!(
        "isotable: {} samples ({} pass, {} warn, {} fail) from {}",
        rows.len(),
        counts[0],
        counts[1],
        counts[2],
        seen.iter().map(|(k, n)| format!("{n} {k}")).collect::<Vec<_>>().join(", ")
    );
    for u in &unmatched {
        eprintln!("  warning: `{u}` matched no sample in the sheet; it got its own row");
    }
    Ok(())
}

fn merge(row: &mut BTreeMap<&'static str, Value>, r: Record) {
    for (k, v) in r.fields {
        match (row.get_mut(k), v) {
            // Several files per sample (R1/R2 QC reports): add counts, keep the worst quality.
            (Some(Value::Int(a)), Value::Int(b)) if k.starts_with("reads") => *a += b,
            (Some(Value::Num(a)), Value::Num(b)) if k == "q30_trimmed" => *a = a.min(b),
            (Some(Value::Text(a)), Value::Text(b)) if k == "leanqc_fails" => {
                let mut set: BTreeSet<String> = a.split(',').chain(b.split(',')).filter(|s| !s.is_empty()).map(String::from).collect();
                set.remove("");
                *a = set.into_iter().collect::<Vec<_>>().join(",");
            }
            (_, v) => {
                row.insert(k, v);
            }
        }
    }
}

fn num(row: &BTreeMap<&'static str, Value>, k: &str) -> Option<f64> {
    match row.get(k)? {
        Value::Num(x) => Some(*x),
        Value::Int(x) => Some(*x as f64),
        Value::Text(_) => None,
    }
}

/// MIMAG tier from completeness and contamination alone. The full "high-quality" standard also
/// needs the 23S, 16S and 5S rRNA genes and at least 18 tRNAs, which these tools don't report.
fn mimag(completeness: f64, contamination: f64) -> &'static str {
    if completeness > 90.0 && contamination < 5.0 {
        "high*"
    } else if completeness >= 50.0 && contamination < 10.0 {
        "medium"
    } else {
        "low"
    }
}

fn grade(row: &BTreeMap<&'static str, Value>, l: &Limits, mode: Mode) -> (Status, Vec<String>) {
    let mut worst = Status::Pass;
    let mut why = Vec::new();
    let mut flag = |s: Status, msg: String| {
        worst = worst.max(s);
        why.push(format!("{}: {msg}", s.name()));
    };
    if row.is_empty() {
        flag(Status::Fail, "no tool output found for this sample".into());
    }
    if let Some(q) = num(row, "q30_trimmed").filter(|q| *q < l.min_q30) {
        flag(Status::Warn, format!("Q30 {q:.1}% < {:.0}%", l.min_q30));
    }
    if let Some(Value::Text(f)) = row.get("leanqc_fails").filter(|v| !v.show().is_empty()) {
        flag(Status::Warn, format!("leanqc failed {f}"));
    }
    if let Some(c) = num(row, "contigs") {
        let c = c as u64;
        if c > l.max_contigs * 2 {
            flag(Status::Fail, format!("{c} contigs > {}", l.max_contigs * 2));
        } else if c > l.max_contigs {
            flag(Status::Warn, format!("{c} contigs > {}", l.max_contigs));
        }
    }
    if let Some(n) = num(row, "n50") {
        let n = n as u64;
        if n < l.min_n50 / 2 {
            flag(Status::Fail, format!("N50 {n} < {}", l.min_n50 / 2));
        } else if n < l.min_n50 {
            flag(Status::Warn, format!("N50 {n} < {}", l.min_n50));
        }
    }
    if let Some(b) = num(row, "busco_complete") {
        if b < l.min_busco - 5.0 {
            flag(Status::Fail, format!("BUSCO {b:.1}% < {:.0}%", l.min_busco - 5.0));
        } else if b < l.min_busco {
            flag(Status::Warn, format!("BUSCO {b:.1}% < {:.0}%", l.min_busco));
        }
    }
    let gtdb = row.get("gtdb").map(Value::show).unwrap_or_default();
    // Reduced-genome lineages: CheckM2 is far better than CheckM1 here but has still been shown to
    // underestimate complete, circular Patescibacteria genomes.
    let reduced = gtdb.contains("p__Patescibacteri");
    if let (Some(c), Some(x)) = (num(row, "completeness"), num(row, "contamination")) {
        match mode {
            Mode::Isolate => {
                if c < l.min_completeness - 5.0 {
                    flag(Status::Fail, format!("completeness {c:.1}% < {:.0}%", l.min_completeness - 5.0));
                } else if c < l.min_completeness {
                    flag(Status::Warn, format!("completeness {c:.1}% < {:.0}%", l.min_completeness));
                }
                if x > 5.0 {
                    flag(Status::Fail, format!("contamination {x:.1}% > 5%"));
                } else if x > l.max_contamination {
                    flag(Status::Warn, format!("contamination {x:.1}% > {:.0}%", l.max_contamination));
                }
            }
            Mode::Mag => match mimag(c, x) {
                "high*" => {}
                "medium" => flag(Status::Warn, format!("MIMAG medium ({c:.1}% complete, {x:.1}% contamination)")),
                _ => flag(Status::Fail, format!("MIMAG low ({c:.1}% complete, {x:.1}% contamination)")),
            },
        }
        if reduced && c < 90.0 {
            why.push("note: Patescibacteria have reduced genomes; CheckM2 can underestimate their completeness (see docs)".into());
        }
    }
    (worst, why)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mimag_tiers() {
        assert_eq!(mimag(95.0, 1.0), "high*");
        assert_eq!(mimag(90.0, 1.0), "medium"); // high needs > 90
        assert_eq!(mimag(60.0, 9.9), "medium");
        assert_eq!(mimag(60.0, 10.0), "low");
        assert_eq!(mimag(40.0, 1.0), "low");
    }
}
