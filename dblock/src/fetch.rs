//! Downloading, verifying and unpacking one database into the store.
//!
//! Two ways to get a database onto disk:
//! - **download, then extract** (default): the archive is saved, resumable after an interrupted
//!   connection, checked against its MD5, then unpacked. Needs room for archive + contents.
//! - **stream**: the archive is unpacked while it downloads and never stored. Needs room for
//!   the contents only, which is what makes GTDB-Tk (61 GB archive, ~100 GB unpacked) possible
//!   on a laptop. Not resumable; the MD5 is still checked over the whole stream and nothing
//!   is kept if it doesn't match.
//!
//! Either way the contents land in a uniquely named temporary folder and only take their final
//! name once the checksum matches and the folder's marker (with its file list and SHA-256 sums)
//! is written, so a half-finished or corrupt database is never mistaken for a good one.
//! One process at a time works on a given database: `obtain` holds `<store>/.lock-<id>`.

use crate::catalog::{Entry, Format, human};
use crate::project::{self, Locked};
use anyhow::{Context, Result, bail};
use md5::{Digest, Md5};
use sha2::Sha256;
use std::fs::{self, File};
use std::io::{self, IsTerminal, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Free space kept in reserve on top of what a database needs.
const MARGIN: u64 = 2_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    DownloadThenExtract,
    Stream,
}

pub struct Plan {
    pub mode: Mode,
    pub free: u64,
    pub need: u64,
    /// Bytes already downloaded by an earlier, interrupted attempt.
    pub resumable: u64,
}

pub struct Opts {
    pub force_stream: bool,
    pub keep_archive: bool,
}

pub fn target_dir(store: &Path, e: &Entry) -> PathBuf {
    store.join(&e.id)
}

fn partial_path(store: &Path, e: &Entry) -> PathBuf {
    let name = e.urls[0].rsplit('/').next().unwrap_or(&e.id);
    store.join(".partial").join(format!("{}--{name}.part", e.id))
}

/// What the store holds for a database.
pub enum Stored {
    Absent,
    /// Intact (file names and sizes match its marker) and the release the catalog describes.
    Ready(Box<Locked>),
    /// Something is there but can't be reused; the text says what and what to do.
    Bad(String),
}

/// Is the database already in the store, intact, and the same release the catalog describes?
pub fn existing(store: &Path, e: &Entry) -> Stored {
    let dir = target_dir(store, e);
    if dir.symlink_metadata().is_err() {
        return Stored::Absent;
    }
    let Some(mut marker) = project::read_marker(&dir) else {
        return Stored::Bad(format!(
            "{} exists but has no readable {} marker: a fetch that crashed while moving it into place \
             (older dblock wrote the marker last) or a folder made by hand. Delete it and fetch again",
            dir.display(),
            project::MARKER
        ));
    };
    if marker.id != e.id {
        return Stored::Bad(format!("{}: its marker is for `{}`, not `{}`; move it away, then fetch again", dir.display(), marker.id, e.id));
    }
    if !marker.archive_md5.eq_ignore_ascii_case(&e.md5) {
        return Stored::Bad(format!(
            "{} holds a different release (archive md5 {}, the catalog wants {}); move or delete it, then fetch again",
            dir.display(),
            marker.archive_md5,
            e.md5
        ));
    }
    match manifest(&dir) {
        Ok((_, _, m)) if m == marker.manifest_md5 => {}
        Ok((files, bytes, _)) => {
            return Stored::Bad(format!(
                "{} has changed since it was fetched (now {files} files / {}, its marker says {} files / {}); \
                 delete it and fetch again",
                dir.display(),
                human(bytes),
                marker.files,
                human(marker.bytes)
            ));
        }
        Err(err) => return Stored::Bad(format!("{}: {err:#}", dir.display())),
    }
    // The store may have moved since the marker was written: point at where it is now.
    let old = PathBuf::from(&marker.path);
    if let Ok(rel) = Path::new(&marker.data_path).strip_prefix(&old) {
        marker.data_path = dir.join(rel).display().to_string();
    }
    marker.path = dir.display().to_string();
    Stored::Ready(Box::new(marker))
}

