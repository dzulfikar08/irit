//! GTDB-Tk's ANI species screen without the 85 GB skani database.
//!
//! GTDB-Tk 2.7+ starts `classify_wf` with `skani search` against all species representatives,
//! then gives a genome the species of its closest reference (AF >= 0.5) when the ANI reaches that
//! reference's radius. `skani search` compares a genome in full only with the references that
//! pass a marker k-mer screen, and compares them one pair at a time. So the same rows come out
//! of a small database holding just those references. This module:
//!
//! 1. sketches the genomes with skani 0.3.1 (`skani sketch`, the database's parameters);
//! 2. screens them locally against a compact index of every reference's marker k-mers
//!    (`screen.rs`), keeping every reference skani would keep;
//! 3. range-reads only those references' full sketches (`source.rs`) into a cache;
//! 4. writes a skani database of just those references (`skani.rs`);
//! 5. runs GTDB-Tk's own ANI screen code (`ANIScreener`) against it, so the radius and AF rule,
//!    the other-references note and the output files are GTDB-Tk's.
//!
//! See `docs/research/ani-laptop.md`.

pub mod gz;
pub mod screen;
pub mod skani;
pub mod source;

use crate::tools::{self, Runner};
use anyhow::{Context, Result, bail, ensure};
use screen::ScreenIndex;
use sha2::{Digest, Sha256};
use source::Source;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

/// Options for `gtdbprep ani`.
#[derive(Debug, Clone)]
pub struct AniOptions {
    pub genomes: PathBuf,
    pub extension: String,
    /// Screen index folder (`meta.json`, `refs.bin`, `markers.idx`, maybe `gz.idx`).
    pub index: PathBuf,
    /// Where sketches missing from the cache come from: a path or URL of `sketches.db`, a URL
    /// with `{part}`, or `gtdb` (GTDB's archive via the seek index; opt-in). `None` works only
    /// when every sketch needed is cached.
    pub source: Option<String>,
    pub part_size: Option<u64>,
    /// Override the archive URL of the `gtdb` source (a mirror or a local copy).
    pub gtdb_url: Option<String>,
    pub work: PathBuf,
    /// Fetched sketches are kept here (default: [`default_cache`]).
    pub cache: Option<PathBuf>,
    pub skani: PathBuf,
    pub allow_skani_version: bool,
    /// Verification only: run GTDB-Tk's screen against this full skani database folder instead.
    pub full_db: Option<PathBuf>,
    /// Range requests in flight at once when fetching sketches.
    pub fetch_jobs: usize,
    /// Fetch only the references that could give the species call (see
    /// [`screen::could_reach_species`]). Same species calls; fewer "other related references"
    /// and unassigned-hits rows than GTDB-Tk on the full database.
    pub species_only: bool,
}

/// What a run did, per genome and in total.
#[derive(Debug, Clone, Default)]
pub struct AniReport {
    pub genomes: Vec<GenomeReport>,
    /// References screened in (all genomes together).
    pub candidates: usize,
    /// References used: the screened-in ones, or in a species-only run those that could give
    /// the species call.
    pub used: usize,
    pub species_only: bool,
    pub cached: usize,
    pub fetched_bytes: u64,
    pub requests: u64,
    pub assigned: BTreeSet<String>,
    pub out_dir: PathBuf,
    pub seconds_screen: f64,
    pub seconds_fetch: f64,
    pub seconds_gtdbtk: f64,
}

#[derive(Debug, Clone)]
pub struct GenomeReport {
    pub id: String,
    pub markers: usize,
    pub candidates: usize,
    pub used: usize,
}

/// GTDB-Tk's own genome listing (`_genomes_to_process`: IDs, checks), and its ANI screen
/// (`ANIScreener.run_aniscreen`, as `classify_wf` calls it). Mode `list` writes {id: path};
/// mode `screen` writes the IDs the screen assigned.
const ANI_PY: &str = r#"
import json, logging, sys
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
    genome_json, out_dir, prefix, cpus, result = sys.argv[3:8]
    from gtdbtk.ani_screen import ANIScreener
    with open(genome_json) as fh:
        genomes = json.load(fh)
    classified, _, _ = ANIScreener(int(cpus)).run_aniscreen(genomes=genomes, out_dir=out_dir, prefix=prefix)
    assigned = sorted({g for d in classified.values() for g in d})
    with open(result, 'w') as fh:
        json.dump(assigned, fh)
