//! `prep`, `classify` and `verify`.

use crate::hash::{sha256_file, walk_files};
use crate::manifest::{self, Client, GenomeEntry, Manifest, SketchInfo};
use crate::outputs::{self, BUNDLE_FILES, Outcome, Summary};
use crate::reference;
use crate::rename::{self, InputGenome};
use crate::tools::{self, Runner, ToolVersion};
use anyhow::{Context, Result, bail};
use flate2::{Compression, read::MultiGzDecoder, write::GzEncoder};
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub const PREFIX: &str = "gtdbtk";
/// skani version whose sketch format GTDB-Tk's R232 database uses (GTDB-Tk 2.6.0 changelog).
pub const SKANI_PINNED: &str = "0.3.1";
pub const SKETCH_WARNING: &str = "skani sketches contain real DNA: about 1 in 125 of the genome's 15-mers \
with their positions and 1 in 1,000 of its 21-mers. Names are opaque, but the sketch is not anonymous; \
do not send it for unpublished or sensitive genomes.";

/// Entries `prep --force` may delete from a work folder. Nothing else is touched.
const WORK_ENTRIES: [&str; 14] = [
    "baseline-input",
    "bundle.tar",
    "renamed",
    "contigs",
    "mapping.tsv",
    "gtdbtk",
    "logs",
    "sketch",
    "bundle",
    "replay",
    "replay-bundle",
    "replay-align",
    "baseline",
    "verify.tsv",
];
pub const MAPPING: &str = "mapping.tsv";

#[derive(Debug, Clone)]
pub struct Tools {
    pub env_prefix: Option<PathBuf>,
    pub data: PathBuf,
    pub cpus: usize,
}

