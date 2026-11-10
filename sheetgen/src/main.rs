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

    /// List `Undetermined_S0_*` files (reads bcl2fastq couldn't assign to a sample) as a sample
    /// instead of skipping them.
    #[arg(long)]
    include_undetermined: bool,

    /// List Nanopore reads that failed MinKNOW's quality filter (`_fail_` in the name, files under
    /// a `fastq_fail/` folder, Guppy `fastq_runid_*` files under `fail/`) instead of skipping them.
    #[arg(long)]
    include_fail: bool,
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
    /// File name without the FASTQ extension.
    stem: String,
    /// File stem with the mate token blanked: R1 and R2 of one pair share it.
    pair_stem: String,
    /// The mate token as written (`R1`, `r2`, `1`), empty when unpaired.
    token: String,
    /// The sample name came from the folder (`--name-from-dir`), not the file name.
    named_by_dir: bool,
    /// Looks like Oxford Nanopore MinKNOW/Guppy output (`barcodeNN/` folder, `_pass_`/`_fail_`
    /// or `fastq_runid_` name).
    ont: bool,
    /// Unpaired by the numbered-sibling rule (`x_0`, `x_1`, `x_2` chunks), not by its name.
    demoted: bool,
}

impl Fastq {
    /// The mate came from a bare `_1`/`_2` or a lowercase `_r1`/`_r2`, which are also used for
    /// replicates and chunks, rather than from `R1`/`R2`.
    fn weak(&self) -> bool {
        is_weak(&self.token)
    }
}

fn is_weak(token: &str) -> bool {
    token.starts_with(|c: char| c.is_ascii_digit() || c == 'r')
}

/// How a file stem was read.
#[derive(Debug, Clone, PartialEq)]
struct Classified {
    sample: String,
    lane: String,
    /// `None` for index reads (I1/I2).
    mate: Option<Mate>,
    pair_stem: String,
    /// Mate token as written, empty when unpaired.
    token: String,
}

/// One output row: one sample, one lane.
#[derive(Debug, Clone)]
pub struct Unit {
    pub sample: String,
    pub r1: Option<PathBuf>,
    pub r2: Option<PathBuf>,
    pub long: Option<PathBuf>,
    /// Lane token (`001`, or `1`/`01` in BGI/MGI names); empty when there is none.
    pub lane: String,
    /// R1/R2 were matched only on a bare `_1`/`_2` or lowercase `_r1`/`_r2` suffix.
    pub numeric_pair: bool,
}

static FASTQ_EXT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\.f(ast)?q(\.gz|\.bz2|\.zst)?$").unwrap());
// bcl2fastq/BCL Convert: Sample_S1_L001_R1_001.fastq.gz
static ILLUMINA: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<s>.+?)_S\d+(?:_L(?P<lane>\d{3}))?_(?P<r>[RIri][12])_\d{3}$").unwrap()
});
// Novogene/BGI `S1_EKDL01_HGV_L1_1`, MGI `FP150002315_L01_1`: a 1-2 digit lane only directly
// before a bare numeric mate. Elsewhere `_L1` is part of the name (`strain_L1_R1`).
static BGI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?P<s>.+?)_L(?P<lane>\d{1,2})_(?P<r>[12])$").unwrap());
// sample_R1, sample_r1, sample_1, sample.R2, sample_L002_R1_001, sample_R1_001.
static GENERIC: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?P<s>.+?)(?:_L(?P<lane>\d{3}))?[._-](?P<r>[RIri]?[12])(?:_\d{3})?$").unwrap()
});
// MinKNOW: barcode01/FAX123_pass_barcode01_abc_0.fastq.gz; Guppy: pass/fastq_runid_abc_0_1.fastq.
// Chunk numbers are not mates.
static ONT_DIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(barcode\d+|unclassified)$").unwrap());
static ONT_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)(_(pass|fail)_|^fastq_runid_)").unwrap());
// Guppy's `pass/` and `fail/` folders: ONT only for files without a mate token in the name.
static GUPPY_DIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(pass|fail)$").unwrap());
static GUPPY_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^fastq_runid_").unwrap());
static CHUNK_NO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"_\d+$").unwrap());
// bcl2fastq/BCL Convert read 3/4 (10x ATAC, UMI reads): Sample_S1_L001_R3_001.
static ILLUMINA_R34: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?P<pre>.+?_S\d+(?:_L\d{3})?_)R(?P<n>[34])(?P<post>_\d{3})$").unwrap());
static ONT_FAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)_fail_").unwrap());
static UNDETERMINED: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^undetermined_S0_").unwrap());

