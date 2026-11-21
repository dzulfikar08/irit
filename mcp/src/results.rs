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

/// Contig count and length per sample in a concatenated FASTA, in file order.
pub fn concatenated_samples(path: &Path) -> Vec<Value> {
    use std::io::BufRead;
    let Ok(f) = std::fs::File::open(path) else { return Vec::new() };
    let mut out: Vec<(String, u64, u64)> = Vec::new();
    for line in std::io::BufReader::new(f).split(b'\n').map_while(Result::ok) {
        if let Some(h) = line.strip_prefix(b">") {
            let h = String::from_utf8_lossy(h);
            let sample = h.split_once(':').map(|(s, _)| s).unwrap_or("").to_string();
            if out.last().map(|o| &o.0) != Some(&sample) {
                out.push((sample, 0, 0));
            }
            out.last_mut().unwrap().1 += 1;
        } else if let Some(o) = out.last_mut() {
            o.2 += line.iter().filter(|b| !b.is_ascii_whitespace()).count() as u64;
        }
    }
    out.into_iter().map(|(s, n, l)| json!({"sample": s, "contigs": n, "total_length": l})).collect()
}

/// Data rows of a table with a header line.
pub fn table_rows(path: &Path) -> Result<usize, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(text.lines().skip(1).filter(|l| !l.trim().is_empty()).count())
}

/// Contig count, total length, N50, longest contig and GC % of an assembly FASTA (Shovill's
/// contigs.fa), over every contig whatever its length (replay.sh compares the same numbers):
/// isotable's assembly statistics with no length cut-off.
pub fn assembly_stats(path: &Path) -> Result<Value, String> {
    let s = isotable::asm::stats(path, 0).map_err(|e| format!("{}: {e:#}", path.display()))?;
    Ok(json!({
        "contigs": s.contigs,
        "total_length": s.length,
        "n50": s.n50,
        "longest": s.largest,
        "gc_percent": s.gc.unwrap_or(0.0),
    }))
}

/// `coverm genome`'s table, read by isotable's parser: per genome (named after the file for one
/// genome, as isotable joins it) the mean depth and the percentage of the genome covered.
pub fn coverm_genome_summary(path: &Path) -> Result<Vec<Value>, String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match isotable::parse::coverm_genome(path, &name) {
        Ok(Some(recs)) => Ok(isotable_records(recs, "sample")),
        Ok(None) => Err(format!("{}: not a `coverm genome` table with Mean and Covered Fraction columns", path.display())),
        Err(e) => Err(format!("{}: {e:#}", path.display())),
    }
}

/// galah's cluster definition (`representative<TAB>member` lines), as clusters sorted by
/// representative, members sorted: `{representative, members, size}`. galah writes the lines in a
/// different order between runs with several threads, so they are sorted here.
pub fn galah_clusters(path: &Path) -> Result<Vec<Value>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut clusters: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for l in text.lines().filter(|l| !l.trim().is_empty()) {
        let (rep, member) = l.split_once('\t').ok_or_else(|| format!("{}: not a galah cluster file (line {l:?})", path.display()))?;
        clusters.entry(rep.to_string()).or_default().push(member.to_string());
    }
    Ok(clusters
        .into_iter()
        .map(|(rep, mut members)| {
            members.sort();
            json!({"representative": rep, "size": members.len(), "members": members})
        })
        .collect())
}

/// `coverm genome`'s dense table with several samples: per sample (named after its read 1 file),
/// the `unmapped` row's relative abundance and how many genomes have a relative abundance above 0;
/// and the genome count. Missing Relative Abundance columns are an error.
pub fn coverm_abundance_summary(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (header, rows) = parse_table(&text);
    if header.first().map(String::as_str) != Some("Genome") {
        return Err(format!("{}: not a `coverm genome` table (no Genome column)", path.display()));
    }
    const RA: &str = " Relative Abundance (%)";
    let samples: Vec<(usize, &str)> = header.iter().enumerate().filter_map(|(i, h)| h.strip_suffix(RA).map(|s| (i, s))).collect();
    if samples.is_empty() {
        return Err(format!("{}: no Relative Abundance columns", path.display()));
    }
    let rows: Vec<&Vec<String>> = rows.iter().filter(|r| r.len() == header.len()).collect();
    let genomes: Vec<&Vec<String>> = rows.iter().copied().filter(|r| r[0] != "unmapped").collect();
    let num = |s: &str| s.parse::<f64>().ok();
    let per_sample: Vec<Value> = samples
        .iter()
        .map(|&(i, s)| {
            let unmapped = rows.iter().find(|r| r[0] == "unmapped").and_then(|r| num(&r[i]));
            json!({
                "sample": s,
                "unmapped_percent": unmapped,
                "genomes_detected": genomes.iter().filter(|r| num(&r[i]).is_some_and(|x| x > 0.0)).count(),
            })
        })
        .collect();
    Ok(json!({
        "genomes": genomes.len(),
        "samples": per_sample,
        "columns_per_sample": header.iter().skip(1).filter_map(|h| h.strip_prefix(samples[0].1)?.strip_prefix(' ')).collect::<Vec<_>>(),
    }))
}

/// Kleborate columns returned per assembly (the part of the header after the last `__`).
const KLEBORATE_KEEP: [&str; 22] = [
    "strain", "species", "species_match", "QC_warnings", "ST", "virulence_score", "resistance_score", "num_resistance_classes",
    "num_resistance_genes", "Yersiniabactin", "YbST", "Colibactin", "CbST", "Aerobactin", "AbST", "Salmochelin", "SmST", "RmpADC",
    "RmST", "rmpA2", "K_locus", "O_locus",
];

