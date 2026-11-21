//! Finding and reading each tool's output, and working out which sample a record belongs to.

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::io::{BufRead, Read};
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Int(u64),
    Num(f64),
    Text(String),
}

impl Value {
    pub fn show(&self) -> String {
        match self {
            Value::Int(n) => n.to_string(),
            Value::Num(x) => {
                let s = format!("{x:.2}");
                s.trim_end_matches('0').trim_end_matches('.').to_string()
            }
            Value::Text(t) => t.clone(),
        }
    }
}

/// What a record's name refers to, which decides how it may be matched to a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// One read file (leanqc and FastQC run per file): `x_R1.fastq.gz`. Mate suffixes may be stripped.
    ReadFile,
    /// One sample's reads as a whole (fastp, Kraken 2 report): named after the sample.
    Sample,
    /// An assembly, bin or genome (QUAST, BUSCO, CheckM2, GTDB-Tk, mlst, AMRFinderPlus).
    Genome,
}

/// What one file says about one sample.
pub struct Record {
    pub raw_name: String,
    pub kind: Kind,
    /// Tool name, e.g. `fastp`.
    pub tool: &'static str,
    /// File the record came from.
    pub source: String,
    pub fields: Vec<(&'static str, Value)>,
}

/// Records found, how many files of each kind were read, and files skipped (with why).
pub type Scan = (Vec<Record>, BTreeMap<&'static str, usize>, Vec<String>);

pub fn scan(dirs: &[std::path::PathBuf]) -> Result<Scan> {
    let mut out = Vec::new();
    let mut seen = BTreeMap::new();
    let mut skipped = Vec::new();
    for dir in dirs {
        for entry in walkdir::WalkDir::new(dir).follow_links(true).sort_by_file_name() {
            let entry = match entry {
                Ok(e) => e,
                // A dangling symlink or unreadable subfolder: note it and carry on.
                Err(e) if e.depth() > 0 => {
                    let at = e.path().map(|p| p.display().to_string()).unwrap_or_default();
                    let why = e.io_error().map(|io| io.to_string()).unwrap_or_else(|| e.to_string());
                    skipped.push(format!("{at} ({why})"));
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let by_header = by_header(path, &name, &mut skipped);
            let parsed: Parsed = if let Some(p) = by_header {
                p
            } else if name.ends_with(".fastp.json") {
                Some(("fastp", fastp(path, &name)))
            } else if name.ends_with("_leanqc.json") {
                Some(("leanqc", leanqc(path)))
            } else if name.ends_with("_fastqc.zip") {
                Some(("FastQC", fastqc_zip(path)))
            } else if name == "fastqc_data.txt" {
                Some(("FastQC", fastqc_dir(path)))
            } else if name == "transposed_report.tsv" {
                Some(("QUAST", quast(path)))
            } else if name == "contigs.fa" && path.with_file_name("shovill.log").is_file() {
                Some(("Shovill", assembly(path, Some(&path.with_file_name("shovill.log")))))
            } else if name == "assembly.fasta" && path.with_file_name("flye.log").is_file() {
                // Flye's `<outdir>/assembly.fasta`, named after its folder like Shovill's.
                Some(("Flye", assembly(path, None)))
            } else if name == "consensus.fasta" && path.with_file_name("consensus_probs.hdf").is_file() {
                // medaka_consensus's polished `<outdir>/consensus.fasta`; it replaces the draft.
                Some(("Medaka", assembly(path, None)))
            } else if name.starts_with("short_summary") && name.ends_with(".json") {
                Some(("BUSCO", busco(path, &name)))
            } else if name == "quality_report.tsv" {
                Some(("CheckM2", checkm2(path)))
            } else if name.contains("summary.tsv") && (name.contains("gtdbtk") || name.contains("gtdb")) {
                Some(("GTDB-Tk", gtdbtk(path)))
            } else if (name.contains("kraken2") || name.contains(".kreport")) && (name.ends_with(".txt") || name.ends_with(".kreport") || name.ends_with(".report")) {
                match kraken(path, &name) {
                    Ok(None) => {
                        skipped.push(format!("{} (named like Kraken 2 output but not a Kraken 2 report)", path.display()));
                        None
                    }
                    r => Some(("Kraken 2", r.map(|r| r.into_iter().collect()))),
                }
            } else if name.to_ascii_lowercase().contains("coverm") && name.ends_with(".tsv") {
                match coverm_genome(path, &name) {
                    Ok(None) => {
                        skipped.push(format!("{} (named like CoverM output but not a `coverm genome` table with Mean and Covered Fraction)", path.display()));
                        None
                    }
                    r => Some(("CoverM", r.map(Option::unwrap_or_default))),
                }
            } else if KLEBORATE_OUTPUTS.contains(&name.as_str()) {
                Some(("Kleborate", kleborate(path)))
            } else if name.contains("mlst") && name.ends_with(".tsv") {
                Some(("mlst", mlst(path)))
            } else if name.to_ascii_lowercase().contains("amrfinder") && name.ends_with(".tsv") {
                Some(("AMRFinderPlus", amrfinder(path, &name)))
            } else {
                None
            };
            if let Some((tool, result)) = parsed {
                let mut records = match result {
                    // One corrupt or truncated FastQC report (an interrupted copy, say) shouldn't
                    // stop the run: list it and go on; that read file's evidence is then missing.
                    Err(e) if tool == "FastQC" => {
                        skipped.push(format!("{} (unreadable FastQC report, left out: {e:#})", path.display()));
                        continue;
                    }
                    r => r.with_context(|| format!("reading {tool} output {}", path.display()))?,
                };
                *seen.entry(tool).or_default() += 1;
                for r in &mut records {
                    r.tool = tool;
                    r.source = path.display().to_string();
                }
                out.extend(records);
            }
        }
    }
    drop_polished_drafts(&mut out, &mut skipped);
    Ok((out, seen, skipped))
}

/// A Medaka consensus is the polished version of a draft assembly in the same scan (Flye's): the
/// draft would only disagree with it by a few bases, so its record is left out, not a conflict.
fn drop_polished_drafts(out: &mut Vec<Record>, skipped: &mut Vec<String>) {
    let polished: Vec<String> = out.iter().filter(|r| r.tool == "Medaka").map(|r| r.raw_name.clone()).collect();
    out.retain(|r| {
        let draft = r.tool == "Flye" && polished.contains(&r.raw_name);
        if draft {
            skipped.push(format!("{} (Flye draft of {}; its Medaka consensus is used instead)", r.source, r.raw_name));
        }
        !draft
    });
}

/// A file's tool and what reading it gave; `None` when it is not read.
type Parsed = Option<(&'static str, Result<Vec<Record>>)>;

/// Outputs recognised by their header rather than their name: hAMRonization's TSV and nf-core/mag's
/// bin summary, which users and pipelines name freely (`hamronization_combined_report.tsv`,
/// `<tool>.tsv`, `bin_summary.tsv`). `None`: not one of them, try the name rules. `Some(None)`: a
/// file named like one of them that can't be read as one; it is listed in `skipped` with the reason.
fn by_header(path: &Path, name: &str, skipped: &mut Vec<String>) -> Option<Parsed> {
    let lower = name.to_ascii_lowercase();
    let named_hamr = lower.contains("hamroniz");
    let mut skip = |why: String| {
        skipped.push(format!("{} ({why})", path.display()));
        Some(None)
    };
    if named_hamr && (lower.ends_with(".json") || lower.ends_with(".html")) {
        return skip("hAMRonization JSON or HTML report, not read; isotable reads its TSV (`hamronize <tool> --format tsv`, `hamronize summarize -t tsv`)".into());
    }
    if !(lower.ends_with(".tsv") || lower.ends_with(".txt")) {
        return None;
    }
    let header = first_line(path).unwrap_or_default();
    let cols: Vec<&str> = header.split('\t').map(str::trim).collect();
    let has = |c: &str| cols.contains(&c);
    if has("input_file_name") && has("gene_symbol") {
        let missing: Vec<&str> = HAMR_REQUIRED.iter().copied().filter(|c| !has(c)).collect();
        if !missing.is_empty() {
            return skip(format!("hAMRonization-like table without {}; not a layout isotable knows (hAMRonization 1.1.9 to 1.4.1)", missing.join(", ")));
        }
        return Some(Some(("hAMRonization", hamronization(path))));
    }
    if has("bin") && cols.iter().any(|c| c.starts_with("Depth ")) {
        return match mag_bin_summary(path) {
            Ok(Err(why)) => skip(format!("nf-core/mag bin summary, not read: {why}")),
            r => Some(Some(("nf-core/mag", r.map(|r| r.unwrap_or_default())))),
        };
    }
    if named_hamr {
        return skip(format!("named like hAMRonization output but its header lacks {}", HAMR_REQUIRED.join(", ")));
    }
    if name == "bin_summary.tsv" {
        return skip("named like nf-core/mag's bin summary but its header has no `bin` and `Depth <sample>` columns".into());
    }
    None
}

/// The first line of a file, without reading more than 64 KiB of it.
fn first_line(path: &Path) -> Option<String> {
    let mut line = String::new();
    std::io::BufReader::new(std::fs::File::open(path).ok()?.take(64 << 10)).read_line(&mut line).ok()?;
    Some(line.trim_end_matches(['\r', '\n']).to_string())
}

/// hAMRonization's mandatory fields that isotable uses. The 36 columns of its TSV (per-tool
/// `hamronize <tool> --format tsv` and `hamronize summarize -t tsv`) are the same from 1.1.9 to
/// 1.4.1 (`hAMRonization/hAMRonizedResult.py`).
const HAMR_REQUIRED: &[&str] = &["input_file_name", "gene_symbol", "analysis_software_name", "genetic_variation_type"];

static FASTQ_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\.(fastq|fq)(\.(gz|bz2|zst))?$").unwrap());

/// hAMRonization TSV (matched against 1.4.1, columns unchanged since 1.1.9): one row per hit, from
/// any of the AMR tools it converts. Rows are grouped by `input_file_name` (hAMRonization keeps its
/// base name and drops `.gz`, `.fna`, `.fasta`, `.fsa`, `.faa`, `.fa`; isotable strips folders and
/// extensions again, as for every name) and by tool run (`analysis_software_name` and version,
/// reference database and version). Each group is one record: `amr_genes` lists `gene_symbol`, with
/// the mutation added for variant rows (`gyrA p.S83I`), as the tool's database names it.
pub fn hamronization(path: &Path) -> Result<Vec<Record>> {
    type Run = (String, String, String, String, String);
    let mut groups: BTreeMap<Run, std::collections::BTreeSet<String>> = BTreeMap::new();
    let get = |r: &BTreeMap<String, String>, k: &str| r.get(k).map(|v| v.trim().to_string()).unwrap_or_default();
    for (i, r) in tsv(path)?.iter().enumerate() {
        let file = get(r, "input_file_name");
        if file.is_empty() {
            bail!("row {} has no input_file_name", i + 2);
        }
        let symbol = get(r, "gene_symbol");
        if symbol.is_empty() {
            continue;
        }
        let mut gene = symbol;
        if get(r, "genetic_variation_type") != "gene_presence_detected" {
            let mutation = [get(r, "amino_acid_mutation"), get(r, "nucleotide_mutation")].into_iter().find(|m| !m.is_empty());
            if let Some(m) = mutation {
                gene = format!("{gene} {m}");
            }
        }
        let software = match get(r, "analysis_software_name") {
            s if s.is_empty() => "unknown".to_string(),
            s => s.to_ascii_lowercase(),
        };
        let file = file.rsplit(['/', '\\']).next().unwrap_or(&file).to_string();
        let run = (
            file,
            software,
            get(r, "analysis_software_version"),
            get(r, "reference_database_name"),
            get(r, "reference_database_version"),
        );
        groups.entry(run).or_default().insert(gene);
    }
    Ok(groups
        .into_iter()
        .map(|((file, software, version, db, db_version), genes)| {
            // Read-based tools (SRST2, ARIBA, GROOT...) name the read file; mate tags then count.
            let kind = if FASTQ_NAME.is_match(&file) { Kind::ReadFile } else { Kind::Genome };
            let mut run = format!("{software} {version}").trim().to_string();
            let db = format!("{db} {db_version}").trim().to_string();
            if !db.is_empty() {
                run = format!("{run}, {db}");
            }
            let genes = genes.into_iter().collect::<Vec<_>>().join(",");
            rec(
                file,
                kind,
                vec![("amr_software", Value::Text(software)), ("amr_run", Value::Text(run)), ("amr_genes", Value::Text(genes))],
            )
        })
        .collect())
}

/// nf-core/mag's `GenomeBinning/bin_summary.tsv` (`bin/combine_tables.py`), one row per bin. Two
/// layouts, matched against the pipeline's own script:
/// - 5.2.0 to 5.5.0 (checked at 5.5.0): every column but `bin` and `Depth <sample>` carries the
///   tool's suffix (`Completeness_checkm2`, `Completeness_checkm`, `Complete_busco`,
///   `classification_gtdbtk`, ...), and BUSCO, CheckM and CheckM2 can all be there.
/// - 3.0.0 to 5.1.0 (checked at 3.4.0 and 5.0.0): one bin QC tool, columns as the tool writes them
///   (`Completeness` with CheckM2's `Completeness_Model_Used` or CheckM's `Bin Id`; BUSCO's batch
///   summary `Complete` from 4.0, nf-core/mag's own `%Complete (specific)` before), and a pandas
///   index column first.
///
/// CheckM2 goes to the same fields as a CheckM2 `quality_report.tsv`, CheckM to its own; QUAST's
/// columns are not read (isotable's contig and N50 thresholds are for isolates). `Err(reason)` for
/// a table with none of the columns isotable reads, or an unsuffixed `Completeness` whose tool can't
/// be told.
pub fn mag_bin_summary(path: &Path) -> Result<std::result::Result<Vec<Record>, String>> {
    let header = first_line(path).unwrap_or_default();
    let cols: Vec<&str> = header.split('\t').map(str::trim).collect();
    let has = |c: &str| cols.contains(&c);
    let suffixed = cols.iter().any(|c| ["_busco", "_checkm", "_checkm2", "_quast", "_gtdbtk"].iter().any(|s| c.ends_with(s)));
    let pair = |a: &'static str, b: &'static str| (has(a) && has(b)).then_some((a, b));
    let checkm2 = pair("Completeness_checkm2", "Contamination_checkm2")
        .or_else(|| (!suffixed && has("Completeness_Model_Used")).then(|| pair("Completeness", "Contamination")).flatten());
    let checkm = pair("Completeness_checkm", "Contamination_checkm")
        .or_else(|| (!suffixed && has("Bin Id")).then(|| pair("Completeness", "Contamination")).flatten());
    if !suffixed && has("Completeness") && checkm2.is_none() && checkm.is_none() {
        return Ok(Err("a `Completeness` column without CheckM2's `Completeness_Model_Used` or CheckM's `Bin Id`, so which tool it is from is unclear".into()));
    }
    // (complete %, lineage) in order of preference; 3.x tables have specific and domain columns.
    let busco: Vec<(&str, &str)> = if has("Complete_busco") {
        vec![("Complete_busco", "Dataset_busco")]
    } else if !suffixed && has("Input_file") && has("Complete") {
        vec![("Complete", "Dataset")]
    } else {
        [("%Complete (specific)", "Specific lineage dataset"), ("%Complete (domain)", "Domain")].into_iter().filter(|(c, _)| has(c)).collect()
    };
    let gtdb = if has("classification_gtdbtk") { Some("classification_gtdbtk") } else { (!suffixed && has("classification")).then_some("classification") };
    let depths: Vec<&str> = cols.iter().copied().filter(|c| c.starts_with("Depth ")).collect();
    if checkm2.is_none() && checkm.is_none() && busco.is_empty() && gtdb.is_none() {
        return Ok(Err("no CheckM2, CheckM, BUSCO or GTDB-Tk columns".into()));
    }
    let mut out = Vec::new();
    for r in tsv(path)? {
        let Some(bin) = r.get("bin").filter(|b| !b.is_empty()) else { continue };
        let mut fields = Vec::new();
        for (tool, keys) in [(checkm2, ["completeness", "contamination"]), (checkm, ["checkm_completeness", "checkm_contamination"])] {
            if let Some((c, x)) = tool
                && let (Some(c), Some(x)) = (get_num(&r, c), get_num(&r, x))
            {
                fields.push((keys[0], Value::Num(c)));
                fields.push((keys[1], Value::Num(x)));
            }
        }
        if let Some((c, lineage)) = busco.iter().find_map(|(c, l)| Some((get_num(&r, c)?, r.get(*l)))) {
            fields.push(("busco_complete", Value::Num(c)));
            if let Some(l) = lineage.filter(|l| !l.is_empty()) {
                fields.push(("busco_lineage", Value::Text(l.clone())));
            }
        }
        if let Some(tax) = gtdb.and_then(|g| r.get(g)).filter(|t| !t.is_empty()) {
            fields.push(("gtdb", Value::Text(tax.clone())));
        }
        let d: Vec<String> = depths
            .iter()
            .filter_map(|c| Some(format!("{}={}", c.trim_start_matches("Depth "), r.get(*c).filter(|v| !v.is_empty())?)))
            .collect();
        if !d.is_empty() {
            fields.push(("mag_depths", Value::Text(d.join(","))));
        }
        out.push(rec(bin.clone(), Kind::Genome, fields));
    }
    Ok(Ok(out))
}

fn rec(raw_name: impl Into<String>, kind: Kind, fields: Vec<(&'static str, Value)>) -> Record {
    Record { raw_name: raw_name.into(), kind, tool: "", source: String::new(), fields }
}

fn json(path: &Path) -> Result<Json> {
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

/// Rows of a tab-separated file with a header, as column → value maps.
fn tsv(path: &Path) -> Result<Vec<BTreeMap<String, String>>> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = lines.next() else { return Ok(Vec::new()) };
    let cols: Vec<String> = header.trim_start_matches('#').split('\t').map(|c| c.trim().to_string()).collect();
    Ok(lines
        .map(|l| cols.iter().cloned().zip(l.split('\t').map(|v| v.trim().to_string())).collect())
        .collect())
}

fn get_num(row: &BTreeMap<String, String>, key: &str) -> Option<f64> {
    row.get(key)?.parse().ok()
}

pub fn fastp(path: &Path, name: &str) -> Result<Vec<Record>> {
    let j = json(path)?;
    let s = &j["summary"];
    let mut fields = Vec::new();
    if let Some(n) = s["before_filtering"]["total_reads"].as_u64() {
        fields.push(("reads_raw", Value::Int(n)));
    }
    if let Some(n) = s["after_filtering"]["total_reads"].as_u64() {
        fields.push(("reads_trimmed", Value::Int(n)));
    }
    if let Some(q) = s["after_filtering"]["q30_rate"].as_f64() {
        fields.push(("q30_trimmed", Value::Num(q * 100.0)));
    }
    if let Some(n) = s["after_filtering"]["q30_bases"].as_u64() {
        fields.push(("q30_bases", Value::Int(n)));
    }
    if let Some(n) = s["after_filtering"]["total_bases"].as_u64() {
        fields.push(("bases_trimmed", Value::Int(n)));
    }
    if let Some(d) = j["duplication"]["rate"].as_f64() {
        fields.push(("duplication", Value::Num(d * 100.0)));
    }
    // What went in, to tell a rerun on the same reads from a separate run (see `pool_fastp`).
    if let Some(b) = s["before_filtering"].as_object().filter(|b| b.contains_key("total_bases")) {
        fields.push(("raw_summary", Value::Text(Json::Object(b.clone()).to_string())));
    }
    if let Some(cmd) = j["command"].as_str() {
        let inputs = fastp_inputs(cmd);
        if !inputs.is_empty() {
            fields.push(("raw_inputs", Value::Text(inputs.join(" "))));
        }
    }
    // nf-core names fastp reports after the sample (`<sample>.fastp.json`), one per read pair.
    Ok(vec![rec(name.trim_end_matches(".fastp.json"), Kind::Sample, fields)])
}

/// Input file names (`-i/--in1`, `-I/--in2`) in fastp's recorded command line, without folders.
fn fastp_inputs(cmd: &str) -> Vec<String> {
    let words: Vec<&str> = cmd.split_whitespace().collect();
    let mut out = Vec::new();
    for (i, w) in words.iter().enumerate() {
        let value = match w.split_once('=') {
            Some(("--in1" | "--in2", v)) => Some(v),
            _ if matches!(*w, "-i" | "-I" | "--in1" | "--in2") => words.get(i + 1).copied(),
            _ => None,
        };
        if let Some(v) = value {
            out.push(v.rsplit(['/', '\\']).next().unwrap_or(v).to_string());
        }
    }
    out
}

/// leanqc's checks made for short reads (FastQC's per-position and per-read modules). leanqc runs
/// them on long reads too and says in its report that they don't fit: on the nanopore run
/// (docs/research/nanopore-laptop.md) a clean R10.4.1 sample failed per-position quality, per-position
/// base content and per-read GC. isotable doesn't grade them for long reads.
pub const SHORT_READ_CHECKS: &[&str] =
    &["Per-position quality", "Per-tile quality", "Per-read quality", "Per-position base content", "Per-read GC", "Read length"];

fn leanqc(path: &Path) -> Result<Vec<Record>> {
    let j = json(path)?;
    let file = j["file"].as_str().unwrap_or_default().to_string();
    let fails: Vec<&str> = j["checks"]
        .as_array()
        .map(|a| a.iter().filter(|c| c["status"] == "fail").filter_map(|c| c["name"].as_str()).collect())
        .unwrap_or_default();
    let mut fields = vec![("leanqc_fails", Value::Text(fails.join(",")))];
    // Long reads (median length above 1,000 bp, or leanqc --long): read count, bases, read N50
    // and the median read quality, the evidence fastp gives for short reads.
    let l = &j["long_reads"];
    if let (Some(reads), Some(bases)) = (l["reads"].as_u64(), l["bases"].as_u64()) {
        fields.push(("long_reads", Value::Int(reads)));
        fields.push(("long_bases", Value::Int(bases)));
        if let Some(n) = l["n50"].as_u64() {
            fields.push(("read_n50", Value::Int(n)));
        }
        if let Some(q) = l["read_quality"]["median"].as_f64() {
            fields.push(("read_q", Value::Num(q)));
        }
    }
    Ok(vec![rec(file, Kind::ReadFile, fields)])
}

/// FastQC (0.12.x, 0.13, FastQC-Rust): `<name>_fastqc.zip`, read without extracting.
fn fastqc_zip(path: &Path) -> Result<Vec<Record>> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    // FastQC puts everything in one `<name>_fastqc/` folder inside the zip.
    let names: Vec<String> = zip.file_names().filter_map(|n| n.ok().map(|n| n.into_owned())).collect();
    let find = |want: &str| names.iter().find(|n| n.rsplit('/').next() == Some(want) && n.matches('/').count() <= 1).cloned();
    let (Some(data_name), summary_name) = (find("fastqc_data.txt"), find("summary.txt")) else {
        bail!("no fastqc_data.txt inside the zip");
    };
    let mut read = |n: &str| -> Result<String> {
        let mut s = String::new();
        std::io::Read::take(zip.by_name(n)?, 64 << 20).read_to_string(&mut s)?;
        Ok(s)
    };
    let data = read(&data_name)?;
    let summary = summary_name.map(|n| read(&n)).transpose()?;
    let stem = path.file_name().unwrap_or_default().to_string_lossy();
    Ok(vec![fastqc(&data, summary.as_deref(), stem.trim_end_matches(".zip"))?])
}

/// FastQC's extracted `<name>_fastqc/` folder: `fastqc_data.txt` and the `summary.txt` beside it.
fn fastqc_dir(path: &Path) -> Result<Vec<Record>> {
    let data = std::fs::read_to_string(path)?;
    let dir = path.parent().unwrap_or(Path::new("."));
    let summary_path = dir.join("summary.txt");
    let summary = if summary_path.is_file() { Some(std::fs::read_to_string(summary_path)?) } else { None };
    let stem = dir.file_name().unwrap_or_default().to_string_lossy();
    Ok(vec![fastqc(&data, summary.as_deref(), &stem)?])
}

/// One FastQC report: total sequences, %GC, and failed and warned modules. Module statuses come
/// from `summary.txt` (`FAIL<TAB>module<TAB>file`), or from the `>>module<TAB>status` lines in
/// `fastqc_data.txt` when there is no summary. Named after the read file FastQC records.
fn fastqc(data: &str, summary: Option<&str>, stem: &str) -> Result<Record> {
    if !data.starts_with("##FastQC") {
        bail!("fastqc_data.txt does not start with ##FastQC");
    }
    let mut file = None;
    let mut fields = Vec::new();
    let mut from_data = Vec::new();
    for line in data.lines() {
        let Some((key, value)) = line.split_once('\t') else { continue };
        let value = value.trim();
        match key {
            "Filename" => file = Some(value.to_string()),
            "Total Sequences" => {
                if let Ok(n) = value.replace(',', "").parse() {
                    fields.push(("fastqc_sequences", Value::Int(n)));
                }
            }
            "%GC" => {
                if let Ok(x) = value.parse() {
                    fields.push(("fastqc_gc", Value::Num(x)));
                }
            }
            _ => {
                if let Some(module) = key.strip_prefix(">>") {
                    from_data.push((value.to_ascii_uppercase(), module.to_string()));
                }
            }
        }
    }
    let statuses: Vec<(String, String)> = match summary {
        Some(s) => s
            .lines()
            .filter_map(|l| {
                let mut c = l.split('\t');
                Some((c.next()?.trim().to_ascii_uppercase(), c.next()?.trim().to_string()))
            })
            .collect(),
        None => from_data,
    };
    let with = |want: &str| statuses.iter().filter(|(s, _)| s == want).map(|(_, m)| m.as_str()).collect::<Vec<_>>().join(",");
    fields.push(("fastqc_fails", Value::Text(with("FAIL"))));
    fields.push(("fastqc_warns", Value::Text(with("WARN"))));
    // FastQC records the base name of the read file; `stdin` when reading a pipe.
    let raw = file
        .map(|f| f.rsplit(['/', '\\']).next().unwrap_or(&f).to_string())
        .filter(|f| !f.is_empty() && f != "stdin")
        .unwrap_or_else(|| stem.trim_end_matches("_fastqc").to_string());
    Ok(rec(raw, Kind::ReadFile, fields))
}

fn quast(path: &Path) -> Result<Vec<Record>> {
    Ok(tsv(path)?
        .into_iter()
        .filter_map(|r| {
            let name = r.get("Assembly")?.clone();
            let mut fields = Vec::new();
            if let Some(v) = get_num(&r, "# contigs") {
                fields.push(("contigs", Value::Int(v as u64)));
            }
            if let Some(v) = get_num(&r, "Total length") {
                fields.push(("length", Value::Int(v as u64)));
            }
            if let Some(v) = get_num(&r, "N50") {
                fields.push(("n50", Value::Int(v as u64)));
            }
            if let Some(v) = get_num(&r, "GC (%)") {
                fields.push(("gc", Value::Num(v)));
            }
            // The same names as the statistics isotable computes from an assembly, so the two agree.
            for (col, key) in [("Largest contig", "largest"), ("L50", "l50"), ("N90", "n90"), ("# contigs (>= 1000 bp)", "contigs_1kb")] {
                if let Some(v) = get_num(&r, col) {
                    fields.push((key, Value::Int(v as u64)));
                }
            }
            if let Some(v) = get_num(&r, "# N's per 100 kbp") {
                fields.push(("ns_per_100kbp", Value::Num(v)));
            }
            Some(rec(name, Kind::Genome, fields))
        })
        .collect())
}

/// An assembly's FASTA (Shovill's `contigs.fa`, Flye's `assembly.fasta`, Medaka's
/// `consensus.fasta`), named after its folder as these tools always use the same file name.
/// Statistics are counted the way QUAST reports them (see [`crate::asm`]: contigs of 500 bp or
/// more; GC over A, C, G and T), so a QUAST report on the same assembly agrees. With Shovill's log,
/// also the sequencing depth Shovill estimated: read bases over the genome size it used, which is
/// its own k-mer estimate unless `--gsize` was given (docs/research/coverage-depth.md: up to 3x off).
/// The contig length cut-off for assembly statistics (`isotable --min-contig`); QUAST's 500 bp
/// unless set.
pub static MIN_CONTIG: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(crate::asm::QUAST_MIN_CONTIG);

fn assembly(path: &Path, shovill_log: Option<&Path>) -> Result<Vec<Record>> {
    let name = path
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .context("the assembly has no folder to name the sample")?;
    let s = crate::asm::stats(path, MIN_CONTIG.load(std::sync::atomic::Ordering::Relaxed))?;
    let mut fields = vec![("contigs", Value::Int(s.contigs)), ("length", Value::Int(s.length))];
    if s.contigs > 0 {
        fields.push(("n50", Value::Int(s.n50)));
    }
    fields.extend(s.gc.map(|g| ("gc", Value::Num(g))));
    if s.contigs > 0 {
        fields.extend([("largest", Value::Int(s.largest)), ("l50", Value::Int(s.l50)), ("n90", Value::Int(s.n90))]);
        fields.push(("ns_per_100kbp", Value::Num(s.ns_per_100kbp)));
    }
    fields.extend([("contigs_1kb", Value::Int(s.contigs_1kb)), ("contigs_all", Value::Int(s.contigs_all))]);
    if let Some(log) = shovill_log {
        let text = std::fs::read_to_string(log)?;
        fields.extend(shovill_depth(&text).map(|d| ("shovill_depth", Value::Num(d))));
    }
    Ok(vec![rec(name, Kind::Genome, fields)])
}

/// `[shovill] Estimated sequencing depth: 70 x` (Shovill 1.1-1.4), the depth before any subsampling.
pub fn shovill_depth(log: &str) -> Option<f64> {
    log.lines()
        .find_map(|l| l.trim().strip_prefix("[shovill] Estimated sequencing depth: "))
        .and_then(|r| r.split_whitespace().next())
        .and_then(|n| n.trim_end_matches('x').parse().ok())
}

/// `coverm genome` with methods mean and covered_fraction (CoverM 0.8.0: `Genome`, then `<sample>
/// Mean` and `<sample> Covered Fraction` per read set). A table of one genome is one sample's,
/// named after the file (`<sample>.coverm.tsv`): with `--single-genome` CoverM calls the genome
/// `genome1`. Several genomes are named after their rows. Depths of several read sets add up; the
/// covered fraction is given for one read set only. `None` for any other table (`coverm contig`).
pub fn coverm_genome(path: &Path, name: &str) -> Result<Option<Vec<Record>>> {
    let text = std::fs::read_to_string(path)?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = lines.next() else { return Ok(None) };
    let cols: Vec<&str> = header.split('\t').collect();
    if cols.first() != Some(&"Genome") {
        return Ok(None);
    }
    let means: Vec<usize> = (1..cols.len()).filter(|&i| cols[i].ends_with(" Mean")).collect();
    let covered: Vec<usize> = (1..cols.len()).filter(|&i| cols[i].ends_with(" Covered Fraction")).collect();
    if means.is_empty() {
        return Ok(None);
    }
    let rows: Vec<Vec<&str>> = lines.map(|l| l.split('\t').collect::<Vec<_>>()).filter(|r| r[0] != "unmapped").collect();
    let stem = name.split(".coverm").next().unwrap_or(name).split("_coverm").next().unwrap_or(name).trim_end_matches(".tsv");
    let mut out = Vec::new();
    for r in &rows {
        let num = |i: usize| -> Result<f64> {
            r.get(i).and_then(|v| v.trim().parse().ok()).with_context(|| format!("no number in column {} of row {}", cols[i], r[0]))
        };
        let depth: f64 = means.iter().map(|&i| num(i)).sum::<Result<f64>>()?;
        let mut fields = vec![("mapped_depth", Value::Num(depth))];
        if let [i] = covered[..] {
            fields.push(("covered_fraction", Value::Num(num(i)? * 100.0)));
        }
        let raw = if rows.len() == 1 && !stem.is_empty() { stem.to_string() } else { r[0].to_string() };
        out.push(rec(raw, Kind::Genome, fields));
    }
    Ok(Some(out))
}

/// Kleborate 3's output tables, one per species group: `klebsiella_pneumo_complex_output.txt`,
/// `klebsiella_oxytoca_complex_output.txt`, `escherichia_output.txt`. Kleborate 3.2.4 writes each
/// assembly to the table of its species, whatever the preset.
pub const KLEBORATE_OUTPUTS: &[&str] = &["klebsiella_pneumo_complex_output.txt", "klebsiella_oxytoca_complex_output.txt", "escherichia_output.txt"];

/// The species of the *K. pneumoniae* species complex (KpSC), as Kleborate names them.
pub const KPSC_SPECIES: &[&str] =
    &["Klebsiella pneumoniae", "Klebsiella quasipneumoniae", "Klebsiella variicola", "Klebsiella quasivariicola", "Klebsiella africana"];

/// Kleborate column (the part after the last `__` of its full header, the whole name with
/// `--trim_headers`) and isotable's field.
pub const KLEBORATE_FIELDS: &[(&str, &str)] = &[
    ("species", "kleborate_species"),
    ("species_match", "kleborate_species_match"),
    ("ST", "kleborate_st"),
    ("virulence_score", "virulence_score"),
    ("resistance_score", "resistance_score"),
    ("K_locus", "k_locus"),
    ("O_locus", "o_locus"),
    ("Yersiniabactin", "ybt"),
    ("Colibactin", "clb"),
    ("Aerobactin", "iuc"),
    ("Salmochelin", "iro"),
    ("RmpADC", "rmpadc"),
];

/// Kleborate 3's table: one row per assembly, named by its `strain` column (the assembly's file
/// name without the extension).
pub fn kleborate(path: &Path) -> Result<Vec<Record>> {
    let rows = tsv(path)?;
    let mut out = Vec::new();
    for r in rows {
        let Some(strain) = r.get("strain").filter(|s| !s.is_empty()) else { continue };
        let mut fields = Vec::new();
        for (col, key) in KLEBORATE_FIELDS {
            let v = r.iter().find(|(k, _)| k.rsplit("__").next() == Some(col)).map(|(_, v)| v.trim());
            if let Some(v) = v.filter(|v| !v.is_empty()) {
                fields.push((*key, Value::Text(v.to_string())));
            }
        }
        out.push(rec(strain.clone(), Kind::Genome, fields));
    }
    if out.is_empty() && !std::fs::read_to_string(path)?.lines().next().is_some_and(|h| h.split('\t').any(|c| c == "strain")) {
        bail!("no strain column; not a Kleborate 3 table");
    }
    Ok(out)
}

fn busco(path: &Path, name: &str) -> Result<Vec<Record>> {
    let j = json(path)?;
    // short_summary.specific.bacteria_odb10.<sample>.json
    let rest = name.trim_end_matches(".json");
    let rest = rest.strip_prefix("short_summary.specific.").or_else(|| rest.strip_prefix("short_summary.generic.")).unwrap_or(rest);
    let (lineage, sample) = rest.split_once('.').unwrap_or(("", rest));
    let lineage = j["lineage_dataset"]["name"].as_str().unwrap_or(lineage).to_string();
    let mut fields = vec![("busco_lineage", Value::Text(lineage))];
    if let Some(c) = j["results"]["Complete percentage"].as_f64() {
        fields.push(("busco_complete", Value::Num(c)));
    }
    Ok(vec![rec(sample, Kind::Genome, fields)])
}

pub fn checkm2(path: &Path) -> Result<Vec<Record>> {
    Ok(tsv(path)?
        .into_iter()
        .filter_map(|r| {
            Some(rec(
                r.get("Name")?.clone(),
                Kind::Genome,
                vec![
                    ("completeness", Value::Num(get_num(&r, "Completeness")?)),
                    ("contamination", Value::Num(get_num(&r, "Contamination")?)),
                ],
            ))
        })
        .collect())
}

/// GTDB-Tk's `<prefix>.<marker>.summary.tsv` (`classification`), or the ANI screen's
/// `classify/ani_screen/<prefix>.<marker>.ani_summary.tsv` (`reference_taxonomy`), which is all
/// there is when the screen assigned every genome (gtdbprep ani, or classify_wf with nothing left
/// to place). For a screened genome GTDB-Tk's summary gives the same taxonomy, so the two agree.
pub fn gtdbtk(path: &Path) -> Result<Vec<Record>> {
    Ok(tsv(path)?
        .into_iter()
        .filter_map(|r| {
            let tax = r.get("classification").or_else(|| r.get("reference_taxonomy"))?;
            Some(rec(r.get("user_genome")?.clone(), Kind::Genome, vec![("gtdb", Value::Text(tax.clone()))]))
        })
        .collect())
}

/// A Kraken 2 report line: percent, clade reads, direct reads, [minimizer columns,] rank, taxid, name.
fn kraken_line(line: &str) -> Option<(f64, &str, &str)> {
    let cols: Vec<&str> = line.split('\t').collect();
    if cols.len() != 6 && cols.len() != 8 {
        return None;
    }
    let pct: f64 = cols[0].trim().parse().ok()?;
    cols[1].trim().parse::<u64>().ok()?;
    cols[2].trim().parse::<u64>().ok()?;
    let rank = cols[cols.len() - 3].trim();
    let rank_ok = rank == "-" || {
        let mut c = rank.chars();
        c.next().is_some_and(|r| "URDKPCOFGS".contains(r)) && c.all(|d| d.is_ascii_digit())
    };
    if !rank_ok || cols[cols.len() - 2].trim().parse::<u64>().is_err() {
        return None;
    }
    Some((pct, rank, cols[cols.len() - 1]))
}

/// Kraken 2 report: takes the top species. `None` if the file isn't a report (e.g. the
/// per-read `*.classifiedreads.txt` output, which can be gigabytes): only the first line is
/// read in that case, and reports are streamed, never loaded whole.
fn kraken(path: &Path, name: &str) -> Result<Option<Record>> {
    let reader = std::io::BufReader::new(std::fs::File::open(path)?);
    let mut best: Option<(f64, String)> = None;
    let mut first = true;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Some((pct, rank, taxon)) = kraken_line(&line) else {
            if first {
                return Ok(None);
            }
            bail!("line not in Kraken 2 report format: {}", line.chars().take(80).collect::<String>());
        };
        first = false;
        if rank != "S" {
            continue;
        }
        if best.as_ref().is_none_or(|(p, _)| pct > *p) {
            best = Some((pct, taxon.trim().to_string()));
        }
    }
    if first {
        return Ok(None);
    }
    let raw = name.split(".kraken2").next().unwrap_or(name).split(".kreport").next().unwrap_or(name).to_string();
    let fields = match best {
        Some((p, sp)) => vec![("kraken_top", Value::Text(sp)), ("kraken_top_pct", Value::Num(p))],
        None => vec![("kraken_top", Value::Text("no species-level hit".into()))],
    };
    Ok(Some(rec(raw, Kind::Sample, fields)))
}

/// mlst: FILE, SCHEME, ST, alleles... (tab-separated, header optional).
pub fn mlst(path: &Path) -> Result<Vec<Record>> {
    let text = std::fs::read_to_string(path)?;
    Ok(text
        .lines()
        .filter(|l| !l.starts_with("FILE\t") && !l.trim().is_empty())
        .filter_map(|l| {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() < 3 {
                return None;
            }
            let file = Path::new(c[0]).file_name()?.to_string_lossy().to_string();
            Some(rec(file, Kind::Genome, vec![("mlst_scheme", Value::Text(c[1].into())), ("mlst_st", Value::Text(c[2].into()))]))
        })
        .collect())
}

/// The AMR hits (type AMR) of an AMRFinderPlus report, one entry per element symbol and name,
/// sorted. A symbol shared by hits with different names (`blaOXA` for three OXA families when no
/// allele matched exactly) gets the name in brackets, and a hit found more than once gets
/// `x<count>`, so distinct genes stay distinct. Commas in a name become `;` (the list is
/// comma-separated).
pub fn amrfinder_genes(rows: &[BTreeMap<String, String>]) -> Vec<String> {
    let mut found: BTreeMap<&str, BTreeMap<String, u64>> = BTreeMap::new();
    for r in rows {
        let t = r.get("Element type").or_else(|| r.get("Type")).map(String::as_str).unwrap_or("AMR");
        let Some(symbol) = r.get("Element symbol").or_else(|| r.get("Gene symbol")) else { continue };
        if t != "AMR" || symbol.is_empty() {
            continue;
        }
        let name = r.get("Element name").or_else(|| r.get("Sequence name")).map(|n| n.replace(',', ";")).unwrap_or_default();
        *found.entry(symbol.as_str()).or_default().entry(name).or_default() += 1;
    }
    let mut genes = Vec::new();
    for (symbol, names) in found {
        for (name, count) in &names {
            let mut g = symbol.to_string();
            if names.len() > 1 {
                g += &format!(" ({name})");
            }
            if *count > 1 {
                g += &format!(" x{count}");
            }
            genes.push(g);
        }
    }
    genes
}

/// AMRFinderPlus: one file per sample; lists AMR genes (column names changed in 4.x).
pub fn amrfinder(path: &Path, name: &str) -> Result<Vec<Record>> {
    let genes = amrfinder_genes(&tsv(path)?);
    let raw = name.split(".amrfinder").next().unwrap_or(name).split("_amrfinder").next().unwrap_or(name);
    let raw = raw.trim_end_matches(".tsv").to_string();
    let value = if genes.is_empty() { "none".to_string() } else { genes.join(",") };
    Ok(vec![rec(raw, Kind::Genome, vec![("amr_genes", Value::Text(value))])])
}

/// Sample names from a sample sheet: the `sample`/`ID`/`sample-id` column, else the first.
pub fn sheet_samples(path: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#'));
    let header = lines.next().context("empty sample sheet")?;
    let sep = if header.contains('\t') { '\t' } else { ',' };
    let cols: Vec<String> = header.split(sep).map(|c| c.trim().to_ascii_lowercase()).collect();
    let idx = cols.iter().position(|c| matches!(c.as_str(), "sample" | "id" | "sample-id" | "sample_id")).unwrap_or(0);
    let mut names: Vec<String> = lines.filter_map(|l| l.split(sep).nth(idx).map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect();
    names.sort();
    names.dedup();
    Ok(names)
}

/// Read files a sample sheet names (columns such as `fastq_1`/`fastq_2` of nf-core and sheetgen
/// sheets): each FASTQ file's name without folders and extensions, mapped to its row's sample. QC
/// reports are named after the read file, which need not start with the sample name
/// (`SRR30733615_1.fastq.gz` for sample BAE-1).
pub fn sheet_read_files(path: &Path) -> Result<BTreeMap<String, String>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty() && !l.starts_with('#'));
    let header = lines.next().context("empty sample sheet")?;
    let sep = if header.contains('\t') { '\t' } else { ',' };
    let cols: Vec<String> = header.split(sep).map(|c| c.trim().to_ascii_lowercase()).collect();
    let idx = cols.iter().position(|c| matches!(c.as_str(), "sample" | "id" | "sample-id" | "sample_id")).unwrap_or(0);
    let fastq = Regex::new(r"(?i)\.(fastq|fq)(\.(gz|bz2|zst))?$").unwrap();
    let mut out = BTreeMap::new();
    for line in lines {
        let cells: Vec<&str> = line.split(sep).map(str::trim).collect();
        let Some(sample) = cells.get(idx).filter(|s| !s.is_empty()) else { continue };
        for (i, cell) in cells.iter().enumerate() {
            if i == idx || !fastq.is_match(cell) {
                continue;
            }
            let file = cell.rsplit(['/', '\\']).next().unwrap_or(cell);
            out.insert(base_name(file), sample.to_string());
        }
    }
    Ok(out)
}

/// File extensions and assembler tags: stripped from any record's name.
static TOOL_SUFFIX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(\.(fastq|fq|fasta|fa|fna|gz|bz2|zst|scaffolds|contigs|assembly))$|([._-](unicycler|spades|skesa|shovill|dragonflye|flye|canu|megahit|metaspades))$",
    )
    .unwrap()
});
/// Subsampling tags such as `_1M`, `_500k`.
static SUBSAMPLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)[._-]\d+(\.\d+)?[km]$").unwrap());
/// Read-file mate, lane and index tags (`_R1`, `_1`, `_R1_001`, `_L001`, `_S3`).
static READ_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([._-]R?[12](_\d{3})?|_L\d{3}|_S\d+)$").unwrap());
/// Suffixes trimmers add to a read file's name: fastp (`x_1.fastp.fastq.gz`), Trim Galore
/// (`x_1_val_1.fq.gz`, single-end `x_trimmed.fq.gz`) and the common `.trimmed`.
static TRIMMED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(\.fastp|_val_[12]|[._]trimmed)$").unwrap());
static MATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?P<sep>[._-])(?P<r>R?)(?P<n>[12])(?P<tail>(_\d{3})?)$").unwrap());

