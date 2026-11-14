//! `place`: GTDB-Tk tree placement on placement-only reference data, with gPlace as the placer.
//!
//! `classify_wf` aligns user genomes against GTDB's reference MSA (7.9 GB), which the
//! `gtdbtk-r232-placement-lite` data leaves out, so it can't place genomes on that data. Placement
//! itself doesn't need the MSA. This runs the route that does work:
//!
//! 1. `gtdbtk identify`, then `gtdbtk align --skip_gtdb_refs`;
//! 2. `gtdbtk classify` with the ANI screen skipped (no skani database on disk), through GTDB-Tk's
//!    own `main()` after setting `options.skip_ani_screen`, as `classify_wf` does after its own
//!    screen;
//! 3. with a `pplacer` shim first on `PATH` that runs `gplace place --max_memory <cap>` with
//!    pplacer's arguments, and a check afterwards that every placement file came from gPlace.
//!
//! Genomes `gtdbprep ani` already assigned to a species can be skipped (`--skip-assigned`).
//! See `docs/tools/gtdbprep.md` and `docs/research/gtdb-placement-test.md`.

use crate::outputs::Summary;
use crate::tools::{self, Runner};
use anyhow::{Context, Result, anyhow, bail, ensure};
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

pub const PREFIX: &str = "gtdbtk";
/// The table `place` writes into the work folder (one row per input genome).
pub const TABLE: &str = "place.tsv";
/// The run record; its presence marks a work folder as `place`'s own.
pub const RECORD: &str = "place.json";
/// gPlace's memory cap: about 5 GiB peaks on every R232 tree (research/gplace-checks.md).
pub const DEFAULT_MAX_MEMORY: &str = "6G";
/// Entries a rerun of `place` deletes from its work folder. Nothing else is touched.
const WORK_ENTRIES: [&str; 9] = [
    "genomes",
    "genomes.json",
    "bin",
    "identify",
    "align",
    "classify",
    "logs",
    TABLE,
    RECORD,
];
/// The shim's call log, under `logs/`.
const CALLS: &str = "gplace-calls.log";

#[derive(Debug, Clone)]
pub struct PlaceOptions {
    pub genomes: PathBuf,
    pub extension: String,
    pub work: PathBuf,
    /// gPlace's `--max_memory` (e.g. `6G`).
    pub max_memory: String,
    /// GTDB-Tk's `--pplacer_cpus`, passed to gPlace as `-j`.
    pub pplacer_cpus: usize,
    /// Work folder of `gtdbprep ani`: genomes in its `assigned.json` are not placed.
    pub skip_assigned: Option<PathBuf>,
    /// The `gplace` program (default: the environment's, else the one on `PATH`).
    pub gplace: Option<PathBuf>,
    pub full_tree: bool,
}

/// One row of `place.tsv`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceRow {
    pub genome: String,
    pub classification: String,
    /// `placement`, `ani_screen` (skipped, from `gtdbprep ani`), `not_placed` (GTDB-Tk gave it
    /// no placement, e.g. too little of the alignment) or `no_result` (no row from GTDB-Tk).
    pub method: String,
    /// The rest are GTDB-Tk's columns, `N/A` where they don't apply.
    pub gtdbtk_method: String,
    pub note: String,
    pub pplacer_taxonomy: String,
    pub red_value: String,
    pub msa_percent: String,
    pub translation_table: String,
    /// Closest placement reference (placed genomes) or the species reference (ANI screen).
    pub closest_reference: String,
    pub closest_reference_ani: String,
    pub closest_reference_af: String,
    pub warnings: String,
}

pub const TABLE_HEADER: &str = "genome\tclassification\tmethod\tgtdbtk_method\tnote\tpplacer_taxonomy\t\
red_value\tmsa_percent\ttranslation_table\tclosest_reference\tclosest_reference_ani\tclosest_reference_af\twarnings";

impl PlaceRow {
    fn fields(&self) -> [&str; 13] {
        [
            &self.genome,
            &self.classification,
            &self.method,
            &self.gtdbtk_method,
            &self.note,
            &self.pplacer_taxonomy,
            &self.red_value,
            &self.msa_percent,
            &self.translation_table,
            &self.closest_reference,
            &self.closest_reference_ani,
            &self.closest_reference_af,
            &self.warnings,
        ]
    }
}

pub fn rows_to_tsv(rows: &[PlaceRow]) -> String {
    let mut s = format!("{TABLE_HEADER}\n");
    for r in rows {
        s.push_str(&r.fields().join("\t"));
        s.push('\n');
    }
    s
}

/// Parse `place.tsv` back (for callers that wrap the command).
pub fn parse_table(text: &str) -> Result<Vec<PlaceRow>> {
    let mut lines = text.lines();
    ensure!(
        lines.next() == Some(TABLE_HEADER),
        "not a gtdbprep place.tsv (header differs)"
    );
    lines
        .filter(|l| !l.is_empty())
        .map(|l| {
            let f: Vec<String> = l.split('\t').map(str::to_string).collect();
            ensure!(
                f.len() == 13,
                "place.tsv row has {} columns, not 13",
                f.len()
            );
            let mut f = f.into_iter();
            let mut n = || f.next().unwrap();
            Ok(PlaceRow {
                genome: n(),
                classification: n(),
                method: n(),
                gtdbtk_method: n(),
                note: n(),
                pplacer_taxonomy: n(),
                red_value: n(),
                msa_percent: n(),
                translation_table: n(),
                closest_reference: n(),
                closest_reference_ani: n(),
                closest_reference_af: n(),
                warnings: n(),
            })
        })
        .collect()
}

