//! Third-party tools (fastp, Shovill, Bakta, CheckM2, GTDB-Tk, mlst, AMRFinderPlus): typed
//! parameters, checks before running, and their argument vectors.
//!
//! Every flag here exists in the pinned upstream version (see [`PINNED`]); the comment above each
//! builder names the source file it was read from. Nothing is passed that the pinned version
//! doesn't have. Options are written `--name=value` (fastp's cmdline.h, Python's argparse, Perl's
//! Getopt::Long and NCBI's ShellApplication all accept that form), so a value starting with `-` is
//! never read as a flag.

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

pub const PINNED: [Pinned; 7] = [
    Pinned { tool: "fastp", version: "1.4.0", bioconda: "fastp=1.4.0", url: "https://github.com/OpenGene/fastp" },
    Pinned { tool: "shovill", version: "1.4.2", bioconda: "shovill=1.4.2", url: "https://github.com/tseemann/shovill" },
    Pinned { tool: "bakta", version: "1.12.2", bioconda: "bakta=1.12.2", url: "https://github.com/oschwengers/bakta" },
    Pinned { tool: "checkm2", version: "1.1.0", bioconda: "checkm2=1.1.0", url: "https://github.com/chklovski/CheckM2" },
    Pinned { tool: "gtdbtk", version: "2.8.0", bioconda: "gtdbtk=2.8.0", url: "https://github.com/Ecogenomics/GTDBTk" },
    Pinned { tool: "mlst", version: "2.35.0", bioconda: "mlst=2.35.0", url: "https://github.com/tseemann/mlst" },
    Pinned { tool: "amrfinder", version: "4.2.7", bioconda: "ncbi-amrfinderplus=4.2.7", url: "https://github.com/ncbi/amr" },
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

// ---------- Shovill ----------
// Flags: Shovill v1.4.2 bin/shovill (`setOptions`, and the checks after it),
// https://github.com/tseemann/shovill/blob/v1.4.2/bin/shovill; same list as `shovill --help` of the
// bioconda shovill=1.4.2 package. Bioconda's current Shovill is 1.4.2, not 1.1.0: 1.4.x added
// --plasmid and changed --mincov's default to 2.
// Not offered: --opts (free text pasted into the assembler's shell command), --tmpdir (Shovill makes
// its own temporary folder), --check, --version, --help.

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ShovillAssembler {
    /// SPAdes (Shovill's default; needs SPAdes 3.14-3.x).
    Spades,
    /// SKESA: fast, conservative contigs.
    Skesa,
    Megahit,
    Velvet,
}

impl ShovillAssembler {
    fn name(self) -> &'static str {
        match self {
            ShovillAssembler::Spades => "spades",
            ShovillAssembler::Skesa => "skesa",
            ShovillAssembler::Megahit => "megahit",
            ShovillAssembler::Velvet => "velvet",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ShovillParams {
    /// Read 1 FASTQ (plain or gzip), paired-end Illumina.
    pub r1: String,
    /// Read 2 FASTQ (plain or gzip).
    pub r2: String,
    /// Output folder, inside the root. The assembly is `<outdir>/contigs.fa`; use one folder per sample.
    pub outdir: String,
    /// Allow an outdir that already has files (Shovill replaces files of the same name). Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// Assembler. Shovill's default is spades.
    #[serde(default)]
    pub assembler: Option<ShovillAssembler>,
    /// Estimated genome size, e.g. 2.8M or 5000000. Default: estimated from the reads with KMC.
    #[serde(default)]
    pub gsize: Option<String>,
    /// Sub-sample the reads to this depth; 0 turns sub-sampling off. Default 150.
    #[serde(default)]
    pub depth: Option<u32>,
    /// Drop contigs shorter than this; 0 = Shovill picks. Default 0.
    #[serde(default)]
    pub minlen: Option<u32>,
    /// Drop contigs with lower coverage than this; 0 = Shovill picks. Default 2.
    #[serde(default)]
    pub mincov: Option<f64>,
    /// Contig name format with one counter, e.g. `s1_%05d`. Default `contig%05d`.
    #[serde(default)]
    pub namefmt: Option<String>,
    /// Keep intermediate files (reads, BAM, assembler folder).
    #[serde(default)]
    pub keepfiles: bool,
    /// CPUs. Shovill's default is up to 8.
    #[serde(default)]
    pub cpus: Option<u32>,
    /// RAM limit in GB, at least 8 (Shovill 1.4.2 refuses less) and no more than the machine has. Default: up to 16.
    #[serde(default)]
    pub ram: Option<f64>,
    /// SPAdes plasmid mode (assembler spades only).
    #[serde(default)]
    pub plasmid: bool,
    /// K-mers to use, comma-separated, e.g. 31,55,77. Default: chosen from the read length.
    #[serde(default)]
    pub kmers: Option<String>,
    /// Trim adapters with Trimmomatic first (off by default; use fastp beforehand instead).
    #[serde(default)]
    pub trim: bool,
    /// Skip read error correction (Lighter).
    #[serde(default)]
    pub noreadcorr: bool,
    /// Skip read stitching (FLASH).
    #[serde(default)]
    pub nostitch: bool,
    /// Skip post-assembly correction (Pilon). Faster.
    #[serde(default)]
    pub nocorr: bool,
}

/// Shovill 1.4.2's `$MIN_RAM_GB`.
const SHOVILL_MIN_RAM_GB: f64 = 8.0;

/// Shovill pastes the absolute output folder unquoted into a shell command (`seqkit stats
/// $outdir/contigs.fa`), so the folder's path may only hold characters a shell leaves alone.
fn shell_safe_path(field: &str, shown: &str, p: &Path) -> Check<()> {
    let s = v::resolve(p).to_string_lossy().into_owned();
    if let Some(c) = s.chars().find(|c| !(c.is_ascii_alphanumeric() || "/._-+,=@%".contains(*c))) {
        return v::invalid(
            field,
            format!("{shown} resolves to {s}, which has {c:?}; Shovill passes this path to a shell unquoted, so use letters, digits and / . _ - + , = @ % only"),
        );
    }
    Ok(())
}

pub fn shovill(ctx: &Ctx, p: &ShovillParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("shovill");
    // shovill.log carries times and the machine's CPUs and RAM. SPAdes (Shovill's default) gave
    // different contigs.fa bytes in two identical runs in the real-tools workflow (same length within
    // the S. aureus check; run 43194785664); SKESA's were identical.
    inv.volatile = match p.assembler {
        None | Some(ShovillAssembler::Spades) => &["shovill.log", "contigs.fa"],
        Some(_) => &["shovill.log"],
    };
    let r1 = v::existing_file(cwd, "r1", &p.r1)?;
    v::sequence_file(&r1, "r1", &p.r1, Seq::Fastq)?;
    let r2 = v::existing_file(cwd, "r2", &p.r2)?;
    if r2 == r1 {
        return v::invalid("r2", "r2 is the same file as r1");
    }
    v::sequence_file(&r2, "r2", &p.r2, Seq::Fastq)?;
    inv.inputs.extend([r1, r2]);
    inv.opt("R1", &p.r1);
    inv.opt("R2", &p.r2);
    let outdir = v::output_dir(ctx, "outdir", &p.outdir, Some(p.overwrite), true)?;
    shell_safe_path("outdir", &p.outdir, &outdir)?;
    inv.outputs.push(outdir.clone());
    inv.opt("outdir", &p.outdir);
    // Shovill refuses any existing folder, even an empty one, without --force. irit-mcp has
    // already refused a non-empty one unless overwrite is true, and replay.sh reruns into it.
    inv.flag("force", true);
    inv.expect = vec![vec![outdir.join("contigs.fa")]];
    if let Some(a) = p.assembler {
        inv.opt("assembler", a.name());
    }
    if p.plasmid && p.assembler.is_some_and(|a| a != ShovillAssembler::Spades) {
        return v::invalid("plasmid", "plasmid mode needs assembler spades (Shovill 1.4.2 refuses it otherwise)");
    }
    if let Some(g) = &p.gsize {
        // Shovill's parse_genome_size: digits and dots, then an optional G, M or K.
        let num = g.trim_end_matches(['G', 'M', 'K', 'g', 'm', 'k']);
        let ok = !num.is_empty() && num.chars().all(|c| c.is_ascii_digit() || c == '.') && num.parse::<f64>().is_ok_and(|x| x > 0.0);
        if !ok || g.len() - num.len() > 1 {
            return v::invalid("gsize", format!("{g:?} is not a genome size like 2.8M, 4500K or 5000000"));
        }
        inv.opt("gsize", g);
    }
    if let Some(d) = p.depth {
        v::range("depth", d, 0, 100_000)?;
        inv.opt("depth", d);
    }
    if let Some(m) = p.minlen {
        inv.opt("minlen", m);
    }
    if let Some(c) = p.mincov {
        if !c.is_finite() || c < 0.0 {
            return v::invalid("mincov", "must be 0 or more");
        }
        inv.opt("mincov", c);
    }
    if let Some(f) = &p.namefmt {
        // Shovill wants a `%<digits>d` counter; anything else printf would read is refused too.
        let counters = f.matches('%').count();
        let ok = counters == 1
            && f.split_once('%').is_some_and(|(_, rest)| {
                let digits = rest.trim_start_matches(|c: char| c.is_ascii_digit());
                digits.len() < rest.len() && digits.starts_with('d')
            })
            && f.chars().all(|c| c.is_ascii_alphanumeric() || "_.-%".contains(c));
        if !ok {
            return v::invalid("namefmt", format!("{f:?} needs exactly one counter like %05d, and only letters, digits and _ . -"));
        }
        inv.opt("namefmt", f);
    }
    inv.flag("keepfiles", p.keepfiles);
    if let Some(c) = p.cpus {
        v::range("cpus", c, 1, 1024)?;
        inv.opt("cpus", c);
    }
    if let Some(r) = p.ram {
        if !r.is_finite() || r < SHOVILL_MIN_RAM_GB {
            return v::invalid("ram", format!("{r} GB is below the {SHOVILL_MIN_RAM_GB} GB Shovill 1.4.2 needs"));
        }
        inv.opt("ram", r);
    }
    inv.flag("plasmid", p.plasmid);
    if let Some(k) = &p.kmers {
        let ok = !k.is_empty() && k.split(',').all(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit()));
        if !ok {
            return v::invalid("kmers", format!("{k:?} is not a comma-separated list of k-mer sizes like 31,55,77"));
        }
        inv.opt("kmers", k);
    }
    for (name, on) in [("trim", p.trim), ("noreadcorr", p.noreadcorr), ("nostitch", p.nostitch), ("nocorr", p.nocorr)] {
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
    // Seen in the real-tools workflow: the log, GenBank/EMBL (LOCUS date), the JSON (run start and
    // end only) and the SVG plot differ between identical runs. The .txt, GFF3, TSV and FASTA
    // outputs don't, and are hash-checked.
    inv.volatile = &[".log", ".gbff", ".embl", ".json", ".svg"];
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

/// Files the split-tree placement reads (GTDB-Tk 2.8.0 `config/common.py`: BACKBONE_PPLACER_REF_PKG,
/// CLASS_LEVEL_TREE_MAPPING_FILE). Without them pplacer fails after the ANI screen and marker steps.
const GTDBTK_SPLIT_FILES: [&str; 2] = ["split/backbone/pplacer/gtdbtk_package_backbone.refpkg", "split/class_level/tree_mapping.tsv"];

fn gtdbtk_data(field: &str, shown: &str, dir: &Path, full_tree: bool) -> Check<PathBuf> {
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
    if !full_tree {
        let missing: Vec<&str> = GTDBTK_SPLIT_FILES.iter().copied().filter(|f| !dir.join(f).exists()).collect();
        if !missing.is_empty() {
            return v::invalid(
                field,
                format!(
                    "{shown} has no {}, which classify_wf needs to place genomes on the split tree. \
                     GTDB's mock database has an empty split/ and needs full_tree: true",
                    missing.join(" or ")
                ),
            );
        }
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
    inv.inputs.push(gtdbtk_data("data_path", &data_text, &data, p.full_tree)?);
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

/// An output whose name isotable's file search must recognise: it ends in `.tsv` and holds `tag`
/// (isotable/src/parse.rs `scan`: `mlst` for mlst, `amrfinder` for AMRFinderPlus, any case for the latter).
fn isotable_name(field: &str, value: &str, tag: &str, example: &str) -> Check<()> {
    let name = Path::new(value).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if !name.ends_with(".tsv") || !name.to_ascii_lowercase().contains(tag) {
        return v::invalid(field, format!("{value}: name it like {example} (ending in .tsv, with \"{tag}\" in the name) so isotable finds it"));
    }
    Ok(())
}

/// The sample part of `<sample>.<tag>.tsv` (or `_<tag>`, `-<tag>`), if the name has one.
fn sample_from_output(value: &str, tag: &str) -> Option<String> {
    let name = Path::new(value).file_name()?.to_string_lossy().into_owned();
    let stem = name.strip_suffix(".tsv")?;
    let lower = stem.to_ascii_lowercase();
    [".", "_", "-"].iter().find_map(|sep| {
        let suffix = format!("{sep}{tag}");
        lower.ends_with(&suffix).then(|| stem[..stem.len() - suffix.len()].to_string())
    }).filter(|s| !s.is_empty())
}

// ---------- mlst ----------
// Flags: mlst v2.35.0 bin/mlst (`setOptions` and the main block),
// https://github.com/tseemann/mlst/blob/v2.35.0/bin/mlst; same list as `mlst --help` of bioconda
// mlst=2.35.0. Not offered: --csv and --legacy (isotable reads the tab-separated layout), --blastdb and
// --datadir (the PubMLST data comes with the installed mlst, or from $MLST_DBDIR; see `mlst_data`), --fofn,
// --list/--longlist/--info/--check (they don't type anything), --exclude, --quiet, --debug.

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MlstParams {
    /// Assemblies, FASTA (plain or gzip). Usually one: `<outdir>/contigs.fa` from shovill.
    pub contigs: Vec<String>,
    /// Result table (tab-separated), inside the root. Name it `<sample>.mlst.tsv`: isotable finds files
    /// with "mlst" in the name ending in .tsv, and the sample part becomes the default label.
    pub outfile: String,
    /// Replace existing output files. Default false: they are refused.
    #[serde(default)]
    pub overwrite: bool,
    /// Name in the FILE column instead of the file's path; one assembly only. Default, for one
    /// assembly: the sample part of outfile (`s1.mlst.tsv` gives `s1`), so isotable joins the row to
    /// that sample rather than to `contigs`.
    #[serde(default)]
    pub label: Option<String>,
    /// Also write the results as JSON here (inside the root).
    #[serde(default)]
    pub json: Option<String>,
    /// Write novel alleles to this FASTA file (inside the root).
    #[serde(default)]
    pub novel: Option<String>,
    /// Use this PubMLST scheme instead of detecting it, e.g. saureus, ecoli_achtman_4. Sets minscore to 0.
    #[serde(default)]
    pub scheme: Option<String>,
    /// More detailed table: adds a header row and STATUS and SCORE columns (isotable reads it too).
    #[serde(default)]
    pub full: bool,
    /// % DNA identity of a full allele to call it similar (~). Default 95.
    #[serde(default)]
    pub minid: Option<f64>,
    /// % DNA coverage to report a partial allele (?). Default 50.
    #[serde(default)]
    pub mincov: Option<f64>,
    /// Minimum score out of 100 to match a scheme when detecting it. Default 50.
    #[serde(default)]
    pub minscore: Option<f64>,
    /// BLAST threads. Default 1.
    #[serde(default)]
    pub threads: Option<u32>,
}

/// The data mlst 2.35.0 types with: VERSION.txt (the data's date), scheme_species_map.tab, the BLAST
/// database of every allele (`blast/mlst.fa.*`, built from the allele files, which are not listed
/// separately) and each scheme's profile table (`pubmlst/<scheme>/<scheme>.txt`, allele numbers to
/// ST). About 180 files for 2.35.0's 162 schemes.
///
/// Where it is comes from mlst itself: `mlst --help` prints the resolved `--datadir` and `--blastdb`
/// defaults (bin/mlst: `$MLST_DBDIR`, else `$FindBin::RealBin/../db`), so a wrapper script in front of
/// mlst or a set `$MLST_DBDIR` is followed. If --help gives no paths, `db/` next to the real path
/// of the program (Bioconda's layout), or `$MLST_DBDIR`, is used.
pub fn mlst_data(program: &Path, ctx: &Ctx) -> Vec<PathBuf> {
    let mut cmd = std::process::Command::new(program);
    cmd.arg("--help").current_dir(&ctx.cwd).stdin(std::process::Stdio::null());
    if let Some(d) = ctx.var("MLST_DBDIR") {
        cmd.env("MLST_DBDIR", d);
    }
    let help = cmd.output().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()).unwrap_or_default();
    let (datadir, blastdb) = mlst_help_paths(&help);
    let (datadir, blastdb) = match (datadir, blastdb) {
        (Some(d), Some(b)) => (d, b),
        _ => {
            let db = match ctx.var("MLST_DBDIR").filter(|v| !v.is_empty()) {
                Some(d) => ctx.cwd.join(d),
                None => match std::fs::canonicalize(program).ok().and_then(|r| r.parent()?.parent().map(|p| p.join("db"))) {
                    Some(db) => db,
                    None => return Vec::new(),
                },
            };
            (db.join("pubmlst"), db.join("blast").join("mlst.fa"))
        }
    };
    let mut files = Vec::new();
    if let Some(db) = datadir.parent() {
        files.push(db.join("VERSION.txt"));
        files.push(db.join("scheme_species_map.tab"));
    }
    if let (Some(dir), Some(prefix)) = (blastdb.parent(), blastdb.file_name().and_then(|n| n.to_str()))
        && let Ok(rd) = std::fs::read_dir(dir)
    {
        files.extend(rd.flatten().map(|e| e.path()).filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.strip_prefix(prefix).is_some_and(|r| r.starts_with('.')))
        }));
    }
    if let Ok(rd) = std::fs::read_dir(&datadir) {
        for e in rd.flatten() {
            let profile = e.path().join(format!("{}.txt", e.file_name().to_string_lossy()));
            if profile.is_file() {
                files.push(profile);
            }
        }
    }
    files.retain(|p| p.is_file());
    files.sort();
    files.dedup();
    files
}

/// `--datadir` and `--blastdb` defaults from mlst 2.35.0's --help, printed as
/// `  --datadir [X]       PubMLST data folders [...] (default '/path')`.
fn mlst_help_paths(help: &str) -> (Option<PathBuf>, Option<PathBuf>) {
    let find = |opt: &str| {
        help.lines().find(|l| l.trim_start().starts_with(opt)).and_then(|l| {
            let (_, rest) = l.rsplit_once("(default '")?;
            let path = rest.strip_suffix("')")?;
            (!path.is_empty()).then(|| PathBuf::from(path))
        })
    };
    (find("--datadir"), find("--blastdb"))
}

pub fn mlst(ctx: &Ctx, p: &MlstParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("mlst");
    inv.bundled_data = Some(mlst_data);
    // mlst reads it from the environment it inherits; recorded so replay.sh types with the same data.
    if let Some(d) = ctx.var("MLST_DBDIR").filter(|v| !v.is_empty()) {
        inv.env.insert("MLST_DBDIR".into(), d.to_string_lossy().into_owned());
    }
    // Seen in the real-tools workflow: the JSON has the same values but its keys in another order
    // each run (mlst writes it with to_json without `canonical`, and Perl randomises hash order).
    // The table isotable reads is identical, and hash-checked.
    inv.volatile = &[".json"];
    v::non_empty_list("contigs", &p.contigs)?;
    for c in &p.contigs {
        let path = v::existing_file(cwd, "contigs", c)?;
        v::sequence_file(&path, "contigs", c, Seq::Fasta)?;
        inv.inputs.push(path);
    }
    isotable_name("outfile", &p.outfile, "mlst", "s1.mlst.tsv")?;
    let out = v::output_file(ctx, "outfile", &p.outfile, p.overwrite)?;
    inv.outputs.push(out.clone());
    inv.opt("outfile", &p.outfile);
    inv.expect = vec![vec![out.clone()]];
    let mut written = vec![("outfile", out)];
    for (field, val) in [("json", &p.json), ("novel", &p.novel)] {
        if let Some(path) = output_file_opt(ctx, &mut inv, field, val, p.overwrite, field)? {
            if field == "json" {
                inv.expect.push(vec![path.clone()]);
            }
            written.push((field, path));
        }
    }
    for (i, (field, a)) in written.iter().enumerate() {
        if written[..i].iter().any(|(_, b)| b == a) || inv.inputs.contains(a) {
            return v::invalid(field, format!("{} is used twice; give every input and output its own path", a.display()));
        }
    }
    let label = match (&p.label, p.contigs.len()) {
        (Some(_), n) if n > 1 => return v::invalid("label", "label works with one assembly only (mlst refuses it for several)"),
        (Some(l), _) => Some(l.clone()),
        (None, 1) => sample_from_output(&p.outfile, "mlst"),
        (None, _) => None,
    };
    if let Some(l) = &label {
        // It becomes the first column of a tab-separated table.
        if l.is_empty() || l.contains(['\t', ',']) || l.chars().any(char::is_control) {
            return v::invalid("label", format!("{l:?} can't be a table cell (no tabs, commas or control characters)"));
        }
        inv.opt("label", l);
    }
    if let Some(s) = &p.scheme {
        if s.is_empty() || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return v::invalid("scheme", format!("{s:?} is not a PubMLST scheme name like saureus or ecoli_achtman_4"));
        }
        inv.opt("scheme", s);
    }
    inv.flag("full", p.full);
    for (name, val) in [("minid", p.minid), ("mincov", p.mincov), ("minscore", p.minscore)] {
        if let Some(x) = val {
            v::percent(name, x)?;
            inv.opt(name, x);
        }
    }
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("threads", t);
    }
    let files: Vec<String> = p.contigs.iter().map(|c| list_item(c)).collect();
    inv.positionals(&files);
    Ok(inv)
}