"#;

pub const PREFIX: &str = "gtdbtk";

/// A GTDB-Tk data folder whose `skani/database` is `db` and whose other folders link to `data`.
fn synthetic_data(data: &Path, dest: &Path, db: &Path) -> Result<()> {
    if dest.exists() {
        fs::remove_dir_all(dest)?;
    }
    fs::create_dir_all(dest.join("skani"))?;
    for e in fs::read_dir(data).with_context(|| format!("reading {}", data.display()))? {
        let e = e?;
        if e.file_name() == "skani" {
            continue;
        }
        symlink(&e.path(), &dest.join(e.file_name()))?;
    }
    symlink(db, &dest.join("skani/database"))?;
    for needed in [
        "taxonomy/gtdb_taxonomy.tsv",
        "radii/gtdb_radii.tsv",
        "metadata",
    ] {
        ensure!(
            dest.join(needed).exists(),
            "{} has no {needed}; the ANI screen needs GTDB-Tk's taxonomy, radii and metadata \
             (dblock add gtdbtk-r232-placement-lite has them)",
            data.display()
        );
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(src: &Path, dest: &Path) -> Result<()> {
    let src = fs::canonicalize(src).with_context(|| format!("resolving {}", src.display()))?;
    std::os::unix::fs::symlink(&src, dest)
        .with_context(|| format!("linking {} to {}", dest.display(), src.display()))
}

#[cfg(not(unix))]
fn symlink(_src: &Path, _dest: &Path) -> Result<()> {
    bail!("gtdbprep ani needs a Unix system (GTDB-Tk runs on Linux and macOS)")
}

fn run_py(runner: &Runner, name: &str, args: &[&std::ffi::OsStr], bin_dir: &Path) -> Result<()> {
    let python = runner.gtdbtk_python()?;
    let mut cmd = runner.command_at(&python);
    // Our skani first on PATH: GTDB-Tk calls `skani` by name.
    let mut paths = vec![bin_dir.to_path_buf()];
    if let Some(p) = cmd
        .get_envs()
        .find(|(k, _)| *k == "PATH")
        .and_then(|(_, v)| v)
    {
        paths.extend(std::env::split_paths(p));
    } else if let Some(p) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&p));
    }
    cmd.env("PATH", std::env::join_paths(paths)?);
    cmd.arg("-c").arg(ANI_PY).args(args);
    runner.run(name, cmd)
}

/// The skani version a binary reports (`skani 0.3.1` -> `0.3.1`).
pub fn skani_version(skani: &Path) -> Result<String> {
    let out = Command::new(skani)
        .arg("--version")
        .output()
        .with_context(|| format!("running {} --version", skani.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find_map(|l| l.trim().strip_prefix("skani "))
        .map(|v| v.trim().to_string())
        .with_context(|| format!("{} --version printed no version", skani.display()))
}

/// The per-user sketch cache, shared by all work folders, so a sketch is fetched once per
/// machine: `$XDG_CACHE_HOME/irit/gtdbprep-ani`, else `~/.cache/irit/gtdbprep-ani`, else
/// `<work>/cache`.
pub fn default_cache(work: &Path) -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".cache"))
        });
    match base {
        Some(b) => b.join("irit").join("gtdbprep-ani"),
        None => work.join("cache"),
    }
}

/// Pick the source of full sketches.
/// `missing` sketches of `missing_bytes` (uncompressed) together are not in the cache; the error
/// without a source says how much that is, so the network use can be weighed before allowing it.
pub fn open_source(opts: &AniOptions, ix: &ScreenIndex, missing: usize, missing_bytes: u64) -> Result<Source> {
    let Some(source) = opts.source.as_deref() else {
        let narrower = if opts.species_only {
            String::new()
        } else {
            " --species-only fetches only the references within a species radius, often far fewer.".into()
        };
        bail!(
            "{missing} reference sketch(es), {} uncompressed, are not in the cache; pass --source with \
             an unpacked sketches.db (path or URL), or --source gtdb to range-read about that much \
             from GTDB's own archive (this loads GTDB's server, so it is opt-in; fetched sketches are \
             cached and never fetched twice).{narrower}",
            human_bytes(missing_bytes)
        )
    };
    if source == "gtdb" {
        let g = ix.meta.gzip_source.as_ref().context(
            "this index has no seek index for GTDB's archive; pass --source <sketches.db path or URL>",
        )?;
        let gi = gz::GzIndex::open(&ix.dir.join(&g.checkpoints))?;
        let url = opts.gtdb_url.clone().unwrap_or_else(|| g.url.clone());
        return Ok(Source::gzip(&url, gi, g.member_offset, g.compressed_size));
    }
    if source.contains("{part}") {
        let size = opts
            .part_size
            .context("a source with {part} needs --part-size")?;
        return Source::parts(source, size);
    }
    Ok(Source::plain(source))
}

