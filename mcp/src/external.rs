//! Third-party tools (fastp, Bakta, CheckM2, GTDB-Tk): typed parameters, checks before running,
//! and their argument vectors.
//!
//! Every flag here exists in the pinned upstream version (see [`PINNED`]); the comment above each
//! builder names the source file it was read from. Nothing is passed that the pinned version
//! doesn't have. Options are written `--name=value` (fastp's cmdline.h and Python's argparse both
//! accept that form), so a value starting with `-` is never read as a flag.

use crate::params::Invocation;
use crate::validate::{self as v, Check, Ctx, Seq};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The upstream version each wrapper's flags were checked against.
pub struct Pinned {
    pub tool: &'static str,
    pub version: &'static str,
    pub bioconda: &'static str,
    pub url: &'static str,
}

pub const PINNED: [Pinned; 4] = [
    Pinned { tool: "fastp", version: "1.4.0", bioconda: "fastp=1.4.0", url: "https://github.com/OpenGene/fastp" },
    Pinned { tool: "bakta", version: "1.12.2", bioconda: "bakta=1.12.2", url: "https://github.com/oschwengers/bakta" },
    Pinned { tool: "checkm2", version: "1.1.0", bioconda: "checkm2=1.1.0", url: "https://github.com/chklovski/CheckM2" },
    Pinned { tool: "gtdbtk", version: "2.8.0", bioconda: "gtdbtk=2.8.0", url: "https://github.com/Ecogenomics/GTDBTk" },
];

pub fn pinned(tool: &str) -> Option<&'static Pinned> {
    PINNED.iter().find(|p| p.tool == tool)
}

/// What to tell the user when the program isn't installed.
pub fn install_hint(tool: &str) -> String {
    match pinned(tool) {
        Some(p) => format!(
            "{tool} not found on PATH. Install the version irit-mcp was checked against with Bioconda \
             (`conda install -c conda-forge -c bioconda {}`, or mamba/micromamba), then restart irit-mcp from that \
             environment, or start it with --tool-path {tool}=/path/to/{tool}",
            p.bioconda
        ),
        None => format!("{tool} not found on PATH"),
    }
}

/// The version number in a `--version` line: the first word that starts with a digit (after an
/// optional `v`) and has a dot. `gtdbtk: version 2.8.0 Copyright ...` gives `2.8.0`.
pub fn version_number(line: &str) -> Option<&str> {
    line.split_whitespace()
        .map(|w| w.trim_end_matches([',', ';', ')']))
        .map(|w| w.strip_prefix('v').unwrap_or(w))
        .find(|w| w.starts_with(|c: char| c.is_ascii_digit()) && w.contains('.'))
}

/// A warning when the installed version isn't the pinned one (the call still runs).
pub fn version_warning(tool: &str, reported: Option<&str>) -> Option<String> {
    let p = pinned(tool)?;
    match reported.and_then(version_number) {
        Some(got) if got == p.version => None,
        Some(got) => Some(format!(
            "installed {tool} is {got}; irit-mcp's parameters were checked against {tool} {}. Flags may differ: check the result",
            p.version
        )),
        None => Some(format!("could not read {tool}'s version; irit-mcp's parameters were checked against {tool} {}", p.version)),
    }
}

fn external(tool: &'static str) -> Invocation {
    let mut inv = Invocation::new(tool);
    inv.external = true;
    inv
}

/// A path given as a list item (CheckM2's `--input a b c`): `./` in front of one starting with `-`.
fn list_item(value: &str) -> String {
    if value.starts_with('-') { format!("./{value}") } else { value.to_string() }
}

/// A database location from the parameter, else from the variable the tool itself reads.
/// Returns the text to pass on and the path resolved against the working folder.
fn db_location(ctx: &Ctx, field: &str, given: &Option<String>, var: &str) -> Check<(String, PathBuf)> {
    let text = match given {
        Some(g) => {
            v::text(field, g)?;
            g.clone()
        }
        None => match ctx.var(var) {
            Some(x) if !x.is_empty() => x.to_string_lossy().into_owned(),
            _ => {
                return v::invalid(
                    field,
                    format!(
                        "no database given and {var} is not set; pass {field} (dblock_env shows the path of a dblock-managed database)"
                    ),
                );
            }
        },
    };
    let path = ctx.cwd.join(&text);
    Ok((text, path))
}

fn output_file_opt(
    ctx: &Ctx,
    inv: &mut Invocation,
    field: &str,
    value: &Option<String>,
    overwrite: bool,
    flag: &str,
) -> Check<Option<PathBuf>> {
    let Some(val) = value else { return Ok(None) };
    let p = v::output_file(ctx, field, val, overwrite)?;
    inv.outputs.push(p.clone());
    inv.opt(flag, val);
    Ok(Some(p))
}