// ---------- AMRFinderPlus ----------
// Flags: AMRFinderPlus amrfinder_v4.2.7 amrfinder.cpp (`addKey`/`addFlag` calls and the checks in
// `shellBody`), https://github.com/ncbi/amr/blob/amrfinder_v4.2.7/amrfinder.cpp; same list as
// `amrfinder --help` of bioconda ncbi-amrfinderplus=4.2.7. Database files: amrfinder_update.cpp (what it
// downloads) and amrfinder_index.cpp (what it builds). Not offered: --update/--force_update (they change
// the database outside the record: fetch it with dblock instead), --blast_bin/--hmmer_bin, --pgap,
// --gpipe_org, --parm (for NCBI's testing), --debug, --log, --quiet, --list_organisms, --database_version,
// --nucleotide_flank5_output/_size.

/// amrfinder.cpp 4.2.7 `dataVer_min`: older data is refused.
const AMRFINDER_DATA_MIN: &str = "2025-09-22.2";

/// Files amrfinder_update 4.2.7 downloads besides the per-organism AMR_DNA-* files and changes.txt.
const AMRFINDER_DB_FILES: [&str; 11] = [
    "AMR.LIB",
    "AMRProt.fa",
    "AMRProt-mutation.tsv",
    "AMRProt-suppress.tsv",
    "AMRProt-susceptible.fa",
    "AMRProt-susceptible.tsv",
    "AMR_CDS.fa",
    "database_format_version.txt",
    "fam.tsv",
    "taxgroup.tsv",
    "version.txt",
];