/// `17 MB`, `1.2 GB`, `512 kB` (decimal units, one decimal below 10).
fn human_bytes(n: u64) -> String {
    let units = [("GB", 1e9), ("MB", 1e6), ("kB", 1e3)];
    for (unit, size) in units {
        let v = n as f64 / size;
        if v >= 1.0 {
            return if v < 10.0 { format!("{v:.1} {unit}") } else { format!("{v:.0} {unit}") };
        }
    }
    format!("{n} B")
}

/// The smallest species radius in GTDB-Tk's `radii/gtdb_radii.tsv` (95.0 in R232).
pub fn min_radius(data: &Path) -> Result<f64> {
    let path = data.join("radii/gtdb_radii.tsv");
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let mut min = f64::INFINITY;
    for (n, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let field = line.split('\t').nth(2);
        match field.map(|f| f.trim().parse::<f64>()) {
            Some(Ok(v)) => min = min.min(v),
            // A header line is fine; anything else is not.
            _ if n == 0 => {}
            _ => bail!("{}: line {}: no radius in the third column", path.display(), n + 1),
        }
    }
    ensure!(min.is_finite(), "{}: no radii", path.display());
    Ok(min)
}

fn read_genome_list(path: &Path) -> Result<BTreeMap<String, PathBuf>> {
    let m: BTreeMap<String, String> = serde_json::from_str(&fs::read_to_string(path)?)?;
    Ok(m.into_iter().map(|(k, v)| (k, PathBuf::from(v))).collect())
}

