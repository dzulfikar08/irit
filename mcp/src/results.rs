//! Turning irit outputs into structured JSON for the agent.

use crate::record::FileRecord;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;
use std::path::Path;

/// Parses a CSV (RFC 4180 quoting) or TSV table; the delimiter is taken from the header line.
pub fn parse_table(text: &str) -> (Vec<String>, Vec<Vec<String>>) {
    let first = text.lines().next().unwrap_or("");
    let delim = if first.contains('\t') { '\t' } else { ',' };
    let mut rows = Vec::new();
    let (mut row, mut cell, mut quoted, mut chars) = (Vec::new(), String::new(), false, text.chars().peekable());
    while let Some(c) = chars.next() {
        match c {
            '"' if delim == ',' && quoted && chars.peek() == Some(&'"') => {
                chars.next();
                cell.push('"');
            }
            '"' if delim == ',' && (quoted || cell.is_empty()) => quoted = !quoted,
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell));
                rows.push(std::mem::take(&mut row));
            }
            c if c == delim && !quoted => row.push(std::mem::take(&mut cell)),
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    let mut it = rows.into_iter();
    let header = it.next().unwrap_or_default();
    (header, it.collect())
}

pub fn table_objects(header: &[String], rows: &[Vec<String>]) -> Vec<Value> {
    rows.iter()
        .map(|r| {
            let m: Map<String, Value> = header.iter().cloned().zip(r.iter().map(|c| json!(c))).collect();
            Value::Object(m)
        })
        .collect()
}

/// Per-file verdicts from the `*_leanqc.json` files a leanqc call wrote.
pub fn leanqc_reports(outputs: &[FileRecord]) -> Vec<Value> {
    let mut reports = Vec::new();
    for o in outputs.iter().filter(|o| o.path.ends_with("_leanqc.json")) {
        let Ok(text) = std::fs::read_to_string(&o.path) else { continue };
        let Ok(j) = serde_json::from_str::<Value>(&text) else { continue };
        let html = o.path.trim_end_matches(".json").to_string() + ".html";
        let checks = j["checks"].as_array().cloned().unwrap_or_default();
        let pick = |status: &str| -> Vec<Value> {
            checks
                .iter()
                .filter(|c| c["status"] == status)
                .map(|c| json!({"check": c["name"], "detail": c["detail"]}))
                .collect()
        };
        let (fails, warns) = (pick("fail"), pick("warn"));
        let verdict = if !fails.is_empty() { "fail" } else if !warns.is_empty() { "warn" } else { "pass" };
        reports.push(json!({
            "file": j["file"],
            "verdict": verdict,
            "reads": j["reads"],
            "mean_len": j["mean_len"],
            "gc_percent": j["gc_percent"],
            "q30_percent": j["q30_percent"],
            "fails": fails,
            "warns": warns,
            "warnings": j["warnings"],
            "json_report": o.path,
            "html_report": outputs.iter().any(|x| x.path == html).then_some(html),
        }));
    }
    reports
}

/// isotable rows with `reasons` split into a list and the remaining columns as `values`.
pub fn isotable_rows(text: &str) -> Value {
    let (header, rows) = parse_table(text);
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let samples: Vec<Value> = table_objects(&header, &rows)
        .into_iter()
        .map(|mut row| {
            let obj = row.as_object_mut().unwrap();
            let sample = obj.remove("sample").unwrap_or(Value::Null);
            let status = obj.remove("status").unwrap_or(Value::Null);
            let reasons: Vec<String> = match obj.remove("reasons") {
                Some(Value::String(r)) => r.split("; ").filter(|s| !s.is_empty()).map(str::to_string).collect(),
                _ => Vec::new(),
            };
            *counts.entry(status.as_str().unwrap_or("?").to_string()).or_default() += 1;
            json!({"sample": sample, "status": status, "reasons": reasons, "values": obj})
        })
        .collect();
    json!({"counts": counts, "samples": samples, "columns": header})
}

