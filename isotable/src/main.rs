//! isotable: one row per sample from the outputs of the usual QC, assembly and typing tools.
//!
//! Point it at a results folder; it finds fastp, leanqc, FastQC, QUAST, Shovill/Flye/Medaka
//! assemblies, BUSCO, CheckM2, GTDB-Tk, Kraken 2, CoverM, mlst, AMRFinderPlus and Kleborate outputs
//! by name, joins them by sample, and grades each sample against thresholds you can change. Two
//! files that disagree about one sample make a `conflict`; a sample missing a required measurement
//! is `incomplete`, never `pass`. `isotable assembly-stats` prints QUAST-style statistics of
//! assembly FASTA files.

use isotable::parse;

use anyhow::{Result, bail};
use clap::{Parser, ValueEnum};
use parse::{How, Kind, Record, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    version,
    about = "One QC row per sample from fastp, leanqc, FastQC, QUAST, assembly, BUSCO, CheckM2, GTDB-Tk, Kraken 2, CoverM, mlst, AMRFinderPlus and Kleborate outputs",
    args_conflicts_with_subcommands = true,
    subcommand_negates_reqs = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

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

    /// Measurements a sample needs before it can pass, comma-separated, from: reads, q30,
    /// contigs, n50, completeness (CheckM2 or BUSCO), contamination, kraken, gtdb, depth (CoverM),
    /// mlst, amr, kleborate.
    /// Default: reads,q30,contigs,n50,completeness (isolate); completeness,contamination (MAG).
    /// `--require none` turns the check off.
    #[arg(long, value_delimiter = ',')]
    require: Option<Vec<String>>,

    /// Failed FastQC modules (and the matching leanqc checks) that make a sample `warn`,
    /// comma-separated, by FastQC or leanqc name; `all` or `none`. Default: Per base sequence
    /// quality, Per sequence quality scores, Per base N content, Adapter Content. Every failed
    /// module is still listed in `fastqc_fails`.
    #[arg(long, value_delimiter = ',')]
    fastqc_grade: Option<Vec<String>>,

    #[command(flatten)]
    limits: Limits,
}

#[derive(clap::Subcommand)]
enum Command {
    /// QUAST-style statistics of assembly FASTA files (plain or gzip): contigs, total length,
    /// largest contig, N50, L50, N90, L90, GC %, Ns per 100 kbp, contigs >= 1 kb, and every contig
    /// regardless of length. One row per file.
    AssemblyStats(StatsArgs),
}

#[derive(clap::Args)]
struct StatsArgs {
    /// Assembly FASTA files.
    #[arg(required = true)]
    assemblies: Vec<PathBuf>,
    /// Leave out contigs shorter than this from every statistic except contigs_all and
    /// length_all. 500 is QUAST's default, so a QUAST report agrees.
    #[arg(long, default_value_t = isotable::asm::QUAST_MIN_CONTIG)]
    min_contig: u64,
    /// JSON (an array of objects) instead of TSV.
    #[arg(long)]
    json: bool,
    /// Output file (default: stdout).
    #[arg(short, long)]
    output: Option<PathBuf>,
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
    /// Fail below this many reads after trimming (fastp; all of a sample's runs together).
    #[arg(long, default_value_t = 100_000)]
    min_reads: u64,
    /// Warn below this percentage of Q30 bases after trimming.
    #[arg(long, default_value_t = 80.0)]
    min_q30: f64,
    /// Long reads: warn below this median read quality (leanqc). Q10 is the usual basecaller pass
    /// threshold; R10.4.1 hac/sup reads are typically Q15-25.
    #[arg(long, default_value_t = 10.0)]
    min_read_q: f64,
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
    /// Warn below this mean depth from read mapping (CoverM genome); fail below half of it.
    /// Shovill's own depth estimate is shown but not graded. 0 turns it off.
    #[arg(long, default_value_t = 20.0)]
    min_depth: f64,
}

/// Shovill's depth estimate and the mapped depth disagree when one is more than this times the other.
const DEPTH_DISAGREE: f64 = 1.5;

/// Worst wins: a conflict hides everything, then fail, then missing evidence, then warn.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Status {
    Pass,
    Warn,
    Incomplete,
    Fail,
    Conflict,
}

impl Status {
    fn name(self) -> &'static str {
        match self {
            Status::Pass => "pass",
            Status::Warn => "warn",
            Status::Incomplete => "incomplete",
            Status::Fail => "fail",
            Status::Conflict => "conflict",
        }
    }
}

/// Requirement tokens and the fields that satisfy them.
const REQUIREMENTS: &[(&str, &[&str], &str)] = &[
    ("reads", &["reads_trimmed", "reads_raw", "long_reads"], "read count (fastp, or leanqc for long reads)"),
    ("q30", &["q30_trimmed", "read_q"], "Q30 (fastp), or median read quality (leanqc, long reads)"),
    ("contigs", &["contigs"], "contig count, contigs of 500 bp or more (QUAST, or a Shovill, Flye or Medaka assembly)"),
    ("n50", &["n50"], "N50 of contigs of 500 bp or more (QUAST, or a Shovill, Flye or Medaka assembly)"),
    ("completeness", &["completeness", "busco_complete"], "completeness (CheckM2 or BUSCO)"),
    ("contamination", &["contamination"], "contamination (CheckM2)"),
    ("kraken", &["kraken_top"], "Kraken 2 report"),
    ("gtdb", &["gtdb"], "GTDB-Tk classification (or its ANI screen)"),
    ("depth", &["mapped_depth"], "mean depth from read mapping (CoverM genome)"),
    ("mlst", &["mlst_st"], "mlst result"),
    ("amr", &["amr_genes"], "AMRFinderPlus result"),
    ("kleborate", &["kleborate_species"], "Kleborate result"),
];

/// FastQC modules and the leanqc check that looks at the same thing (`None`: leanqc has none).
const MODULES: &[(&str, Option<&str>)] = &[
    ("Basic Statistics", None),
    ("Per base sequence quality", Some("Per-position quality")),
    ("Per tile sequence quality", Some("Per-tile quality")),
    ("Per sequence quality scores", Some("Per-read quality")),
    ("Per base sequence content", Some("Per-position base content")),
    ("Per sequence GC content", Some("Per-read GC")),
    ("Per base N content", Some("N content")),
    ("Sequence Length Distribution", Some("Read length")),
    ("Sequence Duplication Levels", Some("Duplication")),
    ("Overrepresented sequences", Some("Overrepresented sequences")),
    ("Adapter Content", Some("Adapter content")),
    ("Kmer Content", None),
];

/// Modules graded by default: the ones whose failure means poor reads. The others (base
/// composition, duplication, tiles, GC shape, k-mers...) routinely fail on healthy libraries
/// (random-hexamer bias in the first bases, high coverage of a small genome, a bad tile).
const DEFAULT_GRADE: &[&str] = &["Per base sequence quality", "Per sequence quality scores", "Per base N content", "Adapter Content"];

/// Which failed FastQC modules and leanqc checks count towards the grade (lowercased names).
struct Graded {
    all: bool,
    fastqc: BTreeSet<String>,
    leanqc: BTreeSet<String>,
}

impl Graded {
    fn fastqc(&self, module: &str) -> bool {
        self.all || self.fastqc.contains(&module.to_ascii_lowercase())
    }
    fn leanqc(&self, check: &str) -> bool {
        self.all || self.leanqc.contains(&check.to_ascii_lowercase())
    }
}

fn graded(given: Option<&[String]>) -> Result<Graded> {
    let defaults: Vec<String> = DEFAULT_GRADE.iter().map(|s| s.to_string()).collect();
    let given = given.unwrap_or(&defaults);
    let mut g = Graded { all: false, fastqc: BTreeSet::new(), leanqc: BTreeSet::new() };
    for t in given.iter().map(|t| t.trim()).filter(|t| !t.is_empty()) {
        match t.to_ascii_lowercase().as_str() {
            "all" => g.all = true,
            "none" => {}
            want => {
                let Some((f, l)) =
                    MODULES.iter().find(|(f, l)| f.eq_ignore_ascii_case(want) || l.is_some_and(|l| l.eq_ignore_ascii_case(want)))
                else {
                    bail!(
                        "--fastqc-grade {t}: unknown; use all, none, or FastQC module names: {}",
                        MODULES.iter().map(|(f, _)| *f).collect::<Vec<_>>().join(", ")
                    );
                };
                g.fastqc.insert(f.to_ascii_lowercase());
                if let Some(l) = l {
                    g.leanqc.insert(l.to_ascii_lowercase());
                }
            }
        }
    }
    Ok(g)
}

/// One field of one sample: a value and the files it came from, or disagreeing values.
#[derive(Debug, Clone)]
enum Cell {
    One { value: Value, sources: Vec<String> },
    Conflict(Vec<(String, Value)>),
}

impl Cell {
    fn value(&self) -> Option<&Value> {
        match self {
            Cell::One { value, .. } => Some(value),
            Cell::Conflict(_) => None,
        }
    }
    fn show(&self) -> String {
        match self {
            Cell::One { value, .. } => value.show(),
            Cell::Conflict(_) => "CONFLICT".into(),
        }
    }
}

