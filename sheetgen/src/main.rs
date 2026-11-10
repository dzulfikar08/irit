//! sheetgen: build a pipeline sample sheet from a folder of FASTQ files.
//!
//! Pairs R1/R2, keeps lanes as separate rows (nf-core pipelines merge rows that share a
//! sample name), and, given a pipeline's `schema_input.json`, writes the exact columns
//! that pipeline expects and checks every value against the schema before you launch.

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
}

/// One output row: one sample, one lane.
#[derive(Debug, Clone)]
pub struct Unit {
    pub sample: String,
    pub r1: Option<PathBuf>,
    pub r2: Option<PathBuf>,
    pub long: Option<PathBuf>,
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

    let (files, skipped) = scan(&cli.dirs, cli.name_from_dir)?;
    if files.is_empty() {
        bail!("no FASTQ files found under {}", show_dirs(&cli.dirs));
    }
    let (mut units, mut warnings) = pair(files, cli.long);
    for s in &skipped {
        warnings.push(format!("skipped index read file {s}"));
    }
    check_names(&units, &mut warnings);
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
        let schema = schema::Schema::load(path)?;
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
fn scan(dirs: &[PathBuf], name_from_dir: bool) -> Result<(Vec<Fastq>, Vec<String>)> {
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
            let (mut sample, lane, mate) = classify(stem);
            if mate.is_none() {
                skipped.push(path.display().to_string());
                continue;
            }
            if name_from_dir
                && let Some(parent) = path.parent().and_then(|p| p.file_name()) {
                    sample = parent.to_string_lossy().into_owned();
                }
            files.push(Fastq { path, sample, lane, mate: mate.unwrap() });
        }
    }
    Ok((files, skipped))
}

/// Split a file stem into (sample, lane, mate). `None` mate means an index read (I1/I2).
fn classify(stem: &str) -> (String, String, Option<Mate>) {
    let caps = ILLUMINA.captures(stem).or_else(|| GENERIC.captures(stem));
    let Some(c) = caps else {
        return (stem.to_string(), String::new(), Some(Mate::Unpaired));
    };
    let lane = c.name("lane").map(|m| m.as_str().to_string()).unwrap_or_default();
    let mate = match c["r"].to_ascii_uppercase().as_str() {
        "R1" | "1" => Some(Mate::R1),
        "R2" | "2" => Some(Mate::R2),
        _ => None, // I1 / I2
    };
    (c["s"].to_string(), lane, mate)
}

/// Group files into (sample, lane) units and pair mates.
fn pair(files: Vec<Fastq>, long: bool) -> (Vec<Unit>, Vec<String>) {
    let mut groups: BTreeMap<(String, String), Vec<Fastq>> = BTreeMap::new();
    for f in files {
        groups.entry((f.sample.clone(), f.lane.clone())).or_default().push(f);
    }
    let mut units = Vec::new();
    let mut warnings = Vec::new();
    for ((sample, lane), fs) in groups {
        let at = if lane.is_empty() { sample.clone() } else { format!("{sample} lane {lane}") };
        let pick = |m: Mate| -> Vec<&Fastq> { fs.iter().filter(|f| f.mate == m).collect() };
        let (r1, r2, un) = (pick(Mate::R1), pick(Mate::R2), pick(Mate::Unpaired));
        if r1.len() > 1 || r2.len() > 1 {
            let list: Vec<_> = fs.iter().map(|f| f.path.display().to_string()).collect();
            warnings.push(format!(
                "{at}: several files claim the same mate, each gets its own row: {}",
                list.join(", ")
            ));
        }
        let n = r1.len().max(r2.len());
        for i in 0..n {
            let a = r1.get(i).map(|f| f.path.clone());
            let b = r2.get(i).map(|f| f.path.clone());
            match (&a, &b) {
                (Some(_), None) => warnings.push(format!("{at}: R1 without R2, written as single-end")),
                (None, Some(p)) => warnings.push(format!(
                    "{at}: R2 without R1 ({}); check the file name",
                    p.display()
                )),
                _ => {}
            }
            units.push(Unit { sample: sample.clone(), r1: a, r2: b, long: None });
        }
        for f in un {
            if long {
                // Hybrid assembly: put long reads on the sample's short-read row if it has one.
                if let Some(u) = units
                    .iter_mut()
                    .rev()
                    .find(|u| u.sample == sample && u.long.is_none() && u.r1.is_some())
                {
                    u.long = Some(f.path.clone());
                    continue;
                }
                units.push(Unit { sample: sample.clone(), r1: None, r2: None, long: Some(f.path.clone()) });
            } else {
                units.push(Unit { sample: sample.clone(), r1: Some(f.path.clone()), r2: None, long: None });
            }
        }
    }
    (units, warnings)
}

/// Make names safe for pipelines (no spaces or shell-hostile characters) and catch collisions.
fn check_names(units: &[Unit], warnings: &mut Vec<String>) {
    let mut by_clean: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for u in units {
        by_clean.entry(clean_name(&u.sample)).or_default().insert(&u.sample);
    }
    for (clean, originals) in by_clean {
        if originals.len() > 1 {
            let list: Vec<_> = originals.into_iter().collect();
            warnings.push(format!(
                "samples {} all become `{clean}` after cleaning; rename the files",
                list.join(", ")
            ));
        }
    }
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
            let (a, l, m) = classify(s);
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