fn main() -> Result<()> {
    let cli = Cli::parse();
    let fixed = parse_sets(&cli.set)?;

    let opts = ScanOpts {
        name_from_dir: cli.name_from_dir,
        // Numeric pairing stays on with --long: hybrid SRA/fetchngs downloads are `x_1`, `x_2`
        // (short) next to `x` (long). Nanopore chunks are never paired (see `scan`).
        pair_numeric: !cli.no_pair_numeric,
        include_undetermined: cli.include_undetermined,
        include_fail: cli.include_fail,
    };
    let (files, notes) = scan(&cli.dirs, &opts)?;
    if files.is_empty() {
        bail!("no FASTQ files found under {}", show_dirs(&cli.dirs));
    }
    let (mut units, mut warnings) = pair(files, &PairOpts { long: cli.long, no_pair_numeric: cli.no_pair_numeric })?;
    warnings.extend(notes);
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
        check_long(&units, schema.has_long_column(), schema.has_run_column(), &mut warnings)?;
        schema.build(&units, &fixed, &fmt_path, &mut warnings)?
    } else {
        check_long(&units, false, false, &mut warnings)?;
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
            "  note: paired only because of a bare _1/_2 or lowercase _r1/_r2 suffix: {} (SRA-style names; if these are separate single-end samples or replicates, rerun with --no-pair-numeric for _1/_2, or rename _r1/_r2 files)",
            list.join(", ")
        );
    }
    let short_lanes = merged_short_lanes(&units);
    if !short_lanes.is_empty() {
        eprintln!(
            "  note: lanes merged into one sample on a short _L<n> token (BGI/MGI style): {} (if these are different samples, rename the files)",
            short_lanes.join(", ")
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

/// Samples whose rows come from several 1-2 digit lanes (`x_L1_1`, `x_L2_1`), as `x (L1, L2)`.
fn merged_short_lanes(units: &[Unit]) -> Vec<String> {
    let mut lanes: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for u in units.iter().filter(|u| (1..=2).contains(&u.lane.len())) {
        lanes.entry(&u.sample).or_default().insert(&u.lane);
    }
    lanes
        .into_iter()
        .filter(|(_, l)| l.len() > 1)
        .map(|(s, l)| format!("{s} ({})", l.into_iter().map(|l| format!("L{l}")).collect::<Vec<_>>().join(", ")))
        .collect()
}

struct ScanOpts {
    name_from_dir: bool,
    pair_numeric: bool,
    include_undetermined: bool,
    include_fail: bool,
}

/// Walk the folders and classify every FASTQ file. Returns the files and notes about files
/// skipped (index reads, Undetermined, dangling symlinks).
fn scan(dirs: &[PathBuf], opts: &ScanOpts) -> Result<(Vec<Fastq>, Vec<String>)> {
    let mut files = Vec::new();
    let mut notes = Vec::new();
    let mut seen = BTreeSet::new();
    let mut failed = 0usize;
    for dir in dirs {
        let dir = dir
            .canonicalize()
            .with_context(|| format!("cannot open {}", dir.display()))?;
        for entry in walkdir::WalkDir::new(&dir).follow_links(true).sort_by_file_name() {
            let entry = match entry {
                Ok(e) => e,
                // A dangling symlink or an unreadable subfolder: skip it, don't drop the run.
                Err(e) if e.depth() > 0 => {
                    let at = e.path().map(|p| p.display().to_string()).unwrap_or_default();
                    notes.push(format!("skipped {at}: {}", e.io_error().map(|io| io.to_string()).unwrap_or_else(|| e.to_string())));
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
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
            if !opts.include_undetermined && UNDETERMINED.is_match(stem) {
                notes.push(format!(
                    "skipped {} (reads not assigned to any sample; --include-undetermined lists them)",
                    path.display()
                ));
                continue;
            }
            // MinKNOW's failed reads: `_fail_` in the name, or a `fastq_fail/` folder below the
            // scanned one (pointing sheetgen at fastq_fail/ itself is taken as meant).
            let in_fail_dir = path
                .strip_prefix(&dir)
                .ok()
                .and_then(Path::parent)
                .is_some_and(|rel| {
                    rel.components().any(|c| {
                        let c = c.as_os_str();
                        // Guppy's `fail/` only for Guppy-named files; `fail` alone is too common.
                        c.eq_ignore_ascii_case("fastq_fail") || (c.eq_ignore_ascii_case("fail") && GUPPY_NAME.is_match(stem))
                    })
                });
            if !opts.include_fail && (ONT_FAIL.is_match(stem) || in_fail_dir) {
                failed += 1;
                continue;
            }
            let parent = path.parent().and_then(|p| p.file_name()).map(|p| p.to_string_lossy().into_owned());
            let ont = ONT_NAME.is_match(stem)
                || parent.as_deref().is_some_and(|p| {
                    ONT_DIR.is_match(p) || (GUPPY_DIR.is_match(p) && classify(stem, true).mate == Some(Mate::Unpaired))
                });
            let c = if ont {
                // Never pair Nanopore chunks: `_0`, `_1`, ... are chunk numbers, not mates. The
                // chunk number is dropped from the sample name, so chunks of a run stay one sample.
                let mut c = unpaired(stem);
                c.sample = CHUNK_NO.replace(stem, "").into_owned();
                c
            } else {
                classify(stem, opts.pair_numeric)
            };
            let Some(mate) = c.mate else {
                notes.push(format!("skipped index read file {}", path.display()));
                continue;
            };
            let (sample, named_by_dir) = match parent {
                Some(p) if opts.name_from_dir => (p, true),
                _ => (c.sample, false),
            };
            files.push(Fastq {
                path,
                sample,
                lane: c.lane,
                mate,
                stem: stem.to_string(),
                pair_stem: c.pair_stem,
                token: c.token,
                named_by_dir,
                ont,
                demoted: false,
            });
        }
    }
    if failed > 0 {
        notes.push(format!(
            "skipped {failed} Nanopore files that failed MinKNOW's quality filter (`_fail_` in the name, under fastq_fail/, or Guppy files under fail/); --include-fail lists them"
        ));
    }
    Ok((files, notes))
}

fn unpaired(stem: &str) -> Classified {
    Classified {
        sample: stem.to_string(),
        lane: String::new(),
        mate: Some(Mate::Unpaired),
        pair_stem: stem.to_string(),
        token: String::new(),
    }
}

/// Split a file stem into sample, lane, mate (`None` for an index read I1/I2), the stem with
/// the mate token blanked (shared by the two files of a pair) and the mate token. Without
/// `pair_numeric`, bare `_1`/`_2` are part of the sample name.
fn classify(stem: &str, pair_numeric: bool) -> Classified {
    let caps = ILLUMINA.captures(stem).or_else(|| BGI.captures(stem)).or_else(|| GENERIC.captures(stem));
    let Some(c) = caps else { return unpaired(stem) };
    let r = c.name("r").unwrap();
    let numeric = !r.as_str().starts_with(['R', 'r', 'I', 'i']);
    if numeric && !pair_numeric {
        return unpaired(stem);
    }
    let mate = match r.as_str().to_ascii_uppercase().as_str() {
        "R1" | "1" => Some(Mate::R1),
        "R2" | "2" => Some(Mate::R2),
        _ => None, // I1 / I2
    };
    Classified {
        sample: c["s"].to_string(),
        lane: c.name("lane").map(|m| m.as_str().to_string()).unwrap_or_default(),
        mate,
        pair_stem: format!("{}\u{0}{}", &stem[..r.start()], &stem[r.end()..]),
        token: r.as_str().to_string(),
    }
}

/// Files next to a weak pair (`x_1`/`x_2`, `x_r1`/`x_r2`) numbered beyond 2 or 0 (`x_3`,
/// `x_r3`, `x_0`), with their numbers: the numbers then count replicates, chunks or extra reads,
/// not mates.
fn numbered_siblings<'a>(pair_stem: &str, token: &str, stems: &'a BTreeSet<String>) -> Vec<(&'a str, &'a str)> {
    let Some((pre, post)) = pair_stem.split_once('\u{0}') else { return Vec::new() };
    let head = format!("{pre}{}", &token[..token.len() - 1]);
    stems
        .iter()
        .filter_map(|s| {
            let n = s.strip_prefix(head.as_str())?.strip_suffix(post)?;
            (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) && n != "1" && n != "2").then_some((s.as_str(), n))
        })
        .collect()
}

/// Longest of the first few reads of a FASTQ file (plain or gzip), or `None` if the file can't
/// be read or has no complete record.
fn first_reads_max_len(path: &Path) -> Option<usize> {
    use std::io::{BufRead, BufReader, Read};
    const RECORDS: usize = 10;
    let file = std::fs::File::open(path).ok()?;
    let name = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    let reader: Box<dyn Read> = if name.ends_with(".gz") {
        Box::new(flate2::read::MultiGzDecoder::new(file))
    } else if name.ends_with(".fastq") || name.ends_with(".fq") {
        Box::new(file)
    } else {
        return None; // .bz2/.zst: not sniffed
    };
    let mut lines = BufReader::new(reader).lines();
    let mut max = None;
    for _ in 0..RECORDS {
        let Some(Ok(head)) = lines.next() else { break };
        let (Some(Ok(seq)), Some(Ok(_)), Some(Ok(_))) = (lines.next(), lines.next(), lines.next()) else { break };
        if !head.starts_with('@') {
            return None;
        }
        max = Some(max.unwrap_or(0).max(seq.trim_end().len()));
    }
    max
}

/// Short-read evidence for a weak `_1`/`_2` pair under `--long`: a bare-named long-read file
/// beside it (`x` next to `x_1`/`x_2`, SRA hybrid layout), or first reads all under 1,000 bp.
fn short_pair_evidence(fs: &[Fastq], long_stems: &BTreeSet<String>) -> bool {
    const SHORT_MAX: usize = 1000;
    let bare = fs[0].pair_stem.split_once('\u{0}').map(|(pre, post)| {
        format!("{}{post}", pre.trim_end_matches(['_', '.', '-']))
    });
    if bare.is_some_and(|b| long_stems.contains(&b)) {
        return true;
    }
    fs.iter().all(|f| first_reads_max_len(&f.path).is_some_and(|n| n < SHORT_MAX))
}

/// The other mate's file stem for a weak mate token (`x_1` → `x_2`), if `stem` has one.
fn weak_counterpart(stem: &str) -> Option<String> {
    let c = classify(stem, true);
    if !is_weak(&c.token) {
        return None;
    }
    let other = match c.mate? {
        Mate::R1 => '2',
        Mate::R2 => '1',
        Mate::Unpaired => return None,
    };
    let (pre, post) = c.pair_stem.split_once('\u{0}')?;
    Some(format!("{pre}{}{other}{post}", &c.token[..c.token.len() - 1]))
}

struct PairOpts {
    long: bool,
    no_pair_numeric: bool,
}

/// Pair mates and make one unit per pair or unpaired file.
///
/// Two files are mates only if they sit in the same folder and their names differ only in the
/// mate token (`A_R1` / `A_R2`). Several pairs for one sample (lanes, runs in other folders)
/// become several rows, which nf-core pipelines merge. Two files claiming the same mate of the
/// same pair (`A_R1.fq.gz` next to `A_R1.fastq.gz`) are an error.
fn pair(files: Vec<Fastq>, opts: &PairOpts) -> Result<(Vec<Unit>, Vec<String>)> {
    let long = opts.long;
    type Key = (String, String, PathBuf, String);
    let dir_of = |f: &Fastq| f.path.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut stems: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    // Stems of files without any mate token: candidate long-read files.
    let mut bare_stems: BTreeMap<PathBuf, BTreeSet<String>> = BTreeMap::new();
    for f in &files {
        stems.entry(dir_of(f)).or_default().insert(f.stem.clone());
        if f.mate == Mate::Unpaired {
            bare_stems.entry(dir_of(f)).or_default().insert(f.stem.clone());
        }
    }
    // 10x / UMI runs: R3 (and R4) next to R1/R2. Which read is the cDNA, barcode or UMI differs
    // between kits, so refuse rather than list R3 as a sample of its own.
    let mut extra_reads = Vec::new();
    for f in files.iter().filter(|f| f.mate == Mate::Unpaired && !f.ont) {
        let Some(c) = ILLUMINA_R34.captures(&f.stem) else { continue };
        let r1 = format!("{}R1{}", &c["pre"], &c["post"]);
        if stems[&dir_of(f)].contains(&r1) {
            extra_reads.push(format!("{} (next to {r1})", f.path.display()));
        }
    }
    if !extra_reads.is_empty() {
        bail!(
            "R3/R4 read files next to R1/R2 (10x Genomics or UMI layout):\n  {}\n\
             in these runs R1/R2/R3 hold barcodes, UMIs or cDNA depending on the kit, so a plain R1/R2 sheet would be wrong; \
             use the pipeline's own input for this layout (e.g. nf-core/scrnaseq, or a UMI-aware pipeline), \
             or move the R3/R4 files out of the folder if you know R1/R2 are the reads to list",
            extra_reads.join("\n  ")
        );
    }
    let mut groups: BTreeMap<Key, Vec<Fastq>> = BTreeMap::new();
    let mut unpaired = Vec::new();
    for f in files {
        if f.mate == Mate::Unpaired {
            unpaired.push(f);
            continue;
        }
        groups.entry((f.sample.clone(), f.lane.clone(), dir_of(&f), f.pair_stem.clone())).or_default().push(f);
    }
    let mut units = Vec::new();
    let mut warnings = Vec::new();
    // `x_0`, `x_1`, `x_2` are chunks, not mates: keep them apart. `s_1`, `s_2` next to `s_3`
    // (replicates, or fasterq-dump's `_1`..`_4` for 10x runs) is ambiguous: refuse.
    let mut kept = BTreeMap::new();
    let mut ambiguous = Vec::new();
    let mut ambiguous_r = false;
    let mut weak_long = Vec::new();
    for (key, fs) in groups {
        let weak = fs.iter().all(Fastq::weak);
        let siblings = if weak { numbered_siblings(&key.3, &fs[0].token, &stems[&key.2]) } else { Vec::new() };
        let names: Vec<_> = fs.iter().map(|f| f.stem.as_str()).collect();
        if siblings.is_empty() {
            // Under --long a bare `_1`/`_2` may just as well number long-read files: pair only
            // with evidence that they are short reads.
            let both = fs.iter().any(|f| f.mate == Mate::R1) && fs.iter().any(|f| f.mate == Mate::R2);
            if long && weak && both && !short_pair_evidence(&fs, bare_stems.get(&key.2).unwrap_or(&BTreeSet::new())) {
                weak_long.push(fs.iter().map(|f| f.path.display().to_string()).collect::<Vec<_>>().join(", "));
            }
            kept.insert(key, fs);
            continue;
        }
        let sib: Vec<_> = siblings.iter().map(|(s, _)| *s).collect();
        if !siblings.iter().any(|(_, n)| n.bytes().all(|b| b == b'0')) {
            ambiguous_r |= fs.iter().any(|f| f.token.starts_with('r'));
            ambiguous.push(format!("{} (next to {})", names.join(", "), sib.join(", ")));
            continue;
        }
        warnings.push(format!(
            "not paired: {} ({} next to them: numbered from 0, so chunks, not mates; name mates _R1/_R2 to pair them)",
            names.join(", "),
            sib.join(", ")
        ));
        for mut f in fs {
            if !f.named_by_dir {
                f.sample = f.stem.clone();
            }
            f.lane.clear();
            f.mate = Mate::Unpaired;
            f.demoted = true;
            unpaired.push(f);
        }
    }
    if !ambiguous.is_empty() {
        let fix = if ambiguous_r {
            "rename the files: mates _R1/_R2, replicates e.g. _rep1, _rep2 (lowercase _r1/_r2 always pair)"
        } else {
            "pass --no-pair-numeric to list every file as a single-end sample, or rename mates _R1/_R2"
        };
        bail!(
            "these files may be mates, but more files are numbered the same way, so the numbers may count replicates or extra reads \
             (fasterq-dump writes _1.._4 for 10x runs) rather than mates:\n  {}\n{fix}",
            ambiguous.join("\n  ")
        );
    }
    if !weak_long.is_empty() {
        bail!(
            "with --long, these files are told apart only by a bare _1/_2 or lowercase _r1/_r2, which could be short-read mates or \
             numbered long-read files, and nothing shows which (no long-read file named without the suffix beside them, \
             and their first reads are not all under 1,000 bp):\n  {}\n\
             rename short-read mates _R1/_R2; if they are separate long-read files, pass --no-pair-numeric (bare _1/_2) \
             or rename them (lowercase _r1/_r2 always pair)",
            weak_long.join("\n  ")
        );
    }
    let groups = kept;
    let ont = unpaired.iter().filter(|f| f.ont).count();
    if ont > 0 && !long {
        warnings.push(format!(
            "{ont} files look like Oxford Nanopore (MinKNOW) output and were written as single-end short reads; \
             pass --long (and usually --name-from-dir) for long reads"
        ));
    }
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
        let numeric_pair = a.is_some() && b.is_some() && fs.iter().all(Fastq::weak);
        units.push(Unit { sample, r1: a, r2: b, long: None, lane, numeric_pair });
    }
    if !orphans.is_empty() {
        bail!(
            "R2 files without an R1 next to them (mates are paired only within one folder, by names that differ only in R1/R2 or _1/_2):\n  {}",
            orphans.join("\n  ")
        );
    }
    unpaired.sort_by(|a, b| a.path.cmp(&b.path));
    let mut mate_like = Vec::new();
    for f in unpaired {
        if long {
            // Hybrid assembly: put long reads on the sample's short-read row if it has one.
            // Further long files of the sample get rows of their own (see `check_long`).
            if let Some(u) = units
                .iter_mut()
                .rev()
                .find(|u| u.sample == f.sample && u.long.is_none() && u.r1.is_some())
            {
                u.long = Some(f.path.clone());
                continue;
            }
            // A long-only `x_1` next to `x_2` is almost certainly a short-read mate (chunks the
            // sibling rule unpaired were already judged).
            if !f.ont
                && !f.demoted
                && let Some(other) = weak_counterpart(&f.stem)
                && stems[&dir_of(&f)].contains(&other)
            {
                mate_like.push(format!("{} (next to {other})", f.path.display()));
            }
            units.push(Unit { sample: f.sample, r1: None, r2: None, long: Some(f.path), lane: String::new(), numeric_pair: false });
        } else {
            units.push(Unit { sample: f.sample, r1: Some(f.path), r2: None, long: None, lane: String::new(), numeric_pair: false });
        }
    }
    if !mate_like.is_empty() {
        let list = mate_like.join("\n  ");
        if opts.no_pair_numeric {
            warnings.push(format!("listed as long reads although the names look like short-read mates (--no-pair-numeric):\n  {list}"));
        } else {
            bail!(
                "these files would become long-read-only rows, but their names look like short-read mates:\n  {list}\n\
                 name short-read mates _R1/_R2, or pass --no-pair-numeric if they really are separate long-read samples"
            );
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

/// Long reads must land in a long-read column, and one sample's several long-read files only
/// make sense where the sheet has a run column to tell the rows apart.
fn check_long(units: &[Unit], long_column: bool, run_column: bool, warnings: &mut Vec<String>) -> Result<()> {
    let mut per_sample: BTreeMap<&str, Vec<String>> = BTreeMap::new();
    for u in units {
        if let Some(p) = &u.long {
            per_sample.entry(&u.sample).or_default().push(p.display().to_string());
        }
    }
    if per_sample.is_empty() {
        return Ok(());
    }
    if !long_column {
        if let Some(u) = units.iter().find(|u| u.long.is_some() && u.r1.is_some()) {
            bail!(
                "sample `{}` has short reads and a long-read file ({}), but this sheet has no long-read column; \
                 hybrid samples need a schema with a long-read column (e.g. --pipeline nf-core/bacass)",
                u.sample,
                u.long.as_ref().unwrap().display()
            );
        }
        warnings.push("this sheet has no long-read column; long-read-only rows have their reads in the R1 column".into());
    }
    let several: Vec<_> = per_sample.into_iter().filter(|(_, v)| v.len() > 1).collect();
    if several.is_empty() {
        return Ok(());
    }
    let list: Vec<String> = several.iter().map(|(s, v)| format!("{s}: {} files ({})", v.len(), v.join(", "))).collect();
    if run_column {
        warnings.push(format!("several long-read files per sample, one row each: {}", list.join("; ")));
        return Ok(());
    }
    bail!(
        "these samples have several long-read files, but the sheet takes one long-read file per sample \
         (nf-core/bacass, for one, takes a single LongFastQ per row):\n  {}\n\
         concatenate each sample's files first, e.g. `cat barcode01/*.fastq.gz > barcode01.fastq.gz` \
         (gzip files can be concatenated as they are), then point sheetgen at the merged files",
        list.join("\n  ")
    )
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
            let c = classify(s, true);
            (c.sample, c.lane, c.mate)
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

    fn opts(pair_numeric: bool) -> ScanOpts {
        ScanOpts { name_from_dir: false, pair_numeric, include_undetermined: false, include_fail: false }
    }

    const SHORT: PairOpts = PairOpts { long: false, no_pair_numeric: false };
    const LONG: PairOpts = PairOpts { long: true, no_pair_numeric: false };

    fn units_of(root: &Path, pair_numeric: bool) -> Result<Vec<Unit>> {
        let (files, _) = scan(&[root.to_path_buf()], &opts(pair_numeric))?;
        Ok(pair(files, &PairOpts { long: false, no_pair_numeric: !pair_numeric })?.0)
    }

    /// Scan and pair the way `main` does for `--long [--name-from-dir]`.
    fn long_units(root: &Path, name_from_dir: bool) -> Result<(Vec<Unit>, Vec<String>)> {
        let o = ScanOpts { name_from_dir, ..opts(true) };
        let (files, _) = scan(&[root.to_path_buf()], &o)?;
        pair(files, &LONG)
    }

    fn file_name(p: &Option<PathBuf>) -> Option<String> {
        p.as_ref().map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
    }

    #[test]
    fn hybrid_sra_and_fetchngs_names_pair_under_long() {
        for (short, long) in [
            (["SRR1_1.fastq.gz", "SRR1_2.fastq.gz"], "SRR1.fastq.gz"),
            (["SRX1_SRR1_1.fastq.gz", "SRX1_SRR1_2.fastq.gz"], "SRX1_SRR1.fastq.gz"),
        ] {
            let t = tempfile::tempdir().unwrap();
            touch(t.path(), &[short[0], short[1], long]);
            let (u, _) = long_units(t.path(), false).unwrap();
            assert_eq!(u.len(), 1, "{u:?}");
            assert_eq!(u[0].sample, long.trim_end_matches(".fastq.gz"));
            assert_eq!(file_name(&u[0].r1).as_deref(), Some(short[0]));
            assert_eq!(file_name(&u[0].r2).as_deref(), Some(short[1]));
            assert_eq!(file_name(&u[0].long).as_deref(), Some(long));
            // bacass: one row with R1, R2 and LongFastQ.
            let s = bacass();
            let fmt = |p: &Path| p.display().to_string();
            let (_, rows) = s.build(&u, &BTreeMap::new(), &fmt, &mut Vec::new()).unwrap();
            assert_eq!(rows.len(), 1);
            assert!(rows[0][3].ends_with(long), "{:?}", rows[0]);
        }
    }

    #[test]
    fn long_only_rows_that_look_like_mates_are_refused() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["SRR1_1.fastq.gz", "SRR1_2.fastq.gz"]);
        let (files, _) = scan(&[t.path().to_path_buf()], &opts(false)).unwrap();
        // --no-pair-numeric --long: allowed (asked for), with a warning.
        let (u, w) = pair(files, &PairOpts { long: true, no_pair_numeric: true }).unwrap();
        assert_eq!(u.len(), 2);
        assert!(w.iter().any(|w| w.contains("look like short-read mates")), "{w:?}");
        // Replicate-looking names: refused by the numbered-sibling rule, with or without --long.
        touch(t.path(), &["rep/x_r1.fastq.gz", "rep/x_r2.fastq.gz", "rep/x_r3.fastq.gz"]);
        let err = long_units(&t.path().join("rep"), false).unwrap_err().to_string();
        assert!(err.contains("more files are numbered the same way") && err.contains("rename"), "{err}");
        // Chunks (numbered from 0) are unpaired by the sibling rule and not refused as mates.
        touch(t.path(), &["chunks/x_0.fq.gz", "chunks/x_1.fq.gz", "chunks/x_2.fq.gz"]);
        let (u, w) = long_units(&t.path().join("chunks"), false).unwrap();
        assert_eq!(u.len(), 3);
        assert!(u.iter().all(|u| u.long.is_some() && u.r1.is_none()), "{u:?}");
        assert!(w.iter().any(|w| w.contains("not paired")) && !w.iter().any(|w| w.contains("short-read mates")), "{w:?}");
    }

    fn write_gz_fastq(path: &Path, reads: usize, len: usize) {
        use std::io::Write;
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut gz = flate2::write::GzEncoder::new(std::fs::File::create(path).unwrap(), flate2::Compression::fast());
        for i in 0..reads {
            writeln!(gz, "@r{i}\n{}\n+\n{}", "A".repeat(len), "I".repeat(len)).unwrap();
        }
        gz.finish().unwrap();
    }

    #[test]
    fn long_mode_pairs_bare_numeric_mates_only_with_evidence() {
        // No evidence (empty files, no bare-named long file): refused, not paired.
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["reads_1.fastq.gz", "reads_2.fastq.gz"]);
        let err = long_units(t.path(), false).unwrap_err().to_string();
        assert!(err.contains("could be short-read mates or numbered long-read files") && err.contains("--no-pair-numeric"), "{err}");
        // Long reads named _1/_2: refused too.
        let t = tempfile::tempdir().unwrap();
        write_gz_fastq(&t.path().join("reads_1.fastq.gz"), 10, 5000);
        write_gz_fastq(&t.path().join("reads_2.fastq.gz"), 10, 5000);
        assert!(long_units(t.path(), false).is_err());
        // --no-pair-numeric lists them as two long-read files.
        let (files, _) = scan(&[t.path().to_path_buf()], &opts(false)).unwrap();
        let (u, _) = pair(files, &PairOpts { long: true, no_pair_numeric: true }).unwrap();
        assert_eq!(u.len(), 2);
        assert!(u.iter().all(|u| u.long.is_some() && u.r1.is_none()));
        // Short reads (sniffed, gzip): paired.
        let t = tempfile::tempdir().unwrap();
        write_gz_fastq(&t.path().join("reads_1.fastq.gz"), 20, 150);
        write_gz_fastq(&t.path().join("reads_2.fastq.gz"), 20, 150);
        let (u, _) = long_units(t.path(), false).unwrap();
        assert_eq!(u.len(), 1);
        assert!(u[0].r1.is_some() && u[0].r2.is_some() && u[0].long.is_none(), "{u:?}");
        // Lowercase _r1/_r2 follow the same rule.
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["x_r1.fastq.gz", "x_r2.fastq.gz"]);
        assert!(long_units(t.path(), false).is_err());
        // Uppercase R1/R2 need no evidence.
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["x_R1.fastq.gz", "x_R2.fastq.gz"]);
        assert_eq!(long_units(t.path(), false).unwrap().0.len(), 1);
    }

    #[test]
    fn sniffs_first_reads() {
        let t = tempfile::tempdir().unwrap();
        write_gz_fastq(&t.path().join("a.fq.gz"), 3, 120);
        assert_eq!(first_reads_max_len(&t.path().join("a.fq.gz")), Some(120));
        std::fs::write(t.path().join("b.fastq"), "@r\nACGT\n+\nIIII\n").unwrap();
        assert_eq!(first_reads_max_len(&t.path().join("b.fastq")), Some(4));
        std::fs::write(t.path().join("c.fastq"), "").unwrap();
        assert_eq!(first_reads_max_len(&t.path().join("c.fastq")), None);
        std::fs::write(t.path().join("d.fastq.gz"), "").unwrap();
        assert_eq!(first_reads_max_len(&t.path().join("d.fastq.gz")), None);
    }

    #[test]
    fn guppy_pass_folder_chunks_are_nanopore() {
        let t = tempfile::tempdir().unwrap();
        touch(
            t.path(),
            &[
                "pass/fastq_runid_abc_0_0.fastq",
                "pass/fastq_runid_abc_0_1.fastq",
                "pass/fastq_runid_abc_0_2.fastq",
                "pass/fastq_runid_abc_0_3.fastq",
                "fail/fastq_runid_abc_0_0.fastq",
            ],
        );
        let (files, notes) = scan(&[t.path().to_path_buf()], &opts(true)).unwrap();
        assert_eq!(files.len(), 4, "fail/ skipped");
        assert!(notes.iter().any(|n| n.contains("skipped 1 Nanopore")), "{notes:?}");
        assert!(files.iter().all(|f| f.ont && f.sample == "fastq_runid_abc_0"));
        let (u, _) = long_units(t.path(), false).unwrap();
        assert_eq!(u.len(), 4);
        let err = check_long(&u, true, false, &mut Vec::new()).unwrap_err().to_string();
        assert!(err.contains("fastq_runid_abc_0: 4 files") && err.contains("concatenate"), "{err}");
        // Short-read mates in a folder named `pass` still pair.
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["pass/A_R1.fastq.gz", "pass/A_R2.fastq.gz", "fail/B_R1.fastq.gz", "fail/B_R2.fastq.gz"]);
        let u = units_of(t.path(), true).unwrap();
        assert_eq!(u.len(), 2);
        assert!(u.iter().all(|u| u.r2.is_some()));
    }

    #[test]
    fn ambiguous_numbered_siblings_are_refused() {
        for names in [&["s_1.fastq.gz", "s_2.fastq.gz", "s_3.fastq.gz"][..], &["SRR5_1.fastq.gz", "SRR5_2.fastq.gz", "SRR5_3.fastq.gz", "SRR5_4.fastq.gz"]] {
            let t = tempfile::tempdir().unwrap();
            touch(t.path(), names);
            let err = units_of(t.path(), true).unwrap_err().to_string();
            assert!(err.contains("more files are numbered the same way") && err.contains("--no-pair-numeric"), "{err}");
            assert!(long_units(t.path(), false).is_err());
            // --no-pair-numeric: every file a single-end sample.
            let u = units_of(t.path(), false).unwrap();
            assert_eq!(u.len(), names.len());
            assert!(u.iter().all(|u| u.r2.is_none()));
        }
    }

    #[test]
    fn tenx_read3_next_to_r1_r2_is_refused() {
        let t = tempfile::tempdir().unwrap();
        touch(
            t.path(),
            &["x_S1_L001_R1_001.fastq.gz", "x_S1_L001_R2_001.fastq.gz", "x_S1_L001_R3_001.fastq.gz", "x_S1_L001_I1_001.fastq.gz"],
        );
        let err = units_of(t.path(), true).unwrap_err().to_string();
        assert!(err.contains("10x Genomics or UMI") && err.contains("x_S1_L001_R3_001"), "{err}");
        // An R3 without R1/R2 beside it is just a file name.
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["y_S1_L001_R3_001.fastq.gz"]);
        assert_eq!(units_of(t.path(), true).unwrap().len(), 1);
    }

    #[test]
    fn short_lane_tokens_need_the_bgi_shape() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["strain_L1_R1.fastq.gz", "strain_L1_R2.fastq.gz", "strain_L2_R1.fastq.gz", "strain_L2_R2.fastq.gz"]);
        let u = units_of(t.path(), true).unwrap();
        let names: Vec<_> = u.iter().map(|u| u.sample.as_str()).collect();
        assert_eq!(names, ["strain_L1", "strain_L2"]);
        assert!(merged_short_lanes(&u).is_empty());
        // The BGI shape still merges, and says so.
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["S1_EKDL01_HGV_L1_1.fq.gz", "S1_EKDL01_HGV_L1_2.fq.gz", "S1_EKDL01_HGV_L2_1.fq.gz", "S1_EKDL01_HGV_L2_2.fq.gz"]);
        let u = units_of(t.path(), true).unwrap();
        assert_eq!(merged_short_lanes(&u), ["S1_EKDL01_HGV (L1, L2)"]);
        // Illumina lanes always have three digits.
        assert_eq!(classify("x_S1_L1_R1_001", true).sample, "x_S1_L1");
    }

    #[test]
    fn numbered_replicates_are_not_paired() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["x_0.fq.gz", "x_1.fq.gz", "x_2.fq.gz", "y_r0.fq.gz", "y_r1.fq.gz", "y_r2.fq.gz"]);
        let (files, _) = scan(&[t.path().to_path_buf()], &opts(true)).unwrap();
        let (u, w) = pair(files, &SHORT).unwrap();
        let names: Vec<_> = u.iter().map(|u| (u.sample.as_str(), u.r2.is_some())).collect();
        assert_eq!(
            names,
            [("x_0", false), ("x_1", false), ("x_2", false), ("y_r0", false), ("y_r1", false), ("y_r2", false)]
        );
        assert_eq!(w.iter().filter(|w| w.contains("not paired")).count(), 2, "{w:?}");
        // Replicates numbered from 1 (ctrl_r3 beside ctrl_r1/ctrl_r2): ambiguous, refused.
        let t2 = tempfile::tempdir().unwrap();
        touch(t2.path(), &["ctrl_r1.fastq.gz", "ctrl_r2.fastq.gz", "ctrl_r3.fastq.gz"]);
        let err = units_of(t2.path(), true).unwrap_err().to_string();
        assert!(err.contains("ctrl_r1, ctrl_r2 (next to ctrl_r3)") && err.contains("_rep1"), "{err}");
        // Two lowercase mates alone pair, flagged like bare _1/_2.
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["ctrl_r1.fastq.gz", "ctrl_r2.fastq.gz", "B_R1.fq.gz", "B_R2.fq.gz", "B_R3.fq.gz"]);
        let u = units_of(t.path(), true).unwrap();
        let flagged: Vec<_> = u.iter().map(|u| (u.sample.as_str(), u.numeric_pair)).collect();
        assert_eq!(flagged, [("B", false), ("B_R3", false), ("ctrl", true)]);
    }

    #[test]
    fn nanopore_fail_reads_are_skipped_unless_asked() {
        let t = tempfile::tempdir().unwrap();
        touch(
            t.path(),
            &[
                "run/fastq_pass/barcode01/FAX1_pass_barcode01_abc_0.fastq.gz",
                "run/fastq_fail/barcode01/FAX1_fail_barcode01_abc_0.fastq.gz",
                "run/fastq_fail/barcode01/other_0.fastq.gz",
                "run/FAX1_fail_barcode01_abc_1.fastq.gz",
            ],
        );
        let o = ScanOpts { name_from_dir: true, ..opts(true) };
        let (files, notes) = scan(&[t.path().to_path_buf()], &o).unwrap();
        assert_eq!(files.len(), 1);
        assert!(notes.iter().any(|n| n.contains("skipped 3 Nanopore") && n.contains("--include-fail")), "{notes:?}");
        let o = ScanOpts { include_fail: true, ..o };
        assert_eq!(scan(&[t.path().to_path_buf()], &o).unwrap().0.len(), 4);
        // Pointing at fastq_fail/ itself is taken as meant (only the `_fail_` name is skipped).
        let (files, _) = scan(&[t.path().join("run/fastq_fail")], &opts(true)).unwrap();
        assert_eq!(files.len(), 1);
    }

    fn bacass() -> schema::Schema {
        let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bacass-2.7.0.schema_input.json");
        schema::Schema::load(&p, false).unwrap()
    }

    #[test]
    fn hybrid_long_reads_need_a_long_read_column() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["hyb/A_R1.fastq.gz", "hyb/A_R2.fastq.gz", "hyb/A.fastq.gz"]);
        let (u, _) = long_units(t.path(), false).unwrap();
        assert_eq!(u.len(), 1);
        assert!(u[0].r1.is_some() && u[0].r2.is_some() && u[0].long.is_some());
        // Default nf-core table: no long-read column, so refuse instead of dropping the file.
        let err = check_long(&u, false, false, &mut Vec::new()).unwrap_err().to_string();
        assert!(err.contains("no long-read column") && err.contains("A.fastq.gz"), "{err}");
        // A schema without a long-read column refuses it too.
        let rnaseq = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rnaseq-3.27.0.schema_input.json");
        let rnaseq = schema::Schema::load(&rnaseq, false).unwrap();
        assert!(!rnaseq.has_long_column());
        let fmt = |p: &Path| p.display().to_string();
        let err = rnaseq.build(&u, &BTreeMap::new(), &fmt, &mut Vec::new()).unwrap_err().to_string();
        assert!(err.contains("no long-read column"), "{err}");
        // bacass has LongFastQ: all three files land in the row.
        let s = bacass();
        check_long(&u, s.has_long_column(), s.has_run_column(), &mut Vec::new()).unwrap();
        let (header, rows) = s.build(&u, &BTreeMap::new(), &fmt, &mut Vec::new()).unwrap();
        assert_eq!(header[3], "LongFastQ");
        assert!(rows[0][1].ends_with("A_R1.fastq.gz") && rows[0][2].ends_with("A_R2.fastq.gz"));
        assert!(Path::new(&rows[0][3]).ends_with(Path::new("hyb").join("A.fastq.gz")), "{:?}", rows[0]);
    }

    #[test]
    fn long_only_rows_without_long_column_warn() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["B.fastq.gz"]);
        let (u, _) = long_units(t.path(), false).unwrap();
        let mut w = Vec::new();
        check_long(&u, false, false, &mut w).unwrap();
        assert!(w.iter().any(|w| w.contains("long-read-only rows")), "{w:?}");
    }

    const ONT: [&str; 4] = [
        "ont/barcode01/FAX123_pass_barcode01_abc_0.fastq.gz",
        "ont/barcode01/FAX123_pass_barcode01_abc_1.fastq.gz",
        "ont/barcode01/FAX123_pass_barcode01_abc_2.fastq.gz",
        "ont/barcode01/FAX123_pass_barcode01_abc_3.fastq.gz",
    ];

    #[test]
    fn nanopore_chunks_are_never_paired() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &ONT);
        let (u, _) = long_units(t.path(), true).unwrap();
        assert_eq!(u.len(), 4);
        assert!(u.iter().all(|u| u.sample == "barcode01" && u.r1.is_none() && u.r2.is_none() && u.long.is_some()));
        // bacass takes one long-read file per row: ask to concatenate.
        let s = bacass();
        let err = check_long(&u, s.has_long_column(), s.has_run_column(), &mut Vec::new()).unwrap_err().to_string();
        assert!(err.contains("barcode01: 4 files") && err.contains("concatenate"), "{err}");
        // A schema with a run column may list them as rows of one sample.
        let mut w = Vec::new();
        check_long(&u, true, true, &mut w).unwrap();
        assert!(w.iter().any(|w| w.contains("one row each")), "{w:?}");

        // Even with numeric pairing on and no --long: no pairing, no "mate missing" error.
        let (files, _) = scan(&[t.path().to_path_buf()], &opts(true)).unwrap();
        let (u, w) = pair(files, &SHORT).unwrap();
        assert_eq!(u.len(), 4);
        assert!(u.iter().all(|u| u.r2.is_none()));
        assert!(w.iter().any(|w| w.contains("Oxford Nanopore") && w.contains("--long")), "{w:?}");
    }

    #[test]
    fn pass_fail_names_outside_barcode_folders_are_not_paired() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["run/FAX9_pass_x_1.fastq.gz", "run/FAX9_pass_x_2.fastq.gz"]);
        let u = units_of(t.path(), true).unwrap();
        assert_eq!(u.len(), 2);
        assert!(u.iter().all(|u| u.r2.is_none()));
    }

    #[test]
    fn short_lane_numbers_and_lowercase_mates() {
        let t = tempfile::tempdir().unwrap();
        touch(
            t.path(),
            &[
                "S1_EKDL01_HGV_L1_1.fq.gz",
                "S1_EKDL01_HGV_L1_2.fq.gz",
                "S1_EKDL01_HGV_L2_1.fq.gz",
                "S1_EKDL01_HGV_L2_2.fq.gz",
                "FP150002315_L01_1.fq.gz",
                "FP150002315_L01_2.fq.gz",
                "a_r1.fastq.gz",
                "a_r2.fastq.gz",
            ],
        );
        let u = units_of(t.path(), true).unwrap();
        let rows: Vec<_> = u.iter().map(|u| (u.sample.as_str(), u.r2.is_some())).collect();
        assert_eq!(
            rows,
            vec![("FP150002315", true), ("S1_EKDL01_HGV", true), ("S1_EKDL01_HGV", true), ("a", true)]
        );
        let c = classify("a_r1", true);
        assert_eq!((c.lane.as_str(), c.mate, c.token.as_str()), ("", Some(Mate::R1), "r1"));
        assert_eq!(classify("x_S1_L001_r2_001", true).lane, "001");
    }

    #[cfg(unix)]
    #[test]
    fn dangling_symlinks_are_skipped() {
        let t = tempfile::tempdir().unwrap();
        touch(t.path(), &["A_R1.fastq.gz", "A_R2.fastq.gz"]);
        std::os::unix::fs::symlink(t.path().join("gone.fastq.gz"), t.path().join("B_R1.fastq.gz")).unwrap();
        let (files, notes) = scan(&[t.path().to_path_buf()], &opts(true)).unwrap();
        assert_eq!(files.len(), 2);
        assert!(notes.iter().any(|n| n.contains("B_R1.fastq.gz")), "{notes:?}");
    }

    #[test]
    fn undetermined_reads_are_skipped_unless_asked() {
        let t = tempfile::tempdir().unwrap();
        touch(
            t.path(),
            &["Undetermined_S0_L001_R1_001.fastq.gz", "Undetermined_S0_L001_R2_001.fastq.gz", "A_S1_L001_R1_001.fastq.gz", "A_S1_L001_R2_001.fastq.gz"],
        );
        let (files, notes) = scan(&[t.path().to_path_buf()], &opts(true)).unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(notes.iter().filter(|n| n.contains("--include-undetermined")).count(), 2);
        let o = ScanOpts { include_undetermined: true, ..opts(true) };
        assert_eq!(scan(&[t.path().to_path_buf()], &o).unwrap().0.len(), 4);
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