/// The catalog is a fixed-width text table: id, download size, size on disk, description,
/// and an optional trailing `[licence note]`.
pub fn catalog_rows(text: &str) -> Vec<Value> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let t: Vec<&str> = line.split_whitespace().collect();
            if t.len() < 5 {
                return None;
            }
            let mut desc = t[5..].join(" ");
            let mut note = None;
            if let (Some(i), true) = (desc.rfind(" ["), desc.ends_with(']')) {
                note = Some(desc[i + 2..desc.len() - 1].to_string());
                desc.truncate(i);
            }
            Some(json!({
                "id": t[0],
                "download": format!("{} {}", t[1], t[2]),
                "on_disk": format!("{} {}", t[3], t[4]),
                "description": desc,
                "licence_note": note,
            }))
        })
        .collect()
}

/// `export NAME='value'` lines from `dblock env`, unquoted.
pub fn env_exports(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter_map(|l| l.strip_prefix("export "))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| {
            let v = v.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')).map(|v| v.replace(r"'\''", "'"));
            (k.to_string(), v.unwrap_or_default())
        })
        .collect()
}

/// isotable's records as JSON objects: the name the file gives, then each value isotable reads
/// (numbers as numbers, rounded to 2 decimals). isotable's join helpers (`raw_*`) are left out.
fn isotable_records(records: Vec<isotable::parse::Record>, name_key: &str) -> Vec<Value> {
    use isotable::parse::Value as V;
    records
        .into_iter()
        .map(|r| {
            let mut m = Map::new();
            m.insert(name_key.into(), json!(r.raw_name));
            for (k, v) in r.fields.into_iter().filter(|(k, _)| !k.starts_with("raw_")) {
                let v = match v {
                    V::Int(n) => json!(n),
                    V::Num(x) => json!((x * 100.0).round() / 100.0),
                    V::Text(t) => json!(t),
                };
                m.insert(k.into(), v);
            }
            Value::Object(m)
        })
        .collect()
}

/// fastp's JSON report, read by isotable's parser: reads before/after, Q30 and duplication (%).
pub fn fastp_summary(path: &Path) -> Result<Value, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let recs = isotable::parse::fastp(path, &name).map_err(|e| format!("{}: {e:#}", path.display()))?;
    Ok(isotable_records(recs, "sample").into_iter().next().unwrap_or(Value::Null))
}

/// CheckM2's quality_report.tsv, one entry per genome (isotable's parser).
pub fn checkm2_summary(path: &Path) -> Result<Vec<Value>, String> {
    isotable::parse::checkm2(path).map(|r| isotable_records(r, "genome")).map_err(|e| format!("{}: {e:#}", path.display()))
}

/// GTDB-Tk summary.tsv rows: isotable's `gtdb` classification, plus the columns that say how the
/// call was made and whether GTDB-Tk had doubts.
pub fn gtdbtk_summary(path: &Path) -> Result<Vec<Value>, String> {
    let recs = isotable::parse::gtdbtk(path).map_err(|e| format!("{}: {e:#}", path.display()))?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (header, rows) = parse_table(&text);
    let extra = ["classification_method", "closest_genome_reference", "closest_genome_ani", "msa_percent", "warnings", "note"];
    let mut out = isotable_records(recs, "genome");
    for o in &mut out {
        let Some(row) = rows.iter().find(|r| r.first().map(String::as_str) == o["genome"].as_str()) else { continue };
        for col in extra {
            let Some(i) = header.iter().position(|h| h == col) else { continue };
            match row.get(i).map(String::as_str) {
                None | Some("" | "N/A") => {}
                Some(cell) => o[col] = json!(cell),
            }
        }
    }
    Ok(out)
}

/// Contig count, total length, N50, longest contig and GC % of an assembly FASTA (Shovill's
/// contigs.fa), computed here: Shovill prints its own stats only to its log.
pub fn assembly_stats(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut lens: Vec<u64> = Vec::new();
    let (mut gc, mut acgt) = (0u64, 0u64);
    for line in text.lines() {
        if line.starts_with('>') {
            lens.push(0);
            continue;
        }
        let Some(last) = lens.last_mut() else { return Err(format!("{}: not FASTA", path.display())) };
        let line = line.trim_end();
        *last += line.len() as u64;
        for b in line.bytes() {
            match b.to_ascii_uppercase() {
                b'G' | b'C' => {
                    gc += 1;
                    acgt += 1;
                }
                b'A' | b'T' => acgt += 1,
                _ => {}
            }
        }
    }
    let total: u64 = lens.iter().sum();
    let mut sorted = lens.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    let mut acc = 0;
    let n50 = sorted.iter().find(|&&l| {
        acc += l;
        acc * 2 >= total
    });
    Ok(json!({
        "contigs": lens.len(),
        "total_length": total,
        "n50": n50.copied().unwrap_or(0),
        "longest": sorted.first().copied().unwrap_or(0),
        "gc_percent": if acgt > 0 { (gc as f64 * 10000.0 / acgt as f64).round() / 100.0 } else { 0.0 },
    }))
}