impl Tools {
    fn runner(&self, work: &Path) -> Runner {
        Runner {
            env_prefix: self.env_prefix.clone(),
            data: self.data.clone(),
            cpus: self.cpus,
            log_dir: work.join("logs"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct PrepOptions {
    pub genomes: PathBuf,
    pub extension: String,
    pub work: PathBuf,
    /// Copy the bundle here: a folder, or a `.tar` file.
    pub out: Option<PathBuf>,
    pub force: bool,
    pub sketch: Option<PathBuf>,
    pub allow_ref_mismatch: bool,
}

/// Opaque ID column for inputs `prep` left out: they get no ID and are not in the bundle.
pub const NO_ID: &str = "-";

/// One row of the local mapping file (never put in the bundle).
#[derive(Debug, Clone, PartialEq)]
pub struct MapRow {
    /// Opaque ID, or `NO_ID` for an excluded input.
    pub id: String,
    pub gtdbtk_id: String,
    pub source: PathBuf,
    pub source_sha256: String,
    pub contigs: usize,
    /// Why `prep` left this input out (empty or not FASTA); `None` if it is in the bundle.
    pub excluded: Option<String>,
}

/// Inputs in the bundle, and inputs `prep` left out.
pub fn split_mapping(rows: &[MapRow]) -> (Vec<&MapRow>, Vec<&MapRow>) {
    rows.iter().partition(|r| r.excluded.is_none())
}

/// Write a file only its owner can read (0600 on unix): the mapping and contig tables name the
/// user's genomes.
fn write_private(path: &Path, contents: &[u8]) -> Result<()> {
    let mut o = fs::OpenOptions::new();
    o.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        o.mode(0o600);
        let mut f = o
            .open(path)
            .with_context(|| format!("creating {}", path.display()))?;
        // `mode` only applies to new files.
        f.set_permissions(fs::Permissions::from_mode(0o600))?;
        f.write_all(contents)?;
    }
    #[cfg(not(unix))]
    o.open(path)
        .with_context(|| format!("creating {}", path.display()))?
        .write_all(contents)?;
    Ok(())
}

fn one_line(s: &str) -> String {
    s.replace(['\t', '\n', '\r'], " ")
}

pub fn write_mapping(path: &Path, rows: &[MapRow]) -> Result<()> {
    let mut s = String::from("# gtdbprep local mapping: keep on this computer, never send\n");
    s.push_str("opaque_id\tgtdbtk_id\tsource\tsource_sha256\tcontigs\texcluded\n");
    for r in rows {
        s.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\n",
            r.id,
            r.gtdbtk_id,
            r.source.display(),
            r.source_sha256,
            r.contigs,
            r.excluded.as_deref().map(one_line).unwrap_or_default()
        ));
    }
    write_private(path, s.as_bytes())
}

pub fn read_mapping(path: &Path) -> Result<Vec<MapRow>> {
    let text = fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut rows = Vec::new();
    for line in text
        .lines()
        .filter(|l| !l.starts_with('#') && !l.starts_with("opaque_id\t") && !l.is_empty())
    {
        let f: Vec<&str> = line.split('\t').collect();
        if f.len() != 5 && f.len() != 6 {
            bail!("{}: bad line {line:?}", path.display());
        }
        rows.push(MapRow {
            id: f[0].into(),
            gtdbtk_id: f[1].into(),
            source: f[2].into(),
            source_sha256: f[3].into(),
            contigs: f[4].parse()?,
            excluded: f.get(5).filter(|v| !v.is_empty()).map(|v| v.to_string()),
        });
    }
    Ok(rows)
}

/// `--out` must not exist yet (an empty folder is fine), and its parent must exist. Checked
/// before any work starts, and again just before writing.
pub fn check_out(out: &Path) -> Result<()> {
    if out.exists() && !(out.is_dir() && fs::read_dir(out)?.next().is_none()) {
        bail!("--out {} already exists", out.display());
    }
    let parent = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if !parent.is_dir() {
        bail!(
            "--out {}: folder {} does not exist",
            out.display(),
            parent.display()
        );
    }
    Ok(())
}

/// GTDB-Tk 2.8.0 refuses genome paths that contain a space ("unsupported by downstream
/// applications"), and it reads the renamed genomes from the work folder. Input folders may have
/// spaces: only gtdbprep reads them.
pub fn check_no_space(work: &Path) -> Result<()> {
    for form in [
        Some(work.to_path_buf()),
        std::path::absolute(work).ok(),
        fs::canonicalize(work).ok(),
    ]
    .into_iter()
    .flatten()
    {
        if form.to_string_lossy().contains(' ') {
            bail!(
                "work folder {} has a space in its path; GTDB-Tk refuses genome paths with spaces, \
                 so choose a work folder without one",
                form.display()
            );
        }
    }
    Ok(())
}

fn prepare_work(work: &Path, force: bool) -> Result<()> {
    if work.exists() {
        let occupied: Vec<String> = fs::read_dir(work)?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        if !occupied.is_empty() {
            if !force {
                bail!(
                    "work folder {} is not empty; use --force to replace gtdbprep's files in it",
                    work.display()
                );
            }
            for name in occupied {
                if WORK_ENTRIES.contains(&name.as_str()) {
                    let p = work.join(&name);
                    if p.is_dir() {
                        fs::remove_dir_all(&p)?
                    } else {
                        fs::remove_file(&p)?
                    }
                }
            }
        }
    }
    fs::create_dir_all(work)?;
    // The work folder holds the mapping back to the user's genomes: owner only.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(work, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Replace local paths in a command line with placeholders, so the manifest names no folders.
fn redact(cmd: &std::process::Command, subs: &[(&Path, &str)]) -> Vec<String> {
    let mut v = vec![
        Path::new(cmd.get_program())
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
    ];
    for a in cmd.get_args() {
        let mut s = a.to_string_lossy().into_owned();
        for (p, name) in subs {
            s = s.replace(&*p.to_string_lossy(), name);
        }
        v.push(s);
    }
    v
}

fn gzip_file(src: &Path, dest: &Path) -> Result<()> {
    let mut enc = GzEncoder::new(File::create(dest)?, Compression::default());
    io::copy(&mut File::open(src)?, &mut enc)?;
    enc.finish()?;
    Ok(())
}

fn read_maybe_gz(path: &Path) -> Result<Vec<u8>> {
    gunzip_if_gz(fs::read(path).with_context(|| format!("reading {}", path.display()))?)
}

/// A string that must not appear in the bundle.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Needle {
    pub text: String,
    /// Match only as a whole word (not next to a letter, digit or `_`). Used for user and host
    /// names, which are short and would otherwise match inside ordinary words.
    pub word: bool,
}

impl Needle {
    fn found_in(&self, data: &[u8]) -> bool {
        let n = self.text.as_bytes();
        if n.is_empty() || data.len() < n.len() {
            return false;
        }
        let is_word = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        (0..=data.len() - n.len()).any(|i| {
            &data[i..i + n.len()] == n
                && (!self.word
                    || ((i == 0 || !is_word(data[i - 1]))
                        && data.get(i + n.len()).is_none_or(|&b| !is_word(b))))
        })
    }
}

/// The machine's host name (Linux `/proc`, then `/etc/hostname`, then `$HOSTNAME`).
fn hostname() -> Option<String> {
    ["/proc/sys/kernel/hostname", "/etc/hostname"]
        .iter()
        .find_map(|p| fs::read_to_string(p).ok())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// A path as given, made absolute, and with symlinks resolved.
fn path_forms(p: &Path) -> Vec<String> {
    let mut v = vec![p.to_string_lossy().into_owned()];
    if let Ok(a) = std::path::absolute(p) {
        v.push(a.to_string_lossy().into_owned());
    }
    if let Ok(c) = fs::canonicalize(p) {
        v.push(c.to_string_lossy().into_owned());
    }
    v
}

/// Strings that must not appear anywhere in the bundle: genome IDs, file names, contig headers,
/// the given folders and input files (as given, absolute and canonical), `$HOME`, the user name
/// and the host name.
///
/// Names and paths shorter than 6 characters are left out: they would match by chance inside
/// alignments and marker lists. They cannot reach the bundle anyway, because GTDB-Tk only ever
/// sees the renamed files (opaque file names and contig headers), and the manifest's commands
/// have local paths replaced by placeholders. The scan is a second check, not the mechanism.
pub fn leak_needles(
    inputs: &[InputGenome],
    headers: &[Vec<String>],
    paths: &[&Path],
) -> Vec<Needle> {
    let mut set = HashSet::new();
    for g in inputs {
        set.insert(g.gtdbtk_id.clone());
        if let Some(n) = g.path.file_name() {
            set.insert(n.to_string_lossy().into_owned());
        }
        set.extend(path_forms(&g.path));
    }
    for hs in headers {
        for h in hs {
            set.insert(h.clone());
            if let Some(tok) = h.split_whitespace().next() {
                set.insert(tok.to_string());
            }
        }
    }
    for p in paths {
        set.extend(path_forms(p));
    }
    if let Some(home) = std::env::var_os("HOME") {
        set.extend(path_forms(Path::new(&home)));
    }
    let mut v: Vec<Needle> = set
        .into_iter()
        .filter(|s| s.len() >= 6 && s != "/")
        .map(|text| Needle { text, word: false })
        .collect();
    let mut people: HashSet<String> = ["USER", "LOGNAME"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .collect();
    people.extend(hostname());
    v.extend(
        people
            .into_iter()
            .filter(|s| s.len() >= 3)
            .map(|text| Needle { text, word: true }),
    );
    v.sort();
    v.dedup();
    v
}

fn scan_bytes(name: &str, data: &[u8], needles: &[Needle], hits: &mut Vec<(String, String)>) {
    for n in needles {
        if n.found_in(data) {
            hits.push((name.to_string(), n.text.clone()));
        }
    }
}

fn gunzip_if_gz(raw: Vec<u8>) -> Result<Vec<u8>> {
    if raw.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        MultiGzDecoder::new(&raw[..]).read_to_end(&mut out)?;
        Ok(out)
    } else {
        Ok(raw)
    }
}

/// Look for any needle in every bundle file (gzip files are scanned decompressed).
pub fn scan_for_leaks(bundle: &Path, needles: &[Needle]) -> Result<Vec<(String, String)>> {
    let mut hits = Vec::new();
    for rel in walk_files(bundle)? {
        scan_bytes(
            &rel,
            &read_maybe_gz(&bundle.join(&rel))?,
            needles,
            &mut hits,
        );
    }
    Ok(hits)
}

/// The same for a `.tar` or `.tar.gz`: the archive's raw bytes (headers included), then every
/// entry's contents, decompressed if gzipped.
pub fn scan_archive_for_leaks(archive: &Path, needles: &[Needle]) -> Result<Vec<(String, String)>> {
    let tar_bytes = read_maybe_gz(archive)?;
    let mut hits = Vec::new();
    scan_bytes("(tar headers)", &tar_bytes, needles, &mut hits);
    let mut a = tar::Archive::new(&tar_bytes[..]);
    for entry in a.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().into_owned();
        scan_bytes(&name, entry.path_bytes().as_ref(), needles, &mut hits);
        let mut data = Vec::new();
        entry.read_to_end(&mut data)?;
        scan_bytes(&name, &gunzip_if_gz(data)?, needles, &mut hits);
    }
    Ok(hits)
}

fn leak_error(hits: &[(String, String)]) -> anyhow::Error {
    let list: Vec<String> = hits.iter().map(|(f, n)| format!("{n:?} in {f}")).collect();
    anyhow::anyhow!("the bundle would reveal local names: {}", list.join(", "))
}

/// Copy a bundle file. Gzip files are decompressed and written again with a fixed header (no
/// file name, mtime 0), so GTDB-Tk's own gzip header (source file name, wall-clock time) does not
/// leave the laptop.
fn copy_into_bundle(src: &Path, dest: &Path) -> Result<()> {
    let raw = fs::read(src)?;
    if raw.starts_with(&[0x1f, 0x8b]) {
        let plain = gunzip_if_gz(raw)?;
        let mut enc = flate2::GzBuilder::new()
            .mtime(0)
            .write(File::create(dest)?, Compression::default());
        enc.write_all(&plain)?;
        enc.finish()?;
    } else {
        fs::write(dest, raw)?;
    }
    Ok(())
}

/// Write a tar with fixed metadata (mtime 0, mode 644, owner 0) in sorted order, so the same
/// bundle always gives the same bytes.
pub fn write_tar(bundle: &Path, dest: &Path) -> Result<()> {
    let mut b = tar::Builder::new(File::create(dest)?);
    for rel in walk_files(bundle)? {
        let data = fs::read(bundle.join(&rel))?;
        let mut h = tar::Header::new_ustar();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        h.set_mtime(0);
        h.set_uid(0);
        h.set_gid(0);
        h.set_entry_type(tar::EntryType::Regular);
        b.append_data(&mut h, &rel, &data[..])?;
    }
    b.into_inner()?.flush()?;
    Ok(())
}

fn link_or_copy(src: &Path, dest: &Path) -> Result<()> {
    #[cfg(unix)]
    if std::os::unix::fs::symlink(src, dest).is_ok() {
        return Ok(());
    }
    fs::copy(src, dest).with_context(|| format!("copying {}", src.display()))?;
    Ok(())
}

fn copy_dir(src: &Path, dest: &Path) -> Result<()> {
    for rel in walk_files(src)? {
        let to = dest.join(&rel);
        fs::create_dir_all(to.parent().unwrap())?;
        fs::copy(src.join(&rel), &to)?;
    }
    Ok(())
}

#[derive(Debug)]
pub struct PrepResult {
    pub bundle: PathBuf,
    pub manifest: Manifest,
    pub total_bytes: u64,
    /// Inputs left out (GTDB-Tk genome ID, reason); recorded in the local mapping only.
    pub excluded: Vec<(String, String)>,
}

pub fn prep(opts: &PrepOptions, tools: &Tools) -> Result<PrepResult> {
    let started = manifest::now_rfc3339();
    if let Some(out) = &opts.out {
        check_out(out)?;
    }
    let reference = reference::check(&tools.data)?;
    for (name, f) in &reference.folders {
        if f.matches_gtdbtk == Some(false) && !f.enforced {
            eprintln!(
                "gtdbprep: note: {name}/ differs from GTDB-Tk's REF_HASHES (fine: only used to check \
                 that genome IDs don't clash with GTDB accessions, and opaque IDs never do)"
            );
        } else if f.matches_gtdbtk == Some(false) {
            let msg = format!(
                "reference folder {name}/ has GTDB-Tk hash {} but GTDB-Tk {} expects {}",
                f.gtdbtk_sha1,
                reference.gtdb_release,
                f.expected_sha1.as_deref().unwrap_or("?")
            );
            if opts.allow_ref_mismatch {
                eprintln!("gtdbprep: warning: {msg}");
            } else {
                bail!("{msg} (use --allow-ref-mismatch to go on anyway)");
            }
        }
    }
    let skani_version = match &opts.sketch {
        Some(skani) => {
            let v = tools::skani_version(skani)
                .with_context(|| format!("running {} --version", skani.display()))?;
            if v != SKANI_PINNED {
                bail!(
                    "{} is skani {v}; sketches must be skani {SKANI_PINNED} to match GTDB-Tk's R232 database format",
                    skani.display()
                );
            }
            Some(v)
        }
        None => None,
    };

    let work = &opts.work;
    check_no_space(work)?;
    prepare_work(work, opts.force)?;
    let work = &fs::canonicalize(work)?;
    let runner = tools.runner(work);
    let inputs = rename::list_genomes(&opts.genomes, &opts.extension)?;

    // 1. Rename genomes and contigs.
    let renamed = work.join("renamed");
    let contigs_dir = work.join("contigs");
    fs::create_dir_all(&renamed)?;
    fs::create_dir_all(&contigs_dir)?;
    let (mut rows, all_headers) = rename_inputs(&inputs, &renamed, &contigs_dir)?;
    // From here on, list genomes by opaque ID so the bundle does not reveal the original order.
    rows.sort_by(|a, b| a.id.cmp(&b.id));
    write_mapping(&work.join(MAPPING), &rows)?;
    let (kept, left_out) = split_mapping(&rows);
    let excluded_inputs: Vec<(String, String)> = left_out
        .iter()
        .map(|r| (r.gtdbtk_id.clone(), r.excluded.clone().unwrap_or_default()))
        .collect();
    let rows: Vec<MapRow> = kept.into_iter().cloned().collect();
    eprintln!(
        "gtdbprep: renamed {} genome(s), left out {}; mapping kept in {}",
        rows.len(),
        excluded_inputs.len(),
        work.join(MAPPING).display()
    );
    if rows.is_empty() {
        bail!("no usable genomes: every input file is empty or not FASTA");
    }

    // 2. GTDB-Tk identify + align --skip_gtdb_refs.
    let gt = work.join("gtdbtk");
    let subs: [(&Path, &str); 2] = [
        (work.as_path(), "<work>"),
        (tools.data.as_path(), "<GTDBTK_DATA_PATH>"),
    ];
    let mut commands = Vec::new();
    let mut identify = runner.command("gtdbtk");
    identify
        .arg("identify")
        .arg("--genome_dir")
        .arg(&renamed)
        .args([
            "-x",
            "fna",
            "--prefix",
            PREFIX,
            "--cpus",
            &tools.cpus.to_string(),
            "--out_dir",
        ])
        .arg(&gt);
    commands.push(redact(&identify, &subs));
    runner.run("identify", identify)?;
    let mut align = runner.command("gtdbtk");
    align
        .arg("align")
        .arg("--identify_dir")
        .arg(&gt)
        .args([
            "--skip_gtdb_refs",
            "--prefix",
            PREFIX,
            "--cpus",
            &tools.cpus.to_string(),
            "--out_dir",
        ])
        .arg(&gt);
    commands.push(redact(&align, &subs));
    runner.run("align", align)?;

    // 3. Assemble the bundle.
    let bundle = work.join("bundle");
    fs::create_dir_all(&bundle)?;
    for t in BUNDLE_FILES {
        let rel = outputs::bundle_path(t, PREFIX);
        let src = gt.join(&rel);
        let dest = bundle.join(&rel);
        if src.is_file() {
            fs::create_dir_all(dest.parent().unwrap())?;
            copy_into_bundle(&src, &dest)?;
        } else if let Some(plain) = rel
            .strip_suffix(".gz")
            .map(|p| gt.join(p))
            .filter(|p| p.is_file())
        {
            fs::create_dir_all(dest.parent().unwrap())?;
            gzip_file(&plain, &dest)?;
        }
    }

    // 4. Optional skani sketches, named by opaque ID only.
    let sketches = if let (Some(skani), Some(version)) = (&opts.sketch, skani_version) {
        let sk = work.join("sketch");
        let mut cmd = std::process::Command::new(skani);
        cmd.current_dir(&renamed)
            .args([
                "sketch",
                "--separate-sketches",
                "-t",
                &tools.cpus.to_string(),
                "-o",
            ])
            .arg(&sk)
            .args(rows.iter().map(|r| format!("{}.fna", r.id)));
        commands.push(redact(&cmd, &subs));
        runner.run("skani-sketch", cmd)?;
        fs::create_dir_all(bundle.join("sketches"))?;
        for r in &rows {
            let name = format!("{}.fna.sketch", r.id);
            fs::copy(sk.join(&name), bundle.join("sketches").join(&name))
                .with_context(|| format!("skani wrote no {name}"))?;
        }
        eprintln!("gtdbprep: WARNING: {SKETCH_WARNING}");
        Some(SketchInfo {
            skani_version: version,
            c: 125,
            marker_c: 1000,
            k: 15,
            warning: SKETCH_WARNING.into(),
        })
    } else {
        None
    };

    // 5. Per-genome facts for the manifest, read from GTDB-Tk's files.
    let read = |rel: &str| {
        fs::read_to_string(gt.join(outputs::bundle_path(rel, PREFIX))).unwrap_or_default()
    };
    let bac = outputs::single_copy_counts(&read("identify/{p}.bac120.markers_summary.tsv"))
        .unwrap_or_default();
    let ar = outputs::single_copy_counts(&read("identify/{p}.ar53.markers_summary.tsv"))
        .unwrap_or_default();
    let tln = outputs::two_columns(&read("identify/{p}.translation_table_summary.tsv"));
    let mut excluded = BTreeMap::new();
    for rel in [
        "identify/{p}.failed_genomes.tsv",
        "align/{p}.bac120.filtered.tsv",
        "align/{p}.ar53.filtered.tsv",
        "align/intermediate_results/{p}.align.failed.tsv",
        "align/intermediate_results/{p}.identify.failed.tsv",
    ] {
        for (g, why) in outputs::two_columns(&read(rel)) {
            excluded.entry(g).or_insert(why);
        }
    }
    let mut in_msa = BTreeMap::new();
    for domain in ["bac120", "ar53"] {
        let p = bundle.join(format!("align/{PREFIX}.{domain}.user_msa.fasta.gz"));
        if p.is_file() {
            for name in outputs::fasta_names(&p)? {
                in_msa.insert(name, domain.to_string());
            }
        }
    }
    let mut genomes = Vec::new();
    for r in &rows {
        let (b, a) = (bac.get(&r.id).copied(), ar.get(&r.id).copied());
        genomes.push(GenomeEntry {
            id: r.id.clone(),
            fasta_sha256: sha256_file(&renamed.join(format!("{}.fna", r.id)))?.0,
            contigs: r.contigs,
            domain: b.map(|b| outputs::domain_call(b, a.unwrap_or(0)).to_string()),
            bac120_single_copy: b,
            ar53_single_copy: a,
            translation_table: tln.get(&r.id).and_then(|v| v.parse().ok()),
            in_msa: in_msa.get(&r.id).cloned(),
            excluded: excluded.get(&r.id).cloned(),
        });
    }

    // 6. Manifest.
    let mut tool_versions = tools::laptop_versions(&runner);
    if let Some(s) = &sketches {
        tool_versions.insert(
            "skani".into(),
            ToolVersion {
                version: Some(s.skani_version.clone()),
                reported: Some(format!("skani {}", s.skani_version)),
            },
        );
    }
    let privacy = privacy_text(sketches.is_some());
    let mut m = Manifest {
        format: manifest::FORMAT.into(),
        client: Client {
            name: env!("CARGO_PKG_NAME").into(),
            version: env!("CARGO_PKG_VERSION").into(),
        },
        created: String::new(),
        started,
        gtdb_release: reference.gtdb_release.clone(),
        tools: tool_versions,
        commands,
        reference,
        prefix: PREFIX.into(),
        genomes,
        inputs_excluded: excluded_inputs.len(),
        sketches,
        files: manifest::hash_bundle_files(&bundle)?,
        privacy,
    };
    m.created = manifest::now_rfc3339();
    m.write(&bundle)?;
    let total = m.total_bytes(&bundle);

    // 7. Nothing identifying may be in the bundle, manifest included. On a hit the bundle is
    // deleted, so it cannot be sent by mistake.
    let needles = leak_needles(
        &inputs,
        &all_headers,
        &[work, &opts.work, &tools.data, &opts.genomes],
    );
    let hits = scan_for_leaks(&bundle, &needles)?;
    if !hits.is_empty() {
        fs::remove_dir_all(&bundle)?;
        return Err(leak_error(&hits).context("bundle deleted"));
    }

    if let Some(out) = &opts.out {
        write_out(&bundle, out, work, &needles)?;
    }
    Ok(PrepResult {
        bundle,
        manifest: m,
        total_bytes: total,
        excluded: excluded_inputs,
    })
}

/// Rename every input into `renamed/` and write its contig table. Inputs that are empty or not
/// FASTA are left out with a warning and recorded with `NO_ID` and the reason; any other error
/// stops prep. Returns the mapping rows (input order) and the original headers of renamed inputs.
fn rename_inputs(
    inputs: &[InputGenome],
    renamed: &Path,
    contigs_dir: &Path,
) -> Result<(Vec<MapRow>, Vec<Vec<String>>)> {
    let mut taken = HashSet::new();
    let mut rows = Vec::new();
    let mut all_headers = Vec::new();
    for g in inputs {
        let id = rename::random_id(&taken)?;
        taken.insert(id.clone());
        let dest = renamed.join(format!("{id}.fna"));
        let source = fs::canonicalize(&g.path)?;
        let source_sha256 = sha256_file(&g.path)?.0;
        let headers = match rename::rename_file(&g.path, &dest, &id) {
            Ok(h) => h,
            Err(e) => {
                let Some(nf) = e.downcast_ref::<rename::NotFasta>() else {
                    return Err(e);
                };
                let _ = fs::remove_file(&dest);
                eprintln!("gtdbprep: warning: leaving out {}: {nf}", g.path.display());
                rows.push(MapRow {
                    id: NO_ID.into(),
                    gtdbtk_id: g.gtdbtk_id.clone(),
                    source,
                    source_sha256,
                    contigs: 0,
                    excluded: Some(nf.to_string()),
                });
                continue;
            }
        };
        let mut t = String::from("opaque_contig\toriginal_header\n");
        for (i, h) in headers.iter().enumerate() {
            t.push_str(&format!("{}\t{h}\n", rename::contig_name(&id, i + 1)));
        }
        write_private(&contigs_dir.join(format!("{id}.tsv")), t.as_bytes())?;
        rows.push(MapRow {
            id,
            gtdbtk_id: g.gtdbtk_id.clone(),
            source,
            source_sha256,
            contigs: headers.len(),
            excluded: None,
        });
        all_headers.push(headers);
    }
    Ok((rows, all_headers))
}

/// The bundle's own privacy statements (written into `manifest.json`).
pub fn privacy_text(with_sketches: bool) -> Vec<String> {
    let dna = if with_sketches {
        "DNA sequences (except the sparse k-mer samples in the skani sketches, see below)"
    } else {
        "DNA sequences"
    };
    let mut v = vec![
        "Contains: masked marker-protein alignments (bac120: 5,010 columns, ar53: 4,369), marker counts, \
         genetic code, domain call and contig count per genome, tool versions and reference hashes."
            .to_string(),
        format!(
            "Does not contain: {dna}, gene or protein sequences outside the markers, file names, contig names, \
             local paths, or the mapping from opaque IDs to genomes."
        ),
        "The alignment says what the organism is (that is its purpose) and, for novel lineages, gives partial \
         sequences of conserved housekeeping proteins. fasta_sha256 lets someone who already has a genome \
         confirm it is in the bundle. The contig count per genome is a mild identifying feature: it can \
         help match the bundle to a known assembly."
            .to_string(),
    ];
    if with_sketches {
        v.push(SKETCH_WARNING.to_string());
    }
    v
}

/// Copy the bundle to `--out` (folder, `.tar` or `.tar.gz`) and scan what was written. On a hit
/// the copy is deleted.
fn write_out(bundle: &Path, out: &Path, work: &Path, needles: &[Needle]) -> Result<()> {
    check_out(out)?;
    let name = out.file_name().unwrap_or_default().to_string_lossy();
    let archive = name.ends_with(".tar.gz") || name.ends_with(".tgz") || name.ends_with(".tar");
    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let tmp = work.join("bundle.tar");
        write_tar(bundle, &tmp)?;
        // Fixed gzip header (no name, mtime 0), so the bytes depend only on the tar and the
        // compressor.
        let mut enc = flate2::GzBuilder::new()
            .mtime(0)
            .write(File::create(out)?, Compression::best());
        io::copy(&mut File::open(&tmp)?, &mut enc)?;
        enc.finish()?;
        fs::remove_file(&tmp)?;
    } else if name.ends_with(".tar") {
        write_tar(bundle, out)?;
    } else {
        copy_dir(bundle, out)?;
    }
    let hits = if archive {
        scan_archive_for_leaks(out, needles)?
    } else {
        scan_for_leaks(out, needles)?
    };
    if !hits.is_empty() {
        if archive {
            fs::remove_file(out)?;
        } else {
            // `out` was empty or absent before (check_out): remove everything we put there.
            for e in fs::read_dir(out)? {
                let p = e?.path();
                if p.is_dir() {
                    fs::remove_dir_all(&p)?
                } else {
                    fs::remove_file(&p)?
                }
            }
        }
        return Err(leak_error(&hits).context(format!("{} deleted", out.display())));
    }
    Ok(())
}

