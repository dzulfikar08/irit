//! sheetgen: build a pipeline sample sheet from a folder of FASTQ files.
//!
//! Pairs R1/R2 files that sit in the same folder and differ only in the mate token, keeps
//! lanes and runs as separate rows (nf-core pipelines merge rows that share a sample name),
//! and, given a pipeline's `schema_input.json`, writes the exact columns that pipeline
//! expects and checks the rows against the schema before you launch.

mod schema;

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

#[derive(Parser)]
#[command(version, about = "Build a pipeline sample sheet from a folder of FASTQ files")]
struct Cli {
    /// Folders to scan (recursively) for FASTQ files.
    #[arg(required = true)]
    dirs: Vec<PathBuf>,

    /// Output file; `.tsv` writes tabs, anything else commas. Default: stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Output layout when no schema is given.
    #[arg(short, long, value_enum, default_value_t = Format::Nfcore)]
    format: Format,

    /// Pipeline input schema (`assets/schema_input.json`) to take columns and rules from.
    #[arg(long, conflicts_with = "pipeline")]
    schema: Option<PathBuf>,

    /// Pipeline name, e.g. `nf-core/bacass`: finds its schema in Nextflow's asset cache.
    #[arg(long)]
    pipeline: Option<String>,

    /// Fixed value for a column, e.g. `--set strandedness=auto`. Repeatable.
    #[arg(long = "set", value_name = "COLUMN=VALUE")]
    set: Vec<String>,

    /// Name samples after the folder that holds the files instead of the file name.
    #[arg(long)]
    name_from_dir: bool,

    /// Treat unpaired files as long reads (goes in the schema's long-read column).
    #[arg(long)]
    long: bool,

    /// Write paths relative to the current directory instead of absolute.
    #[arg(long)]
    relative: bool,

    /// Create clean symlinks (no spaces, `<sample>_<n>_R1.fastq.gz`) in this folder and
    /// list those instead of the original paths. The original files are not touched.
    #[arg(long, value_name = "DIR")]
    link_into: Option<PathBuf>,

    /// Don't pair files on a bare `_1`/`_2` (or `.1`/`.2`) suffix; only `R1`/`R2` names pair.
    /// Use when `x_1.fq.gz` and `x_2.fq.gz` are two single-end samples.
    #[arg(long)]
    no_pair_numeric: bool,

    /// Allow different sample names that become the same after cleaning (`s 1`, `s_1`); the
    /// pipeline then merges them into one sample.
    #[arg(long)]
    allow_merge: bool,

    /// Write the sheet even if the schema has rules sheetgen can't evaluate.
    #[arg(long)]
    allow_unchecked: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    /// sample,fastq_1,fastq_2 (most nf-core pipelines).
    Nfcore,
    /// QIIME 2 PairedEnd/SingleEnd manifest (TSV, absolute paths).
    Qiime2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Mate {
    R1,
    R2,
    Unpaired,
}

#[derive(Debug, Clone)]
struct Fastq {
    path: PathBuf,
    sample: String,
    lane: String,
    mate: Mate,
    /// File stem with the mate token blanked: R1 and R2 of one pair share it.
    pair_stem: String,
    /// The mate came from a bare `_1`/`_2` suffix rather than `R1`/`R2`.
    numeric: bool,
}

/// How a file stem was read: sample, lane, mate (`None` for index reads), pair stem, numeric.
type Classified = (String, String, Option<Mate>, String, bool);

/// One output row: one sample, one lane.
#[derive(Debug, Clone)]
pub struct Unit {
    pub sample: String,
    pub r1: Option<PathBuf>,
    pub r2: Option<PathBuf>,
    pub long: Option<PathBuf>,
    /// R1/R2 were matched only on a bare `_1`/`_2` suffix.
    pub numeric_pair: bool,
}

static FASTQ_EXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.f(ast)?q(\.gz|\.bz2|\.zst)?$").unwrap());
// bcl2fastq/BCL Convert: Sample_S1_L001_R1_001.fastq.gz
static ILLUMINA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<s>.+?)_S\d+(?:_L(?P<lane>\d{3}))?_(?P<r>[RI][12])_\d{3}$").unwrap()
});
// sample_R1, sample_1, sample.R2, sample_L002_R1_001, sample_R1_001
static GENERIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<s>.+?)(?:_L(?P<lane>\d{3}))?[._-](?P<r>[RI]?[12])(?:_\d{3})?$").unwrap()
});

