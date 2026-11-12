//! Parameter checks done before any irit binary is started.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Where tools run and where they may write.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// Working folder of every tool; relative parameters are resolved against it.
    pub cwd: PathBuf,
    /// Canonical folder that output paths must resolve inside.
    pub root: PathBuf,
    /// `--allow-outside-root`: skip the root check.
    pub allow_outside_root: bool,
    /// Environment variables that change how the tools resolve paths, captured at startup
    /// (`HOME`, `NXF_HOME`, `DBLOCK_STORE`, `XDG_DATA_HOME`). Also passed to every tool.
    pub vars: BTreeMap<String, OsString>,
    /// Resolved folder of the run records (`--runs-dir`). Tools may never write under it, even
    /// with `allow_outside_root`: the records are what proves what was run.
    pub runs_root: Option<PathBuf>,
}

/// The last five are the database locations Bakta, CheckM2, GTDB-Tk, AMRFinderPlus and mlst read when no
/// path is given; the server resolves them up front and records the value it used.
pub const RESOLUTION_VARS: [&str; 9] =
    ["HOME", "NXF_HOME", "DBLOCK_STORE", "XDG_DATA_HOME", "BAKTA_DB", "CHECKM2DB", "GTDBTK_DATA_PATH", "AMRFINDER_DB", "MLST_DBDIR"];

impl Ctx {
    pub fn new(cwd: PathBuf, root: PathBuf, allow_outside_root: bool, runs_root: &Path) -> Ctx {
        let vars = RESOLUTION_VARS.iter().filter_map(|k| Some((k.to_string(), std::env::var_os(k)?))).collect();
        Ctx { cwd, root, allow_outside_root, vars, runs_root: Some(resolve(runs_root)) }
    }

    pub fn var(&self, k: &str) -> Option<&OsString> {
        self.vars.get(k)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Invalid {
    pub field: String,
    pub message: String,
}

pub type Check<T> = Result<T, Invalid>;

pub fn invalid<T>(field: &str, message: impl Into<String>) -> Check<T> {
    Err(Invalid { field: field.to_string(), message: message.into() })
}

fn non_empty(field: &str, value: &str) -> Check<()> {
    if value.is_empty() {
        return invalid(field, "must not be empty");
    }
    if let Some(c) = value.chars().find(|c| c.is_control()) {
        return invalid(field, format!("must not contain control characters (found U+{:04X})", c as u32));
    }
    Ok(())
}

/// Paths are resolved against `cwd` for checking but passed to the tool as written.
pub fn existing_file(cwd: &Path, field: &str, value: &str) -> Check<PathBuf> {
    non_empty(field, value)?;
    let p = cwd.join(value);
    match std::fs::metadata(&p) {
        Ok(m) if m.is_file() => Ok(p),
        Ok(_) => invalid(field, format!("{value} is not a file")),
        Err(_) => invalid(field, format!("{value} does not exist")),
    }
}

pub fn existing_dir(cwd: &Path, field: &str, value: &str) -> Check<PathBuf> {
    non_empty(field, value)?;
    let p = cwd.join(value);
    match std::fs::metadata(&p) {
        Ok(m) if m.is_dir() => Ok(p),
        Ok(_) => invalid(field, format!("{value} is not a folder")),
        Err(_) => invalid(field, format!("{value} does not exist")),
    }
}

/// `p` with `.`, `..` and symlinks resolved, also for parts that don't exist yet: the longest
/// existing prefix is canonicalized and the rest appended. A dangling symlink is followed to
/// where it points, since writing through it would create the file there.
pub fn resolve(p: &Path) -> PathBuf {
    resolve_depth(p, 0)
}

fn resolve_depth(p: &Path, depth: u32) -> PathBuf {
    let mut cur = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                cur = canonical_prefix(&cur, depth);
                cur.pop();
            }
            c => cur.push(c),
        }
    }
    canonical_prefix(&cur, depth)
}