// ---------- fastp ----------
// Flags: fastp v1.4.0 src/main.cpp (`cmd.add` calls), https://github.com/OpenGene/fastp/blob/v1.4.0/src/main.cpp

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FastpParams {
    /// Read 1 FASTQ (plain or gzip).
    pub in1: String,
    /// Read 2 FASTQ for paired-end data.
    #[serde(default)]
    pub in2: Option<String>,
    /// Trimmed read 1 output (`.gz` for gzip). Omit to write only the reports. Inside the root.
    #[serde(default)]
    pub out1: Option<String>,
    /// Trimmed read 2 output; needs in2 and out1. Inside the root.
    #[serde(default)]
    pub out2: Option<String>,
    /// JSON report path, inside the root. Name it `<sample>.fastp.json` so isotable can find it.
    pub json: String,
    /// HTML report path, inside the root. Default: the JSON path with `.html`.
    #[serde(default)]
    pub html: Option<String>,
    /// Replace existing output and report files. Default false: they are refused.
    #[serde(default)]
    pub overwrite: bool,
    /// Title in the HTML report.
    #[serde(default)]
    pub report_title: Option<String>,
    /// Worker threads (1-16). fastp's default is 3.
    #[serde(default)]
    pub thread: Option<u32>,
    /// gzip level for `.gz` outputs, 1-9. fastp's default is 4.
    #[serde(default)]
    pub compression: Option<u32>,
    /// Input uses phred64 qualities (output is converted to phred33).
    #[serde(default)]
    pub phred64: bool,
    /// Paired-end only: also detect the adapter by overlap analysis (slower, slightly cleaner).
    #[serde(default)]
    pub detect_adapter_for_pe: bool,
    /// Read 1 adapter (A/C/G/T/N). Default: auto-detected.
    #[serde(default)]
    pub adapter_sequence: Option<String>,
    /// Read 2 adapter, paired-end only. Default: same as adapter_sequence.
    #[serde(default)]
    pub adapter_sequence_r2: Option<String>,
    /// Turn adapter trimming off.
    #[serde(default)]
    pub disable_adapter_trimming: bool,
    /// Bases to trim from the front of read 1.
    #[serde(default)]
    pub trim_front1: Option<u32>,
    /// Bases to trim from the tail of read 1.
    #[serde(default)]
    pub trim_tail1: Option<u32>,
    /// Force polyG tail trimming (on by default for NextSeq/NovaSeq data).
    #[serde(default)]
    pub trim_poly_g: bool,
    /// Turn polyG tail trimming off.
    #[serde(default)]
    pub disable_trim_poly_g: bool,
    /// Sliding window from 5' to 3', dropping low-quality bases at the front.
    #[serde(default)]
    pub cut_front: bool,
    /// Sliding window from 3' to 5', dropping low-quality bases at the tail.
    #[serde(default)]
    pub cut_tail: bool,
    /// Sliding window from front to tail; at the first low-quality window, drop it and everything after.
    #[serde(default)]
    pub cut_right: bool,
    /// Window size for the cut options, 1-1000. Default 4.
    #[serde(default)]
    pub cut_window_size: Option<u32>,
    /// Mean quality for the cut options, 1-36. Default 20.
    #[serde(default)]
    pub cut_mean_quality: Option<u32>,
    /// A base is qualified at this phred quality or above. Default 15.
    #[serde(default)]
    pub qualified_quality_phred: Option<u32>,
    /// Percent of unqualified bases allowed, 0-100. Default 40.
    #[serde(default)]
    pub unqualified_percent_limit: Option<u32>,
    /// Discard a read with more N bases than this. Default 5.
    #[serde(default)]
    pub n_base_limit: Option<u32>,
    /// Discard a read with mean quality below this. Default 0 (off).
    #[serde(default)]
    pub average_qual: Option<u32>,
    /// Discard reads shorter than this after trimming. Default 15.
    #[serde(default)]
    pub length_required: Option<u32>,
    /// Paired-end only: correct mismatched bases in overlapping regions.
    #[serde(default)]
    pub correction: bool,
    /// Drop duplicated reads/pairs.
    #[serde(default)]
    pub dedup: bool,
    /// Report overrepresented sequences.
    #[serde(default)]
    pub overrepresentation_analysis: bool,
}

fn adapter(field: &str, s: &str) -> Check<()> {
    if s.is_empty() || !s.chars().all(|c| "ACGTNacgtn".contains(c)) {
        return v::invalid(field, format!("{s:?} is not an adapter sequence (A, C, G, T, N)"));
    }
    Ok(())
}

/// The HTML report path: as given, else the JSON path with `.html`.
pub fn fastp_html(p: &FastpParams) -> String {
    p.html.clone().unwrap_or_else(|| match p.json.strip_suffix(".json") {
        Some(stem) => format!("{stem}.html"),
        None => format!("{}.html", p.json),
    })
}

pub fn fastp(ctx: &Ctx, p: &FastpParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("fastp");
    // Seen in the real-tools workflow: the HTML report differs between identical runs; the JSON doesn't.
    inv.volatile = &[".html"];
    let in1 = v::existing_file(cwd, "in1", &p.in1)?;
    v::sequence_file(&in1, "in1", &p.in1, Seq::Fastq)?;
    inv.inputs.push(in1);
    inv.opt("in1", &p.in1);
    let paired = p.in2.is_some();
    if let Some(in2) = &p.in2 {
        let path = v::existing_file(cwd, "in2", in2)?;
        if path == inv.inputs[0] {
            return v::invalid("in2", "in2 is the same file as in1");
        }
        v::sequence_file(&path, "in2", in2, Seq::Fastq)?;
        inv.inputs.push(path);
        inv.opt("in2", in2);
    }
    if p.out2.is_some() && (!paired || p.out1.is_none()) {
        return v::invalid("out2", "out2 needs in2 and out1");
    }
    if paired && p.out1.is_some() && p.out2.is_none() {
        return v::invalid("out2", "paired-end input with out1 needs out2 as well");
    }
    for (field, on) in [("detect_adapter_for_pe", p.detect_adapter_for_pe), ("correction", p.correction)] {
        if on && !paired {
            return v::invalid(field, "only for paired-end input (give in2)");
        }
    }
    if p.adapter_sequence_r2.is_some() && !paired {
        return v::invalid("adapter_sequence_r2", "only for paired-end input (give in2)");
    }
    if p.trim_poly_g && p.disable_trim_poly_g {
        return v::invalid("trim_poly_g", "trim_poly_g and disable_trim_poly_g contradict each other");
    }
    let o1 = output_file_opt(ctx, &mut inv, "out1", &p.out1, p.overwrite, "out1")?;
    let o2 = output_file_opt(ctx, &mut inv, "out2", &p.out2, p.overwrite, "out2")?;
    let json = v::output_file(ctx, "json", &p.json, p.overwrite)?;
    inv.outputs.push(json.clone());
    inv.opt("json", &p.json);
    let html_text = fastp_html(p);
    let html = v::output_file(ctx, "html", &html_text, p.overwrite)?;
    inv.outputs.push(html.clone());
    inv.opt("html", &html_text);
    let outs: Vec<(&str, &PathBuf)> = [("out1", o1.as_ref()), ("out2", o2.as_ref()), ("json", Some(&json)), ("html", Some(&html))]
        .into_iter()
        .filter_map(|(f, p)| Some((f, p?)))
        .collect();
    for (i, (field, a)) in outs.iter().enumerate() {
        if outs[..i].iter().any(|(_, b)| b == a) || inv.inputs.contains(a) {
            return v::invalid(field, format!("{} is used twice; give every input and output its own path", a.display()));
        }
    }
    inv.expect = vec![vec![json], vec![html]];
    inv.expect.extend([o1, o2].into_iter().flatten().map(|o| vec![o]));

    if let Some(t) = &p.report_title {
        v::text("report_title", t)?;
        inv.opt("report_title", t);
    }
    let ranged = [
        ("thread", p.thread, 1, 16),
        ("compression", p.compression, 1, 9),
        ("trim_front1", p.trim_front1, 0, 10_000),
        ("trim_tail1", p.trim_tail1, 0, 10_000),
        ("cut_window_size", p.cut_window_size, 1, 1000),
        ("cut_mean_quality", p.cut_mean_quality, 1, 36),
        ("qualified_quality_phred", p.qualified_quality_phred, 0, 93),
        ("unqualified_percent_limit", p.unqualified_percent_limit, 0, 100),
        ("n_base_limit", p.n_base_limit, 0, 10_000),
        ("average_qual", p.average_qual, 0, 93),
        ("length_required", p.length_required, 0, 1_000_000),
    ];
    for (name, val, lo, hi) in ranged {
        if let Some(x) = val {
            v::range(name, x, lo, hi)?;
            inv.opt(name, x);
        }
    }
    for (name, seq) in [("adapter_sequence", &p.adapter_sequence), ("adapter_sequence_r2", &p.adapter_sequence_r2)] {
        if let Some(s) = seq {
            adapter(name, s)?;
            inv.opt(name, s);
        }
    }
    for (name, on) in [
        ("phred64", p.phred64),
        ("detect_adapter_for_pe", p.detect_adapter_for_pe),
        ("disable_adapter_trimming", p.disable_adapter_trimming),
        ("trim_poly_g", p.trim_poly_g),
        ("disable_trim_poly_g", p.disable_trim_poly_g),
        ("cut_front", p.cut_front),
        ("cut_tail", p.cut_tail),
        ("cut_right", p.cut_right),
        ("correction", p.correction),
        ("dedup", p.dedup),
        ("overrepresentation_analysis", p.overrepresentation_analysis),
    ] {
        inv.flag(name, on);
    }
    Ok(inv)
}

