//! Reading GTDB-Tk's small output files, and comparing two `summary.tsv` results.

use anyhow::{Context, Result, bail};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// GTDB-Tk output files the classify step reads from `--align_dir`, relative to it. `{p}` is the
/// prefix. Copied into the bundle when GTDB-Tk wrote them.
pub const BUNDLE_FILES: [&str; 10] = [
    "identify/{p}.bac120.markers_summary.tsv",
    "identify/{p}.ar53.markers_summary.tsv",
    "identify/{p}.translation_table_summary.tsv",
    "identify/{p}.failed_genomes.tsv",
    "align/{p}.bac120.user_msa.fasta.gz",
    "align/{p}.ar53.user_msa.fasta.gz",
    "align/{p}.bac120.filtered.tsv",
    "align/{p}.ar53.filtered.tsv",
    "align/intermediate_results/{p}.align.failed.tsv",
    "align/intermediate_results/{p}.identify.failed.tsv",
];

pub fn bundle_path(template: &str, prefix: &str) -> String {
    template.replace("{p}", prefix)
}

/// `markers_summary.tsv`: genome → number of single-copy markers (unique + multiple-unique,
/// as GTDB-Tk's `get_single_copy_hits` counts them).
pub fn single_copy_counts(text: &str) -> Result<BTreeMap<String, usize>> {
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap_or_default().split('\t').collect();
    let col = |name: &str| header.iter().position(|h| *h == name);
    let (Some(unique), Some(muq)) = (
        col("number_unique_genes"),
        col("number_multiple_unique_genes"),
    ) else {
        bail!("not a GTDB-Tk markers_summary.tsv header");
    };
    let mut out = BTreeMap::new();
    for line in lines.filter(|l| !l.is_empty()) {
        let f: Vec<&str> = line.split('\t').collect();
        let n = |i: usize| f.get(i).and_then(|v| v.parse::<usize>().ok()).unwrap_or(0);
        out.insert(f[0].to_string(), n(unique) + n(muq));
    }
    Ok(out)
}

/// Two-column `genome<TAB>value` files (translation table summary, filtered/failed lists).
pub fn two_columns(text: &str) -> BTreeMap<String, String> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut it = l.splitn(2, '\t');
            (
                it.next().unwrap_or_default().to_string(),
                it.next().unwrap_or_default().trim().to_string(),
            )
        })
        .collect()
}

/// GTDB-Tk's domain call: bacterial if the bac120 single-copy percentage is at least the ar53 one.
pub fn domain_call(bac: usize, ar: usize) -> &'static str {
    let bac_pct = bac as f64 * 100.0 / 120.0;
    let ar_pct = ar as f64 * 100.0 / 53.0;
    if bac_pct >= ar_pct { "bac120" } else { "ar53" }
}

/// FASTA record names in a (possibly gzipped) file.
pub fn fasta_names(path: &Path) -> Result<Vec<String>> {
    use std::io::BufRead;
    let reader = crate::rename::open_fasta(path)?;
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if let Some(h) = line.strip_prefix('>') {
            out.push(h.split_whitespace().next().unwrap_or_default().to_string());
        }
    }
    Ok(out)
}

/// A GTDB-Tk `summary.tsv`: header and rows keyed by `user_genome`.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    pub header: Vec<String>,
    pub rows: BTreeMap<String, Vec<String>>,
}

impl Summary {
    pub fn parse(text: &str) -> Result<Summary> {
        let mut lines = text.lines();
        let header: Vec<String> = lines
            .next()
            .unwrap_or_default()
            .split('\t')
            .map(str::to_string)
            .collect();
        if header.first().map(String::as_str) != Some("user_genome") {
            bail!("not a GTDB-Tk summary.tsv (first column is not user_genome)");
        }
        let mut rows = BTreeMap::new();
        for line in lines.filter(|l| !l.is_empty()) {
            let f: Vec<String> = line.split('\t').map(str::to_string).collect();
            if rows.insert(f[0].clone(), f[1..].to_vec()).is_some() {
                bail!("genome {} appears twice in summary.tsv", f[0]);
            }
        }
        Ok(Summary { header, rows })
    }

    /// Read and merge `<prefix>.bac120.summary.tsv` and `<prefix>.ar53.summary.tsv` from a GTDB-Tk
    /// output folder (either may be absent).
    pub fn read_dir(out_dir: &Path, prefix: &str) -> Result<Summary> {
        let mut merged: Option<Summary> = None;
        for domain in ["bac120", "ar53"] {
            let p = out_dir.join(format!("classify/{prefix}.{domain}.summary.tsv"));
            if !p.is_file() {
                continue;
            }
            let s = Summary::parse(&fs::read_to_string(&p)?)
                .with_context(|| format!("reading {}", p.display()))?;
            match &mut merged {
                None => merged = Some(s),
                Some(m) => {
                    if m.header != s.header {
                        bail!("bac120 and ar53 summaries have different columns");
                    }
                    for (k, v) in s.rows {
                        if m.rows.insert(k.clone(), v).is_some() {
                            bail!("genome {k} is in both the bac120 and ar53 summaries");
                        }
                    }
                }
            }
        }
        merged.with_context(|| format!("no summary.tsv in {}/classify", out_dir.display()))
    }