fn canonical_prefix(p: &Path, depth: u32) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(p) {
        return c;
    }
    if depth < 40
        && let Ok(target) = std::fs::read_link(p)
    {
        let base = p.parent().unwrap_or(Path::new("/"));
        return resolve_depth(&base.join(target), depth + 1);
    }
    match (p.parent(), p.file_name()) {
        (Some(parent), Some(name)) if !parent.as_os_str().is_empty() => canonical_prefix(parent, depth).join(name),
        _ => p.to_path_buf(),
    }
}

/// Refuses a write target that resolves outside the server's root (unless allowed), or into
/// the run records (always).
pub fn inside_root(ctx: &Ctx, field: &str, value: &str, p: &Path) -> Check<()> {
    let r = resolve(p);
    if let Some(runs) = &ctx.runs_root
        && r.starts_with(runs)
    {
        return invalid(
            field,
            format!("{value} resolves to {}, inside the run records {}; tools may not write there", r.display(), runs.display()),
        );
    }
    if ctx.allow_outside_root {
        return Ok(());
    }
    if !r.starts_with(&ctx.root) {
        return invalid(
            field,
            format!(
                "{value} resolves to {}, outside the server's root {}; write inside the root (irit-mcp --root, or --allow-outside-root)",
                r.display(),
                ctx.root.display()
            ),
        );
    }
    Ok(())
}

/// A file the tool will write: inside the root (its folder is created if missing), and an
/// existing file (or symlink) is only replaced with `overwrite`.
pub fn output_file(ctx: &Ctx, field: &str, value: &str, overwrite: bool) -> Check<PathBuf> {
    non_empty(field, value)?;
    let p = ctx.cwd.join(value);
    if p.is_dir() {
        return invalid(field, format!("{value} is a folder; give a file name"));
    }
    parent_exists(field, value, &p)?;
    inside_root(ctx, field, value, &p)?;
    if !overwrite && std::fs::symlink_metadata(&p).is_ok() {
        return invalid(field, format!("{value} already exists; pass overwrite: true to replace it"));
    }
    Ok(p)
}

/// A folder the tool will create or fill, inside the root (its parent is created if missing).
/// With `overwrite: Some(false)`, a folder that already has entries is refused.
///
/// `writes_through_links`: the tool opens files in the folder by name, so a symlink already in
/// the folder would be followed. Then every symlink in it must also resolve inside the root
/// (and outside the run records). Tools that only create links (sheetgen `link_into`) pass false.
pub fn output_dir(ctx: &Ctx, field: &str, value: &str, overwrite: Option<bool>, writes_through_links: bool) -> Check<PathBuf> {
    non_empty(field, value)?;
    let p = ctx.cwd.join(value);
    if p.exists() && !p.is_dir() {
        return invalid(field, format!("{value} exists and is not a folder"));
    }
    parent_exists(field, value, &p)?;
    inside_root(ctx, field, value, &p)?;
    if overwrite == Some(false) && std::fs::read_dir(&p).is_ok_and(|mut rd| rd.next().is_some()) {
        return invalid(field, format!("{value} already has files that may be replaced; pass overwrite: true, or pick an empty folder"));
    }
    if writes_through_links && let Ok(rd) = std::fs::read_dir(&p) {
        for e in rd.flatten() {
            if e.file_type().is_ok_and(|t| t.is_symlink()) {
                let name = format!("{value}/{}", e.file_name().to_string_lossy());
                inside_root(ctx, field, &name, &e.path())
                    .map_err(|e| Invalid { message: format!("{} (the tool would write through this symlink; remove it or pick another folder)", e.message), ..e })?;
            }
        }
    }
    Ok(p)
}