/// Kleborate's columns that say how sure Kaptive is, and what the loci mean.
const KLEBORATE_KAPTIVE: [&str; 6] = ["K_type", "K_locus_confidence", "K_locus_identity", "O_type", "O_locus_confidence", "O_locus_identity"];

/// Kleborate 3's table, one object per assembly: species, ST, scores, the virulence loci, K/O loci
/// (with Kaptive's confidence), and the AMR columns that found something (`amr`, e.g.
/// `Bla_ESBL_acquired: CTX-M-15`). isotable's parser must read the file too.
pub fn kleborate_summary(path: &Path) -> Result<Vec<Value>, String> {
    isotable::parse::kleborate(path).map_err(|e| format!("{}: {e:#}", path.display()))?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (header, rows) = parse_table(&text);
    let short: Vec<&str> = header.iter().map(|h| h.rsplit("__").next().unwrap_or(h)).collect();
    Ok(rows
        .iter()
        .filter(|r| r.len() == header.len())
        .map(|r| {
            let mut o = Map::new();
            let mut amr = Map::new();
            for ((h, k), cell) in header.iter().zip(&short).zip(r) {
                if KLEBORATE_KEEP.contains(k) || KLEBORATE_KAPTIVE.contains(k) {
                    o.insert(k.to_string(), json!(cell));
                } else if h.contains("__amr__") && cell != "-" && !cell.is_empty() {
                    amr.insert(k.to_string(), json!(cell));
                }
            }
            o.insert("amr".into(), Value::Object(amr));
            Value::Object(o)
        })
        .collect())
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
    // isotable's list, as a list: one entry per element symbol and name (amrfinder_genes).
    let maps: Vec<std::collections::BTreeMap<String, String>> =
        rows.iter().filter(|r| r.len() == header.len()).map(|r| header.iter().cloned().zip(r.iter().cloned()).collect()).collect();
    Ok(json!({
        "sample": isotable["sample"],
        "amr_genes": isotable::parse::amrfinder_genes(&maps),
        "counts_by_type": counts,
        "hits": hits,
    }))
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
        // gtdbprep writes `-` for a genome skani could not sketch (too short): not screened.
        if g["markers"] == "-" {
            g["screened"] = json!(false);
            g["note"] = json!("not screened: skani made no sketch of it, usually because the assembly is too short");
        }
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

/// Columns of sylph 1.0.0's profile table (`print_header` in src/contain.rs); the coverage column is
/// `True_cov` with -u, else `Eff_cov`.
const SYLPH_COLUMNS: [&str; 6] = ["Sample_file", "Genome_file", "Taxonomic_abundance", "Sequence_abundance", "Adjusted_ANI", "Contig_name"];

/// sylph's profile, per sample in the order given (`(Sample_file, name)`, then any other sample in
/// the table): genomes detected, the sum of Sequence_abundance (with -u the share of the sample the
/// genomes account for), and the `top` genomes by taxonomic abundance.
pub fn sylph_summary(path: &Path, samples: &[(String, String)], top: usize) -> Result<Vec<Value>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (header, rows) = parse_table(&text);
    let col = |name: &str| header.iter().position(|h| h == name);
    let idx: Vec<usize> = SYLPH_COLUMNS
        .iter()
        .map(|c| col(c).ok_or_else(|| format!("{}: no {c} column; not a sylph profile table", path.display())))
        .collect::<Result<_, _>>()?;
    let [s_file, g_file, tax, seq, ani, contig] = idx[..] else { unreachable!() };
    let cov = col("True_cov").or_else(|| col("Eff_cov"));
    let num = |s: &str| s.parse::<f64>().ok();
    let rows: Vec<&Vec<String>> = rows.iter().filter(|r| r.len() == header.len()).collect();
    let mut order: Vec<(String, String)> = samples.to_vec();
    for r in &rows {
        if !order.iter().any(|(f, _)| *f == r[s_file]) {
            order.push((r[s_file].clone(), r[s_file].clone()));
        }
    }
    Ok(order
        .iter()
        .map(|(file, name)| {
            let mut hits: Vec<&&Vec<String>> = rows.iter().filter(|r| r[s_file] == *file).collect();
            hits.sort_by(|a, b| num(&b[tax]).unwrap_or(0.0).total_cmp(&num(&a[tax]).unwrap_or(0.0)));
            let sum: f64 = hits.iter().filter_map(|r| num(&r[seq])).sum();
            json!({
                "sample": name,
                "sample_file": file,
                "genomes_detected": hits.len(),
                // + 0.0: no `-0.0` when nothing was detected
                "sequence_abundance_sum": (sum * 1000.0).round() / 1000.0 + 0.0,
                "top": hits.iter().take(top).map(|r| {
                    let mut o = json!({
                        "genome": r[g_file],
                        "contig_name": r[contig],
                        "taxonomic_abundance": num(&r[tax]),
                        "sequence_abundance": num(&r[seq]),
                        "adjusted_ani": num(&r[ani]),
                    });
                    if let Some(c) = cov {
                        o[header[c].to_lowercase()] = json!(num(&r[c]));
                    }
                    o
                }).collect::<Vec<_>>(),
            })
        })
        .collect())
}