fn main() -> Result<()> {
    let cli = Cli::parse();
    let fixed = parse_sets(&cli.set)?;

    let (files, skipped) = scan(&cli.dirs, cli.name_from_dir, !cli.no_pair_numeric)?;
    if files.is_empty() {
        bail!("no FASTQ files found under {}", show_dirs(&cli.dirs));
    }
    let (mut units, mut warnings) = pair(files, cli.long)?;
    for s in &skipped {
        warnings.push(format!("skipped index read file {s}"));
    }
    check_names(&units, cli.allow_merge, &mut warnings)?;
    if let Some(dir) = &cli.link_into {
        link_into(&mut units, dir)?;
    }

    let cwd = std::env::current_dir()?;
    let fmt_path = |p: &Path| -> String {
        if cli.relative {
            pathdiff(p, &cwd).display().to_string()
        } else {
            p.display().to_string()
        }
    };

    let schema_path = match (&cli.schema, &cli.pipeline) {
        (Some(p), _) => Some(p.clone()),
        (None, Some(name)) => Some(schema::find_for_pipeline(name)?),
        _ => None,
    };

    let table = if let Some(path) = &schema_path {
        let schema = schema::Schema::load(path, cli.allow_unchecked)?;
        schema.build(&units, &fixed, &fmt_path, &mut warnings)?
    } else {
        match cli.format {
            Format::Nfcore => nfcore_table(&units, &fmt_path),
            Format::Qiime2 => {
                if cli.relative {
                    bail!("QIIME 2 manifests need absolute paths; drop --relative");
                }
                qiime2_table(&units, &fmt_path)?
            }
        }
    };

    let tab = match (&cli.output, cli.format, &schema_path) {
        (Some(o), _, _) => o.extension().is_some_and(|e| e.eq_ignore_ascii_case("tsv")),
        (None, Format::Qiime2, None) => true,
        // bacass and a few others declare TSV input in their test sheets; stdout default is CSV.
        _ => false,
    };
    let text = render(&table, if tab { '\t' } else { ',' })?;
    match &cli.output {
        Some(o) => std::fs::write(o, text).with_context(|| format!("writing {}", o.display()))?,
        None => print!("{text}"),
    }

    let samples: BTreeSet<_> = units.iter().map(|u| &u.sample).collect();
    let paired = units.iter().filter(|u| u.r1.is_some() && u.r2.is_some()).count();
    eprintln!(
        "sheetgen: {} samples, {} rows ({} paired, {} single/long){}",
        samples.len(),
        units.len(),
        paired,
        units.len() - paired,
        schema_path
            .as_ref()
            .map(|p| format!(", columns from {}", p.display()))
            .unwrap_or_default()
    );
    let numeric: BTreeSet<&str> = units.iter().filter(|u| u.numeric_pair).map(|u| u.sample.as_str()).collect();
    if !numeric.is_empty() {
        let list: Vec<_> = numeric.into_iter().collect();
        eprintln!(
            "  note: paired only because of a bare _1/_2 suffix: {} (SRA-style names; if these are separate single-end samples, rerun with --no-pair-numeric)",
            list.join(", ")
        );
    }
    for w in &warnings {
        eprintln!("  warning: {w}");
    }
    Ok(())
}

fn show_dirs(dirs: &[PathBuf]) -> String {
    dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
}

fn parse_sets(sets: &[String]) -> Result<BTreeMap<String, String>> {
    sets.iter()
        .map(|s| {
            let (k, v) = s
                .split_once('=')
                .with_context(|| format!("--set expects COLUMN=VALUE, got `{s}`"))?;
            Ok((k.trim().to_string(), v.trim().to_string()))
        })
        .collect()
}