/// mlst's table: one object per row. With a header (`--full`), keyed by its lower-cased column
/// names; without, mlst 2.35.0's layout FILE, SCHEME, ST, then one `gene(allele)` per column.
/// `sample` is what isotable will join the row to.
pub fn mlst_summary(path: &Path) -> Result<Vec<Value>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    // isotable reads the same file; a table it can't read is an error here too.
    isotable::parse::mlst(path).map_err(|e| format!("{}: {e:#}", path.display()))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty()).peekable();
    let header: Option<Vec<String>> =
        lines.next_if(|l| l.starts_with("FILE\t")).map(|h| h.split('\t').map(|c| c.to_ascii_lowercase()).collect());
    let rows: Vec<Value> = lines
        .map(|l| l.split('\t').collect::<Vec<&str>>())
        .filter(|c| c.len() >= 3)
        .map(|c| {
            let mut o = match &header {
                Some(h) => Value::Object(h.iter().cloned().zip(c.iter().map(|x| json!(x))).collect()),
                None => json!({"file": c[0], "scheme": c[1], "st": c[2], "alleles": c[3..].join(" ")}),
            };
            // isotable's join name: the FILE column's file name without extensions.
            let file = Path::new(c[0]).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            o["sample"] = json!(isotable::parse::base_name(&file));
            o
        })
        .collect();
    Ok(rows)
}

/// AMRFinderPlus's report: its rows (the columns that say what was found and how well), counts per
/// element type, and the AMR gene list isotable reads (isotable's parser; 4.x column names).
pub fn amrfinder_summary(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let recs = isotable::parse::amrfinder(path, &name).map_err(|e| format!("{}: {e:#}", path.display()))?;
    let isotable = isotable_records(recs, "sample").into_iter().next().unwrap_or(Value::Null);
    let (header, rows) = parse_table(&text);
    if !header.iter().any(|h| h == "Element symbol" || h == "Gene symbol") {
        return Err(format!("{}: no `Element symbol` column; not an AMRFinderPlus 4.x report", path.display()));
    }
    let keep = [
        "Name", "Contig id", "Protein id", "Element symbol", "Element name", "Type", "Subtype", "Class", "Subclass", "Method",
        "% Coverage of reference", "% Identity to reference",
    ];
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let hits: Vec<Value> = rows
        .iter()
        .filter(|r| r.len() == header.len())
        .map(|r| {
            let mut o = Map::new();
            for (h, cell) in header.iter().zip(r) {
                if keep.contains(&h.as_str()) && cell != "NA" {
                    o.insert(h.to_lowercase().replace("% ", "").replace(' ', "_"), json!(cell));
                }
            }
            *counts.entry(o.get("type").and_then(Value::as_str).unwrap_or("?").to_string()).or_default() += 1;
            Value::Object(o)
        })
        .collect();
    Ok(json!({"sample": isotable["sample"], "amr_genes": isotable["amr_genes"], "counts_by_type": counts, "hits": hits}))
}