/// The folder an output goes into may be missing: the server creates it before the tool runs
/// (see `output_folders`). Whatever part of the path exists must be folders, though.
fn parent_exists(field: &str, value: &str, p: &Path) -> Check<()> {
    let mut up = p.parent();
    while let Some(d) = up.filter(|d| !d.as_os_str().is_empty()) {
        match std::fs::metadata(d) {
            Ok(m) if m.is_dir() => return Ok(()),
            Ok(_) => return invalid(field, format!("{} is a file, not a folder, so {value} can't be written", d.display())),
            Err(_) => up = d.parent(),
        }
    }
    Ok(())
}

/// Folders the outputs go into, which the server creates when missing before the tool runs and
/// replay.sh creates with `mkdir -p`: the parents of `outputs` other than the working folder,
/// inside the root (outside it only when missing, with `allow_outside_root`).
pub fn output_folders(ctx: &Ctx, outputs: &[PathBuf]) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = outputs
        .iter()
        .filter_map(|o| o.parent())
        .filter(|d| !d.as_os_str().is_empty() && *d != ctx.cwd)
        .filter(|d| !d.is_dir() || resolve(d).starts_with(&ctx.root))
        .map(Path::to_path_buf)
        .collect();
    out.sort();
    out.dedup();
    out
}

pub fn non_empty_list<T>(field: &str, list: &[T]) -> Check<()> {
    if list.is_empty() {
        return invalid(field, "give at least one entry");
    }
    Ok(())
}

/// Same rule dblock applies to catalog ids (they become folder names).
pub fn db_id(field: &str, id: &str) -> Check<()> {
    non_empty(field, id)?;
    let ok = id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !ok || !id.starts_with(|c: char| c.is_ascii_alphanumeric()) || id.contains("..") {
        return invalid(field, format!("{id:?} is not a database id (A-Z a-z 0-9 . _ -, starting with a letter or digit, no ..); see dblock_catalog"));
    }
    Ok(())
}

/// `owner/name`, as Nextflow names pipelines.
pub fn pipeline_name(field: &str, name: &str) -> Check<()> {
    let part_ok = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    match name.split_once('/') {
        Some((a, b)) if part_ok(a) && part_ok(b) && !a.starts_with('.') && !b.starts_with('.') => Ok(()),
        _ => invalid(field, format!("{name:?} is not a pipeline name like nf-core/bacass")),
    }
}

pub fn column_name(field: &str, col: &str) -> Check<()> {
    non_empty(field, col)?;
    if col.contains('=') || col.chars().any(char::is_whitespace) {
        return invalid(field, format!("{col:?} is not a column name"));
    }
    Ok(())
}

pub fn text(field: &str, value: &str) -> Check<()> {
    non_empty(field, value)
}

pub fn range<T: PartialOrd + std::fmt::Display>(field: &str, v: T, lo: T, hi: T) -> Check<()> {
    if v < lo || v > hi {
        return invalid(field, format!("{v} is outside {lo}..={hi}"));
    }
    Ok(())
}

/// What a sequence file should hold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Seq {
    Fastq,
    Fasta,
}

