//! Finding and reading each tool's output, and working out which sample a record belongs to.

use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::io::BufRead;
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
    /// One read file (leanqc runs per file): `x_R1.fastq.gz`. Mate suffixes may be stripped.
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
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            let parsed: Option<(&'static str, Result<Vec<Record>>)> = if name.ends_with(".fastp.json") {
                Some(("fastp", fastp(path, &name)))
            } else if name.ends_with("_leanqc.json") {
                Some(("leanqc", leanqc(path)))
            } else if name == "transposed_report.tsv" {
                Some(("QUAST", quast(path)))
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
                let mut records = result.with_context(|| format!("reading {tool} output {}", path.display()))?;
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

fn fastp(path: &Path, name: &str) -> Result<Vec<Record>> {
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
    // nf-core names fastp reports after the sample (`<sample>.fastp.json`), one per read pair.
    Ok(vec![rec(name.trim_end_matches(".fastp.json"), Kind::Sample, fields)])
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

fn checkm2(path: &Path) -> Result<Vec<Record>> {
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

fn gtdbtk(path: &Path) -> Result<Vec<Record>> {
    Ok(tsv(path)?
        .into_iter()
        .filter_map(|r| {
            Some(rec(r.get("user_genome")?.clone(), Kind::Genome, vec![("gtdb", Value::Text(r.get("classification")?.clone()))]))
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
fn mlst(path: &Path) -> Result<Vec<Record>> {
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
fn amrfinder(path: &Path, name: &str) -> Result<Vec<Record>> {
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
    fn mates() {
        assert_eq!(mate_parts("x_R1"), Some(("x".into(), "x_R2".into())));
        assert_eq!(mate_parts("x_2"), Some(("x".into(), "x_1".into())));
        assert_eq!(mate_parts("x_R1_001"), Some(("x".into(), "x_R2_001".into())));
        assert_eq!(mate_parts("x"), None);
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
}