/// Run the ANI screen. Writes GTDB-Tk's `classify/ani_screen/` files under `<work>/out`.
pub fn run(opts: &AniOptions, tools_: &crate::pipeline::Tools) -> Result<AniReport> {
    let ix = ScreenIndex::open(&opts.index)?;
    let work = &opts.work;
    fs::create_dir_all(work)?;
    let work = fs::canonicalize(work)?;
    crate::pipeline::check_no_space(&work)?;
    let logs = work.join("logs");
    let mut runner = Runner {
        env_prefix: tools_.env_prefix.clone(),
        data: work.join("gtdbtk_data"),
        cpus: tools_.cpus,
        log_dir: logs,
    };

    // skani: the version the database was written with, unless told otherwise.
    let skani_path = if opts.skani.components().count() == 1 {
        tools::which(&opts.skani.to_string_lossy(), runner.path_var())
            .with_context(|| format!("{} not found on PATH", opts.skani.display()))?
    } else {
        fs::canonicalize(&opts.skani).with_context(|| format!("{}", opts.skani.display()))?
    };
    let version = skani_version(&skani_path)?;
    let want = ix.meta.skani_version.clone().unwrap_or_default();
    if version != want && !opts.allow_skani_version {
        bail!(
            "skani {version} at {}, but the database was made with skani {want}; \
             pass --allow-skani-version to go on",
            skani_path.display()
        );
    }
    let bin_dir = work.join("bin");
    fs::create_dir_all(&bin_dir)?;
    let link = bin_dir.join("skani");
    let _ = fs::remove_file(&link);
    symlink(&skani_path, &link)?;

    // The mini database starts empty; GTDB-Tk lists the genomes with its own rules.
    let db_dir = work.join("db");
    fs::create_dir_all(&db_dir)?;
    let full_db = opts.full_db.as_ref().map(fs::canonicalize).transpose()?;
    synthetic_data(
        &tools_.data,
        &runner.data,
        full_db.as_deref().unwrap_or(&db_dir),
    )?;
    let list_json = work.join("genomes.json");
    let genomes_dir =
        fs::canonicalize(&opts.genomes).with_context(|| format!("{}", opts.genomes.display()))?;
    run_py(
        &runner,
        "list_genomes",
        &[
            "list".as_ref(),
            tools::GTDBTK_SUPPORTED.join(",").as_ref(),
            genomes_dir.as_os_str(),
            opts.extension.as_ref(),
            list_json.as_os_str(),
        ],
        &bin_dir,
    )?;
    let genomes = read_genome_list(&list_json)?;
    ensure!(!genomes.is_empty(), "no genomes found");

    let mut report = AniReport::default();
    let t0 = Instant::now();
    if full_db.is_none() {
        // Sketch the genomes with the database's parameters; keep only their marker sets.
        let qdir = work.join("query_sketch");
        if qdir.exists() {
            fs::remove_dir_all(&qdir)?;
        }
        let ql = work.join("query_list.txt");
        let paths: Vec<String> = genomes
            .values()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        fs::write(&ql, paths.join("\n") + "\n")?;
        let mut cmd = Command::new(&skani_path);
        cmd.arg("sketch")
            .arg("-l")
            .arg(&ql)
            .arg("-o")
            .arg(&qdir)
            .args(["-t", &runner.cpus.to_string()])
            .args(["-c", &ix.meta.sketch_c.to_string()])
            .args(["-m", &ix.meta.marker_c.to_string()]);
        runner.run("skani_sketch", cmd)?;
        let mut mf = skani::MarkersFile::open(&qdir.join("markers.bin"))?;
        ensure!(
            mf.params == ix.params,
            "the query sketches' parameters differ from the database's"
        );
        let mut by_path = BTreeMap::new();
        while let Some(m) = mf.next_sketch()? {
            by_path.insert(m.file_name.clone(), m.markers);
        }
        let mut queries = Vec::new();
        for (id, p) in &genomes {
            let m = by_path
                .remove(&*p.to_string_lossy())
                .with_context(|| format!("skani did not sketch {}", p.display()))?;
            queries.push((id.clone(), m));
        }
        let sets: Vec<Vec<u64>> = queries.iter().map(|(_, m)| m.clone()).collect();
        let hits = ix.screen(&sets)?;
        report.seconds_screen = t0.elapsed().as_secs_f64();
        report.species_only = opts.species_only;
        let radius = if opts.species_only {
            Some(min_radius(&tools_.data)?)
        } else {
            None
        };
        let mut screened = BTreeSet::new();
        let mut all = BTreeSet::new();
        // genome, reference, shared markers, query markers, reference markers, used
        let mut rows = Vec::new();
        for ((id, m), h) in queries.iter().zip(&hits) {
            let mut used = 0;
            for hit in h {
                let r_markers = ix.refs[hit.reference as usize].markers as usize;
                let keep = radius.is_none_or(|r| {
                    screen::could_reach_species(hit.shared, m.len(), r_markers, r)
                });
                screened.insert(hit.reference);
                if keep {
                    all.insert(hit.reference);
                    used += 1;
                }
                rows.push((id.clone(), hit.reference, hit.shared, m.len(), r_markers, keep));
            }
            report.genomes.push(GenomeReport {
                id: id.clone(),
                markers: m.len(),
                candidates: h.len(),
                used,
            });
        }
        report.candidates = screened.len();
        report.used = all.len();

        // Fetch what the cache doesn't have, check it against the index, write the database.
        let t1 = Instant::now();
        let cache = opts.cache.clone().unwrap_or_else(|| default_cache(&work));
        let tag = ix
            .meta
            .skani_metadata
            .get("sketches.db_md5")
            .map_or("unknown", |m| &m[..m.len().min(12)]);
        let cache = cache.join(format!(
            "{}-{tag}",
            ix.meta.gtdb_release.as_deref().unwrap_or("db")
        ));
        fs::create_dir_all(&cache)?;
        let path_of = |i: u32| cache.join(format!("{i}.rec"));
        let mut missing = Vec::new();
        for &i in &all {
            let r = ix.refs[i as usize];
            let ok = fs::read(path_of(i))
                .map(|b| Sha256::digest(&b)[..16] == r.sha)
                .unwrap_or(false);
            if ok {
                report.cached += 1;
            } else {
                missing.push(i);
            }
        }
        if !missing.is_empty() {
            let bytes = missing.iter().map(|&i| ix.refs[i as usize].length as u64).sum();
            let src = open_source(opts, &ix, missing.len(), bytes)?;
            eprintln!(
                "gtdbprep: fetching {} reference sketch(es) from {}",
                missing.len(),
                src.describe()
            );
            let src = src.with_jobs(opts.fetch_jobs);
            let ranges: Vec<(u64, u64)> = missing
                .iter()
                .map(|&i| {
                    (
                        ix.refs[i as usize].offset,
                        ix.refs[i as usize].length as u64,
                    )
                })
                .collect();
            // Each sketch is checked and written to the cache as soon as it arrives, so an
            // interrupted run keeps what it fetched.
            let result = src.read_each(&ranges, |k, rec| {
                let i = missing[k];
                ensure!(
                    Sha256::digest(&rec)[..16] == ix.refs[i as usize].sha,
                    "reference {i}: the fetched sketch does not match the index"
                );
                // Unique per process: several runs may share one cache.
                let tmp = path_of(i).with_extension(format!("part{}", std::process::id()));
                fs::write(&tmp, &rec)?;
                fs::rename(&tmp, path_of(i))?;
                Ok(())
            });
            report.fetched_bytes = src.fetched_bytes();
            report.requests = src.request_count();
            result?;
        }
        let mut records = Vec::with_capacity(all.len());
        let mut names = BTreeMap::new();
        for &i in &all {
            let rec = fs::read(path_of(i))?;
            let name = skani::record_name(&rec, &ix.params)?;
            names.insert(i, name.clone());
            records.push((name, rec));
        }
        write_screen_tsv(&work.join("screen.tsv"), &rows, &names, radius)?;
        if db_dir.exists() {
            fs::remove_dir_all(&db_dir)?;
        }
        skani::write_db(&db_dir, &ix.params, &records)?;
        report.seconds_fetch = t1.elapsed().as_secs_f64();
    }

    // GTDB-Tk's own screen against the small database.
    let t2 = Instant::now();
    let out = work.join("out");
    if out.exists() {
        fs::remove_dir_all(&out)?;
    }
    fs::create_dir_all(&out)?;
    let result = work.join("assigned.json");
    runner.data = work.join("gtdbtk_data");
    run_py(
        &runner,
        "ani_screen",
        &[
            "screen".as_ref(),
            tools::GTDBTK_SUPPORTED.join(",").as_ref(),
            list_json.as_os_str(),
            out.as_os_str(),
            PREFIX.as_ref(),
            runner.cpus.to_string().as_ref(),
            result.as_os_str(),
        ],
        &bin_dir,
    )?;
    report.seconds_gtdbtk = t2.elapsed().as_secs_f64();
    let assigned: Vec<String> = serde_json::from_str(&fs::read_to_string(&result)?)?;
    report.assigned = assigned.into_iter().collect();
    report.out_dir = out;
    Ok(report)
}