/// Take `<store>/.lock-<id>`, waiting for another dblock that holds it. Released on drop.
pub fn lock_database(store: &Path, id: &str) -> Result<File> {
    fs::create_dir_all(store).with_context(|| format!("creating {}", store.display()))?;
    let lock_path = store.join(format!(".lock-{id}"));
    let lock = File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .with_context(|| format!("opening {}", lock_path.display()))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => {
            eprintln!("  another dblock is working on {id} in this store; waiting for it");
            lock.lock().with_context(|| format!("locking {}", lock_path.display()))?;
        }
        Err(fs::TryLockError::Error(err)) => return Err(err).with_context(|| format!("locking {}", lock_path.display())),
    }
    Ok(lock)
}

/// Where one database's temporary folders go: `<store>/.tmp/<id>/`. A folder per id, so cleaning
/// up after `gtdbtk-r232` can't touch `gtdbtk-r232-mockup`'s (a name prefix would).
fn tmp_parent(store: &Path, id: &str) -> PathBuf {
    store.join(".tmp").join(id)
}

/// Remove temporary folders a crashed run left for this database. Call with its lock held.
fn clean_tmp(store: &Path, id: &str) -> Result<()> {
    let parent = tmp_parent(store, id);
    if parent.is_dir() {
        for entry in fs::read_dir(&parent)?.flatten() {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    // dblock 0.1 used `<store>/.tmp-<id>-XXXXXX`. Only exact matches: six random alphanumerics
    // after the id, so `.tmp-<id>-mockup-XXXXXX` (another id's) is left alone.
    let legacy = format!(".tmp-{id}-");
    for entry in fs::read_dir(store)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(rest) = name.strip_prefix(&legacy)
            && rest.len() == 6
            && rest.chars().all(|c| c.is_ascii_alphanumeric())
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
    Ok(())
}

/// Lock the database's slot in the store, then reuse what's there or fetch it.
/// Returns the lock entry and whether it was already in the store.
pub fn obtain(store: &Path, e: &Entry, opts: &Opts) -> Result<(Locked, bool)> {
    // Held until `lock` drops at the end of this function.
    let lock = lock_database(store, &e.id)?;
    match existing(store, e) {
        Stored::Ready(l) => return Ok((*l, true)),
        Stored::Bad(why) => bail!(why),
        Stored::Absent => {}
    }
    // Temporary folders left by a crashed run (nobody else can be using them: we hold the lock).
    clean_tmp(store, &e.id)?;
    let plan = plan(store, e, opts)?;
    let locked = fetch(store, e, &plan, opts)?;
    drop(lock);
    Ok((locked, false))
}

/// Decide how to fetch, or explain why it won't fit.
pub fn plan(store: &Path, e: &Entry, opts: &Opts) -> Result<Plan> {
    fs::create_dir_all(store).with_context(|| format!("creating {}", store.display()))?;
    let free = fs4::available_space(store).with_context(|| format!("checking free space on {}", store.display()))?;
    let resumable = fs::metadata(partial_path(store, e)).map(|m| m.len()).unwrap_or(0);
    let stored = if e.format == Format::File { e.size } else { e.extracted };
    let full = if e.format == Format::File { e.size } else { e.size + e.extracted };
    let can_stream = e.format != Format::File;

    if opts.force_stream && can_stream {
        if free < stored + MARGIN {
            bail!(too_small(e, free, stored));
        }
        return Ok(Plan { mode: Mode::Stream, free, need: stored, resumable: 0 });
    }
    if free + resumable >= full + MARGIN {
        return Ok(Plan { mode: Mode::DownloadThenExtract, free, need: full - resumable.min(e.size), resumable });
    }
    if can_stream && free >= stored + MARGIN {
        return Ok(Plan { mode: Mode::Stream, free, need: stored, resumable: 0 });
    }
    bail!(too_small(e, free, stored))
}

fn too_small(e: &Entry, free: u64, stored: u64) -> String {
    format!(
        "{} needs about {} of free space (plus {} reserve) and {} is free.\n  \
         Free some space, or point the store at a bigger disk: `store = \"/mnt/big/dblock\"` in dblock.toml or DBLOCK_STORE=...",
        e.id,
        human(stored),
        human(MARGIN),
        human(free)
    )
}

/// Download into a fresh temporary folder, write its marker there, then move it into place.
/// Call through `obtain`, which holds the per-database lock.
fn fetch(store: &Path, e: &Entry, plan: &Plan, opts: &Opts) -> Result<Locked> {
    let parent = tmp_parent(store, &e.id);
    fs::create_dir_all(&parent).with_context(|| format!("creating {}", parent.display()))?;
    let tmp_dir = tempfile::Builder::new()
        .prefix("fetch-")
        .tempdir_in(&parent)
        .with_context(|| format!("creating a temporary folder in {}", parent.display()))?;
    let tmp = tmp_dir.path();

    let mut last_err = None;
    let mut used_url = None;
    for url in &e.urls {
        let r = match plan.mode {
            Mode::DownloadThenExtract => download_then_extract(store, e, url, tmp, opts),
            Mode::Stream => stream_extract(e, url, tmp),
        };
        match r {
            Ok(()) => {
                used_url = Some(url.clone());
                break;
            }
            Err(err) => {
                eprintln!("  {url}: {err:#}");
                // A checksum mismatch is the data, not the mirror: don't hide it behind a retry.
                if err.to_string().contains("checksum") {
                    return Err(err);
                }
                last_err = Some(err);
                fs::remove_dir_all(tmp)?;
                fs::create_dir(tmp)?;
            }
        }
    }
    let Some(url) = used_url else {
        return Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no URL worked")));
    };

    let target = target_dir(store, e);
    if target.symlink_metadata().is_ok() {
        bail!("{} appeared while downloading; move or delete it, then fetch again", target.display());
    }
    eprintln!("  recording file sizes and SHA-256 sums");
    let (files, bytes, manifest_md5) = manifest(tmp)?;
    let contents_sha256 = write_sums(tmp)?;
    let data_rel = data_path(tmp, &e.env_points_to)?.strip_prefix(tmp)?.to_path_buf();
    let locked = Locked {
        id: e.id.clone(),
        name: e.name.clone(),
        version: e.version.clone(),
        tool: e.tool.clone(),
        url,
        archive_md5: e.md5.clone(),
        archive_bytes: e.size,
        fetched: humantime::format_rfc3339_seconds(std::time::SystemTime::now()).to_string(),
        path: target.display().to_string(),
        data_path: target.join(data_rel).display().to_string(),
        files,
        bytes,
        manifest_md5,
        contents_sha256: Some(contents_sha256),
        env: e.env.clone(),
        licence: e.licence.clone(),
        non_commercial: e.non_commercial,
    };
    // The marker goes in before the rename, so a folder under the final name is always complete.
    project::write_marker(tmp, &locked)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // tempfile makes the folder private (0700); the store is meant to be shared.
        fs::set_permissions(tmp, fs::Permissions::from_mode(0o755))?;
    }
    let tmp = tmp_dir.keep();
    if let Err(err) = fs::rename(&tmp, &target) {
        let _ = fs::remove_dir_all(&tmp);
        return Err(err).with_context(|| format!("moving into {}", target.display()));
    }
    Ok(locked)
}

fn get(url: &str, from: u64) -> Result<(ureq::http::Response<ureq::Body>, bool)> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_connect(Some(Duration::from_secs(30)))
        .timeout_recv_body(None)
        .user_agent(concat!("dblock/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let mut req = agent.get(url);
    if from > 0 {
        req = req.header("Range", format!("bytes={from}-"));
    }
    let resp = req.call().with_context(|| format!("requesting {url}"))?;
    let partial = resp.status().as_u16() == 206;
    Ok((resp, partial))
}

fn download_then_extract(store: &Path, e: &Entry, url: &str, tmp: &Path, opts: &Opts) -> Result<()> {
    let part = partial_path(store, e);
    fs::create_dir_all(part.parent().unwrap())?;
    let mut have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > e.size {
        // A different file was left here: empty it, or its tail would survive the new download.
        fs::OpenOptions::new().write(true).open(&part)?.set_len(0)?;
        have = 0;
    }

    let mut hasher = Md5::new();
    if have > 0 {
        eprintln!("  resuming at {} of {}; re-reading what's there to keep the checksum honest", human(have), human(e.size));
        let mut f = File::open(&part)?;
        hash_reader(&mut (&mut f).take(have), &mut hasher)?;
    }
    if have < e.size {
        let (resp, partial) = get(url, have)?;
        let mut file = fs::OpenOptions::new().create(true).write(true).truncate(false).open(&part)?;
        if have > 0 && !partial {
            eprintln!("  server ignored the resume request; starting over");
            have = 0;
            hasher = Md5::new();
            file.set_len(0)?;
        }
        file.seek(io::SeekFrom::Start(have))?;
        let mut body = resp.into_body().into_reader();
        let mut progress = Progress::new(e.size, have);
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = body.read(&mut buf).context("download interrupted; run `dblock fetch` again to resume")?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            file.write_all(&buf[..n])?;
            progress.add(n as u64);
        }
        file.flush()?;
        progress.done();
    }

    let got = fs::metadata(&part)?.len();
    let md5 = hex(&hasher.finalize());
    if got != e.size || md5 != e.md5 {
        let _ = fs::remove_file(&part);
        bail!(
            "checksum mismatch for {}: got {} bytes with md5 {md5}, expected {} bytes with md5 {} ({}). \
             The partial file was deleted; fetch again, and if it repeats the publisher may have replaced the file.",
            e.id,
            got,
            e.size,
            e.md5,
            e.md5_source
        );
    }
    eprintln!("  checksum ok ({md5})");

    if e.format == Format::File {
        let name = url.rsplit('/').next().unwrap_or(&e.id);
        fs::rename(&part, tmp.join(name)).or_else(|_| fs::copy(&part, tmp.join(name)).map(|_| ()))?;
        let _ = fs::remove_file(&part);
        return Ok(());
    }
    eprintln!("  unpacking");
    unpack(e.format, File::open(&part)?, tmp)?;
    if opts.keep_archive {
        let keep = store.join("archives");
        fs::create_dir_all(&keep)?;
        fs::rename(&part, keep.join(part.file_name().unwrap().to_string_lossy().trim_end_matches(".part")))?;
    } else {
        fs::remove_file(&part)?;
    }
    Ok(())
}

fn stream_extract(e: &Entry, url: &str, tmp: &Path) -> Result<()> {
    eprintln!("  streaming: unpacking while downloading, so the archive never touches the disk (not resumable)");
    let (resp, _) = get(url, 0)?;
    let mut reader = HashingReader { inner: resp.into_body().into_reader(), hasher: Md5::new(), progress: Progress::new(e.size, 0) };
    unpack_stream(e.format, &mut reader, tmp)?;
    // tar stops at its end marker; hash any padding after it so the MD5 covers the whole file.
    io::copy(&mut reader, &mut io::sink())?;
    let HashingReader { hasher, progress, .. } = reader;
    progress.done();
    let md5 = hex(&hasher.finalize());
    if progress.done_bytes != e.size || md5 != e.md5 {
        bail!(
            "checksum mismatch for {}: streamed {} bytes with md5 {md5}, expected {} bytes with md5 {}. Nothing was kept.",
            e.id,
            progress.done_bytes,
            e.size,
            e.md5
        );
    }
    eprintln!("  checksum ok ({md5})");
    Ok(())
}

fn unpack(format: Format, src: impl Read, dest: &Path) -> Result<()> {
    let src = io::BufReader::with_capacity(1 << 20, src);
    match format {
        Format::TarGz => tar_unpack(flate2::read::MultiGzDecoder::new(src), dest),
        Format::TarXz => tar_unpack(liblzma::read::XzDecoder::new_multi_decoder(src), dest),
        Format::TarBz2 => tar_unpack(bzip2::read::MultiBzDecoder::new(src), dest),
        Format::File => unreachable!("plain files are not unpacked"),
    }
}

/// Like `unpack`, but borrows the source so the caller can drain and inspect it afterwards.
fn unpack_stream<R: Read>(format: Format, src: &mut R, dest: &Path) -> Result<()> {
    match format {
        Format::TarGz => {
            let mut d = flate2::read::MultiGzDecoder::new(io::BufReader::new(&mut *src));
            tar_unpack(&mut d, dest)?;
            io::copy(&mut d, &mut io::sink())?;
        }
        Format::TarXz => {
            let mut d = liblzma::read::XzDecoder::new_multi_decoder(io::BufReader::new(&mut *src));
            tar_unpack(&mut d, dest)?;
            io::copy(&mut d, &mut io::sink())?;
        }
        Format::TarBz2 => {
            let mut d = bzip2::read::MultiBzDecoder::new(io::BufReader::new(&mut *src));
            tar_unpack(&mut d, dest)?;
            io::copy(&mut d, &mut io::sink())?;
        }
        Format::File => bail!("plain files can't be streamed"),
    }
    Ok(())
}

fn tar_unpack(r: impl Read, dest: &Path) -> Result<()> {
    let mut ar = tar::Archive::new(r);
    ar.set_preserve_permissions(false);
    ar.set_overwrite(false);
    // `unpack` refuses entries that would land outside `dest` (absolute paths, `..`).
    ar.unpack(dest).with_context(|| format!("unpacking into {}", dest.display()))?;
    Ok(())
}

struct HashingReader<R> {
    inner: R,
    hasher: Md5,
    progress: Progress,
}

impl<R: Read> Read for HashingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.hasher.update(&buf[..n]);
        self.progress.add(n as u64);
        Ok(n)
    }
}

