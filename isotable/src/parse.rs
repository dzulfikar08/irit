//! Finding and reading each tool's output, and working out which sample a record belongs to.

use anyhow::{Context, Result};
use regex::Regex;
use serde_json::Value as Json;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Clone)]
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

/// What one file says about one sample.
pub struct Record {
    pub raw_name: String,
    pub fields: Vec<(&'static str, Value)>,
}

/// Records found, and how many files of each kind were read.
pub fn scan(dirs: &[std::path::PathBuf]) -> Result<(Vec<Record>, BTreeMap<&'static str, usize>)> {
    let mut out = Vec::new();
    let mut seen = BTreeMap::new();
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
                Some(("Kraken 2", kraken(path, &name)))
            } else if name.contains("mlst") && name.ends_with(".tsv") {
                Some(("mlst", mlst(path)))
            } else if name.to_ascii_lowercase().contains("amrfinder") && name.ends_with(".tsv") {
                Some(("AMRFinderPlus", amrfinder(path, &name)))
            } else {
                None
            };
            if let Some((kind, result)) = parsed {
                let records = result.with_context(|| format!("reading {kind} output {}", path.display()))?;
                *seen.entry(kind).or_default() += 1;
                out.extend(records);
            }
        }
    }
    Ok((out, seen))
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
    if let Some(d) = j["duplication"]["rate"].as_f64() {
        fields.push(("duplication", Value::Num(d * 100.0)));
    }
    Ok(vec![Record { raw_name: name.trim_end_matches(".fastp.json").to_string(), fields }])
}

fn leanqc(path: &Path) -> Result<Vec<Record>> {
    let j = json(path)?;
    let file = j["file"].as_str().unwrap_or_default().to_string();
    let fails: Vec<&str> = j["checks"]
        .as_array()
        .map(|a| a.iter().filter(|c| c["status"] == "fail").filter_map(|c| c["name"].as_str()).collect())
        .unwrap_or_default();
    Ok(vec![Record { raw_name: file, fields: vec![("leanqc_fails", Value::Text(fails.join(",")))] }])
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
            Some(Record { raw_name: name, fields })
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
    Ok(vec![Record { raw_name: sample.to_string(), fields }])
}

fn checkm2(path: &Path) -> Result<Vec<Record>> {
    Ok(tsv(path)?
        .into_iter()
        .filter_map(|r| {
            Some(Record {
                raw_name: r.get("Name")?.clone(),
                fields: vec![
                    ("completeness", Value::Num(get_num(&r, "Completeness")?)),
                    ("contamination", Value::Num(get_num(&r, "Contamination")?)),
                ],
            })
        })
        .collect())
}

fn gtdbtk(path: &Path) -> Result<Vec<Record>> {
    Ok(tsv(path)?
        .into_iter()
        .filter_map(|r| {
            Some(Record {
                raw_name: r.get("user_genome")?.clone(),
                fields: vec![("gtdb", Value::Text(r.get("classification")?.clone()))],
            })
        })
        .collect())
}

/// Kraken 2 report: percent, clade reads, direct reads, rank, taxid, name. Takes the top species.
fn kraken(path: &Path, name: &str) -> Result<Vec<Record>> {
    let text = std::fs::read_to_string(path)?;
    let mut best: Option<(f64, String)> = None;
    for line in text.lines() {
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 6 || cols[3].trim() != "S" {
            continue;
        }
        let pct: f64 = cols[0].trim().parse().unwrap_or(0.0);
        if best.as_ref().is_none_or(|(p, _)| pct > *p) {
            best = Some((pct, cols[cols.len() - 1].trim().to_string()));
        }
    }
    let raw = name.split(".kraken2").next().unwrap_or(name).split(".kreport").next().unwrap_or(name).to_string();
    let fields = match best {
        Some((p, sp)) => vec![("kraken_top", Value::Text(sp)), ("kraken_top_pct", Value::Num(p))],
        None => vec![("kraken_top", Value::Text("no species-level hit".into()))],
    };
    Ok(vec![Record { raw_name: raw, fields }])
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
            Some(Record {
                raw_name: file,
                fields: vec![("mlst_scheme", Value::Text(c[1].into())), ("mlst_st", Value::Text(c[2].into()))],
            })
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
    Ok(vec![Record { raw_name: raw, fields: vec![("amr_genes", Value::Text(value))] }])
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

static SUFFIXES: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(\.(fastq|fq|fasta|fa|fna|gz|bz2|zst|scaffolds|contigs|assembly))$|([._-](unicycler|spades|skesa|shovill|dragonflye|flye|canu|megahit|metaspades))$|(_R?[12](_\d{3})?)$",
    )
    .unwrap()
});

/// The sample a raw name (file or assembly name) belongs to.
///
/// With a sheet: the longest sheet name that the raw name equals or starts with, followed by a
/// separator (`ERR044595-unicycler.scaffolds` → `ERR044595`). Without one: the raw name with
/// file extensions, assembler tags and read-mate suffixes stripped.
pub fn match_sample(raw: &str, sheet: &[String]) -> String {
    let fit = sheet
        .iter()
        .filter(|s| raw == s.as_str() || (raw.starts_with(s.as_str()) && !raw[s.len()..].starts_with(|c: char| c.is_ascii_alphanumeric())))
        .max_by_key(|s| s.len());
    if let Some(s) = fit {
        return s.clone();
    }
    let mut name = raw.to_string();
    loop {
        let next = SUFFIXES.replace(&name, "").to_string();
        if next == name || next.is_empty() {
            return name;
        }
        name = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_without_sheet() {
        assert_eq!(match_sample("ERR044595-unicycler.scaffolds", &[]), "ERR044595");
        assert_eq!(match_sample("ERR044595-unicycler.scaffolds.fa", &[]), "ERR044595");
        assert_eq!(match_sample("iso_7_R1.fastq.gz", &[]), "iso_7");
        assert_eq!(match_sample("bin.12.fa", &[]), "bin.12");
    }

    #[test]
    fn names_with_sheet() {
        let sheet = vec!["ERR044595".to_string(), "ERR0445".to_string()];
        assert_eq!(match_sample("ERR044595_1M_1.fastq.gz", &sheet), "ERR044595");
        assert_eq!(match_sample("ERR044595-unicycler", &sheet), "ERR044595");
        assert_eq!(match_sample("ERR04459X", &sheet), "ERR04459X");
    }
}