/// What a run did.
#[derive(Debug, Clone, Default)]
pub struct PlaceReport {
    pub rows: Vec<PlaceRow>,
    /// Genomes given to GTDB-Tk.
    pub placed: usize,
    /// Genomes skipped because `gtdbprep ani` assigned them.
    pub skipped: usize,
    /// `pplacer` calls the shim turned into gPlace calls.
    pub gplace_calls: usize,
    pub table: PathBuf,
    /// GTDB-Tk's own output folder (`gtdbtk.bac120.summary.tsv` and friends).
    pub gtdbtk_out: PathBuf,
    pub seconds_identify: f64,
    pub seconds_align: f64,
    pub seconds_classify: f64,
}

/// A memory size as gPlace's `--max_memory` reads it: `6G`, `8.5G`, `500M`, `6GiB`, `1.5T`, a
/// plain number of GiB, or bytes with a `B` suffix. Units are binary. Returns bytes.
pub fn parse_memory(value: &str) -> Result<u64> {
    let bad = || anyhow!("invalid memory size {value:?} (examples: 6G, 500M, 8.5G)");
    let s = value.trim();
    let end = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, rest) = s.split_at(end);
    if num.is_empty() || num.matches('.').count() > 1 || num.ends_with('.') {
        return Err(bad());
    }
    let rest = rest.trim_start().to_ascii_uppercase();
    let (unit, suffix) = match rest.chars().next() {
        Some(c @ ('K' | 'M' | 'G' | 'T')) => (Some(c), &rest[1..]),
        _ => (None, rest.as_str()),
    };
    if !matches!(suffix, "" | "B" | "IB") {
        return Err(bad());
    }
    let power = match unit {
        Some(c) => "KMGT".find(c).unwrap() as i32 + 1,
        None if suffix.is_empty() => 3,
        None => 0,
    };
    let bytes = num.parse::<f64>().map_err(|_| bad())? * 1024f64.powi(power);
    ensure!(bytes >= 1.0, "memory size must be positive: {value:?}");
    Ok(bytes as u64)
}

fn sh_quote(p: &Path) -> Result<String> {
    let s = p
        .to_str()
        .with_context(|| format!("{} is not valid UTF-8", p.display()))?;
    Ok(format!("'{}'", s.replace('\'', r"'\''")))
}

/// The `pplacer` shim. GTDB-Tk 2.8 runs `pplacer -m WAG -j N -c REFPKG -o OUT.json MSA` (plus
/// `--mmap-file` with `--scratch_dir`); gPlace takes the same arguments. `--version` goes to the
/// real pplacer if there is one (GTDB-Tk logs it), otherwise to gPlace.
pub fn shim_script(
    gplace: &Path,
    real_pplacer: Option<&Path>,
    calls: &Path,
    max_memory: &str,
) -> Result<String> {
    let version = match real_pplacer {
        Some(p) => format!("exec {} --version", sh_quote(p)?),
        None => format!("exec {} --version", sh_quote(gplace)?),
    };
    Ok(format!(
        "#!/bin/sh\n\
         # Written by gtdbprep place: GTDB-Tk calls `pplacer`; this runs gPlace with the same\n\
         # arguments and a memory cap, and logs each call.\n\
         if [ \"${{1:-}}\" = --version ]; then {version}; fi\n\
         printf '%s\\n' \"$*\" >> {calls}\n\
         exec {gplace} place --max_memory {mem} \"$@\"\n",
        calls = sh_quote(calls)?,
        gplace = sh_quote(gplace)?,
        mem = sh_quote(Path::new(max_memory))?,
    ))
}

/// The `-o` targets in the shim's call log, resolved against `cwd`.
pub fn logged_outputs(log: &str, cwd: &Path) -> Vec<PathBuf> {
    log.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            it.by_ref().find(|w| *w == "-o")?;
            it.next().map(|o| cwd.join(o))
        })
        .collect()
}

/// Every placement file (`pplacer.*.json`) GTDB-Tk left under `dir`, recursively.
fn placement_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for e in fs::read_dir(dir).with_context(|| format!("listing {}", dir.display()))? {
        let e = e?;
        let p = e.path();
        if e.file_type()?.is_dir() {
            out.extend(placement_files(&p)?);
        } else {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with("pplacer.") && name.ends_with(".json") {
                out.push(p);
            }
        }
    }
    Ok(out)
}

/// Check that gPlace wrote every placement file under `classify_dir`: each must be the `-o` of a
/// call in the shim's log. Returns the number of calls.
pub fn check_gplace_ran(classify_dir: &Path, calls_log: &Path, cwd: &Path) -> Result<usize> {
    let text = fs::read_to_string(calls_log).unwrap_or_default();
    let logged: BTreeSet<PathBuf> = logged_outputs(&text, cwd)
        .into_iter()
        .map(|p| fs::canonicalize(&p).unwrap_or(p))
        .collect();
    for f in placement_files(classify_dir)? {
        let f = fs::canonicalize(&f)?;
        ensure!(
            logged.contains(&f),
            "{} was not written through the gPlace shim (did the real pplacer run?)",
            f.display()
        );
    }
    Ok(text.lines().filter(|l| !l.trim().is_empty()).count())
}

/// What `gtdbprep ani` assigned: the genome IDs and, for each, its row of GTDB-Tk's
/// `ani_summary.tsv` (reference, ANI, AF, taxonomy).
#[derive(Debug, Clone, Default)]
pub struct AniCalls {
    pub assigned: BTreeSet<String>,
    pub rows: BTreeMap<String, [String; 4]>,
}

