//! Tool parameters (JSON Schema via schemars) and their translation into irit argument vectors.
//!
//! Options are always emitted as `--name=value` and positionals after `--`, so a value that
//! starts with `-` can never be read as a flag.

use crate::validate::{self as v, Check, Ctx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Invocation {
    pub tool: &'static str,
    pub args: Vec<String>,
    /// Files and folders the tool reads; hashed into the run record.
    pub inputs: Vec<PathBuf>,
    /// Files and folders the tool may write; new or changed files are hashed afterwards.
    pub outputs: Vec<PathBuf>,
    pub stdout_is_output: bool,
    /// Environment the tool's path resolution depends on, as resolved values (e.g. the
    /// `DBLOCK_STORE` dblock will use). Passed to the tool, recorded, and exported by replay.sh.
    pub env: BTreeMap<String, String>,
    /// A third-party program (fastp, Bakta, ...) rather than an irit binary: found on PATH or
    /// through `--tool-path`, never in `--bin-dir`.
    pub external: bool,
    /// Files the tool must have written (new or changed) when it exits 0; in each group at least
    /// one. A zero exit without them is recorded as a failure.
    pub expect: Vec<Vec<PathBuf>>,
    /// File-name endings of outputs that differ between identical runs (logs, dated files);
    /// recorded with their hash, but replay.sh only checks they exist.
    pub volatile: &'static [&'static str],
    /// Data the program brings with it (found from the program's own path once it is resolved),
    /// hashed into the record with the inputs. mlst's bundled PubMLST schemes, for example.
    pub bundled_data: Option<fn(&Path, &Ctx) -> Vec<PathBuf>>,
    /// Symlinks the server makes before the tool runs (path, target as written into the link,
    /// relative to the link's folder), e.g. `genomes/s1.fna -> ../asm/s1/contigs.fa`. Recorded,
    /// and recreated by replay.sh.
    pub links: Vec<(PathBuf, PathBuf)>,
    /// An assembly the tool writes: its contig count, length and N50 are recorded, and replay.sh
    /// prints them beside the replayed assembly's (Shovill's outputs are volatile).
    pub assembly: Option<PathBuf>,
    /// Notes for the agent and the record that don't stop the call (data the record can't vouch for).
    pub warnings: Vec<String>,
}

impl Invocation {
    pub(crate) fn new(tool: &'static str) -> Self {
        Invocation {
            tool,
            args: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            stdout_is_output: false,
            env: BTreeMap::new(),
            external: false,
            expect: Vec::new(),
            volatile: &[],
            bundled_data: None,
            links: Vec::new(),
            assembly: None,
            warnings: Vec::new(),
        }
    }
    pub(crate) fn opt(&mut self, name: &str, value: impl ToString) {
        self.args.push(format!("--{name}={}", value.to_string()));
    }
    pub(crate) fn flag(&mut self, name: &str, on: bool) {
        if on {
            self.args.push(format!("--{name}"));
        }
    }
    pub(crate) fn positionals(&mut self, values: &[String]) {
        if !values.is_empty() {
            self.args.push("--".into());
            self.args.extend(values.iter().cloned());
        }
    }
}

// ---------- sheetgen ----------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SheetFormat {
    /// sample,fastq_1,fastq_2 (most nf-core pipelines)
    Nfcore,
    /// QIIME 2 PairedEnd/SingleEnd manifest (TSV, absolute paths)
    Qiime2,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SheetgenParams {
    /// Folders to scan recursively for FASTQ files (relative to the server's working directory or absolute).
    pub dirs: Vec<String>,
    /// File to write the sheet to; `.tsv` writes tabs, anything else commas. Omit to get the sheet back only in the result.
    /// Must be inside the server's root.
    #[serde(default)]
    pub output: Option<String>,
    /// Replace `output` if it already exists. Default false: an existing file is refused.
    #[serde(default)]
    pub overwrite: bool,
    /// Layout when no schema or pipeline is given. Default nfcore.
    #[serde(default)]
    pub format: Option<SheetFormat>,
    /// Pipeline name like `nf-core/bacass`; its `assets/schema_input.json` must be in Nextflow's cache (run `nextflow pull` first). Mutually exclusive with `schema`.
    #[serde(default)]
    pub pipeline: Option<String>,
    /// Path to a pipeline input schema (`schema_input.json`). Mutually exclusive with `pipeline`.
    #[serde(default)]
    pub schema: Option<String>,
    /// Fixed values for columns the file names can't tell, e.g. {"strandedness": "auto"}.
    #[serde(default)]
    pub set: BTreeMap<String, String>,
    /// Name samples after the folder holding the files instead of the file name.
    #[serde(default)]
    pub name_from_dir: bool,
    /// Treat unpaired files as long reads (the schema's long-read column, for hybrid assembly).
    #[serde(default)]
    pub long: bool,
    /// Write paths relative to the working directory instead of absolute.
    #[serde(default)]
    pub relative: bool,
    /// Create clean, space-free symlinks `<sample>_<n>_R1.fastq.gz` in this folder and list those. Originals are not touched.
    #[serde(default)]
    pub link_into: Option<String>,
    /// Don't pair on a bare `_1`/`_2` suffix (use when `x_1.fq.gz` and `x_2.fq.gz` are two single-end samples).
    #[serde(default)]
    pub no_pair_numeric: bool,
    /// Accept different names that become the same after cleaning (`s 1`, `s_1`); the pipeline merges them.
    #[serde(default)]
    pub allow_merge: bool,
    /// Write the sheet even if the schema has rules sheetgen can't evaluate. Only with the user's agreement.
    #[serde(default)]
    pub allow_unchecked: bool,
}

pub fn sheetgen(ctx: &Ctx, p: &SheetgenParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = Invocation::new("sheetgen");
    v::non_empty_list("dirs", &p.dirs)?;
    for d in &p.dirs {
        inv.inputs.push(v::existing_dir(cwd, "dirs", d)?);
    }
    if p.pipeline.is_some() && p.schema.is_some() {
        return v::invalid("schema", "give either pipeline or schema, not both");
    }
    if p.format == Some(SheetFormat::Qiime2) && (p.pipeline.is_some() || p.schema.is_some()) {
        return v::invalid("format", "format applies only when no pipeline or schema is given");
    }
    if let Some(o) = &p.output {
        inv.outputs.push(v::output_file(ctx, "output", o, p.overwrite)?);
        inv.opt("output", o);
    } else {
        inv.stdout_is_output = true;
    }
    if let Some(f) = p.format {
        inv.opt("format", if f == SheetFormat::Nfcore { "nfcore" } else { "qiime2" });
    }
    if let Some(name) = &p.pipeline {
        v::pipeline_name("pipeline", name)?;
        inv.opt("pipeline", name);
        if let Some(home) = nxf_home(ctx) {
            if let Some(schema) = pipeline_schema(&home, name) {
                inv.inputs.push(schema);
            }
            inv.env.insert("NXF_HOME".into(), home.to_string_lossy().into_owned());
        }
    }
    if let Some(s) = &p.schema {
        inv.inputs.push(v::existing_file(cwd, "schema", s)?);
        inv.opt("schema", s);
    }
    for (col, val) in &p.set {
        v::column_name("set", col)?;
        v::text("set", val)?;
        inv.opt("set", format!("{col}={val}"));
    }
    inv.flag("name-from-dir", p.name_from_dir);
    inv.flag("long", p.long);
    inv.flag("relative", p.relative);
    if let Some(l) = &p.link_into {
        // sheetgen itself refuses to replace a file in this folder, so no overwrite check.
        inv.outputs.push(v::output_dir(ctx, "link_into", l, None, false)?);
        inv.opt("link-into", l);
    }
    inv.flag("no-pair-numeric", p.no_pair_numeric);
    inv.flag("allow-merge", p.allow_merge);
    inv.flag("allow-unchecked", p.allow_unchecked);
    inv.positionals(&p.dirs);
    Ok(inv)
}