/// Reads the start of a FASTQ/FASTA file (plain or gzip) and refuses anything else: an empty
/// file, a FASTA given as reads, a compression the tool can't read.
pub fn sequence_file(path: &Path, field: &str, value: &str, kind: Seq) -> Check<()> {
    use std::io::Read;
    let io_err = |e: std::io::Error| Invalid { field: field.into(), message: format!("{value}: {e}") };
    let mut head = Vec::new();
    std::fs::File::open(path).and_then(|f| f.take(1 << 16).read_to_end(&mut head)).map_err(io_err)?;
    match head.as_slice() {
        [0x1f, 0x8b, ..] => {
            // Only the first record matters, so one read of the decompressed stream is enough.
            let mut buf = vec![0u8; 4096];
            let mut gz = flate2::read::MultiGzDecoder::new(std::fs::File::open(path).map_err(io_err)?);
            let n = gz.read(&mut buf).unwrap_or(0);
            if n == 0 {
                return invalid(field, format!("{value} is gzip, but it could not be decompressed or holds nothing"));
            }
            buf.truncate(n);
            head = buf;
        }
        [b'B', b'Z', b'h', ..] | [0xfd, b'7', b'z', b'X', b'Z', 0x00, ..] | [0x28, 0xb5, 0x2f, 0xfd, ..] => {
            return invalid(field, format!("{value} is compressed with bzip2, xz or zstd; this tool reads plain or gzip files only"));
        }
        _ => {}
    }
    let (want, name) = match kind {
        Seq::Fastq => (b'@', "FASTQ (first record starting with @)"),
        Seq::Fasta => (b'>', "FASTA (first record starting with >)"),
    };
    match head.iter().find(|b| !b.is_ascii_whitespace()) {
        None => invalid(field, format!("{value} is empty")),
        Some(&b) if b == want => Ok(()),
        Some(&b'>') if kind == Seq::Fastq => invalid(field, format!("{value} looks like FASTA, not {name}")),
        Some(&b'@') if kind == Seq::Fasta => invalid(field, format!("{value} looks like FASTQ, not {name}")),
        Some(_) => invalid(field, format!("{value} does not look like {name}")),
    }
}

/// Files directly in `dir` whose name ends with `.{extension}` (as GTDB-Tk and CheckM2 match them).
pub fn files_with_extension(dir: &Path, extension: &str) -> Vec<PathBuf> {
    let suffix = format!(".{}", extension.trim_start_matches('.'));
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.is_file() && p.to_string_lossy().ends_with(&suffix)).collect())
        .unwrap_or_default();
    out.sort();
    out
}

/// A file name extension like `fna` or `fna.gz`: no separators, no leading dash.
pub fn extension(field: &str, value: &str) -> Check<()> {
    non_empty(field, value)?;
    let ok = value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !ok || value.starts_with('-') {
        return invalid(field, format!("{value:?} is not a file extension like fna or fna.gz"));
    }
    Ok(())
}

/// A name that becomes part of output file names (a prefix): no separators, no leading dot or dash.
pub fn file_stem(field: &str, value: &str) -> Check<()> {
    non_empty(field, value)?;
    let ok = value.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !ok || value.starts_with(['.', '-']) {
        return invalid(field, format!("{value:?} must be letters, digits, . _ - (not starting with . or -)"));
    }
    Ok(())
}