/// What `ani-index gz-scan` found (`gz.json`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ScanInfo {
    pub url: String,
    pub md5: String,
    pub compressed_size: u64,
    pub gzip_members: u64,
    pub spacing: u64,
    pub target: gz::Member,
    pub checkpoints: usize,
    pub members: Vec<gz::Member>,
}

/// Read a GTDB-Tk archive (a file, or `-` for standard input) once; write `gz-full.idx`
/// (every checkpoint) and `gz.json` into `out`.
pub fn gz_scan(
    input: &Path,
    out: &Path,
    spacing: u64,
    member: &str,
    url: &str,
) -> Result<ScanInfo> {
    fs::create_dir_all(out)?;
    let r = if input == Path::new("-") {
        gz::scan(std::io::stdin().lock(), spacing, member)?
    } else {
        let f = fs::File::open(input).with_context(|| format!("opening {}", input.display()))?;
        gz::scan(
            std::io::BufReader::with_capacity(1 << 20, f),
            spacing,
            member,
        )?
    };
    let target = r
        .target
        .clone()
        .with_context(|| format!("no tar member ending in {member}"))?;
    gz::write_index(&out.join("gz-full.idx"), &r.checkpoints, spacing, 1)?;
    let info = ScanInfo {
        url: url.to_string(),
        md5: r.md5,
        compressed_size: r.compressed_size,
        gzip_members: r.gzip_members,
        spacing,
        target,
        checkpoints: r.checkpoints.len(),
        members: r.members,
    };
    fs::write(
        out.join("gz.json"),
        serde_json::to_string_pretty(&info)? + "\n",
    )?;
    Ok(info)
}