fn hash_reader(r: &mut impl Read, h: &mut Md5) -> Result<()> {
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        h.update(&buf[..n]);
    }
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Progress on stderr: a live line on a terminal, a line per 10% in logs.
struct Progress {
    total: u64,
    done_bytes: u64,
    start_bytes: u64,
    started: Instant,
    last: Instant,
    last_decile: u64,
    tty: bool,
}

impl Progress {
    fn new(total: u64, already: u64) -> Self {
        let now = Instant::now();
        Progress {
            total,
            done_bytes: already,
            start_bytes: already,
            started: now,
            last: now,
            last_decile: already * 10 / total.max(1),
            tty: io::stderr().is_terminal(),
        }
    }

    fn add(&mut self, n: u64) {
        self.done_bytes += n;
        let decile = self.done_bytes * 10 / self.total.max(1);
        if self.tty && self.last.elapsed() >= Duration::from_millis(500) {
            self.last = Instant::now();
            eprint!("\r  {}\x1b[K", self.line());
        } else if !self.tty && decile > self.last_decile {
            self.last_decile = decile;
            eprintln!("  {}", self.line());
        }
    }

    fn line(&self) -> String {
        let secs = self.started.elapsed().as_secs_f64().max(0.001);
        let rate = (self.done_bytes - self.start_bytes) as f64 / secs;
        let pct = self.done_bytes as f64 * 100.0 / self.total.max(1) as f64;
        let left = if rate > 0.0 { (self.total.saturating_sub(self.done_bytes)) as f64 / rate } else { 0.0 };
        format!(
            "{} / {} ({pct:.0}%), {}/s, about {} left",
            human(self.done_bytes),
            human(self.total),
            human(rate as u64),
            humantime::format_duration(Duration::from_secs(left as u64))
        )
    }

