//! Reading the genome × sample table and the taxonomy tables.

use anyhow::{Context, Result, bail};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Column names `coverm genome` 0.8.0 appends to each sample (`<sample> <Method>`), with the
/// `--methods` name for each and whether it measures abundance.
pub const COVERM_METHODS: &[(&str, &str, bool)] = &[
    ("Relative Abundance (%)", "relative_abundance", true),
    ("Trimmed Mean", "trimmed_mean", true),
    ("Mean", "mean", true),
    ("Read Count", "count", true),
    ("Reads per base", "reads_per_base", true),
    ("RPKM", "rpkm", true),
    ("TPM", "tpm", true),
    ("Covered Bases", "covered_bases", false),
    ("Covered Fraction", "covered_fraction", false),
    ("Variance", "variance", false),
    ("Length", "length", false),
];

/// Abundances, one row per genome and one column per sample.
#[derive(Debug, Clone)]
pub struct Table {
    pub genomes: Vec<String>,
    pub samples: Vec<String>,
    pub values: Vec<Vec<f64>>,
    /// The CoverM column used (`Relative Abundance (%)`), None for a plain table.
    pub measure: Option<String>,
}

/// The CoverM method a column header ends with, and the sample part before it.
fn split_method(col: &str) -> Option<(&str, &'static str)> {
    COVERM_METHODS.iter().find_map(|&(m, _, _)| {
        if col == m {
            Some(("", m))
        } else {
            col.strip_suffix(m).and_then(|s| s.strip_suffix(' ')).map(|s| (s, m))
        }
    })
}

/// The CoverM header a `--measure` value names: the header itself or the `--methods` name, any case.
fn measure_header(want: &str) -> Result<&'static str> {
    let w = want.trim().to_ascii_lowercase();
    COVERM_METHODS
        .iter()
        .find(|&&(h, m, _)| h.to_ascii_lowercase() == w || m == w)
        .map(|&(h, _, _)| h)
        .with_context(|| format!("unknown measure '{want}'; CoverM's are {}", COVERM_METHODS.iter().map(|m| m.1).collect::<Vec<_>>().join(", ")))
}

/// Reads a `coverm genome` table (`Genome`, then `<sample> <Method>` columns, one or more methods)
/// or a plain table (first column genome, one column per sample). CoverM's `unmapped` row is dropped.
pub fn read_table(path: &Path, measure: Option<&str>) -> Result<Table> {
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    parse_table(&text, measure).with_context(|| format!("in {}", path.display()))
}