/// Walk the folders and classify every FASTQ file. Returns the files and skipped index reads.
fn scan(dirs: &[PathBuf], name_from_dir: bool, pair_numeric: bool) -> Result<(Vec<Fastq>, Vec<String>)> {
    let mut files = Vec::new();
    let mut skipped = Vec::new();
    let mut seen = BTreeSet::new();
    for dir in dirs {
        let dir = dir
            .canonicalize()
            .with_context(|| format!("cannot open {}", dir.display()))?;
        for entry in walkdir::WalkDir::new(&dir).follow_links(true).sort_by_file_name() {
            let entry = entry?;
            if !entry.file_type().is_file() {
                continue;
            }
            // Keep the link's own path and name: labs often symlink raw data under clearer
            // names, and the target's name (or a shared target) must not decide the sample.
            let path = entry.path().to_path_buf();
            let name = entry.file_name().to_string_lossy();
            let Some(m) = FASTQ_EXT.find(&name) else { continue };
            if !seen.insert(path.clone()) {
                continue; // same folder given twice, or nested in another argument
            }
            let stem = &name[..m.start()];
            let (mut sample, lane, mate, pair_stem, numeric) = classify(stem, pair_numeric);
            if mate.is_none() {
                skipped.push(path.display().to_string());
                continue;
            }
            if name_from_dir
                && let Some(parent) = path.parent().and_then(|p| p.file_name()) {
                    sample = parent.to_string_lossy().into_owned();
                }
            files.push(Fastq { path, sample, lane, mate: mate.unwrap(), pair_stem, numeric });
        }
    }
    Ok((files, skipped))
}

/// Split a file stem into sample, lane, mate (`None` for an index read I1/I2), the stem with
/// the mate token blanked (shared by the two files of a pair) and whether the mate came from
/// a bare `_1`/`_2`. Without `pair_numeric`, bare `_1`/`_2` are part of the sample name.
fn classify(stem: &str, pair_numeric: bool) -> Classified {
    let unpaired = || (stem.to_string(), String::new(), Some(Mate::Unpaired), stem.to_string(), false);
    let caps = ILLUMINA.captures(stem).or_else(|| GENERIC.captures(stem));
    let Some(c) = caps else { return unpaired() };
    let r = c.name("r").unwrap();
    let numeric = !r.as_str().starts_with(['R', 'r', 'I', 'i']);
    if numeric && !pair_numeric {
        return unpaired();
    }
    let lane = c.name("lane").map(|m| m.as_str().to_string()).unwrap_or_default();
    let mate = match r.as_str().to_ascii_uppercase().as_str() {
        "R1" | "1" => Some(Mate::R1),
        "R2" | "2" => Some(Mate::R2),
        _ => None, // I1 / I2
    };
    let pair_stem = format!("{}\u{0}{}", &stem[..r.start()], &stem[r.end()..]);
    (c["s"].to_string(), lane, mate, pair_stem, numeric)
}