/// Indexes amrfinder_index 4.2.7 builds (hmmpress, makeblastdb) that amrfinder needs: AMRProt.fa.phr
/// is amrfinder.cpp's own check; the other two are the HMM and nucleotide BLAST databases it searches.
const AMRFINDER_INDEX_FILES: [&str; 3] = ["AMRProt.fa.phr", "AMR.LIB.h3m", "AMR_CDS.fa.nin"];

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AmrAnnotationFormat {
    Bakta,
    Genbank,
    Microscope,
    Patric,
    Pgap,
    Prodigal,
    Prokka,
    Pseudomonasdb,
    Rast,
    Standard,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AmrfinderParams {
    /// Assembly (nucleotide FASTA, plain or gzip). Give this, protein, or both (both need gff).
    #[serde(default)]
    pub nucleotide: Option<String>,
    /// Proteins (FASTA), e.g. Bakta's `<prefix>.faa`.
    #[serde(default)]
    pub protein: Option<String>,
    /// GFF with the proteins' locations in `nucleotide`, e.g. Bakta's `<prefix>.gff3`; needs protein.
    #[serde(default)]
    pub gff: Option<String>,
    /// Which tool wrote the GFF. AMRFinderPlus's default is genbank; use bakta for Bakta's GFF3.
    #[serde(default)]
    pub annotation_format: Option<AmrAnnotationFormat>,
    /// AMRFinderPlus database folder (holding version.txt, indexed). Default: $AMRFINDER_DB as the server started with.
    /// dblock_env shows the path of a dblock-managed one (amrfinderplus-db-4.2-2026-10-29.1).
    #[serde(default)]
    pub database: Option<String>,
    /// Taxonomy group for point mutations and organism-specific genes, e.g. Escherichia,
    /// Staphylococcus_aureus, Klebsiella_pneumoniae. Checked against the database's taxgroup.tsv.
    #[serde(default)]
    pub organism: Option<String>,
    /// Also report the "plus" genes (virulence, stress, ...).
    #[serde(default)]
    pub plus: bool,
    /// Report proteins common to the organism too; needs organism and plus.
    #[serde(default)]
    pub report_common: bool,
    /// Report all equally scoring BLAST and HMM matches.
    #[serde(default)]
    pub report_all_equal: bool,
    /// Minimum identity, 0-1, or -1 for the curated cut-offs (the default).
    #[serde(default)]
    pub ident_min: Option<f64>,
    /// Minimum coverage of the reference protein, 0-1. Default 0.5.
    #[serde(default)]
    pub coverage_min: Option<f64>,
    /// Genetic code for translated searches. Default 11.
    #[serde(default)]
    pub translation_table: Option<u8>,
    /// Text for a first "Name" column on every row, e.g. the sample name.
    #[serde(default)]
    pub name: Option<String>,
    /// Report, inside the root. Name it `<sample>.amrfinder.tsv` so isotable finds it and takes the sample name from it.
    pub output: String,
    /// Also write every mutation position checked here (needs organism).
    #[serde(default)]
    pub mutation_all: Option<String>,
    /// Write the reported proteins as FASTA here (needs protein).
    #[serde(default)]
    pub protein_output: Option<String>,
    /// Write the reported nucleotide sequences as FASTA here (needs nucleotide).
    #[serde(default)]
    pub nucleotide_output: Option<String>,
    /// Print the hierarchy node (family) of each hit.
    #[serde(default)]
    pub print_node: bool,
    /// Threads. Default 4.
    #[serde(default)]
    pub threads: Option<u32>,
    /// Replace existing output files. Default false: they are refused.
    #[serde(default)]
    pub overwrite: bool,
}

/// `2025-09-22.2` as (date, release) for comparing.
fn data_version(s: &str) -> Option<(String, u32)> {
    let (date, n) = s.trim().split_once('.')?;
    let ok = date.len() == 10 && date.chars().enumerate().all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() });
    if !ok {
        return None;
    }
    Some((date.to_string(), n.parse().ok()?))
}