    /// Rename genomes (opaque → original). Fails if a genome has no mapping.
    pub fn map_ids(&self, map: &BTreeMap<String, String>) -> Result<Summary> {
        let mut rows = BTreeMap::new();
        for (k, v) in &self.rows {
            let Some(orig) = map.get(k) else {
                bail!("summary has unknown genome {k}")
            };
            rows.insert(orig.clone(), v.clone());
        }
        Ok(Summary {
            header: self.header.clone(),
            rows,
        })
    }

    pub fn to_tsv(&self) -> String {
        let mut s = self.header.join("\t");
        s.push('\n');
        for (k, v) in &self.rows {
            s.push_str(k);
            for f in v {
                s.push('\t');
                s.push_str(f);
            }
            s.push('\n');
        }
        s
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Identical,
    /// Columns whose values differ: (column, expected, got).
    Different(Vec<(String, String, String)>),
    MissingFromBundleRun,
    MissingFromBaseline,
    /// `prep` left this input out (empty or not FASTA); the reason.
    ExcludedByPrep(String),
}

/// Compare a baseline (`classify_wf`) summary with the bundle run's, genome by genome.
pub fn compare(baseline: &Summary, bundle: &Summary) -> Result<Vec<(String, Outcome)>> {
    if baseline.header != bundle.header {
        bail!(
            "summary columns differ: {:?} vs {:?}",
            baseline.header,
            bundle.header
        );
    }
    let mut out = Vec::new();
    for (g, want) in &baseline.rows {
        let outcome = match bundle.rows.get(g) {
            None => Outcome::MissingFromBundleRun,
            Some(got) if got == want => Outcome::Identical,
            Some(got) => {
                let n = want.len().max(got.len());
                let diffs = (0..n)
                    .filter(|&i| want.get(i) != got.get(i))
                    .map(|i| {
                        (
                            baseline.header.get(i + 1).cloned().unwrap_or_default(),
                            want.get(i).cloned().unwrap_or_default(),
                            got.get(i).cloned().unwrap_or_default(),
                        )
                    })
                    .collect();
                Outcome::Different(diffs)
            }
        };
        out.push((g.clone(), outcome));
    }
    for g in bundle.rows.keys() {
        if !baseline.rows.contains_key(g) {
            out.push((g.clone(), Outcome::MissingFromBaseline));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HDR: &str = "user_genome\tclassification\tmsa_percent\n";

    #[test]
    fn marker_counts_include_multiple_unique() {
        let t = "name\tnumber_unique_genes\tnumber_multiple_genes\tnumber_multiple_unique_genes\tnumber_missing_genes\n\
                 ga\t117\t1\t2\t0\n";
        assert_eq!(single_copy_counts(t).unwrap()["ga"], 119);
        assert!(single_copy_counts("x\ty\n").is_err());
    }

    #[test]
    fn domain_call_matches_gtdbtk() {
        assert_eq!(domain_call(119, 7), "bac120");
        assert_eq!(domain_call(5, 50), "ar53");
        assert_eq!(domain_call(0, 0), "bac120");
    }

    #[test]
    fn compare_reports_per_genome() {
        let base = Summary::parse(&format!("{HDR}a\td__B\t98.3\nb\td__A\t90\nc\tx\t1\n")).unwrap();
        let mine = Summary::parse(&format!("{HDR}gA\td__B\t98.3\ngB\td__A\t91\n")).unwrap();
        let map = BTreeMap::from([
            ("gA".to_string(), "a".to_string()),
            ("gB".to_string(), "b".to_string()),
        ]);
        let mine = mine.map_ids(&map).unwrap();
        let r = compare(&base, &mine).unwrap();
        assert_eq!(r[0], ("a".into(), Outcome::Identical));
        assert_eq!(
            r[1],
            (
                "b".into(),
                Outcome::Different(vec![("msa_percent".into(), "90".into(), "91".into())])
            )
        );
        assert_eq!(r[2], ("c".into(), Outcome::MissingFromBundleRun));
    }

    #[test]
    fn unmapped_ids_are_an_error() {
        let s = Summary::parse(&format!("{HDR}gZ\tx\t1\n")).unwrap();
        assert!(s.map_ids(&BTreeMap::new()).is_err());
    }

    #[test]
    fn summary_roundtrips() {
        let t = format!("{HDR}a\td__B\t98.3\n");
        assert_eq!(Summary::parse(&t).unwrap().to_tsv(), t);
    }
}