/// abricate's table (`#FILE SEQUENCE START END STRAND GENE COVERAGE COVERAGE_MAP GAPS %COVERAGE
/// %IDENTITY DATABASE ACCESSION PRODUCT RESISTANCE`): the genes found per file, and per gene the files
/// it was found in with its best identity and coverage.
pub fn abricate_summary(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let (header, rows) = parse_table(&text);
    if header.first().map(String::as_str) != Some("#FILE") {
        return Err(format!("{}: not an abricate table (no #FILE column)", path.display()));
    }
    let col = |name: &str| header.iter().position(|h| h == name).ok_or_else(|| format!("{}: no {name} column", path.display()));
    let (gene, ident, cover, product, resistance) = (col("GENE")?, col("%IDENTITY")?, col("%COVERAGE")?, col("PRODUCT")?, col("RESISTANCE")?);
    let rows: Vec<&Vec<String>> = rows.iter().filter(|r| r.len() == header.len()).collect();
    let num = |s: &str| s.parse::<f64>().ok();
    let mut per_file: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut per_gene: BTreeMap<&str, Vec<&&Vec<String>>> = BTreeMap::new();
    for r in &rows {
        per_file.entry(r[0].as_str()).or_default().push(r[gene].as_str());
        per_gene.entry(r[gene].as_str()).or_default().push(r);
    }
    let files: Vec<Value> = per_file
        .into_iter()
        .map(|(f, mut genes)| {
            genes.sort();
            genes.dedup();
            json!({"file": f, "genes": genes})
        })
        .collect();
    let genes: Vec<Value> = per_gene
        .into_iter()
        .map(|(g, hits)| {
            let mut in_files: Vec<&str> = hits.iter().map(|r| r[0].as_str()).collect();
            in_files.dedup();
            let best = |i: usize| hits.iter().filter_map(|r| num(&r[i])).fold(None, |m: Option<f64>, x| Some(m.map_or(x, |m| m.max(x))));
            json!({
                "gene": g,
                "hits": hits.len(),
                "files": in_files,
                "best_identity": best(ident),
                "best_coverage": best(cover),
                "product": hits[0][product],
                "resistance": hits[0][resistance],
            })
        })
        .collect();
    Ok(json!({"hits": rows.len(), "files": files, "genes": genes}))
}

/// A hAMRonization table (from `hamronize <tool>` or `hamronize summarize`), per input file: the
/// genes (with the mutation for variant rows, as isotable lists them) and which tool run reported
/// which. An empty file (a report without hits) has none.
pub fn hamronized_summary(path: &Path) -> Result<Value, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(json!({"rows": 0, "samples": []}));
    }
    let (header, rows) = parse_table(&text);
    let col = |name: &str| header.iter().position(|h| h == name).ok_or_else(|| format!("{}: no {name} column; not a hAMRonization table", path.display()));
    let (file, gene, tool, tool_v, db, db_v, kind) = (
        col("input_file_name")?,
        col("gene_symbol")?,
        col("analysis_software_name")?,
        col("analysis_software_version")?,
        col("reference_database_name")?,
        col("reference_database_version")?,
        col("genetic_variation_type")?,
    );
    let (aa, nt) = (header.iter().position(|h| h == "amino_acid_mutation"), header.iter().position(|h| h == "nucleotide_mutation"));
    let rows: Vec<&Vec<String>> = rows.iter().filter(|r| r.len() == header.len()).collect();
    let mut samples: BTreeMap<&str, BTreeMap<String, Vec<String>>> = BTreeMap::new();
    for r in &rows {
        let mut name = r[gene].clone();
        if r[kind] != "gene_presence_detected" {
            let m = [aa, nt].into_iter().flatten().map(|i| r[i].as_str()).find(|m| !m.is_empty() && *m != "NA");
            if let Some(m) = m {
                name = format!("{name} {m}");
            }
        }
        let run = format!("{} {}, {} {}", r[tool], r[tool_v], r[db], r[db_v]);
        samples.entry(r[file].as_str()).or_default().entry(run).or_default().push(name);
    }
    let samples: Vec<Value> = samples
        .into_iter()
        .map(|(f, runs)| {
            let mut all: Vec<String> = runs.values().flatten().cloned().collect();
            all.sort();
            all.dedup();
            let by_tool: Map<String, Value> = runs
                .into_iter()
                .map(|(run, mut g)| {
                    g.sort();
                    g.dedup();
                    (run, json!(g))
                })
                .collect();
            json!({"input_file_name": f, "genes": all, "by_tool": by_tool})
        })
        .collect();
    Ok(json!({"rows": rows.len(), "samples": samples}))
}

/// The files `sheetgen verify` found a problem with, each with why: the first problem in it
/// (and where), an MD5 or size that differs from the expected one, or no expected MD5 for it.
pub fn reads_problems(report: &Value) -> Vec<Value> {
    let files = report["files"].as_array().map(Vec::as_slice).unwrap_or_default();
    files
        .iter()
        .filter(|f| f["status"].as_str().is_some_and(|s| s != "ok"))
        .map(|f| {
            let mut why = Vec::new();
            let p = &f["problem"];
            if let Some(what) = p["what"].as_str() {
                let at: Vec<String> = [("byte", "byte_offset"), ("record", "record"), ("line", "line")]
                    .iter()
                    .filter_map(|(word, k)| p[*k].as_u64().map(|n| format!("{word} {n}")))
                    .collect();
                why.push(if at.is_empty() { what.to_string() } else { format!("{what} (at {})", at.join(", ")) });
            }
            let from = f["expected_from"].as_str().unwrap_or("the expected list");
            if let (Some(got), Some(want)) = (f["md5"].as_str(), f["expected_md5"].as_str())
                && got != want
            {
                why.push(format!("MD5 {got}, but {from} lists {want}"));
            }
            if let (Some(got), Some(want)) = (f["bytes"].as_u64(), f["expected_bytes"].as_u64())
                && got != want
            {
                why.push(format!("{got} bytes, but {from} lists {want}"));
            }
            // sheetgen's Verdict::name: "not listed", with a space
            if f["status"] == "not listed" {
                why.push("no expected MD5 for it in the md5 files or ENA".into());
            }
            if why.is_empty() {
                why.push(format!("status {}", f["status"].as_str().unwrap_or("?")));
            }
            json!({"path": f["path"], "status": f["status"], "why": why.join("; ")})
        })
        .collect()
}