/// `4.2.0` as numbers.
fn dotted(s: &str) -> Option<Vec<u64>> {
    s.trim().split('.').map(|x| x.parse().ok()).collect()
}

/// Organisms the database knows: first columns of taxgroup.tsv, AMRProt-mutation.tsv and
/// AMRProt-susceptible.tsv, as amrfinder.cpp's `db2organisms` reads them.
fn amrfinder_organisms(dir: &Path) -> Vec<String> {
    let mut out: Vec<String> = ["taxgroup.tsv", "AMRProt-mutation.tsv", "AMRProt-susceptible.tsv"]
        .iter()
        .filter_map(|f| std::fs::read_to_string(dir.join(f)).ok())
        .flat_map(|t| t.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).filter_map(|l| l.split('\t').next()).map(str::to_string).collect::<Vec<_>>())
        .collect();
    out.sort();
    out.dedup();
    out
}

/// Checks an AMRFinderPlus database folder the way amrfinder 4.2.7 does before it searches:
/// present, indexed, a format this version reads and data new enough. Returns version.txt and
/// the data version.
fn amrfinder_db(field: &str, shown: &str, dir: &Path, organism: Option<&str>, nucleotide: bool) -> Check<(PathBuf, String)> {
    if !dir.is_dir() {
        return v::invalid(field, format!("{shown} is not a folder"));
    }
    let version_file = dir.join("version.txt");
    let version = std::fs::read_to_string(&version_file).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    let Some(version) = version else {
        return v::invalid(
            field,
            format!("{shown} has no version.txt, so it is not an AMRFinderPlus database folder (amrfinder_update puts it in a dated folder with a `latest` link)"),
        );
    };
    let missing: Vec<&str> = AMRFINDER_DB_FILES.iter().copied().filter(|f| !dir.join(f).is_file()).collect();
    if !missing.is_empty() {
        return v::invalid(field, format!("{shown} is missing {} (incomplete download?)", missing.join(", ")));
    }
    if let Some(idx) = AMRFINDER_INDEX_FILES.iter().find(|f| !dir.join(f).is_file()) {
        return v::invalid(
            field,
            format!("{shown} is not indexed (no {idx}); run amrfinder_index on it, or fetch it with dblock (amrfinderplus-db-4.2-2026-10-29.1), which does"),
        );
    }
    let format_text = std::fs::read_to_string(dir.join("database_format_version.txt")).unwrap_or_default();
    match (dotted(&format_text), dotted(pinned("amrfinder").unwrap().version)) {
        (Some(need), Some(have)) if need > have => {
            return v::invalid(field, format!("{shown} is format {}, which needs AMRFinderPlus {} or newer; irit-mcp is checked against 4.2.7", format_text.trim(), format_text.trim()));
        }
        (Some(_), _) => {}
        (None, _) => return v::invalid(field, format!("{shown}/database_format_version.txt is not a version like 4.2.0")),
    }
    match (data_version(&version), data_version(AMRFINDER_DATA_MIN)) {
        (Some(got), Some(min)) if got < min => {
            return v::invalid(field, format!("{shown} is data version {version}; AMRFinderPlus 4.2.7 needs {AMRFINDER_DATA_MIN} or newer"));
        }
        (Some(_), _) => {}
        (None, _) => return v::invalid(field, format!("{shown}/version.txt holds {version:?}, not a version like 2026-10-29.1")),
    }
    if let Some(org) = organism {
        let known = amrfinder_organisms(dir);
        if !known.iter().any(|k| k == org) {
            return v::invalid("organism", format!("{org:?} is not an organism in {shown}; it knows {}", known.join(", ")));
        }
        // amrfinder.cpp: with a nucleotide input, an organism with its own DNA file needs that file's BLAST index.
        let dna = dir.join(format!("AMR_DNA-{org}.fa"));
        let indexed = ["ndb", "nin"].iter().any(|ext| dir.join(format!("AMR_DNA-{org}.fa.{ext}")).is_file());
        if nucleotide && dna.is_file() && !indexed {
            return v::invalid(field, format!("{shown} has AMR_DNA-{org}.fa but no BLAST index for it; run amrfinder_index on the folder"));
        }
    }
    Ok((version_file, version))
}