pub fn size_report(r: &PrepResult) -> String {
    let n = r.manifest.genomes.len().max(1) as f64;
    let sk = r.manifest.sketch_bytes();
    let mut s = format!(
        "bundle: {} genome(s), {} bytes ({:.1} KB per genome)",
        r.manifest.genomes.len(),
        r.total_bytes,
        r.total_bytes as f64 / n / 1000.0
    );
    if sk > 0 {
        let place = r.total_bytes - sk;
        s.push_str(&format!(
            "; without sketches {} bytes ({:.1} KB per genome); sketches {:.1} KB per genome",
            place,
            place as f64 / n / 1000.0,
            sk as f64 / n / 1000.0
        ));
    }
    s
}

/// Open a bundle (folder or `.tar`) for reading; tar files are unpacked under `work`.
fn open_bundle(bundle: &Path, work: &Path) -> Result<PathBuf> {
    if bundle.is_dir() {
        return Ok(fs::canonicalize(bundle)?);
    }
    let dest = work.join("replay-bundle");
    if dest.exists() {
        fs::remove_dir_all(&dest)?;
    }
    fs::create_dir_all(&dest)?;
    let raw: Box<dyn Read> =
        Box::new(File::open(bundle).with_context(|| format!("opening {}", bundle.display()))?);
    let mut magic = [0u8; 2];
    File::open(bundle)?.read_exact(&mut magic)?;
    let reader: Box<dyn Read> = if magic == [0x1f, 0x8b] {
        Box::new(MultiGzDecoder::new(raw))
    } else {
        raw
    };
    tar::Archive::new(reader).unpack(&dest)?;
    Ok(dest)
}