/// Copy every `every`-th checkpoint of a scan into `<index>/gz.idx` and record the archive in
/// the index's `meta.json`.
pub fn gz_attach(index: &Path, scan: &Path, every: usize) -> Result<usize> {
    let info: ScanInfo = serde_json::from_str(&fs::read_to_string(scan.join("gz.json"))?)?;
    let ix = ScreenIndex::open(index)?;
    ensure!(
        info.target.size == ix.meta.sketches_db_size,
        "the archive's sketches.db is {} bytes, the index's {}",
        info.target.size,
        ix.meta.sketches_db_size
    );
    let full = gz::GzIndex::open(&scan.join("gz-full.idx"))?;
    let n = full.copy_every(&index.join("gz.idx"), every)?;
    let mut meta = ix.meta.clone();
    meta.gzip_source = Some(screen::GzipSource {
        url: info.url,
        md5: info.md5,
        compressed_size: info.compressed_size,
        member: info.target.path,
        member_offset: info.target.offset,
        member_size: info.target.size,
        checkpoints: "gz.idx".into(),
        spacing: info.spacing * every.max(1) as u64,
        gzip_members: info.gzip_members,
    });
    screen::write_meta(index, &meta)?;
    Ok(n)
}

/// The genome ID in a skani record name (`.../GCF_000009045.1_genomic.fna.gz` ->
/// `GCF_000009045.1`), as GTDB-Tk reports it.
fn accession(name: &str) -> &str {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    file.split_once("_genomic").map_or(file, |(a, _)| a)
}

/// `<work>/screen.tsv`: every reference that passed the screen for each genome, with the
/// shared-marker count, and whether it was used (fetched into the small database).
fn write_screen_tsv(
    path: &Path,
    rows: &[(String, u32, u64, usize, usize, bool)],
    names: &BTreeMap<u32, String>,
    radius: Option<f64>,
) -> Result<()> {
    let mut s = String::from(
        "genome\treference_index\treference\tshared_markers\tquery_markers\treference_markers\tspecies_min_shared\tused\n",
    );
    for (g, i, shared, qm, rm, used) in rows {
        let need = radius.map_or(String::from("-"), |r| {
            screen::species_min_shared(*qm, *rm, r).to_string()
        });
        s += &format!(
            "{g}\t{i}\t{}\t{shared}\t{qm}\t{rm}\t{need}\t{used}\n",
            names.get(i).map_or("-", |n| accession(n))
        );
    }
    fs::write(path, s).with_context(|| format!("writing {}", path.display()))
}

impl AniReport {
    pub fn to_tsv(&self) -> String {
        let mut s = String::from("genome\tmarkers\tcandidates\tassigned_by_ani");
        s += if self.species_only { "\tused\n" } else { "\n" };
        for g in &self.genomes {
            s += &format!(
                "{}\t{}\t{}\t{}",
                g.id,
                g.markers,
                g.candidates,
                self.assigned.contains(&g.id)
            );
            if self.species_only {
                s += &format!("\t{}", g.used);
            }
            s.push('\n');
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One screened-in reference for a test genome (see `testdata/species_only_hits.tsv`).
    struct Row<'a> {
        genome: &'a str,
        reference: &'a str,
        shared: u64,
        query_markers: usize,
        reference_markers: usize,
        ani: f64,
        af: f64,
        radius: f64,
    }

    fn fixture() -> Vec<Row<'static>> {
        include_str!("testdata/species_only_hits.tsv")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .skip(1)
            .map(|l| {
                let f: Vec<&str> = l.split('\t').collect();
                Row {
                    genome: f[0],
                    reference: f[1],
                    shared: f[2].parse().unwrap(),
                    query_markers: f[3].parse().unwrap(),
                    reference_markers: f[4].parse().unwrap(),
                    ani: f[5].parse().unwrap(),
                    af: f[6].parse().unwrap(),
                    radius: f[7].parse().unwrap(),
                }
            })
            .collect()
    }

