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
            let parsed: Option<(&'static str, Result<Vec<Record>>)> = if name.ends_with(".fastp.json") {
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
                Some(("Shovill", shovill(path)))
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
    Ok((out, seen, skipped))
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

fn leanqc(path: &Path) -> Result<Vec<Record>> {
    let j = json(path)?;
    let file = j["file"].as_str().unwrap_or_default().to_string();
    let fails: Vec<&str> = j["checks"]
        .as_array()
        .map(|a| a.iter().filter(|c| c["status"] == "fail").filter_map(|c| c["name"].as_str()).collect())
        .unwrap_or_default();
    Ok(vec![rec(file, Kind::ReadFile, vec![("leanqc_fails", Value::Text(fails.join(",")))])])
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
            Some(rec(name, Kind::Genome, fields))
        })
        .collect())
}

/// QUAST's default `--min-contig`: its `# contigs`, `Total length`, `N50` and `GC (%)` count only
/// contigs at least this long.
const QUAST_MIN_CONTIG: u64 = 500;

/// Shovill's `<outdir>/contigs.fa` (recognised by the `shovill.log` beside it), named after its
/// folder as Shovill always names the file contigs.fa. Contigs, length, N50 and GC are counted the
/// way QUAST reports them (contigs of 500 bp or more; GC over A, C, G and T), so a QUAST report on
/// the same assembly agrees.
fn shovill(path: &Path) -> Result<Vec<Record>> {
    let name = path
        .parent()
        .and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .context("contigs.fa has no folder to name the sample")?;
    // (length, G+C, A+C+G+T) per contig
    let mut contigs: Vec<(u64, u64, u64)> = Vec::new();
    let mut cur: Option<(u64, u64, u64)> = None;
    for line in std::io::BufReader::new(std::fs::File::open(path)?).lines() {
        let line = line?;
        if line.starts_with('>') {
            contigs.extend(cur.replace((0, 0, 0)));
        } else if let Some((len, gc, acgt)) = cur.as_mut() {
            let seq = line.trim_end().as_bytes();
            *len += seq.len() as u64;
            for b in seq {
                match b.to_ascii_uppercase() {
                    b'G' | b'C' => {
                        *gc += 1;
                        *acgt += 1;
                    }
                    b'A' | b'T' => *acgt += 1,
                    _ => {}
                }
            }
        } else if !line.trim().is_empty() {
            bail!("not FASTA: sequence before the first > line");
        }
    }
    contigs.extend(cur);
    contigs.retain(|c| c.0 >= QUAST_MIN_CONTIG);
    contigs.sort_unstable_by_key(|a| std::cmp::Reverse(a.0));
    let total: u64 = contigs.iter().map(|c| c.0).sum();
    let mut acc = 0;
    let n50 = contigs.iter().find(|c| {
        acc += c.0;
        acc * 2 >= total
    });
    let mut fields = vec![("contigs", Value::Int(contigs.len() as u64)), ("length", Value::Int(total))];
    fields.extend(n50.map(|c| ("n50", Value::Int(c.0))));
    let (gc, acgt) = contigs.iter().fold((0, 0), |(g, a), c| (g + c.1, a + c.2));
    if acgt > 0 {
        // two decimals, as QUAST prints it
        fields.push(("gc", Value::Num((gc as f64 * 10000.0 / acgt as f64).round() / 100.0)));
    }
    Ok(vec![rec(name, Kind::Genome, fields)])
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

/// AMRFinderPlus: one file per sample; lists AMR genes (column names changed in 4.x).
pub fn amrfinder(path: &Path, name: &str) -> Result<Vec<Record>> {
    let rows = tsv(path)?;
    let mut genes: Vec<String> = rows
        .iter()
        .filter(|r| {
            let t = r.get("Element type").or_else(|| r.get("Type")).map(String::as_str).unwrap_or("AMR");
            t == "AMR"
        })
        .filter_map(|r| r.get("Element symbol").or_else(|| r.get("Gene symbol")).cloned())
        .collect();
    genes.sort();
    genes.dedup();
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

#[cfg(test)]
mod tests {
    use super::*;

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
        std::fs::write(dir.join("shovill.log"), "").unwrap();
        let (records, seen, _) = scan(&[t.path().to_path_buf()]).unwrap();
        assert_eq!(seen["Shovill"], 1);
        assert_eq!(records[0].raw_name, "s1");
        // GC over the two counted contigs: 400 C + 600 G of 1600
        assert_eq!(records[0].fields, [("contigs", Value::Int(2)), ("length", Value::Int(1600)), ("n50", Value::Int(1000)), ("gc", Value::Num(62.5))]);
        std::fs::write(dir.join("contigs.fa"), "ACGT\n").unwrap();
        assert!(scan(&[t.path().to_path_buf()]).is_err());
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
}