// ---------- Bakta ----------
// Flags: Bakta v1.12.2 bakta/utils.py `parse_arguments`; database check: bakta/db.py `check`
// (https://github.com/oschwengers/bakta/blob/v1.12.2/bakta/utils.py, .../bakta/db.py).

/// Bakta 1.12.2's `__db_schema_version__`: it refuses any other major database version.
pub const BAKTA_DB_SCHEMA: u64 = 6;

/// `bakta/db.py` FILE_NAMES at v1.12.2; plus `psc.dmnd` (full) or `pscc.dmnd` (light).
const BAKTA_DB_FILES: [&str; 26] = [
    "antifam.h3f",
    "antifam.h3i",
    "antifam.h3m",
    "antifam.h3p",
    "bakta.db",
    "expert-protein-sequences.dmnd",
    "ncRNA-genes.i1f",
    "ncRNA-genes.i1i",
    "ncRNA-genes.i1m",
    "ncRNA-genes.i1p",
    "ncRNA-regions.i1f",
    "ncRNA-regions.i1i",
    "ncRNA-regions.i1m",
    "ncRNA-regions.i1p",
    "oric.fna",
    "orit.fna",
    "pfam.h3f",
    "pfam.h3i",
    "pfam.h3m",
    "pfam.h3p",
    "rfam-go.tsv",
    "rRNA.i1f",
    "rRNA.i1i",
    "rRNA.i1m",
    "rRNA.i1p",
    "sorf.dmnd",
];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Gram {
    /// `--gram +`
    Positive,
    /// `--gram -`
    Negative,
    /// `--gram ?` (Bakta's default)
    Unknown,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BaktaSkip {
    Trna,
    Tmrna,
    Rrna,
    Ncrna,
    NcrnaRegion,
    Crispr,
    Cds,
    Pseudo,
    Sorf,
    Gap,
    Ori,
    Filter,
    Plot,
}

impl BaktaSkip {
    fn flag(self) -> &'static str {
        match self {
            BaktaSkip::Trna => "skip-trna",
            BaktaSkip::Tmrna => "skip-tmrna",
            BaktaSkip::Rrna => "skip-rrna",
            BaktaSkip::Ncrna => "skip-ncrna",
            BaktaSkip::NcrnaRegion => "skip-ncrna-region",
            BaktaSkip::Crispr => "skip-crispr",
            BaktaSkip::Cds => "skip-cds",
            BaktaSkip::Pseudo => "skip-pseudo",
            BaktaSkip::Sorf => "skip-sorf",
            BaktaSkip::Gap => "skip-gap",
            BaktaSkip::Ori => "skip-ori",
            BaktaSkip::Filter => "skip-filter",
            BaktaSkip::Plot => "skip-plot",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BaktaParams {
    /// Genome assembly, FASTA (plain or gzip).
    pub genome: String,
    /// Bakta database folder (the one holding version.json, schema v6 for Bakta 1.12). Default: $BAKTA_DB as the server started with. dblock_env shows the path of a dblock-managed one (bakta-db-v6-full or -light).
    #[serde(default)]
    pub db: Option<String>,
    /// Output folder, inside the root.
    pub outdir: String,
    /// Allow an outdir that already has files (Bakta replaces files of the same name). Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// Prefix of the output files. Default: the genome file name without its last extension (as Bakta).
    #[serde(default)]
    pub prefix: Option<String>,
    /// Drop contigs shorter than this. Bakta's default 1 (200 with compliant).
    #[serde(default)]
    pub min_contig_length: Option<u32>,
    #[serde(default)]
    pub genus: Option<String>,
    #[serde(default)]
    pub species: Option<String>,
    #[serde(default)]
    pub strain: Option<String>,
    /// Plasmid name.
    #[serde(default)]
    pub plasmid: Option<String>,
    /// All sequences are complete replicons (chromosome/plasmids).
    #[serde(default)]
    pub complete: bool,
    /// Translation table: 11 (default), 4 or 25.
    #[serde(default)]
    pub translation_table: Option<u8>,
    /// Gram type for signal peptide prediction.
    #[serde(default)]
    pub gram: Option<Gram>,
    /// Locus (sequence) name prefix. Default `contig`.
    #[serde(default)]
    pub locus: Option<String>,
    /// Locus tag prefix. Default: generated.
    #[serde(default)]
    pub locus_tag: Option<String>,
    /// Keep the original contig headers.
    #[serde(default)]
    pub keep_contig_headers: bool,
    /// Force GenBank/ENA/DDBJ compliance.
    #[serde(default)]
    pub compliant: bool,
    /// Metagenome mode (affects CDS prediction only).
    #[serde(default)]
    pub meta: bool,
    /// Threads. Default: all available CPUs.
    #[serde(default)]
    pub threads: Option<u32>,
    /// Steps to skip, e.g. ["plot", "crispr"].
    #[serde(default)]
    pub skip: Vec<BaktaSkip>,
}

/// Checks a Bakta database folder the way Bakta 1.12.2's `db.check` does, before hours are spent.
fn bakta_db(field: &str, shown: &str, dir: &Path) -> Check<PathBuf> {
    if !dir.is_dir() {
        return v::invalid(field, format!("{shown} is not a folder"));
    }
    let version = dir.join("version.json");
    let info: serde_json::Value = match std::fs::read(&version).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
        Some(j) => j,
        None => {
            return v::invalid(
                field,
                format!(
                    "{shown} has no readable version.json, so it is not a Bakta database folder (maybe the database is in a db/ or db-light/ folder inside it)"
                ),
            );
        }
    };
    for key in ["date", "major", "minor", "type"] {
        if info.get(key).is_none() {
            return v::invalid(field, format!("{shown}/version.json has no {key:?}; not a Bakta database"));
        }
    }
    let major = info["major"].as_u64().unwrap_or(0);
    if major != BAKTA_DB_SCHEMA {
        return v::invalid(
            field,
            format!(
                "{shown} is Bakta database v{major}.{}; Bakta 1.12 needs v{BAKTA_DB_SCHEMA}.x (dblock: bakta-db-v6-full or bakta-db-v6-light)",
                info["minor"]
            ),
        );
    }
    let protein = if info["type"] == "full" { "psc.dmnd" } else { "pscc.dmnd" };
    let missing: Vec<&str> = BAKTA_DB_FILES.iter().copied().chain([protein]).filter(|f| !dir.join(f).is_file()).collect();
    if !missing.is_empty() {
        return v::invalid(field, format!("{shown} is missing {} (incomplete download?)", missing.join(", ")));
    }
    Ok(version)
}

pub fn bakta(ctx: &Ctx, p: &BaktaParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("bakta");
    inv.volatile = &[".log"];
    let genome = v::existing_file(cwd, "genome", &p.genome)?;
    v::sequence_file(&genome, "genome", &p.genome, Seq::Fasta)?;
    inv.inputs.push(genome);
    let (db_text, db) = db_location(ctx, "db", &p.db, "BAKTA_DB")?;
    // version.json only: the database itself is tens of GB (dblock_verify checks it).
    inv.inputs.push(bakta_db("db", &db_text, &db)?);
    inv.opt("db", &db_text);
    let outdir = v::output_dir(ctx, "outdir", &p.outdir, Some(p.overwrite), true)?;
    inv.outputs.push(outdir.clone());
    inv.opt("output", &p.outdir);
    // Bakta refuses any existing output folder without --force; irit-mcp has already refused a
    // non-empty one unless overwrite is true, and replay.sh reruns into the same folder.
    inv.flag("force", true);
    let prefix = match &p.prefix {
        Some(x) => x.clone(),
        // Python's Path(genome).stem
        None => Path::new(&p.genome).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
    };
    v::file_stem("prefix", &prefix)?;
    inv.opt("prefix", &prefix);
    inv.expect = ["txt", "json", "tsv", "gff3", "gbff"].iter().map(|ext| vec![outdir.join(format!("{prefix}.{ext}"))]).collect();
    if let Some(m) = p.min_contig_length {
        inv.opt("min-contig-length", m);
    }
    for (name, val) in [("genus", &p.genus), ("species", &p.species), ("strain", &p.strain), ("plasmid", &p.plasmid)] {
        if let Some(x) = val {
            v::text(name, x)?;
            inv.opt(name, x);
        }
    }
    inv.flag("complete", p.complete);
    if let Some(t) = p.translation_table {
        if ![11, 4, 25].contains(&t) {
            return v::invalid("translation_table", format!("{t} is not 11, 4 or 25"));
        }
        inv.opt("translation-table", t);
    }
    if let Some(g) = p.gram {
        inv.opt(
            "gram",
            match g {
                Gram::Positive => "+",
                Gram::Negative => "-",
                Gram::Unknown => "?",
            },
        );
    }
    // Bakta's own rules (bakta/config.py): locus 1-20 of alphanumerics and _.:*#-; locus tag 1-24
    // of alphanumerics and _.-.
    if let Some(l) = &p.locus {
        let ok = (1..=20).contains(&l.len()) && l.chars().all(|c| c.is_ascii_alphanumeric() || "_.:*#-".contains(c));
        if !ok {
            return v::invalid("locus", "1-20 letters, digits or _ . : * # -");
        }
        inv.opt("locus", l);
    }
    if let Some(l) = &p.locus_tag {
        let ok = (1..=24).contains(&l.len()) && l.chars().all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c));
        if !ok {
            return v::invalid("locus_tag", "1-24 letters, digits or _ . -");
        }
        inv.opt("locus-tag", l);
    }
    inv.flag("keep-contig-headers", p.keep_contig_headers);
    inv.flag("compliant", p.compliant);
    inv.flag("meta", p.meta);
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("threads", t);
    }
    for (i, s) in p.skip.iter().enumerate() {
        inv.flag(s.flag(), !p.skip[..i].contains(s));
    }
    inv.positionals(std::slice::from_ref(&p.genome));
    Ok(inv)
}