/// The Nextflow home sheetgen will search: `$NXF_HOME`, else `$HOME/.nextflow`.
fn nxf_home(ctx: &Ctx) -> Option<PathBuf> {
    ctx.var("NXF_HOME").map(PathBuf::from).or_else(|| ctx.var("HOME").map(|h| PathBuf::from(h).join(".nextflow")))
}

/// Mirrors sheetgen's lookup (`schema::find_for_pipeline`): `assets/<name>/assets/schema_input.json`,
/// else the newest clone under `assets/.repos/<name>/clones/`. Used to hash the schema as an input.
fn pipeline_schema(home: &Path, name: &str) -> Option<PathBuf> {
    let assets = home.join("assets");
    let direct = assets.join(name).join("assets/schema_input.json");
    if direct.is_file() {
        return Some(direct);
    }
    let rd = std::fs::read_dir(assets.join(".repos").join(name).join("clones")).ok()?;
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .map(|e| e.path().join("assets/schema_input.json"))
        .filter(|p| p.is_file())
        .map(|p| (p.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), p))
        .collect();
    found.sort();
    found.pop().map(|(_, p)| p)
}

// ---------- leanqc ----------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LeanqcParams {
    /// FASTQ or FASTA files (plain, gzip, bzip2, xz or zstd).
    pub files: Vec<String>,
    /// Folder for the reports, inside the server's root. Default `leanqc`.
    #[serde(default)]
    pub outdir: Option<String>,
    /// Allow writing into an `outdir` that already has files (reports of the same name are replaced). Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// Files processed at once, one thread each (1-64). Default 4.
    #[serde(default)]
    pub threads: Option<u32>,
    /// Distinct sequences tracked for duplication and overrepresented sequences. Default 100000 (as FastQC).
    #[serde(default)]
    pub dup_distinct: Option<u64>,
    /// Quality encoding, instead of detecting it: 33 (Sanger, Illumina 1.8+) or 64 (Illumina 1.3-1.7).
    #[serde(default)]
    pub phred: Option<u8>,
    /// Skip the HTML reports (JSON and TSV only).
    #[serde(default)]
    pub no_html: bool,
}

pub fn leanqc(ctx: &Ctx, p: &LeanqcParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = Invocation::new("leanqc");
    v::non_empty_list("files", &p.files)?;
    for f in &p.files {
        inv.inputs.push(v::existing_file(cwd, "files", f)?);
    }
    let outdir = p.outdir.as_deref().unwrap_or("leanqc");
    inv.outputs.push(v::output_dir(ctx, "outdir", outdir, Some(p.overwrite), true)?);
    inv.opt("outdir", outdir);
    if let Some(t) = p.threads {
        v::range("threads", t, 1, 64)?;
        inv.opt("threads", t);
    }
    if let Some(d) = p.dup_distinct {
        v::range("dup_distinct", d, 1, 100_000_000)?;
        inv.opt("dup-distinct", d);
    }
    if let Some(ph) = p.phred {
        if ph != 33 && ph != 64 {
            return v::invalid("phred", format!("{ph} is not 33 or 64"));
        }
        inv.opt("phred", ph);
    }
    inv.flag("no-html", p.no_html);
    inv.positionals(&p.files);
    Ok(inv)
}

// ---------- seqplan ----------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SeqplanParams {
    /// Kit or flow cell id, e.g. miseq-v3, miseq-v2, iseq-100, nextseq-550-mid, nextseq-p1, nextseq-p2, novaseq-x-1.5b,
    /// minion-r10.4.1, flongle-r10.4.1. An unknown id is refused with the full list.
    pub kit: String,
    /// Read configuration, e.g. 2x150. Default: the kit's usual one (2x300 for miseq-v3, 2x150 for most others).
    #[serde(default)]
    pub reads: Option<String>,
    /// Organism preset with NCBI's expected genome size, e.g. klebsiella-pneumoniae, escherichia-coli,
    /// staphylococcus-aureus, mycobacterium-tuberculosis. Give this or genome_size.
    #[serde(default)]
    pub organism: Option<String>,
    /// Genome size in bp with an optional k/M/G suffix, e.g. 5.5M.
    #[serde(default)]
    pub genome_size: Option<String>,
    /// Target mean coverage per isolate (×). Default 50.
    #[serde(default)]
    pub coverage: Option<f64>,
    /// Fraction of run output that ends up in sample reads. Default 0.8 (Illumina), 1.0 (nanopore).
    #[serde(default)]
    pub usable: Option<f64>,
    /// Barcode-imbalance margin (1-10). Default 1.3.
    #[serde(default)]
    pub imbalance: Option<f64>,
    /// Barcodes the library kit offers (e.g. 24 or 96 for ONT Rapid Barcoding, 96 or 384 Illumina
    /// indexes): caps isolates per run. Ask the user which kit they use rather than guessing.
    #[serde(default)]
    pub barcodes: Option<u64>,
    /// Number of isolates to sequence: also returns runs needed and cost per isolate for them.
    #[serde(default)]
    pub samples: Option<u64>,
    /// Price of one run as the user gave it. Never invent prices: leave this out unless the user gave one.
    #[serde(default)]
    pub run_price: Option<f64>,
    /// Currency label for run_price, e.g. USD or IDR.
    #[serde(default)]
    pub currency: Option<String>,
}

pub fn seqplan(_ctx: &Ctx, p: &SeqplanParams) -> Check<Invocation> {
    let mut inv = Invocation::new("seqplan");
    v::db_id("kit", &p.kit)?;
    inv.opt("kit", &p.kit);
    if let Some(r) = &p.reads {
        let ok = r == "long"
            || r.split_once('x').is_some_and(|(n, l)| {
                !n.is_empty() && !l.is_empty() && n.chars().chain(l.chars()).all(|c| c.is_ascii_digit())
            });
        if !ok {
            return v::invalid("reads", format!("{r:?} is not a read configuration like 2x150"));
        }
        inv.opt("reads", r);
    }
    match (&p.organism, &p.genome_size) {
        (Some(o), None) => {
            v::db_id("organism", o)?;
            inv.opt("organism", o);
        }
        (None, Some(s)) => {
            let ok = !s.is_empty() && s.len() <= 16 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '.');
            if !ok {
                return v::invalid("genome_size", format!("{s:?} is not a size like 5.5M"));
            }
            inv.opt("genome-size", s);
        }
        _ => return v::invalid("genome_size", "give exactly one of organism and genome_size"),
    }
    for (field, flag, val) in [("coverage", "coverage", p.coverage), ("usable", "usable", p.usable), ("imbalance", "imbalance", p.imbalance), ("run_price", "run-price", p.run_price)] {
        if let Some(x) = val {
            if !(x.is_finite() && x >= 0.0) {
                return v::invalid(field, "must be a non-negative number");
            }
            inv.opt(flag, x);
        }
    }
    if let Some(n) = p.barcodes {
        if n == 0 {
            return v::invalid("barcodes", "must be at least 1");
        }
        inv.opt("barcodes", n);
    }
    if let Some(n) = p.samples {
        if n == 0 {
            return v::invalid("samples", "must be at least 1");
        }
        inv.opt("samples", n);
    }
    if let Some(c) = &p.currency {
        if c.is_empty() || c.len() > 8 || !c.chars().all(|c| c.is_ascii_alphabetic()) {
            return v::invalid("currency", format!("{c:?} is not a currency label like USD"));
        }
        inv.opt("currency", c);
    }
    inv.opt("format", "json");
    inv.stdout_is_output = true;
    Ok(inv)
}