/// Everything known about one sample.
#[derive(Default)]
struct Row {
    cells: BTreeMap<&'static str, Cell>,
    /// fastp reports, kept apart: identical copies count once, distinct runs are pooled.
    fastp: Vec<Record>,
    /// leanqc failed checks per read file; `[raw reads, trimmed reads]`.
    leanqc: [BTreeMap<String, String>; 2],
    /// leanqc's long-read numbers per read file; `[raw reads, trimmed reads]`.
    long: [BTreeMap<String, Vec<(&'static str, Value)>>; 2],
    /// FastQC reports per report (a zip and its sibling extracted folder are one key);
    /// `[raw reads, trimmed reads]`.
    fastqc: [BTreeMap<String, Vec<Record>>; 2],
    notes: Vec<String>,
}

/// Columns in output order: (key, header).
const COLUMNS: &[(&str, &str)] = &[
    ("reads_raw", "reads_raw"),
    ("reads_trimmed", "reads_trimmed"),
    ("q30_trimmed", "q30_trimmed"),
    ("duplication", "duplication"),
    ("long_reads", "long_reads"),
    ("long_bases", "long_read_bases"),
    ("read_n50", "read_n50"),
    ("read_q", "read_q_median"),
    ("leanqc_fails", "leanqc_fails"),
    ("leanqc_trimmed_fails", "leanqc_trimmed_fails"),
    ("fastqc_sequences", "fastqc_sequences"),
    ("fastqc_gc", "fastqc_gc"),
    ("fastqc_fails", "fastqc_fails"),
    ("fastqc_warns", "fastqc_warns"),
    ("fastqc_trimmed_sequences", "fastqc_trimmed_sequences"),
    ("fastqc_trimmed_gc", "fastqc_trimmed_gc"),
    ("fastqc_trimmed_fails", "fastqc_trimmed_fails"),
    ("fastqc_trimmed_warns", "fastqc_trimmed_warns"),
    ("kraken_top", "kraken_top_species"),
    ("kraken_top_pct", "kraken_top_pct"),
    ("contigs", "contigs"),
    ("length", "assembly_length"),
    ("n50", "n50"),
    ("gc", "gc_percent"),
    ("largest", "largest_contig"),
    ("l50", "l50"),
    ("n90", "n90"),
    ("ns_per_100kbp", "ns_per_100kbp"),
    ("contigs_1kb", "contigs_1kb"),
    ("contigs_all", "contigs_all"),
    ("shovill_depth", "shovill_depth_est"),
    ("mapped_depth", "mapped_depth"),
    ("covered_fraction", "covered_percent"),
    ("busco_complete", "busco_complete"),
    ("busco_lineage", "busco_lineage"),
    ("completeness", "checkm2_completeness"),
    ("contamination", "checkm2_contamination"),
    ("mimag", "mimag_tier"),
    ("gtdb", "gtdb_classification"),
    ("mlst_scheme", "mlst_scheme"),
    ("mlst_st", "mlst_st"),
    ("amr_genes", "amr_genes"),
    ("kleborate_species", "kleborate_species"),
    ("kleborate_species_match", "kleborate_species_match"),
    ("kleborate_st", "kleborate_st"),
    ("virulence_score", "virulence_score"),
    ("resistance_score", "resistance_score"),
    ("k_locus", "k_locus"),
    ("o_locus", "o_locus"),
    ("ybt", "ybt"),
    ("clb", "clb"),
    ("iuc", "iuc"),
    ("iro", "iro"),
    ("rmpadc", "rmpadc"),
];

/// `isotable assembly-stats`: one row (TSV) or object (JSON) per assembly, named as given.
fn assembly_stats(a: &StatsArgs) -> Result<()> {
    let mut rows = Vec::new();
    for p in &a.assemblies {
        let s = isotable::asm::stats(p, a.min_contig).map_err(|e| anyhow::anyhow!("{}: {e:#}", p.display()))?;
        rows.push((p.display().to_string(), s));
    }
    let text = if a.json {
        let list: Vec<serde_json::Value> = rows
            .iter()
            .map(|(name, s)| {
                let mut m = serde_json::Map::new();
                m.insert("assembly".into(), name.clone().into());
                for (k, v) in s.columns() {
                    let v = match (v.parse::<u64>(), v.parse::<f64>()) {
                        (Ok(n), _) => n.into(),
                        (_, Ok(x)) => x.into(),
                        _ => serde_json::Value::Null,
                    };
                    m.insert(k.into(), v);
                }
                serde_json::Value::Object(m)
            })
            .collect();
        serde_json::to_string_pretty(&list)? + "\n"
    } else {
        let mut out = String::from("assembly");
        // clap requires at least one assembly
        for (k, _) in rows[0].1.columns() {
            out.push('\t');
            out.push_str(k);
        }
        out.push('\n');
        for (name, s) in &rows {
            out.push_str(name);
            for (_, v) in s.columns() {
                out.push('\t');
                out.push_str(&v);
            }
            out.push('\n');
        }
        out
    };
    match &a.output {
        Some(p) => std::fs::write(p, text)?,
        None => print!("{text}"),
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(Command::AssemblyStats(a)) = &cli.command {
        return assembly_stats(a);
    }
    let sheet = match &cli.sheet {
        Some(p) => parse::sheet_samples(p)?,
        None => Vec::new(),
    };
    let required = requirements(cli.require.as_deref(), cli.mode)?;
    let grade_list = graded(cli.fastqc_grade.as_deref())?;
    let (records, seen, skipped) = parse::scan(&cli.dirs)?;
    if records.is_empty() {
        bail!("no recognised tool outputs under the given folders");
    }
    let (rows, joins, unmatched) = join(records, &sheet);
    let rows = finish(rows, cli.mode);

    let present: Vec<_> = COLUMNS.iter().filter(|(k, _)| rows.values().any(|r| r.cells.contains_key(k))).collect();
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

    let mut counts: BTreeMap<Status, usize> = BTreeMap::new();
    for (sample, row) in &rows {
        let (status, reasons) = grade(row, &cli.limits, cli.mode, &required, &grade_list);
        *counts.entry(status).or_default() += 1;
        let cells: Vec<String> = present.iter().map(|(k, _)| row.cells.get(k).map(Cell::show).unwrap_or_default()).collect();
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
    let tally: Vec<String> = counts.iter().map(|(s, n)| format!("{n} {}", s.name())).collect();
    eprintln!(
        "isotable: {} samples ({}) from {}",
        rows.len(),
        tally.join(", "),
        seen.iter().map(|(k, n)| format!("{n} {k}")).collect::<Vec<_>>().join(", ")
    );
    for (raw, sample) in &joins {
        eprintln!("  note: `{raw}` joined to sample {sample} (not an exact name match)");
    }
    for row in rows.values() {
        for n in &row.notes {
            eprintln!("  note: {n}");
        }
    }
    for s in &skipped {
        eprintln!("  skipped: {s}");
    }
    for u in &unmatched {
        eprintln!("  warning: `{u}` matched no sample in the sheet; it got its own row");
    }
    Ok(())
}

fn requirements(given: Option<&[String]>, mode: Mode) -> Result<Vec<&'static str>> {
    let defaults: &[&str] = match mode {
        Mode::Isolate => &["reads", "q30", "contigs", "n50", "completeness"],
        Mode::Mag => &["completeness", "contamination"],
    };
    let Some(given) = given else { return Ok(defaults.to_vec()) };
    let mut out = Vec::new();
    for g in given.iter().map(|g| g.trim()).filter(|g| !g.is_empty() && *g != "none") {
        match REQUIREMENTS.iter().find(|(k, _, _)| *k == g) {
            Some((k, _, _)) => out.push(*k),
            None => bail!(
                "--require {g}: unknown; choose from {}",
                REQUIREMENTS.iter().map(|(k, _, _)| *k).collect::<Vec<_>>().join(", ")
            ),
        }
    }
    Ok(out)
}

/// Joined rows; (raw name, sample) for every non-exact join; raw names that matched no sheet row.
type Joined = (BTreeMap<String, Row>, BTreeSet<(String, String)>, BTreeSet<String>);

/// Assign every record to a sample and collect its values, keeping track of where each came from.
fn join(records: Vec<Record>, sheet: &[String]) -> Joined {
    // Without a sheet, a read file's mate suffix goes only if the other mate's report is there
    // and no other tool uses the unstripped name as a sample.
    // Trimmed read files (`x_1.fastp.fastq.gz`, `x_1_val_1.fq.gz`) are matched under the name of
    // the read file they came from, and kept apart from the raw reads' evidence.
    let untrimmed = |r: &Record| if r.kind == Kind::ReadFile { parse::untrimmed(&r.raw_name) } else { None };
    let read_names: BTreeSet<String> = records
        .iter()
        .filter(|r| r.kind == Kind::ReadFile)
        .map(|r| untrimmed(r).unwrap_or_else(|| parse::base_name(&r.raw_name)))
        .collect();
    let other_names: BTreeSet<String> =
        records.iter().filter(|r| r.kind != Kind::ReadFile).map(|r| parse::base_name(&r.raw_name)).collect();
    let strip_mate = |raw: &str| {
        let base = parse::base_name(raw);
        !other_names.contains(&base) && parse::mate_parts(&base).is_some_and(|(_, other)| read_names.contains(&other))
    };

    let mut rows: BTreeMap<String, Row> = BTreeMap::new();
    let mut joins = BTreeSet::new();
    let mut unmatched = BTreeSet::new();
    for r in records {
        let trimmed = untrimmed(&r);
        let name = trimmed.clone().unwrap_or_else(|| r.raw_name.clone());
        let t = usize::from(trimmed.is_some());
        let (sample, how) = parse::match_sample(&name, r.kind, sheet, strip_mate(&name));
        match how {
            How::Exact => {}
            How::Unmatched => {
                unmatched.insert(r.raw_name.clone());
            }
            How::Prefix | How::Stripped => {
                joins.insert((r.raw_name.clone(), sample.clone()));
            }
        }
        let row = rows.entry(sample).or_default();
        match r.tool {
            "fastp" => row.fastp.push(r),
            "FastQC" => row.fastqc[t].entry(report_key(&r.source)).or_default().push(r),
            "leanqc" => {
                let mut long = Vec::new();
                for (k, v) in r.fields {
                    match k {
                        "leanqc_fails" => {
                            row.leanqc[t].insert(r.raw_name.clone(), v.show());
                        }
                        _ => long.push((k, v)),
                    }
                }
                if !long.is_empty() {
                    row.long[t].insert(r.raw_name.clone(), long);
                }
            }
            _ => {
                for (k, v) in r.fields {
                    put(&mut row.cells, k, v, &r.source);
                }
            }
        }
    }
    for s in sheet {
        rows.entry(s.clone()).or_default();
    }
    (rows, joins, unmatched)
}

/// A FastQC report's identity: `dir/x_fastqc.zip` and `dir/x_fastqc/fastqc_data.txt` are both
/// `dir/x_fastqc` (the zip and the folder `fastqc --extract` leaves beside it); reports anywhere
/// else are different reports, even when they name the same read file.
fn report_key(source: &str) -> String {
    let p = std::path::Path::new(source);
    let key = if p.file_name().is_some_and(|n| n == "fastqc_data.txt") { p.parent().unwrap_or(p).to_path_buf() } else { p.with_extension("") };
    key.display().to_string()
}

/// Set a single-valued field; a different value from another file makes it a conflict.
fn put(cells: &mut BTreeMap<&'static str, Cell>, k: &'static str, v: Value, source: &str) {
    match cells.get_mut(k) {
        None => {
            cells.insert(k, Cell::One { value: v, sources: vec![source.to_string()] });
        }
        Some(Cell::One { value, sources }) if *value == v => sources.push(source.to_string()),
        Some(Cell::One { value, sources }) => {
            let mut all: Vec<(String, Value)> = sources.iter().map(|s| (s.clone(), value.clone())).collect();
            all.push((source.to_string(), v));
            cells.insert(k, Cell::Conflict(all));
        }
        Some(Cell::Conflict(all)) => all.push((source.to_string(), v)),
    }
}

/// Pool fastp runs, fold in leanqc and FastQC, and add the MIMAG tier.
fn finish(mut rows: BTreeMap<String, Row>, mode: Mode) -> BTreeMap<String, Row> {
    for row in rows.values_mut() {
        pool_fastp(row);
        long_reads(row);
        for (t, key) in ["leanqc_fails", "leanqc_trimmed_fails"].into_iter().enumerate() {
            let files = &row.leanqc[t];
            if !files.is_empty() {
                let fails: BTreeSet<&str> = files.values().flat_map(|f| f.split(',')).filter(|s| !s.is_empty()).collect();
                let sources = files.keys().cloned().collect();
                let value = Value::Text(fails.into_iter().collect::<Vec<_>>().join(","));
                row.cells.insert(key, Cell::One { value, sources });
            }
        }
        merge_fastqc(row, false);
        merge_fastqc(row, true);
        if mode == Mode::Mag
            && let (Some(c), Some(x)) = (num(row, "completeness"), num(row, "contamination"))
        {
            row.cells.insert("mimag", Cell::One { value: Value::Text(mimag(c, x).into()), sources: Vec::new() });
        }
    }
    rows
}

/// A sample's long reads (leanqc): reads and bases add up over files; the read N50 is given for one
/// file only (it can't be pooled from per-file N50s); the median read quality of several files is
/// their reads-weighted mean, noted. Trimmed reads' numbers win, as for fastp. leanqc's short-read
/// checks are taken out of a long-read file's fails, with a note.
fn long_reads(row: &mut Row) {
    for t in 0..2 {
        for file in row.long[t].keys() {
            let Some(f) = row.leanqc[t].get_mut(file) else { continue };
            let (short, keep): (Vec<&str>, Vec<&str>) =
                f.split(',').filter(|c| !c.is_empty()).partition(|c| parse::SHORT_READ_CHECKS.contains(c));
            if !short.is_empty() {
                row.notes.push(format!("{file}: long reads, so leanqc's short-read checks are not graded (failed: {})", short.join(", ")));
                *f = keep.join(",");
            }
        }
    }
    let t = if row.long[1].is_empty() { 0 } else { 1 };
    let files = &row.long[t];
    if files.is_empty() {
        return;
    }
    let get = |fields: &[(&str, Value)], k: &str| fields.iter().find(|(n, _)| *n == k).map(|(_, v)| v.clone());
    let int = |v: Option<Value>| match v {
        Some(Value::Int(n)) => n,
        _ => 0,
    };
    let sources: Vec<String> = files.keys().cloned().collect();
    let one = |value: Value| Cell::One { value, sources: sources.clone() };
    let reads: u64 = files.values().map(|f| int(get(f, "long_reads"))).sum();
    let bases: u64 = files.values().map(|f| int(get(f, "long_bases"))).sum();
    let mut cells = vec![("long_reads", one(Value::Int(reads))), ("long_bases", one(Value::Int(bases)))];
    if files.len() == 1 {
        let f = files.values().next().unwrap();
        cells.extend(get(f, "read_n50").map(|v| ("read_n50", one(v))));
        cells.extend(get(f, "read_q").map(|v| ("read_q", one(v))));
    } else {
        let qs: Vec<(f64, f64)> = files
            .values()
            .filter_map(|f| match get(f, "read_q") {
                Some(Value::Num(q)) => Some((q, int(get(f, "long_reads")) as f64)),
                _ => None,
            })
            .collect();
        let n: f64 = qs.iter().map(|(_, r)| r).sum();
        if qs.len() == files.len() && n > 0.0 {
            let q = (qs.iter().map(|(q, r)| q * r).sum::<f64>() / n * 10.0).round() / 10.0;
            cells.push(("read_q", one(Value::Num(q))));
            row.notes.push(format!("{} long-read files: read_q_median is the reads-weighted mean of their medians; no pooled read N50", files.len()));
        }
    }
    for (k, c) in cells {
        row.cells.insert(k, c);
    }
}

/// The column a FastQC field goes to, for raw or trimmed reads.
fn fastqc_key(k: &'static str, trimmed: bool) -> &'static str {
    match (k, trimmed) {
        (_, false) => k,
        ("fastqc_sequences", true) => "fastqc_trimmed_sequences",
        ("fastqc_gc", true) => "fastqc_trimmed_gc",
        ("fastqc_fails", true) => "fastqc_trimmed_fails",
        ("fastqc_warns", true) => "fastqc_trimmed_warns",
        _ => k,
    }
}

/// FastQC reports of a sample's read files (R1 and R2, lanes): sequences add up, %GC is weighted
/// by sequences, failed and warned modules are merged. A zip and its sibling extracted folder are
/// one report and must agree; if they don't, the fields that differ are conflicts. Raw and
/// trimmed reads go to separate columns.
fn merge_fastqc(row: &mut Row, trimmed: bool) {
    let files = std::mem::take(&mut row.fastqc[usize::from(trimmed)]);
    if files.is_empty() {
        return;
    }
    let mut by_name: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (key, reports) in &files {
        if let Some(r) = reports.first() {
            by_name.entry(r.raw_name.as_str()).or_default().push(key.as_str());
        }
    }
    for (name, keys) in by_name.iter().filter(|(_, k)| k.len() > 1) {
        row.notes.push(format!("FastQC reports {} name the same read file ({name}) but are separate reports; counted as separate read files", keys.join(", ")));
    }
    let mut per_file: Vec<BTreeMap<&'static str, Value>> = Vec::new();
    let mut sources = Vec::new();
    for reports in files.values() {
        let mut cells = BTreeMap::new();
        for r in reports {
            sources.push(r.source.clone());
            for (k, v) in &r.fields {
                put(&mut cells, k, v.clone(), &r.source);
            }
        }
        let mut values = BTreeMap::new();
        for (k, c) in cells {
            match c {
                Cell::One { value, .. } => {
                    values.insert(k, value);
                }
                conflict => {
                    row.cells.insert(fastqc_key(k, trimmed), conflict);
                }
            }
        }
        per_file.push(values);
    }
    let int = |m: &BTreeMap<&str, Value>| match m.get("fastqc_sequences") {
        Some(Value::Int(n)) => Some(*n),
        _ => None,
    };
    let gc = |m: &BTreeMap<&str, Value>| match m.get("fastqc_gc") {
        Some(Value::Num(x)) => Some(*x),
        _ => None,
    };
    let mut out: Vec<(&'static str, Value)> = Vec::new();
    if let Some(total) = per_file.iter().map(int).sum::<Option<u64>>() {
        out.push(("fastqc_sequences", Value::Int(total)));
        let weighted: Option<f64> = per_file.iter().map(|m| Some(gc(m)? * int(m)? as f64)).sum();
        if let Some(w) = weighted.filter(|_| total > 0) {
            out.push(("fastqc_gc", Value::Num(w / total as f64)));
        }
    }
    for k in ["fastqc_fails", "fastqc_warns"] {
        let modules: BTreeSet<&str> = per_file
            .iter()
            .filter_map(|m| m.get(k))
            .flat_map(|v| match v {
                Value::Text(t) => t.split(',').filter(|s| !s.is_empty()).collect(),
                _ => Vec::new(),
            })
            .collect();
        out.push((k, Value::Text(modules.into_iter().collect::<Vec<_>>().join(","))));
    }
    for (k, v) in out {
        row.cells.entry(fastqc_key(k, trimmed)).or_insert(Cell::One { value: v, sources: sources.clone() });
    }
}

/// Two fastp reports describe the same raw reads. When both carry fastp's `before_filtering`
/// summary (reads, bases, Q20/Q30 bases, GC), it alone decides. Only when one lacks it do the
/// input file names in fastp's command plus the raw read count decide.
fn same_input(a: &Record, b: &Record) -> bool {
    let get = |r: &Record, k: &str| r.fields.iter().find(|(f, _)| *f == k).map(|(_, v)| v.clone());
    let summary = |r| get(r, "raw_summary");
    if let (Some(x), Some(y)) = (summary(a), summary(b)) {
        return x == y;
    }
    let inputs = |r| get(r, "raw_inputs");
    inputs(a).is_some() && inputs(a) == inputs(b) && get(a, "reads_raw").is_some() && get(a, "reads_raw") == get(b, "reads_raw")
}

/// Several fastp reports for one sample: identical copies count once; reports on different raw
/// reads are separate runs, so reads add up and Q30 is pooled as total Q30 bases over total bases.
/// Two reports on the same raw reads with different results (fastp rerun with other settings) are
/// not runs to add up: the sample's totals are computed with each of them in turn, and fields that
/// come out differently become conflicts naming those reports. Other runs are still pooled in.
/// When reports are linked only through a third one (A matches B and C on names and read count,
/// but B and C have different summaries), which are reruns is unclear: every fastp field becomes
/// a conflict listing each report's own numbers.
fn pool_fastp(row: &mut Row) {
    let mut runs: Vec<Record> = Vec::new();
    for r in std::mem::take(&mut row.fastp) {
        match runs.iter().find(|k| k.fields == r.fields) {
            Some(k) => row.notes.push(format!("{} has the same numbers as {}; counted once", r.source, k.source)),
            None => runs.push(r),
        }
    }
    if runs.is_empty() {
        return;
    }
    // Group reports on the same raw reads (transitively).
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for i in 0..runs.len() {
        let hits: Vec<usize> =
            (0..groups.len()).filter(|&g| groups[g].iter().any(|&j| same_input(&runs[i], &runs[j]))).collect();
        let mut merged = vec![i];
        for &g in hits.iter().rev() {
            merged.extend(groups.remove(g));
        }
        merged.sort();
        groups.push(merged);
    }
    groups.sort();
    // A group holds only if every pair in it passes `same_input`.
    let mut unclear = Vec::new();
    for g in &groups {
        let split: Vec<String> = g
            .iter()
            .enumerate()
            .flat_map(|(n, &i)| g[n + 1..].iter().map(move |&j| (i, j)))
            .filter(|&(i, j)| !same_input(&runs[i], &runs[j]))
            .map(|(i, j)| format!("{} and {}", runs[i].source, runs[j].source))
            .collect();
        if !split.is_empty() {
            let names: Vec<&str> = g.iter().map(|&i| runs[i].source.as_str()).collect();
            unclear.push(format!(
                "{} look like analyses of the same raw reads only through each other ({} differ), so which are reruns and which are separate runs is unclear",
                names.join(", "),
                split.join("; ")
            ));
        }
    }
    if !unclear.is_empty() {
        row.notes.extend(unclear);
        row.notes.push("fastp fields are shown per report; remove the reports that don't belong".into());
        let mut cells: BTreeMap<&'static str, Vec<(String, Value)>> = BTreeMap::new();
        for r in &runs {
            for (k, v) in pooled(&[r]).0 {
                cells.entry(k).or_default().push((r.source.clone(), v));
            }
        }
        for (k, all) in cells {
            row.cells.insert(k, Cell::Conflict(all));
        }
        return;
    }
    let reps: Vec<&Record> = groups.iter().map(|g| &runs[g[0]]).collect();
    let (base, note) = pooled(&reps);
    if let Some(n) = note {
        row.notes.push(n);
    }
    // For each rerun group, the totals with each of its reports standing in for the group.
    let mut variants: Vec<(usize, &str, BTreeMap<&'static str, Value>)> = Vec::new();
    for (gi, g) in groups.iter().enumerate().filter(|(_, g)| g.len() > 1) {
        let names: Vec<&str> = g.iter().map(|&i| runs[i].source.as_str()).collect();
        let others: Vec<&str> =
            groups.iter().enumerate().filter(|(o, _)| *o != gi).map(|(_, o)| runs[o[0]].source.as_str()).collect();
        let pooled_with = if others.is_empty() { String::new() } else { format!(" (pooled with the other runs: {})", others.join(", ")) };
        row.notes.push(format!(
            "{} analysed the same raw reads with different results; only one should count, so fields that depend on the choice show both{pooled_with}",
            names.join(" and ")
        ));
        for &i in g {
            let mut set = reps.clone();
            set[gi] = &runs[i];
            variants.push((gi, runs[i].source.as_str(), pooled(&set).0));
        }
    }
    let sources: Vec<String> = runs.iter().map(|r| r.source.clone()).collect();
    for (k, v) in base {
        let differing: BTreeSet<usize> =
            variants.iter().filter(|(_, _, m)| m.get(k) != Some(&v)).map(|(g, _, _)| *g).collect();
        if differing.is_empty() {
            row.cells.insert(k, Cell::One { value: v, sources: sources.clone() });
        } else {
            let all = variants
                .iter()
                .filter(|(g, _, _)| differing.contains(g))
                .filter_map(|(_, src, m)| Some((src.to_string(), m.get(k)?.clone())))
                .collect();
            row.cells.insert(k, Cell::Conflict(all));
        }
    }
}

/// Totals over separate runs: reads add up, Q30 is total Q30 bases over total bases, duplication
/// is weighted by reads. Returns a note when Q30 can't be pooled.
fn pooled(runs: &[&Record]) -> (BTreeMap<&'static str, Value>, Option<String>) {
    let get = |r: &Record, k: &str| r.fields.iter().find(|(f, _)| *f == k).map(|(_, v)| v.clone());
    let int = |r: &Record, k: &str| match get(r, k) {
        Some(Value::Int(n)) => Some(n),
        _ => None,
    };
    let fnum = |r: &Record, k: &str| match get(r, k) {
        Some(Value::Num(x)) => Some(x),
        _ => None,
    };
    let mut out = BTreeMap::new();
    let mut note = None;
    for k in ["reads_raw", "reads_trimmed"] {
        if let Some(sum) = runs.iter().map(|r| int(r, k)).sum::<Option<u64>>() {
            out.insert(k, Value::Int(sum));
        }
    }
    let q30: Option<u64> = runs.iter().map(|r| int(r, "q30_bases")).sum();
    let bases: Option<u64> = runs.iter().map(|r| int(r, "bases_trimmed")).sum();
    match (q30, bases) {
        (Some(q), Some(b)) if b > 0 => {
            out.insert("q30_trimmed", Value::Num(q as f64 * 100.0 / b as f64));
        }
        _ if runs.len() == 1 => {
            if let Some(q) = fnum(runs[0], "q30_trimmed") {
                out.insert("q30_trimmed", Value::Num(q));
            }
        }
        // Older fastp without base counts: no honest way to pool the rates.
        _ => note = Some("several fastp reports without q30_bases/total_bases; Q30 left out".into()),
    }
    let weights: Option<Vec<(f64, f64)>> =
        runs.iter().map(|r| Some((fnum(r, "duplication")?, int(r, "reads_raw")? as f64))).collect();
    if let Some(w) = weights {
        let total: f64 = w.iter().map(|(_, n)| n).sum();
        if total > 0.0 {
            out.insert("duplication", Value::Num(w.iter().map(|(d, n)| d * n).sum::<f64>() / total));
        }
    }
    (out, note)
}

fn num(row: &Row, k: &str) -> Option<f64> {
    match row.cells.get(k)?.value()? {
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

fn grade(row: &Row, l: &Limits, mode: Mode, required: &[&str], graded: &Graded) -> (Status, Vec<String>) {
    let mut worst = Status::Pass;
    let mut why = Vec::new();
    let mut flag = |s: Status, msg: String| {
        worst = worst.max(s);
        why.push(format!("{}: {msg}", s.name()));
    };
    if row.cells.is_empty() {
        flag(Status::Fail, "no tool output found for this sample".into());
    }
    for (k, cell) in &row.cells {
        if let Cell::Conflict(all) = cell {
            let list: Vec<String> = all.iter().map(|(src, v)| format!("{} in {src}", v.show())).collect();
            flag(Status::Conflict, format!("{k} differs between files: {}", list.join(" vs ")));
        }
    }
    for req in required {
        let (_, fields, label) = REQUIREMENTS.iter().find(|(k, _, _)| k == req).unwrap();
        if !fields.iter().any(|f| row.cells.contains_key(f)) {
            flag(Status::Incomplete, format!("no {label}"));
        }
    }
    if let Some(n) = num(row, "reads_trimmed").or_else(|| num(row, "reads_raw")) {
        let n = n as u64;
        if n < l.min_reads {
            flag(Status::Fail, format!("{n} reads < {}", l.min_reads));
        }
    }
    if let Some(q) = num(row, "q30_trimmed").filter(|q| *q < l.min_q30) {
        flag(Status::Warn, format!("Q30 {q:.1}% < {:.0}%", l.min_q30));
    }
    if let Some(q) = num(row, "read_q").filter(|q| *q < l.min_read_q) {
        flag(Status::Warn, format!("median read quality Q{q:.1} < Q{:.0}", l.min_read_q));
    }
    // Read QC: only the graded modules count, from the trimmed reads' reports when there are
    // any (those reads went on to assembly), else from the raw reads'. FastQC is separate evidence
    // from leanqc, shown as its own reason; the status is the worst either way, so a problem both
    // tools see still makes one `warn`.
    for (tool, raw, trimmed) in [("leanqc", "leanqc_fails", "leanqc_trimmed_fails"), ("FastQC", "fastqc_fails", "fastqc_trimmed_fails")] {
        let counts = |m: &str| if tool == "leanqc" { graded.leanqc(m) } else { graded.fastqc(m) };
        let (key, what) = if row.cells.contains_key(trimmed) { (trimmed, " (trimmed reads)") } else { (raw, "") };
        let Some(Cell::One { value: Value::Text(f), .. }) = row.cells.get(key) else { continue };
        let fails: Vec<&str> = f.split(',').filter(|m| !m.is_empty() && counts(m)).collect();
        if !fails.is_empty() {
            flag(Status::Warn, format!("{tool}{what} failed {}", fails.join(",")));
        }
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
    if let Some(d) = num(row, "mapped_depth").filter(|_| l.min_depth > 0.0) {
        if d < l.min_depth / 2.0 {
            flag(Status::Fail, format!("mapped depth {d:.1}x < {:.0}x", l.min_depth / 2.0));
        } else if d < l.min_depth {
            flag(Status::Warn, format!("mapped depth {d:.1}x < {:.0}x", l.min_depth));
        }
    }
    if let (Some(est), Some(d)) = (num(row, "shovill_depth"), num(row, "mapped_depth"))
        && est > 0.0
        && d > 0.0
        && est.max(d) / est.min(d) > DEPTH_DISAGREE
    {
        flag(
            Status::Warn,
            format!(
                "Shovill's depth estimate {est:.0}x and the mapped depth {d:.1}x differ by more than {DEPTH_DISAGREE}x; Shovill divides by its own genome-size estimate unless given gsize, so report the mapped depth"
            ),
        );
    }
    // Kleborate 3 writes an assembly whose species its preset doesn't cover (a kpsc run on E. coli)
    // to that species' table, with the preset's typing columns `Not Tested`.
    if row.cells.get("kleborate_st").map(Cell::show).as_deref() == Some("Not Tested") {
        let sp = row.cells.get("kleborate_species").map(Cell::show).unwrap_or_default();
        flag(Status::Warn, format!("Kleborate's species call is {sp}, which the preset it ran doesn't type, so its typing was not run (Not Tested)"));
    }
    if let Some(b) = num(row, "busco_complete") {
        if b < l.min_busco - 5.0 {
            flag(Status::Fail, format!("BUSCO {b:.1}% < {:.0}%", l.min_busco - 5.0));
        } else if b < l.min_busco {
            flag(Status::Warn, format!("BUSCO {b:.1}% < {:.0}%", l.min_busco));
        }
    }
    let gtdb = row.cells.get("gtdb").map(Cell::show).unwrap_or_default();
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
    use std::path::Path;

    fn write(root: &Path, name: &str, body: &str) {
        let p = root.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn fastp(reads: u64, q30_bases: u64, bases: u64) -> String {
        format!(
            r#"{{"summary":{{"before_filtering":{{"total_reads":{reads}}},"after_filtering":{{"total_reads":{reads},"total_bases":{bases},"q30_bases":{q30_bases},"q30_rate":{}}}}},"duplication":{{"rate":0.01}}}}"#,
            q30_bases as f64 / bases as f64
        )
    }

    const QUAST: &str = "Assembly\t# contigs\tTotal length\tN50\tGC (%)\npatient_1\t40\t2800000\t120000\t50.1\npatient_2\t900\t1500000\t2000\t38.0\n";

    fn limits() -> Limits {
        Limits {
            min_reads: 100_000,
            min_q30: 80.0,
            min_read_q: 10.0,
            max_contigs: 300,
            min_n50: 10_000,
            min_busco: 95.0,
            min_completeness: 95.0,
            max_contamination: 2.0,
            min_depth: 20.0,
        }
    }

    fn table(root: &Path, sheet: &[&str], mode: Mode, require: Option<&[&str]>) -> BTreeMap<String, (Status, Vec<String>, Row)> {
        table_graded(root, sheet, mode, require, None)
    }

    fn table_graded(
        root: &Path,
        sheet: &[&str],
        mode: Mode,
        require: Option<&[&str]>,
        grade_list: Option<&[&str]>,
    ) -> BTreeMap<String, (Status, Vec<String>, Row)> {
        let sheet: Vec<String> = sheet.iter().map(|s| s.to_string()).collect();
        let require: Option<Vec<String>> = require.map(|r| r.iter().map(|s| s.to_string()).collect());
        let required = requirements(require.as_deref(), mode).unwrap();
        let grade_list: Option<Vec<String>> = grade_list.map(|g| g.iter().map(|s| s.to_string()).collect());
        let graded = graded(grade_list.as_deref()).unwrap();
        let (records, _, _) = parse::scan(&[root.to_path_buf()]).unwrap();
        let (rows, _, _) = join(records, &sheet);
        finish(rows, mode)
            .into_iter()
            .map(|(k, row)| {
                let (s, why) = grade(&row, &limits(), mode, &required, &graded);
                (k, (s, why, row))
            })
            .collect()
    }

    fn cell(row: &Row, k: &str) -> String {
        row.cells.get(k).map(Cell::show).unwrap_or_default()
    }

    #[test]
    fn numbered_samples_stay_apart_without_a_sheet() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "a/patient_1.fastp.json", &fastp(1_000_000, 95, 100));
        write(t.path(), "a/patient_2.fastp.json", &fastp(10, 50, 100));
        write(t.path(), "b/transposed_report.tsv", QUAST);
        let rows = table(t.path(), &[], Mode::Isolate, None);
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["patient_1", "patient_2"]);
        let (_, _, p1) = &rows["patient_1"];
        assert_eq!((cell(p1, "reads_trimmed"), cell(p1, "n50"), cell(p1, "q30_trimmed")), ("1000000".into(), "120000".into(), "95".into()));
        let (s2, _, p2) = &rows["patient_2"];
        assert_eq!((cell(p2, "reads_trimmed"), cell(p2, "n50")), ("10".into(), "2000".into()));
        assert_eq!(*s2, Status::Fail);
    }

    #[test]
    fn sheet_prefix_needs_a_known_suffix() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "a/patient_1.fastp.json", &fastp(1_000_000, 95, 100));
        write(t.path(), "b/transposed_report.tsv", QUAST);
        let sheet = ["patient"];
        let records = parse::scan(&[t.path().to_path_buf()]).unwrap().0;
        let (rows, _, unmatched) = join(records, &sheet.map(String::from));
        assert!(rows["patient"].cells.is_empty() && rows["patient"].fastp.is_empty());
        assert_eq!(unmatched.into_iter().collect::<Vec<_>>(), ["patient_1", "patient_2"]);
        let rows = table(t.path(), &sheet, Mode::Isolate, None);
        assert_eq!(rows["patient"].0, Status::Fail);
    }

    #[test]
    fn disagreeing_files_make_a_conflict() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "run1/transposed_report.tsv", "Assembly\t# contigs\tN50\nS\t40\t120000\n");
        write(t.path(), "run2/transposed_report.tsv", "Assembly\t# contigs\tN50\nS\t40\t2000\n");
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["none"]));
        let (status, why, row) = &rows["S"];
        assert_eq!(*status, Status::Conflict);
        assert_eq!(cell(row, "n50"), "CONFLICT");
        assert_eq!(cell(row, "contigs"), "40"); // same value from both files is fine
        assert!(why.iter().any(|w| w.contains("n50 differs") && w.contains("run2")), "{why:?}");
    }

    #[test]
    fn duplicate_fastp_counts_once_and_runs_pool() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "a/S.fastp.json", &fastp(500_000, 90, 100));
        write(t.path(), "copy/S.fastp.json", &fastp(500_000, 90, 100));
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["none"]));
        assert_eq!(cell(&rows["S"].2, "reads_trimmed"), "500000");

        write(t.path(), "lane2/S.fastp.json", &fastp(500_000, 60, 300));
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["none"]));
        let row = &rows["S"].2;
        assert_eq!(cell(row, "reads_trimmed"), "1000000");
        assert_eq!(cell(row, "q30_trimmed"), "37.5"); // (90 + 60) / (100 + 300), not the mean of rates
    }

    /// A fastp report as fastp writes it: full before_filtering summary and the command line.
    fn fastp_full(raw_reads: u64, raw_bases: u64, trimmed: u64, in1: &str) -> String {
        format!(
            r#"{{"summary":{{"before_filtering":{{"total_reads":{raw_reads},"total_bases":{raw_bases},"q30_bases":{}}},
            "after_filtering":{{"total_reads":{trimmed},"total_bases":{},"q30_bases":{},"q30_rate":0.97}}}},
            "duplication":{{"rate":0.01}},
            "command":"fastp --in1 {in1}_R1.fastq.gz --in2 {in1}_R2.fastq.gz --json x.fastp.json"}}"#,
            raw_bases * 9 / 10,
            trimmed * 75,
            trimmed * 75 * 97 / 100
        )
    }

    #[test]
    fn two_analyses_of_the_same_reads_conflict() {
        // bacass ERR064912 reanalysed with stricter trimming in a sibling folder.
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "a/ERR064912.fastp.json", &fastp_full(2_000_000, 150_000_000, 1_988_356, "ERR064912"));
        write(t.path(), "b/ERR064912.fastp.json", &fastp_full(2_000_000, 150_000_000, 1_938_356, "ERR064912"));
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["reads"]));
        let (status, why, row) = &rows["ERR064912"];
        assert_eq!(*status, Status::Conflict, "{why:?}");
        assert_eq!(cell(row, "reads_raw"), "2000000"); // not 4,000,000
        assert_eq!(cell(row, "reads_trimmed"), "CONFLICT");
        assert!(why.iter().any(|w| w.contains("reads_trimmed differs")), "{why:?}");
        assert!(row.notes.iter().any(|n| n.contains("same raw reads")), "{:?}", row.notes);
        assert!(!row.cells.contains_key("raw_summary") && !row.cells.contains_key("raw_inputs"));

        // Same input names (nf-core renames inputs after the sample) but different raw reads:
        // separate runs, pooled.
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "run1/S.fastp.json", &fastp_full(2_000_000, 150_000_000, 1_900_000, "S"));
        write(t.path(), "run2/S.fastp.json", &fastp_full(1_000_000, 75_000_000, 950_000, "S"));
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["reads"]));
        let (status, why, row) = &rows["S"];
        assert_eq!(*status, Status::Pass, "{why:?}");
        assert_eq!(cell(row, "reads_raw"), "3000000");
        assert_eq!(cell(row, "reads_trimmed"), "2850000");
    }

    /// fastp report with nf-core's staged input names (`S_1.fastq.gz`, `S_2.fastq.gz`).
    fn fastp_staged(raw_reads: u64, raw_bases: u64, trimmed: u64) -> String {
        fastp_full(raw_reads, raw_bases, trimmed, "S").replace("S_R1", "S_1").replace("S_R2", "S_2")
    }

    #[test]
    fn distinct_runs_with_the_same_names_and_read_count_pool() {
        // Two sequencing runs of S, staged under the same names, 1,000,000 raw reads each, but
        // different raw bases and Q30: fastp's before_filtering summary tells them apart.
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "run1/S.fastp.json", &fastp_staged(1_000_000, 150_000_000, 980_000));
        write(t.path(), "run2/S.fastp.json", &fastp_staged(1_000_000, 148_000_000, 970_000));
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["reads"]));
        let (status, why, row) = &rows["S"];
        assert_eq!(*status, Status::Pass, "{why:?}");
        assert_eq!(cell(row, "reads_raw"), "2000000");
        assert_eq!(cell(row, "reads_trimmed"), "1950000");
        assert!(!row.notes.iter().any(|n| n.contains("same raw reads")), "{:?}", row.notes);

        // A rerun of run1 next to them: run2 is still pooled in, only the choice between the two
        // run1 reports is a conflict.
        write(t.path(), "rerun1/S.fastp.json", &fastp_staged(1_000_000, 150_000_000, 900_000));
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["reads"]));
        let (status, why, row) = &rows["S"];
        assert_eq!(*status, Status::Conflict, "{why:?}");
        assert_eq!(cell(row, "reads_raw"), "2000000"); // not 3,000,000
        let Cell::Conflict(all) = &row.cells["reads_trimmed"] else { panic!("{:?}", row.cells["reads_trimmed"]) };
        let mut got: Vec<(String, String)> = all.iter().map(|(s, v)| (s.clone(), v.show())).collect();
        got.sort();
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got[0].0.contains("rerun1") && got[0].1 == "1870000", "{got:?}");
        assert!(got[1].0.contains("run1") && got[1].1 == "1950000", "{got:?}");
        let note = row.notes.iter().find(|n| n.contains("same raw reads")).unwrap();
        assert!(note.contains("run1") && note.contains("rerun1") && note.contains("pooled with the other runs") && note.contains("run2"), "{note}");
    }

    #[test]
    fn same_input_names_and_raw_count_conflict_without_a_full_summary() {
        let mut a = Vec::new();
        let rec = |trimmed: u64, src: &str| Record {
            raw_name: "S".into(),
            kind: parse::Kind::Sample,
            tool: "fastp",
            source: src.into(),
            fields: vec![
                ("reads_raw", Value::Int(100)),
                ("reads_trimmed", Value::Int(trimmed)),
                ("raw_inputs", Value::Text("S_R1.fq.gz S_R2.fq.gz".into())),
            ],
        };
        a.push(rec(90, "x"));
        a.push(rec(80, "y"));
        let mut row = Row { fastp: a, ..Default::default() };
        pool_fastp(&mut row);
        assert!(matches!(row.cells["reads_trimmed"], Cell::Conflict(_)));
        assert_eq!(row.cells["reads_raw"].show(), "100");
    }

    #[test]
    fn reports_linked_only_through_a_third_are_not_grouped() {
        // A has no before_filtering summary and matches B and C on input names and raw count;
        // B and C carry different summaries, so they are not reruns of each other.
        let rec = |src: &str, trimmed: u64, summary: Option<&str>| {
            let mut fields = vec![
                ("reads_raw", Value::Int(100)),
                ("reads_trimmed", Value::Int(trimmed)),
                ("raw_inputs", Value::Text("S_1.fq.gz S_2.fq.gz".into())),
            ];
            if let Some(s) = summary {
                fields.push(("raw_summary", Value::Text(s.into())));
            }
            Record { raw_name: "S".into(), kind: parse::Kind::Sample, tool: "fastp", source: src.into(), fields }
        };
        let make = |name: &str| match name {
            "A" => rec("A", 90, None),
            "B" => rec("B", 80, Some("100 15000 13500 0.5")),
            _ => rec("C", 70, Some("100 14800 13000 0.5")),
        };
        let (a, b, c) = (make("A"), make("B"), make("C"));
        assert!(same_input(&a, &b) && same_input(&a, &c) && !same_input(&b, &c));
        for order in [["A", "B", "C"], ["B", "C", "A"], ["B", "A", "C"]] {
            let mut row = Row { fastp: order.iter().map(|n| make(n)).collect(), ..Default::default() };
            pool_fastp(&mut row);
            for k in ["reads_raw", "reads_trimmed"] {
                let Cell::Conflict(all) = &row.cells[k] else { panic!("{k}: {:?}", row.cells[k]) };
                assert_eq!(all.len(), 3, "{all:?}");
            }
            let Cell::Conflict(all) = &row.cells["reads_trimmed"] else { unreachable!() };
            let mut got: Vec<_> = all.iter().map(|(s, v)| (s.as_str(), v.show())).collect();
            got.sort();
            assert_eq!(got, [("A", "90".into()), ("B", "80".into()), ("C", "70".into())]);
            let note = row.notes.iter().find(|n| n.contains("only through each other")).expect("note");
            assert!(note.contains("B and C differ"), "{note}");
        }
        // Graded as a conflict.
        let mut row = Row { fastp: vec![a, b, c], ..Default::default() };
        pool_fastp(&mut row);
        assert_eq!(grade(&row, &limits(), Mode::Isolate, &["reads"], &graded(None).unwrap()).0, Status::Conflict);
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlinks_are_skipped() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "S.fastp.json", &fastp(500_000, 90, 100));
        std::os::unix::fs::symlink(t.path().join("gone.json"), t.path().join("T.fastp.json")).unwrap();
        let (records, _, skipped) = parse::scan(&[t.path().to_path_buf()]).unwrap();
        assert_eq!(records.len(), 1);
        assert!(skipped.iter().any(|s| s.contains("T.fastp.json")), "{skipped:?}");
    }

    #[test]
    fn missing_evidence_is_incomplete_and_few_reads_fail() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "x.fastp.json", r#"{"summary":{"before_filtering":{"total_reads":5},"after_filtering":{"total_reads":5,"q30_rate":0.99}},"duplication":{"rate":0.01}}"#);
        let rows = table(t.path(), &[], Mode::Isolate, None);
        let (status, why, _) = &rows["x"];
        assert_eq!(*status, Status::Fail, "{why:?}");
        assert!(why.iter().any(|w| w.starts_with("incomplete: no N50")), "{why:?}");
        assert!(why.iter().any(|w| w.contains("5 reads < 100000")), "{why:?}");

        let t = tempfile::tempdir().unwrap();
        write(t.path(), "x.fastp.json", &fastp(1_000_000, 95, 100));
        let rows = table(t.path(), &[], Mode::Isolate, None);
        assert_eq!(rows["x"].0, Status::Incomplete);
        assert_eq!(table(t.path(), &[], Mode::Isolate, Some(&["reads", "q30"]))["x"].0, Status::Pass);
    }

    fn long_lq(file: &str, reads: u64, q: f64, fails: &[&str]) -> String {
        let checks: Vec<String> = fails.iter().map(|c| format!(r#"{{"name":"{c}","status":"fail"}}"#)).collect();
        format!(
            r#"{{"file":"{file}","checks":[{}],"long_reads":{{"reads":{reads},"bases":{},"n50":7826,"read_quality":{{"median":{q}}}}}}}"#,
            checks.join(","),
            reads * 4000
        )
    }

    /// Issue #21: a nanopore-only isolate has leanqc's long-read numbers instead of fastp's.
    #[test]
    fn long_reads_stand_in_for_fastp() {
        let t = tempfile::tempdir().unwrap();
        // The nanopore run's sample: clean reads that fail three short-read checks.
        let fails = ["Per-position quality", "Per-position base content", "Per-read GC", "Adapter content"];
        write(t.path(), "ont_leanqc.json", &long_lq("ont.fastq.gz", 84759, 22.3, &fails));
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["reads", "q30"]));
        let (status, why, row) = &rows["ont"];
        assert_eq!(*status, Status::Warn, "{why:?}");
        // Only the check that fits long reads is graded; the others are in a note.
        assert_eq!(why, &["warn: leanqc failed Adapter content"]);
        assert_eq!(cell(row, "leanqc_fails"), "Adapter content");
        assert!(row.notes.iter().any(|n| n.contains("not graded (failed: Per-position quality, Per-position base content, Per-read GC)")), "{:?}", row.notes);
        assert_eq!((cell(row, "long_reads"), cell(row, "read_n50"), cell(row, "read_q")), ("84759".into(), "7826".into(), "22.3".into()));

        // Low read quality warns; the fastp read minimum doesn't apply to long reads.
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "ont_leanqc.json", &long_lq("ont.fastq.gz", 5000, 8.4, &[]));
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["reads", "q30"]));
        assert_eq!(rows["ont"].1, ["warn: median read quality Q8.4 < Q10"]);

        // Two runs: reads add up, quality is weighted by reads, no pooled N50.
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "ont_1_leanqc.json", &long_lq("ont_1.fastq.gz", 3000, 20.0, &[]));
        write(t.path(), "ont_2_leanqc.json", &long_lq("ont_2.fastq.gz", 1000, 12.0, &[]));
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["reads", "q30"]));
        let (status, why, row) = &rows["ont"];
        assert_eq!(*status, Status::Pass, "{why:?}");
        assert_eq!((cell(row, "long_reads"), cell(row, "read_q")), ("4000".into(), "18".into()));
        assert!(!row.cells.contains_key("read_n50"));
    }

    #[test]
    fn per_read_kraken_output_does_not_replace_the_report() {
        let t = tempfile::tempdir().unwrap();
        write(t.path(), "S.kraken2.report.txt", "100.00\t10\t0\tR\t1\troot\n 90.00\t9\t9\tS\t562\t  Escherichia coli\n");
        write(t.path(), "S.kraken2.classifiedreads.txt", "C\tr1\t1280\t150\t1280:116\n");
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["kraken"]));
        assert_eq!(cell(&rows["S"].2, "kraken_top"), "Escherichia coli");
        assert_eq!(rows["S"].0, Status::Pass);
    }

    #[test]
    fn leanqc_mates_join_only_when_both_are_there() {
        let t = tempfile::tempdir().unwrap();
        let lq = |f: &str| format!(r#"{{"file":"{f}","checks":[{{"name":"Duplication","status":"fail"}}]}}"#);
        write(t.path(), "iso_R1_leanqc.json", &lq("iso_R1.fastq.gz"));
        write(t.path(), "iso_R2_leanqc.json", &lq("iso_R2.fastq.gz"));
        write(t.path(), "solo_1_leanqc.json", &lq("solo_1.fastq.gz"));
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["none"]));
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["iso", "solo_1"]);
    }

    /// The FastQC fixtures: R1 as `_fastqc.zip`, R2 as an extracted `_fastqc/` folder.
    fn fastqc_fixtures(to: &Path) {
        let from = Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/fastqc");
        std::fs::copy(from.join("ERR044595_1M_1_fastqc.zip"), to.join("ERR044595_1M_1_fastqc.zip")).unwrap();
        for f in ["fastqc_data.txt", "summary.txt"] {
            let body = std::fs::read_to_string(from.join("ERR044595_1M_2_fastqc").join(f)).unwrap();
            write(to, &format!("ERR044595_1M_2_fastqc/{f}"), &body);
        }
    }

    #[test]
    fn fastqc_zip_and_folder_mates_join_and_warn() {
        let t = tempfile::tempdir().unwrap();
        fastqc_fixtures(t.path());
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["none"]));
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["ERR044595_1M"]);
        let (status, why, row) = &rows["ERR044595_1M"];
        assert_eq!(*status, Status::Warn, "{why:?}");
        assert_eq!(cell(row, "fastqc_sequences"), "2000000");
        assert_eq!(cell(row, "fastqc_gc"), "39"); // 38 and 40, weighted by sequences
        assert_eq!(cell(row, "fastqc_fails"), "Adapter Content,Per base sequence quality");
        assert_eq!(cell(row, "fastqc_warns"), "Sequence Duplication Levels");
        assert!(why.iter().any(|w| w == "warn: FastQC failed Adapter Content,Per base sequence quality"), "{why:?}");

        // With a sheet: read-file suffixes (`_1M_1.fastq.gz`) join the sheet name.
        let rows = table(t.path(), &["ERR044595"], Mode::Isolate, Some(&["none"]));
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["ERR044595"]);
        assert_eq!(cell(&rows["ERR044595"].2, "fastqc_sequences"), "2000000");
    }

    #[test]
    fn fastqc_single_mate_keeps_its_name() {
        let t = tempfile::tempdir().unwrap();
        fastqc_fixtures(t.path());
        std::fs::remove_dir_all(t.path().join("ERR044595_1M_2_fastqc")).unwrap();
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["none"]));
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["ERR044595_1M_1"]);
        let row = &rows["ERR044595_1M_1"].2;
        assert_eq!((cell(row, "fastqc_sequences"), cell(row, "fastqc_fails")), ("1000000".into(), "Adapter Content".into()));
    }

    #[test]
    fn fastqc_zip_and_its_extracted_folder_count_once() {
        // `fastqc --extract` leaves both the zip and the folder.
        let t = tempfile::tempdir().unwrap();
        fastqc_fixtures(t.path());
        let mut zip = zip::ZipArchive::new(std::fs::File::open(t.path().join("ERR044595_1M_1_fastqc.zip")).unwrap()).unwrap();
        for f in ["fastqc_data.txt", "summary.txt"] {
            let name = format!("ERR044595_1M_1_fastqc/{f}");
            let mut body = String::new();
            std::io::Read::read_to_string(&mut zip.by_name(&name).unwrap(), &mut body).unwrap();
            write(t.path(), &name, &body);
        }
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["none"]));
        let (status, why, row) = &rows["ERR044595_1M"];
        assert_eq!(*status, Status::Warn, "{why:?}");
        assert_eq!(cell(row, "fastqc_sequences"), "2000000"); // not 3,000,000

        // A different report of the same read file is a conflict, not a second file.
        let data = t.path().join("ERR044595_1M_1_fastqc/fastqc_data.txt");
        let body = std::fs::read_to_string(&data).unwrap().replace("Total Sequences\t1000000", "Total Sequences\t900000");
        std::fs::write(&data, body).unwrap();
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["none"]));
        let (status, why, row) = &rows["ERR044595_1M"];
        assert_eq!(*status, Status::Conflict, "{why:?}");
        assert_eq!(cell(row, "fastqc_sequences"), "CONFLICT");
    }

    #[test]
    fn leanqc_and_fastqc_on_one_sample_are_both_reported_once_each() {
        let t = tempfile::tempdir().unwrap();
        fastqc_fixtures(t.path());
        let lq = |f: &str| format!(r#"{{"file":"{f}","checks":[{{"name":"Adapter content","status":"fail"}}]}}"#);
        write(t.path(), "ERR044595_1M_1_leanqc.json", &lq("ERR044595_1M_1.fastq.gz"));
        write(t.path(), "ERR044595_1M_2_leanqc.json", &lq("ERR044595_1M_2.fastq.gz"));
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["none"]));
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["ERR044595_1M"]);
        let (status, why, row) = &rows["ERR044595_1M"];
        assert_eq!(*status, Status::Warn, "{why:?}");
        assert_eq!(cell(row, "leanqc_fails"), "Adapter content");
        assert_eq!(why.len(), 2, "{why:?}");
        assert!(why[0].contains("leanqc failed Adapter content") && why[1].contains("FastQC failed"), "{why:?}");
    }

    /// An extracted FastQC folder `<dir>/` for read file `file`, with `n` sequences and these
    /// modules failed (all others pass).
    fn fqc(root: &Path, dir: &str, file: &str, n: u64, fails: &[&str]) {
        write(root, &format!("{dir}/fastqc_data.txt"), &format!("##FastQC\t0.12.1\n>>Basic Statistics\tpass\nFilename\t{file}\nTotal Sequences\t{n}\n%GC\t50\n>>END_MODULE\n"));
        let summary: String = MODULES
            .iter()
            .map(|(m, _)| format!("{}\t{m}\t{file}\n", if fails.contains(m) { "FAIL" } else { "PASS" }))
            .collect();
        write(root, &format!("{dir}/summary.txt"), &summary);
    }

    #[test]
    fn only_quality_modules_grade_by_default() {
        // Per base sequence content, duplication and tiles fail on healthy Illumina libraries.
        let t = tempfile::tempdir().unwrap();
        let noisy = ["Per base sequence content", "Sequence Duplication Levels", "Per tile sequence quality"];
        fqc(t.path(), "S_1_fastqc", "S_1.fastq.gz", 1000, &noisy);
        fqc(t.path(), "S_2_fastqc", "S_2.fastq.gz", 1000, &noisy);
        let lq = |f: &str, check: &str| format!(r#"{{"file":"{f}","checks":[{{"name":"{check}","status":"fail"}}]}}"#);
        write(t.path(), "S_1_leanqc.json", &lq("S_1.fastq.gz", "Per-position base content"));
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["none"]));
        let (status, why, row) = &rows["S"];
        assert_eq!(*status, Status::Pass, "{why:?}");
        assert_eq!(cell(row, "fastqc_fails"), "Per base sequence content,Per tile sequence quality,Sequence Duplication Levels");
        assert_eq!(cell(row, "leanqc_fails"), "Per-position base content");

        let rows = table_graded(t.path(), &[], Mode::Isolate, Some(&["none"]), Some(&["all"]));
        let (status, why, _) = &rows["S"];
        assert_eq!(*status, Status::Warn, "{why:?}");
        assert!(why.iter().any(|w| w == "warn: leanqc failed Per-position base content"), "{why:?}");
        // One name grades the FastQC module and the matching leanqc check.
        let rows = table_graded(t.path(), &[], Mode::Isolate, Some(&["none"]), Some(&["per base sequence content"]));
        let why = &rows["S"].1;
        assert_eq!(why, &["warn: leanqc failed Per-position base content", "warn: FastQC failed Per base sequence content"]);

        // Quality and adapters do grade; `none` turns read-QC grading off.
        fqc(t.path(), "S_2_fastqc", "S_2.fastq.gz", 1000, &["Adapter Content", "Per base sequence content"]);
        write(t.path(), "S_2_leanqc.json", &lq("S_2.fastq.gz", "Per-read quality"));
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["none"]));
        let (status, why, _) = &rows["S"];
        assert_eq!(*status, Status::Warn, "{why:?}");
        assert_eq!(why, &["warn: leanqc failed Per-read quality", "warn: FastQC failed Adapter Content"]);
        assert_eq!(table_graded(t.path(), &[], Mode::Isolate, Some(&["none"]), Some(&["none"]))["S"].0, Status::Pass);
        assert!(graded(Some(&["Kmer".to_string()])).is_err());
    }

    #[test]
    fn same_read_file_name_in_two_folders_is_two_reports() {
        // Generic names (`reads_1.fastq.gz`) in two run folders: not one file's zip and folder.
        let t = tempfile::tempdir().unwrap();
        for d in ["A", "B"] {
            fqc(t.path(), &format!("{d}/reads_1_fastqc"), "reads_1.fastq.gz", 1000, &[]);
            fqc(t.path(), &format!("{d}/reads_2_fastqc"), "reads_2.fastq.gz", 1000, &[]);
        }
        let p = t.path().join("B/reads_1_fastqc/fastqc_data.txt");
        std::fs::write(&p, std::fs::read_to_string(&p).unwrap().replace("Total Sequences\t1000", "Total Sequences\t900")).unwrap();
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["none"]));
        let (status, why, row) = &rows["reads"];
        assert_eq!(*status, Status::Pass, "{why:?}");
        assert_eq!(cell(row, "fastqc_sequences"), "3900");
        assert!(row.notes.iter().any(|n| n.contains("reads_1.fastq.gz") && n.contains("separate reports")), "{:?}", row.notes);
        assert_eq!(report_key("x/S_1_fastqc.zip"), report_key("x/S_1_fastqc/fastqc_data.txt"));
        assert_ne!(report_key("x/S_1_fastqc.zip"), report_key("y/S_1_fastqc/fastqc_data.txt"));
    }

    #[test]
    fn truncated_fastqc_zip_is_skipped() {
        let t = tempfile::tempdir().unwrap();
        fastqc_fixtures(t.path());
        let zip = std::fs::read(t.path().join("ERR044595_1M_1_fastqc.zip")).unwrap();
        std::fs::write(t.path().join("ERR064912_1_fastqc.zip"), &zip[..zip.len() / 2]).unwrap();
        let (records, seen, skipped) = parse::scan(&[t.path().to_path_buf()]).unwrap();
        assert_eq!((records.len(), seen["FastQC"]), (2, 2));
        assert!(skipped.iter().any(|s| s.contains("ERR064912_1_fastqc.zip") && s.contains("unreadable FastQC")), "{skipped:?}");
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["none"]));
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["ERR044595_1M"]);
    }

    #[test]
    fn trimmed_read_reports_join_the_sample_in_their_own_columns() {
        let t = tempfile::tempdir().unwrap();
        fqc(t.path(), "raw/S_1_fastqc", "S_1.fastq.gz", 1000, &["Adapter Content"]);
        fqc(t.path(), "raw/S_2_fastqc", "S_2.fastq.gz", 1000, &["Adapter Content"]);
        fqc(t.path(), "fastp/S_1.fastp_fastqc", "S_1.fastp.fastq.gz", 900, &[]);
        fqc(t.path(), "fastp/S_2.fastp_fastqc", "S_2.fastp.fastq.gz", 900, &[]);
        for sheet in [&[][..], &["S"][..]] {
            let rows = table(t.path(), sheet, Mode::Isolate, Some(&["none"]));
            assert_eq!(rows.keys().collect::<Vec<_>>(), ["S"], "sheet {sheet:?}");
            let (status, why, row) = &rows["S"];
            // Raw and trimmed are never added up; the trimmed reads' reports grade.
            assert_eq!((cell(row, "fastqc_sequences"), cell(row, "fastqc_trimmed_sequences")), ("2000".into(), "1800".into()));
            assert_eq!((cell(row, "fastqc_fails"), cell(row, "fastqc_trimmed_fails")), ("Adapter Content".into(), String::new()));
            assert_eq!(*status, Status::Pass, "{why:?}");
        }
        // Trim Galore's `_val_N` and a failure in the trimmed reads.
        let t = tempfile::tempdir().unwrap();
        fqc(t.path(), "S_1_val_1_fastqc", "S_1_val_1.fq.gz", 900, &["Per base sequence quality"]);
        fqc(t.path(), "S_2_val_2_fastqc", "S_2_val_2.fq.gz", 900, &[]);
        let rows = table(t.path(), &["S"], Mode::Isolate, Some(&["none"]));
        let (status, why, row) = &rows["S"];
        assert_eq!(cell(row, "fastqc_trimmed_sequences"), "1800");
        assert!(!row.cells.contains_key("fastqc_sequences"));
        assert_eq!(*status, Status::Warn);
        assert_eq!(why, &["warn: FastQC (trimmed reads) failed Per base sequence quality"]);
    }

    #[test]
    fn lanes_merge_only_with_a_sheet() {
        let t = tempfile::tempdir().unwrap();
        for lane in ["L001", "L002"] {
            for m in ["R1", "R2"] {
                fqc(t.path(), &format!("S_{lane}_{m}_001_fastqc"), &format!("S_{lane}_{m}_001.fastq.gz"), 1000, &[]);
            }
        }
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["none"]));
        assert_eq!(rows.keys().collect::<Vec<_>>(), ["S_L001", "S_L002"]);
        let rows = table(t.path(), &["S"], Mode::Isolate, Some(&["none"]));
        assert_eq!(cell(&rows["S"].2, "fastqc_sequences"), "4000");
    }

    #[test]
    fn mimag_tiers() {
        assert_eq!(mimag(95.0, 1.0), "high*");
        assert_eq!(mimag(90.0, 1.0), "medium"); // high needs > 90
        assert_eq!(mimag(60.0, 9.9), "medium");
        assert_eq!(mimag(60.0, 10.0), "low");
        assert_eq!(mimag(40.0, 1.0), "low");
    }

    #[test]
    fn mapped_depth_is_graded_and_compared_with_shovills_estimate() {
        let t = tempfile::tempdir().unwrap();
        let contigs = format!(">c1\n{}\n", "ACGT".repeat(5000));
        for (s, est, mapped) in [("s1", 50, 50.3), ("s2", 61, 18.0), ("s3", 9, 8.5)] {
            write(t.path(), &format!("asm/{s}/contigs.fa"), &contigs);
            write(t.path(), &format!("asm/{s}/shovill.log"), &format!("[shovill] Estimated sequencing depth: {est} x\n"));
            write(
                t.path(),
                &format!("cov/{s}.coverm.tsv"),
                &format!("Genome\tcontigs.fa/{s}_R1.fq.gz Mean\tcontigs.fa/{s}_R1.fq.gz Covered Fraction\ngenome1\t{mapped}\t0.999\n"),
            );
        }
        write(t.path(), "kleb/klebsiella_pneumo_complex_output.txt", "strain\tenterobacterales__species__species\tklebsiella_pneumo_complex__mlst__ST\ns1\tKlebsiella pneumoniae\tST716\n");
        // a kpsc run on E. coli: its row goes to the Escherichia table, typing not run
        write(t.path(), "kleb/escherichia_output.txt", "strain\tenterobacterales__species__species\tklebsiella_pneumo_complex__mlst__ST\ns2\tEscherichia coli\tNot Tested\n");
        let rows = table(t.path(), &[], Mode::Isolate, Some(&["depth", "kleborate"]));
        let (s1, why1, r1) = &rows["s1"];
        assert_eq!(*s1, Status::Pass, "{why1:?}");
        assert_eq!((cell(r1, "shovill_depth"), cell(r1, "mapped_depth"), cell(r1, "covered_fraction")), ("50".into(), "50.3".into(), "99.9".into()));
        assert_eq!(cell(r1, "kleborate_st"), "ST716");
        let (s2, why2, _) = &rows["s2"];
        assert_eq!(*s2, Status::Warn);
        assert!(why2.iter().any(|w| w.contains("mapped depth 18.0x < 20x")), "{why2:?}");
        assert!(why2.iter().any(|w| w.contains("differ by more than 1.5x")), "{why2:?}");
        assert!(why2.iter().any(|w| w.contains("Escherichia coli, which the preset it ran doesn't type")), "{why2:?}");
        let (s3, why3, _) = &rows["s3"];
        assert_eq!(*s3, Status::Fail, "{why3:?}");
        assert!(why3.iter().any(|w| w.contains("no Kleborate result")), "{why3:?}");
        assert!(!why3.iter().any(|w| w.contains("differ by")), "{why3:?}");
    }
}