    fn done(&self) {
        if self.tty {
            eprintln!("\r  {}\x1b[K", self.line());
        }
    }
}

/// What `walk` found at a path: a regular file of some size, or a symlink (never followed).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Item {
    File(u64),
    Link(String),
}

/// Counted files, total bytes and a fingerprint of the folder's file list and sizes. Symlinks
/// count as entries with their target, so retargeting one changes the fingerprint; folders
/// without links get the same fingerprint as before links were recorded.
pub fn manifest(dir: &Path) -> Result<(u64, u64, String)> {
    let mut items = Vec::new();
    walk(dir, dir, &mut items)?;
    items.sort();
    let mut h = Md5::new();
    let mut bytes = 0;
    for (rel, item) in &items {
        match item {
            Item::File(size) => {
                h.update(format!("{rel}\t{size}\n").as_bytes());
                bytes += size;
            }
            Item::Link(target) => h.update(format!("{rel}\t-> {target}\n").as_bytes()),
        }
    }
    Ok((items.len() as u64, bytes, hex(&h.finalize())))
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, Item)>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        let rel = || -> Result<String> { Ok(path.strip_prefix(root)?.to_string_lossy().into_owned()) };
        if ft.is_dir() {
            walk(root, &path, out)?;
        } else if ft.is_symlink() {
            let target = fs::read_link(&path).with_context(|| format!("reading link {}", path.display()))?;
            out.push((rel()?, Item::Link(target.to_string_lossy().into_owned())));
        } else if ft.is_file() {
            let rel = rel()?;
            if rel != project::MARKER && rel != project::SUMS {
                out.push((rel, Item::File(entry.metadata()?.len())));
            }
        }
    }
    Ok(())
}