/// Read `<ani work>/assigned.json` and `<ani work>/out/classify/ani_screen/*.ani_summary.tsv`.
pub fn read_ani(dir: &Path) -> Result<AniCalls> {
    let aj = dir.join("assigned.json");
    let text = fs::read_to_string(&aj).with_context(|| {
        format!(
            "reading {} (--skip-assigned takes the --work folder of `gtdbprep ani`)",
            aj.display()
        )
    })?;
    let assigned: BTreeSet<String> = serde_json::from_str::<Vec<String>>(&text)
        .with_context(|| format!("{} is not a list of genome IDs", aj.display()))?
        .into_iter()
        .collect();
    let mut rows = BTreeMap::new();
    for domain in ["bac120", "ar53"] {
        let p = dir.join(format!(
            "out/classify/ani_screen/{PREFIX}.{domain}.ani_summary.tsv"
        ));
        if !p.is_file() {
            continue;
        }
        let text = fs::read_to_string(&p)?;
        let mut lines = text.lines();
        let header: Vec<&str> = lines.next().unwrap_or_default().split('\t').collect();
        let col = |n: &str| {
            header
                .iter()
                .position(|h| *h == n)
                .with_context(|| format!("{} has no {n} column", p.display()))
        };
        let cols = [
            col("reference_genome")?,
            col("skani_ani")?,
            col("skani_af")?,
            col("reference_taxonomy")?,
        ];
        for l in lines.filter(|l| !l.is_empty()) {
            let f: Vec<&str> = l.split('\t').collect();
            let get = |i: usize| f.get(i).copied().unwrap_or("N/A").to_string();
            rows.insert(get(0), cols.map(get));
        }
    }
    let missing: Vec<&String> = assigned.iter().filter(|g| !rows.contains_key(*g)).collect();
    ensure!(
        missing.is_empty(),
        "{} lists genomes that its ani_summary.tsv files don't: {missing:?}",
        aj.display()
    );
    Ok(AniCalls { assigned, rows })
}

/// One row per input genome: GTDB-Tk's summary rows for placed genomes, the ANI screen's call
/// for skipped ones, and `no_result` for genomes GTDB-Tk returned nothing for.
pub fn build_rows(
    genomes: &BTreeSet<String>,
    summary: Option<&Summary>,
    ani: &AniCalls,
) -> Vec<PlaceRow> {
    let na = || "N/A".to_string();
    genomes
        .iter()
        .map(|g| {
            if ani.assigned.contains(g) {
                let [reference, a, af, taxonomy] = ani.rows[g].clone();
                return PlaceRow {
                    genome: g.clone(),
                    classification: taxonomy,
                    method: "ani_screen".into(),
                    gtdbtk_method: "ANI screen (gtdbprep ani)".into(),
                    note: na(),
                    pplacer_taxonomy: na(),
                    red_value: na(),
                    msa_percent: na(),
                    translation_table: na(),
                    closest_reference: reference,
                    closest_reference_ani: a,
                    closest_reference_af: af,
                    warnings: na(),
                };
            }
            let Some((s, row)) = summary.and_then(|s| s.rows.get(g).map(|r| (s, r))) else {
                return PlaceRow {
                    genome: g.clone(),
                    classification: na(),
                    method: "no_result".into(),
                    gtdbtk_method: na(),
                    note: "no row in GTDB-Tk's summary (see identify/gtdbtk.failed_genomes.tsv)"
                        .into(),
                    pplacer_taxonomy: na(),
                    red_value: na(),
                    msa_percent: na(),
                    translation_table: na(),
                    closest_reference: na(),
                    closest_reference_ani: na(),
                    closest_reference_af: na(),
                    warnings: na(),
                };
            };
            // `rows` hold every column but the first (user_genome).
            let get = |name: &str| {
                s.header
                    .iter()
                    .position(|h| h == name)
                    .and_then(|i| i.checked_sub(1))
                    .and_then(|i| row.get(i))
                    .cloned()
                    .unwrap_or_else(na)
            };
            let pplacer_taxonomy = get("pplacer_taxonomy");
            let method = if pplacer_taxonomy == "N/A" {
                "not_placed"
            } else {
                "placement"
            };
            PlaceRow {
                genome: g.clone(),
                classification: get("classification"),
                method: method.into(),
                gtdbtk_method: get("classification_method"),
                note: get("note"),
                pplacer_taxonomy,
                red_value: get("red_value"),
                msa_percent: get("msa_percent"),
                translation_table: get("translation_table"),
                closest_reference: get("closest_placement_reference"),
                closest_reference_ani: get("closest_placement_ani"),
                closest_reference_af: get("closest_placement_af"),
                warnings: get("warnings"),
            }
        })
        .collect()
}

/// GTDB-Tk's genome listing (`_genomes_to_process`, so IDs follow its rules), and `classify`
/// with the ANI screen skipped, after checking that `pplacer` on `PATH` is the gPlace shim.
const PLACE_PY: &str = r#"
import json, os, shutil, sys
mode, supported = sys.argv[1], sys.argv[2]
import gtdbtk
if gtdbtk.__version__ not in supported.split(','):
    sys.exit('gtdbprep: GTDB-Tk %s is not supported for this call (supported: %s)'
             % (gtdbtk.__version__, supported))
if mode == 'list':
    genome_dir, extension, result = sys.argv[3:6]
    from gtdbtk.main import OptionsParser
    genomes, _ = OptionsParser(gtdbtk.__version__)._genomes_to_process(genome_dir, None, extension)
    with open(result, 'w') as fh:
        json.dump(genomes, fh)
else:
    shim = sys.argv[3]
    found = shutil.which('pplacer')
    if found is None or os.path.realpath(found) != os.path.realpath(shim):
        sys.exit('gtdbprep: pplacer on PATH is %s, not the gPlace shim %s' % (found, shim))
    import gtdbtk.main as m
    orig = m.OptionsParser.classify
    def classify(self, options, *a, **k):
        # No skani database on placement-only data; classify_wf sets the same flag after its
        # own ANI step.
        options.skip_ani_screen = True
        return orig(self, options, *a, **k)
    m.OptionsParser.classify = classify
    from gtdbtk.__main__ import main
    sys.argv = ['gtdbtk'] + sys.argv[4:]
    main()
"#;