pub fn parse_table(text: &str, measure: Option<&str>) -> Result<Table> {
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.trim().is_empty() && !l.starts_with('#'));
    let header: Vec<&str> = lines.next().context("the table is empty")?.split('\t').collect();
    if header.len() < 2 {
        bail!("the header has no sample columns (is the table tab-separated?)");
    }
    let methods: Vec<Option<(&str, &str)>> = header[1..].iter().map(|c| split_method(c)).collect();
    let n_coverm = methods.iter().filter(|m| m.is_some()).count();
    let (cols, used): (Vec<usize>, Option<&str>) = if n_coverm == 0 {
        if let Some(m) = measure {
            bail!("--measure {m} given, but no column ends with a CoverM method name");
        }
        ((1..header.len()).collect(), None)
    } else {
        if n_coverm != methods.len() {
            let odd: Vec<&str> = header[1..].iter().zip(&methods).filter(|(_, m)| m.is_none()).map(|(c, _)| *c).collect();
            bail!("some columns end with a CoverM method name and some don't: {}", odd.join(", "));
        }
        let mut present: Vec<&str> = Vec::new();
        for (_, m) in methods.iter().flatten() {
            if !present.contains(m) {
                present.push(m);
            }
        }
        let pick = match measure {
            Some(w) => {
                let h = measure_header(w)?;
                if !present.contains(&h) {
                    bail!("no '{h}' columns; the table has {}", present.join(", "));
                }
                h
            }
            None => {
                let abundance: Vec<&str> = present.iter().copied().filter(|p| COVERM_METHODS.iter().any(|m| m.0 == *p && m.2)).collect();
                match abundance[..] {
                    [one] => one,
                    [] => bail!("the table has no abundance columns ({}); write one with coverm genome -m relative_abundance or -m count", present.join(", ")),
                    _ => bail!("the table has several abundance measures ({}); pick one with --measure", abundance.join(", ")),
                }
            }
        };
        if !COVERM_METHODS.iter().any(|m| m.0 == pick && m.2) {
            bail!("'{pick}' is not an abundance; use relative_abundance, count, mean, trimmed_mean, reads_per_base, rpkm or tpm");
        }
        let cols = (1..header.len()).filter(|&i| methods[i - 1].map(|(_, m)| m) == Some(pick)).collect();
        (cols, Some(pick))
    };
    let samples: Vec<String> = cols
        .iter()
        .map(|&i| match methods[i - 1] {
            Some((s, _)) if !s.is_empty() => s.to_string(),
            Some(_) => format!("sample{i}"),
            None => header[i].to_string(),
        })
        .collect();
    let mut seen = HashSet::new();
    for s in &samples {
        if !seen.insert(s) {
            bail!("sample '{s}' appears twice");
        }
    }
    let mut genomes = Vec::new();
    let mut values = Vec::new();
    let mut seen = HashSet::new();
    for (n, line) in lines.enumerate() {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != header.len() {
            bail!("line {}: {} columns, the header has {}", n + 2, f.len(), header.len());
        }
        let g = f[0].trim();
        if g == "unmapped" && used.is_some() {
            continue;
        }
        if !seen.insert(g.to_string()) {
            bail!("genome '{g}' appears twice");
        }
        let row = cols
            .iter()
            .map(|&i| {
                let v: f64 = f[i].trim().parse().with_context(|| format!("genome {g}, column '{}': '{}' is not a number", header[i], f[i]))?;
                if !v.is_finite() || v < 0.0 {
                    bail!("genome {g}, column '{}': {v} is not a non-negative abundance", header[i]);
                }
                Ok(v)
            })
            .collect::<Result<Vec<f64>>>()?;
        genomes.push(g.to_string());
        values.push(row);
    }
    if genomes.is_empty() {
        bail!("the table has no genome rows");
    }
    Ok(Table { genomes, samples, values, measure: used.map(String::from) })
}

/// A genome name as a match key: no `.gz` and no FASTA extension, so `bin.1.fa` matches `bin.1`.
pub fn genome_key(name: &str) -> String {
    let n = name.trim();
    let n = n.strip_suffix(".gz").unwrap_or(n);
    for ext in [".fasta", ".fna", ".fas", ".fa"] {
        if let Some(s) = n.strip_suffix(ext) {
            return s.to_string();
        }
    }
    n.to_string()
}

/// Reads taxonomy tables into genome key → lineage. Takes GTDB-Tk summaries (`user_genome` and
/// `classification` columns) or two-column tables (genome, lineage) with or without a header.
pub fn read_taxonomy(paths: &[impl AsRef<Path>]) -> Result<HashMap<String, String>> {
    let mut out = HashMap::new();
    for p in paths {
        let p = p.as_ref();
        let text = std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
        for (g, l) in parse_taxonomy(&text).with_context(|| format!("in {}", p.display()))? {
            if let Some(old) = out.insert(genome_key(&g), l.clone())
                && old != l
            {
                bail!("genome {g} has two lineages: {old} and {l}");
            }
        }
    }
    Ok(out)
}

pub fn parse_taxonomy(text: &str) -> Result<Vec<(String, String)>> {
    let mut lines = text.lines().map(|l| l.trim_end_matches('\r')).filter(|l| !l.trim().is_empty() && !l.starts_with('#')).peekable();
    let (gi, li) = match lines.peek() {
        None => return Ok(Vec::new()),
        Some(first) => {
            let cols: Vec<&str> = first.split('\t').collect();
            if let Some(c) = cols.iter().position(|c| *c == "classification") {
                let g = cols.iter().position(|c| *c == "user_genome").unwrap_or(0);
                lines.next();
                (g, c)
            } else {
                if cols.len() < 2 {
                    bail!("expected two tab-separated columns (genome, lineage)");
                }
                if !cols[1].contains("__") && !cols[1].contains(';') {
                    lines.next(); // a header line
                }
                (0, 1)
            }
        }
    };
    let mut out = Vec::new();
    for l in lines {
        let f: Vec<&str> = l.split('\t').collect();
        if let (Some(g), Some(t)) = (f.get(gi), f.get(li)) {
            out.push((g.trim().to_string(), t.trim().to_string()));
        }
    }
    Ok(out)
}