// ---------- isotable ----------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Single-genome assemblies of isolates.
    Isolate,
    /// Metagenome-assembled genomes, graded by MIMAG tiers from CheckM2.
    Mag,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Requirement {
    Reads,
    Q30,
    Contigs,
    N50,
    Completeness,
    Contamination,
    Kraken,
    Gtdb,
    /// Mean depth from read mapping (coverm_genome).
    Depth,
    Mlst,
    Amr,
    Kleborate,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IsotableParams {
    /// Results folders to search recursively for fastp, leanqc, QUAST, Shovill (`contigs.fa` beside `shovill.log`), Flye, Medaka, BUSCO, CheckM2, GTDB-Tk, Kraken 2, CoverM genome (`<sample>.coverm.tsv`), mlst, AMRFinderPlus and Kleborate outputs.
    pub dirs: Vec<String>,
    /// Sample sheet (CSV/TSV, e.g. from sheetgen) with the real sample names to join on. Strongly recommended: without it, joins are by file name only. Its FASTQ columns (fastq_1, fastq_2) also join QC reports named after the read files, e.g. SRA run names.
    #[serde(default)]
    pub sheet: Option<String>,
    /// Grading rules. Default isolate.
    #[serde(default)]
    pub mode: Option<Mode>,
    /// Also write the table to this TSV file (inside the server's root).
    #[serde(default)]
    pub output: Option<String>,
    /// Also write a MultiQC custom-content table here (inside the server's root).
    #[serde(default)]
    pub mqc: Option<String>,
    /// Replace `output` / `mqc` if they already exist. Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// Measurements a sample needs before it can pass. Omit for the defaults (isolate: reads,q30,contigs,n50,completeness; mag: completeness,contamination). An empty list turns the check off.
    #[serde(default)]
    pub require: Option<Vec<Requirement>>,
    /// Fail below this many reads after trimming. Default 100000.
    #[serde(default)]
    pub min_reads: Option<u64>,
    /// Warn below this % of Q30 bases after trimming. Default 80.
    #[serde(default)]
    pub min_q30: Option<f64>,
    /// Warn above this many contigs; fail above twice it. Default 300.
    #[serde(default)]
    pub max_contigs: Option<u64>,
    /// Warn below this N50; fail below half of it. Default 10000.
    #[serde(default)]
    pub min_n50: Option<u64>,
    /// Warn below this BUSCO complete %; fail 5 points lower. Default 95.
    #[serde(default)]
    pub min_busco: Option<f64>,
    /// Isolate mode: warn below this CheckM2 completeness %; fail 5 points lower. Default 95.
    #[serde(default)]
    pub min_completeness: Option<f64>,
    /// Isolate mode: warn above this CheckM2 contamination %; fail above 5. Default 2.
    #[serde(default)]
    pub max_contamination: Option<f64>,
    /// Warn below this mean depth from read mapping (coverm_genome); fail below half of it. Default 20 (docs/research/coverage-depth.md); 0 turns it off. Shovill's own depth estimate is shown but never graded.
    #[serde(default)]
    pub min_depth: Option<f64>,
    /// Count assembly statistics over contigs of this length or more. Default 500 (QUAST's); 200 matches what GenBank accepts and what many papers report. A QUAST report made at another cut-off then conflicts.
    #[serde(default)]
    pub min_contig: Option<u64>,
}

pub fn isotable(ctx: &Ctx, p: &IsotableParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = Invocation::new("isotable");
    v::non_empty_list("dirs", &p.dirs)?;
    for d in &p.dirs {
        inv.inputs.push(v::existing_dir(cwd, "dirs", d)?);
    }
    if let Some(s) = &p.sheet {
        inv.inputs.push(v::existing_file(cwd, "sheet", s)?);
        inv.opt("sheet", s);
    }
    if let Some(m) = p.mode {
        inv.opt("mode", if m == Mode::Isolate { "isolate" } else { "mag" });
    }
    if let Some(o) = &p.output {
        inv.outputs.push(v::output_file(ctx, "output", o, p.overwrite)?);
        inv.opt("output", o);
    } else {
        inv.stdout_is_output = true;
    }
    if let Some(m) = &p.mqc {
        inv.outputs.push(v::output_file(ctx, "mqc", m, p.overwrite)?);
        inv.opt("mqc", m);
    }
    if let Some(req) = &p.require {
        let list: Vec<String> = req
            .iter()
            .map(|r| serde_json::to_value(r).unwrap().as_str().unwrap().to_string())
            .collect();
        inv.opt("require", if list.is_empty() { "none".to_string() } else { list.join(",") });
    }
    if let Some(x) = p.min_reads {
        inv.opt("min-reads", x);
    }
    for (name, field, val) in [
        ("min-q30", "min_q30", p.min_q30),
        ("min-busco", "min_busco", p.min_busco),
        ("min-completeness", "min_completeness", p.min_completeness),
        ("max-contamination", "max_contamination", p.max_contamination),
    ] {
        if let Some(x) = val {
            v::percent(field, x)?;
            inv.opt(name, x);
        }
    }
    if let Some(x) = p.min_contig {
        v::range("min_contig", x, 1, 1_000_000)?;
        inv.opt("min-contig", x);
    }
    if let Some(x) = p.max_contigs {
        v::range("max_contigs", x, 1, u64::MAX / 2)?;
        inv.opt("max-contigs", x);
    }
    if let Some(x) = p.min_n50 {
        inv.opt("min-n50", x);
    }
    if let Some(x) = p.min_depth {
        if !x.is_finite() || !(0.0..=100_000.0).contains(&x) {
            return v::invalid("min_depth", format!("{x} is not a depth between 0 and 100000"));
        }
        inv.opt("min-depth", x);
    }
    inv.positionals(&p.dirs);
    Ok(inv)
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AssemblyStatsParams {
    /// Assembly FASTA files (plain or gzip): Shovill's contigs.fa, Flye's assembly.fasta, a reference genome, ...
    pub assemblies: Vec<String>,
    /// Leave out contigs shorter than this (bp) from every statistic except contigs_all and length_all. Default 500, QUAST's default, so the numbers match a QUAST report.
    #[serde(default)]
    pub min_contig: Option<u64>,
}

/// `isotable assembly-stats --json`: QUAST-style statistics, computed by isotable (stdout is the result).
pub fn assembly_stats(ctx: &Ctx, p: &AssemblyStatsParams) -> Check<Invocation> {
    let mut inv = Invocation::new("isotable");
    inv.args.push("assembly-stats".into());
    v::non_empty_list("assemblies", &p.assemblies)?;
    for a in &p.assemblies {
        let path = v::existing_file(&ctx.cwd, "assemblies", a)?;
        v::sequence_file(&path, "assemblies", a, crate::validate::Seq::Fasta)?;
        inv.inputs.push(path);
    }
    inv.flag("json", true);
    // given explicitly, so the record says which cut-off the numbers use
    let min = p.min_contig.unwrap_or(isotable::asm::QUAST_MIN_CONTIG);
    v::range("min_contig", min, 0, 1_000_000_000)?;
    inv.opt("min-contig", min);
    inv.stdout_is_output = true;
    inv.positionals(&p.assemblies);
    Ok(inv)
}

// ---------- sheetgen verify ----------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadsVerifyParams {
    /// Read files, or folders to scan (recursively) for FASTQ files.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Sample sheets whose FASTQ columns name files to check too.
    #[serde(default)]
    pub sheets: Vec<String>,
    /// Expected MD5s in `md5sum` format (`<md5>  <file>`), e.g. a provider's MD5SUMS.
    #[serde(default)]
    pub md5: Vec<String>,
    /// Look up expected MD5s and sizes at ENA for the run accession (ERR/SRR/DRR) in each file name. Network use: ask the user first.
    #[serde(default)]
    pub ena: bool,
    /// Run accession for files whose name has none, as `file=accession`, e.g. `reads/AHB29_ont.fastq.gz=ERR15424439`.
    #[serde(default)]
    pub runs: Vec<String>,
    /// Files checked at the same time (each uses about four threads). Default 1.
    #[serde(default)]
    pub jobs: Option<u32>,
}

/// `sheetgen verify --json`: gzip and FASTQ integrity plus MD5/SHA-256 against expected sums
/// (sheetgen/src/verify.rs). Exits 1 when any file has a problem; the report is still on stdout.
pub fn reads_verify(ctx: &Ctx, p: &ReadsVerifyParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = Invocation::new("sheetgen");
    inv.args.push("verify".into());
    if p.paths.is_empty() && p.sheets.is_empty() {
        return v::invalid("paths", "give read files or folders (paths), or sample sheets (sheets)");
    }
    for s in &p.sheets {
        inv.inputs.push(v::existing_file(cwd, "sheets", s)?);
        inv.opt("sheet", s);
    }
    for m in &p.md5 {
        inv.inputs.push(v::existing_file(cwd, "md5", m)?);
        inv.opt("md5", m);
    }
    for r in &p.runs {
        let Some((file, acc)) = r.split_once('=') else {
            return v::invalid("runs", format!("{r}: write it as file=accession"));
        };
        let ok = acc.len() > 3 && ["ERR", "SRR", "DRR"].iter().any(|x| acc.starts_with(x)) && acc[3..].chars().all(|c| c.is_ascii_digit());
        if file.is_empty() || !ok {
            return v::invalid("runs", format!("{r}: write it as file=accession, with an ERR/SRR/DRR run accession"));
        }
        inv.opt("run", r);
    }
    inv.flag("ena", p.ena);
    if let Some(j) = p.jobs {
        v::range("jobs", j, 1, 64)?;
        inv.opt("jobs", j);
    }
    inv.flag("json", true);
    for path in &p.paths {
        let full = cwd.join(path);
        if !full.exists() {
            return v::invalid("paths", format!("{path} does not exist"));
        }
        inv.inputs.push(full);
    }
    // The JSON report has timings, so replay doesn't compare it; it re-runs the check (exit status).
    inv.positionals(&p.paths);
    Ok(inv)
}

// ---------- cooccur ----------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum CooccurMethod {
    /// Proportionality on CLR values (Lovell et al. 2015). The default.
    Rho,
    /// Rank correlation of CLR values (about 10 times slower).
    Spearman,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CooccurParams {
    /// Genome x sample table: coverm_abundance's output (or a plain TSV, genome first, one column per sample). Needs at least 5 samples; finding anything needs many more (about 20-40, docs/tools/cooccur.md).
    pub table: String,
    /// Which CoverM measure to use when the table has several: relative_abundance, count, mean, trimmed_mean, reads_per_base, rpkm or tpm.
    #[serde(default)]
    pub measure: Option<String>,
    /// Taxonomy tables: GTDB-Tk summaries (user_genome, classification) or two columns (genome, lineage); Patescibacteria are labelled CPR.
    #[serde(default)]
    pub taxonomy: Vec<String>,
    /// Statistic. Default rho.
    #[serde(default)]
    pub method: Option<CooccurMethod>,
    /// Keep genomes present in at least this fraction of samples. Default 0.5.
    #[serde(default)]
    pub min_prevalence: Option<f64>,
    /// Pairs with q at or below this pass. Default 0.05.
    #[serde(default)]
    pub fdr: Option<f64>,
    /// Pairs must also reach this statistic (absolute value), e.g. 0.6. Default 0.
    #[serde(default)]
    pub min_stat: Option<f64>,
    /// Only positive association (two-sided by default).
    #[serde(default)]
    pub positive_only: bool,
    /// Random seed. Default 1: the same call gives the same output.
    #[serde(default)]
    pub seed: Option<u64>,
    /// Edge list TSV to write, inside the root.
    pub output: String,
    /// Summary JSON to write, inside the root. Default: the output name with .summary.json.
    #[serde(default)]
    pub summary: Option<String>,
    /// Replace existing outputs. Default false.
    #[serde(default)]
    pub overwrite: bool,
}

/// `cooccur`: co-occurrence of genome abundances (cooccur/src/main.rs).
pub fn cooccur(ctx: &Ctx, p: &CooccurParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = Invocation::new("cooccur");
    inv.inputs.push(v::existing_file(cwd, "table", &p.table)?);
    if let Some(m) = &p.measure {
        v::text("measure", m)?;
        inv.opt("measure", m);
    }
    for t in &p.taxonomy {
        inv.inputs.push(v::existing_file(cwd, "taxonomy", t)?);
        inv.opt("taxonomy", t);
    }
    if let Some(m) = p.method {
        inv.opt("method", if m == CooccurMethod::Rho { "rho" } else { "spearman" });
    }
    for (name, field, val) in [("min-prevalence", "min_prevalence", p.min_prevalence), ("fdr", "fdr", p.fdr)] {
        if let Some(x) = val {
            if !x.is_finite() || !(0.0..=1.0).contains(&x) {
                return v::invalid(field, "a fraction between 0 and 1");
            }
            inv.opt(name, x);
        }
    }
    if let Some(x) = p.min_stat {
        if !x.is_finite() || !(0.0..=1.0).contains(&x) {
            return v::invalid("min_stat", "between 0 and 1");
        }
        inv.opt("min-stat", x);
    }
    if p.positive_only {
        inv.opt("alternative", "greater");
    }
    inv.opt("seed", p.seed.unwrap_or(1));
    let out = v::output_file(ctx, "output", &p.output, p.overwrite)?;
    let summary_text = p.summary.clone().unwrap_or_else(|| format!("{}.summary.json", p.output.trim_end_matches(".tsv")));
    let summary = v::output_file(ctx, "summary", &summary_text, p.overwrite)?;
    inv.opt("output", &p.output);
    inv.opt("summary", &summary_text);
    inv.expect = vec![vec![out.clone()], vec![summary.clone()]];
    inv.outputs.extend([out, summary]);
    inv.positionals(std::slice::from_ref(&p.table));
    Ok(inv)
}

// ---------- gtdbprep place ----------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GtdbprepPlaceParams {
    /// Folder of genome FASTA files (e.g. bins). The file name is the genome's name.
    pub genome_dir: String,
    /// Extension of the genome files. Default fna.
    #[serde(default)]
    pub extension: Option<String>,
    /// Work folder, inside the root: GTDB-Tk's identify/, align/, classify/, plus place.tsv and place.json. Must be new or empty, or an earlier place run's (with overwrite).
    pub work: String,
    /// Allow a work folder from an earlier place run (its files are replaced). Default false.
    #[serde(default)]
    pub overwrite: bool,
    /// gtdbprep_ani's work folder: genomes its screen assigned are not placed and keep the screen's call.
    #[serde(default)]
    pub skip_assigned: Option<String>,
    /// GTDB-Tk reference data with the placement trees: dblock's gtdbtk-r232-placement-lite. Default: $GTDBTK_DATA_PATH as the server started with.
    #[serde(default)]
    pub data_path: Option<String>,
    /// Environment with GTDB-Tk 2.8.0 and gPlace. Default: $GTDBPREP_ENV as the server started with.
    #[serde(default)]
    pub env: Option<String>,
    /// gPlace's memory cap, e.g. 6G (kept every R232 tree at about 5 GiB). Default 6G.
    #[serde(default)]
    pub max_memory: Option<String>,
    /// CPUs. Default 4.
    #[serde(default)]
    pub cpus: Option<u32>,
}