/// Make the work folder, or clear `place`'s own entries from one an earlier `place` run made.
/// Refuses any other non-empty folder. Returns its canonical path.
fn prepare_work(work: &Path) -> Result<PathBuf> {
    if work.exists() {
        let names: Vec<String> = fs::read_dir(work)?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        if !names.is_empty() {
            ensure!(
                work.join(RECORD).is_file(),
                "work folder {} is not empty and was not made by `gtdbprep place`; use an empty \
                 or new folder",
                work.display()
            );
            for n in names.iter().filter(|n| WORK_ENTRIES.contains(&n.as_str())) {
                let p = work.join(n);
                if p.is_dir() {
                    fs::remove_dir_all(&p)?
                } else {
                    fs::remove_file(&p)?
                }
            }
        }
    }
    fs::create_dir_all(work)?;
    let work = fs::canonicalize(work)?;
    crate::pipeline::check_no_space(&work)?;
    Ok(work)
}

#[cfg(unix)]
fn make_executable(p: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(p, fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(unix)]
fn symlink(src: &Path, dest: &Path) -> Result<()> {
    std::os::unix::fs::symlink(src, dest)
        .with_context(|| format!("linking {} to {}", dest.display(), src.display()))
}

#[cfg(not(unix))]
fn make_executable(_: &Path) -> Result<()> {
    bail!("gtdbprep place needs a Unix system (GTDB-Tk runs on Linux and macOS)")
}

#[cfg(not(unix))]
fn symlink(_: &Path, _: &Path) -> Result<()> {
    bail!("gtdbprep place needs a Unix system (GTDB-Tk runs on Linux and macOS)")
}

/// Run identify, align (no references) and classify (no ANI screen) with gPlace as the placer.
pub fn run(opts: &PlaceOptions, t: &crate::pipeline::Tools) -> Result<PlaceReport> {
    parse_memory(&opts.max_memory)?;
    let max_memory: String = opts.max_memory.split_whitespace().collect();
    ensure!(opts.pplacer_cpus >= 1, "--pplacer-cpus must be at least 1");
    let ani = match &opts.skip_assigned {
        Some(d) => read_ani(d)?,
        None => AniCalls::default(),
    };
    let genomes_dir = fs::canonicalize(&opts.genomes)
        .with_context(|| format!("genome folder {}", opts.genomes.display()))?;
    let data = fs::canonicalize(&t.data)
        .with_context(|| format!("reference data {}", t.data.display()))?;
    let work = prepare_work(&opts.work)?;
    let started = humantime::format_rfc3339_seconds(std::time::SystemTime::now()).to_string();
    fs::write(
        work.join(RECORD),
        serde_json::to_string_pretty(&serde_json::json!({
            "format": "gtdbprep-place/1", "started": started, "status": "running"
        }))?,
    )?;
    let runner = Runner {
        env_prefix: t.env_prefix.clone(),
        data: data.clone(),
        cpus: t.cpus,
        log_dir: work.join("logs"),
    };
    fs::create_dir_all(&runner.log_dir)?;
    let base_path = runner.path_var().or_else(|| std::env::var_os("PATH"));

    // gPlace, and the real pplacer (only for `--version`).
    let gplace = match &opts.gplace {
        Some(p) if p.components().count() > 1 => fs::canonicalize(p)
            .with_context(|| format!("gPlace program {}", p.display()))?,
        Some(p) => tools::which(&p.to_string_lossy(), base_path.clone())
            .with_context(|| format!("{} not found on PATH", p.display()))?,
        None => tools::which("gplace", base_path.clone()).context(
            "gPlace not found: install it into the GTDB-Tk environment (see docs/tools/gtdbprep.md) \
             or pass --gplace",
        )?,
    };
    let gplace_version = runner
        .command_at(&gplace)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr)
            );
            text.lines()
                .map(str::trim)
                .find(|l| tools::first_version(l).is_some())
                .map(str::to_string)
        })
        .with_context(|| format!("{} --version printed no version", gplace.display()))?;
    let real_pplacer = tools::which("pplacer", base_path.clone());
    let bin = work.join("bin");
    fs::create_dir_all(&bin)?;
    let shim = bin.join("pplacer");
    let calls_log = runner.log_dir.join(CALLS);
    fs::write(
        &shim,
        shim_script(&gplace, real_pplacer.as_deref(), &calls_log, &max_memory)?,
    )?;
    make_executable(&shim)?;
    let mut paths = vec![bin.clone()];
    if let Some(p) = &base_path {
        paths.extend(std::env::split_paths(p));
    }
    let path: OsString = std::env::join_paths(paths)?;
    let python = runner.gtdbtk_python()?;
    let command = |program: &Path| {
        let mut c = runner.command_at(program);
        c.env("PATH", &path).current_dir(&work);
        c
    };

    // Inputs are linked into the work folder first: GTDB-Tk refuses genome paths with spaces.
    // Then GTDB-Tk's own listing names them, so IDs match the ones `gtdbprep ani` reports, and
    // the genomes the ANI screen assigned are unlinked again.
    let staged = work.join("genomes");
    fs::create_dir_all(&staged)?;
    for e in fs::read_dir(&genomes_dir)? {
        let e = e?;
        let name = e.file_name();
        if name.to_string_lossy().ends_with(opts.extension.as_str()) && e.path().is_file() {
            symlink(&fs::canonicalize(e.path())?, &staged.join(&name))?;
        }
    }
    let list_json = work.join("genomes.json");
    let mut c = command(&python);
    c.arg("-c")
        .arg(PLACE_PY)
        .arg("list")
        .arg(tools::GTDBTK_SUPPORTED.join(","))
        .arg(&staged)
        .arg(&opts.extension)
        .arg(&list_json);
    runner.run("list_genomes", c).with_context(|| {
        format!(
            "listing *{} genomes in {}",
            opts.extension,
            genomes_dir.display()
        )
    })?;
    let listed: BTreeMap<String, PathBuf> = serde_json::from_str(&fs::read_to_string(&list_json)?)?;
    ensure!(
        !listed.is_empty(),
        "no *{} genomes in {}",
        opts.extension,
        genomes_dir.display()
    );
    let ids: BTreeSet<String> = listed.keys().cloned().collect();
    let elsewhere = ani.assigned.difference(&ids).count();
    if elsewhere > 0 {
        eprintln!(
            "gtdbprep: {elsewhere} genome(s) the ANI screen assigned are not in {}; ignored",
            genomes_dir.display()
        );
    }
    let mut placed = 0;
    for (id, link) in &listed {
        if ani.assigned.contains(id) {
            fs::remove_file(link).with_context(|| format!("removing {}", link.display()))?;
        } else {
            placed += 1;
        }
    }
    let skipped = ids.len() - placed;
    let out = work.join("classify");
    let mut report = PlaceReport {
        placed,
        skipped,
        table: work.join(TABLE),
        gtdbtk_out: out.clone(),
        ..Default::default()
    };
    let cpus = runner.cpus.to_string();
    let mut summary = None;
    if placed > 0 {
        let t0 = Instant::now();
        let mut c = command(Path::new(
            &tools::which("gtdbtk", Some(path.clone())).context("no gtdbtk on PATH (use --env)")?,
        ));
        c.arg("identify")
            .arg("--genome_dir")
            .arg(&staged)
            .arg("--out_dir")
            .arg(work.join("identify"))
            .args(["--extension", &opts.extension, "--cpus", &cpus]);
        runner.run("identify", c)?;
        report.seconds_identify = t0.elapsed().as_secs_f64();

        let t1 = Instant::now();
        let mut c = command(Path::new(
            &tools::which("gtdbtk", Some(path.clone())).context("no gtdbtk on PATH")?,
        ));
        c.arg("align")
            .arg("--identify_dir")
            .arg(work.join("identify"))
            .arg("--out_dir")
            .arg(work.join("align"))
            .args(["--skip_gtdb_refs", "--cpus", &cpus]);
        runner.run("align", c)?;
        report.seconds_align = t1.elapsed().as_secs_f64();

        let t2 = Instant::now();
        let mut c = command(&python);
        c.arg("-c")
            .arg(PLACE_PY)
            .arg("classify")
            .arg(tools::GTDBTK_SUPPORTED.join(","))
            .arg(&shim)
            .arg("classify")
            .arg("--genome_dir")
            .arg(&staged)
            .arg("--align_dir")
            .arg(work.join("align"))
            .arg("--out_dir")
            .arg(&out)
            .args([
                "--extension",
                &opts.extension,
                "--cpus",
                &cpus,
                "--pplacer_cpus",
                &opts.pplacer_cpus.to_string(),
            ]);
        if opts.full_tree {
            c.arg("--full_tree");
        }
        runner.run("classify", c)?;
        report.seconds_classify = t2.elapsed().as_secs_f64();
        report.gplace_calls = check_gplace_ran(&out, &calls_log, &work)?;
        summary = match Summary::read_dir(&out, PREFIX) {
            Ok(s) => Some(s),
            Err(_) if !out.join("classify").is_dir() => None,
            Err(e) => return Err(e),
        };
    }
    report.rows = build_rows(&ids, summary.as_ref(), &ani);
    let n_placed = report
        .rows
        .iter()
        .filter(|r| r.method == "placement")
        .count();
    if n_placed > 0 && report.gplace_calls == 0 {
        bail!(
            "{n_placed} genome(s) were placed but the gPlace shim logged no call ({})",
            calls_log.display()
        );
    }
    fs::write(&report.table, rows_to_tsv(&report.rows))?;

    let version = |p: &str| runner.version_line(p, &["--version"]);
    let record = serde_json::json!({
        "format": "gtdbprep-place/1",
        "client": {"name": "gtdbprep", "version": env!("CARGO_PKG_VERSION")},
        "started": started,
        "finished": humantime::format_rfc3339_seconds(std::time::SystemTime::now()).to_string(),
        "status": "done",
        "tools": {
            "gtdbtk": version("gtdbtk"),
            "gplace": gplace_version,
            "gplace_path": gplace,
            "pplacer_for_version": real_pplacer,
        },
        "options": {
            "genomes": genomes_dir,
            "extension": opts.extension,
            "data": data,
            "max_memory": max_memory,
            "cpus": runner.cpus,
            "pplacer_cpus": opts.pplacer_cpus,
            "full_tree": opts.full_tree,
            "skip_assigned": opts.skip_assigned,
        },
        "genomes": {"input": ids.len(), "placed": placed, "skipped_ani": skipped},
        "gplace_calls": report.gplace_calls,
        "seconds": {
            "identify": report.seconds_identify,
            "align": report.seconds_align,
            "classify": report.seconds_classify,
        },
    });
    fs::write(work.join(RECORD), serde_json::to_string_pretty(&record)?)?;
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_sizes_as_gplace_reads_them() {
        let g = 1u64 << 30;
        assert_eq!(parse_memory("6G").unwrap(), 6 * g);
        assert_eq!(parse_memory("6").unwrap(), 6 * g);
        assert_eq!(parse_memory(" 8.5g ").unwrap(), 8 * g + g / 2);
        assert_eq!(parse_memory("500M").unwrap(), 500 << 20);
        assert_eq!(parse_memory("6GiB").unwrap(), 6 * g);
        assert_eq!(parse_memory("6 GB").unwrap(), 6 * g);
        assert_eq!(parse_memory("1024B").unwrap(), 1024);
        assert_eq!(parse_memory(".5T").unwrap(), 512 * g);
        for bad in ["", "G", "6X", "6.", "1.2.3G", "-1G", "6G;rm", "0", "6 G B"] {
            assert!(parse_memory(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn shim_quotes_paths_and_logs_calls() {
        let s = shim_script(
            Path::new("/env/bin/gplace"),
            Some(Path::new("/env/bin/pplacer")),
            Path::new("/w/it's/logs/gplace-calls.log"),
            "6G",
        )
        .unwrap();
        assert!(s.starts_with("#!/bin/sh\n"));
        assert!(s.contains("then exec '/env/bin/pplacer' --version; fi"));
        assert!(s.contains(r"'/w/it'\''s/logs/gplace-calls.log'"));
        assert!(s.contains("exec '/env/bin/gplace' place --max_memory '6G' \"$@\""));
        let s = shim_script(Path::new("/g"), None, Path::new("/l"), "6G").unwrap();
        assert!(s.contains("then exec '/g' --version; fi"));
    }

    #[test]
    fn placement_outputs_must_come_from_logged_gplace_calls() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path();
        let p = w.join("classify/classify/intermediate_results/pplacer");
        fs::create_dir_all(p.join("tree_6")).unwrap();
        fs::write(p.join("pplacer.backbone.bac120.json"), "{}").unwrap();
        fs::write(p.join("tree_6/pplacer.class_level.bac120.json"), "{}").unwrap();
        fs::write(p.join("gplace.log"), "x").unwrap();
        let log = w.join("calls.log");
        fs::write(
            &log,
            "-m wag -j 1 -c /d/backbone.refpkg -o classify/classify/intermediate_results/pplacer/pplacer.backbone.bac120.json msa.fasta.gz\n",
        )
        .unwrap();
        let e = check_gplace_ran(&w.join("classify"), &log, w).unwrap_err();
        assert!(e.to_string().contains("tree_6/pplacer.class_level"), "{e}");
        let mut text = fs::read_to_string(&log).unwrap();
        text.push_str(&format!(
            "-m wag -j 1 -c /d/6.refpkg -o {} m.fasta\n",
            p.join("tree_6/pplacer.class_level.bac120.json").display()
        ));
        fs::write(&log, text).unwrap();
        assert_eq!(check_gplace_ran(&w.join("classify"), &log, w).unwrap(), 2);
        assert_eq!(
            logged_outputs(
                "--max_memory 6G -o a.json\nno output here\n",
                Path::new("/w")
            ),
            vec![PathBuf::from("/w/a.json")]
        );
    }

    /// GTDB-Tk 2.8.0's summary for the three Crystal Geyser bins of research/mag-laptop.md.
    pub(crate) const LAPTOP_SUMMARY: &str = "user_genome\tclassification\tclosest_genome_reference\tclosest_genome_reference_radius\tclosest_genome_taxonomy\tclosest_genome_ani\tclosest_genome_af\tclosest_placement_reference\tclosest_placement_radius\tclosest_placement_taxonomy\tclosest_placement_ani\tclosest_placement_af\tpplacer_taxonomy\tclassification_method\tnote\tother_related_references(genome_id,species_name,radius,ANI,AF)\tmsa_percent\ttranslation_table\tred_value\twarnings
bin.12\tUnclassified Bacteria\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tInsufficient number of amino acids in MSA (3.6%)
bin.13\td__Bacteria;p__Pseudomonadota;c__Gammaproteobacteria;o__Burkholderiales;f__Gallionellaceae;g__Sideroxyarcus;s__\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\td__Bacteria;p__Pseudomonadota;c__Gammaproteobacteria;o__Burkholderiales;f__Gallionellaceae;g__Sideroxyarcus;s__\ttaxonomic classification fully defined by topology\tclassification based on placement in class-level tree\tN/A\t33.13\t11\t0.99999\tGenome has more than 10.0% of markers with multiple hits
bin.16\td__Bacteria;p__Patescibacteriota;c__Minisyncoccia;o__Phycocordibacterales;f__UBA918;g__OLB19;s__\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\tN/A\td__Bacteria;p__Patescibacteriota;c__Minisyncoccia;o__Phycocordibacterales;f__UBA918;g__OLB19;s__\ttaxonomic classification fully defined by topology\tclassification based on placement in class-level tree\tN/A\t52.06\t11\t0.98325\tN/A
";

    #[test]
    fn rows_from_summary_and_ani_screen() {
        let s = Summary::parse(LAPTOP_SUMMARY).unwrap();
        let mut ani = AniCalls::default();
        ani.assigned.insert("bin.1".into());
        ani.rows.insert(
            "bin.1".into(),
            [
                "GCA_001873625.1".into(),
                "99.96".into(),
                "0.9641".into(),
                "d__Bacteria;s__UBA1547 sp001873625".into(),
            ],
        );
        let ids: BTreeSet<String> = ["bin.1", "bin.12", "bin.13", "bin.16", "bin.99"]
            .map(String::from)
            .into();
        let rows = build_rows(&ids, Some(&s), &ani);
        let by: BTreeMap<&str, &PlaceRow> = rows.iter().map(|r| (r.genome.as_str(), r)).collect();
        assert_eq!(by["bin.1"].method, "ani_screen");
        assert_eq!(by["bin.1"].closest_reference, "GCA_001873625.1");
        assert_eq!(
            by["bin.1"].classification,
            "d__Bacteria;s__UBA1547 sp001873625"
        );
        assert_eq!(by["bin.12"].method, "not_placed");
        assert_eq!(by["bin.12"].classification, "Unclassified Bacteria");
        assert!(by["bin.12"].warnings.contains("3.6%"));
        assert_eq!(by["bin.13"].method, "placement");
        assert!(by["bin.13"].classification.contains("g__Sideroxyarcus;s__"));
        assert_eq!(by["bin.13"].msa_percent, "33.13");
        assert_eq!(by["bin.13"].red_value, "0.99999");
        assert_eq!(
            by["bin.16"].gtdbtk_method,
            "taxonomic classification fully defined by topology"
        );
        assert_eq!(
            by["bin.16"].note,
            "classification based on placement in class-level tree"
        );
        assert!(by["bin.16"].classification.contains("g__OLB19;s__"));
        assert_eq!(by["bin.99"].method, "no_result");
        let tsv = rows_to_tsv(&rows);
        assert_eq!(parse_table(&tsv).unwrap(), rows);
        assert!(parse_table("genome\tclassification\n").is_err());
    }

    #[test]
    fn ani_output_is_read_and_checked() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path();
        assert!(
            read_ani(a)
                .unwrap_err()
                .to_string()
                .contains("gtdbprep ani")
        );
        fs::write(a.join("assigned.json"), r#"["bin.1", "bin.2"]"#).unwrap();
        let s = a.join("out/classify/ani_screen");
        fs::create_dir_all(&s).unwrap();
        let header =
            "user_genome\treference_genome\tskani_ani\tskani_af\treference_taxonomy\tother\n";
        fs::write(
            s.join("gtdbtk.bac120.ani_summary.tsv"),
            format!("{header}bin.1\tGCA_1\t99.9\t0.9\td__Bacteria;s__x\t\n"),
        )
        .unwrap();
        let e = read_ani(a).unwrap_err().to_string();
        assert!(e.contains("bin.2"), "{e}");
        fs::write(
            s.join("gtdbtk.ar53.ani_summary.tsv"),
            format!("{header}bin.2\tGCA_2\t97.0\t0.6\td__Archaea;s__y\t\n"),
        )
        .unwrap();
        let c = read_ani(a).unwrap();
        assert_eq!(c.assigned.len(), 2);
        assert_eq!(c.rows["bin.2"][0], "GCA_2");
        assert_eq!(c.rows["bin.2"][3], "d__Archaea;s__y");
    }

    #[test]
    fn work_folder_must_be_empty_or_our_own() {
        let d = tempfile::tempdir().unwrap();
        let w = d.path().join("w");
        prepare_work(&w).unwrap();
        fs::write(w.join("other.txt"), "keep").unwrap();
        assert!(prepare_work(&w).is_err());
        fs::write(w.join(RECORD), "{}").unwrap();
        fs::create_dir_all(w.join("classify/x")).unwrap();
        prepare_work(&w).unwrap();
        assert!(!w.join("classify").exists() && !w.join(RECORD).exists());
        assert!(w.join("other.txt").exists());
        assert!(prepare_work(&d.path().join("has space")).is_err());
    }

    /// A stand-in GTDB-Tk 2.8.0 (Python package plus `gtdbtk` script) and `gplace`: `classify`
    /// refuses to run unless the ANI screen is skipped, and calls `pplacer` by name.
    #[cfg(unix)]
    fn fake_env(root: &Path, system_python: &str) -> PathBuf {
        let pkg = root.join("pkg/gtdbtk");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(pkg.join("__init__.py"), "__version__ = '2.8.0'\n").unwrap();
        let summary_header = LAPTOP_SUMMARY.lines().next().unwrap();
        fs::write(
            pkg.join("main.py"),
            format!(
                r#"import os, subprocess, sys
HEADER = {summary_header:?}
class OptionsParser:
    def __init__(self, version): pass
    def _genomes_to_process(self, genome_dir, batchfile, extension):
        return {{f[:-len(extension) - 1]: os.path.join(genome_dir, f)
                for f in sorted(os.listdir(genome_dir)) if f.endswith('.' + extension)}}, {{}}
    def parse_options(self, o):
        get = lambda n: o.argv[o.argv.index(n) + 1]
        if o.subparser_name in ('identify', 'align'):
            os.makedirs(get('--out_dir'))
        elif o.subparser_name == 'classify':
            self.classify(o)
    def classify(self, o):
        if not getattr(o, 'skip_ani_screen', False):
            sys.exit('ERROR: Controlled exit: no skani database')
        get = lambda n: o.argv[o.argv.index(n) + 1]
        out = get('--out_dir')
        ids = sorted(f[:-len(get('--extension')) - 1] for f in os.listdir(get('--genome_dir')))
        pp = os.path.join(out, 'classify/intermediate_results/pplacer')
        os.makedirs(pp)
        subprocess.run(['pplacer', '-m', 'WAG', '-j', get('--pplacer_cpus'), '-c', 'REF',
                        '-o', os.path.join(pp, 'pplacer.backbone.bac120.json'), 'msa'], check=True)
        na = ['N/A'] * 19
        with open(os.path.join(out, 'classify/gtdbtk.bac120.summary.tsv'), 'w') as fh:
            fh.write(HEADER + '\n')
            for i, g in enumerate(ids):
                row = list(na)
                if i == 0:
                    row[0] = 'Unclassified Bacteria'
                else:
                    row[0] = row[11] = 'd__Bacteria;g__G{{}};s__'.format(i)
                    row[15] = '50.0'
                fh.write('\t'.join([g] + row) + '\n')
"#
            ),
        )
        .unwrap();
        fs::write(
            pkg.join("__main__.py"),
            "import sys, types\n\
             def main():\n    \
             from gtdbtk.main import OptionsParser\n    \
             o = types.SimpleNamespace(subparser_name=sys.argv[1], argv=sys.argv[2:])\n    \
             OptionsParser('2.8.0').parse_options(o)\n",
        )
        .unwrap();
        let env = root.join("env");
        fs::create_dir_all(env.join("bin")).unwrap();
        let pypath = root.join("pkg");
        let script = |name: &str, body: String| {
            let p = env.join("bin").join(name);
            fs::write(&p, body).unwrap();
            make_executable(&p).unwrap();
        };
        script(
            "python3",
            format!(
                "#!/bin/sh\nPYTHONPATH='{}' exec '{system_python}' \"$@\"\n",
                pypath.display()
            ),
        );
        script(
            "gtdbtk",
            format!(
                "#!/bin/sh\nexec '{}' -c 'import sys; sys.argv = [\"gtdbtk\"] + sys.argv[1:]\n\
                 from gtdbtk.__main__ import main; main()' \"$@\"\n",
                env.join("bin/python3").display()
            ),
        );
        // gPlace stand-in: writes the -o file and records its arguments.
        script(
            "gplace",
            format!(
                "#!/bin/sh\nif [ \"$1\" = --version ]; then echo 'gplace: version 0.1.1'; exit 0; fi\n\
                 echo \"$@\" >> '{}'\n\
                 while [ $# -gt 0 ]; do if [ \"$1\" = -o ]; then echo '{{}}' > \"$2\"; fi; shift; done\n",
                root.join("gplace-args.txt").display()
            ),
        );
        // A real pplacer would write the file too; this one fails so a bypassed shim shows.
        script(
            "pplacer",
            "#!/bin/sh\nif [ \"$1\" = --version ]; then echo v1.1.alpha19-0-g807f6f3; exit 0; fi\nexit 9\n"
                .into(),
        );
        env
    }

    #[cfg(unix)]
    #[test]
    fn place_end_to_end_with_stand_ins() {
        let Some(sys_py) = std::process::Command::new("python3")
            .args(["-c", "import sys; print(sys.executable)"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        else {
            eprintln!("no python3; skipping");
            return;
        };
        let d = tempfile::tempdir().unwrap();
        let root = d.path();
        let env = fake_env(root, &sys_py);
        let genomes = root.join("my genomes");
        fs::create_dir_all(&genomes).unwrap();
        for g in ["bin.1", "bin.12", "bin.13"] {
            fs::write(genomes.join(format!("{g}.fa")), ">c1\nACGT\n").unwrap();
        }
        fs::write(genomes.join("notes.txt"), "not a genome").unwrap();
        let ani = root.join("ani");
        fs::create_dir_all(ani.join("out/classify/ani_screen")).unwrap();
        fs::write(ani.join("assigned.json"), r#"["bin.1", "bin.77"]"#).unwrap();
        fs::write(
            ani.join("out/classify/ani_screen/gtdbtk.bac120.ani_summary.tsv"),
            "user_genome\treference_genome\tskani_ani\tskani_af\treference_taxonomy\n\
             bin.1\tGCA_1\t99.9\t0.96\td__Bacteria;s__X y\nbin.77\tGCA_7\t99\t0.9\td__Bacteria;s__Z\n",
        )
        .unwrap();
        let data = root.join("data");
        fs::create_dir_all(&data).unwrap();
        let tools_ = crate::pipeline::Tools {
            env_prefix: Some(env.clone()),
            data,
            cpus: 2,
        };
        let opts = PlaceOptions {
            genomes: genomes.clone(),
            extension: "fa".into(),
            work: root.join("work"),
            max_memory: "6G".into(),
            pplacer_cpus: 1,
            skip_assigned: Some(ani.clone()),
            gplace: None,
            full_tree: false,
        };
        let r = run(&opts, &tools_).unwrap();
        assert_eq!((r.placed, r.skipped, r.gplace_calls), (2, 1, 1));
        let rows = parse_table(&fs::read_to_string(&r.table).unwrap()).unwrap();
        let m: Vec<(&str, &str, &str)> = rows
            .iter()
            .map(|r| {
                (
                    r.genome.as_str(),
                    r.method.as_str(),
                    r.classification.as_str(),
                )
            })
            .collect();
        assert_eq!(
            m,
            [
                ("bin.1", "ani_screen", "d__Bacteria;s__X y"),
                ("bin.12", "not_placed", "Unclassified Bacteria"),
                ("bin.13", "placement", "d__Bacteria;g__G1;s__"),
            ]
        );
        let args = fs::read_to_string(root.join("gplace-args.txt")).unwrap();
        assert!(
            args.starts_with("place --max_memory 6G -m WAG -j 1 -c REF -o "),
            "{args}"
        );
        assert!(
            r.gtdbtk_out
                .join("classify/gtdbtk.bac120.summary.tsv")
                .is_file()
        );
        let rec: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(root.join("work").join(RECORD)).unwrap())
                .unwrap();
        assert_eq!(rec["status"], "done");
        assert_eq!(rec["tools"]["gplace"], "gplace: version 0.1.1");
        assert_eq!(rec["genomes"]["placed"], 2);

        // A rerun in the same folder replaces its own files.
        let r = run(&opts, &tools_).unwrap();
        assert_eq!(r.gplace_calls, 1);

        // Everything assigned: nothing to place, still a table.
        fs::write(
            ani.join("assigned.json"),
            r#"["bin.1", "bin.12", "bin.13"]"#,
        )
        .unwrap();
        fs::write(
            ani.join("out/classify/ani_screen/gtdbtk.bac120.ani_summary.tsv"),
            "user_genome\treference_genome\tskani_ani\tskani_af\treference_taxonomy\n\
             bin.1\tA\t99\t0.9\tt1\nbin.12\tB\t99\t0.9\tt2\nbin.13\tC\t99\t0.9\tt3\n",
        )
        .unwrap();
        let r = run(&opts, &tools_).unwrap();
        assert_eq!((r.placed, r.skipped, r.gplace_calls), (0, 3, 0));
        assert!(r.rows.iter().all(|r| r.method == "ani_screen"));

        // Without --skip-assigned every genome is placed; a bad memory size stops before anything runs.
        let mut o = opts.clone();
        o.skip_assigned = None;
        assert_eq!(run(&o, &tools_).unwrap().placed, 3);
        o.max_memory = "lots".into();
        assert!(run(&o, &tools_).is_err());
        // No gPlace: a clear error.
        fs::remove_file(env.join("bin/gplace")).unwrap();
        let mut o = opts.clone();
        o.skip_assigned = None;
        let e = format!("{:#}", run(&o, &tools_).unwrap_err());
        assert!(
            e.contains("gPlace not found") || e.contains("gplace"),
            "{e}"
        );
    }
}