/// `sha256  size  path` for every file, sorted by path (`/` separators on every platform).
/// A symlink is listed as `link  0  path -> target`: its target is recorded, not followed.
pub fn sums(dir: &Path) -> Result<String> {
    let mut items = Vec::new();
    walk(dir, dir, &mut items)?;
    items.sort();
    let mut out = String::new();
    let mut buf = vec![0u8; 1 << 20];
    for (rel, item) in &items {
        if let Item::Link(target) = item {
            out.push_str(&format!("link  0  {} -> {target}\n", rel.replace('\\', "/")));
            continue;
        }
        let path = dir.join(rel);
        let mut f = File::open(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut h = Sha256::new();
        let mut size = 0u64;
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
            size += n as u64;
        }
        out.push_str(&format!("{}  {size}  {}\n", hex(&h.finalize()), rel.replace('\\', "/")));
    }
    Ok(out)
}

pub fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

/// Hash every file, keep the list next to the data, and return the list's own SHA-256.
fn write_sums(dir: &Path) -> Result<String> {
    let text = sums(dir)?;
    fs::write(dir.join(project::SUMS), &text)?;
    Ok(sha256_hex(&text))
}

/// The path a tool should be given: the database folder (or the single folder inside it,
/// as GTDB-Tk's tarball wraps everything in `release232/`), or one file matched by `glob:*.ext`.
fn data_path(target: &Path, points_to: &str) -> Result<PathBuf> {
    if let Some(pattern) = points_to.strip_prefix("glob:") {
        let suffix = pattern.trim_start_matches('*');
        let mut items = Vec::new();
        walk(target, target, &mut items)?;
        let hits: Vec<_> = items.iter().filter(|(r, i)| matches!(i, Item::File(_)) && r.ends_with(suffix)).collect();
        return match hits.as_slice() {
            [(one, _)] => Ok(target.join(one)),
            [] => bail!("no {pattern} file in the download"),
            many => bail!(
                "{} files match {pattern} ({}); the catalog entry must name one (e.g. a longer suffix)",
                many.len(),
                many.iter().map(|(r, _)| r.as_str()).collect::<Vec<_>>().join(", ")
            ),
        };
    }
    let inside: Vec<_> = fs::read_dir(target)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() != project::MARKER && e.file_name() != project::SUMS)
        .collect();
    if let [only] = inside.as_slice()
        && only.file_type()?.is_dir() {
            return Ok(only.path());
        }
    Ok(target.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A `.tar.gz` holding `mydb/a.txt` and `mydb/b.txt`, and its MD5.
    fn archive() -> (Vec<u8>, String) {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default()));
        for (name, body) in [("mydb/a.txt", &b"hello"[..]), ("mydb/b.txt", &b"world"[..])] {
            let mut h = tar::Header::new_gnu();
            h.set_size(body.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append_data(&mut h, name, body).unwrap();
        }
        let bytes = tar.into_inner().unwrap().finish().unwrap();
        let md5 = hex(&Md5::digest(&bytes));
        (bytes, md5)
    }

    /// Minimal HTTP server: every GET gets the whole body, slowly enough for fetches to overlap.
    fn serve(body: Vec<u8>) -> (String, Arc<AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/mydb.tar.gz", listener.local_addr().unwrap());
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                counter.fetch_add(1, Ordering::SeqCst);
                let body = body.clone();
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                        line.clear();
                    }
                    let mut stream = stream;
                    let head = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                    stream.write_all(head.as_bytes()).unwrap();
                    std::thread::sleep(Duration::from_millis(300));
                    let _ = stream.write_all(&body);
                });
            }
        });
        (url, hits)
    }

    fn entry(url: &str, md5: &str, size: usize, points_to: &str) -> Entry {
        serde_json::from_value(serde_json::json!({
            "id": "mydb", "name": "t", "version": "1", "tool": "t", "urls": [url], "format": "tar.gz",
            "size": size, "extracted": 100, "md5": md5, "md5_source": "test", "env": "MYDB",
            "env_points_to": points_to, "compat": null, "licence": "x"
        }))
        .unwrap()
    }

    fn opts() -> Opts {
        Opts { force_stream: false, keep_archive: false }
    }

    #[test]
    fn concurrent_fetches_of_one_database_download_it_once() {
        let (body, md5) = archive();
        let (url, hits) = serve(body.clone());
        let e = entry(&url, &md5, body.len(), "dir");
        let store = tempfile::tempdir().unwrap();
        let results: Vec<_> = std::thread::scope(|s| {
            let a = s.spawn(|| obtain(store.path(), &e, &opts()));
            let b = s.spawn(|| obtain(store.path(), &e, &opts()));
            vec![a.join().unwrap(), b.join().unwrap()]
        });
        let results: Vec<_> = results.into_iter().map(|r| r.unwrap()).collect();
        assert_eq!(results.iter().filter(|(_, reused)| !reused).count(), 1, "exactly one download");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        assert_eq!(results[0].0.manifest_md5, results[1].0.manifest_md5);
        let leftovers: Vec<_> = fs::read_dir(store.path())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp-"))
            .collect();
        assert!(leftovers.is_empty());
        assert_eq!(fs::read_dir(tmp_parent(store.path(), "mydb")).unwrap().count(), 0);
        assert!(store.path().join("mydb").join(project::MARKER).is_file());
        assert!(results[0].0.data_path.ends_with("mydb/mydb") || results[0].0.data_path.ends_with("mydb\\mydb"));
    }

    #[test]
    fn uppercase_catalog_md5_downloads_fine() {
        let (body, md5) = archive();
        let (url, _) = serve(body.clone());
        let dir = tempfile::tempdir().unwrap();
        let cat = dir.path().join("cat.json");
        let e = entry(&url, &md5.to_ascii_uppercase(), body.len(), "dir");
        fs::write(&cat, serde_json::to_string(&vec![e]).unwrap()).unwrap();
        let entries = crate::catalog::load(Some(&cat)).unwrap();
        let e = crate::catalog::find(&entries, "mydb").unwrap();
        let (l, _) = obtain(&dir.path().join("store"), e, &opts()).unwrap();
        assert_eq!(l.archive_md5, md5);
    }

    #[test]
    fn folder_without_marker_is_reported_not_overwritten() {
        let (body, md5) = archive();
        let e = entry("http://127.0.0.1:9/never", &md5, body.len(), "dir");
        let store = tempfile::tempdir().unwrap();
        fs::create_dir_all(store.path().join("mydb/mydb")).unwrap();
        let err = obtain(store.path(), &e, &opts()).unwrap_err().to_string();
        assert!(err.contains("no readable .dblock.toml marker"), "{err}");
    }

    #[test]
    fn changed_folder_is_not_reused_and_deep_sums_catch_same_size_edits() {
        let (body, md5) = archive();
        let (url, hits) = serve(body.clone());
        let e = entry(&url, &md5, body.len(), "dir");
        let store = tempfile::tempdir().unwrap();
        let (l, _) = obtain(store.path(), &e, &opts()).unwrap();
        let a = store.path().join("mydb/mydb/a.txt");

        // Same length: names and sizes still match, the SHA-256 list does not.
        fs::write(&a, "HELLO").unwrap();
        assert!(matches!(existing(store.path(), &e), Stored::Ready(_)));
        assert_ne!(Some(sha256_hex(&sums(&store.path().join("mydb")).unwrap())), l.contents_sha256);

        // Different length: not reused.
        fs::write(&a, "hello!").unwrap();
        let Stored::Bad(why) = existing(store.path(), &e) else { panic!("reused a changed folder") };
        assert!(why.contains("changed since it was fetched"), "{why}");
        assert!(obtain(store.path(), &e, &opts()).is_err());
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn cleanup_leaves_other_ids_temp_folders_alone() {
        let (body, md5) = archive();
        let (url, _) = serve(body.clone());
        let mut e = entry(&url, &md5, body.len(), "dir");
        e.id = "gtdbtk-r232".into();
        let store = tempfile::tempdir().unwrap();
        // An in-progress unpack of `gtdbtk-r232-mockup` (its own lock, not ours), in both the
        // current layout and dblock 0.1's.
        let theirs = tmp_parent(store.path(), "gtdbtk-r232-mockup").join("fetch-abc123");
        fs::create_dir_all(&theirs).unwrap();
        let theirs_legacy = store.path().join(".tmp-gtdbtk-r232-mockup-abc123");
        fs::create_dir_all(&theirs_legacy).unwrap();
        // Our own crashed leftovers, both layouts.
        let ours = tmp_parent(store.path(), "gtdbtk-r232").join("fetch-zzz999");
        fs::create_dir_all(&ours).unwrap();
        let ours_legacy = store.path().join(".tmp-gtdbtk-r232-Xy12Zq");
        fs::create_dir_all(&ours_legacy).unwrap();
        obtain(store.path(), &e, &opts()).unwrap();
        assert!(theirs.is_dir() && theirs_legacy.is_dir(), "deleted another database's temp folder");
        assert!(!ours.exists() && !ours_legacy.exists());
        assert!(store.path().join("gtdbtk-r232").join(project::MARKER).is_file());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_are_recorded_in_manifest_and_sums() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.txt"), "hello").unwrap();
        fs::write(d.path().join("b.txt"), "world").unwrap();
        std::os::unix::fs::symlink("a.txt", d.path().join("current")).unwrap();
        let (files, bytes, m1) = manifest(d.path()).unwrap();
        assert_eq!((files, bytes), (3, 10));
        let s1 = sums(d.path()).unwrap();
        assert!(s1.contains("link  0  current -> a.txt"), "{s1}");
        // Retarget the link: same names, same file sizes, but both fingerprints change.
        fs::remove_file(d.path().join("current")).unwrap();
        std::os::unix::fs::symlink("b.txt", d.path().join("current")).unwrap();
        assert_ne!(manifest(d.path()).unwrap().2, m1);
        assert_ne!(sums(d.path()).unwrap(), s1);
        // A folder without links keeps the fingerprint format of dblock 0.1.
        fs::remove_file(d.path().join("current")).unwrap();
        let mut h = Md5::new();
        h.update(b"a.txt\t5\nb.txt\t5\n");
        assert_eq!(manifest(d.path()).unwrap().2, hex(&h.finalize()));
    }

    #[test]
    fn oversized_leftover_partial_is_emptied() {
        let (body, md5) = archive();
        let (url, _) = serve(body.clone());
        let e = entry(&url, &md5, body.len(), "dir");
        let store = tempfile::tempdir().unwrap();
        let part = partial_path(store.path(), &e);
        fs::create_dir_all(part.parent().unwrap()).unwrap();
        fs::write(&part, vec![b'x'; body.len() * 3]).unwrap();
        obtain(store.path(), &e, &opts()).unwrap();
    }

    #[test]
    fn several_glob_hits_are_an_error() {
        let (body, md5) = archive();
        let (url, _) = serve(body.clone());
        let e = entry(&url, &md5, body.len(), "glob:*.txt");
        let store = tempfile::tempdir().unwrap();
        let err = format!("{:#}", obtain(store.path(), &e, &opts()).unwrap_err());
        assert!(err.contains("2 files match"), "{err}");
        assert!(!store.path().join("mydb").exists());
    }
}