/// The phylum of a lineage (`p__…` rank), or the empty string.
pub fn phylum(lineage: &str) -> &str {
    lineage.split(';').map(str::trim).find(|r| r.starts_with("p__")).unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverm_table_with_two_methods() {
        // header layout of coverm genome 0.8.0 -m relative_abundance count
        let t = "Genome\ts1.fq.gz Relative Abundance (%)\ts1.fq.gz Read Count\ts2.fq.gz Relative Abundance (%)\ts2.fq.gz Read Count\n\
                 unmapped\t50\t100\t40\t80\n\
                 bin.1\t30\t60\t40\t80\n\
                 bin.2\t20\t40\t20\t40\n";
        let e = parse_table(t, None).unwrap_err();
        assert!(format!("{e:#}").contains("several abundance measures"), "{e:#}");
        let a = parse_table(t, Some("relative_abundance")).unwrap();
        assert_eq!(a.samples, ["s1.fq.gz", "s2.fq.gz"]);
        assert_eq!(a.genomes, ["bin.1", "bin.2"]);
        assert_eq!(a.values, vec![vec![30.0, 40.0], vec![20.0, 20.0]]);
        let c = parse_table(t, Some("Read Count")).unwrap();
        assert_eq!(c.values[0], [60.0, 80.0]);
        assert_eq!(c.measure.as_deref(), Some("Read Count"));
    }

    #[test]
    fn trimmed_mean_is_not_mean() {
        let t = "Genome\ts1 Trimmed Mean\ts2 Trimmed Mean\nb\t1\t2\n";
        let a = parse_table(t, None).unwrap();
        assert_eq!((a.samples.join(","), a.measure.as_deref()), ("s1,s2".into(), Some("Trimmed Mean")));
        assert!(parse_table(t, Some("mean")).is_err());
    }

    #[test]
    fn plain_and_refused_tables() {
        let a = parse_table("genome\tA\tB\tC\nx\t1\t0\t2\nunmapped\t1\t1\t1\n", None).unwrap();
        assert_eq!(a.genomes, ["x", "unmapped"]); // a plain table keeps every row
        assert!(parse_table("Genome\ts1 Covered Fraction\nb\t0.5\n", None).is_err());
        assert!(parse_table("Genome\ts1 Mean\ts2\nb\t1\t2\n", None).is_err());
        assert!(parse_table("Genome\ts1 Mean\ts2 Mean\nb\t1\t-2\n", None).is_err());
        assert!(parse_table("Genome\ts1 Mean\ts2 Mean\nb\t1\tNA\n", None).is_err());
        assert!(parse_table("Genome\ts1 Mean\ts1 Mean\nb\t1\t2\n", None).is_err());
        assert!(parse_table("Genome\ts1 Mean\ts2 Mean\nb\t1\t2\nb\t1\t2\n", None).is_err());
    }

    #[test]
    fn taxonomy_formats() {
        let gtdb = "user_genome\tclassification\tclosest_genome_reference\n\
                    bin.1\td__Bacteria;p__Patescibacteriota;c__Paceibacteria\tGCA_1\n\
                    bin.2\td__Bacteria;p__Pseudomonadota;c__Gammaproteobacteria\tGCA_2\n";
        let t = parse_taxonomy(gtdb).unwrap();
        assert_eq!(t[0], ("bin.1".into(), "d__Bacteria;p__Patescibacteriota;c__Paceibacteria".into()));
        assert_eq!(phylum(&t[1].1), "p__Pseudomonadota");
        let plain = parse_taxonomy("genome\tlineage\nbin.3.fa\td__Archaea;p__Nanoarchaeota\n").unwrap();
        assert_eq!(plain, vec![("bin.3.fa".into(), "d__Archaea;p__Nanoarchaeota".into())]);
        let noheader = parse_taxonomy("bin.3\td__Archaea;p__Nanoarchaeota\n").unwrap();
        assert_eq!(noheader.len(), 1);
        assert_eq!(genome_key("bin.3.fa.gz"), "bin.3");
        assert_eq!(genome_key("bin.3"), "bin.3");
    }
}