// ---------- CheckM2 ----------
// Flags: CheckM2 1.1.0 checkm2/main.py (`predict` subparser and the shared parent parser),
// https://github.com/chklovski/CheckM2/blob/1.1.0/checkm2/main.py

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Checkm2Model {
    /// CheckM2 chooses per genome (default).
    Auto,
    /// `--general`: gradient boost model.
    General,
    /// `--specific`: neural network model.
    Specific,
    /// `--allmodels`: report both.
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Checkm2Params {
    /// Genome FASTA files (plain or gzip). Give this or genome_dir.
    #[serde(default)]
    pub genomes: Vec<String>,
    /// Folder of genome FASTA files, matched by `extension`. Give this or genomes.
    #[serde(default)]
    pub genome_dir: Option<String>,
    /// With genome_dir: file extension to match. CheckM2's default is .fna.
    #[serde(default)]
    pub extension: Option<String>,
    /// Output folder, inside the root. quality_report.tsv goes here.
    pub outdir: String,
    /// Allow a non-empty outdir. CheckM2 (--force) then DELETES everything in it first. Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// CheckM2 DIAMOND database: the .dmnd file, or a folder holding exactly one. Default: $CHECKM2DB as the server started with (dblock_env shows the path for checkm2-db-v3).
    #[serde(default)]
    pub db: Option<String>,
    /// Threads.
    #[serde(default)]
    pub threads: Option<u32>,
    /// Which quality model to use. Default auto.
    #[serde(default)]
    pub model: Option<Checkm2Model>,
    /// Low memory mode: smaller DIAMOND blocks, less RAM, slower. Use on 8-16 GB laptops.
    #[serde(default)]
    pub lowmem: bool,
    /// Prodigal translation table. Default: CheckM2 picks 11 or 4.
    #[serde(default)]
    pub ttable: Option<u8>,
    /// Inputs are protein FASTA files, not nucleotide.
    #[serde(default)]
    pub genes: bool,
    /// Delete intermediate files (protein files, DIAMOND output) at the end.
    #[serde(default)]
    pub remove_intermediates: bool,
}

/// CheckM2 1.1.0's `allowable_ttables` in main.py.
const CHECKM2_TTABLES: [u8; 25] = [1, 2, 3, 4, 5, 6, 9, 10, 11, 12, 13, 14, 16, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 33];

/// The .dmnd file: given directly, or the only one under a folder (as dblock's `glob:*.dmnd`).
fn checkm2_db(field: &str, shown: &str, path: &Path) -> Check<PathBuf> {
    if path.is_file() {
        if path.extension().is_some_and(|e| e == "dmnd") {
            return Ok(path.to_path_buf());
        }
        return v::invalid(field, format!("{shown} is not a DIAMOND .dmnd file"));
    }
    if !path.is_dir() {
        return v::invalid(field, format!("{shown} does not exist"));
    }
    let mut found = Vec::new();
    let mut stack = vec![(path.to_path_buf(), 0)];
    while let Some((dir, depth)) = stack.pop() {
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() && depth < 3 {
                stack.push((p, depth + 1));
            } else if p.is_file() && p.extension().is_some_and(|e| e == "dmnd") {
                found.push(p);
            }
        }
    }
    match found.len() {
        1 => Ok(found.remove(0)),
        0 => v::invalid(field, format!("no .dmnd file under {shown}; not a CheckM2 database")),
        _ => v::invalid(field, format!("several .dmnd files under {shown}; give the file itself")),
    }
}