pub struct ClassifyOptions {
    pub bundle: PathBuf,
    pub work: PathBuf,
    pub full_tree: bool,
}

/// Classify a bundle with a local `gtdbtk classify`, then map results back to the original
/// names. Needs the work folder `prep` made (renamed genomes for the ANI screen, and the mapping).
pub fn classify(opts: &ClassifyOptions, tools: &Tools) -> Result<Summary> {
    let work = &fs::canonicalize(&opts.work)
        .with_context(|| format!("work folder {} (run prep first)", opts.work.display()))?;
    check_no_space(work)?;
    let bundle = open_bundle(&opts.bundle, work)?;
    let m = Manifest::read(&bundle)?;
    m.verify_files(&bundle)?;
    let mapping = read_mapping(&work.join(MAPPING))?;
    let (kept, left_out) = split_mapping(&mapping);
    for r in &left_out {
        eprintln!(
            "gtdbprep: {} was left out by prep ({}); no result for it",
            r.gtdbtk_id,
            r.excluded.as_deref().unwrap_or_default()
        );
    }
    if left_out.len() != m.inputs_excluded {
        bail!(
            "the manifest counts {} excluded input(s), the mapping in {} has {}",
            m.inputs_excluded,
            work.display(),
            left_out.len()
        );
    }
    let map: BTreeMap<String, String> = kept
        .iter()
        .map(|r| (r.id.clone(), r.gtdbtk_id.clone()))
        .collect();
    let bundle_ids: HashSet<String> = m.genomes.iter().map(|g| g.id.clone()).collect();
    if bundle_ids != map.keys().cloned().collect() {
        bail!(
            "bundle genomes do not match the mapping in {}",
            work.display()
        );
    }
    let out = work.join("replay");
    for d in [&out, &work.join("replay-align")] {
        if d.exists() {
            fs::remove_dir_all(d)?;
        }
    }
    fs::create_dir_all(&out)?;
    let runner = tools.runner(work);

    // classify_wf runs the ANI screen first and never aligns the genomes it assigns, so their
    // msa_percent and translation_table are N/A. A standalone `classify` would fill them in from
    // the alignment inputs. To give classify_wf's result, run GTDB-Tk's own ANI screen
    // (`ANIScreener.run_aniscreen`, which has no CLI subcommand in 2.8) and drop the assigned
    // genomes from the alignment inputs, as a placement server would.
    let assigned = ani_screen(
        &runner,
        &work.join("renamed"),
        &out.join("ani_screen_only"),
        &m.prefix,
        tools.cpus,
        &bundle_ids,
    )?;
    let align_dir = if assigned.is_empty() {
        bundle.clone()
    } else {
        eprintln!(
            "gtdbprep: {} genome(s) assigned by the ANI screen; leaving them out of placement",
            assigned.len()
        );
        let staged = work.join("replay-align");
        drop_genomes(&bundle, &staged, &m.prefix, &assigned)?;
        staged
    };

    let mut cmd = runner.command("gtdbtk");
    cmd.arg("classify")
        .arg("--genome_dir")
        .arg(work.join("renamed"))
        .args([
            "-x",
            "fna",
            "--prefix",
            &m.prefix,
            "--cpus",
            &tools.cpus.to_string(),
            "--align_dir",
        ])
        .arg(&align_dir)
        .arg("--out_dir")
        .arg(&out);
    if opts.full_tree {
        cmd.arg("--full_tree");
    }
    runner.run("classify", cmd)?;
    // GTDB-Tk must only have read the bundle.
    m.verify_files(&bundle)
        .context("gtdbtk classify changed the bundle")?;
    let summary = Summary::read_dir(&out, &m.prefix)?.map_ids(&map)?;
    fs::write(out.join("summary.original_names.tsv"), summary.to_tsv())?;
    Ok(summary)
}