pub fn percent(field: &str, v: f64) -> Check<()> {
    if !v.is_finite() {
        return invalid(field, "must be a number");
    }
    range(field, v, 0.0, 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(dir: &Path) -> Ctx {
        let root = std::fs::canonicalize(dir).unwrap();
        Ctx { cwd: root.clone(), runs_root: Some(root.join("irit-runs")), root, allow_outside_root: false, vars: BTreeMap::new() }
    }

    #[test]
    fn paths() {
        let d = tempfile::tempdir().unwrap();
        let c = ctx(d.path());
        std::fs::write(d.path().join("a.fq"), "x").unwrap();
        std::fs::create_dir(d.path().join("sub")).unwrap();
        assert!(existing_file(d.path(), "f", "a.fq").is_ok());
        assert_eq!(existing_file(d.path(), "f", "sub").unwrap_err().message, "sub is not a file");
        assert!(existing_file(d.path(), "f", "missing.fq").is_err());
        assert!(existing_file(d.path(), "f", "").is_err());
        assert!(existing_dir(d.path(), "d", "sub").is_ok());
        assert!(existing_dir(d.path(), "d", "a.fq").is_err());
        assert!(output_file(&c, "o", "sheet.csv", false).is_ok());
        assert!(output_file(&c, "o", "sub", false).is_err());
        // missing folders are created by the server; a file in the way is refused
        assert!(output_file(&c, "o", "nope/deeper/sheet.csv", false).is_ok());
        assert_eq!(
            output_file(&c, "o", "a.fq/x/sheet.csv", false).unwrap_err().message,
            format!("{} is a file, not a folder, so a.fq/x/sheet.csv can't be written", c.cwd.join("a.fq").display())
        );
        assert!(output_dir(&c, "o", "qc", Some(false), true).is_ok());
        assert!(output_dir(&c, "o", "a.fq", None, true).is_err());
        assert!(output_dir(&c, "o", "nope/qc", None, true).is_ok());
        assert!(output_dir(&c, "o", "a.fq/qc", None, true).is_err());
        let outs = [c.cwd.join("sheet.csv"), c.cwd.join("nope/deeper/sheet.csv"), c.cwd.join("sub/x"), c.cwd.join("sub/y")];
        assert_eq!(output_folders(&c, &outs), [c.cwd.join("nope/deeper"), c.cwd.join("sub")]);
    }

    #[test]
    fn control_characters_are_refused() {
        for bad in ["a\ntouch x #", "a\rb", "tab\there", "esc\u{1b}[2J", "nul\0"] {
            assert!(text("set", bad).is_err(), "{bad:?}");
        }
        assert!(text("set", "auto").is_ok());
        assert!(text("set", "it's $(x) `y` -z").is_ok());
        let d = tempfile::tempdir().unwrap();
        assert!(existing_dir(d.path(), "dirs", "a\nb").unwrap_err().message.contains("control characters"));
    }

    #[test]
    fn writes_stay_inside_the_root() {
        let d = tempfile::tempdir().unwrap();
        let c = ctx(d.path());
        std::fs::create_dir(c.root.join("sub")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        let outside_abs = std::fs::canonicalize(outside.path()).unwrap();
        let target = outside_abs.join("bashrc");
        assert_eq!(output_file(&c, "output", target.to_str().unwrap(), true).unwrap_err().field, "output");
        assert!(output_file(&c, "output", "../escape.csv", true).is_err());
        assert!(output_file(&c, "output", "sub/../ok.csv", false).is_ok());
        assert!(output_dir(&c, "outdir", "sub/../../qc", Some(true), true).is_err());
        // a symlinked folder pointing outside, and a dangling symlink pointing outside
        std::os::unix::fs::symlink(&outside_abs, c.root.join("out")).unwrap();
        assert!(output_file(&c, "output", "out/x.csv", false).is_err());
        assert!(output_dir(&c, "outdir", "out", Some(true), true).is_err());
        std::os::unix::fs::symlink(outside_abs.join("new.csv"), c.root.join("dangling.csv")).unwrap();
        assert!(output_file(&c, "output", "dangling.csv", true).unwrap_err().message.contains("outside"));
        // a symlink that stays inside is fine
        std::os::unix::fs::symlink(c.root.join("sub"), c.root.join("qc")).unwrap();
        assert!(output_dir(&c, "outdir", "qc", Some(false), true).is_ok());
        // the escape hatch
        let open = Ctx { allow_outside_root: true, ..c.clone() };
        assert!(output_file(&open, "output", target.to_str().unwrap(), false).is_ok());
        assert!(!target.exists());
    }

    /// leanqc opens `<outdir>/<name>_leanqc.json` etc. by name, so a symlink already in the
    /// folder is written through: it must resolve inside the root too.
    #[test]
    fn symlinks_inside_an_output_folder_are_checked() {
        let d = tempfile::tempdir().unwrap();
        let c = ctx(d.path());
        let outside = tempfile::tempdir().unwrap();
        let victim = std::fs::canonicalize(outside.path()).unwrap().join("victim");
        std::fs::write(&victim, "keep").unwrap();
        std::fs::create_dir(c.root.join("qc")).unwrap();
        std::fs::write(c.root.join("inside.txt"), "x").unwrap();
        std::os::unix::fs::symlink(c.root.join("inside.txt"), c.root.join("qc/ok_leanqc.json")).unwrap();
        assert!(output_dir(&c, "outdir", "qc", Some(true), true).is_ok());
        std::os::unix::fs::symlink(&victim, c.root.join("qc/s_leanqc.json")).unwrap();
        let e = output_dir(&c, "outdir", "qc", Some(true), true).unwrap_err();
        assert_eq!(e.field, "outdir");
        assert!(e.message.contains("qc/s_leanqc.json") && e.message.contains("write through"), "{}", e.message);
        // dangling, pointing outside
        std::fs::remove_file(c.root.join("qc/s_leanqc.json")).unwrap();
        std::os::unix::fs::symlink(victim.with_file_name("new"), c.root.join("qc/leanqc_mqc.tsv")).unwrap();
        assert!(output_dir(&c, "outdir", "qc", Some(true), true).is_err());
        // a tool that only creates links (sheetgen link_into) is not affected
        assert!(output_dir(&c, "link_into", "qc", None, false).is_ok());
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
    }

    #[test]
    fn run_records_are_never_a_target() {
        let d = tempfile::tempdir().unwrap();
        let c = ctx(d.path());
        std::fs::create_dir_all(c.root.join("irit-runs/old")).unwrap();
        std::fs::write(c.root.join("irit-runs/old/replay.sh"), "x").unwrap();
        let e = output_file(&c, "output", "irit-runs/old/replay.sh", true).unwrap_err();
        assert!(e.message.contains("run records"), "{}", e.message);
        assert!(output_file(&c, "output", "irit-runs/new.csv", true).is_err());
        assert!(output_dir(&c, "outdir", "irit-runs", Some(true), true).is_err());
        assert!(output_dir(&c, "outdir", "irit-runs/old/../qc", Some(true), true).is_err());
        // via a symlink, and with the root check turned off
        std::os::unix::fs::symlink(c.root.join("irit-runs/old"), c.root.join("alias")).unwrap();
        assert!(output_file(&c, "output", "alias/run.json", true).is_err());
        let open = Ctx { allow_outside_root: true, ..c.clone() };
        assert!(output_file(&open, "output", "irit-runs/old/replay.sh", true).is_err());
        assert!(output_file(&c, "output", "irit-runs-not.csv", true).is_ok());
    }

    #[test]
    fn overwrite_must_be_asked_for() {
        let d = tempfile::tempdir().unwrap();
        let c = ctx(d.path());
        std::fs::write(c.root.join("sheet.csv"), "x").unwrap();
        assert!(output_file(&c, "output", "sheet.csv", false).unwrap_err().message.contains("overwrite: true"));
        assert!(output_file(&c, "output", "sheet.csv", true).is_ok());
        std::fs::create_dir(c.root.join("qc")).unwrap();
        assert!(output_dir(&c, "outdir", "qc", Some(false), true).is_ok()); // empty
        std::fs::write(c.root.join("qc/r.json"), "x").unwrap();
        assert!(output_dir(&c, "outdir", "qc", Some(false), true).is_err());
        assert!(output_dir(&c, "outdir", "qc", Some(true), true).is_ok());
        assert!(output_dir(&c, "outdir", "qc", None, true).is_ok());
    }

    #[test]
    fn names() {
        assert!(db_id("ids", "kraken2-viral-20261022").is_ok());
        for bad in ["", ".hidden", "a..b", "a/b", "a b", "--stream"] {
            assert!(db_id("ids", bad).is_err(), "{bad}");
        }
        assert!(pipeline_name("p", "nf-core/bacass").is_ok());
        for bad in ["bacass", "nf-core/", "../x/y", "a/b/c", "a/b c"] {
            assert!(pipeline_name("p", bad).is_err(), "{bad}");
        }
        assert!(column_name("set", "strandedness").is_ok());
        assert!(column_name("set", "a=b").is_err());
        assert!(percent("q", 80.0).is_ok());
        assert!(percent("q", 101.0).is_err());
        assert!(percent("q", f64::NAN).is_err());
    }
}
