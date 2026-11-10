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
    Mlst,
    Amr,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct IsotableParams {
    /// Results folders to search recursively for fastp, leanqc, QUAST, BUSCO, CheckM2, GTDB-Tk, Kraken 2, mlst and AMRFinderPlus outputs.
    pub dirs: Vec<String>,
    /// Sample sheet (CSV/TSV, e.g. from sheetgen) with the real sample names to join on. Strongly recommended: without it, joins are by file name only.
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
    if let Some(x) = p.max_contigs {
        v::range("max_contigs", x, 1, u64::MAX / 2)?;
        inv.opt("max-contigs", x);
    }
    if let Some(x) = p.min_n50 {
        inv.opt("min-n50", x);
    }
    inv.positionals(&p.dirs);
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
fn default_store(ctx: &Ctx) -> Option<PathBuf> {
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
        assert_eq!(bad(json!({"files": ["-r1.fq"], "outdir": "no/such/dir"})), "outdir");
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