/// GTDB-Tk's ANI screen, called through the Python that runs `gtdbtk`. Refuses GTDB-Tk versions
/// not in `tools::GTDBTK_SUPPORTED` (the call uses an internal API) and IDs it was not given.
/// Writes GTDB-Tk's usual `classify/ani_screen/` files under `out` and the assigned genome IDs
/// to `result`.
const ANI_SCREEN_PY: &str = r#"
import json, os, sys
genome_dir, out_dir, prefix, cpus, result, supported = sys.argv[1:7]
import gtdbtk
if gtdbtk.__version__ not in supported.split(','):
    sys.exit('gtdbprep: GTDB-Tk %s is not supported for the ANI-screen call (supported: %s)'
             % (gtdbtk.__version__, supported))
from gtdbtk.ani_screen import ANIScreener
genomes = {f[:-len('.fna')]: os.path.join(genome_dir, f)
           for f in sorted(os.listdir(genome_dir)) if f.endswith('.fna')}
classified, _, _ = ANIScreener(int(cpus)).run_aniscreen(genomes=genomes, out_dir=out_dir, prefix=prefix)
assigned = sorted({g for d in classified.values() for g in d})
unknown = set(assigned) - set(genomes)
if unknown:
    sys.exit('gtdbprep: the ANI screen returned genomes it was not given: %s' % sorted(unknown))
with open(result, 'w') as fh:
    json.dump(assigned, fh)
"#;

fn ani_screen(
    runner: &Runner,
    genome_dir: &Path,
    out: &Path,
    prefix: &str,
    cpus: usize,
    known: &HashSet<String>,
) -> Result<HashSet<String>> {
    fs::create_dir_all(out)?;
    let result = out.join("assigned.json");
    let python = runner.gtdbtk_python()?;
    let mut cmd = runner.command_at(&python);
    cmd.current_dir(out)
        .args(["-c", ANI_SCREEN_PY])
        .arg(genome_dir)
        .arg(out)
        .args([prefix, &cpus.to_string()])
        .arg(&result)
        .arg(tools::GTDBTK_SUPPORTED.join(","));
    runner.run("ani_screen", cmd)?;
    let ids: HashSet<String> = serde_json::from_str::<Vec<String>>(&fs::read_to_string(&result)?)?
        .into_iter()
        .collect();
    let mut unknown: Vec<&String> = ids.difference(known).collect();
    if !unknown.is_empty() {
        unknown.sort();
        bail!("the ANI screen assigned genomes that are not in the bundle: {unknown:?}");
    }
    Ok(ids)
}

/// Copy a bundle's GTDB-Tk files to `dest` without the given genomes: their rows are removed from
/// every TSV and their records from every alignment. This is how the align folder looks after
/// classify_wf, which never runs identify/align on genomes the ANI screen assigned.
pub fn drop_genomes(
    bundle: &Path,
    dest: &Path,
    prefix: &str,
    drop: &HashSet<String>,
) -> Result<()> {
    for t in BUNDLE_FILES {
        let rel = outputs::bundle_path(t, prefix);
        let src = bundle.join(&rel);
        if !src.is_file() {
            continue;
        }
        let to = dest.join(&rel);
        fs::create_dir_all(to.parent().unwrap())?;
        let text = String::from_utf8(read_maybe_gz(&src)?).context("bundle file is not text")?;
        let mut kept = String::new();
        if rel.ends_with(".fasta.gz") {
            let mut keep = true;
            for line in text.split_inclusive('\n') {
                if let Some(h) = line.strip_prefix('>') {
                    keep = !drop.contains(h.split_whitespace().next().unwrap_or_default());
                }
                if keep {
                    kept.push_str(line);
                }
            }
            let mut enc = GzEncoder::new(File::create(&to)?, Compression::default());
            enc.write_all(kept.as_bytes())?;
            enc.finish()?;
        } else {
            for line in text.split_inclusive('\n') {
                if !drop.contains(line.split('\t').next().unwrap_or_default().trim_end()) {
                    kept.push_str(line);
                }
            }
            fs::write(&to, kept)?;
        }
    }
    Ok(())
}