/// Output files a tool result lists; the rest are counted per folder (the run record has them all).
pub const RESULT_OUTPUTS: usize = 20;
/// Characters of JSON a tool result is cut down to. MCP clients refuse larger results (Claude
/// Code: about 25,000 tokens; a 73,000-character isotable result was refused).
/// `IRIT_MCP_RESULT_CHARS` changes it.
pub const RESULT_CHARS: usize = 40_000;
/// Strings up to this many characters are never cut.
const MIN_STRING: usize = 200;

/// One step into a JSON value: an object key or a list index.
#[derive(Debug, Clone)]
enum Step {
    Key(String),
    Index(usize),
}

/// `run.calls.0.inputs`: how a nested list is named in `result_shortened` and `full_tables`.
fn path_name(path: &[Step]) -> String {
    path.iter()
        .map(|s| match s {
            Step::Key(k) => k.clone(),
            Step::Index(i) => i.to_string(),
        })
        .collect::<Vec<_>>()
        .join(".")
}

fn at<'a>(v: &'a mut Value, path: &[Step]) -> Option<&'a mut Value> {
    path.iter().try_fold(v, |v, s| match s {
        Step::Key(k) => v.get_mut(k.as_str()),
        Step::Index(i) => v.get_mut(*i),
    })
}

/// The largest things that can be cut, each as (its JSON length, where it is).
#[derive(Default)]
struct Cuttable {
    /// A list of two or more rows.
    list: Option<(usize, Vec<Step>)>,
    /// A string over MIN_STRING characters.
    string: Option<(usize, Vec<Step>)>,
    /// A list of one row.
    single: Option<(usize, Vec<Step>)>,
}

fn json_len(s: &str) -> usize {
    serde_json::to_string(s).map_or(0, |x| x.len())
}

/// The length of `v` as compact JSON (what serde_json writes), noting the largest lists and
/// strings anywhere in it. `result_shortened` is counted but never cut.
fn measure(v: &Value, path: &mut Vec<Step>, cut: &mut Cuttable) -> usize {
    let keep = |slot: &mut Option<(usize, Vec<Step>)>, n: usize, path: &[Step]| {
        if slot.as_ref().is_none_or(|(m, _)| n > *m) {
            *slot = Some((n, path.to_vec()));
        }
    };
    match v {
        Value::Array(a) => {
            let mut n = 2 + a.len().saturating_sub(1);
            for (i, x) in a.iter().enumerate() {
                path.push(Step::Index(i));
                n += measure(x, path, cut);
                path.pop();
            }
            match a.len() {
                0 => {}
                1 => keep(&mut cut.single, n, path),
                _ => keep(&mut cut.list, n, path),
            }
            n
        }
        Value::Object(m) => {
            let mut n = 2 + m.len().saturating_sub(1);
            for (k, x) in m {
                n += json_len(k) + 1;
                if path.is_empty() && k == "result_shortened" {
                    n += x.to_string().len();
                    continue;
                }
                path.push(Step::Key(k.clone()));
                n += measure(x, path, cut);
                path.pop();
            }
            n
        }
        Value::String(s) => {
            let n = json_len(s);
            if s.chars().count() > MIN_STRING {
                keep(&mut cut.string, n, path);
            }
            n
        }
        other => other.to_string().len(),
    }
}