/// How a record was assigned to a sample.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum How {
    Exact,
    /// Sheet name plus suffixes recognised as tool, file or read tags.
    Prefix,
    /// No sheet: extensions, assembler tags (and, for read files, mate suffixes) stripped.
    Stripped,
    /// A sheet was given and nothing in it fits.
    Unmatched,
}

/// The name with extensions and assembler tags removed.
pub fn base_name(raw: &str) -> String {
    let mut name = raw.to_string();
    loop {
        let next = TOOL_SUFFIX.replace(&name, "").to_string();
        if next == name || next.is_empty() {
            return name;
        }
        name = next;
    }
}

/// A trimmed read file's name without the trimmer's suffix (and extensions), or `None` if the
/// name carries no trimmer suffix.
pub fn untrimmed(raw: &str) -> Option<String> {
    let base = base_name(raw);
    let m = TRIMMED.find(&base)?;
    let stem = base_name(&base[..m.start()]);
    (!stem.is_empty()).then_some(stem)
}

/// `x_R1` → (`x`, `x_R2`): the name without its mate suffix, and the other mate's name.
pub fn mate_parts(name: &str) -> Option<(String, String)> {
    let c = MATE.captures(name)?;
    let other = if &c["n"] == "1" { "2" } else { "1" };
    let start = c.get(0).unwrap().start();
    Some((name[..start].to_string(), format!("{}{}{}{other}{}", &name[..start], &c["sep"], &c["r"], &c["tail"])))
}