pub fn checkm2(ctx: &Ctx, p: &Checkm2Params) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("checkm2");
    inv.volatile = &["checkm2.log"];
    inv.args.push("predict".into());
    let mut input_args = Vec::new();
    match (&p.genome_dir, p.genomes.is_empty()) {
        (Some(_), false) | (None, true) => {
            return v::invalid("genomes", "give either genomes (files) or genome_dir (a folder)");
        }
        (Some(dir), true) => {
            let path = v::existing_dir(cwd, "genome_dir", dir)?;
            let ext = p.extension.as_deref().unwrap_or(".fna");
            v::extension("extension", ext)?;
            let files = v::files_with_extension(&path, ext);
            if files.is_empty() {
                return v::invalid("genome_dir", format!("no *{ext} files in {dir}; set extension (e.g. fa, fasta, fna.gz)"));
            }
            for f in &files {
                let shown = f.strip_prefix(cwd).unwrap_or(f).to_string_lossy().into_owned();
                v::sequence_file(f, "genome_dir", &shown, Seq::Fasta)?;
            }
            inv.inputs.extend(files);
            if p.extension.is_some() {
                inv.opt("extension", ext);
            }
            input_args.push(list_item(dir));
        }
        (None, false) => {
            if p.extension.is_some() {
                return v::invalid("extension", "extension applies to genome_dir only");
            }
            for g in &p.genomes {
                let path = v::existing_file(cwd, "genomes", g)?;
                v::sequence_file(&path, "genomes", g, Seq::Fasta)?;
                inv.inputs.push(path);
                input_args.push(list_item(g));
            }
        }
    }
    let (db_text, db) = db_location(ctx, "db", &p.db, "CHECKM2DB")?;
    let dmnd = checkm2_db("db", &db_text, &db)?;
    // Pass the file itself; the database (about 3 GB) is not hashed (dblock_verify does that).
    inv.opt("database_path", dmnd.to_string_lossy());
    let outdir = v::output_dir(ctx, "outdir", &p.outdir, Some(p.overwrite), true)?;
    inv.outputs.push(outdir.clone());
    inv.opt("output-directory", &p.outdir);
    // CheckM2 refuses a non-empty folder without --force and empties it with --force. irit-mcp
    // has refused a non-empty folder unless overwrite is true, and replay.sh needs --force to
    // run into the folder again.
    inv.flag("force", true);
    inv.expect = vec![vec![outdir.join("quality_report.tsv")]];
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("threads", t);
    }
    match p.model {
        Some(Checkm2Model::General) => inv.flag("general", true),
        Some(Checkm2Model::Specific) => inv.flag("specific", true),
        Some(Checkm2Model::Both) => inv.flag("allmodels", true),
        Some(Checkm2Model::Auto) | None => {}
    }
    inv.flag("lowmem", p.lowmem);
    if let Some(t) = p.ttable {
        if !CHECKM2_TTABLES.contains(&t) {
            return v::invalid("ttable", format!("{t} is not a translation table CheckM2 accepts"));
        }
        inv.opt("ttable", t);
    }
    inv.flag("genes", p.genes);
    inv.flag("remove_intermediates", p.remove_intermediates);
    // `--input` takes several values (nargs='+'), so it goes last, one value per argument.
    inv.args.push("--input".into());
    inv.args.extend(input_args);
    Ok(inv)
}

// ---------- GTDB-Tk ----------
// Flags: GTDB-Tk 2.8.0 gtdbtk/cli.py (`classify_wf` subparser); data folders: gtdbtk/config/common.py;
// outputs: gtdbtk/config/output.py (https://github.com/Ecogenomics/GTDBTk/tree/2.8.0/gtdbtk).
// Not in 2.8.0, so not offered: --skip_ani_screen (only a classify_wf internal), --mash_db (gone
// since 2.5.0, when skani replaced Mash), and any memory cap (--max_memory belongs to gPlace,
// which GTDB-Tk calls as `pplacer`; see docs/research/gtdb-placement-test.md).

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GtdbtkClassifyWfParams {
    /// Folder of genome FASTA files (one genome per file), matched by `extension`.
    pub genome_dir: String,
    /// File extension of the genomes. GTDB-Tk's default is fna; use fna.gz for gzipped files.
    #[serde(default)]
    pub extension: Option<String>,
    /// Output folder, inside the root.
    pub out_dir: String,
    /// Allow an out_dir that already has files (GTDB-Tk writes over them). Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// GTDB-Tk reference data folder (the release folder holding metadata/, markers/, skani/, ...). Default: $GTDBTK_DATA_PATH as the server started with. dblock_env shows the path for gtdbtk-r232.
    #[serde(default)]
    pub data_path: Option<String>,
    /// Prefix of the output files. Default gtdbtk.
    #[serde(default)]
    pub prefix: Option<String>,
    /// CPUs. GTDB-Tk's default is 1.
    #[serde(default)]
    pub cpus: Option<u32>,
    /// CPUs for pplacer placement. Each pplacer thread holds its own copy of the tree in memory, so use 1 on a laptop.
    #[serde(default)]
    pub pplacer_cpus: Option<u32>,
    /// Folder for pplacer's memory-mapped file: much less RAM, slower. Must be inside the root, on a disk with tens of GB free.
    #[serde(default)]
    pub scratch_dir: Option<String>,
    /// Use the unsplit bacterial tree (GTDB-Tk before v2). Needs far more RAM than a laptop has.
    #[serde(default)]
    pub full_tree: bool,
    /// Place genomes on the tree even when skani has already classified them.
    #[serde(default)]
    pub place_species: bool,
    /// Minimum alignment fraction to assign a genome to a species cluster (0-1).
    #[serde(default)]
    pub min_af: Option<f64>,
    /// Exclude genomes with fewer than this % of amino acids in the MSA. Default 10.
    #[serde(default)]
    pub min_perc_aa: Option<f64>,
    /// Keep intermediate files.
    #[serde(default)]
    pub keep_intermediates: bool,
    /// Also write unaligned single-copy marker genes.
    #[serde(default)]
    pub write_single_copy_genes: bool,
    /// GTDB-Tk --force: continue when one genome fails (it does NOT mean overwrite).
    #[serde(default)]
    pub force: bool,
    /// Inputs are called genes (protein FASTA), not nucleotide.
    #[serde(default)]
    pub genes: bool,
}