/// `gtdbprep place`: identify, align --skip_gtdb_refs and classify without the ANI screen, with
/// gPlace as the placer (gtdbprep/src/place.rs).
pub fn gtdbprep_place(ctx: &Ctx, p: &GtdbprepPlaceParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = Invocation::new("gtdbprep");
    // GTDB-Tk's logs and gPlace's call log carry times.
    inv.volatile = &[".log", ".json", ".gz"];
    inv.args.push("place".into());
    let dir = v::existing_dir(cwd, "genome_dir", &p.genome_dir)?;
    let ext = p.extension.as_deref().unwrap_or("fna");
    v::extension("extension", ext)?;
    let files = v::files_with_extension(&dir, ext);
    if files.is_empty() {
        return v::invalid("genome_dir", format!("no *.{} files in {}; set extension (e.g. fa)", ext.trim_start_matches('.'), p.genome_dir));
    }
    inv.inputs.extend(files);
    inv.opt("genomes", &p.genome_dir);
    inv.opt("extension", ext);
    if let Some(a) = &p.skip_assigned {
        let d = v::existing_dir(cwd, "skip_assigned", a)?;
        if !d.join("assigned.json").is_file() {
            return v::invalid("skip_assigned", format!("{a} has no assigned.json; give gtdbprep_ani's work folder"));
        }
        inv.inputs.push(d.join("assigned.json"));
        inv.opt("skip-assigned", a);
    }
    let data = match (&p.data_path, ctx.var("GTDBTK_DATA_PATH")) {
        (Some(d), _) => d.clone(),
        (None, Some(d)) => d.to_string_lossy().into_owned(),
        (None, None) => return v::invalid("data_path", "give data_path (dblock's gtdbtk-r232-placement-lite) or start the server with GTDBTK_DATA_PATH"),
    };
    v::existing_dir(cwd, "data_path", &data)?;
    inv.opt("data", &data);
    if let Some(e) = &p.env {
        v::existing_dir(cwd, "env", e)?;
        inv.opt("env", e);
    } else if let Some(e) = ctx.var("GTDBPREP_ENV").filter(|x| !x.is_empty()) {
        // given as a flag so the command says which environment
        inv.opt("env", e.to_string_lossy());
    }
    if let Some(m) = &p.max_memory {
        let ok = m.len() > 1 && m.ends_with(['G', 'M', 'g', 'm']) && m[..m.len() - 1].parse::<u32>().is_ok_and(|n| n > 0);
        if !ok {
            return v::invalid("max_memory", format!("{m:?}: give it like 6G or 5000M"));
        }
        inv.opt("max-memory", m);
    }
    if let Some(c) = p.cpus {
        v::range("cpus", c, 1, 1024)?;
        inv.opt("cpus", c);
    }
    let work = v::output_dir(ctx, "work", &p.work, Some(p.overwrite), true)?;
    inv.outputs.push(work.clone());
    inv.opt("work", &p.work);
    inv.expect = vec![vec![work.join("place.tsv")]];
    Ok(inv)
}