/// Does `rest` consist only of suffixes a tool may add to a sample name?
fn only_known_suffixes(rest: &str, kind: Kind) -> bool {
    let mut rest = rest.to_string();
    while !rest.is_empty() {
        let next = [&*TOOL_SUFFIX, &*SUBSAMPLE]
            .into_iter()
            .chain((kind == Kind::ReadFile).then_some(&*READ_TAG))
            .map(|re| re.replace(&rest, "").to_string())
            .find(|n| n.len() < rest.len());
        match next {
            Some(n) => rest = n,
            None => return false,
        }
    }
    true
}

/// The sample a record belongs to.
///
/// With a sheet: the sheet name the record's name equals, or else the longest sheet name it starts
/// with when everything after it is a recognised suffix (`ERR044595-unicycler.scaffolds`,
/// `ERR044595_1M_1.fastq.gz`). Mate, lane and index tags count only for read files, so an
/// assembly called `patient_1` never joins sample `patient`. Anything else is unmatched.
///
/// Without a sheet: extensions and assembler tags are stripped; a read file's mate suffix is
/// stripped when `strip_mate` (the caller has seen the other mate's report).
pub fn match_sample(raw: &str, kind: Kind, sheet: &[String], strip_mate: bool) -> (String, How) {
    if !sheet.is_empty() {
        if sheet.iter().any(|s| s == raw) {
            return (raw.to_string(), How::Exact);
        }
        let fit = sheet
            .iter()
            .filter(|s| raw.starts_with(s.as_str()) && only_known_suffixes(&raw[s.len()..], kind))
            .max_by_key(|s| s.len());
        return match fit {
            Some(s) => (s.clone(), How::Prefix),
            None => (base_name(raw), How::Unmatched),
        };
    }
    let base = base_name(raw);
    if kind == Kind::ReadFile && strip_mate && let Some((stem, _)) = mate_parts(&base) {
        return (stem, How::Stripped);
    }
    let how = if base == raw { How::Exact } else { How::Stripped };
    (base, how)
}