/// Folders classify_wf reads in GTDB-Tk 2.8.0 (`gtdbtk/config/common.py`).
const GTDBTK_DIRS: [&str; 9] = ["markers", "masks", "metadata", "mrca_red", "pplacer", "radii", "split", "taxonomy", "skani"];

fn gtdbtk_data(field: &str, shown: &str, dir: &Path) -> Check<PathBuf> {
    if !dir.is_dir() {
        return v::invalid(field, format!("{shown} is not a folder"));
    }
    let meta = dir.join("metadata/metadata.txt");
    let version = std::fs::read_to_string(&meta)
        .ok()
        .and_then(|t| t.lines().find_map(|l| l.trim().strip_prefix("VERSION_DATA=").map(str::to_string)));
    if version.is_none() {
        return v::invalid(
            field,
            format!(
                "{shown} has no metadata/metadata.txt with VERSION_DATA, so it is not GTDB-Tk reference data (point at the release folder, e.g. .../release232)"
            ),
        );
    }
    let missing: Vec<&str> = GTDBTK_DIRS.iter().copied().filter(|d| !dir.join(d).is_dir()).collect();
    if missing == ["skani"] {
        return v::invalid(
            field,
            format!(
                "{shown} has no skani/ folder. GTDB-Tk 2.8.0 classify_wf always runs the skani ANI screen and has no option to skip it. \
                 The placement-only data (dblock gtdbtk-r232-placement) has no ANI database; it needs the launcher in docs/research/gtdb-placement-test.md, \
                 not classify_wf"
            ),
        );
    }
    if !missing.is_empty() {
        return v::invalid(field, format!("{shown} is missing {} (incomplete GTDB-Tk data?)", missing.join(", ")));
    }
    Ok(meta)
}

