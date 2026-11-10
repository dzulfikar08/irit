//! Turning irit outputs into structured JSON for the agent.

use crate::record::FileRecord;
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

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

#[cfg(test)]
mod tests {
    use super::*;

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