/// Bakta's `<prefix>.txt`: `Key: value` lines under `Sequence(s):`, `Annotation:` and `Bakta:`
/// (written in bakta/main.py at v1.12.2). Counts become numbers.
pub fn bakta_summary(text: &str) -> Value {
    let mut sections = Map::new();
    let mut current = String::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(head) = line.strip_suffix(':') {
            current = match head {
                "Sequence(s)" => "sequences".into(),
                other => other.to_lowercase(),
            };
            continue;
        }
        let Some((k, v)) = line.split_once(": ") else { continue };
        let value = v.parse::<u64>().map(|n| json!(n)).or_else(|_| v.parse::<f64>().map(|x| json!(x))).unwrap_or_else(|_| json!(v));
        let section = sections.entry(current.clone()).or_insert_with(|| json!({}));
        section[k.to_lowercase().replace(' ', "_")] = value;
    }
    Value::Object(sections)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn external_outputs() {
        let d = tempfile::tempdir().unwrap();
        let fp = d.path().join("s1.fastp.json");
        std::fs::write(
            &fp,
            r#"{"summary": {"before_filtering": {"total_reads": 1000, "total_bases": 150000},
               "after_filtering": {"total_reads": 950, "total_bases": 140000, "q30_rate": 0.91234, "q30_bases": 127000}},
               "duplication": {"rate": 0.012}, "command": "fastp --in1 s1_R1.fq"}"#,
        )
        .unwrap();
        let s = fastp_summary(&fp).unwrap();
        assert_eq!(s["sample"], "s1");
        assert_eq!(s["reads_raw"], 1000);
        assert_eq!(s["reads_trimmed"], 950);
        assert_eq!(s["q30_trimmed"], 91.23);
        assert!(s.get("raw_summary").is_none() && s.get("raw_inputs").is_none());

        let cm = d.path().join("quality_report.tsv");
        std::fs::write(&cm, "Name\tCompleteness\tContamination\tCompleteness_Model_Used\nbin1\t98.5\t0.7\tGradient Boost\n").unwrap();
        assert_eq!(checkm2_summary(&cm).unwrap(), vec![json!({"genome": "bin1", "completeness": 98.5, "contamination": 0.7})]);

        let gt = d.path().join("gtdbtk.bac120.summary.tsv");
        std::fs::write(
            &gt,
            "user_genome\tclassification\tclosest_genome_reference\tclosest_genome_ani\tclassification_method\tnote\twarnings\n\
             s1\td__Bacteria;s__Escherichia coli\tGCF_003697165.2\t98.9\tANI\tN/A\tN/A\n",
        )
        .unwrap();
        let g = gtdbtk_summary(&gt).unwrap();
        assert_eq!(g[0]["genome"], "s1");
        assert_eq!(g[0]["gtdb"], "d__Bacteria;s__Escherichia coli");
        assert_eq!(g[0]["classification_method"], "ANI");
        assert_eq!(g[0]["closest_genome_ani"], "98.9");
        assert!(g[0].get("warnings").is_none());

        let b = bakta_summary(
            "Sequence(s):\nLength: 4641652\nCount: 1\nGC: 50.8\nN50: 4641652\n\nAnnotation:\ntRNAs: 86\nCDSs: 4310\nncRNA regions: 60\n\nBakta:\nSoftware: v1.12.2\nDatabase: v6.0, light\n",
        );
        assert_eq!(b["sequences"]["length"], 4641652);
        assert_eq!(b["sequences"]["gc"], 50.8);
        assert_eq!(b["annotation"]["cdss"], 4310);
        assert_eq!(b["annotation"]["ncrna_regions"], 60);
        assert_eq!(b["bakta"]["database"], "v6.0, light");
    }

    #[test]
    fn tables() {
        let (h, r) = parse_table("sample,fastq_1\n\"a,b\",\"x \"\"q\"\".fq\"\nc,d\n");
        assert_eq!(h, ["sample", "fastq_1"]);
        assert_eq!(r, [vec!["a,b", "x \"q\".fq"], vec!["c", "d"]]);
        let v = isotable_rows("sample\tstatus\treasons\tn50\ns1\twarn\twarn: a; warn: b\t100\ns2\tpass\t\t9\n");
        assert_eq!(v["counts"]["warn"], 1);
        assert_eq!(v["samples"][0]["reasons"], json!(["warn: a", "warn: b"]));
        assert_eq!(v["samples"][1]["reasons"], json!([]));
        assert_eq!(v["samples"][0]["values"]["n50"], "100");
    }

    #[test]
    fn dblock_text() {
        let c = catalog_rows(
            "ID   DOWNLOAD   ON DISK  DESCRIPTION\ncard-4.0.2     4 MB    50 MB  CARD data (broadstreet)  [non-commercial licence]\n",
        );
        assert_eq!(c[0]["id"], "card-4.0.2");
        assert_eq!(c[0]["on_disk"], "50 MB");
        assert_eq!(c[0]["description"], "CARD data (broadstreet)");
        assert_eq!(c[0]["licence_note"], "non-commercial licence");
        let e = env_exports("export CHECKM2DB='/s/it'\\''s/db'\n# note\n");
        assert_eq!(e["CHECKM2DB"], "/s/it's/db");
    }
}