pub fn gtdbtk_classify_wf(ctx: &Ctx, p: &GtdbtkClassifyWfParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("gtdbtk");
    // Seen in the real-tools workflow: logs, gtdbtk.json (step start/end times) and gzip files
    // (gzip headers carry a time) differ between identical runs.
    inv.volatile = &[".log", "gtdbtk.json", ".gz"];
    inv.args.push("classify_wf".into());
    let dir = v::existing_dir(cwd, "genome_dir", &p.genome_dir)?;
    let ext = p.extension.as_deref().unwrap_or("fna");
    v::extension("extension", ext)?;
    let files = v::files_with_extension(&dir, ext);
    if files.is_empty() {
        return v::invalid(
            "genome_dir",
            format!("no *.{} files in {}; set extension (e.g. fa, fasta, fna.gz)", ext.trim_start_matches('.'), p.genome_dir),
        );
    }
    for f in &files {
        let shown = f.strip_prefix(cwd).unwrap_or(f).to_string_lossy().into_owned();
        v::sequence_file(f, "genome_dir", &shown, Seq::Fasta)?;
    }
    inv.inputs.extend(files);
    inv.opt("genome_dir", &p.genome_dir);
    let (data_text, data) = db_location(ctx, "data_path", &p.data_path, "GTDBTK_DATA_PATH")?;
    inv.inputs.push(gtdbtk_data("data_path", &data_text, &data)?);
    // GTDB-Tk has no data-path flag: it reads GTDBTK_DATA_PATH, recorded and exported by replay.sh.
    inv.env.insert("GTDBTK_DATA_PATH".into(), v::resolve(&data).to_string_lossy().into_owned());
    let out = v::output_dir(ctx, "out_dir", &p.out_dir, Some(p.overwrite), true)?;
    inv.outputs.push(out.clone());
    inv.opt("out_dir", &p.out_dir);
    if p.extension.is_some() {
        inv.opt("extension", ext);
    }
    let prefix = p.prefix.as_deref().unwrap_or("gtdbtk");
    v::file_stem("prefix", prefix)?;
    if p.prefix.is_some() {
        inv.opt("prefix", prefix);
    }
    // Either summary may be absent (no archaea, say), but not both.
    inv.expect = vec![["bac120", "ar53"].iter().map(|m| out.join("classify").join(format!("{prefix}.{m}.summary.tsv"))).collect()];
    for (name, val) in [("cpus", p.cpus), ("pplacer_cpus", p.pplacer_cpus)] {
        if let Some(x) = val {
            v::range(name, x, 1, 1024)?;
            inv.opt(name, x);
        }
    }
    if let Some(s) = &p.scratch_dir {
        v::output_dir(ctx, "scratch_dir", s, None, false)?;
        inv.opt("scratch_dir", s);
    }
    if let Some(x) = p.min_af {
        if !x.is_finite() || !(0.0..=1.0).contains(&x) {
            return v::invalid("min_af", format!("{x} is outside 0..=1"));
        }
        inv.opt("min_af", x);
    }
    if let Some(x) = p.min_perc_aa {
        v::percent("min_perc_aa", x)?;
        inv.opt("min_perc_aa", x);
    }
    for (name, on) in [
        ("full_tree", p.full_tree),
        ("place_species", p.place_species),
        ("keep_intermediates", p.keep_intermediates),
        ("write_single_copy_genes", p.write_single_copy_genes),
        ("force", p.force),
        ("genes", p.genes),
    ] {
        inv.flag(name, on);
    }
    Ok(inv)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn ctx(dir: &Path) -> Ctx {
        let root = std::fs::canonicalize(dir).unwrap();
        Ctx { cwd: root.clone(), runs_root: Some(root.join("irit-runs")), root, allow_outside_root: false, vars: BTreeMap::new() }
    }

    fn parse<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
        serde_json::from_value(v).unwrap()
    }

    fn write(dir: &Path, name: &str, text: &str) {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    pub fn fake_bakta_db(dir: &Path, major: u32) {
        write(dir, "version.json", &format!(r#"{{"date": "2025-02-24", "major": {major}, "minor": 0, "type": "light"}}"#));
        for f in BAKTA_DB_FILES.iter().chain(&["pscc.dmnd"]) {
            write(dir, f, "x");
        }
    }

    pub fn fake_gtdbtk_data(dir: &Path, with_skani: bool) {
        write(dir, "metadata/metadata.txt", "VERSION_DATA=r232\n");
        for d in GTDBTK_DIRS {
            if d != "skani" || with_skani {
                std::fs::create_dir_all(dir.join(d)).unwrap();
            }
        }
    }

    #[test]
    fn versions() {
        assert_eq!(version_number("gtdbtk: version 2.8.0 Copyright 2017 Pierre-Alain Chaumeil"), Some("2.8.0"));
        assert_eq!(version_number("bakta 1.12.2"), Some("1.12.2"));
        assert_eq!(version_number("1.1.0"), Some("1.1.0"));
        assert_eq!(version_number("fastp v1.4.0"), Some("1.4.0"));
        assert_eq!(version_number("no version"), None);
        assert!(version_warning("fastp", Some("fastp 1.4.0")).is_none());
        assert!(version_warning("fastp", Some("fastp 0.23.4")).unwrap().contains("0.23.4"));
        assert!(version_warning("fastp", None).is_some());
        assert!(install_hint("bakta").contains("bioconda bakta=1.12.2"));
    }

    #[test]
    fn fastp_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "-r1.fq", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "r2.fq", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "asm.fa", ">c1\nACGT\n");
        std::fs::create_dir(d.path().join("trim")).unwrap();
        let p: FastpParams = parse(json!({
            "in1": "-r1.fq", "in2": "r2.fq", "out1": "trim/s_1.fq.gz", "out2": "trim/s_2.fq.gz",
            "json": "trim/s.fastp.json", "thread": 4, "detect_adapter_for_pe": true, "cut_tail": true,
            "length_required": 50, "adapter_sequence": "AGATCGGAAGAGC"
        }));
        let inv = fastp(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "--in1=-r1.fq",
                "--in2=r2.fq",
                "--out1=trim/s_1.fq.gz",
                "--out2=trim/s_2.fq.gz",
                "--json=trim/s.fastp.json",
                "--html=trim/s.fastp.html",
                "--thread=4",
                "--length_required=50",
                "--adapter_sequence=AGATCGGAAGAGC",
                "--detect_adapter_for_pe",
                "--cut_tail",
            ]
        );
        assert!(inv.external);
        assert_eq!(inv.expect.len(), 4);
        assert_eq!(inv.outputs.len(), 4);

        let bad = |v: serde_json::Value| fastp(&cx, &parse(v)).unwrap_err().field;
        assert_eq!(bad(json!({"in1": "missing.fq", "json": "a.json"})), "in1");
        assert_eq!(bad(json!({"in1": "asm.fa", "json": "a.json"})), "in1"); // FASTA, not FASTQ
        assert_eq!(bad(json!({"in1": "-r1.fq", "in2": "-r1.fq", "json": "a.json"})), "in2");
        assert_eq!(bad(json!({"in1": "-r1.fq", "out2": "b.fq", "json": "a.json"})), "out2");
        assert_eq!(bad(json!({"in1": "-r1.fq", "in2": "r2.fq", "out1": "a.fq", "json": "a.json"})), "out2");
        assert_eq!(bad(json!({"in1": "-r1.fq", "correction": true, "json": "a.json"})), "correction");
        assert_eq!(bad(json!({"in1": "-r1.fq", "trim_poly_g": true, "disable_trim_poly_g": true, "json": "a.json"})), "trim_poly_g");
        assert_eq!(bad(json!({"in1": "-r1.fq", "cut_mean_quality": 40, "json": "a.json"})), "cut_mean_quality");
        assert_eq!(bad(json!({"in1": "-r1.fq", "adapter_sequence": "AGT;rm", "json": "a.json"})), "adapter_sequence");
        assert_eq!(bad(json!({"in1": "-r1.fq", "json": "../a.json"})), "json");
        assert_eq!(bad(json!({"in1": "-r1.fq", "json": "a.json", "html": "a.json"})), "html");
        assert_eq!(bad(json!({"in1": "-r1.fq", "json": "-r1.fq", "overwrite": true})), "json");
        // invented flags never reach fastp
        assert!(serde_json::from_value::<FastpParams>(json!({"in1": "x", "json": "a.json", "trim_qual": 20})).is_err());
        assert!(serde_json::from_value::<FastpParams>(json!({"in1": "x", "json": "a.json", "dont_overwrite": true})).is_err());
    }

    #[test]
    fn gzip_and_empty_inputs() {
        use std::io::Write;
        let d = tempfile::tempdir().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(b"@r\nACGT\n+\nIIII\n").unwrap();
        std::fs::write(d.path().join("r.fq.gz"), gz.finish().unwrap()).unwrap();
        std::fs::write(d.path().join("empty.fq"), "").unwrap();
        std::fs::write(d.path().join("r.fq.bz2"), b"BZh91AY&SY").unwrap();
        let p = |n: &str| d.path().join(n);
        assert!(v::sequence_file(&p("r.fq.gz"), "in1", "r.fq.gz", Seq::Fastq).is_ok());
        assert!(v::sequence_file(&p("r.fq.gz"), "g", "r.fq.gz", Seq::Fasta).unwrap_err().message.contains("FASTQ"));
        assert!(v::sequence_file(&p("empty.fq"), "in1", "empty.fq", Seq::Fastq).unwrap_err().message.contains("empty"));
        assert!(v::sequence_file(&p("r.fq.bz2"), "in1", "r.fq.bz2", Seq::Fastq).unwrap_err().message.contains("bzip2"));
    }

    #[test]
    fn bakta_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let mut cx = ctx(d.path());
        write(d.path(), "asm/s1.fna", ">c1\nACGT\n");
        write(d.path(), "reads.fq", "@a\nACGT\n+\nIIII\n");
        fake_bakta_db(&d.path().join("db"), 6);
        let p: BaktaParams = parse(json!({
            "genome": "asm/s1.fna", "db": "db", "outdir": "ann/s1", "genus": "Escherichia",
            "gram": "negative", "translation_table": 11, "skip": ["plot", "crispr", "plot"], "threads": 4
        }));
        std::fs::create_dir(d.path().join("ann")).unwrap();
        let inv = bakta(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "--db=db",
                "--output=ann/s1",
                "--force",
                "--prefix=s1",
                "--genus=Escherichia",
                "--translation-table=11",
                "--gram=-",
                "--threads=4",
                "--skip-plot",
                "--skip-crispr",
                "--",
                "asm/s1.fna",
            ]
        );
        assert!(inv.inputs.contains(&cx.root.join("db/version.json")));
        assert!(inv.expect.iter().any(|g| g[0].ends_with("ann/s1/s1.txt")));

        let bad = |cx: &Ctx, v: serde_json::Value| bakta(cx, &parse(v)).unwrap_err();
        assert_eq!(bad(&cx, json!({"genome": "reads.fq", "db": "db", "outdir": "o"})).field, "genome");
        assert_eq!(bad(&cx, json!({"genome": "asm/s1.fna", "outdir": "o"})).field, "db"); // no BAKTA_DB
        assert_eq!(
            bad(&cx, json!({"genome": "asm/s1.fna", "db": "db", "outdir": "o", "translation_table": 12})).field,
            "translation_table"
        );
        assert_eq!(bad(&cx, json!({"genome": "asm/s1.fna", "db": "db", "outdir": "o", "locus_tag": "a b"})).field, "locus_tag");
        assert_eq!(bad(&cx, json!({"genome": "asm/s1.fna", "db": "db", "outdir": "o", "prefix": "../x"})).field, "prefix");
        // the database is checked like Bakta does
        assert!(bad(&cx, json!({"genome": "asm/s1.fna", "db": "asm", "outdir": "o"})).message.contains("version.json"));
        fake_bakta_db(&d.path().join("db5"), 5);
        assert!(bad(&cx, json!({"genome": "asm/s1.fna", "db": "db5", "outdir": "o"})).message.contains("v6"));
        std::fs::remove_file(d.path().join("db/sorf.dmnd")).unwrap();
        assert!(bad(&cx, json!({"genome": "asm/s1.fna", "db": "db", "outdir": "o"})).message.contains("sorf.dmnd"));
        // $BAKTA_DB is used (and passed explicitly) when db is left out
        fake_bakta_db(&d.path().join("db6"), 6);
        cx.vars.insert("BAKTA_DB".into(), cx.root.join("db6").into());
        let inv = bakta(&cx, &parse(json!({"genome": "asm/s1.fna", "outdir": "o"}))).unwrap();
        assert_eq!(inv.args[0], format!("--db={}", cx.root.join("db6").display()));
        // a non-empty outdir needs overwrite
        write(d.path(), "o/old.txt", "x");
        assert_eq!(bad(&cx, json!({"genome": "asm/s1.fna", "outdir": "o"})).field, "outdir");
        assert!(serde_json::from_value::<BaktaParams>(json!({"genome": "g", "outdir": "o", "skip": ["everything"]})).is_err());
        assert!(serde_json::from_value::<BaktaParams>(json!({"genome": "g", "outdir": "o", "kingdom": "Bacteria"})).is_err());
    }

    #[test]
    fn checkm2_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "bins/a.fna", ">c\nACGT\n");
        write(d.path(), "bins/b.fna", ">c\nACGT\n");
        write(d.path(), "bins/notes.txt", "x");
        write(d.path(), "-odd.fa", ">c\nACGT\n");
        write(d.path(), "db/CheckM2_database/uniref100.KO.1.dmnd", "x");
        let p: Checkm2Params =
            parse(json!({"genome_dir": "bins", "outdir": "cm2", "db": "db", "lowmem": true, "threads": 2, "model": "both"}));
        let inv = checkm2(&cx, &p).unwrap();
        let dmnd = cx.root.join("db/CheckM2_database/uniref100.KO.1.dmnd");
        assert_eq!(
            inv.args,
            vec![
                "predict".to_string(),
                format!("--database_path={}", dmnd.display()),
                "--output-directory=cm2".into(),
                "--force".into(),
                "--threads=2".into(),
                "--allmodels".into(),
                "--lowmem".into(),
                "--input".into(),
                "bins".into(),
            ]
        );
        assert_eq!(inv.inputs.len(), 2);
        let p: Checkm2Params = parse(json!({"genomes": ["-odd.fa", "bins/a.fna"], "outdir": "cm2", "db": dmnd.to_str().unwrap()}));
        let inv = checkm2(&cx, &p).unwrap();
        assert_eq!(&inv.args[inv.args.len() - 3..], ["--input", "./-odd.fa", "bins/a.fna"]);

        let bad = |v: serde_json::Value| checkm2(&cx, &parse(v)).unwrap_err().field;
        assert_eq!(bad(json!({"outdir": "o", "db": "db"})), "genomes");
        assert_eq!(bad(json!({"genomes": ["bins/a.fna"], "genome_dir": "bins", "outdir": "o", "db": "db"})), "genomes");
        assert_eq!(bad(json!({"genome_dir": "bins", "extension": "fa", "outdir": "o", "db": "db"})), "genome_dir");
        assert_eq!(bad(json!({"genomes": ["bins/a.fna"], "outdir": "o", "db": "bins"})), "db");
        assert_eq!(bad(json!({"genomes": ["bins/a.fna"], "outdir": "o", "db": "db", "ttable": 7})), "ttable");
        assert_eq!(bad(json!({"genomes": ["bins/notes.txt"], "outdir": "o", "db": "db"})), "genomes");
        write(d.path(), "cm2/old", "x");
        assert_eq!(bad(json!({"genomes": ["bins/a.fna"], "outdir": "cm2", "db": "db"})), "outdir");
        assert!(serde_json::from_value::<Checkm2Params>(json!({"genomes": ["a"], "outdir": "o", "model": "fast"})).is_err());
    }

    #[test]
    fn gtdbtk_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "genomes/a.fna.gz", "");
        write(d.path(), "genomes/b.fa", ">c\nACGT\n");
        fake_gtdbtk_data(&d.path().join("gtdb/release232"), true);
        std::fs::create_dir(d.path().join("scratch")).unwrap();
        let p: GtdbtkClassifyWfParams = parse(json!({
            "genome_dir": "genomes", "extension": "fa", "out_dir": "gtdbtk", "data_path": "gtdb/release232",
            "cpus": 4, "pplacer_cpus": 1, "scratch_dir": "scratch", "force": true
        }));
        let inv = gtdbtk_classify_wf(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "classify_wf",
                "--genome_dir=genomes",
                "--out_dir=gtdbtk",
                "--extension=fa",
                "--cpus=4",
                "--pplacer_cpus=1",
                "--scratch_dir=scratch",
                "--force",
            ]
        );
        assert_eq!(inv.env["GTDBTK_DATA_PATH"], cx.root.join("gtdb/release232").to_str().unwrap());
        assert_eq!(inv.expect[0].len(), 2);
        assert!(inv.expect[0][0].ends_with("gtdbtk/classify/gtdbtk.bac120.summary.tsv"));

        let bad = |v: serde_json::Value| gtdbtk_classify_wf(&cx, &parse(v)).unwrap_err();
        let base = |extra: serde_json::Value| {
            let mut j = json!({"genome_dir": "genomes", "extension": "fa", "out_dir": "o", "data_path": "gtdb/release232"});
            j.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            j
        };
        // the empty a.fna.gz is caught before GTDB-Tk starts
        assert_eq!(bad(base(json!({"extension": "fna.gz"}))).field, "genome_dir");
        assert_eq!(bad(base(json!({"extension": "fasta"}))).field, "genome_dir");
        assert_eq!(bad(base(json!({"min_af": 1.5}))).field, "min_af");
        assert_eq!(bad(base(json!({"scratch_dir": "/tmp"}))).field, "scratch_dir");
        assert_eq!(bad(base(json!({"data_path": "genomes"}))).field, "data_path");
        fake_gtdbtk_data(&d.path().join("placement/release232"), false);
        let e = bad(base(json!({"data_path": "placement/release232"})));
        assert!(e.message.contains("skani") && e.message.contains("no option to skip"), "{}", e.message);
        // flags that don't exist in GTDB-Tk 2.8.0 are refused by the schema
        for invented in ["skip_ani_screen", "max_memory", "mash_db"] {
            assert!(serde_json::from_value::<GtdbtkClassifyWfParams>(base(json!({invented: true}))).is_err(), "{invented}");
        }
    }
}