/// Fits a tool result into `limit` characters, the same way for every tool. A long `outputs`
/// list keeps the files nearest the output folder and counts the rest per folder. Then, while
/// the result (with the note about what was cut) is too large, the larger of the largest list
/// anywhere in it with two or more rows (`genomes`, `report.files`, `run.calls`, `call.inputs`)
/// and the longest string (over 200 characters) is halved; when neither is left, one-row lists
/// are emptied. `result_shortened`
/// names each shortened list and where all its rows are: the file the tool gave in `full_tables`
/// (list name -> path), else the run record. A list named in `keep_last` keeps its last rows
/// instead of its first (describe_run's calls: the newest). Both hint keys are removed here.
/// Returns whether the result changed.
pub fn fit_result(v: &mut Value, limit: usize) -> bool {
    let Some(obj) = v.as_object_mut() else { return false };
    let full = match obj.remove("full_tables") {
        Some(Value::Object(m)) => m,
        _ => Map::new(),
    };
    let keep_last: Vec<String> = match obj.remove("keep_last") {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect(),
        _ => Vec::new(),
    };
    let mut changed = !full.is_empty() || !keep_last.is_empty();
    let record = match obj.get("call_id").and_then(Value::as_u64) {
        Some(id) => format!("the run record: describe_run with call_id {id}"),
        None => "the run record".to_string(),
    };
    if let Some(Value::Array(outs)) = obj.get_mut("outputs")
        && outs.len() > RESULT_OUTPUTS
    {
        let total = outs.len();
        let depth = |o: &Value| o["path"].as_str().unwrap_or("").matches('/').count();
        let mut order: Vec<usize> = (0..total).collect();
        order.sort_by_key(|&i| (depth(&outs[i]), i));
        let keep: std::collections::BTreeSet<usize> = order[..RESULT_OUTPUTS].iter().copied().collect();
        let mut folders: BTreeMap<String, u64> = BTreeMap::new();
        let mut kept = Vec::with_capacity(RESULT_OUTPUTS);
        for (i, o) in outs.drain(..).enumerate() {
            if keep.contains(&i) {
                kept.push(o);
            } else {
                let dir = Path::new(o["path"].as_str().unwrap_or("")).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
                *folders.entry(dir).or_default() += 1;
            }
        }
        *outs = kept;
        obj.insert("outputs_not_listed".into(), json!({"count": total - RESULT_OUTPUTS, "by_folder": folders, "listed_in": record}));
        changed = true;
    }
    let (mut lists, mut strings) = (Map::new(), Map::new());
    loop {
        if !lists.is_empty() || !strings.is_empty() {
            let mut note = json!({"why": format!("the whole result is over {limit} characters, more than MCP clients accept"), "lists": lists});
            if !strings.is_empty() {
                note["strings"] = json!(strings);
            }
            v["result_shortened"] = note;
        }
        let mut cut = Cuttable::default();
        if measure(v, &mut Vec::new(), &mut cut) <= limit {
            break;
        }
        changed = true;
        // the larger of the largest list and the longest string; one-row lists last
        let list_first = match (&cut.list, &cut.string) {
            (Some((l, _)), Some((s, _))) => l >= s,
            (list, string) => list.is_some() || string.is_none(),
        };
        let list = if list_first { cut.list.or(cut.single.filter(|_| cut.string.is_none())) } else { None };
        if let Some((_, path)) = list {
            let name = path_name(&path);
            let Some(Value::Array(rows)) = at(v, &path) else { break };
            let total = lists.get(&name).and_then(|e: &Value| e["total"].as_u64()).unwrap_or(rows.len() as u64);
            let keep = rows.len() / 2;
            let last = keep_last.contains(&name);
            if last {
                rows.drain(..rows.len() - keep);
            } else {
                rows.truncate(keep);
            }
            let all_in = full.get(&name).cloned().unwrap_or_else(|| json!(record));
            let mut entry = json!({"shown": keep, "total": total, "all_rows_in": all_in});
            if last {
                entry["kept"] = json!("the last rows");
            }
            lists.insert(name, entry);
        } else if let Some((_, path)) = cut.string {
            let name = path_name(&path);
            let Some(Value::String(text)) = at(v, &path) else { break };
            let chars = text.chars().count();
            let length = strings.get(&name).and_then(|e: &Value| e["length"].as_u64()).unwrap_or(chars as u64);
            let shown = chars / 2;
            *text = text.chars().take(shown).collect::<String>() + " [cut]";
            strings.insert(name, json!({"length": length, "shown": shown}));
        } else {
            break;
        }
    }
    changed
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
        let stdout = "genome\tmarkers\tcandidates\tassigned_by_ani\tused\nb168\t4089\t21\ttrue\t14\nfrag\t12\t0\tfalse\t0\ntiny\t-\t-\tfalse\t-\n\
                      21 reference(s) screened in, 14 could give the species call (--species-only)\n";
        let g = gtdbprep_ani_summary(stdout, d.path()).unwrap();
        assert_eq!(g[0]["species"], "s__Bacillus subtilis");
        assert_eq!((&g[0]["ani"], &g[0]["af"], &g[0]["used"], &g[0]["assigned_by_ani"]), (&json!(100.0), &json!(1.0), &json!(14), &json!(true)));
        assert_eq!(g[1]["assigned_by_ani"], false);
        assert!(g[1].get("species").is_none());
        assert!(g[1].get("screened").is_none());
        assert_eq!((&g[2]["screened"], &g[2]["assigned_by_ani"]), (&json!(false), &json!(false)));
        assert!(gtdbprep_ani_summary("nothing\n", d.path()).is_err());
    }

    #[test]
    fn galah_and_coverm_abundance_tables() {
        let d = tempfile::tempdir().unwrap();
        // galah 0.6.3 with several threads: lines in any order
        let c = d.path().join("clusters.tsv");
        std::fs::write(&c, "b/s2_1.fa\tb/s2_1.fa\na/s1_0.fa\ta/s1_0.fa\nb/s2_1.fa\ta/s1_3.fa\na/s1_0.fa\tb/s2_0.fa\n").unwrap();
        assert_eq!(
            galah_clusters(&c).unwrap(),
            [
                json!({"representative": "a/s1_0.fa", "size": 2, "members": ["a/s1_0.fa", "b/s2_0.fa"]}),
                json!({"representative": "b/s2_1.fa", "size": 2, "members": ["a/s1_3.fa", "b/s2_1.fa"]}),
            ]
        );
        std::fs::write(&c, "no tab here\n").unwrap();
        assert!(galah_clusters(&c).is_err());
        // as CoverM 0.8.0 wrote it for two samples (read 1 file names) and three genomes
        let t = d.path().join("ab.tsv");
        std::fs::write(
            &t,
            "Genome\ts1_R1.fq.gz Relative Abundance (%)\ts1_R1.fq.gz Mean\ts1_R1.fq.gz Covered Fraction\ts1_R1.fq.gz Read Count\t\
             s2_R1.fq.gz Relative Abundance (%)\ts2_R1.fq.gz Mean\ts2_R1.fq.gz Covered Fraction\ts2_R1.fq.gz Read Count\n\
             unmapped\t11.1\tNA\tNA\tNA\t15.151512\tNA\tNA\tNA\n\
             bin.3\t45.47339\t0.26225328\t0.20197713\t6000\t4.6665273\t0.043685634\t0.041151453\t1000\n\
             bin.5\t43.42661\t0.25044912\t0.20406914\t2001\t53.476135\t0.50061613\t0.342701\t4000\n\
             bin.12\t0\t0\t0\t0\t26.705828\t0.25000626\t0.21438038\t600\n",
        )
        .unwrap();
        let s = coverm_abundance_summary(&t).unwrap();
        assert_eq!(s["genomes"], 3);
        assert_eq!(s["samples"][0], json!({"sample": "s1_R1.fq.gz", "unmapped_percent": 11.1, "genomes_detected": 2}));
        assert_eq!(s["samples"][1]["genomes_detected"], 3);
        assert_eq!(s["columns_per_sample"], json!(["Relative Abundance (%)", "Mean", "Covered Fraction", "Read Count"]));
        std::fs::write(&t, "Genome\ts1 Mean\nbin.3\t1\n").unwrap();
        assert!(coverm_abundance_summary(&t).is_err());
    }

    #[test]
    fn kleborate_and_coverm_genome_tables() {
        let d = tempfile::tempdir().unwrap();
        let k = d.path().join("klebsiella_pneumo_complex_output.txt");
        // columns as Kleborate 3.2.4 wrote them for BfR-EC-20867 (shortened)
        std::fs::write(
            &k,
            "strain\tenterobacterales__species__species\tenterobacterales__species__species_match\tgeneral__contig_stats__QC_warnings\tklebsiella_pneumo_complex__mlst__ST\tklebsiella__ybst__Yersiniabactin\tklebsiella_pneumo_complex__amr__Sul_acquired\tklebsiella_pneumo_complex__amr__Tet_acquired\tklebsiella_pneumo_complex__kaptive__K_locus\tklebsiella_pneumo_complex__kaptive__K_locus_confidence\n\
             BfR-EC-20867\tKlebsiella pneumoniae\tstrong\t-\tST716\tybt 14; ICEKp12\tsul2\t-\tKL110\tTypeable\n\
             x\tEscherichia coli\tweak\tN50\t-\t-\t-\t-\t-\t-\n",
        )
        .unwrap();
        let rows = kleborate_summary(&k).unwrap();
        assert_eq!(rows[0]["ST"], "ST716");
        assert_eq!(rows[0]["Yersiniabactin"], "ybt 14; ICEKp12");
        assert_eq!(rows[0]["K_locus_confidence"], "Typeable");
        assert_eq!(rows[0]["amr"], json!({"Sul_acquired": "sul2"}));
        let strains = ["BfR-EC-20867".to_string(), "x".to_string(), "mg".to_string()];
        let w = crate::external::kleborate_warnings(&strains, true, &rows);
        assert_eq!(w.len(), 4, "{w:?}");
        assert!(w[0].starts_with("mg: Kleborate wrote no row"), "{w:?}");
        assert!(w[1].starts_with("x: Kleborate's species call is \"Escherichia coli\", outside"), "{w:?}");
        // another preset: no KpSC warning
        assert!(crate::external::kleborate_warnings(&strains[..1], false, &rows[..1]).is_empty());

        let c = d.path().join("s1.coverm.tsv");
        std::fs::write(&c, "Genome\tcontigs.fa/s1_R1.fq.gz Mean\tcontigs.fa/s1_R1.fq.gz Covered Fraction\ngenome1\t50.337517\t0.9999248\n").unwrap();
        let g = coverm_genome_summary(&c).unwrap();
        assert_eq!(g, [json!({"sample": "s1", "mapped_depth": 50.34, "covered_fraction": 99.99})]);
        assert!(crate::external::coverm_depth_warnings("asm/s1/contigs.fa", "[shovill] Estimated sequencing depth: 52 x\n", &g).is_empty());
        let w = crate::external::coverm_depth_warnings("asm/s1/contigs.fa", "[shovill] Estimated sequencing depth: 160 x\n", &g);
        assert!(w[0].contains("Shovill estimated 160x depth, mapping gives 50.3x"), "{w:?}");
        std::fs::write(&c, "Contig\ts1 Mean\nk1\t5\n").unwrap();
        assert!(coverm_genome_summary(&c).is_err());
    }

    #[test]
    fn sylph_abricate_and_hamronized_tables() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("profile.tsv");
        let head = "Sample_file\tGenome_file\tTaxonomic_abundance\tSequence_abundance\tAdjusted_ANI\tTrue_cov\tANI_5-95_percentile\tEff_lambda\tLambda_5-95_percentile\tMedian_cov\tMean_cov_geq1\tContainment_ind\tNaive_ANI\tkmers_reassigned\tContig_name\n";
        let row = |s: &str, g: &str, tax: f64, seq: f64| format!("{s}\t{g}\t{tax}\t{seq}\t99.5\t3.2\tNA-NA\t3.2\tNA-NA\t3\t3\t100/120\t99.1\t0\t{g}_1 desc\n");
        let text = format!("{head}{}{}{}", row("r/a_R1.fq", "m/x.fa", 20.0, 10.0), row("r/a_R1.fq", "m/y.fa", 80.0, 15.5), row("r/z.fq", "m/x.fa", 100.0, 40.0));
        std::fs::write(&p, text).unwrap();
        let names = [("r/a_R1.fq".to_string(), "a".to_string()), ("r/none.fq".to_string(), "none".to_string())];
        let v = sylph_summary(&p, &names, 1).unwrap();
        assert_eq!(v.len(), 3, "a, none, and z which wasn't named");
        assert_eq!((&v[0]["sample"], &v[0]["genomes_detected"], &v[0]["sequence_abundance_sum"]), (&json!("a"), &json!(2), &json!(25.5)));
        assert_eq!(v[0]["top"], json!([{"genome": "m/y.fa", "contig_name": "m/y.fa_1 desc", "taxonomic_abundance": 80.0, "sequence_abundance": 15.5, "adjusted_ani": 99.5, "true_cov": 3.2}]));
        assert_eq!((&v[1]["genomes_detected"], &v[1]["sequence_abundance_sum"]), (&json!(0), &json!(0.0)));
        assert_eq!(v[2]["sample"], "r/z.fq");
        std::fs::write(&p, "Genome\tMean\n").unwrap();
        assert!(sylph_summary(&p, &names, 1).unwrap_err().contains("Sample_file"));

        let a = d.path().join("abricate.tsv");
        let h = "#FILE\tSEQUENCE\tSTART\tEND\tSTRAND\tGENE\tCOVERAGE\tCOVERAGE_MAP\tGAPS\t%COVERAGE\t%IDENTITY\tDATABASE\tACCESSION\tPRODUCT\tRESISTANCE\n";
        let hit = |f: &str, g: &str, id: &str| format!("{f}\tc1\t1\t9\t+\t{g}\t1-9/9\t===\t0/0\t100.00\t{id}\tncbi\tNG_1\t{g} product\tTETRACYCLINE\n");
        std::fs::write(&a, format!("{h}{}{}{}", hit("g/s1.fna", "tet(A)", "99.00"), hit("g/s1.fna", "tet(A)", "100.00"), hit("g/s2.fna", "sul1", "98.50"))).unwrap();
        let v = abricate_summary(&a).unwrap();
        assert_eq!(v["hits"], 3);
        assert_eq!(v["files"], json!([{"file": "g/s1.fna", "genes": ["tet(A)"]}, {"file": "g/s2.fna", "genes": ["sul1"]}]));
        assert_eq!((&v["genes"][1]["gene"], &v["genes"][1]["hits"], &v["genes"][1]["best_identity"]), (&json!("tet(A)"), &json!(2), &json!(100.0)));
        std::fs::write(&a, h).unwrap();
        assert_eq!(abricate_summary(&a).unwrap()["genes"], json!([]));

        let t = d.path().join("h.tsv");
        std::fs::write(&t, "").unwrap();
        assert_eq!(hamronized_summary(&t).unwrap(), json!({"rows": 0, "samples": []}));
        let head = "input_file_name\tgene_symbol\tgene_name\treference_database_name\treference_database_version\treference_accession\tanalysis_software_name\tanalysis_software_version\tgenetic_variation_type\tamino_acid_mutation\tnucleotide_mutation\n";
        std::fs::write(
            &t,
            format!("{head}s1\tgyrA\tgyrase\tNCBI Reference Gene Database\t2026-10-29.1\tWP_1\tamrfinderplus\t4.2.7\tprotein_variant_detected\tp.S83L\t\ns1\ttet(A)\ttet\tncbi\t2026-Apr-3\tNG_1\tabricate\t1.4.0\tgene_presence_detected\t\t\n"),
        )
        .unwrap();
        let v = hamronized_summary(&t).unwrap();
        assert_eq!(v["samples"][0]["genes"], json!(["gyrA p.S83L", "tet(A)"]));
        assert_eq!(v["samples"][0]["by_tool"]["amrfinderplus 4.2.7, NCBI Reference Gene Database 2026-10-29.1"], json!(["gyrA p.S83L"]));
    }

    #[test]
    fn amr_genes_keep_hits_that_share_a_symbol_apart() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("s1.amrfinder.tsv");
        let h = "Protein id\tContig id\tElement symbol\tElement name\tType\tClass\tMethod\n";
        let rows = "NA\tc1\tblaOXA\tOXA-48 family carbapenem-hydrolyzing class D beta-lactamase\tAMR\tBETA-LACTAM\tBLASTX\n\
                    NA\tc2\tblaOXA\tOXA-1 family class D beta-lactamase\tAMR\tBETA-LACTAM\tBLASTX\n\
                    NA\tc3\tblaOXA\tOXA-10 family class D beta-lactamase\tAMR\tBETA-LACTAM\tPARTIALX\n\
                    NA\tc4\ttet(A)\ttetracycline efflux MFS transporter Tet(A)\tAMR\tTETRACYCLINE\tEXACTX\n\
                    NA\tc5\ttet(A)\ttetracycline efflux MFS transporter Tet(A)\tAMR\tTETRACYCLINE\tEXACTX\n\
                    NA\tc6\tsea\tenterotoxin type A\tVIRULENCE\tNA\tEXACTX\n";
        std::fs::write(&path, format!("{h}{rows}")).unwrap();
        let v = amrfinder_summary(&path).unwrap();
        assert_eq!(
            v["amr_genes"],
            json!([
                "blaOXA (OXA-1 family class D beta-lactamase)",
                "blaOXA (OXA-10 family class D beta-lactamase)",
                "blaOXA (OXA-48 family carbapenem-hydrolyzing class D beta-lactamase)",
                "tet(A) x2"
            ])
        );
        assert_eq!(v["counts_by_type"], json!({"AMR": 5, "VIRULENCE": 1}));
    }

    #[test]
    fn large_results_are_fitted_with_a_pointer_to_the_full_table() {
        // a small result is left alone
        let mut v = json!({"call_id": 3, "genomes": [{"genome": "a"}]});
        assert!(!fit_result(&mut v, RESULT_CHARS));
        assert!(v.get("result_shortened").is_none());

        // 90 intermediate files: the 20 nearest the folder are listed, the rest counted
        let mut outputs: Vec<Value> = (0..90)
            .map(|i| json!({"path": format!("/w/asm/intermediate_contigs/k{i}.contigs.fa"), "size": 1, "sha256": "ab"}))
            .collect();
        outputs.push(json!({"path": "/w/asm/final.contigs.fa", "size": 1, "sha256": "cd"}));
        let mut v = json!({"call_id": 4, "outputs": outputs});
        assert!(fit_result(&mut v, RESULT_CHARS));
        let listed = v["outputs"].as_array().unwrap();
        assert_eq!(listed.len(), RESULT_OUTPUTS);
        assert!(listed.iter().any(|o| o["path"] == "/w/asm/final.contigs.fa"), "{v}");
        assert_eq!(v["outputs_not_listed"]["count"], 71);
        assert_eq!(v["outputs_not_listed"]["by_folder"]["/w/asm/intermediate_contigs"], 71);
        assert!(v["outputs_not_listed"]["listed_in"].as_str().unwrap().contains("call_id 4"));

        // 167 genome rows over the limit: the list is halved until it fits, and says where the rest is
        let genomes: Vec<Value> = (0..167)
            .map(|i| json!({"genome": format!("bin.{i}"), "taxonomy": "d__Bacteria;p__Pseudomonadota;c__Gammaproteobacteria;o__Enterobacterales;f__Enterobacteriaceae;g__Escherichia;s__Escherichia coli", "ani": 98.7}))
            .collect();
        let report = json!({"files": (0..30).map(|i| json!({"path": format!("r{i}.fq.gz"), "status": "ok"})).collect::<Vec<_>>()});
        let mut v = json!({"call_id": 5, "genomes": genomes, "report": report, "full_tables": {"genomes": "/w/ani/ani_summary.tsv"}});
        let before = v.to_string().len();
        assert!(before > 30_000);
        assert!(fit_result(&mut v, 10_000));
        assert!(v.to_string().len() <= 10_000, "{}", v.to_string().len());
        assert!(v.get("full_tables").is_none());
        let s = &v["result_shortened"]["lists"]["genomes"];
        assert_eq!(s["total"], 167);
        assert_eq!(s["shown"], v["genomes"].as_array().unwrap().len());
        assert_eq!(s["all_rows_in"], "/w/ani/ani_summary.tsv");
        assert_eq!(v["genomes"][0]["genome"], "bin.0");
        // a nested list without a file named points at the run record
        let mut v = json!({"call_id": 6, "report": {"files": (0..400).map(|i| json!({"path": format!("reads/r{i}_1.fastq.gz"), "status": "ok", "md5": "0123456789abcdef0123456789abcdef"})).collect::<Vec<_>>()}});
        assert!(fit_result(&mut v, 8_000));
        assert!(v["result_shortened"]["lists"]["report.files"]["all_rows_in"].as_str().unwrap().contains("call_id 6"), "{v}");
        assert_eq!(v["result_shortened"]["lists"]["report.files"]["total"], 400);
        assert!(v.to_string().len() <= 8_000);
    }

    #[test]
    fn the_limit_holds_for_deep_lists_long_strings_and_newest_calls() {
        // the length measured is serde_json's own
        let v = json!({"a": [1, "x\"y", {"b": null, "c": [true, 2.5]}], "é": "\n"});
        assert_eq!(measure(&v, &mut Vec::new(), &mut Cuttable::default()), v.to_string().len());

        // describe_run detail with one call of 3,000 inputs: the list two levels down is cut
        let inputs: Vec<Value> = (0..3000).map(|i| json!({"path": format!("/w/bins/b{i}.fa"), "size": 9, "sha256": "ab".repeat(32)})).collect();
        let mut v = json!({"run": {"calls": [{"id": 1, "inputs": inputs, "stderr": "e".repeat(5000)}]}, "keep_last": ["run.calls"]});
        assert!(fit_result(&mut v, 6_000));
        assert!(v.to_string().len() <= 6_000, "{}", v.to_string().len());
        assert_eq!(v["result_shortened"]["lists"]["run.calls.0.inputs"]["total"], 3000, "{}", v["result_shortened"]);
        assert!(v.get("keep_last").is_none());

        // one huge string and nothing else to cut: it is shortened, and the note says so
        let mut v = json!({"call_id": 2, "stdout": "z".repeat(50_000)});
        assert!(fit_result(&mut v, 5_000));
        assert!(v.to_string().len() <= 5_000);
        assert_eq!(v["result_shortened"]["strings"]["stdout"]["length"], 50_000);
        assert!(v["stdout"].as_str().unwrap().ends_with(" [cut]"));

        // describe_run's calls keep the newest when shortened
        let calls: Vec<Value> = (1..=50).map(|i| json!({"id": i, "mcp_tool": "fastp", "error": "x".repeat(150)})).collect();
        let mut v = json!({"run": {"calls": calls}, "keep_last": ["run.calls"], "full_tables": {"run.calls": "/w/irit-runs/r/run.json"}});
        assert!(fit_result(&mut v, 3_000));
        assert!(v.to_string().len() <= 3_000);
        let kept = v["run"]["calls"].as_array().unwrap();
        assert_eq!(kept.last().unwrap()["id"], 50);
        assert_eq!(v["result_shortened"]["lists"]["run.calls"]["kept"], "the last rows");
        assert_eq!(v["result_shortened"]["lists"]["run.calls"]["all_rows_in"], "/w/irit-runs/r/run.json");
    }


    #[test]
    fn reads_problems_name_each_file_and_why() {
        let report = json!({"files": [
            {"path": "reads/a_1.fastq.gz", "status": "ok", "md5": "aa"},
            {"path": "reads/b_1.fastq.gz", "status": "corrupt", "md5": "bb",
             "problem": {"what": "gzip member 1: CRC-32 differs", "byte_offset": 120, "record": 2, "line": null}},
            {"path": "reads/c_1.fastq.gz", "status": "mismatch", "md5": "cc", "expected_md5": "dd", "expected_from": "ENA ERR1", "bytes": 10, "expected_bytes": 12},
            {"path": "reads/d_1.fastq.gz", "status": "not listed", "md5": "ee"},
            {"path": "reads/e_1.fastq.gz", "status": "corrupt", "problem": {"what": "No such file"}}
        ]});
        let p = reads_problems(&report);
        assert_eq!(p.len(), 4);
        assert_eq!(p[0], json!({"path": "reads/b_1.fastq.gz", "status": "corrupt", "why": "gzip member 1: CRC-32 differs (at byte 120, record 2)"}));
        assert_eq!(p[1]["why"], "MD5 cc, but ENA ERR1 lists dd; 10 bytes, but ENA ERR1 lists 12");
        assert_eq!(p[2]["why"], "no expected MD5 for it in the md5 files or ENA");
        assert_eq!(p[3]["why"], "No such file");
    }

}