/// The sheet sample a MAG bin was binned from: the longest sheet name that appears in the bin's
/// name as a whole part, at the start or after `-`, `_` or `.`, and followed by one of them.
/// MetaBAT2 with the sample as prefix (`D01-S1.12`), SemiBin2 with the sample as tag
/// (`D01-S1_3`) or multi-sample (`D01-S1_SemiBin_3`), and nf-core/mag (`MEGAHIT-MetaBAT2-D01-S1.12`)
/// all name bins so. `None` when no sheet name fits (MetaBAT2's default `bin.12`).
pub fn bin_sample(name: &str, sheet: &[String]) -> Option<String> {
    let sep = |c: char| matches!(c, '-' | '_' | '.');
    let fits = |s: &str| {
        name.match_indices(s).any(|(i, _)| {
            name[..i].chars().next_back().is_none_or(sep) && name[i + s.len()..].chars().next().is_some_and(sep)
        })
    };
    sheet.iter().filter(|s| !s.is_empty() && fits(s)).max_by_key(|s| s.len()).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amrfinder_hits_sharing_a_symbol_stay_apart() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("s1.amrfinder.tsv");
        std::fs::write(
            &p,
            "Contig id\tElement symbol\tElement name\tType\n\
             c1\tblaOXA\tOXA-48 family carbapenem-hydrolyzing class D beta-lactamase\tAMR\n\
             c2\tblaOXA\tOXA-1 family class D beta-lactamase\tAMR\n\
             c3\ttet(A)\ttetracycline efflux MFS transporter Tet(A)\tAMR\n\
             c4\ttet(A)\ttetracycline efflux MFS transporter Tet(A)\tAMR\n\
             c5\tmecA\tPBP2a, methicillin resistance\tAMR\n\
             c6\tsea\tenterotoxin type A\tVIRULENCE\n",
        )
        .unwrap();
        let r = amrfinder(&p, "s1.amrfinder.tsv").unwrap();
        assert_eq!(r[0].raw_name, "s1");
        assert_eq!(
            r[0].fields,
            vec![(
                "amr_genes",
                Value::Text(
                    "blaOXA (OXA-1 family class D beta-lactamase),blaOXA (OXA-48 family carbapenem-hydrolyzing class D beta-lactamase),mecA,tet(A) x2"
                        .into()
                )
            )]
        );
    }

    #[test]
    fn bins_of_sheet_samples() {
        let sheet: Vec<String> = ["D01-S1", "D01-S10", "D01", "1"].map(String::from).to_vec();
        let b = |name| bin_sample(name, &sheet);
        assert_eq!(b("D01-S1.12").as_deref(), Some("D01-S1"));
        assert_eq!(b("D01-S10.3").as_deref(), Some("D01-S10"));
        assert_eq!(b("D01-S1_3").as_deref(), Some("D01-S1"));
        assert_eq!(b("D01-S1_SemiBin_3").as_deref(), Some("D01-S1"));
        assert_eq!(b("MEGAHIT-MetaBAT2-D01-S1.12").as_deref(), Some("D01-S1"));
        // a sheet name must be followed by a separator: `bin.1` is not a bin of sample `1`
        assert_eq!(b("bin.1"), None);
        assert_eq!(b("D01-S100.1").as_deref(), Some("D01"));
        assert_eq!(b("XD01-S2.1"), None);
    }

    #[test]
    fn names_without_sheet() {
        let m = |raw, kind, strip| match_sample(raw, kind, &[], strip).0;
        assert_eq!(m("ERR044595-unicycler.scaffolds", Kind::Genome, false), "ERR044595");
        assert_eq!(m("ERR044595-unicycler.scaffolds.fa", Kind::Genome, false), "ERR044595");
        assert_eq!(m("iso_7_R1.fastq.gz", Kind::ReadFile, true), "iso_7");
        assert_eq!(m("iso_7_R1.fastq.gz", Kind::ReadFile, false), "iso_7_R1");
        assert_eq!(m("bin.12.fa", Kind::Genome, false), "bin.12");
        // Assemblies and per-sample reports keep their _1/_2.
        assert_eq!(m("patient_1", Kind::Genome, true), "patient_1");
        assert_eq!(m("patient_2", Kind::Sample, true), "patient_2");
    }

    #[test]
    fn names_with_sheet() {
        let sheet = vec!["ERR044595".to_string(), "ERR0445".to_string(), "patient".to_string()];
        let m = |raw, kind| match_sample(raw, kind, &sheet, false);
        assert_eq!(m("ERR044595_1M_1.fastq.gz", Kind::ReadFile), ("ERR044595".into(), How::Prefix));
        assert_eq!(m("ERR044595-unicycler", Kind::Genome), ("ERR044595".into(), How::Prefix));
        assert_eq!(m("ERR044595-unicycler.scaffolds.fa", Kind::Genome), ("ERR044595".into(), How::Prefix));
        assert_eq!(m("ERR044595", Kind::Sample), ("ERR044595".into(), How::Exact));
        assert_eq!(m("ERR04459X", Kind::Genome).1, How::Unmatched);
        // `patient_1` is another sample, not a mate or tag of `patient`.
        assert_eq!(m("patient_1", Kind::Genome).1, How::Unmatched);
        assert_eq!(m("patient_2", Kind::Sample).1, How::Unmatched);
        assert_eq!(m("patient_R1.fastq.gz", Kind::ReadFile), ("patient".into(), How::Prefix));
        assert_eq!(m("patient_extra", Kind::ReadFile).1, How::Unmatched);
    }

    #[test]
    fn trimmer_suffixes() {
        assert_eq!(untrimmed("S_1.fastp.fastq.gz").as_deref(), Some("S_1"));
        assert_eq!(untrimmed("S_1_val_1.fq.gz").as_deref(), Some("S_1"));
        assert_eq!(untrimmed("S_R2_val_2.fq.gz").as_deref(), Some("S_R2"));
        assert_eq!(untrimmed("S_trimmed.fq.gz").as_deref(), Some("S"));
        assert_eq!(untrimmed("S_1.trimmed.fastq").as_deref(), Some("S_1"));
        assert_eq!(untrimmed("S_1.fastq.gz"), None);
        assert_eq!(untrimmed("value_1.fq.gz"), None);
        assert_eq!(untrimmed("_trimmed.fq"), None);
    }

    #[test]
    fn mates() {
        assert_eq!(mate_parts("x_R1"), Some(("x".into(), "x_R2".into())));
        assert_eq!(mate_parts("x_2"), Some(("x".into(), "x_1".into())));
        assert_eq!(mate_parts("x_R1_001"), Some(("x".into(), "x_R2_001".into())));
        assert_eq!(mate_parts("x"), None);
    }

    #[test]
    fn fastqc_statuses_from_summary_or_module_lines() {
        let data = "##FastQC\t0.12.1\n>>Basic Statistics\tpass\n#Measure\tValue\nFilename\treads/x_R1.fq.gz\nTotal Sequences\t42\n%GC\t51\n>>END_MODULE\n>>Per base sequence quality\tfail\n>>END_MODULE\n>>Adapter Content\twarn\n>>END_MODULE\n";
        let get = |r: &Record, k: &str| r.fields.iter().find(|(f, _)| *f == k).map(|(_, v)| v.show()).unwrap();
        // No summary.txt: the module lines decide.
        let r = fastqc(data, None, "x_R1_fastqc").unwrap();
        assert_eq!((r.raw_name.as_str(), r.kind), ("x_R1.fq.gz", Kind::ReadFile));
        assert_eq!((get(&r, "fastqc_sequences"), get(&r, "fastqc_gc")), ("42".into(), "51".into()));
        assert_eq!((get(&r, "fastqc_fails"), get(&r, "fastqc_warns")), ("Per base sequence quality".into(), "Adapter Content".into()));
        // summary.txt wins when present.
        let r = fastqc(data, Some("PASS\tBasic Statistics\tx\nFAIL\tAdapter Content\tx\n"), "x").unwrap();
        assert_eq!((get(&r, "fastqc_fails"), get(&r, "fastqc_warns")), ("Adapter Content".into(), String::new()));
        // Reading stdin: named after the report instead.
        let r = fastqc(&data.replace("reads/x_R1.fq.gz", "stdin"), None, "y_2_fastqc").unwrap();
        assert_eq!(r.raw_name, "y_2");
        assert!(fastqc("not fastqc", None, "x").is_err());
    }

    #[test]
    fn shovill_contigs_counted_like_quast() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("s1");
        std::fs::create_dir(&dir).unwrap();
        // 1000 + 600 + 499 (left out, as QUAST does) bp; N50 is 1000
        let fa = format!(">c1 len=1000 sw=shovill-skesa/1.4.2 date=20261112\n{}\n{}\n>c2\n{}\n>c3\n{}\n", "A".repeat(600), "C".repeat(400), "G".repeat(600), "T".repeat(499));
        std::fs::write(dir.join("contigs.fa"), fa).unwrap();
        // without shovill.log beside it, a contigs.fa is not taken as Shovill's
        assert!(scan(&[t.path().to_path_buf()]).unwrap().0.is_empty());
        std::fs::write(dir.join("shovill.log"), "[shovill] Using genome size 1600 bp\n[shovill] Estimated sequencing depth: 61 x\n").unwrap();
        let (records, seen, _) = scan(&[t.path().to_path_buf()]).unwrap();
        assert_eq!(seen["Shovill"], 1);
        assert_eq!(records[0].raw_name, "s1");
        // GC over the two counted contigs: 400 C + 600 G of 1600
        assert_eq!(
            records[0].fields,
            [
                ("contigs", Value::Int(2)),
                ("length", Value::Int(1600)),
                ("n50", Value::Int(1000)),
                ("gc", Value::Num(62.5)),
                ("largest", Value::Int(1000)),
                ("l50", Value::Int(1)),
                ("n90", Value::Int(600)),
                ("ns_per_100kbp", Value::Num(0.0)),
                ("contigs_1kb", Value::Int(1)),
                ("contigs_all", Value::Int(3)),
                ("shovill_depth", Value::Num(61.0)),
            ]
        );
        std::fs::write(dir.join("contigs.fa"), "ACGT\n").unwrap();
        assert!(scan(&[t.path().to_path_buf()]).is_err());
    }

    #[test]
    fn long_read_assemblies_and_polished_drafts() {
        let t = tempfile::tempdir().unwrap();
        let (flye, medaka) = (t.path().join("flye/s1"), t.path().join("medaka/s1"));
        std::fs::create_dir_all(&flye).unwrap();
        std::fs::create_dir_all(&medaka).unwrap();
        std::fs::write(flye.join("assembly.fasta"), format!(">contig_1\n{}\n>contig_2\n{}\n", "A".repeat(5000), "C".repeat(800))).unwrap();
        // without flye.log beside it, an assembly.fasta is not taken as Flye's
        assert!(scan(&[t.path().to_path_buf()]).unwrap().0.is_empty());
        std::fs::write(flye.join("flye.log"), "").unwrap();
        let (records, seen, _) = scan(&[t.path().to_path_buf()]).unwrap();
        assert_eq!((seen["Flye"], records.len()), (1, 1));
        assert_eq!(records[0].raw_name, "s1");
        // GC as for Shovill: 800 C of 5800 counted bases; no Shovill log, so no depth estimate
        assert_eq!(records[0].fields[..4], [("contigs", Value::Int(2)), ("length", Value::Int(5800)), ("n50", Value::Int(5000)), ("gc", Value::Num(13.79))]);
        assert!(!records[0].fields.iter().any(|(k, _)| *k == "shovill_depth"));
        // The Medaka consensus of the same sample replaces the draft (a few bases differ).
        std::fs::write(medaka.join("consensus.fasta"), format!(">contig_1\n{}\n>contig_2\n{}\n", "A".repeat(5001), "C".repeat(800))).unwrap();
        std::fs::write(medaka.join("consensus_probs.hdf"), "").unwrap();
        let (records, seen, skipped) = scan(&[t.path().to_path_buf()]).unwrap();
        assert_eq!((seen["Flye"], seen["Medaka"]), (1, 1));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].tool, "Medaka");
        assert_eq!(records[0].fields[1], ("length", Value::Int(5801)));
        assert!(skipped.iter().any(|s| s.contains("Flye draft of s1")), "{skipped:?}");
    }

    #[test]
    fn kraken_reports_are_recognised_by_content() {
        let dir = std::env::temp_dir().join(format!("isotable-kraken-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let report = dir.join("s1.kraken2.report.txt");
        std::fs::write(&report, " 10.00\t100\t100\tU\t0\tunclassified\n 90.00\t900\t0\tR\t1\troot\n 80.00\t800\t800\tS\t562\t      Escherichia coli\n 5.00\t50\t50\tS1\t9\t        strain x\n").unwrap();
        let r = kraken(&report, "s1.kraken2.report.txt").unwrap().unwrap();
        assert_eq!(r.raw_name, "s1");
        assert_eq!(r.fields[0], ("kraken_top", Value::Text("Escherichia coli".into())));
        // Per-read output: C/U, read id, taxid, length, LCA mappings.
        let reads = dir.join("s1.kraken2.classifiedreads.txt");
        std::fs::write(&reads, "C\tread1\t562\t150\t562:116\nU\tread2\t0\t150\t0:116\n").unwrap();
        assert!(kraken(&reads, "s1.kraken2.classifiedreads.txt").unwrap().is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn gtdb_ani_screen_summary_gives_the_classification() {
        let t = tempfile::tempdir().unwrap();
        let dir = t.path().join("classify/ani_screen");
        std::fs::create_dir_all(&dir).unwrap();
        let tax = "d__Bacteria;p__Pseudomonadota;c__Gammaproteobacteria;o__Enterobacterales;f__Enterobacteriaceae;g__Klebsiella;s__Klebsiella pneumoniae";
        // as GTDB-Tk 2.8 writes it (other_related_references shortened)
        std::fs::write(
            dir.join("gtdbtk.bac120.ani_summary.tsv"),
            format!("user_genome\treference_genome\tskani_ani\tskani_af\treference_taxonomy\tother_related_references(genome_id,species_name,radius,ANI,AF)\ns1\tGCF_000742135.1\t99.14\t0.8736\t{tax}\tGCF_020526085.1, s__Klebsiella africana, 95.17, 95.18, 0.826\n"),
        )
        .unwrap();
        let (records, seen, _) = scan(&[t.path().to_path_buf()]).unwrap();
        assert_eq!(seen["GTDB-Tk"], 1);
        assert_eq!(records[0].raw_name, "s1");
        assert_eq!(records[0].fields, [("gtdb", Value::Text(tax.into()))]);
        // classify_wf's summary for the same genome says the same, so the two don't conflict
        std::fs::write(t.path().join("classify/gtdbtk.bac120.summary.tsv"), format!("user_genome\tclassification\ns1\t{tax}\n")).unwrap();
        let (records, _, _) = scan(&[t.path().to_path_buf()]).unwrap();
        assert!(records.iter().all(|r| r.fields == [("gtdb", Value::Text(tax.into()))]));
    }

    #[test]
    fn coverm_genome_tables() {
        let t = tempfile::tempdir().unwrap();
        // as coverm genome --single-genome --methods mean covered_fraction wrote it on the laptop
        let one = t.path().join("BfR-EC-20867.coverm.tsv");
        std::fs::write(&one, "Genome\tcontigs.fa/BfR-EC-20867_R1.fastq.gz Mean\tcontigs.fa/BfR-EC-20867_R1.fastq.gz Covered Fraction\ngenome1\t50.337517\t0.9999248\n").unwrap();
        let r = coverm_genome(&one, "BfR-EC-20867.coverm.tsv").unwrap().unwrap();
        assert_eq!(r[0].raw_name, "BfR-EC-20867");
        assert_eq!(r[0].fields[0], ("mapped_depth", Value::Num(50.337517)));
        assert_eq!(r[0].fields[1].1.show(), "99.99");
        // two read sets add up; several genomes keep their own names
        let two = t.path().join("x_coverm.tsv");
        std::fs::write(&two, "Genome\ta Mean\tb Mean\na\t10\t5\nb\t1\t2\n").unwrap();
        let r = coverm_genome(&two, "x_coverm.tsv").unwrap().unwrap();
        assert_eq!((r[0].raw_name.as_str(), &r[0].fields), ("a", &vec![("mapped_depth", Value::Num(15.0))]));
        // coverm contig's table is not taken
        let contig = t.path().join("s.coverm.tsv");
        std::fs::write(&contig, "Contig\ts Mean\nk1\t5\n").unwrap();
        assert!(coverm_genome(&contig, "s.coverm.tsv").unwrap().is_none());
        let (_, seen, skipped) = scan(&[t.path().to_path_buf()]).unwrap();
        assert_eq!(seen["CoverM"], 2);
        assert!(skipped.iter().any(|s| s.contains("s.coverm.tsv")), "{skipped:?}");
    }

    fn testdata(rel: &str) -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata").join(rel)
    }

    fn field(r: &Record, k: &str) -> Option<String> {
        r.fields.iter().find(|(f, _)| *f == k).map(|(_, v)| v.show())
    }

    #[test]
    fn hamronization_summary_rows_group_by_file_and_tool_run() {
        let r = hamronization(&testdata("hamronization/hamronization_combined_report.tsv")).unwrap();
        let got: Vec<(String, String, String)> =
            r.iter().map(|r| (r.raw_name.clone(), field(r, "amr_software").unwrap(), field(r, "amr_genes").unwrap())).collect();
        let g = "GCF_010120755.1_ASM1012075v1_genomic".to_string();
        assert_eq!(
            got,
            [
                (g.clone(), "abricate".into(), "aacA-ENT1,eat(A),msr(C)".into()),
                // AMRFinderPlus point mutations: hAMRonization splits gyrA_S83I into gyrA and p.S83I
                (g.clone(), "amrfinderplus".into(), "cirA p.Y253CfsTer5,gyrA p.S83I,mgrB p.Q30Ter".into()),
                // NDM-1 on two contigs counts once; nalC without and with a mutation stay apart
                (g.clone(), "rgi".into(), "NDM-1,nalC,nalC p.G71E".into()),
                ("SAMPLE_B".into(), "amrfinderplus".into(), "blaTEM-156,pmrB p.C84R,sul2".into()),
            ]
        );
        assert!(r.iter().all(|r| r.kind == Kind::Genome));
        assert_eq!(field(&r[0], "amr_run").as_deref(), Some("abricate 1.0.1, ncbi 2024-01"));
        assert_eq!(field(&r[1], "amr_run").as_deref(), Some("amrfinderplus 4.2.7, NCBI Reference Gene Database 2025-07-16.1"));
    }

    #[test]
    fn hamronization_is_recognised_by_its_header() {
        let t = tempfile::tempdir().unwrap();
        let header = std::fs::read_to_string(testdata("hamronization/hamronized_abricate.tsv")).unwrap();
        let header = header.lines().next().unwrap();
        // named like AMRFinderPlus output, as nf-core/funcscan's per-tool tables can be: read as
        // hAMRonization, not as an AMRFinderPlus file with no genes
        let row = "reads/S1_R1.fastq\tblaKPC-2\tKPC-2\tResFinder\t2.1\tAY034847\tsrst2\t0.2.0\tgene_presence_detected";
        std::fs::write(t.path().join("S1.amrfinderplus.tsv"), format!("{header}\n{row}\n")).unwrap();
        std::fs::write(t.path().join("hamronization_combined_report.json"), "[]").unwrap();
        std::fs::write(t.path().join("hamronized_old.tsv"), "input_file_name\tgene_symbol\tgene_name\nS2\tblaOXA-48\tOXA-48\n").unwrap();
        std::fs::write(t.path().join("hamronized_other.tsv"), "sample\tgene\nS3\tsul1\n").unwrap();
        let (records, seen, skipped) = scan(&[t.path().to_path_buf()]).unwrap();
        assert_eq!((seen.get("hAMRonization"), seen.get("AMRFinderPlus")), (Some(&1), None));
        assert_eq!(records.len(), 1);
        // a read-based tool names the read file, so mate tags can be stripped like read QC's
        assert_eq!((records[0].raw_name.as_str(), records[0].kind), ("S1_R1.fastq", Kind::ReadFile));
        assert_eq!(field(&records[0], "amr_genes").as_deref(), Some("blaKPC-2"));
        let why = skipped.join("\n");
        assert!(why.contains("hamronization_combined_report.json (hAMRonization JSON or HTML report, not read"), "{why}");
        assert!(why.contains("hamronized_old.tsv (hAMRonization-like table without analysis_software_name, genetic_variation_type"), "{why}");
        assert!(why.contains("hamronized_other.tsv (named like hAMRonization output but its header lacks"), "{why}");
    }

    #[test]
    fn nf_core_mag_bin_summary_suffixed_columns() {
        // combine_tables.py of nf-core/mag 5.5.0 with BUSCO, CheckM, CheckM2, QUAST and GTDB-Tk
        let r = mag_bin_summary(&testdata("nfcore-mag/5.5.0/bin_summary.tsv")).unwrap().unwrap();
        assert_eq!(r.len(), 5);
        assert_eq!((r[0].raw_name.as_str(), r[0].kind), ("MEGAHIT-MetaBAT2-sample1.1.fa", Kind::Genome));
        let get = |i: usize, k: &str| field(&r[i], k).unwrap_or_default();
        assert_eq!((get(0, "completeness"), get(0, "contamination")), ("97.5".into(), "1.2".into()));
        assert_eq!((get(0, "checkm_completeness"), get(0, "checkm_contamination")), ("95.1".into(), "1.5".into()));
        assert_eq!((get(0, "busco_complete"), get(0, "busco_lineage")), ("96.8".into(), "bacteria_odb12".into()));
        assert_eq!(get(0, "mag_depths"), "sample1=21.4,sample2=3.2");
        assert!(get(2, "gtdb").contains("p__Patescibacteria"));
        // QUAST's columns are not read
        assert!(field(&r[0], "n50").is_none() && field(&r[0], "contigs").is_none());
    }

    #[test]
    fn nf_core_mag_bin_summary_one_qc_tool_unsuffixed() {
        // combine_tables.py of nf-core/mag 5.0.0, --binqc_tool checkm2: index column, no suffixes
        let r = mag_bin_summary(&testdata("nfcore-mag/5.0.0/bin_summary.tsv")).unwrap().unwrap();
        assert_eq!(r.len(), 5);
        assert_eq!(r[4].raw_name, "MEGAHIT-MetaBAT2-sample1.5.fa");
        assert_eq!((field(&r[4], "completeness"), field(&r[4], "contamination")), (Some("92".into()), Some("12.5".into())));
        assert!(field(&r[4], "checkm_completeness").is_none() && field(&r[4], "busco_complete").is_none());
        assert!(field(&r[4], "gtdb").unwrap().contains("p__Actinomycetota"));

        let t = tempfile::tempdir().unwrap();
        let p = t.path().join("bin_summary.tsv");
        // --binqc_tool checkm: CheckM's columns, which go to their own fields
        std::fs::write(&p, "\tbin\tDepth s1\tBin Id\tMarker lineage\tCompleteness\tContamination\n0\tb.1.fa\t3.5\tb.1\tk__Bacteria (UID203)\t88.2\t1.1\n").unwrap();
        let r = mag_bin_summary(&p).unwrap().unwrap();
        assert_eq!(r[0].fields, [("checkm_completeness", Value::Num(88.2)), ("checkm_contamination", Value::Num(1.1)), ("mag_depths", Value::Text("s1=3.5".into()))]);
        // 3.x with BUSCO: nf-core/mag's own summary columns; the specific lineage when there is one
        std::fs::write(&p, "\tbin\tDepth s1\tGenomeBin\tDomain\t%Complete (domain)\tSpecific lineage dataset\t%Complete (specific)\n0\tb.1.fa\t3.5\tb.1.fa\tbacteria_odb10\t80.6\tburkholderiales_odb10\t91.2\n1\tb.2.fa\t1.0\tb.2.fa\tbacteria_odb10\t50.0\t\t\n").unwrap();
        let r = mag_bin_summary(&p).unwrap().unwrap();
        assert_eq!((field(&r[0], "busco_complete"), field(&r[0], "busco_lineage")), (Some("91.2".into()), Some("burkholderiales_odb10".into())));
        assert_eq!((field(&r[1], "busco_complete"), field(&r[1], "busco_lineage")), (Some("50".into()), Some("bacteria_odb10".into())));
        // Completeness whose tool can't be told, and depths alone, are not read
        std::fs::write(&p, "\tbin\tDepth s1\tCompleteness\tContamination\n0\tb.1.fa\t3.5\t90\t1\n").unwrap();
        assert!(mag_bin_summary(&p).unwrap().err().unwrap().contains("which tool"));
        std::fs::write(&p, "\tbin\tDepth s1\n0\tb.1.fa\t3.5\n").unwrap();
        assert!(mag_bin_summary(&p).unwrap().is_err());
        let (records, _, skipped) = scan(&[t.path().to_path_buf()]).unwrap();
        assert!(records.is_empty());
        assert!(skipped[0].contains("nf-core/mag bin summary, not read: no CheckM2, CheckM, BUSCO or GTDB-Tk columns"), "{skipped:?}");
        std::fs::write(&p, "Bin\tCompleteness\n").unwrap();
        let (_, _, skipped) = scan(&[t.path().to_path_buf()]).unwrap();
        assert!(skipped[0].contains("named like nf-core/mag's bin summary but its header has no `bin`"), "{skipped:?}");
    }

    #[test]
    fn kleborate_tables_full_or_trimmed_headers() {
        let t = tempfile::tempdir().unwrap();
        let full = "strain\tenterobacterales__species__species\tenterobacterales__species__species_match\tklebsiella_pneumo_complex__mlst__ST\tklebsiella__ybst__Yersiniabactin\tklebsiella_pneumo_complex__virulence_score__virulence_score\tklebsiella_pneumo_complex__kaptive__K_locus\nBfR-EC-20867\tKlebsiella pneumoniae\tstrong\tST716\tybt 9; ICEKp3\t1\tKL25\n";
        std::fs::write(t.path().join("klebsiella_pneumo_complex_output.txt"), full).unwrap();
        let (r, seen, _) = scan(&[t.path().to_path_buf()]).unwrap();
        assert_eq!(seen["Kleborate"], 1);
        assert_eq!(r[0].raw_name, "BfR-EC-20867");
        let get = |k: &str| r[0].fields.iter().find(|(f, _)| *f == k).map(|(_, v)| v.show());
        assert_eq!((get("kleborate_st"), get("ybt"), get("k_locus")), (Some("ST716".into()), Some("ybt 9; ICEKp3".into()), Some("KL25".into())));
        assert_eq!(get("kleborate_species_match").as_deref(), Some("strong"));
        let trimmed = t.path().join("escherichia_output.txt");
        std::fs::write(&trimmed, "strain\tspecies\tST\ne1\tEscherichia coli\tST131\n").unwrap();
        assert_eq!(kleborate(&trimmed).unwrap()[0].fields[1], ("kleborate_st", Value::Text("ST131".into())));
        std::fs::write(&trimmed, "name\tspecies\n").unwrap();
        assert!(kleborate(&trimmed).is_err());
    }
}