/// Pair mates and make one unit per pair or unpaired file.
///
/// Two files are mates only if they sit in the same folder and their names differ only in the
/// mate token (`A_R1` / `A_R2`). Several pairs for one sample (lanes, runs in other folders)
/// become several rows, which nf-core pipelines merge. Two files claiming the same mate of the
/// same pair (`A_R1.fq.gz` next to `A_R1.fastq.gz`) are an error.
fn pair(files: Vec<Fastq>, long: bool) -> Result<(Vec<Unit>, Vec<String>)> {
    type Key = (String, String, PathBuf, String);
    let mut groups: BTreeMap<Key, Vec<Fastq>> = BTreeMap::new();
    let mut unpaired = Vec::new();
    for f in files {
        if f.mate == Mate::Unpaired {
            unpaired.push(f);
            continue;
        }
        let dir = f.path.parent().map(Path::to_path_buf).unwrap_or_default();
        groups.entry((f.sample.clone(), f.lane.clone(), dir, f.pair_stem.clone())).or_default().push(f);
    }
    let mut units = Vec::new();
    let mut warnings = Vec::new();
    let mut orphans = Vec::new();
    for ((sample, lane, _, _), fs) in groups {
        let at = if lane.is_empty() { sample.clone() } else { format!("{sample} lane {lane}") };
        let pick = |m: Mate| -> Vec<&Fastq> { fs.iter().filter(|f| f.mate == m).collect() };
        let (r1, r2) = (pick(Mate::R1), pick(Mate::R2));
        if r1.len() > 1 || r2.len() > 1 {
            let list: Vec<_> = fs.iter().map(|f| f.path.display().to_string()).collect();
            bail!("{at}: several files claim the same mate of one pair, so it's unclear which belong together: {}", list.join(", "));
        }
        let a = r1.first().map(|f| f.path.clone());
        let b = r2.first().map(|f| f.path.clone());
        match (&a, &b) {
            (Some(p), None) => warnings.push(format!("{at}: R1 without R2 ({}), written as single-end", p.display())),
            (None, Some(p)) => orphans.push(p.display().to_string()),
            _ => {}
        }
        let numeric_pair = a.is_some() && b.is_some() && fs.iter().all(|f| f.numeric);
        units.push(Unit { sample, r1: a, r2: b, long: None, numeric_pair });
    }
    if !orphans.is_empty() {
        bail!(
            "R2 files without an R1 next to them (mates are paired only within one folder, by names that differ only in R1/R2 or _1/_2):\n  {}",
            orphans.join("\n  ")
        );
    }
    for f in unpaired {
        if long {
            // Hybrid assembly: put long reads on the sample's short-read row if it has one.
            if let Some(u) = units
                .iter_mut()
                .rev()
                .find(|u| u.sample == f.sample && u.long.is_none() && u.r1.is_some())
            {
                u.long = Some(f.path.clone());
                continue;
            }
            units.push(Unit { sample: f.sample, r1: None, r2: None, long: Some(f.path), numeric_pair: false });
        } else {
            units.push(Unit { sample: f.sample, r1: Some(f.path), r2: None, long: None, numeric_pair: false });
        }
    }
    // Keep each sample's rows together.
    units.sort_by(|a, b| a.sample.cmp(&b.sample));
    // nf-core pipelines refuse a sample whose runs are partly single-end, partly paired; it
    // usually means a mate file is missing or misnamed.
    let mut kinds: BTreeMap<&str, BTreeSet<bool>> = BTreeMap::new();
    for u in units.iter().filter(|u| u.r1.is_some() || u.r2.is_some()) {
        kinds.entry(&u.sample).or_default().insert(u.r1.is_some() && u.r2.is_some());
    }
    let mixed: Vec<&str> = kinds.into_iter().filter(|(_, k)| k.len() > 1).map(|(s, _)| s).collect();
    if !mixed.is_empty() {
        let detail: Vec<String> = units
            .iter()
            .filter(|u| mixed.contains(&u.sample.as_str()))
            .map(|u| {
                let show = |p: &Option<PathBuf>| p.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| "-".into());
                format!("{}: {} + {}", u.sample, show(&u.r1), show(&u.r2))
            })
            .collect();
        bail!(
            "these samples have both single-end and paired rows, which pipelines reject; a mate is probably missing or named differently:\n  {}",
            detail.join("\n  ")
        );
    }
    Ok((units, warnings))
}

/// Different names that become the same after cleaning would be merged into one sample by the
/// pipeline: an error unless `allow_merge`.
fn check_names(units: &[Unit], allow_merge: bool, warnings: &mut Vec<String>) -> Result<()> {
    let mut by_clean: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for u in units {
        by_clean.entry(clean_name(&u.sample)).or_default().insert(&u.sample);
    }
    let mut clashes = Vec::new();
    for (clean, originals) in by_clean {
        if originals.len() > 1 {
            let list: Vec<_> = originals.into_iter().map(|o| format!("`{o}`")).collect();
            clashes.push(format!("{} all become `{clean}`", list.join(", ")));
        }
    }
    if clashes.is_empty() {
        return Ok(());
    }
    if !allow_merge {
        bail!(
            "sample names collide after cleaning, and the pipeline would merge them into one sample:\n  {}\n\
             rename the files, or pass --allow-merge if they really are one sample",
            clashes.join("\n  ")
        );
    }
    for c in clashes {
        warnings.push(format!("merged as one sample (--allow-merge): {c}"));
    }
    Ok(())
}

/// Replace every path with a clean symlink in `dir`, numbered per sample so lanes stay apart.
fn link_into(units: &mut [Unit], dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let dir = dir.canonicalize()?;
    let mut n_per_sample: BTreeMap<String, usize> = BTreeMap::new();
    for u in units.iter_mut() {
        let name = clean_name(&u.sample);
        let n = n_per_sample.entry(name.clone()).or_default();
        *n += 1;
        let n = *n;
        for (slot, tag) in [(&mut u.r1, "R1"), (&mut u.r2, "R2"), (&mut u.long, "long")] {
            let Some(src) = slot.as_ref() else { continue };
            let file = src.file_name().unwrap_or_default().to_string_lossy();
            let ext = FASTQ_EXT.find(&file).map(|m| m.as_str().to_ascii_lowercase()).unwrap_or_default();
            let link = dir.join(format!("{name}_{n}_{tag}{ext}"));
            let target = src.canonicalize().with_context(|| format!("resolving {}", src.display()))?;
            match std::fs::read_link(&link) {
                Ok(existing) if existing == target => {}
                Ok(_) | Err(_) if link.symlink_metadata().is_ok() => {
                    bail!("{} already exists and points elsewhere; use an empty folder", link.display())
                }
                _ => symlink_file(&target, &link).with_context(|| format!("linking {}", link.display()))?,
            }
            *slot = Some(link);
        }
    }
    Ok(())
}

