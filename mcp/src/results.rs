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

/// CoverM's per-sample mapping lines on stderr: `In sample 'X', found N reads mapped out of M
/// total (P%)`.
pub fn coverm_mapped(stderr: &str) -> Vec<Value> {
    stderr
        .lines()
        .filter_map(|l| {
            let rest = &l[l.find("In sample '")? + "In sample '".len()..];
            let (sample, rest) = rest.split_once("', found ")?;
            let (mapped, rest) = rest.split_once(" reads mapped out of ")?;
            let (total, rest) = rest.split_once(" total (")?;
            let pct = rest.split_once('%')?.0;
            Some(json!({
                "sample": sample,
                "reads_mapped": mapped.trim().parse::<u64>().ok()?,
                "reads_total": total.trim().parse::<u64>().ok()?,
                "percent": pct.trim().parse::<f64>().ok()?,
            }))
        })
        .collect()
}

/// Data rows of a table with a header line.
pub fn table_rows(path: &Path) -> Result<usize, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(text.lines().skip(1).filter(|l| !l.trim().is_empty()).count())
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

/// Flye's `assembly_info.txt` (Flye 2.9.6 `flye/assembly/scaffolder.py` header `#seq_name length
/// cov. circ. repeat mult. alt_group graph_path`): per contig its name, length, mean depth and
/// whether Flye closed it into a circle. A complete bacterial genome is one circular chromosome
/// plus circular plasmids.
pub fn flye_info(path: &Path) -> Result<Vec<Value>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut lines = text.lines().filter(|l| !l.trim().is_empty());
    let header: Vec<&str> = lines.next().ok_or_else(|| format!("{}: empty", path.display()))?.trim_start_matches('#').split('\t').collect();
    let col = |name: &str| header.iter().position(|h| h.trim() == name).ok_or_else(|| format!("{}: no {name} column", path.display()));
    let (name, len, cov, circ) = (col("seq_name")?, col("length")?, col("cov.")?, col("circ.")?);
    let mut out = Vec::new();
    for l in lines {
        let f: Vec<&str> = l.split('\t').collect();
        let get = |i: usize| f.get(i).copied().ok_or_else(|| format!("{}: short line {l:?}", path.display()));
        let length: u64 = get(len)?.parse().map_err(|_| format!("{}: bad length in {l:?}", path.display()))?;
        let depth: f64 = get(cov)?.parse().map_err(|_| format!("{}: bad cov. in {l:?}", path.display()))?;
        out.push(json!({"name": get(name)?, "length": length, "depth": depth, "circular": get(circ)? == "Y"}));
    }
    out.sort_by(|a, b| b["length"].as_u64().cmp(&a["length"].as_u64()));
    Ok(out)
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