pub fn amrfinder(ctx: &Ctx, p: &AmrfinderParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = external("amrfinder");
    if p.nucleotide.is_none() && p.protein.is_none() {
        return v::invalid("nucleotide", "give nucleotide (an assembly), protein, or both");
    }
    for (field, val, flag) in [("protein", &p.protein, "protein"), ("nucleotide", &p.nucleotide, "nucleotide")] {
        if let Some(f) = val {
            let path = v::existing_file(cwd, field, f)?;
            v::sequence_file(&path, field, f, Seq::Fasta)?;
            inv.inputs.push(path);
            inv.opt(flag, f);
        }
    }
    match (&p.gff, p.protein.is_some(), p.nucleotide.is_some()) {
        (Some(_), false, _) => return v::invalid("gff", "gff gives the proteins' locations, so it needs protein"),
        (None, true, true) => return v::invalid("gff", "with both protein and nucleotide, AMRFinderPlus needs gff to link them"),
        (Some(g), true, _) => {
            inv.inputs.push(v::existing_file(cwd, "gff", g)?);
            inv.opt("gff", g);
        }
        (None, _, _) => {}
    }
    if let Some(a) = p.annotation_format {
        if p.gff.is_none() {
            return v::invalid("annotation_format", "annotation_format describes the gff; give gff too");
        }
        inv.opt("annotation_format", serde_json::to_value(a).unwrap().as_str().unwrap());
    }
    let (db_text, db) = db_location(ctx, "database", &p.database, "AMRFINDER_DB")?;
    if let Some(o) = &p.organism {
        v::text("organism", o)?;
    }
    // version.txt only: the database itself is checked by dblock_verify.
    let (version_file, _) = amrfinder_db("database", &db_text, &db, p.organism.as_deref(), p.nucleotide.is_some())?;
    inv.inputs.push(version_file);
    inv.opt("database", &db_text);
    if let Some(o) = &p.organism {
        inv.opt("organism", o);
    }
    if p.report_common && (p.organism.is_none() || !p.plus) {
        return v::invalid("report_common", "report_common needs organism and plus (amrfinder refuses it otherwise)");
    }
    inv.flag("plus", p.plus);
    inv.flag("report_common", p.report_common);
    inv.flag("report_all_equal", p.report_all_equal);
    if let Some(x) = p.ident_min {
        if !(x == -1.0 || (0.0..=1.0).contains(&x)) {
            return v::invalid("ident_min", format!("{x} is not -1 or between 0 and 1"));
        }
        inv.opt("ident_min", x);
    }
    if let Some(x) = p.coverage_min {
        if !x.is_finite() || !(0.0..=1.0).contains(&x) {
            return v::invalid("coverage_min", format!("{x} is not between 0 and 1"));
        }
        inv.opt("coverage_min", x);
    }
    if let Some(t) = p.translation_table {
        v::range("translation_table", t, 1, 33)?;
        inv.opt("translation_table", t);
    }
    if let Some(n) = &p.name {
        v::text("name", n)?;
        inv.opt("name", n);
    }
    isotable_name("output", &p.output, "amrfinder", "s1.amrfinder.tsv")?;
    let out = v::output_file(ctx, "output", &p.output, p.overwrite)?;
    inv.outputs.push(out.clone());
    inv.opt("output", &p.output);
    inv.expect = vec![vec![out.clone()]];
    let mut written = vec![("output", out)];
    // amrfinder.cpp refuses the last two without their input; mutation_all without organism only
    // warns that no point mutations are screened, so the file would say nothing.
    for (field, val, have, need) in [
        ("mutation_all", &p.mutation_all, p.organism.is_some(), "organism"),
        ("protein_output", &p.protein_output, p.protein.is_some(), "protein"),
        ("nucleotide_output", &p.nucleotide_output, p.nucleotide.is_some(), "nucleotide"),
    ] {
        if val.is_some() && !have {
            return v::invalid(field, format!("{field} needs {need}"));
        }
        if let Some(path) = output_file_opt(ctx, &mut inv, field, val, p.overwrite, field)? {
            written.push((field, path));
        }
    }
    for (i, (field, a)) in written.iter().enumerate() {
        if written[..i].iter().any(|(_, b)| b == a) || inv.inputs.contains(a) {
            return v::invalid(field, format!("{} is used twice; give every input and output its own path", a.display()));
        }
    }
    inv.flag("print_node", p.print_node);
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 1024)?;
        inv.opt("threads", t);
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
        std::fs::create_dir_all(dir.join(GTDBTK_SPLIT_FILES[0])).unwrap();
        write(dir, GTDBTK_SPLIT_FILES[1], "");
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
        // an empty split/ (GTDB's mock database) is refused unless full_tree is set
        fake_gtdbtk_data(&d.path().join("mock/release232"), true);
        std::fs::remove_dir_all(d.path().join("mock/release232/split")).unwrap();
        std::fs::create_dir(d.path().join("mock/release232/split")).unwrap();
        let e = bad(base(json!({"data_path": "mock/release232"})));
        assert!(e.field == "data_path" && e.message.contains("full_tree"), "{}", e.message);
        assert!(gtdbtk_classify_wf(&cx, &parse(base(json!({"data_path": "mock/release232", "full_tree": true})))).is_ok());
        fake_gtdbtk_data(&d.path().join("placement/release232"), false);
        let e = bad(base(json!({"data_path": "placement/release232"})));
        assert!(e.message.contains("skani") && e.message.contains("no option to skip"), "{}", e.message);
        // flags that don't exist in GTDB-Tk 2.8.0 are refused by the schema
        for invented in ["skip_ani_screen", "max_memory", "mash_db"] {
            assert!(serde_json::from_value::<GtdbtkClassifyWfParams>(base(json!({invented: true}))).is_err(), "{invented}");
        }
    }

    #[test]
    fn shovill_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "reads/s1_R1.fq.gz.txt", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "reads/s1_R1.fq", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "reads/s1_R2.fq", "@a\nACGT\n+\nIIII\n");
        write(d.path(), "asm.fa", ">c\nACGT\n");
        std::fs::create_dir(d.path().join("asm")).unwrap();
        let p: ShovillParams = parse(json!({
            "r1": "reads/s1_R1.fq", "r2": "reads/s1_R2.fq", "outdir": "asm/s1", "assembler": "skesa", "gsize": "2.8M",
            "depth": 40, "cpus": 4, "ram": 8, "nocorr": true, "namefmt": "s1_%05d", "kmers": "31,55"
        }));
        let inv = shovill(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "--R1=reads/s1_R1.fq",
                "--R2=reads/s1_R2.fq",
                "--outdir=asm/s1",
                "--force",
                "--assembler=skesa",
                "--gsize=2.8M",
                "--depth=40",
                "--namefmt=s1_%05d",
                "--cpus=4",
                "--ram=8",
                "--kmers=31,55",
                "--nocorr",
            ]
        );
        // SKESA's contigs are reproducible; SPAdes's (also the default) are not.
        assert_eq!(inv.volatile, ["shovill.log"]);
        let spades: ShovillParams = parse(json!({"r1": "reads/s1_R1.fq", "r2": "reads/s1_R2.fq", "outdir": "asm/s2"}));
        assert_eq!(shovill(&cx, &spades).unwrap().volatile, ["shovill.log", "contigs.fa"]);
        assert!(inv.expect[0][0].ends_with("asm/s1/contigs.fa"));
        assert_eq!(inv.inputs.len(), 2);

        let base = json!({"r1": "reads/s1_R1.fq", "r2": "reads/s1_R2.fq", "outdir": "asm/x"});
        let bad = |extra: serde_json::Value| {
            let mut j = base.clone();
            j.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            shovill(&cx, &parse(j)).unwrap_err()
        };
        assert_eq!(bad(json!({"r2": "reads/s1_R1.fq"})).field, "r2");
        assert_eq!(bad(json!({"r1": "asm.fa"})).field, "r1"); // FASTA, not reads
        assert_eq!(bad(json!({"ram": 4})).field, "ram");
        assert_eq!(bad(json!({"gsize": "big"})).field, "gsize");
        assert_eq!(bad(json!({"gsize": "2.8MM"})).field, "gsize");
        assert_eq!(bad(json!({"namefmt": "contig"})).field, "namefmt");
        assert_eq!(bad(json!({"namefmt": "c%s%05d"})).field, "namefmt");
        assert_eq!(bad(json!({"kmers": "31;rm -rf"})).field, "kmers");
        assert_eq!(bad(json!({"plasmid": true, "assembler": "skesa"})).field, "plasmid");
        assert_eq!(bad(json!({"outdir": "../asm"})).field, "outdir");
        assert_eq!(bad(json!({"outdir": "asm/s 1"})).field, "outdir"); // reaches a shell unquoted
        write(d.path(), "asm/full/contigs.fa", ">c\nA\n");
        assert_eq!(bad(json!({"outdir": "asm/full"})).field, "outdir");
        assert!(shovill(&cx, &parse(json!({"r1": "reads/s1_R1.fq", "r2": "reads/s1_R2.fq", "outdir": "asm/full", "overwrite": true}))).is_ok());
        // --opts would paste free text into a shell command; it is not a parameter
        for invented in ["opts", "tmpdir", "check"] {
            let mut j = base.clone();
            j[invented] = json!("x");
            assert!(serde_json::from_value::<ShovillParams>(j).is_err(), "{invented}");
        }
    }

    #[test]
    #[cfg(unix)]
    fn mlst_data_follows_the_program_or_mlst_dbdir() {
        let d = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(d.path()).unwrap();
        // Bioconda's layout: the real script in <prefix>/bin, data in <prefix>/db; PATH may hold a symlink.
        write(&root, "pkg/bin/mlst", "#!/bin/sh\n");
        for f in ["VERSION.txt", "scheme_species_map.tab", "blast/mlst.fa.nsq", "blast/mlst.fa.nin", "blast/other.txt",
            "pubmlst/saureus/saureus.txt", "pubmlst/saureus/arcC.tfa", "pubmlst/ecoli/ecoli.txt", "pubmlst/stray.txt"] {
            write(&root, &format!("pkg/db/{f}"), "x\n");
        }
        std::fs::create_dir(root.join("path")).unwrap();
        std::os::unix::fs::symlink(root.join("pkg/bin/mlst"), root.join("path/mlst")).unwrap();
        let mut cx = ctx(&root);
        let names = |v: Vec<PathBuf>| v.iter().map(|p| p.strip_prefix(&root).unwrap().display().to_string()).collect::<Vec<_>>();
        let want = ["pkg/db/VERSION.txt", "pkg/db/blast/mlst.fa.nin", "pkg/db/blast/mlst.fa.nsq", "pkg/db/pubmlst/ecoli/ecoli.txt",
            "pkg/db/pubmlst/saureus/saureus.txt", "pkg/db/scheme_species_map.tab"];
        assert_eq!(names(mlst_data(&root.join("path/mlst"), &cx)), want);
        // $MLST_DBDIR wins, relative to the working folder, and is passed on and recorded.
        write(&root, "mine/VERSION.txt", "2026-01-01\n");
        cx.vars.insert("MLST_DBDIR".into(), "mine".into());
        assert_eq!(names(mlst_data(&root.join("path/mlst"), &cx)), ["mine/VERSION.txt"]);
        write(&root, "asm/contigs.fa", ">c\nACGT\n");
        std::fs::create_dir(root.join("typing")).unwrap();
        let p: MlstParams = parse(json!({"contigs": ["asm/contigs.fa"], "outfile": "typing/s1.mlst.tsv"}));
        let inv = mlst(&cx, &p).unwrap();
        assert_eq!(inv.env.get("MLST_DBDIR").map(String::as_str), Some("mine"));
        assert!(inv.bundled_data.is_some());
        // A program whose data can't be found records none (the call still runs).
        assert!(mlst_data(&root.join("nowhere/mlst"), &ctx(&root)).is_empty());
        // A wrapper script in front of mlst (as in the real-tools workflow): the paths come from
        // mlst's own --help, not from where the wrapper is.
        use std::os::unix::fs::PermissionsExt;
        let help = format!(
            "#!/bin/sh\necho 'OPTIONS'\necho \"  --blastdb [X]       BLAST database prefix [default via MLST_DBDIR/blast/mlst.fa; --blastdb overrides] (default '{0}/pkg/db/blast/mlst.fa')\"\necho \"  --datadir [X]       PubMLST data folders [default via MLST_DBDIR/pubmlst; --datadir overrides] (default '{0}/pkg/db/pubmlst')\"\n",
            root.display()
        );
        write(&root, "wrap/mlst", &help);
        std::fs::set_permissions(root.join("wrap/mlst"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(names(mlst_data(&root.join("wrap/mlst"), &ctx(&root))), want);
        assert_eq!(mlst_help_paths("  --datadir [X] x (default '')\n"), (None, None));
    }

    #[test]
    fn mlst_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        write(d.path(), "asm/s1/contigs.fa", ">c\nACGT\n");
        write(d.path(), "asm/s2/contigs.fa", ">c\nACGT\n");
        write(d.path(), "reads.fq", "@a\nACGT\n+\nIIII\n");
        std::fs::create_dir(d.path().join("typing")).unwrap();
        let p: MlstParams = parse(json!({
            "contigs": ["asm/s1/contigs.fa"], "outfile": "typing/s1.mlst.tsv", "json": "typing/s1.mlst.json",
            "scheme": "saureus", "minid": 90, "threads": 2
        }));
        let inv = mlst(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "--outfile=typing/s1.mlst.tsv",
                "--json=typing/s1.mlst.json",
                "--label=s1",
                "--scheme=saureus",
                "--minid=90",
                "--threads=2",
                "--",
                "asm/s1/contigs.fa",
            ]
        );
        assert_eq!(inv.expect.len(), 2);
        // several assemblies: no label, each row named by its path
        let p: MlstParams = parse(json!({"contigs": ["asm/s1/contigs.fa", "asm/s2/contigs.fa"], "outfile": "typing/all_mlst.tsv", "full": true}));
        let inv = mlst(&cx, &p).unwrap();
        assert_eq!(inv.args, ["--outfile=typing/all_mlst.tsv", "--full", "--", "asm/s1/contigs.fa", "asm/s2/contigs.fa"]);
        assert_eq!(sample_from_output("typing/S-7_mlst.tsv", "mlst").as_deref(), Some("S-7"));
        assert_eq!(sample_from_output("typing/mlst.tsv", "mlst"), None);
        assert_eq!(sample_from_output("x/s1.AMRFinder.tsv", "amrfinder").as_deref(), Some("s1"));

        let bad = |v: serde_json::Value| mlst(&cx, &parse(v)).unwrap_err().field;
        assert_eq!(bad(json!({"contigs": [], "outfile": "typing/a.mlst.tsv"})), "contigs");
        assert_eq!(bad(json!({"contigs": ["reads.fq"], "outfile": "typing/a.mlst.tsv"})), "contigs");
        assert_eq!(bad(json!({"contigs": ["asm/s1/contigs.fa"], "outfile": "typing/s1.tsv"})), "outfile"); // isotable wouldn't find it
        assert_eq!(bad(json!({"contigs": ["asm/s1/contigs.fa"], "outfile": "typing/s1.mlst.csv"})), "outfile");
        assert_eq!(bad(json!({"contigs": ["asm/s1/contigs.fa", "asm/s2/contigs.fa"], "outfile": "typing/a.mlst.tsv", "label": "x"})), "label");
        assert_eq!(bad(json!({"contigs": ["asm/s1/contigs.fa"], "outfile": "typing/a.mlst.tsv", "label": "a\tb"})), "label");
        assert_eq!(bad(json!({"contigs": ["asm/s1/contigs.fa"], "outfile": "typing/a.mlst.tsv", "scheme": "../x"})), "scheme");
        assert_eq!(bad(json!({"contigs": ["asm/s1/contigs.fa"], "outfile": "typing/a.mlst.tsv", "minscore": 101})), "minscore");
        assert_eq!(bad(json!({"contigs": ["asm/s1/contigs.fa"], "outfile": "typing/a.mlst.tsv", "json": "typing/a.mlst.tsv"})), "json");
        assert!(serde_json::from_value::<MlstParams>(json!({"contigs": ["a"], "outfile": "a.mlst.tsv", "csv": true})).is_err());
        assert!(serde_json::from_value::<MlstParams>(json!({"contigs": ["a"], "outfile": "a.mlst.tsv", "datadir": "x"})).is_err());
    }

    /// The files amrfinder_update 4.2.7 downloads and amrfinder_index's outputs, as empty stand-ins.
    pub fn fake_amrfinder_db(dir: &Path, version: &str, format: &str, indexed: bool) {
        for f in AMRFINDER_DB_FILES {
            write(dir, f, "");
        }
        write(dir, "version.txt", &format!("{version}\n"));
        write(dir, "database_format_version.txt", &format!("{format}\n"));
        write(dir, "taxgroup.tsv", "#taxgroup\tgpipe_taxgroup\tnumber_of_nucl_ref_genes\nEscherichia\tEscherichia_coli_Shigella\t6\nStaphylococcus_aureus\tStaphylococcus_aureus\t2\nPseudomonas_aeruginosa\tPseudomonas_aeruginosa\t0\n");
        write(dir, "AMR_DNA-Escherichia.fa", ">x\nA\n");
        write(dir, "AMR_DNA-Staphylococcus_aureus.fa", ">x\nA\n");
        if indexed {
            for f in AMRFINDER_INDEX_FILES {
                write(dir, f, "");
            }
            write(dir, "AMR_DNA-Escherichia.fa.ndb", "");
        }
    }

    #[test]
    fn amrfinder_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let mut cx = ctx(d.path());
        write(d.path(), "asm/s1/contigs.fa", ">c\nACGT\n");
        write(d.path(), "ann/s1.faa", ">p\nMKV\n");
        write(d.path(), "ann/s1.gff3", "##gff-version 3\n");
        std::fs::create_dir(d.path().join("typing")).unwrap();
        fake_amrfinder_db(&d.path().join("amrdb"), "2026-10-29.1", "4.2.0", true);
        let p: AmrfinderParams = parse(json!({
            "nucleotide": "asm/s1/contigs.fa", "database": "amrdb", "organism": "Escherichia", "plus": true,
            "output": "typing/s1.amrfinder.tsv", "threads": 4, "name": "s1", "ident_min": -1, "mutation_all": "typing/s1.mut.tsv"
        }));
        let inv = amrfinder(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            [
                "--nucleotide=asm/s1/contigs.fa",
                "--database=amrdb",
                "--organism=Escherichia",
                "--plus",
                "--ident_min=-1",
                "--name=s1",
                "--output=typing/s1.amrfinder.tsv",
                "--mutation_all=typing/s1.mut.tsv",
                "--threads=4",
            ]
        );
        assert!(inv.inputs.contains(&cx.root.join("amrdb/version.txt")));
        assert_eq!(inv.outputs.len(), 2);
        // proteins with their GFF (Bakta's), and the database from $AMRFINDER_DB
        cx.vars.insert("AMRFINDER_DB".into(), cx.root.join("amrdb").into());
        let p: AmrfinderParams = parse(json!({
            "protein": "ann/s1.faa", "gff": "ann/s1.gff3", "nucleotide": "asm/s1/contigs.fa", "annotation_format": "bakta",
            "output": "typing/s1_amrfinder.tsv"
        }));
        let inv = amrfinder(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            vec![
                "--protein=ann/s1.faa".to_string(),
                "--nucleotide=asm/s1/contigs.fa".into(),
                "--gff=ann/s1.gff3".into(),
                "--annotation_format=bakta".into(),
                format!("--database={}", cx.root.join("amrdb").display()),
                "--output=typing/s1_amrfinder.tsv".into(),
            ]
        );

        let base = json!({"nucleotide": "asm/s1/contigs.fa", "database": "amrdb", "output": "typing/s1.amrfinder.tsv"});
        let bad = |extra: serde_json::Value| {
            let mut j = base.clone();
            j.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
            j.as_object_mut().unwrap().retain(|_, v| !v.is_null());
            amrfinder(&cx, &parse(j)).unwrap_err()
        };
        assert_eq!(bad(json!({"nucleotide": null})).field, "nucleotide");
        assert_eq!(bad(json!({"protein": "ann/s1.faa"})).field, "gff"); // both inputs need the GFF
        assert_eq!(bad(json!({"gff": "ann/s1.gff3"})).field, "gff"); // a GFF needs proteins
        assert_eq!(bad(json!({"annotation_format": "prokka"})).field, "annotation_format");
        assert_eq!(bad(json!({"organism": "Escherichia_coli"})).field, "organism");
        assert!(bad(json!({"organism": "Escherichia_coli"})).message.contains("Staphylococcus_aureus"));
        assert_eq!(bad(json!({"report_common": true, "organism": "Escherichia"})).field, "report_common"); // needs plus
        assert_eq!(bad(json!({"ident_min": 1.5})).field, "ident_min");
        assert_eq!(bad(json!({"coverage_min": -0.1})).field, "coverage_min");
        assert_eq!(bad(json!({"output": "typing/s1.tsv"})).field, "output");
        assert_eq!(bad(json!({"mutation_all": "typing/m.tsv"})).field, "mutation_all"); // needs organism
        assert_eq!(bad(json!({"protein_output": "typing/p.faa"})).field, "protein_output");
        assert_eq!(bad(json!({"nucleotide_output": "typing/s1.amrfinder.tsv"})).field, "nucleotide_output");
        assert_eq!(bad(json!({"database": "asm"})).field, "database");
        // the database is checked the way amrfinder 4.2.7 does
        fake_amrfinder_db(&d.path().join("raw"), "2026-10-29.1", "4.2.0", false);
        assert!(bad(json!({"database": "raw"})).message.contains("not indexed"));
        fake_amrfinder_db(&d.path().join("old"), "2025-07-16.1", "4.0.0", true);
        assert!(bad(json!({"database": "old"})).message.contains("2025-09-22.2"));
        fake_amrfinder_db(&d.path().join("new"), "2027-01-01.1", "4.4.0", true);
        assert!(bad(json!({"database": "new"})).message.contains("format 4.4.0"));
        // Staphylococcus_aureus has a DNA file without its BLAST index in the fake database
        assert!(bad(json!({"organism": "Staphylococcus_aureus"})).message.contains("AMR_DNA-Staphylococcus_aureus.fa"));
        assert!(amrfinder(&cx, &parse(json!({"protein": "ann/s1.faa", "database": "amrdb", "organism": "Staphylococcus_aureus", "output": "typing/p.amrfinder.tsv"}))).is_ok());
        std::fs::remove_file(d.path().join("amrdb/fam.tsv")).unwrap();
        assert!(bad(json!({})).message.contains("fam.tsv"));
        for invented in ["update", "force_update", "blast_bin", "parm"] {
            let mut j = base.clone();
            j[invented] = json!(true);
            assert!(serde_json::from_value::<AmrfinderParams>(j).is_err(), "{invented}");
        }
    }
}