#[cfg(unix)]
fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Windows allows file symlinks only with Developer Mode or admin rights; the error says so.
#[cfg(windows)]
fn symlink_file(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

pub fn clean_name(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') { c } else { '_' })
        .collect()
}

type Table = (Vec<String>, Vec<Vec<String>>);

fn nfcore_table(units: &[Unit], fmt: &dyn Fn(&Path) -> String) -> Table {
    let header = vec!["sample".into(), "fastq_1".into(), "fastq_2".into()];
    let rows = units
        .iter()
        .map(|u| {
            let r1 = u.r1.as_ref().or(u.long.as_ref());
            vec![
                clean_name(&u.sample),
                r1.map(|p| fmt(p)).unwrap_or_default(),
                u.r2.as_ref().map(|p| fmt(p)).unwrap_or_default(),
            ]
        })
        .collect();
    (header, rows)
}

fn qiime2_table(units: &[Unit], fmt: &dyn Fn(&Path) -> String) -> Result<Table> {
    let mut count: BTreeMap<&str, usize> = BTreeMap::new();
    for u in units {
        *count.entry(&u.sample).or_default() += 1;
    }
    if let Some((s, _)) = count.iter().find(|(_, n)| **n > 1) {
        bail!(
            "sample `{s}` has several lanes; QIIME 2 needs one row per sample, so concatenate lanes first (cat L001 L002 > merged)"
        );
    }
    if let Some(u) = units.iter().find(|u| u.r1.is_none() && u.r2.is_some()) {
        bail!("sample `{}` has an R2 file but no R1; fix the file names first", u.sample);
    }
    let paired = units.iter().filter(|u| u.r2.is_some()).count();
    if paired != 0 && paired != units.len() {
        bail!("mix of paired and single-end samples; QIIME 2 imports one kind per manifest");
    }
    if paired > 0 {
        let header = ["sample-id", "forward-absolute-filepath", "reverse-absolute-filepath"]
            .map(String::from)
            .to_vec();
        let rows = units
            .iter()
            .map(|u| vec![clean_name(&u.sample), fmt(u.r1.as_ref().unwrap()), fmt(u.r2.as_ref().unwrap())])
            .collect();
        Ok((header, rows))
    } else {
        let header = ["sample-id", "absolute-filepath"].map(String::from).to_vec();
        let rows = units
            .iter()
            .map(|u| {
                let p = u.r1.as_ref().or(u.long.as_ref()).unwrap();
                vec![clean_name(&u.sample), fmt(p)]
            })
            .collect();
        Ok((header, rows))
    }
}

fn render((header, rows): &Table, sep: char) -> Result<String> {
    let mut out = String::new();
    for row in std::iter::once(header).chain(rows) {
        for cell in row {
            if cell.contains(sep) || cell.contains('\n') || cell.contains('"') {
                bail!("value `{cell}` contains the column separator; rename the file or use the other format");
            }
        }
        out.push_str(&row.join(&sep.to_string()));
        out.push('\n');
    }
    Ok(out)
}

/// Relative path from `base` to `path` (both absolute).
fn pathdiff(path: &Path, base: &Path) -> PathBuf {
    let p: Vec<_> = path.components().collect();
    let b: Vec<_> = base.components().collect();
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &p[common..] {
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_names() {
        let c = |s| {
            let (a, l, m, _, _) = classify(s, true);
            (a, l, m)
        };
        assert_eq!(c("Ecoli_S3_L002_R1_001"), ("Ecoli".into(), "002".into(), Some(Mate::R1)));
        assert_eq!(c("Ecoli_S3_R2_001"), ("Ecoli".into(), "".into(), Some(Mate::R2)));
        assert_eq!(c("Ecoli_S3_L001_I1_001"), ("Ecoli".into(), "001".into(), None));
        assert_eq!(c("ERR044595_1"), ("ERR044595".into(), "".into(), Some(Mate::R1)));
        assert_eq!(c("ERR044595_1M_2"), ("ERR044595_1M".into(), "".into(), Some(Mate::R2)));
        assert_eq!(c("sample.R1"), ("sample".into(), "".into(), Some(Mate::R1)));
        assert_eq!(c("barcode07"), ("barcode07".into(), "".into(), Some(Mate::Unpaired)));
    }

    fn touch(root: &Path, names: &[&str]) {
        for n in names {
            let p = root.join(n);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "").unwrap();
        }
    }

    fn units_of(root: &Path, pair_numeric: bool) -> Result<Vec<Unit>> {
        let (files, _) = scan(&[root.to_path_buf()], false, pair_numeric)?;
        Ok(pair(files, false)?.0)
    }

    fn rel(root: &Path, p: &Option<PathBuf>) -> Option<String> {
        let root = root.canonicalize().unwrap();
        p.as_ref().map(|p| p.strip_prefix(&root).unwrap().display().to_string().replace('\\', "/"))
    }

    #[test]
    fn mates_pair_only_within_a_folder() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["runA/A_R1.fastq.gz", "runB/A_R1.fastq.gz", "runB/A_R2.fastq.gz"]);
        // runA's lone R1 must not be paired with runB's R2; the sample is then half single-end.
        let err = units_of(t.path(), true).unwrap_err().to_string();
        assert!(err.contains("both single-end and paired rows"), "{err}");
        touch(t.path(), &["runA/A_R2.fastq.gz"]);
        let u = units_of(t.path(), true).unwrap();
        let rows: Vec<_> = u.iter().map(|u| (rel(t.path(), &u.r1), rel(t.path(), &u.r2))).collect();
        assert_eq!(
            rows,
            vec![
                (Some("runA/A_R1.fastq.gz".into()), Some("runA/A_R2.fastq.gz".into())),
                (Some("runB/A_R1.fastq.gz".into()), Some("runB/A_R2.fastq.gz".into())),
            ]
        );
    }

    #[test]
    fn mates_in_different_folders_are_not_paired() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["input1/X_1.fastq.gz", "input2/X_2.fastq.gz"]);
        let err = units_of(t.path(), true).unwrap_err().to_string();
        assert!(err.contains("R2 files without an R1"), "{err}");
    }

    #[test]
    fn two_files_for_one_mate_are_an_error() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["A_R1.fq.gz", "A_R1.fastq.gz", "A_R2.fq.gz"]);
        let err = units_of(t.path(), true).unwrap_err().to_string();
        assert!(err.contains("several files claim the same mate"), "{err}");
    }

    #[test]
    fn names_colliding_after_cleaning_are_refused() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["s 1_R1.fastq.gz", "s 1_R2.fastq.gz", "s_1_R1.fastq.gz", "s_1_R2.fastq.gz"]);
        let u = units_of(t.path(), true).unwrap();
        let mut w = Vec::new();
        let err = check_names(&u, false, &mut w).unwrap_err().to_string();
        assert!(err.contains("`s 1`, `s_1` all become `s_1`"), "{err}");
        assert!(check_names(&u, true, &mut w).is_ok());
        assert_eq!(w.len(), 1);
    }

    #[test]
    fn bare_numeric_pairs_are_flagged_or_kept_apart() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["patient_1.fq.gz", "patient_2.fq.gz", "ctrl_R1.fq.gz", "ctrl_R2.fq.gz"]);
        let u = units_of(t.path(), true).unwrap();
        assert_eq!(u.len(), 2);
        let flagged: Vec<_> = u.iter().map(|u| (u.sample.as_str(), u.numeric_pair)).collect();
        assert_eq!(flagged, vec![("ctrl", false), ("patient", true)]);

        let u = units_of(t.path(), false).unwrap();
        let names: Vec<_> = u.iter().map(|u| (u.sample.as_str(), u.r2.is_some())).collect();
        assert_eq!(names, vec![("ctrl", true), ("patient_1", false), ("patient_2", false)]);
    }

    #[test]
    fn cleans_names() {
        assert_eq!(clean_name("my sample (2)"), "my_sample__2_");
        assert_eq!(clean_name("ok.name-1_x"), "ok.name-1_x");
    }

    #[test]
    fn relative_paths() {
        assert_eq!(pathdiff(Path::new("/a/b/c.fq"), Path::new("/a/x")), PathBuf::from("../b/c.fq"));
    }
}