/// `gtdbprep ani`'s per-genome table (its stdout: genome, markers, candidates, assigned_by_ani[,
/// used]) joined with GTDB-Tk's ANI summaries in `<work>/out/classify/ani_screen`: the reference,
/// ANI, alignment fraction and taxonomy of each genome the screen assigned.
pub fn gtdbprep_ani_summary(stdout: &str, work: &Path) -> Result<Vec<Value>, String> {
    let table: String = stdout.lines().take_while(|l| l.contains('\t')).map(|l| format!("{l}\n")).collect();
    let (header, rows) = parse_table(&table);
    if header.first().map(String::as_str) != Some("genome") {
        return Err("no genome table in gtdbprep's output".into());
    }
    let mut genomes = table_objects(&header, &rows);
    for g in &mut genomes {
        for k in ["markers", "candidates", "used"] {
            if let Some(n) = g[k].as_str().and_then(|s| s.parse::<u64>().ok()) {
                g[k] = json!(n);
            }
        }
        g["assigned_by_ani"] = json!(g["assigned_by_ani"] == "true");
    }
    let dir = work.join("out/classify/ani_screen");
    for marker in ["bac120", "ar53"] {
        let path = dir.join(format!("gtdbtk.{marker}.ani_summary.tsv"));
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let (h, rows) = parse_table(&text);
        let col = |name: &str| h.iter().position(|c| c == name);
        let (Some(user), Some(reference), Some(ani), Some(af), Some(tax)) =
            (col("user_genome"), col("reference_genome"), col("skani_ani"), col("skani_af"), col("reference_taxonomy"))
        else {
            return Err(format!("{}: not a GTDB-Tk ANI summary", path.display()));
        };
        for r in rows {
            let Some(g) = genomes.iter_mut().find(|g| g["genome"].as_str() == r.get(user).map(String::as_str)) else { continue };
            let num = |i: usize| r.get(i).and_then(|s| s.parse::<f64>().ok());
            let taxonomy = r.get(tax).cloned().unwrap_or_default();
            g["species"] = json!(taxonomy.rsplit(';').next().filter(|s| s.starts_with("s__")));
            g["taxonomy"] = json!(taxonomy);
            g["reference"] = json!(r.get(reference));
            g["ani"] = json!(num(ani));
            g["af"] = json!(num(af));
            g["summary_file"] = json!(path);
        }
    }
    Ok(genomes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flye_assembly_info() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("assembly_info.txt");
        std::fs::write(
            &f,
            "#seq_name\tlength\tcov.\tcirc.\trepeat\tmult.\talt_group\tgraph_path\n\
             contig_2\t120544\t131\tY\tN\t2\t*\t2\n\
             contig_1\t5301822\t68\tY\tN\t1\t*\t1\n\
             contig_3\t4012\t240\tN\tY\t4\t*\t-3\n",
        )
        .unwrap();
        let c = flye_info(&f).unwrap();
        assert_eq!(c.len(), 3);
        assert_eq!(c[0], json!({"name": "contig_1", "length": 5301822, "depth": 68.0, "circular": true}));
        assert_eq!(c[2]["circular"], json!(false));
        std::fs::write(&f, "#seq_name\tlength\n").unwrap();
        assert!(flye_info(&f).unwrap_err().contains("cov."));
        assert!(flye_info(&d.path().join("missing.txt")).is_err());
    }

    #[test]
    fn coverm_mapping_lines() {
        let log = "[2026-11-13T05:00:00Z INFO  coverm::contig] Using min-covered-fraction 0%\n\
                   [2026-11-13T05:10:00Z INFO  coverm] In sample 'final.contigs.fa/ERR2809130_1.fastq.gz', found 1234567 reads mapped out of 4445596 total (27.77%)\n\
                   some other line\n";
        assert_eq!(
            coverm_mapped(log),
            [json!({"sample": "final.contigs.fa/ERR2809130_1.fastq.gz", "reads_mapped": 1234567, "reads_total": 4445596, "percent": 27.77})]
        );
        assert!(coverm_mapped("In sample 'x', found many reads").is_empty());
    }

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

    #[test]
    fn gtdbprep_ani_rows() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("out/classify/ani_screen");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("gtdbtk.bac120.ani_summary.tsv"),
            "user_genome\treference_genome\tskani_ani\tskani_af\treference_taxonomy\tother_related_references(genome_id,species_name,radius,ANI,AF)\n\
             b168\tGCF_000009045.1\t100.0\t1.0\td__Bacteria;g__Bacillus;s__Bacillus subtilis\tGCA_1, s__X, 95.0, 94.8, 0.886\n",
        )
        .unwrap();
        let stdout = "genome\tmarkers\tcandidates\tassigned_by_ani\tused\nb168\t4089\t21\ttrue\t14\nfrag\t12\t0\tfalse\t0\n\
                      21 reference(s) screened in, 14 could give the species call (--species-only)\n";
        let g = gtdbprep_ani_summary(stdout, d.path()).unwrap();
        assert_eq!(g[0]["species"], "s__Bacillus subtilis");
        assert_eq!((&g[0]["ani"], &g[0]["af"], &g[0]["used"], &g[0]["assigned_by_ani"]), (&json!(100.0), &json!(1.0), &json!(14), &json!(true)));
        assert_eq!(g[1]["assigned_by_ani"], false);
        assert!(g[1].get("species").is_none());
        assert!(gtdbprep_ani_summary("nothing\n", d.path()).is_err());
    }
}