pub struct VerifyOptions {
    pub prep: PrepOptions,
    pub full_tree: bool,
}

pub struct VerifyReport {
    pub prep: PrepResult,
    pub outcomes: Vec<(String, Outcome)>,
}

impl VerifyReport {
    /// At least one genome was compared, and every compared genome is identical. Inputs `prep`
    /// left out are reported but are not differences.
    pub fn all_identical(&self) -> bool {
        all_identical(&self.outcomes)
    }
}

pub fn all_identical(outcomes: &[(String, Outcome)]) -> bool {
    outcomes.iter().any(|(_, o)| *o == Outcome::Identical)
        && outcomes
            .iter()
            .all(|(_, o)| matches!(o, Outcome::Identical | Outcome::ExcludedByPrep(_)))
}

/// prep → classify from the bundle → plain `classify_wf` on the original files → compare.
pub fn verify(opts: &VerifyOptions, tools: &Tools) -> Result<VerifyReport> {
    let prep_result = prep(&opts.prep, tools)?;
    let work = fs::canonicalize(&opts.prep.work)?;
    let from_bundle = classify(
        &ClassifyOptions {
            bundle: prep_result.bundle.clone(),
            work: work.clone(),
            full_tree: opts.full_tree,
        },
        tools,
    )?;
    let runner = tools.runner(&work);
    let base = work.join("baseline");
    // classify_wf gets the same inputs prep used (same file names, so the same GTDB-Tk genome
    // IDs).
    let mapping = read_mapping(&work.join(MAPPING))?;
    let (kept, left_out) = split_mapping(&mapping);
    // Always a folder of links under the work folder: it leaves out what prep left out, and
    // GTDB-Tk refuses genome paths with a space, which the input folder may have.
    let baseline_input = work.join("baseline-input");
    fs::create_dir_all(&baseline_input)?;
    for r in &kept {
        let name = r.source.file_name().context("input without a file name")?;
        link_or_copy(&r.source, &baseline_input.join(name))?;
    }
    let mut cmd = runner.command("gtdbtk");
    cmd.arg("classify_wf")
        .arg("--genome_dir")
        .arg(&baseline_input)
        .args([
            "-x",
            &opts.prep.extension,
            "--prefix",
            PREFIX,
            "--cpus",
            &tools.cpus.to_string(),
            "--out_dir",
        ])
        .arg(&base);
    if opts.full_tree {
        cmd.arg("--full_tree");
    }
    runner.run("classify_wf", cmd)?;
    let baseline = Summary::read_dir(&base, PREFIX)?;
    let mut outcomes = outputs::compare(&baseline, &from_bundle)?;
    outcomes.extend(left_out.iter().map(|r| {
        (
            r.gtdbtk_id.clone(),
            Outcome::ExcludedByPrep(r.excluded.clone().unwrap_or_default()),
        )
    }));
    let mut t = String::from("genome\tresult\tdetails\n");
    for (g, o) in &outcomes {
        t.push_str(&format!("{g}\t{}\n", outcome_text(o).replace('\n', " ")));
    }
    fs::write(work.join("verify.tsv"), t)?;
    Ok(VerifyReport {
        prep: prep_result,
        outcomes,
    })
}