// ---------- dblock ----------

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DblockProject {
    /// Project folder holding dblock.toml and dblock.lock. Default: the server's working directory.
    #[serde(default)]
    pub project_dir: Option<String>,
    /// Extra catalog entries (JSON file, same format as the built-in catalog).
    #[serde(default)]
    pub catalog: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DblockCatalogParams {
    /// Only databases for this tool, e.g. gtdb-tk, checkm2, kraken, bakta.
    #[serde(default)]
    pub tool: Option<String>,
    /// Extra catalog entries (JSON file).
    #[serde(default)]
    pub catalog: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DblockAddParams {
    /// Catalog ids to add to dblock.toml, exactly as dblock_catalog lists them.
    pub ids: Vec<String>,
    /// Shared store folder, written into dblock.toml; must be inside the server's root (default: $DBLOCK_STORE or ~/.local/share/dblock).
    #[serde(default)]
    pub store: Option<String>,
    #[serde(default)]
    pub project_dir: Option<String>,
    #[serde(default)]
    pub catalog: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DblockFetchParams {
    /// Must be true. Fetching downloads up to tens of GB and fills the disk; run dblock_plan first and get the user's go-ahead.
    pub confirm: bool,
    /// Only these databases (default: all in dblock.toml).
    #[serde(default)]
    pub ids: Vec<String>,
    /// Unpack while downloading; the archive is never stored (less space, not resumable).
    #[serde(default)]
    pub stream: bool,
    /// Keep the downloaded archive in <store>/archives.
    #[serde(default)]
    pub keep_archive: bool,
    #[serde(default)]
    pub project_dir: Option<String>,
    #[serde(default)]
    pub catalog: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DblockVerifyParams {
    /// Re-hash every file against the SHA-256 sums recorded at fetch time (minutes to hours). Default: names and sizes only (seconds).
    #[serde(default)]
    pub deep: bool,
    #[serde(default)]
    pub project_dir: Option<String>,
    #[serde(default)]
    pub catalog: Option<String>,
}

/// `writes`: the call changes files in the project folder, which must then be inside the root.
fn dblock_common(ctx: &Ctx, inv: &mut Invocation, project_dir: &Option<String>, catalog: &Option<String>, needs_toml: bool, writes: bool) -> Check<()> {
    let cwd = ctx.cwd.as_path();
    let dir = match project_dir {
        Some(d) => {
            inv.opt("dir", d);
            v::existing_dir(cwd, "project_dir", d)?
        }
        None => cwd.to_path_buf(),
    };
    if writes {
        // dblock writes dblock.toml / dblock.lock by name, so a symlink there is followed: check
        // where each file resolves, not only the folder.
        let shown = project_dir.as_deref().unwrap_or(".");
        v::inside_root(ctx, "project_dir", shown, &dir)?;
        for f in ["dblock.toml", "dblock.lock"] {
            v::inside_root(ctx, "project_dir", &format!("{shown}/{f}"), &dir.join(f))?;
        }
    }
    if let Some(store) = default_store(ctx) {
        inv.env.insert("DBLOCK_STORE".into(), store.to_string_lossy().into_owned());
    }
    let toml = dir.join("dblock.toml");
    if needs_toml && !toml.is_file() {
        return v::invalid("project_dir", format!("no dblock.toml in {}; add databases with dblock_add first", dir.display()));
    }
    for f in [toml, dir.join("dblock.lock")] {
        if f.is_file() {
            inv.inputs.push(f);
        }
    }
    if let Some(c) = catalog {
        inv.inputs.push(v::existing_file(cwd, "catalog", c)?);
        inv.opt("catalog", c);
    }
    Ok(())
}

/// The store dblock uses when dblock.toml names none (`project::default_store`):
/// `$DBLOCK_STORE`, else `$XDG_DATA_HOME/dblock`, else `$HOME/.local/share/dblock`.
pub(crate) fn default_store(ctx: &Ctx) -> Option<PathBuf> {
    if let Some(s) = ctx.var("DBLOCK_STORE") {
        return Some(PathBuf::from(s));
    }
    if let Some(x) = ctx.var("XDG_DATA_HOME") {
        return Some(PathBuf::from(x).join("dblock"));
    }
    ctx.var("HOME").map(|h| PathBuf::from(h).join(".local/share/dblock"))
}

fn project_path(cwd: &Path, project_dir: &Option<String>) -> PathBuf {
    project_dir.as_ref().map_or_else(|| cwd.to_path_buf(), |d| cwd.join(d))
}

fn dblock_ids(ids: &[String]) -> Check<()> {
    ids.iter().try_for_each(|id| v::db_id("ids", id))
}

pub fn dblock_catalog(ctx: &Ctx, p: &DblockCatalogParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = Invocation::new("dblock");
    inv.args.push("catalog".into());
    if let Some(t) = &p.tool {
        v::db_id("tool", t)?;
        inv.opt("tool", t);
    }
    if let Some(c) = &p.catalog {
        inv.inputs.push(v::existing_file(cwd, "catalog", c)?);
        inv.opt("catalog", c);
    }
    inv.stdout_is_output = true;
    Ok(inv)
}

pub fn dblock_add(ctx: &Ctx, p: &DblockAddParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    let mut inv = Invocation::new("dblock");
    inv.args.push("add".into());
    v::non_empty_list("ids", &p.ids)?;
    dblock_ids(&p.ids)?;
    dblock_common(ctx, &mut inv, &p.project_dir, &p.catalog, false, true)?;
    if let Some(s) = &p.store {
        v::text("store", s)?;
        // dblock expands a leading `~/` with $HOME; anything else is used as written.
        let store = match (s.strip_prefix("~/"), ctx.var("HOME")) {
            (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
            _ => cwd.join(s),
        };
        v::inside_root(ctx, "store", s, &store)?;
        inv.opt("store", s);
    }
    let dir = project_path(cwd, &p.project_dir);
    inv.outputs.push(dir.join("dblock.toml"));
    inv.positionals(&p.ids);
    Ok(inv)
}

pub fn dblock_simple(ctx: &Ctx, sub: &'static str, p: &DblockProject) -> Check<Invocation> {
    let mut inv = Invocation::new("dblock");
    inv.args.push(sub.into());
    dblock_common(ctx, &mut inv, &p.project_dir, &p.catalog, true, false)?;
    // plan/check/env only read; their stdout is the answer.
    inv.stdout_is_output = sub == "env";
    Ok(inv)
}

pub fn dblock_verify(ctx: &Ctx, p: &DblockVerifyParams) -> Check<Invocation> {
    let mut inv = Invocation::new("dblock");
    inv.args.push("verify".into());
    dblock_common(ctx, &mut inv, &p.project_dir, &p.catalog, true, false)?;
    inv.flag("deep", p.deep);
    Ok(inv)
}

pub fn dblock_fetch(ctx: &Ctx, p: &DblockFetchParams) -> Check<Invocation> {
    let cwd = ctx.cwd.as_path();
    if !p.confirm {
        return v::invalid(
            "confirm",
            "fetch downloads large databases; run dblock_plan, show the user the sizes, and call again with confirm: true once they agree",
        );
    }
    let mut inv = Invocation::new("dblock");
    inv.args.push("fetch".into());
    dblock_ids(&p.ids)?;
    dblock_common(ctx, &mut inv, &p.project_dir, &p.catalog, true, true)?;
    inv.flag("stream", p.stream);
    inv.flag("keep-archive", p.keep_archive);
    // The store's data folders are covered by the lock file's per-file SHA-256 list; hashing
    // 100 GB again here would only cost time.
    let dir = project_path(cwd, &p.project_dir);
    inv.outputs.push(dir.join("dblock.lock"));
    inv.positionals(&p.ids);
    Ok(inv)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescribeRunParams {
    /// Run id from list_runs. Default: the current session's run.
    #[serde(default)]
    pub run_id: Option<String>,
    /// Return every call's full record (command, environment, every input and output with SHA-256).
    /// Large: tens of kB per call. Default: a summary of each call (tool, status, version, error,
    /// warnings, output paths).
    #[serde(default)]
    pub detail: bool,
    /// Return this one call's full record only.
    #[serde(default)]
    pub call_id: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListRunsParams {}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx(dir: &Path) -> Ctx {
        let root = std::fs::canonicalize(dir).unwrap();
        Ctx { cwd: root.clone(), runs_root: Some(root.join("irit-runs")), root, allow_outside_root: false, vars: BTreeMap::new() }
    }

    fn parse<T: serde::de::DeserializeOwned>(v: serde_json::Value) -> T {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn verify_cooccur_and_place_args() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        std::fs::create_dir_all(cx.cwd.join("reads")).unwrap();
        std::fs::write(cx.cwd.join("reads/a_1.fastq.gz"), b"x").unwrap();
        std::fs::write(cx.cwd.join("MD5SUMS"), b"x").unwrap();
        let v = reads_verify(&cx, &parse(serde_json::json!({"paths": ["reads"], "md5": ["MD5SUMS"], "runs": ["reads/a_1.fastq.gz=ERR15424439"], "ena": true}))).unwrap();
        assert_eq!(v.args, ["verify", "--md5=MD5SUMS", "--run=reads/a_1.fastq.gz=ERR15424439", "--ena", "--json", "--", "reads"]);
        assert_eq!(reads_verify(&cx, &parse(serde_json::json!({}))).unwrap_err().field, "paths");
        assert_eq!(reads_verify(&cx, &parse(serde_json::json!({"paths": ["reads"], "runs": ["a=XYZ1"]}))).unwrap_err().field, "runs");

        std::fs::write(cx.cwd.join("ab.tsv"), b"Genome\ts1\n").unwrap();
        let c = cooccur(&cx, &parse(serde_json::json!({"table": "ab.tsv", "output": "cooc/edges.tsv", "positive_only": true}))).unwrap();
        assert_eq!(c.args, ["--alternative=greater", "--seed=1", "--output=cooc/edges.tsv", "--summary=cooc/edges.summary.json", "--", "ab.tsv"]);
        assert_eq!(cooccur(&cx, &parse(serde_json::json!({"table": "ab.tsv", "output": "e.tsv", "fdr": 2.0}))).unwrap_err().field, "fdr");

        std::fs::create_dir_all(cx.cwd.join("bins")).unwrap();
        std::fs::write(cx.cwd.join("bins/b1.fa"), b">c\nACGT\n").unwrap();
        std::fs::create_dir_all(cx.cwd.join("gtdb")).unwrap();
        let pl = gtdbprep_place(&cx, &parse(serde_json::json!({"genome_dir": "bins", "extension": "fa", "work": "place", "data_path": "gtdb", "max_memory": "6G"}))).unwrap();
        assert_eq!(pl.args, ["place", "--genomes=bins", "--extension=fa", "--data=gtdb", "--max-memory=6G", "--work=place"]);
        assert_eq!(gtdbprep_place(&cx, &parse(serde_json::json!({"genome_dir": "bins", "extension": "fa", "work": "p", "data_path": "gtdb", "max_memory": "lots"}))).unwrap_err().field, "max_memory");
        assert_eq!(gtdbprep_place(&cx, &parse(serde_json::json!({"genome_dir": "bins", "extension": "fa", "work": "p"}))).unwrap_err().field, "data_path");
    }

    #[test]
    fn seqplan_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        let p: SeqplanParams = parse(json!({"kit": "miseq-v3", "genome_size": "5.5M", "coverage": 50.0, "samples": 96, "run_price": 1500.0, "currency": "USD"}));
        let inv = seqplan(&cx, &p).unwrap();
        assert_eq!(
            inv.args,
            ["--kit=miseq-v3", "--genome-size=5.5M", "--coverage=50", "--run-price=1500", "--samples=96", "--currency=USD", "--format=json"]
        );
        assert!(inv.outputs.is_empty() && inv.inputs.is_empty());
        let p: SeqplanParams = parse(json!({"kit": "minion-r10.4.1", "organism": "klebsiella-pneumoniae", "barcodes": 24}));
        assert_eq!(seqplan(&cx, &p).unwrap().args, ["--kit=minion-r10.4.1", "--organism=klebsiella-pneumoniae", "--barcodes=24", "--format=json"]);
        let p: SeqplanParams = parse(json!({"kit": "nextseq-p1", "reads": "2x150", "organism": "klebsiella-pneumoniae"}));
        assert_eq!(seqplan(&cx, &p).unwrap().args, ["--kit=nextseq-p1", "--reads=2x150", "--organism=klebsiella-pneumoniae", "--format=json"]);

        let bad = |v: serde_json::Value| seqplan(&cx, &parse(v)).unwrap_err().field;
        assert_eq!(bad(json!({"kit": "--help", "genome_size": "5M"})), "kit");
        assert_eq!(bad(json!({"kit": "miseq-v3"})), "genome_size");
        assert_eq!(bad(json!({"kit": "miseq-v3", "genome_size": "5M", "organism": "escherichia-coli"})), "genome_size");
        assert_eq!(bad(json!({"kit": "miseq-v3", "genome_size": "5M;rm"})), "genome_size");
        assert_eq!(bad(json!({"kit": "miseq-v3", "genome_size": "5M", "reads": "2x"})), "reads");
        assert_eq!(bad(json!({"kit": "miseq-v3", "genome_size": "5M", "run_price": -1.0})), "run_price");
        assert_eq!(bad(json!({"kit": "miseq-v3", "genome_size": "5M", "samples": 0})), "samples");
        assert_eq!(bad(json!({"kit": "miseq-v3", "genome_size": "5M", "currency": "U$D"})), "currency");
        assert!(serde_json::from_value::<SeqplanParams>(json!({"kit": "miseq-v3", "price_list": "x"})).is_err());
    }

    #[test]
    fn leanqc_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        std::fs::write(d.path().join("-r1.fq"), "@a\nA\n+\nI\n").unwrap();
        let p: LeanqcParams = parse(json!({"files": ["-r1.fq"], "outdir": "qc", "threads": 2, "no_html": true}));
        let inv = leanqc(&cx, &p).unwrap();
        assert_eq!(inv.args, ["--outdir=qc", "--threads=2", "--no-html", "--", "-r1.fq"]);
        assert_eq!(inv.outputs, [cx.root.join("qc")]);

        let bad = |v: serde_json::Value| leanqc(&cx, &parse(v)).unwrap_err().field;
        assert_eq!(bad(json!({"files": []})), "files");
        assert_eq!(bad(json!({"files": ["missing.fq"]})), "files");
        assert_eq!(bad(json!({"files": ["-r1.fq"], "threads": 0})), "threads");
        assert_eq!(bad(json!({"files": ["-r1.fq"], "phred": 40})), "phred");
        assert_eq!(bad(json!({"files": ["-r1.fq"], "outdir": "-r1.fq/qc"})), "outdir");
        // a missing folder is created by the server
        assert!(leanqc(&cx, &parse(json!({"files": ["-r1.fq"], "outdir": "no/such/dir"}))).is_ok());
        // invented parameters never reach the tool
        assert!(serde_json::from_value::<LeanqcParams>(json!({"files": ["x"], "kmer": true})).is_err());
    }

    #[test]
    fn sheetgen_args_and_checks() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        std::fs::create_dir(d.path().join("reads")).unwrap();
        let p: SheetgenParams = parse(json!({
            "dirs": ["reads"], "pipeline": "nf-core/rnaseq", "set": {"strandedness": "auto"}, "link_into": "links"
        }));
        let inv = sheetgen(&cx, &p).unwrap();
        assert_eq!(inv.args, ["--pipeline=nf-core/rnaseq", "--set=strandedness=auto", "--link-into=links", "--", "reads"]);
        assert!(inv.stdout_is_output);

        let bad = |v: serde_json::Value| sheetgen(&cx, &parse(v)).unwrap_err().field;
        assert_eq!(bad(json!({"dirs": ["reads"], "pipeline": "nf-core/x", "schema": "s.json"})), "schema");
        assert_eq!(bad(json!({"dirs": ["reads"], "pipeline": "bacass"})), "pipeline");
        assert_eq!(bad(json!({"dirs": ["reads"], "set": {"a=b": "c"}})), "set");
        assert_eq!(bad(json!({"dirs": ["nope"]})), "dirs");
        assert!(serde_json::from_value::<SheetgenParams>(json!({"dirs": ["reads"], "format": "csv"})).is_err());
    }

    #[test]
    fn isotable_and_dblock() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        std::fs::create_dir(d.path().join("results")).unwrap();
        let p: IsotableParams = parse(json!({"dirs": ["results"], "mode": "mag", "require": [], "min_q30": 75.5}));
        let inv = isotable(&cx, &p).unwrap();
        assert_eq!(inv.args, ["--mode=mag", "--require=none", "--min-q30=75.5", "--", "results"]);
        let p: IsotableParams = parse(json!({"dirs": ["results"], "require": ["reads", "n50"], "min_busco": 120.0}));
        assert_eq!(isotable(&cx, &p).unwrap_err().field, "min_busco");

        let p: DblockFetchParams = parse(json!({"confirm": false}));
        assert_eq!(dblock_fetch(&cx, &p).unwrap_err().field, "confirm");
        let p: DblockFetchParams = parse(json!({"confirm": true}));
        assert_eq!(dblock_fetch(&cx, &p).unwrap_err().field, "project_dir"); // no dblock.toml yet
        std::fs::write(d.path().join("dblock.toml"), "").unwrap();
        let p: DblockFetchParams = parse(json!({"confirm": true, "ids": ["card-4.0.2"], "stream": true}));
        assert_eq!(dblock_fetch(&cx, &p).unwrap().args, ["fetch", "--stream", "--", "card-4.0.2"]);
        let p: DblockFetchParams = parse(json!({"confirm": true, "ids": ["../etc"]}));
        assert_eq!(dblock_fetch(&cx, &p).unwrap_err().field, "ids");
        let p: DblockAddParams = parse(json!({"ids": ["card-4.0.2"], "project_dir": "results"}));
        assert_eq!(dblock_add(&cx, &p).unwrap().args, ["add", "--dir=results", "--", "card-4.0.2"]);
    }

    #[test]
    fn outputs_refused_outside_root_or_existing() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        std::fs::create_dir(cx.root.join("empty")).unwrap();
        std::fs::write(cx.root.join("r.fq"), "@a\nA\n+\nI\n").unwrap();
        let outside = tempfile::tempdir().unwrap();
        let bashrc = outside.path().join(".bashrc");
        std::fs::write(&bashrc, "keep").unwrap();
        let p: SheetgenParams = parse(json!({"dirs": ["empty"], "output": bashrc.to_str().unwrap(), "overwrite": true}));
        assert_eq!(sheetgen(&cx, &p).unwrap_err().field, "output");
        let p: SheetgenParams = parse(json!({"dirs": ["empty"], "link_into": "../links"}));
        assert_eq!(sheetgen(&cx, &p).unwrap_err().field, "link_into");
        std::fs::write(cx.root.join("sheet.csv"), "old").unwrap();
        let p: SheetgenParams = parse(json!({"dirs": ["empty"], "output": "sheet.csv"}));
        assert!(sheetgen(&cx, &p).unwrap_err().message.contains("overwrite"));
        let p: SheetgenParams = parse(json!({"dirs": ["empty"], "output": "sheet.csv", "overwrite": true}));
        assert!(sheetgen(&cx, &p).is_ok());
        // newline in a set value never reaches argv
        let p: SheetgenParams = parse(json!({"dirs": ["empty"], "set": {"x": "a\ntouch PWNED #"}}));
        assert_eq!(sheetgen(&cx, &p).unwrap_err().field, "set");

        let p: LeanqcParams = parse(json!({"files": ["r.fq"], "outdir": "/tmp"}));
        assert_eq!(leanqc(&cx, &p).unwrap_err().field, "outdir");
        let p: IsotableParams = parse(json!({"dirs": ["empty"], "mqc": "../mqc.tsv"}));
        assert_eq!(isotable(&cx, &p).unwrap_err().field, "mqc");
        let p: IsotableParams = parse(json!({"dirs": ["empty"], "output": "sheet.csv"}));
        assert_eq!(isotable(&cx, &p).unwrap_err().field, "output");

        let p: DblockAddParams = parse(json!({"ids": ["card-4.0.2"], "project_dir": outside.path().to_str().unwrap()}));
        assert_eq!(dblock_add(&cx, &p).unwrap_err().field, "project_dir");
        let p: DblockAddParams = parse(json!({"ids": ["card-4.0.2"], "store": "/var/tmp/dbs"}));
        assert_eq!(dblock_add(&cx, &p).unwrap_err().field, "store");
        let p: DblockAddParams = parse(json!({"ids": ["card-4.0.2"], "store": "dbs"}));
        assert!(dblock_add(&cx, &p).is_ok());
        // reading a project outside the root is fine
        std::fs::write(outside.path().join("dblock.toml"), "").unwrap();
        let p: DblockProject = parse(json!({"project_dir": outside.path().to_str().unwrap()}));
        assert!(dblock_simple(&cx, "plan", &p).is_ok());
        assert_eq!(std::fs::read_to_string(&bashrc).unwrap(), "keep");
    }

    /// dblock writes dblock.toml / dblock.lock by name: a symlink there pointing outside the
    /// root is refused for the calls that write, and still fine for the read-only ones.
    #[test]
    fn dblock_project_files_symlinked_outside_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        let outside = tempfile::tempdir().unwrap();
        let out = std::fs::canonicalize(outside.path()).unwrap();
        std::os::unix::fs::symlink(out.join("toml"), cx.root.join("dblock.toml")).unwrap();
        let p: DblockAddParams = parse(json!({"ids": ["card-4.0.2"]}));
        let e = dblock_add(&cx, &p).unwrap_err();
        assert_eq!(e.field, "project_dir");
        assert!(e.message.contains("dblock.toml"), "{}", e.message);
        assert!(!out.join("toml").exists());

        std::fs::remove_file(cx.root.join("dblock.toml")).unwrap();
        std::fs::write(cx.root.join("dblock.toml"), "").unwrap();
        std::fs::write(out.join("lock"), "keep").unwrap();
        std::os::unix::fs::symlink(out.join("lock"), cx.root.join("dblock.lock")).unwrap();
        assert!(dblock_add(&cx, &p).unwrap_err().message.contains("dblock.lock"));
        let p: DblockFetchParams = parse(json!({"confirm": true}));
        assert!(dblock_fetch(&cx, &p).unwrap_err().message.contains("dblock.lock"));
        let p: DblockProject = parse(json!({}));
        assert!(dblock_simple(&cx, "plan", &p).is_ok());
        // a project folder inside the run records is refused too
        std::fs::create_dir_all(cx.root.join("irit-runs/r")).unwrap();
        let p: DblockAddParams = parse(json!({"ids": ["card-4.0.2"], "project_dir": "irit-runs/r"}));
        assert_eq!(dblock_add(&cx, &p).unwrap_err().field, "project_dir");
    }

    #[test]
    fn tools_cannot_overwrite_run_records() {
        let d = tempfile::tempdir().unwrap();
        let cx = ctx(d.path());
        std::fs::create_dir(cx.root.join("results")).unwrap();
        std::fs::create_dir_all(cx.root.join("irit-runs/old")).unwrap();
        std::fs::write(cx.root.join("irit-runs/old/replay.sh"), "keep").unwrap();
        std::fs::write(cx.root.join("r.fq"), "@a\nA\n+\nI\n").unwrap();
        let p: IsotableParams = parse(json!({"dirs": ["results"], "output": "irit-runs/old/replay.sh", "overwrite": true}));
        assert_eq!(isotable(&cx, &p).unwrap_err().field, "output");
        let p: IsotableParams = parse(json!({"dirs": ["results"], "mqc": "irit-runs/old/run.json", "overwrite": true}));
        assert_eq!(isotable(&cx, &p).unwrap_err().field, "mqc");
        let p: SheetgenParams = parse(json!({"dirs": ["results"], "output": "irit-runs/x.csv"}));
        assert_eq!(sheetgen(&cx, &p).unwrap_err().field, "output");
        let p: SheetgenParams = parse(json!({"dirs": ["results"], "link_into": "irit-runs/old"}));
        assert_eq!(sheetgen(&cx, &p).unwrap_err().field, "link_into");
        let p: LeanqcParams = parse(json!({"files": ["r.fq"], "outdir": "irit-runs/old", "overwrite": true}));
        assert_eq!(leanqc(&cx, &p).unwrap_err().field, "outdir");
    }

    #[test]
    fn resolved_environment_is_recorded() {
        let d = tempfile::tempdir().unwrap();
        let mut cx = ctx(d.path());
        std::fs::write(cx.root.join("dblock.toml"), "").unwrap();
        cx.vars.insert("HOME".into(), "/home/u".into());
        let p: DblockProject = parse(json!({}));
        assert_eq!(dblock_simple(&cx, "plan", &p).unwrap().env["DBLOCK_STORE"], "/home/u/.local/share/dblock");
        cx.vars.insert("XDG_DATA_HOME".into(), "/x".into());
        assert_eq!(dblock_simple(&cx, "plan", &p).unwrap().env["DBLOCK_STORE"], "/x/dblock");
        cx.vars.insert("DBLOCK_STORE".into(), "/big/store".into());
        assert_eq!(dblock_simple(&cx, "plan", &p).unwrap().env["DBLOCK_STORE"], "/big/store");

        let nxf = cx.root.join("nxf");
        let schema = nxf.join("assets/nf-core/bacass/assets/schema_input.json");
        std::fs::create_dir_all(schema.parent().unwrap()).unwrap();
        std::fs::write(&schema, "{}").unwrap();
        std::fs::create_dir(cx.root.join("reads")).unwrap();
        cx.vars.insert("NXF_HOME".into(), nxf.clone().into());
        let p: SheetgenParams = parse(json!({"dirs": ["reads"], "pipeline": "nf-core/bacass"}));
        let inv = sheetgen(&cx, &p).unwrap();
        assert_eq!(inv.env["NXF_HOME"], nxf.to_str().unwrap());
        assert!(inv.inputs.contains(&schema), "{:?}", inv.inputs);
    }
}