    /// GTDB-Tk 2.8.0's species rule (`ANIScreener.sort_skani_ani_screen`): the closest hit with
    /// AF >= 0.5 (by ANI, then AF), assigned only if its ANI reaches its own radius. Returns the
    /// closest reference and whether it was assigned.
    fn gtdbtk_call<'a>(hits: &[&Row<'a>]) -> Option<(&'a str, bool)> {
        let mut pass: Vec<&&Row> = hits
            .iter()
            .filter(|r| r.ani > 0.0 && r.af >= screen::SPECIES_AF)
            .collect();
        pass.sort_by(|a, b| b.ani.total_cmp(&a.ani).then(b.af.total_cmp(&a.af)));
        pass.first().map(|r| (r.reference, r.ani >= r.radius))
    }

    #[test]
    fn species_only_keeps_the_call_on_real_genomes() {
        let rows = fixture();
        let min_radius = rows.iter().map(|r| r.radius).fold(f64::INFINITY, f64::min);
        assert_eq!(min_radius, 95.0);
        let mut genomes: Vec<&str> = rows.iter().map(|r| r.genome).collect();
        genomes.dedup();
        assert_eq!(genomes.len(), 6, "{genomes:?}");
        let (mut all, mut kept_total) = (0, 0);
        for g in genomes {
            let full: Vec<&Row> = rows.iter().filter(|r| r.genome == g).collect();
            let kept: Vec<&Row> = full
                .iter()
                .copied()
                .filter(|r| {
                    screen::could_reach_species(
                        r.shared,
                        r.query_markers,
                        r.reference_markers,
                        min_radius,
                    )
                })
                .collect();
            // Nothing that could be assigned is dropped...
            for r in &full {
                if r.ani >= min_radius && r.af >= screen::SPECIES_AF {
                    assert!(
                        kept.iter().any(|k| k.reference == r.reference),
                        "{g}: dropped {} (ANI {}, AF {})",
                        r.reference,
                        r.ani,
                        r.af
                    );
                }
            }
            // ...so the call is the same.
            assert_eq!(gtdbtk_call(&full), gtdbtk_call(&kept), "{g}");
            // bmut is the one genome no species is assigned to.
            let assigned = gtdbtk_call(&full).is_some_and(|(_, a)| a);
            assert_eq!(assigned, g != "bmut", "{g}");
            all += full.len();
            kept_total += kept.len();
        }
        // And it is worth having: well under the full set is fetched.
        assert!(kept_total * 2 < all, "{kept_total} of {all} kept");
    }

    #[test]
    fn the_call_rule_ignores_references_below_the_smallest_radius() {
        // The closest hit decides, even when a lower one is within its own radius; removing
        // references that can't reach 95 never changes that.
        let r = |reference, ani, af, radius| Row {
            genome: "g",
            reference,
            shared: 0,
            query_markers: 0,
            reference_markers: 0,
            ani,
            af,
            radius,
        };
        let rows = [
            r("a", 96.5, 0.9, 97.0),
            r("b", 96.0, 0.9, 95.0),
            r("c", 93.0, 0.9, 95.0),
            r("d", 99.0, 0.3, 95.0),
        ];
        let full: Vec<&Row> = rows.iter().collect();
        assert_eq!(gtdbtk_call(&full), Some(("a", false)));
        let kept: Vec<&Row> = rows.iter().filter(|r| r.ani >= 95.0).collect();
        assert_eq!(gtdbtk_call(&kept), Some(("a", false)));
        // Dropping "a" by its own radius instead would wrongly assign "b".
        let wrong: Vec<&Row> = rows.iter().filter(|r| r.ani >= r.radius).collect();
        assert_eq!(gtdbtk_call(&wrong), Some(("b", true)));
    }

    #[test]
    fn smallest_radius_and_accessions() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("radii")).unwrap();
        fs::write(
            d.path().join("radii/gtdb_radii.tsv"),
            "s__A\tRS_GCF_1.1\t97.5\ns__B\tGB_GCA_2.1\t95.0\ns__C\tRS_GCF_3.1\t96.0\n",
        )
        .unwrap();
        assert_eq!(min_radius(d.path()).unwrap(), 95.0);
        fs::write(d.path().join("radii/gtdb_radii.tsv"), "s__A\tRS_GCF_1.1\n").unwrap();
        assert!(min_radius(d.path()).is_err());
        assert_eq!(
            accession("/srv/x/GCF/000/009/045/GCF_000009045.1_genomic.fna.gz"),
            "GCF_000009045.1"
        );
        assert_eq!(accession("GCA_1.1"), "GCA_1.1");
    }

    #[test]
    fn sizes_for_the_missing_sketch_message() {
        assert_eq!(human_bytes(17_400_000), "17 MB");
        assert_eq!(human_bytes(1_234_000_000), "1.2 GB");
        assert_eq!(human_bytes(512_000), "512 kB");
        assert_eq!(human_bytes(900), "900 B");
    }
}