pub fn outcome_text(o: &Outcome) -> String {
    match o {
        Outcome::Identical => "identical\t".into(),
        Outcome::MissingFromBundleRun => "different\tmissing from the bundle run".into(),
        Outcome::MissingFromBaseline => "different\tmissing from classify_wf".into(),
        Outcome::ExcludedByPrep(why) => format!("excluded\tleft out by prep: {why}"),
        Outcome::Different(d) => {
            let cols: Vec<String> = d
                .iter()
                .map(|(c, a, b)| format!("{c}: {a:?} vs {b:?}"))
                .collect();
            format!("different\t{}", cols.join("; "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn mode(p: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(p).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn mapping_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let rows = vec![
            MapRow {
                id: "g00".into(),
                gtdbtk_id: "ecoli.fna".into(),
                source: "/x/ecoli.fna.gz".into(),
                source_sha256: "ab".into(),
                contigs: 3,
                excluded: None,
            },
            MapRow {
                id: NO_ID.into(),
                gtdbtk_id: "empty.fna".into(),
                source: "/x/empty.fna.gz".into(),
                source_sha256: "cd".into(),
                contigs: 0,
                excluded: Some("no FASTA headers found".into()),
            },
        ];
        let p = d.path().join("m.tsv");
        write_mapping(&p, &rows).unwrap();
        assert_eq!(read_mapping(&p).unwrap(), rows);
        #[cfg(unix)]
        assert_eq!(mode(&p), 0o600);
        // Mappings written before the `excluded` column still read.
        fs::write(
            &p,
            "opaque_id\tgtdbtk_id\tsource\tsource_sha256\tcontigs\ng1\ta\t/a\tff\t2\n",
        )
        .unwrap();
        assert_eq!(read_mapping(&p).unwrap()[0].excluded, None);
    }

    #[test]
    fn verify_needs_at_least_one_identical_genome() {
        assert!(!all_identical(&[]));
        assert!(!all_identical(&[(
            "a".into(),
            Outcome::ExcludedByPrep("empty".into())
        )]));
        assert!(all_identical(&[
            ("a".into(), Outcome::ExcludedByPrep("empty".into())),
            ("b".into(), Outcome::Identical),
        ]));
        assert!(!all_identical(&[
            ("b".into(), Outcome::Identical),
            ("c".into(), Outcome::MissingFromBundleRun),
        ]));
    }

    #[test]
    fn out_is_checked_up_front() {
        let d = tempfile::tempdir().unwrap();
        check_out(&d.path().join("new.tar.gz")).unwrap();
        check_out(d.path()).unwrap(); // empty folder
        fs::write(d.path().join("taken.tar"), b"x").unwrap();
        assert!(check_out(&d.path().join("taken.tar")).is_err());
        assert!(check_out(&d.path().join("no/such/dir/b.tar")).is_err());
    }

    #[test]
    fn needles_cover_users_hosts_and_path_forms() {
        let d = tempfile::tempdir().unwrap();
        let real = d.path().join("real-folder");
        fs::create_dir_all(&real).unwrap();
        let link = d.path().join("linked-name");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&real, &link).unwrap();
        #[cfg(not(unix))]
        fs::create_dir_all(&link).unwrap();
        let needles = leak_needles(&[], &[], &[&link]);
        let texts: Vec<&str> = needles.iter().map(|n| n.text.as_str()).collect();
        assert!(texts.contains(&&*link.to_string_lossy()));
        #[cfg(unix)]
        assert!(texts.contains(&&*fs::canonicalize(&real).unwrap().to_string_lossy()));
        if let Ok(user) = std::env::var("USER")
            && user.len() >= 3
        {
            assert!(needles.contains(&Needle {
                text: user,
                word: true
            }));
        }
        if let Some(h) = hostname().filter(|h| h.len() >= 3) {
            assert!(needles.iter().any(|n| n.text == h && n.word));
        }
        // Word needles match whole words only.
        let n = Needle {
            text: "dzul".into(),
            word: true,
        };
        assert!(n.found_in(b"by dzul."));
        assert!(n.found_in(b"dzul"));
        assert!(!n.found_in(b"dzulfikar"));
        assert!(!n.found_in(b"my_dzul"));
    }

    #[test]
    fn leak_scan_finds_names_in_gzip() {
        let d = tempfile::tempdir().unwrap();
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(b">NODE_10248_length_9430\nMKV\n").unwrap();
        fs::write(d.path().join("a.gz"), enc.finish().unwrap()).unwrap();
        fs::write(d.path().join("b.tsv"), b"gabc\t11\n").unwrap();
        let inputs = vec![InputGenome {
            path: "/data/sample_bin65.fna".into(),
            gtdbtk_id: "sample_bin65".into(),
        }];
        let headers = vec![vec![
            "NODE_10248_length_9430 cov=1.5".to_string(),
            "c1".to_string(),
        ]];
        let needles = leak_needles(&inputs, &headers, &[]);
        assert!(
            !needles.iter().any(|n| n.text == "c1"),
            "short names are skipped"
        );
        let hits = scan_for_leaks(d.path(), &needles).unwrap();
        assert_eq!(
            hits,
            vec![("a.gz".to_string(), "NODE_10248_length_9430".to_string())]
        );
        // The same file inside a .tar.gz is found too.
        let tgz = d.path().join("x.tar.gz");
        let mut b = tar::Builder::new(GzEncoder::new(
            File::create(&tgz).unwrap(),
            Compression::default(),
        ));
        let data = fs::read(d.path().join("a.gz")).unwrap();
        let mut h = tar::Header::new_ustar();
        h.set_size(data.len() as u64);
        h.set_mode(0o644);
        b.append_data(&mut h, "align/a.gz", &data[..]).unwrap();
        b.into_inner().unwrap().finish().unwrap();
        let hits = scan_archive_for_leaks(&tgz, &needles).unwrap();
        assert_eq!(
            hits,
            vec![(
                "align/a.gz".to_string(),
                "NODE_10248_length_9430".to_string()
            )]
        );
    }

    #[test]
    fn bundle_gzip_headers_are_rewritten() {
        let d = tempfile::tempdir().unwrap();
        let src = d.path().join("in.gz");
        let mut enc = flate2::GzBuilder::new()
            .filename("/home/someone/secret.fasta")
            .mtime(1_700_000_000)
            .write(File::create(&src).unwrap(), Compression::default());
        enc.write_all(b">ga\nMKV\n").unwrap();
        enc.finish().unwrap();
        let dest = d.path().join("out.gz");
        copy_into_bundle(&src, &dest).unwrap();
        let raw = fs::read(&dest).unwrap();
        assert_eq!(raw[3] & 0x08, 0, "no FNAME");
        assert_eq!(&raw[4..8], &[0, 0, 0, 0], "mtime 0");
        assert_eq!(read_maybe_gz(&dest).unwrap(), b">ga\nMKV\n");
    }

    #[test]
    fn privacy_text_matches_sketches() {
        let plain = privacy_text(false).join(" ");
        assert!(plain.contains("Does not contain: DNA sequences,"));
        assert!(plain.contains("contig count"));
        let sk = privacy_text(true).join(" ");
        assert!(sk.contains("except the sparse k-mer samples in the skani sketches"));
        assert!(sk.contains(SKETCH_WARNING));
    }

    /// A stand-in GTDB-Tk: a shell script that writes the files `identify` and `align` would,
    /// with GTDB-Tk-style gzip headers (file name and mtime), and a `--version` that prints a
    /// traceback line with a home-folder path.
    #[cfg(unix)]
    fn fake_env(root: &Path) -> (PathBuf, PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let env = root.join("env dir");
        fs::create_dir_all(env.join("bin")).unwrap();
        let script = r#"#!/bin/sh
cmd=$1; shift
if [ "$cmd" = "--version" ]; then
  echo "Traceback (most recent call last):"
  echo "  File \"$HOME/env/lib/python3.13/site-packages/gtdbtk/__init__.py\", line 1"
  exit 0
fi
while [ $# -gt 0 ]; do
  case "$1" in --genome_dir) gd=$2; shift;; --out_dir) od=$2; shift;; esac
  shift
done
if [ "$cmd" = identify ]; then
  mkdir -p "$od/identify"
  s="$od/identify/gtdbtk.bac120.markers_summary.tsv"
  printf 'name\tnumber_unique_genes\tnumber_multiple_genes\tnumber_multiple_unique_genes\tnumber_missing_genes\n' > "$s"
  : > "$od/identify/gtdbtk.translation_table_summary.tsv"
  for f in "$gd"/*.fna; do
    g=$(basename "$f" .fna)
    printf '%s\t118\t0\t0\t2\n' "$g" >> "$s"
    printf '%s\t11\n' "$g" >> "$od/identify/gtdbtk.translation_table_summary.tsv"
  done
fi
if [ "$cmd" = align ]; then
  mkdir -p "$od/align"
  m="$od/align/gtdbtk.bac120.user_msa.fasta"
  for g in $(tail -n +2 "$od/identify/gtdbtk.bac120.markers_summary.tsv" | cut -f1); do
    printf '>%s\nMKV-\n' "$g"
  done > "$m"
  gzip -f "$m"
fi
"#;
        let p = env.join("bin/gtdbtk");
        fs::write(&p, script).unwrap();
        fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        let data = root.join("data dir");
        for (f, text) in [
            ("metadata/metadata.txt", "VERSION_DATA=r232\n"),
            ("markers/m.hmm", "x"),
            ("masks/m.txt", "x"),
            ("taxonomy/gtdb_taxonomy.tsv", "x"),
        ] {
            fs::create_dir_all(data.join(f).parent().unwrap()).unwrap();
            fs::write(data.join(f), text).unwrap();
        }
        (env, data)
    }

    #[cfg(unix)]
    #[test]
    fn prep_end_to_end_with_fake_gtdbtk() {
        let d = tempfile::tempdir().unwrap();
        let (env, data) = fake_env(d.path());
        let genomes = d.path().join("in put");
        fs::create_dir_all(&genomes).unwrap();
        fs::write(genomes.join("sample_alpha.fna"), b">contig_one\nACGT\n").unwrap();
        fs::write(
            genomes.join("sample_beta.fna"),
            b">contig_two x\nGGCC\n>c3\nA\n",
        )
        .unwrap();
        fs::write(genomes.join("empty_one.fna"), b"").unwrap();
        fs::write(genomes.join("plain_seq.fna"), b"ACGTACGT\n").unwrap();
        let tools = Tools {
            env_prefix: Some(env),
            data: data.clone(),
            cpus: 1,
        };
        // GTDB-Tk refuses spaces in genome paths, so a spaced work folder is refused up front.
        let spaced = d.path().join("my work");
        let mut bad = PrepOptions {
            genomes: genomes.clone(),
            extension: "fna".into(),
            work: spaced.clone(),
            out: None,
            force: false,
            sketch: None,
            allow_ref_mismatch: true,
        };
        let e = prep(&bad, &tools).unwrap_err().to_string();
        assert!(e.contains("has a space"), "{e}");
        assert!(!spaced.exists());
        bad.work = d.path().join("my_work");
        let work = bad.work.clone();
        let out = d.path().join("out dir/bundle.tar.gz");
        let mut opts = PrepOptions {
            genomes: genomes.clone(),
            extension: "fna".into(),
            work: work.clone(),
            out: Some(out.clone()),
            force: false,
            sketch: None,
            allow_ref_mismatch: true,
        };

        // --out is checked before anything runs: its folder does not exist yet.
        let e = prep(&opts, &tools).unwrap_err().to_string();
        assert!(e.contains("does not exist"), "{e}");
        assert!(!work.exists(), "nothing ran");
        fs::create_dir_all(out.parent().unwrap()).unwrap();

        let r = prep(&opts, &tools).unwrap();
        // Bad inputs are left out, counted without names, and listed locally.
        assert_eq!(r.manifest.genomes.len(), 2);
        assert_eq!(r.manifest.inputs_excluded, 2);
        let mut left: Vec<&str> = r.excluded.iter().map(|(g, _)| g.as_str()).collect();
        left.sort();
        assert_eq!(left, vec!["empty_one", "plain_seq"]);
        let mapping = read_mapping(&work.join(MAPPING)).unwrap();
        assert_eq!(mapping.iter().filter(|m| m.id == NO_ID).count(), 2);
        let manifest_text = fs::read_to_string(r.bundle.join("manifest.json")).unwrap();
        for name in ["empty_one", "plain_seq", "sample_alpha", "contig_one"] {
            assert!(!manifest_text.contains(name), "{name} in manifest");
        }
        // The traceback line with $HOME is not kept in the manifest.
        let home = std::env::var("HOME").unwrap();
        assert!(!manifest_text.contains(&home));
        assert!(
            !r.manifest.tools["gtdbtk"]
                .reported
                .as_deref()
                .unwrap_or_default()
                .contains('/')
        );
        // GTDB-Tk's gzip header (file name, mtime) is replaced.
        let gt_msa = fs::read(work.join("gtdbtk/align/gtdbtk.bac120.user_msa.fasta.gz")).unwrap();
        assert_ne!(
            gt_msa[3] & 0x08,
            0,
            "the fake writes FNAME, so the check means something"
        );
        let msa = fs::read(r.bundle.join("align/gtdbtk.bac120.user_msa.fasta.gz")).unwrap();
        assert_eq!(msa[3] & 0x08, 0);
        assert_eq!(&msa[4..8], &[0, 0, 0, 0]);
        r.manifest.verify_files(&r.bundle).unwrap();
        // Local files are owner-only.
        assert_eq!(mode(&work), 0o700);
        assert_eq!(mode(&work.join(MAPPING)), 0o600);
        for e in fs::read_dir(work.join("contigs")).unwrap() {
            assert_eq!(mode(&e.unwrap().path()), 0o600);
        }
        assert!(out.is_file());

        // A local name that reaches only the manifest (here through a reference file name)
        // is caught, and the bundle is deleted.
        fs::write(data.join("masks/sample_alpha.txt"), b"x").unwrap();
        opts.force = true;
        opts.out = None;
        let e = format!("{:#}", prep(&opts, &tools).unwrap_err());
        assert!(
            e.contains("manifest.json") && e.contains("sample_alpha"),
            "{e}"
        );
        assert!(!work.join("bundle").exists());
    }

    #[cfg(unix)]
    fn fake_python(root: &Path, version: &str, system_python: &str) -> Runner {
        use std::os::unix::fs::PermissionsExt;
        let pkg = root.join("pkg/gtdbtk");
        fs::create_dir_all(&pkg).unwrap();
        fs::write(
            pkg.join("__init__.py"),
            format!("__version__ = '{version}'\n"),
        )
        .unwrap();
        // Assigns every genome it is given, plus one it was not given if EXTRA is set.
        fs::write(
            pkg.join("ani_screen.py"),
            "import os\nclass ANIScreener:\n    def __init__(self, cpus): pass\n    \
             def run_aniscreen(self, genomes, out_dir, prefix):\n        \
             d = {g: 1 for g in genomes}\n        \
             if os.path.exists(os.path.join(out_dir, 'EXTRA')): d['gnotgiven'] = 1\n        \
             return {'s__x': d}, None, None\n",
        )
        .unwrap();
        let env = root.join("env");
        fs::create_dir_all(env.join("bin")).unwrap();
        let py = env.join("bin/python3");
        fs::write(
            &py,
            format!(
                "#!/bin/sh\nPYTHONPATH='{}' exec '{system_python}' \"$@\"\n",
                root.join("pkg").display()
            ),
        )
        .unwrap();
        fs::set_permissions(&py, fs::Permissions::from_mode(0o755)).unwrap();
        Runner {
            env_prefix: Some(env),
            data: root.into(),
            cpus: 1,
            log_dir: root.join("logs"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn ani_screen_checks_version_and_ids() {
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
        let genomes = d.path().join("renamed");
        fs::create_dir_all(&genomes).unwrap();
        fs::write(genomes.join("ga.fna"), b">ga_c1\nA\n").unwrap();
        fs::write(genomes.join("gb.fna"), b">gb_c1\nA\n").unwrap();
        let both: HashSet<String> = ["ga".to_string(), "gb".to_string()].into();

        let r = fake_python(&d.path().join("old"), "2.7.3", &sys_py);
        let e = format!(
            "{:#}",
            ani_screen(&r, &genomes, &d.path().join("o1"), "p", 1, &both).unwrap_err()
        );
        assert!(e.contains("not supported"), "{e}");

        let r = fake_python(&d.path().join("ok"), "2.8.0", &sys_py);
        let got = ani_screen(&r, &genomes, &d.path().join("o2"), "p", 1, &both).unwrap();
        assert_eq!(got, both);
        // The Rust side refuses IDs that are not in the bundle's manifest.
        let only_a: HashSet<String> = ["ga".to_string()].into();
        let e = format!(
            "{:#}",
            ani_screen(&r, &genomes, &d.path().join("o3"), "p", 1, &only_a).unwrap_err()
        );
        assert!(e.contains("not in the bundle") && e.contains("gb"), "{e}");
        // The Python side refuses IDs it was not given.
        let o4 = d.path().join("o4");
        fs::create_dir_all(&o4).unwrap();
        fs::write(o4.join("EXTRA"), b"").unwrap();
        let e = format!(
            "{:#}",
            ani_screen(&r, &genomes, &o4, "p", 1, &both).unwrap_err()
        );
        assert!(e.contains("not given"), "{e}");
    }

    #[test]
    fn tar_is_reproducible_and_readable() {
        let d = tempfile::tempdir().unwrap();
        let b = d.path().join("b");
        fs::create_dir_all(b.join("align")).unwrap();
        fs::write(b.join("align/x"), b"1").unwrap();
        fs::write(b.join("manifest.json"), b"{}").unwrap();
        write_tar(&b, &d.path().join("1.tar")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(b.join("align/x"), b"1").unwrap();
        write_tar(&b, &d.path().join("2.tar")).unwrap();
        assert_eq!(
            fs::read(d.path().join("1.tar")).unwrap(),
            fs::read(d.path().join("2.tar")).unwrap()
        );
        let work = d.path().join("w");
        fs::create_dir_all(&work).unwrap();
        let opened = open_bundle(&d.path().join("1.tar"), &work).unwrap();
        assert_eq!(fs::read(opened.join("align/x")).unwrap(), b"1");
    }

    #[test]
    fn dropping_genomes_removes_rows_and_records() {
        let d = tempfile::tempdir().unwrap();
        let b = d.path().join("b");
        fs::create_dir_all(b.join("identify")).unwrap();
        fs::create_dir_all(b.join("align")).unwrap();
        fs::write(
            b.join("identify/p.bac120.markers_summary.tsv"),
            "name\tnumber_unique_genes\nga\t1\ngb\t2\n",
        )
        .unwrap();
        fs::write(
            b.join("identify/p.translation_table_summary.tsv"),
            "ga\t11\ngb\t4\n",
        )
        .unwrap();
        let mut enc = GzEncoder::new(Vec::new(), Compression::default());
        enc.write_all(b">ga\nMK-\n>gb\nMV-\n").unwrap();
        fs::write(
            b.join("align/p.bac120.user_msa.fasta.gz"),
            enc.finish().unwrap(),
        )
        .unwrap();
        let out = d.path().join("o");
        drop_genomes(&b, &out, "p", &HashSet::from(["gb".to_string()])).unwrap();
        assert_eq!(
            fs::read_to_string(out.join("identify/p.bac120.markers_summary.tsv")).unwrap(),
            "name\tnumber_unique_genes\nga\t1\n"
        );
        assert_eq!(
            fs::read_to_string(out.join("identify/p.translation_table_summary.tsv")).unwrap(),
            "ga\t11\n"
        );
        assert_eq!(
            read_maybe_gz(&out.join("align/p.bac120.user_msa.fasta.gz")).unwrap(),
            b">ga\nMK-\n"
        );
    }

    #[test]
    fn force_only_removes_own_entries() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("bundle")).unwrap();
        fs::write(d.path().join("mine.txt"), b"keep").unwrap();
        assert!(prepare_work(d.path(), false).is_err());
        prepare_work(d.path(), true).unwrap();
        assert!(!d.path().join("bundle").exists());
        assert!(d.path().join("mine.txt").exists());
        #[cfg(unix)]
        assert_eq!(mode(d.path()), 0o700);
    }
}
