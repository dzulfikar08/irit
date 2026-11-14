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
//!
//! A catalog entry may `keep` only some folders of its archive (the rest is read for the
//! checksum but never written) and may name `verify_dirs`: folders whose GTDB-Tk folder hash
//! must match, checked before the folder takes its final name.
//!
//! A multi-part entry is several archives unpacked into the same temporary folder. In download
//! mode every part is downloaded and checked first (each resumable on its own, kept under
//! `.partial/` until all are good), then all are unpacked; in stream mode they are streamed one
//! after another. Either way the folder takes its final name only once every part has passed.
//!
//! Downloads run up to `--jobs` at a time (`Slots`): the parts of one entry, and the entries of
//! one `dblock fetch`. Each download streams to its own file under `.partial/` through a 1 MiB
//! buffer. Messages and progress go through `say` and `Progress`, which keep one live line on
//! a terminal and never write into the middle of another line.

use crate::catalog::{Entry, Format, Source, human};
use crate::project::{self, Locked, LockedPart};
use anyhow::{Context, Result, bail};
use md5::{Digest, Md5};
use sha1::Sha1;
use sha2::Sha256;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, IsTerminal, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock};
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
    /// Hard-link byte-identical files inside the database (see `dedupe`).
    pub dedupe: bool,
    /// Downloads at a time, over every database fetched with these options (`--jobs`).
    pub slots: Slots,
    /// Free space claimed by the databases being fetched with these options right now.
    space: Space,
}

impl Opts {
    pub fn new(force_stream: bool, keep_archive: bool, dedupe: bool, jobs: usize) -> Opts {
        Opts { force_stream, keep_archive, dedupe, slots: Slots::new(jobs), space: Space::default() }
    }
}

/// A counting semaphore: at most `limit` downloads hold a slot at once.
pub struct Slots {
    limit: usize,
    free: Mutex<usize>,
    freed: Condvar,
}

impl Slots {
    pub fn new(limit: usize) -> Slots {
        let limit = limit.max(1);
        Slots { limit, free: Mutex::new(limit), freed: Condvar::new() }
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    /// Wait for a free slot; it is given back when the guard drops.
    fn take(&self) -> Slot<'_> {
        let mut free = relock(self.free.lock());
        while *free == 0 {
            free = relock(self.freed.wait(free));
        }
        *free -= 1;
        Slot(self)
    }
}

struct Slot<'a>(&'a Slots);

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        *relock(self.0.free.lock()) += 1;
        self.0.freed.notify_one();
    }
}

/// A panic in another download doesn't make a counter or the screen unusable.
fn relock<T>(r: std::sync::LockResult<MutexGuard<'_, T>>) -> MutexGuard<'_, T> {
    r.unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Space claimed by fetches running side by side: (bytes, fetches).
#[derive(Default)]
struct Space {
    claimed: Mutex<(u64, usize)>,
    released: Condvar,
}

/// One fetch's share of `Space`, given back when it drops (once the database is in place or
/// the fetch failed).
struct Claim<'a> {
    space: &'a Space,
    bytes: u64,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        let mut c = relock(self.space.claimed.lock());
        c.0 -= self.bytes;
        c.1 -= 1;
        self.space.released.notify_all();
    }
}

/// Raised inside a download that stopped because another part of the same entry failed.
#[derive(Debug)]
struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("stopped because another part failed")
    }
}

impl std::error::Error for Cancelled {}

pub fn target_dir(store: &Path, e: &Entry) -> PathBuf {
    store.join(&e.id)
}

fn partial_path(store: &Path, e: &Entry, src: &Source) -> PathBuf {
    store.join(".partial").join(format!("{}--{}.part", e.id, src.name))
}

/// Largest zstd window the decoder accepts, as a power of two. libzstd refuses frames with a
/// window above 2^27 (128 MiB) unless told otherwise; `zstd --long=28` and up need more. This
/// only raises the limit: memory used is the window the archive actually declares.
/// 2^30 (1 GiB) covers `--long=30`; a corrupt frame can't claim more while streaming.
const ZSTD_WINDOW_LOG_MAX: u32 = 30;

fn zstd_reader<R: io::BufRead>(r: R) -> Result<zstd::stream::read::Decoder<'static, R>> {
    let mut d = zstd::stream::read::Decoder::with_buffer(r).context("starting the zstd decoder")?;
    d.window_log_max(ZSTD_WINDOW_LOG_MAX).context("setting the zstd window limit")?;
    Ok(d)
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
    match manifest_as(&dir, marker.manifest_format) {
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
            say(format!("another dblock is working on {id} in this store; waiting for it"));
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
        Stored::Ready(l) if l.manifest_format.is_none() => return Ok((upgrade_marker(&target_dir(store, e), *l)?, true)),
        Stored::Ready(l) => {
            // A folder moved or linked in from another store: its marker still names the old
            // place. Point it here, as the lock will.
            let dir = target_dir(store, e);
            if project::read_marker(&dir).is_some_and(|m| m.path != l.path || m.data_path != l.data_path) {
                project::write_marker(&dir, &l).with_context(|| format!("updating {}", dir.join(project::MARKER).display()))?;
            }
            return Ok((*l, true));
        }
        Stored::Bad(why) => bail!(why),
        Stored::Absent => {}
    }
    // Fail before downloading, not after: the prepare step runs only once every file is in.
    if let Some(prog) = missing_prepare(e) {
        bail!(
            "{prog} is needed to prepare {} and is not on PATH, so nothing was downloaded. Install AMRFinderPlus in the \
             environment dblock runs from (e.g. `conda install -c conda-forge -c bioconda ncbi-amrfinderplus`), then fetch again",
            e.id
        );
    }
    // Temporary folders left by a crashed run (nobody else can be using them: we hold the lock).
    clean_tmp(store, &e.id)?;
    // The claim is held until the database is in place.
    let fetched = plan_claim(store, e, opts).and_then(|(plan, _claim)| fetch(store, e, &plan, opts));
    prune_tmp(store, &e.id);
    drop(lock);
    Ok((fetched?, false))
}

/// Remove `<store>/.tmp/<id>/`, and `<store>/.tmp/`, if they are empty. Call with the lock held.
pub fn prune_tmp(store: &Path, id: &str) {
    let parent = tmp_parent(store, id);
    if fs::remove_dir(&parent).is_ok()
        && let Some(tmp) = parent.parent()
    {
        let _ = fs::remove_dir(tmp);
    }
}

/// Decide how to fetch, or explain why it won't fit.
pub fn plan(store: &Path, e: &Entry, opts: &Opts) -> Result<Plan> {
    plan_with(store, e, opts, 0)
}

/// `plan` for a fetch that may run beside others started with the same options, and claim the
/// space it needs. Space the others claimed counts as taken (it is, or soon will be, although
/// what they have already written is then counted twice). When that changes the plan (it
/// wouldn't fit, or would have to stream), wait for one of them to finish and plan again, so
/// the result is what fetching one at a time would give.
fn plan_claim<'a>(store: &Path, e: &Entry, opts: &'a Opts) -> Result<(Plan, Claim<'a>)> {
    let space = &opts.space;
    let mut claimed = relock(space.claimed.lock());
    let mut said = false;
    loop {
        let alone = plan_with(store, e, opts, 0);
        if claimed.1 > 0 {
            let shared = plan_with(store, e, opts, claimed.0);
            if !matches!((&alone, &shared), (Ok(a), Ok(s)) if a.mode == s.mode) {
                if !said {
                    say("not enough free space to fetch this beside the databases being fetched now; waiting for them");
                    said = true;
                }
                claimed = relock(space.released.wait(claimed));
                continue;
            }
        }
        let plan = alone?;
        let bytes = plan.need;
        claimed.0 += bytes;
        claimed.1 += 1;
        return Ok((plan, Claim { space, bytes }));
    }
}

/// `plan`, with `claimed` bytes of the free space taken by other fetches.
fn plan_with(store: &Path, e: &Entry, opts: &Opts, claimed: u64) -> Result<Plan> {
    fs::create_dir_all(store).with_context(|| format!("creating {}", store.display()))?;
    let free = fs4::available_space(store).with_context(|| format!("checking free space on {}", store.display()))?;
    let free = free.saturating_sub(claimed);
    // Per part, at most its size counts (a bigger leftover is emptied before downloading).
    let resumable: u64 =
        e.sources().iter().map(|src| fs::metadata(partial_path(store, e, src)).map(|m| m.len().min(src.size)).unwrap_or(0)).sum();
    let stored = e.on_disk();
    // Plain files are moved into the folder, not unpacked: they need their own size, plus what
    // `prepare` adds (counted in `on_disk`).
    let full = if e.format == Format::File { stored } else { e.size + stored };
    let can_stream = e.format != Format::File;

    if opts.force_stream && can_stream {
        if free < stored + MARGIN {
            bail!(too_small(e, free, stored));
        }
        return Ok(Plan { mode: Mode::Stream, free, need: stored, resumable: 0 });
    }
    if free + resumable >= full + MARGIN {
        return Ok(Plan { mode: Mode::DownloadThenExtract, free, need: full - resumable, resumable });
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

    let sources = e.sources();
    let mut used = Vec::with_capacity(sources.len());
    match plan.mode {
        Mode::DownloadThenExtract => {
            // Every archive first, so a failure in one part leaves the others resumable.
            used = download_all(store, e, &sources, opts)?;
            let downloaded: Vec<PathBuf> = sources.iter().map(|src| partial_path(store, e, src)).collect();
            // In order, one at a time: parts may share folders.
            for (src, part) in sources.iter().zip(&downloaded) {
                if e.format == Format::File {
                    fs::rename(part, tmp.join(&src.name)).or_else(|_| fs::copy(part, tmp.join(&src.name)).map(|_| ()))?;
                    let _ = fs::remove_file(part);
                    continue;
                }
                say(format!("unpacking {}", src.name));
                unpack(e.format, File::open(part)?, tmp, &e.keep)?;
            }
            if e.format != Format::File {
                for part in &downloaded {
                    if opts.keep_archive {
                        let keep = store.join("archives");
                        fs::create_dir_all(&keep)?;
                        fs::rename(part, keep.join(part.file_name().unwrap().to_string_lossy().trim_end_matches(".part")))?;
                    } else {
                        fs::remove_file(part)?;
                    }
                }
            }
        }
        Mode::Stream => {
            // One part after another, each unpacked as it arrives: they go into one folder.
            for (i, src) in sources.iter().enumerate() {
                let _slot = opts.slots.take();
                if sources.len() > 1 {
                    say(format!("part {} of {}: {} ({})", i + 1, sources.len(), src.name, human(src.size)));
                }
                let mut last_err = None;
                let mut ok = None;
                for url in &src.urls {
                    match stream_extract(e, src, url, tmp) {
                        Ok(()) => {
                            ok = Some(url.clone());
                            break;
                        }
                        Err(err) => {
                            say(format!("{url}: {err:#}"));
                            // A checksum mismatch is the data, not the mirror. And once earlier
                            // parts are unpacked, a half-streamed part can't be taken back out.
                            if err.to_string().contains("checksum") || sources.len() > 1 {
                                return Err(err);
                            }
                            last_err = Some(err);
                            fs::remove_dir_all(tmp)?;
                            fs::create_dir(tmp)?;
                        }
                    }
                }
                let Some(url) = ok else {
                    return Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no URL worked")));
                };
                used.push(url);
            }
        }
    }
    let url = if e.is_multi_part() { e.base_url.clone().unwrap_or_else(|| used[0].clone()) } else { used[0].clone() };
    let parts: Vec<LockedPart> = if e.is_multi_part() {
        sources
            .iter()
            .zip(&used)
            .map(|(src, url)| LockedPart {
                file: src.name.clone(),
                url: url.clone(),
                bytes: src.size,
                md5: src.md5.clone(),
                sha256: src.sha256.clone(),
            })
            .collect()
    } else {
        Vec::new()
    };

    // Errors from here on drop `tmp_dir`, so nothing is kept.
    for k in &e.keep {
        if tmp.join(k).symlink_metadata().is_err() {
            bail!("keep path `{k}` matched nothing in the archive; the catalog entry or the archive's layout is wrong");
        }
    }
    if !e.verify_dirs.is_empty() {
        say("checking GTDB-Tk folder hashes");
        let bad = check_dirs(tmp, &e.verify_dirs)?;
        if !bad.is_empty() {
            bail!("folder hash mismatch for {}: {}. Nothing was kept.", e.id, bad.join("; "));
        }
    }
    // After the checksums: only files that matched the catalog are prepared.
    let prepared = match e.prepare {
        Some(p) => Some(prepare(Path::new(p.program()), tmp).with_context(|| format!("preparing {}; nothing was kept", e.id))?),
        None => None,
    };

    let target = target_dir(store, e);
    if target.symlink_metadata().is_ok() {
        bail!("{} appeared while downloading; move or delete it, then fetch again", target.display());
    }
    say("recording file sizes and SHA-256 sums");
    let (files, bytes, manifest_md5) = manifest(tmp)?;
    let items = hashed(tmp, Some(MANIFEST_FORMAT))?;
    let sums = sums_text(&items);
    let deduped = if opts.dedupe {
        let (d, err) = dedupe(tmp, &items);
        if d.linked > 0 {
            say(format!("hard-linked {} identical file(s), saving {}", d.linked, human(d.saved)));
        }
        if let Some(err) = err {
            say(format!("kept {} identical file(s) as copies: this file system refused a hard link ({err})", d.kept));
        }
        d
    } else {
        Deduped::default()
    };
    // Linking changes no name, size or content, so the list taken before it still holds.
    fs::write(tmp.join(project::SUMS), &sums)?;
    let contents_sha256 = sha256_hex(&sums);
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
        manifest_format: Some(MANIFEST_FORMAT),
        env: e.env.clone(),
        licence: e.licence.clone(),
        non_commercial: e.non_commercial,
        verify_dirs: e.verify_dirs.clone(),
        linked_files: deduped.linked,
        linked_bytes: deduped.saved,
        parts,
        prepared,
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

/// A program on PATH, as `Command` would find it (a name with a folder in it is taken as a path).
pub fn find_program(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.components().count() > 1 {
        return p.is_file().then(|| p.to_path_buf());
    }
    let exe = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join(&exe)).find(|c| c.is_file())
}

/// The catalog's `prepare` program for this entry, when it is not on PATH.
pub fn missing_prepare(e: &Entry) -> Option<&'static str> {
    e.prepare.map(|p| p.program()).filter(|p| find_program(p).is_none())
}

/// Run a catalog's `prepare` program on the fetched folder: `<program> <dir>`. Returns
/// `<program> <its --version>`, recorded in the lock so it's known what built the extra files.
pub fn prepare(program: &Path, dir: &Path) -> Result<String> {
    let name = program.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let version = std::process::Command::new(program).arg("--version").stdin(std::process::Stdio::null()).output();
    let version = match version {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout).lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("").to_string(),
        Ok(o) => bail!("`{name} --version` failed ({}); is it installed?", o.status),
        Err(err) => bail!(
            "{name} is needed to prepare this database and could not be run ({err}). Install AMRFinderPlus in the \
             environment dblock runs from (e.g. `conda install -c conda-forge -c bioconda ncbi-amrfinderplus`), then fetch again"
        ),
    };
    say(format!("running {name} {} (version {version})", dir.display()));
    let out = std::process::Command::new(program).arg(dir).stdin(std::process::Stdio::null()).output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().rev().take(10).collect();
        bail!("{name} failed ({}): {}", out.status, tail.into_iter().rev().collect::<Vec<_>>().join("\n"));
    }
    Ok(format!("{name} {version}").trim().to_string())
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

/// Try a source's URLs in order until one downloads and checks out; returns the URL used.
/// A checksum mismatch stops at once: a mirror serving the same bad file wouldn't fix it.
fn with_mirrors(src: &Source, mut f: impl FnMut(&str) -> Result<()>) -> Result<String> {
    let mut last_err = None;
    for url in &src.urls {
        match f(url) {
            Ok(()) => return Ok(url.clone()),
            // Stopped on purpose: no other mirror either.
            Err(err) if err.is::<Cancelled>() => return Err(err),
            Err(err) => {
                say(format!("{url}: {err:#}"));
                if err.to_string().contains("checksum") {
                    return Err(err);
                }
                last_err = Some(err);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no URL worked")))
}

/// MD5, and SHA-256 when the catalog gives one, over the same bytes.
struct Hashes {
    md5: Md5,
    sha256: Option<Sha256>,
}

impl Hashes {
    fn new(src: &Source) -> Self {
        Hashes { md5: Md5::new(), sha256: src.sha256.as_ref().map(|_| Sha256::new()) }
    }

    fn update(&mut self, b: &[u8]) {
        self.md5.update(b);
        if let Some(h) = &mut self.sha256 {
            h.update(b);
        }
    }

    /// `Ok(md5)` if size and hashes match the source, else the "checksum mismatch" description.
    fn check(self, src: &Source, got: u64) -> std::result::Result<String, String> {
        let md5 = hex(&self.md5.finalize());
        let sha = self.sha256.map(|h| hex(&h.finalize()));
        let mut wrong = Vec::new();
        if got != src.size {
            wrong.push(format!("{got} bytes, expected {}", src.size));
        }
        if md5 != src.md5 {
            wrong.push(format!("md5 {md5}, expected {}", src.md5));
        }
        if let (Some(got), Some(want)) = (&sha, &src.sha256)
            && got != want
        {
            wrong.push(format!("sha256 {got}, expected {want}"));
        }
        if wrong.is_empty() { Ok(md5) } else { Err(wrong.join("; ")) }
    }
}

/// Which archive a message is about: the entry, or the entry and the part.
fn label(e: &Entry, src: &Source) -> String {
    if e.is_multi_part() { format!("{} (part {})", e.id, src.name) } else { e.id.clone() }
}

/// Download every source of an entry to `.partial/` and check each, up to `--jobs` at a time;
/// returns the URL each came from, in the order of `sources`. With one job they go one after
/// another, as `dblock fetch --jobs 1` always did. On the first failure no further source is
/// started and the downloads still running stop where they are; what they got stays under
/// `.partial/` to resume next time, like any interrupted download.
fn download_all(store: &Path, e: &Entry, sources: &[Source], opts: &Opts) -> Result<Vec<String>> {
    let n = sources.len();
    let workers = opts.slots.limit().min(n).max(1);
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let results: Mutex<Vec<Option<Result<String>>>> = Mutex::new((0..n).map(|_| None).collect());
    let outer = tag();
    let work = || {
        while !stop.load(Ordering::SeqCst) {
            let i = next.fetch_add(1, Ordering::SeqCst);
            let Some(src) = sources.get(i) else { break };
            let got = {
                let _slot = opts.slots.take();
                if stop.load(Ordering::SeqCst) {
                    break;
                }
                if n > 1 {
                    say(format!("part {} of {n}: {} ({})", i + 1, src.name, human(src.size)));
                }
                // Side by side, each line says which part it is about.
                if workers > 1 {
                    set_tag(format!("{outer}{}: ", src.name));
                }
                let got = with_mirrors(src, |url| download_verify(store, e, src, url, &stop));
                match &got {
                    Err(err) if err.is::<Cancelled>() => say("stopped, as another part failed; what it got is kept for the next run"),
                    Err(_) => stop.store(true, Ordering::SeqCst),
                    Ok(_) => {}
                }
                set_tag(outer.clone());
                got
            };
            relock(results.lock())[i] = Some(got);
        }
    };
    if workers == 1 {
        work();
    } else {
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(|| {
                    set_tag(outer.clone());
                    work()
                });
            }
        });
    }
    // The failure that stopped the others, not the downloads it stopped; the first in part
    // order if several failed on their own.
    let mut urls = Vec::with_capacity(n);
    for got in relock(results.lock()).drain(..) {
        match got {
            Some(Ok(url)) => urls.push(url),
            Some(Err(err)) if err.is::<Cancelled>() => {}
            Some(Err(err)) => return Err(err),
            None => {}
        }
    }
    if urls.len() != n {
        bail!("{}: {} of {n} parts were downloaded", e.id, urls.len());
    }
    Ok(urls)
}

/// Download one archive to `.partial/` (resuming what an earlier run left) and check it.
/// On success the checked file stays there for the caller to unpack. Stops early, keeping what
/// it has, once `stop` is set.
fn download_verify(store: &Path, e: &Entry, src: &Source, url: &str, stop: &AtomicBool) -> Result<()> {
    let part = partial_path(store, e, src);
    fs::create_dir_all(part.parent().unwrap())?;
    let mut have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
    if have > src.size {
        // A different file was left here: empty it, or its tail would survive the new download.
        fs::OpenOptions::new().write(true).open(&part)?.set_len(0)?;
        have = 0;
    }

    let resumed = have > 0;
    let mut hasher = Hashes::new(src);
    if have > 0 {
        let what = if have == src.size { "already downloaded" } else { "resuming" };
        say(format!("{what}: {} of {}; re-reading what's there to keep the checksum honest", human(have), human(src.size)));
        let mut f = File::open(&part)?;
        hash_reader(&mut (&mut f).take(have), &mut hasher)?;
    }
    if have < src.size {
        let (resp, partial) = get(url, have)?;
        let mut file = fs::OpenOptions::new().create(true).write(true).truncate(false).open(&part)?;
        if have > 0 && !partial {
            say("server ignored the resume request; starting over");
            have = 0;
            hasher = Hashes::new(src);
            file.set_len(0)?;
        }
        file.seek(io::SeekFrom::Start(have))?;
        let mut body = resp.into_body().into_reader();
        let mut progress = Progress::new(src.size, have);
        let mut buf = vec![0u8; 1 << 20];
        loop {
            if stop.load(Ordering::SeqCst) {
                file.flush()?;
                return Err(Cancelled.into());
            }
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
    match hasher.check(src, got) {
        Ok(md5) => {
            let sha = if src.sha256.is_some() { " and sha256" } else { "" };
            say(format!("checksum ok (md5 {md5}{sha})"));
            Ok(())
        }
        Err(wrong) => {
            let _ = fs::remove_file(&part);
            if resumed {
                // Likely left by an earlier release of the same file name: one fresh try.
                say(format!("the partial file left by an earlier run didn't match ({wrong}); downloading it again from the start"));
                return download_verify(store, e, src, url, stop);
            }
            bail!(
                "checksum mismatch for {}: got {wrong} ({}). \
                 The partial file was deleted; fetch again, and if it repeats the publisher may have replaced the file.",
                label(e, src),
                e.md5_source
            );
        }
    }
}

fn stream_extract(e: &Entry, src: &Source, url: &str, tmp: &Path) -> Result<()> {
    say("streaming: unpacking while downloading, so the archive never touches the disk (not resumable)");
    let (resp, _) = get(url, 0)?;
    let mut reader = HashingReader { inner: resp.into_body().into_reader(), hasher: Hashes::new(src), progress: Progress::new(src.size, 0) };
    // A corrupt stream usually fails here, in the decoder, before the checksum can be compared.
    unpack_stream(e.format, &mut reader, tmp, &e.keep)
        .with_context(|| format!("{} is corrupt or cut short (unpacking failed while streaming). Nothing was kept", label(e, src)))?;
    // tar stops at its end marker; hash any padding after it so the MD5 covers the whole file.
    io::copy(&mut reader, &mut io::sink())?;
    let HashingReader { hasher, progress, .. } = reader;
    progress.done();
    match hasher.check(src, progress.bytes()) {
        Ok(md5) => {
            say(format!("checksum ok ({md5})"));
            Ok(())
        }
        Err(wrong) => bail!("checksum mismatch for {}: streamed {wrong}. Nothing was kept.", label(e, src)),
    }
}

fn unpack(format: Format, src: impl Read, dest: &Path, keep: &[String]) -> Result<()> {
    let src = io::BufReader::with_capacity(1 << 20, src);
    match format {
        Format::TarGz => tar_unpack(flate2::read::MultiGzDecoder::new(src), dest, keep),
        Format::TarXz => tar_unpack(liblzma::read::XzDecoder::new_multi_decoder(src), dest, keep),
        Format::TarBz2 => tar_unpack(bzip2::read::MultiBzDecoder::new(src), dest, keep),
        Format::TarZst => tar_unpack(zstd_reader(src)?, dest, keep),
        Format::File => unreachable!("plain files are not unpacked"),
    }
}

/// Like `unpack`, but borrows the source so the caller can drain and inspect it afterwards.
fn unpack_stream<R: Read>(format: Format, src: &mut R, dest: &Path, keep: &[String]) -> Result<()> {
    match format {
        Format::TarGz => {
            let mut d = flate2::read::MultiGzDecoder::new(io::BufReader::new(&mut *src));
            tar_unpack(&mut d, dest, keep)?;
            io::copy(&mut d, &mut io::sink())?;
        }
        Format::TarXz => {
            let mut d = liblzma::read::XzDecoder::new_multi_decoder(io::BufReader::new(&mut *src));
            tar_unpack(&mut d, dest, keep)?;
            io::copy(&mut d, &mut io::sink())?;
        }
        Format::TarBz2 => {
            let mut d = bzip2::read::MultiBzDecoder::new(io::BufReader::new(&mut *src));
            tar_unpack(&mut d, dest, keep)?;
            io::copy(&mut d, &mut io::sink())?;
        }
        Format::TarZst => {
            let mut d = zstd_reader(io::BufReader::new(&mut *src))?;
            tar_unpack(&mut d, dest, keep)?;
            io::copy(&mut d, &mut io::sink())?;
        }
        Format::File => bail!("plain files can't be streamed"),
    }
    Ok(())
}

/// Unpack a tar stream into `dest`; with `keep`, only entries under those paths. Skipped
/// entries are still read through (the caller hashes the whole stream).
fn tar_unpack(r: impl Read, dest: &Path, keep: &[String]) -> Result<()> {
    let mut ar = tar::Archive::new(r);
    ar.set_preserve_permissions(false);
    ar.set_overwrite(false);
    let ctx = || format!("unpacking into {}", dest.display());
    if keep.is_empty() {
        // `unpack` refuses entries that would land outside `dest` (absolute paths, `..`).
        ar.unpack(dest).with_context(ctx)?;
        return Ok(());
    }
    let keep: Vec<PathBuf> = keep.iter().map(|k| without_dots(Path::new(k))).collect();
    fs::create_dir_all(dest).with_context(ctx)?;
    // As `Archive::unpack` does, so the inside-`dest` check compares canonical paths.
    let dest = dest.canonicalize().unwrap_or_else(|_| dest.to_path_buf());
    let kept = |p: &Path| keep.iter().any(|k| p.starts_with(k));
    // As `Archive::unpack` does: folders are created as their files need them and get their own
    // entry (and mode) only at the end, deepest first, so a read-only folder in the archive
    // doesn't stop its files from being written. Folder entries carry no data, so holding them
    // back doesn't disturb the stream.
    let mut dirs = Vec::new();
    for entry in ar.entries().with_context(ctx)? {
        let mut entry = entry.with_context(ctx)?;
        let path = without_dots(&entry.path().with_context(ctx)?);
        // Component-wise: `markers` keeps `markers/x`, not `markers2/x`.
        if !kept(&path) {
            continue;
        }
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            dirs.push((path, entry));
            continue;
        }
        // A link out of the kept folders would point at something never written.
        if kind.is_symlink()
            && let Some(target) = entry.link_name().with_context(ctx)?
            && !link_target(&path, &target).is_some_and(|t| kept(&t))
        {
            bail!(
                "symlink `{}` -> `{}` points outside the kept folders ({}); it would be left dangling. Add its target to the entry's `keep`",
                archive_path(&path),
                archive_path(&target),
                keep.iter().map(|k| archive_path(k)).collect::<Vec<_>>().join(", ")
            );
        }
        // A hard link's target is an archive path; one outside `keep` was never written.
        if kind.is_hard_link()
            && let Some(target) = entry.link_name().with_context(ctx)?
            && !kept(&without_dots(&target))
        {
            bail!(
                "hard link `{}` -> `{}` points outside the kept folders ({}). Add its target to the entry's `keep`",
                archive_path(&path),
                archive_path(&target),
                keep.iter().map(|k| archive_path(k)).collect::<Vec<_>>().join(", ")
            );
        }
        // Same per-entry routine `unpack` uses: `..` and absolute paths can't escape `dest`.
        entry.unpack_in(&dest).with_context(ctx)?;
    }
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, mut entry) in dirs {
        entry.unpack_in(&dest).with_context(ctx)?;
    }
    Ok(())
}

/// An archive path as the archive spells it (`/` separators on every OS), for messages.
fn archive_path(p: &Path) -> String {
    let s = p.to_string_lossy();
    if cfg!(windows) { s.replace('\\', "/") } else { s.into_owned() }
}

/// Where a symlink at archive path `link` with target `target` points, as an archive path;
/// `None` if the target is absolute or climbs above the archive's root.
fn link_target(link: &Path, target: &Path) -> Option<PathBuf> {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in link.parent()?.join(target).components() {
        match c {
            Component::Normal(n) => out.push(n),
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(out)
}

/// `./a/./b` -> `a/b`, so archive paths written with a leading `./` match `keep` prefixes.
fn without_dots(p: &Path) -> PathBuf {
    p.components().filter(|c| !matches!(c, std::path::Component::CurDir)).collect()
}

/// GTDB-Tk's folder hash, ported from `sha1_dir` in `gtdbtk/tools.py` (GTDB-Tk 2.8.0), the
/// function `check_install` (`gtdbtk/misc.py`) compares with `REF_HASHES` (`gtdbtk/config/common.py`):
/// `os.walk(path)` (symlinked folders not entered; links to files are read through), every file's
/// full path, sorted as strings, then one SHA-1 over the files' contents concatenated in that
/// order. Names don't enter the hash, only contents and order.
pub fn sha1_dir(dir: &Path) -> Result<String> {
    fn rec(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
            let path = entry?.path();
            // os.walk sorts entries with `is_dir()`, which follows links: a link to a folder is
            // a folder (not walked, as followlinks=False), anything else is a file.
            if path.is_dir() {
                if !path.symlink_metadata()?.file_type().is_symlink() {
                    rec(&path, out)?;
                }
            } else {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    rec(dir, &mut files)?;
    // Python compares str by code point; for UTF-8 names that is byte order.
    files.sort_by(|a, b| a.as_os_str().as_encoded_bytes().cmp(b.as_os_str().as_encoded_bytes()));
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    for path in &files {
        let mut f = File::open(path).with_context(|| format!("reading {}", path.display()))?;
        loop {
            let n = f.read(&mut buf)?;
            if n == 0 {
                break;
            }
            h.update(&buf[..n]);
        }
    }
    Ok(hex(&h.finalize()))
}

/// Compare each folder's `sha1_dir` with the expected one; returns what differs (empty if all match).
pub fn check_dirs(root: &Path, want: &BTreeMap<String, String>) -> Result<Vec<String>> {
    let mut bad = Vec::new();
    for (dir, expected) in want {
        let path = root.join(dir);
        if !path.is_dir() {
            bad.push(format!("{dir} is missing"));
            continue;
        }
        let got = sha1_dir(&path)?;
        if !got.eq_ignore_ascii_case(expected) {
            bad.push(format!("{dir} hashes to {got}, expected {expected}"));
        }
    }
    Ok(bad)
}

struct HashingReader<R> {
    inner: R,
    hasher: Hashes,
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

fn hash_reader(r: &mut impl Read, h: &mut Hashes) -> Result<()> {
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

thread_local! {
    /// What this thread's messages are about when several things run at once: `"<id>: "` for a
    /// database, plus `"<part>: "` for a part. Empty when one thing runs, so the report reads as
    /// it always did.
    static TAG: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Set the tag this thread's `say` lines and progress lines start with.
pub fn set_tag(tag: String) {
    TAG.with(|t| *t.borrow_mut() = tag);
}

fn tag() -> String {
    TAG.with(|t| t.borrow().clone())
}

/// One line of the fetch report on stderr: `"  <tag><msg>"`.
pub fn say(msg: impl std::fmt::Display) {
    print_line(&format!("  {}{msg}", tag()));
}

/// A whole line on stderr. The live progress line is cleared first, so nothing is written into
/// the middle of it.
pub fn print_line(line: &str) {
    let mut s = screen();
    s.clear_live();
    eprintln!("{line}");
}

/// stderr, shared by every download: the progress of those running, and whether a live
/// progress line (no newline yet) is showing.
struct Screen {
    meters: Vec<Meter>,
    next_id: u64,
    live: bool,
    last_draw: Option<Instant>,
}

static SCREEN: Mutex<Screen> = Mutex::new(Screen { meters: Vec::new(), next_id: 0, live: false, last_draw: None });

fn screen() -> MutexGuard<'static, Screen> {
    relock(SCREEN.lock())
}

fn tty() -> bool {
    static TTY: OnceLock<bool> = OnceLock::new();
    *TTY.get_or_init(|| io::stderr().is_terminal())
}

impl Screen {
    fn clear_live(&mut self) {
        if self.live {
            eprint!("\r\x1b[K");
            self.live = false;
        }
    }

    /// The live line: one download as before, several as one sum.
    fn draw(&mut self) {
        let line = match self.meters.as_slice() {
            [] => return,
            [m] => format!("{}{}", m.label, m.line()),
            ms => {
                let sum = Meter {
                    id: 0,
                    label: String::new(),
                    total: ms.iter().map(|m| m.total).sum(),
                    done: ms.iter().map(|m| m.done).sum(),
                    start: ms.iter().map(|m| m.start).sum(),
                    started: ms.iter().map(|m| m.started).min().unwrap_or_else(Instant::now),
                    last_decile: 0,
                };
                format!("{} downloads: {}", ms.len(), sum.line())
            }
        };
        eprint!("\r  {line}\x1b[K");
        self.live = true;
        self.last_draw = Some(Instant::now());
    }
}

struct Meter {
    id: u64,
    label: String,
    total: u64,
    done: u64,
    /// Bytes already there when it started, left out of the rate.
    start: u64,
    started: Instant,
    last_decile: u64,
}

impl Meter {
    fn line(&self) -> String {
        let secs = self.started.elapsed().as_secs_f64().max(0.001);
        let rate = self.done.saturating_sub(self.start) as f64 / secs;
        let pct = self.done as f64 * 100.0 / self.total.max(1) as f64;
        let left = if rate > 0.0 { (self.total.saturating_sub(self.done)) as f64 / rate } else { 0.0 };
        format!(
            "{} / {} ({pct:.0}%), {}/s, about {} left",
            human(self.done),
            human(self.total),
            human(rate as u64),
            humantime::format_duration(Duration::from_secs(left as u64))
        )
    }
}

/// Progress of one download on stderr: on a terminal one live line (for all downloads running
/// at once, their sum), in logs a line per 10% of each download.
struct Progress {
    id: u64,
    bytes: u64,
}

impl Progress {
    fn new(total: u64, already: u64) -> Self {
        let mut s = screen();
        let id = s.next_id;
        s.next_id += 1;
        let now = Instant::now();
        s.meters.push(Meter { id, label: tag(), total, done: already, start: already, started: now, last_decile: already * 10 / total.max(1) });
        // The first live line comes half a second in.
        s.last_draw = Some(now);
        Progress { id, bytes: already }
    }

    fn add(&mut self, n: u64) {
        self.bytes += n;
        let tty = tty();
        let mut s = screen();
        let Some(m) = s.meters.iter_mut().find(|m| m.id == self.id) else { return };
        m.done += n;
        let decile = m.done * 10 / m.total.max(1);
        let log_line = (!tty && decile > m.last_decile).then(|| {
            m.last_decile = decile;
            format!("  {}{}", m.label, m.line())
        });
        if let Some(line) = log_line {
            eprintln!("{line}");
        } else if tty && s.last_draw.is_none_or(|t| t.elapsed() >= Duration::from_millis(500)) {
            s.draw();
        }
    }

    /// Bytes seen, counting those already there when it started.
    fn bytes(&self) -> u64 {
        self.bytes
    }

    /// On a terminal, leave this download's final figures on a line of their own.
    fn done(&self) {
        if !tty() {
            return;
        }
        let mut s = screen();
        if let Some(line) = s.meters.iter().find(|m| m.id == self.id).map(|m| format!("  {}{}", m.label, m.line())) {
            s.clear_live();
            eprintln!("{line}");
        }
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        screen().meters.retain(|m| m.id != self.id);
    }
}

/// What `walk` found at a path: a regular file of some size, or a symlink (never followed).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Item {
    File(u64),
    Link(String),
}

/// The manifest format new markers get: symlinks listed with their target.
pub const MANIFEST_FORMAT: u32 = 2;

/// Counted files, total bytes and a fingerprint of the folder's file list and sizes. Symlinks
/// count as entries with their target, so retargeting one changes the fingerprint; folders
/// without links get the same fingerprint as before links were recorded.
pub fn manifest(dir: &Path) -> Result<(u64, u64, String)> {
    manifest_as(dir, Some(MANIFEST_FORMAT))
}

/// Everything `walk` finds, sorted; without symlinks for markers from before they were recorded.
fn items_as(dir: &Path, format: Option<u32>) -> Result<Vec<(String, Item)>> {
    let mut items = Vec::new();
    walk(dir, dir, &mut items)?;
    if format.is_none() {
        items.retain(|(_, i)| matches!(i, Item::File(_)));
    }
    items.sort();
    Ok(items)
}

/// `manifest` the way a marker of the given format computed it (`None`: v0.1.0, links left out).
pub fn manifest_as(dir: &Path, format: Option<u32>) -> Result<(u64, u64, String)> {
    let items = items_as(dir, format)?;
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
#[cfg(test)]
pub fn sums(dir: &Path) -> Result<String> {
    sums_as(dir, Some(MANIFEST_FORMAT))
}

/// `sums` the way a marker of the given format computed it (`None`: v0.1.0, links left out).
pub fn sums_as(dir: &Path, format: Option<u32>) -> Result<String> {
    Ok(sums_text(&hashed(dir, format)?))
}

/// One `walk` item with what `sums` records for it: a file's SHA-256 and the size read.
struct Hashed {
    rel: String,
    item: Item,
    /// `(sha256, bytes read)` for a file; `None` for a symlink.
    sum: Option<(String, u64)>,
}

/// Every item `items_as` finds, files hashed: the one read of the data `sums` and `dedupe` share.
fn hashed(dir: &Path, format: Option<u32>) -> Result<Vec<Hashed>> {
    let items = items_as(dir, format)?;
    let mut out = Vec::with_capacity(items.len());
    let mut buf = vec![0u8; 1 << 20];
    for (rel, item) in items {
        let sum = match &item {
            Item::Link(_) => None,
            Item::File(_) => {
                let path = dir.join(&rel);
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
                Some((hex(&h.finalize()), size))
            }
        };
        out.push(Hashed { rel, item, sum });
    }
    Ok(out)
}

fn sums_text(items: &[Hashed]) -> String {
    let mut out = String::new();
    for h in items {
        match (&h.item, &h.sum) {
            (Item::Link(target), _) => out.push_str(&link_line(&h.rel, target)),
            (Item::File(_), Some((sha, size))) => out.push_str(&format!("{sha}  {size}  {}\n", h.rel.replace('\\', "/"))),
            (Item::File(_), None) => unreachable!("files are always hashed"),
        }
    }
    out
}

/// What `dedupe` did.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Deduped {
    /// Files replaced by a hard link to an identical file.
    pub linked: u64,
    /// Bytes those files no longer take on disk.
    pub saved: u64,
    /// Identical files left as copies because the link (or the swap into place) failed.
    pub kept: u64,
}

/// Replace byte-identical regular files under `dir` with hard links to one copy, using the
/// SHA-256 sums already taken for `.dblock.sha256` (files are grouped by size, permissions and
/// SHA-256; nothing is read again). Symlinks are left alone. Each duplicate is swapped in by
/// linking under a temporary name in its own folder and renaming that over it, so a failure at
/// any point leaves the copy as it was. A file that can't be linked (a file system without hard
/// links, a per-file link limit, a read-only folder) is kept as a copy and counted in `kept`;
/// the error of the first one is returned for the caller to mention. Never fails the fetch.
fn dedupe(dir: &Path, items: &[Hashed]) -> (Deduped, Option<io::Error>) {
    let mut groups: BTreeMap<(u64, u32, &str), Vec<&str>> = BTreeMap::new();
    for h in items {
        if let (Item::File(_), Some((sha, size))) = (&h.item, &h.sum)
            && *size > 0
            && let Ok(meta) = fs::symlink_metadata(dir.join(&h.rel))
            && meta.is_file()
        {
            groups.entry((*size, perm_bits(&meta), sha.as_str())).or_default().push(&h.rel);
        }
    }
    let mut done = Deduped::default();
    let mut first_err = None;
    for ((size, _, _), rels) in groups {
        let Some((first, rest)) = rels.split_first() else { continue };
        let mut keep = dir.join(first);
        for rel in rest {
            let dup = dir.join(rel);
            if same_file(&keep, &dup) {
                // Already one file (a hard link in the archive): nothing to save.
                continue;
            }
            match link_over(&keep, &dup) {
                Ok(()) => {
                    done.linked += 1;
                    done.saved += size;
                }
                Err(err) => {
                    done.kept += 1;
                    first_err.get_or_insert(err);
                    // Link later copies to this one: if `keep` hit a link-count limit, this
                    // file still has room.
                    keep = dup;
                }
            }
        }
    }
    (done, first_err)
}

/// Make `dup` a hard link to `keep`: link under a temporary name next to `dup`, then rename it
/// over `dup`. On failure `dup` is untouched and the temporary name removed.
fn link_over(keep: &Path, dup: &Path) -> io::Result<()> {
    let name = dup.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = dup.with_file_name(format!(".dblock-link-{}-{name}", std::process::id()));
    fs::hard_link(keep, &tmp)?;
    let swapped = fs::rename(&tmp, dup);
    // POSIX `rename` between two names of one file succeeds and leaves both: never leave it.
    let _ = fs::remove_file(&tmp);
    swapped
}

/// Permission bits two files must share to become one (a link has one set for every name).
#[cfg(unix)]
fn perm_bits(m: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    m.permissions().mode()
}

#[cfg(not(unix))]
fn perm_bits(m: &fs::Metadata) -> u32 {
    u32::from(m.permissions().readonly())
}

/// Are these two names of one file already?
#[cfg(unix)]
fn same_file(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}

/// Stable Rust has no file id on Windows; relinking two names of one file there is harmless,
/// but is then counted as saved.
#[cfg(not(unix))]
fn same_file(_: &Path, _: &Path) -> bool {
    false
}

fn link_line(rel: &str, target: &str) -> String {
    format!("link  0  {} -> {target}\n", rel.replace('\\', "/"))
}

/// A SHA-256 list with the symlink lines taken out: what the v0.1.0 list of the same folder was.
pub fn without_links(text: &str) -> String {
    text.lines().filter(|l| !l.starts_with("link  0  ")).flat_map(|l| [l, "\n"]).collect()
}

/// Bring a v0.1.0 marker (symlinks left out of its manifest and SHA-256 list) up to the current
/// format, after `existing` has accepted the folder the old way. The new SHA-256 list is the old
/// one (if it still matches the marker) with the symlink lines added, so nothing is re-hashed.
/// Call with the database's lock held.
fn upgrade_marker(dir: &Path, mut l: Locked) -> Result<Locked> {
    let contents = match &l.contents_sha256 {
        None => None,
        Some(want) => {
            let old = fs::read_to_string(dir.join(project::SUMS)).ok().filter(|t| &sha256_hex(t) == want);
            // The list was lost or edited: keep the old format rather than vouch for it.
            let Some(old) = old else { return Ok(l) };
            let mut by_path: BTreeMap<String, String> = old
                .lines()
                .filter_map(|line| Some((line.splitn(3, "  ").nth(2)?.to_string(), format!("{line}\n"))))
                .collect();
            for (rel, item) in items_as(dir, Some(MANIFEST_FORMAT))? {
                if let Item::Link(target) = item {
                    by_path.insert(rel.replace('\\', "/"), link_line(&rel, &target));
                }
            }
            Some(by_path)
        }
    };
    let (files, bytes, manifest_md5) = manifest(dir)?;
    if let Some(by_path) = contents {
        // Same order as `sums`: sorted by path.
        let text: String = by_path.into_values().collect();
        fs::write(dir.join(project::SUMS), &text)?;
        l.contents_sha256 = Some(sha256_hex(&text));
    }
    l.files = files;
    l.bytes = bytes;
    l.manifest_md5 = manifest_md5;
    l.manifest_format = Some(MANIFEST_FORMAT);
    // The marker keeps the paths it was written with; `existing` rewrites them on every read.
    let mut on_disk = project::read_marker(dir).context("marker vanished while upgrading it")?;
    on_disk.files = l.files;
    on_disk.bytes = l.bytes;
    on_disk.manifest_md5 = l.manifest_md5.clone();
    on_disk.contents_sha256 = l.contents_sha256.clone();
    on_disk.manifest_format = l.manifest_format;
    project::write_marker(dir, &on_disk)?;
    say("upgraded its marker to record symlinks (it was written by dblock 0.1.0)");
    Ok(l)
}

pub fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
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
pub(crate) mod tests {
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
        Opts::new(false, false, true, 1)
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
        // Empty temp folders are removed too.
        assert!(!tmp_parent(store.path(), "mydb").exists() && !store.path().join(".tmp").exists());
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
    fn a_folder_moved_from_another_store_is_reused_with_its_marker_updated() {
        let (body, md5) = archive();
        let (url, hits) = serve(body.clone());
        let e = entry(&url, &md5, body.len(), "dir");
        let (first, second) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        obtain(first.path(), &e, &opts()).unwrap();
        fs::rename(first.path().join("mydb"), second.path().join("mydb")).unwrap();
        let (l, reused) = obtain(second.path(), &e, &opts()).unwrap();
        assert!(reused);
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        let here = second.path().join("mydb");
        assert_eq!(l.path, here.display().to_string());
        let marker = project::read_marker(&here).unwrap();
        assert_eq!((marker.path, marker.data_path), (l.path.clone(), l.data_path.clone()));
        assert!(l.data_path.starts_with(&l.path), "{}", l.data_path);
    }

    /// A database folder hard-linked from another store (as the laptop run did to save disk): the
    /// marker is updated here, and the other store's marker, which shared its inode, is not.
    #[test]
    #[cfg(unix)]
    fn a_folder_hard_linked_from_another_store_leaves_that_store_alone() {
        let (body, md5) = archive();
        let (url, hits) = serve(body.clone());
        let e = entry(&url, &md5, body.len(), "dir");
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        let (first, _) = obtain(&a, &e, &opts()).unwrap();
        // cp -al a/mydb b/mydb
        let mut stack = vec![PathBuf::from("mydb")];
        while let Some(rel) = stack.pop() {
            let src = a.join(&rel);
            if src.is_dir() {
                fs::create_dir_all(b.join(&rel)).unwrap();
                stack.extend(fs::read_dir(&src).unwrap().flatten().map(|e| rel.join(e.file_name())));
            } else {
                fs::hard_link(&src, b.join(&rel)).unwrap();
            }
        }
        let marker_a = fs::read_to_string(a.join("mydb").join(project::MARKER)).unwrap();
        let (l, reused) = obtain(&b, &e, &opts()).unwrap();
        assert!(reused);
        assert_eq!(hits.load(Ordering::SeqCst), 1, "reuse downloads nothing");
        assert_eq!(l.fetched, first.fetched, "the data's fetch date is kept");
        let on_disk = project::read_marker(&b.join("mydb")).unwrap();
        assert_eq!((on_disk.path, on_disk.data_path), (l.path.clone(), l.data_path.clone()));
        assert_eq!(fs::read_to_string(a.join("mydb").join(project::MARKER)).unwrap(), marker_a, "the other store's marker changed");
        assert!(!listing(&b.join("mydb")).iter().any(|p| p.contains(".tmp-")), "{:?}", listing(&b.join("mydb")));
        // a file of another size in the linked copy is refused, not reused
        let f = b.join("mydb/mydb/a.txt");
        fs::remove_file(&f).unwrap();
        fs::write(&f, "changed!").unwrap();
        assert!(obtain(&b, &e, &opts()).unwrap_err().to_string().contains("changed since it was fetched"));
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

    #[cfg(unix)]
    #[test]
    fn v010_markers_with_symlinks_are_reused_and_upgraded() {
        let (body, md5) = archive();
        let (url, hits) = serve(body.clone());
        let e = entry(&url, &md5, body.len(), "dir");
        let store = tempfile::tempdir().unwrap();
        obtain(store.path(), &e, &opts()).unwrap();
        // Turn it into what dblock 0.1.0 left: a symlink in the data, and a marker and SHA-256
        // list that leave links out and carry no manifest_format.
        let dir = target_dir(store.path(), &e);
        std::os::unix::fs::symlink("a.txt", dir.join("mydb/current")).unwrap();
        let old_sums = sums_as(&dir, None).unwrap();
        assert!(!old_sums.contains("link"));
        fs::write(dir.join(project::SUMS), &old_sums).unwrap();
        let mut m = project::read_marker(&dir).unwrap();
        let (_, _, old_md5) = manifest_as(&dir, None).unwrap();
        m.manifest_md5 = old_md5.clone();
        m.contents_sha256 = Some(sha256_hex(&old_sums));
        m.manifest_format = None;
        project::write_marker(&dir, &m).unwrap();
        let text = fs::read_to_string(dir.join(project::MARKER)).unwrap();
        assert!(!text.contains("manifest_format"), "{text}");
        // The current format would call it changed; the marker's own format accepts it.
        assert_ne!(manifest(&dir).unwrap().2, old_md5);
        assert!(matches!(existing(store.path(), &e), Stored::Ready(_)));

        let (l, reused) = obtain(store.path(), &e, &opts()).unwrap();
        assert!(reused);
        assert_eq!(hits.load(Ordering::SeqCst), 1, "downloaded again");
        assert_eq!(l.manifest_format, Some(MANIFEST_FORMAT));
        assert_eq!(l.manifest_md5, manifest(&dir).unwrap().2);
        let on_disk = project::read_marker(&dir).unwrap();
        assert_eq!((on_disk.manifest_format, &on_disk.manifest_md5), (Some(MANIFEST_FORMAT), &l.manifest_md5));
        // The upgraded list is exactly what a fresh fetch would write, and matches the marker.
        let new_sums = fs::read_to_string(dir.join(project::SUMS)).unwrap();
        assert_eq!(new_sums, sums(&dir).unwrap());
        assert_eq!(on_disk.contents_sha256, Some(sha256_hex(&new_sums)));
        // Other projects' dblock.lock still hold the 0.1.0 entry: it still verifies the old way.
        assert_eq!(manifest_as(&dir, None).unwrap().2, old_md5);
        assert_eq!(sha256_hex(&without_links(&new_sums)), sha256_hex(&old_sums));
        assert!(matches!(existing(store.path(), &e), Stored::Ready(_)));
    }

    #[test]
    fn oversized_leftover_partial_is_emptied() {
        let (body, md5) = archive();
        let (url, _) = serve(body.clone());
        let e = entry(&url, &md5, body.len(), "dir");
        let store = tempfile::tempdir().unwrap();
        let part = partial_path(store.path(), &e, &e.sources()[0]);
        fs::create_dir_all(part.parent().unwrap()).unwrap();
        fs::write(&part, vec![b'x'; body.len() * 3]).unwrap();
        obtain(store.path(), &e, &opts()).unwrap();
    }

    /// A `.tar.gz` of the given entries (names written as given, `./` included), and its MD5.
    fn archive_of(files: &[(&str, &[u8])]) -> (Vec<u8>, String) {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default()));
        for (name, body) in files {
            let mut h = tar::Header::new_gnu();
            h.set_path(name).unwrap();
            h.set_size(body.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append(&h, *body).unwrap();
        }
        let bytes = tar.into_inner().unwrap().finish().unwrap();
        let md5 = hex(&Md5::digest(&bytes));
        (bytes, md5)
    }

    const MIXED: &[(&str, &[u8])] = &[
        ("db/markers/a.hmm", b"AAA"),
        ("./db/markers/sub/b.hmm", b"BB"),
        ("db/markers2/no.txt", b"not kept: shares a name prefix only"),
        ("db/radii/big.tsv", b"not kept"),
        ("db/masks/m.txt", b"MM"),
        ("readme.txt", b"not kept"),
    ];

    fn kept_entry(url: &str, md5: &str, size: usize) -> Entry {
        let mut e = entry(url, md5, size, "dir");
        e.keep = vec!["db/markers".into(), "./db/masks".into()];
        e
    }

    fn listing(dir: &Path) -> Vec<String> {
        items_as(dir, Some(MANIFEST_FORMAT)).unwrap().into_iter().map(|(r, _)| r.replace('\\', "/")).collect()
    }

    #[test]
    fn keep_unpacks_only_listed_folders_in_both_modes() {
        let (body, md5) = archive_of(MIXED);
        let want = vec!["db/markers/a.hmm", "db/markers/sub/b.hmm", "db/masks/m.txt"];
        for stream in [false, true] {
            let (url, _) = serve(body.clone());
            let e = kept_entry(&url, &md5, body.len());
            let store = tempfile::tempdir().unwrap();
            let (l, _) = obtain(store.path(), &e, &Opts::new(stream, false, true, 1)).unwrap();
            assert_eq!(listing(&target_dir(store.path(), &e)), want, "stream={stream}");
            // Manifest and sums cover the kept files only.
            assert_eq!((l.files, l.bytes), (3, 7), "stream={stream}");
            assert_eq!(l.archive_md5, md5);
            assert!(l.data_path.ends_with("mydb/db") || l.data_path.ends_with("mydb\\db"));
        }
    }

    #[test]
    fn keep_still_checks_the_whole_archive() {
        let (body, _) = archive_of(MIXED);
        // The MD5 of a different archive: the skipped entries are part of what is checked.
        let (_, other_md5) = archive_of(&MIXED[..5]);
        for stream in [false, true] {
            let (url, _) = serve(body.clone());
            let e = kept_entry(&url, &other_md5, body.len());
            let store = tempfile::tempdir().unwrap();
            let err = format!("{:#}", obtain(store.path(), &e, &Opts::new(stream, false, true, 1)).unwrap_err());
            assert!(err.contains("checksum mismatch"), "{err}");
            assert!(!store.path().join("mydb").exists());
        }
    }

    #[test]
    fn keep_path_missing_from_the_archive_is_an_error() {
        let (body, md5) = archive_of(MIXED);
        let (url, _) = serve(body.clone());
        let mut e = kept_entry(&url, &md5, body.len());
        e.keep.push("db/taxonomy".into());
        let store = tempfile::tempdir().unwrap();
        let err = format!("{:#}", obtain(store.path(), &e, &opts()).unwrap_err());
        assert!(err.contains("`db/taxonomy` matched nothing"), "{err}");
        assert!(!store.path().join("mydb").exists());
    }

    /// A `.tar.gz` of raw headers: (path, type, mode, link target, body).
    fn archive_raw(items: &[(&str, tar::EntryType, u32, &str, &[u8])]) -> (Vec<u8>, String) {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default()));
        for (name, kind, mode, link, body) in items {
            let mut h = tar::Header::new_gnu();
            h.set_path(name).unwrap();
            h.set_entry_type(*kind);
            h.set_mode(*mode);
            h.set_size(body.len() as u64);
            if !link.is_empty() {
                h.set_link_name(link).unwrap();
            }
            h.set_cksum();
            tar.append(&h, *body).unwrap();
        }
        let bytes = tar.into_inner().unwrap().finish().unwrap();
        let md5 = hex(&Md5::digest(&bytes));
        (bytes, md5)
    }

    #[cfg(unix)]
    #[test]
    fn keep_unpacks_files_into_read_only_folders_in_both_modes() {
        use std::os::unix::fs::PermissionsExt;
        use tar::EntryType::{Directory, Regular};
        let (body, md5) = archive_raw(&[
            ("db/", Directory, 0o755, "", b""),
            ("db/markers/", Directory, 0o555, "", b""),
            ("db/markers/sub/", Directory, 0o555, "", b""),
            ("db/markers/a", Regular, 0o444, "", b"A"),
            ("db/markers/sub/b", Regular, 0o444, "", b"B"),
            ("db/other/c", Regular, 0o644, "", b"C"),
        ]);
        for stream in [false, true] {
            let (url, _) = serve(body.clone());
            let mut e = entry(&url, &md5, body.len(), "dir");
            e.keep = vec!["db/markers".into()];
            let store = tempfile::tempdir().unwrap();
            let r = obtain(store.path(), &e, &Opts::new(stream, false, true, 1));
            let dir = target_dir(store.path(), &e);
            let markers = dir.join("db/markers");
            let mode = markers.metadata().map(|m| m.permissions().mode() & 0o777);
            // Let the temp folder be cleaned up whatever happened.
            for d in [markers.join("sub"), markers.clone()] {
                let _ = fs::set_permissions(&d, fs::Permissions::from_mode(0o755));
            }
            r.unwrap_or_else(|err| panic!("stream={stream}: {err:#}"));
            assert_eq!(listing(&dir), ["db/markers/a", "db/markers/sub/b"], "stream={stream}");
            assert_eq!(mode.unwrap(), 0o555, "stream={stream}");
        }
    }

    #[test]
    fn kept_symlinks_must_stay_inside_the_kept_folders() {
        use tar::EntryType::{Regular, Symlink};
        let make = |link: &str| {
            archive_raw(&[
                ("db/markers/a", Regular, 0o644, "", b"A"),
                ("db/radii/r", Regular, 0o644, "", b"R"),
                ("db/markers/l", Symlink, 0o777, link, b""),
            ])
        };
        for (link, ok) in [("a", true), ("../markers/a", true), ("../radii/r", false), ("/etc/passwd", false), ("../../../x", false)] {
            let (body, md5) = make(link);
            for stream in [false, true] {
                let (url, _) = serve(body.clone());
                let mut e = entry(&url, &md5, body.len(), "dir");
                e.keep = vec!["db/markers".into()];
                let store = tempfile::tempdir().unwrap();
                let r = obtain(store.path(), &e, &Opts::new(stream, false, true, 1));
                if ok {
                    r.unwrap_or_else(|err| panic!("{link}: {err:#}"));
                } else {
                    let err = format!("{:#}", r.unwrap_err());
                    assert!(err.contains("points outside the kept folders") && err.contains("db/markers/l"), "{link}: {err}");
                    assert!(!target_dir(store.path(), &e).exists());
                }
            }
        }
        // Hard links: the target is an archive path.
        use tar::EntryType::Link;
        for (target, ok) in [("db/markers/a", true), ("./db/markers/a", true), ("db/radii/r", false)] {
            let (body, md5) = archive_raw(&[
                ("db/markers/a", Regular, 0o644, "", b"A"),
                ("db/radii/r", Regular, 0o644, "", b"R"),
                ("db/markers/h", Link, 0o644, target, b""),
            ]);
            let (url, _) = serve(body.clone());
            let mut e = entry(&url, &md5, body.len(), "dir");
            e.keep = vec!["db/markers".into()];
            let store = tempfile::tempdir().unwrap();
            let r = obtain(store.path(), &e, &opts());
            if ok {
                r.unwrap_or_else(|err| panic!("{target}: {err:#}"));
            } else {
                let err = format!("{:#}", r.unwrap_err());
                assert!(err.contains("hard link `db/markers/h`") && err.contains("outside the kept folders"), "{err}");
            }
        }
        assert_eq!(link_target(Path::new("a/b/l"), Path::new("../c")), Some(PathBuf::from("a/c")));
        assert_eq!(link_target(Path::new("a/l"), Path::new("../../c")), None);
    }

    #[test]
    fn filtered_unpack_refuses_paths_outside_dest() {
        // tar::Builder won't write `..`, so patch the name bytes of a raw header.
        let mut h = tar::Header::new_old();
        h.as_old_mut().name[..17].copy_from_slice(b"db/markers/../../");
        h.as_old_mut().name[17..22].copy_from_slice(b"evil\0");
        h.set_size(1);
        h.set_cksum();
        let mut raw = h.as_bytes().to_vec();
        raw.extend_from_slice(b"x");
        raw.resize(512 * 2, 0);
        raw.extend_from_slice(&[0; 1024]);
        let root = tempfile::tempdir().unwrap();
        let dest = root.path().join("d");
        let _ = tar_unpack(&raw[..], &dest, &["db/markers".into()]);
        assert!(!root.path().join("evil").exists());
    }

    #[test]
    fn sha1_dir_matches_gtdbtk_order_and_content() {
        let d = tempfile::tempdir().unwrap();
        fs::create_dir_all(d.path().join("b")).unwrap();
        fs::write(d.path().join("b/x"), b"2").unwrap();
        fs::write(d.path().join("a"), b"1").unwrap();
        // `b.z` sorts before `b/x`: '.' < '/' in Python's string order too.
        fs::write(d.path().join("b.z"), b"9").unwrap();
        fs::write(d.path().join("c"), b"3").unwrap();
        // hashlib.sha1(b"1923").hexdigest(): files a, b.z, b/x, c.
        assert_eq!(sha1_dir(d.path()).unwrap(), hex(&Sha1::digest(b"1923")));
        assert_eq!(sha1_dir(d.path()).unwrap(), "eb72c4ca70ca1394681037753345409cf3d4b488");
    }

    #[cfg(unix)]
    #[test]
    fn sha1_dir_reads_file_links_and_skips_folder_links_like_os_walk() {
        let d = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        fs::write(other.path().join("hidden"), b"zzz").unwrap();
        fs::write(d.path().join("a"), b"1").unwrap();
        std::os::unix::fs::symlink(other.path(), d.path().join("linkdir")).unwrap();
        std::os::unix::fs::symlink(d.path().join("a"), d.path().join("linkfile")).unwrap();
        assert_eq!(sha1_dir(d.path()).unwrap(), hex(&Sha1::digest(b"11")));
    }

    /// Runs GTDB-Tk's own `gtdbtk.tools.sha1_dir` on the same folders when a GTDB-Tk environment
    /// is available (`GTDBTK_PYTHON`, else `/tmp/mm/gtdbtk/bin/python`); skipped otherwise.
    #[test]
    fn sha1_dir_agrees_with_gtdbtk_python_when_available() {
        let python = std::env::var_os("GTDBTK_PYTHON").map(PathBuf::from).unwrap_or_else(|| "/tmp/mm/gtdbtk/bin/python".into());
        if !python.is_file() {
            eprintln!("skipped: no GTDB-Tk Python at {}", python.display());
            return;
        }
        let d = tempfile::tempdir().unwrap();
        for (rel, body) in [("m/a.hmm", &b"alpha"[..]), ("m/sub/b.hmm", b"beta"), ("m/b.z", b"gamma"), ("m/A", b"upper"), ("m/empty", b"")] {
            let p = d.path().join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(d.path().join("m/a.hmm"), d.path().join("m/zlink")).unwrap();
        let out = std::process::Command::new(&python)
            .args(["-I", "-c", "import sys; from gtdbtk.tools import sha1_dir; print(sha1_dir(sys.argv[1], False))"])
            .arg(d.path().join("m"))
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), sha1_dir(&d.path().join("m")).unwrap());
    }

    #[test]
    fn verify_dirs_pass_and_fail() {
        let (body, md5) = archive_of(MIXED);
        // Expected values from `sha1_dir`: contents in path order (markers: a.hmm, sub/b.hmm).
        let markers = hex(&Sha1::digest(b"AAABB"));
        let masks = hex(&Sha1::digest(b"MM"));

        let (url, _) = serve(body.clone());
        let mut e = kept_entry(&url, &md5, body.len());
        e.verify_dirs = BTreeMap::from([("db/markers".to_string(), markers.clone()), ("db/masks".to_string(), masks.clone())]);
        let store = tempfile::tempdir().unwrap();
        let (l, _) = obtain(store.path(), &e, &opts()).unwrap();
        assert_eq!(l.verify_dirs, e.verify_dirs, "recorded for verify --deep");
        assert!(check_dirs(&target_dir(store.path(), &e), &l.verify_dirs).unwrap().is_empty());
        assert_eq!(project::read_marker(&target_dir(store.path(), &e)).unwrap().verify_dirs, e.verify_dirs);
        // Changed afterwards (same size, so plain verify can't see it): check_dirs names it.
        fs::write(target_dir(store.path(), &e).join("db/masks/m.txt"), b"Mm").unwrap();
        let bad = check_dirs(&target_dir(store.path(), &e), &l.verify_dirs).unwrap();
        assert!(bad.len() == 1 && bad[0].starts_with("db/masks hashes to"), "{bad:?}");

        // A wrong expected hash: the fetch fails in either mode and nothing is kept.
        for stream in [false, true] {
            let (url, _) = serve(body.clone());
            let mut e = kept_entry(&url, &md5, body.len());
            e.verify_dirs = BTreeMap::from([("db/markers".to_string(), markers.clone()), ("db/masks".to_string(), "0".repeat(40))]);
            let store = tempfile::tempdir().unwrap();
            let err = format!("{:#}", obtain(store.path(), &e, &Opts::new(stream, false, true, 1)).unwrap_err());
            assert!(err.contains("folder hash mismatch") && err.contains("db/masks hashes to"), "{err}");
            assert!(!err.contains("db/markers"), "{err}");
            assert!(!store.path().join("mydb").exists());
            assert!(!store.path().join(".tmp").exists(), "temporary folder left behind");
        }

        // A listed folder that isn't there.
        let (url, _) = serve(body.clone());
        let mut e = kept_entry(&url, &md5, body.len());
        e.verify_dirs = BTreeMap::from([("db/metadata".to_string(), masks)]);
        let store = tempfile::tempdir().unwrap();
        let err = format!("{:#}", obtain(store.path(), &e, &opts()).unwrap_err());
        assert!(err.contains("db/metadata is missing"), "{err}");
    }

    /// Two identical files in different folders, a third of the same size but different content,
    /// and an empty file twice (nothing to save there).
    const DUPES: &[(&str, &[u8])] = &[
        ("db/a/fasttree.log", b"same bytes in every copy\n"),
        ("db/b/fasttree.log", b"same bytes in every copy\n"),
        ("db/c/other.log", b"SAME BYTES IN EVERY COPY\n"),
        ("db/c/fasttree.log", b"same bytes in every copy\n"),
        ("db/e1", b""),
        ("db/e2", b""),
    ];

    fn fetch_dupes(dedupe: bool) -> (tempfile::TempDir, Entry, Locked) {
        let (body, md5) = archive_of(DUPES);
        let (url, _) = serve(body.clone());
        let e = entry(&url, &md5, body.len(), "dir");
        let store = tempfile::tempdir().unwrap();
        let (l, reused) = obtain(store.path(), &e, &Opts { dedupe, ..opts() }).unwrap();
        assert!(!reused);
        (store, e, l)
    }

    #[cfg(unix)]
    fn ino(p: &Path) -> u64 {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(p).unwrap().ino()
    }

    #[test]
    fn identical_files_are_hard_linked_and_still_verify() {
        let (store, e, l) = fetch_dupes(true);
        let (_plain_store, _, plain) = fetch_dupes(false);
        let db = store.path().join("mydb");
        let at = |rel: &str| db.join(Path::new(rel));
        assert_eq!((l.linked_files, l.linked_bytes), (2, 2 * 25));
        // Every name is still listed, at its full size, and reads back the same.
        assert_eq!((l.files, l.bytes), (plain.files, plain.bytes));
        assert_eq!(l.manifest_md5, plain.manifest_md5);
        assert_eq!(l.contents_sha256, plain.contents_sha256);
        for (name, body) in DUPES {
            assert_eq!(fs::read(at(name)).unwrap(), *body, "{name}");
        }
        #[cfg(unix)]
        {
            let one = ino(&at("db/a/fasttree.log"));
            assert_eq!(ino(&at("db/b/fasttree.log")), one);
            assert_eq!(ino(&at("db/c/fasttree.log")), one);
            assert_ne!(ino(&at("db/c/other.log")), one, "same size, different content: not linked");
            assert_ne!(ino(&at("db/e1")), ino(&at("db/e2")), "empty files are left alone");
        }
        // The recorded marker says the same; reuse, `verify` and `verify --deep` still pass.
        let marker = project::read_marker(&db).unwrap();
        assert_eq!((marker.linked_files, marker.linked_bytes), (2, 50));
        assert!(matches!(existing(store.path(), &e), Stored::Ready(_)));
        assert_eq!(manifest_as(&db, l.manifest_format).unwrap().2, l.manifest_md5);
        assert_eq!(Some(sha256_hex(&sums_as(&db, l.manifest_format).unwrap())), l.contents_sha256);
        // GTDB-Tk's folder hash is over contents in path order: the same as for the copies.
        let mut want = BTreeMap::new();
        want.insert("db".to_string(), sha1_dir(&Path::new(&plain.path).join("db")).unwrap());
        assert!(check_dirs(&db, &want).unwrap().is_empty());
        // No temporary link names left behind.
        assert!(listing(&db).iter().all(|p| !p.contains(".dblock-link-")), "{:?}", listing(&db));
        assert_eq!(listing(&db).len(), DUPES.len());
    }

    #[test]
    fn verify_dirs_are_checked_before_and_hold_after_linking() {
        let (_plain_store, _, plain) = fetch_dupes(false);
        let (body, md5) = archive_of(DUPES);
        let (url, _) = serve(body.clone());
        let mut e = entry(&url, &md5, body.len(), "dir");
        e.verify_dirs.insert("db".into(), sha1_dir(&Path::new(&plain.path).join("db")).unwrap());
        let store = tempfile::tempdir().unwrap();
        let (l, _) = obtain(store.path(), &e, &opts()).unwrap();
        assert_eq!(l.linked_files, 2);
        assert!(check_dirs(Path::new(&l.path), &l.verify_dirs).unwrap().is_empty());
    }

    #[test]
    fn no_dedupe_keeps_copies() {
        let (store, _, l) = fetch_dupes(false);
        assert_eq!((l.linked_files, l.linked_bytes), (0, 0));
        let marker = fs::read_to_string(store.path().join("mydb").join(project::MARKER)).unwrap();
        assert!(!marker.contains("linked_"), "{marker}");
        #[cfg(unix)]
        {
            let db = store.path().join("mydb/db");
            assert_ne!(ino(&db.join("a/fasttree.log")), ino(&db.join("b/fasttree.log")));
        }
    }

    /// A folder the link can't be written into stands in for a file system without hard links:
    /// the copy stays, nothing is left behind, and the error comes back for a note.
    #[cfg(unix)]
    #[test]
    fn a_refused_link_keeps_the_copy() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        for (name, body) in [("a/x", "dup"), ("b/x", "dup")] {
            fs::create_dir_all(d.path().join(name).parent().unwrap()).unwrap();
            fs::write(d.path().join(name), body).unwrap();
        }
        let b = d.path().join("b");
        fs::set_permissions(&b, fs::Permissions::from_mode(0o555)).unwrap();
        if fs::write(b.join("probe"), "").is_ok() {
            // Root ignores folder permissions; nothing to test here.
            let _ = fs::remove_file(b.join("probe"));
            fs::set_permissions(&b, fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }
        let items = hashed(d.path(), Some(MANIFEST_FORMAT)).unwrap();
        let (done, err) = dedupe(d.path(), &items);
        fs::set_permissions(&b, fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(done, Deduped { linked: 0, saved: 0, kept: 1 });
        assert!(err.is_some());
        assert_ne!(ino(&d.path().join("a/x")), ino(&b.join("x")));
        assert_eq!(fs::read_to_string(b.join("x")).unwrap(), "dup");
        assert_eq!(fs::read_dir(&b).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn files_with_different_permissions_are_not_linked() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("run.sh"), "same").unwrap();
        fs::write(d.path().join("data.txt"), "same").unwrap();
        fs::set_permissions(d.path().join("run.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(d.path().join("data.txt"), fs::Permissions::from_mode(0o644)).unwrap();
        let items = hashed(d.path(), Some(MANIFEST_FORMAT)).unwrap();
        assert_eq!(dedupe(d.path(), &items).0.linked, 0);
    }

    #[test]
    fn plan_uses_the_kept_size() {
        let mut e = entry("http://127.0.0.1:9/x", &"0".repeat(32), 1_000, "dir");
        e.extracted = 5_000_000_000_000;
        let store = tempfile::tempdir().unwrap();
        assert!(plan(store.path(), &e, &opts()).is_err(), "5 TB shouldn't fit");
        e.keep = vec!["db".into()];
        e.kept_size = Some(1_000);
        let p = plan(store.path(), &e, &opts()).unwrap();
        assert_eq!(p.need, 2_000);
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

    /// A `.tar.zst` of the given files, compressed with a 2^28 window: above libzstd's default
    /// decoder limit (2^27), like `zstd --long=28`. Plain `zstd -d` refuses it without `--long=28`.
    fn zst_archive(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut enc = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
        enc.long_distance_matching(true).unwrap();
        enc.window_log(28).unwrap();
        let mut tar = tar::Builder::new(enc);
        for (name, body) in files {
            let mut h = tar::Header::new_gnu();
            h.set_path(name).unwrap();
            h.set_size(body.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            tar.append(&h, *body).unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn tar_zst_with_a_long_window_unpacks() {
        let body = zst_archive(&[("db/a.txt", b"hello"), ("db/b.txt", b"world")]);
        // The archive really needs the raised limit: a default decoder refuses it.
        let mut plain = zstd::stream::read::Decoder::new(&body[..]).unwrap();
        assert!(io::copy(&mut plain, &mut io::sink()).is_err(), "default decoder accepted a 2^28 window");
        let md5 = hex(&Md5::digest(&body));
        for stream in [false, true] {
            let (url, _) = serve(body.clone());
            let mut e = entry(&url, &md5, body.len(), "dir");
            e.format = Format::TarZst;
            let store = tempfile::tempdir().unwrap();
            let (l, _) = obtain(store.path(), &e, &Opts::new(stream, false, true, 1))
                .unwrap_or_else(|err| panic!("stream={stream}: {err:#}"));
            assert_eq!(listing(&target_dir(store.path(), &e)), ["db/a.txt", "db/b.txt"], "stream={stream}");
            assert_eq!((l.files, l.bytes), (2, 10));
        }
    }

    /// HTTP server for several files by path, honouring `Range: bytes=N-`. Records each request
    /// as `path` or `path@N` for a range request.
    fn serve_files(files: Vec<(String, Vec<u8>)>) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
        let srv = serve_paced(files, Vec::new());
        (srv.base, srv.log)
    }

    pub(crate) struct Server {
        pub base: String,
        pub log: Arc<std::sync::Mutex<Vec<String>>>,
        /// Most requests that were being answered at the same time.
        pub peak: Arc<AtomicUsize>,
    }

    /// `serve_files`, sending the files named in `pause` in three pieces with that many
    /// milliseconds between them, so downloads overlap (or are still running) when a test needs it.
    pub(crate) fn serve_paced(files: Vec<(String, Vec<u8>)>, pause: Vec<(String, u64)>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}/rel", listener.local_addr().unwrap());
        let log = Arc::new(std::sync::Mutex::new(Vec::new()));
        let (active, peak) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let srv = Server { base, log: log.clone(), peak: peak.clone() };
        let files = Arc::new(files);
        let pause = Arc::new(pause);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (files, pause, seen, active, peak) = (files.clone(), pause.clone(), log.clone(), active.clone(), peak.clone());
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut first = String::new();
                    reader.read_line(&mut first).unwrap();
                    let path = first.split_whitespace().nth(1).unwrap_or("").rsplit('/').next().unwrap_or("").to_string();
                    let mut from = 0usize;
                    let mut line = String::new();
                    while reader.read_line(&mut line).is_ok_and(|n| n > 2) {
                        if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                            from = v.trim().trim_end_matches('-').parse().unwrap_or(0);
                        }
                        line.clear();
                    }
                    seen.lock().unwrap().push(if from > 0 { format!("{path}@{from}") } else { path.clone() });
                    let mut stream = stream;
                    let Some((_, body)) = files.iter().find(|(n, _)| *n == path) else {
                        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                        return;
                    };
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    let head = if from > 0 {
                        format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {from}-{}/{}\r\nConnection: close\r\n\r\n",
                            body.len() - from,
                            body.len() - 1,
                            body.len()
                        )
                    } else {
                        format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                    };
                    stream.write_all(head.as_bytes()).unwrap();
                    let rest = &body[from..];
                    match pause.iter().find(|(n, _)| *n == path) {
                        Some((_, ms)) => {
                            let third = rest.len().div_ceil(3).max(1);
                            for (i, piece) in rest.chunks(third).enumerate() {
                                if i > 0 {
                                    std::thread::sleep(Duration::from_millis(*ms));
                                }
                                if stream.write_all(piece).and_then(|()| stream.flush()).is_err() {
                                    break;
                                }
                            }
                        }
                        None => {
                            let _ = stream.write_all(rest);
                        }
                    }
                    active.fetch_sub(1, Ordering::SeqCst);
                });
            }
        });
        srv
    }

    /// Three `.tar.zst` parts that all unpack under `rel/`, like the GTDB placement mirror.
    pub(crate) fn three_parts() -> Vec<(String, Vec<u8>)> {
        vec![
            ("p-core.tar.zst".into(), zst_archive(&[("rel/markers/a.hmm", b"AAA"), ("rel/masks/m.txt", b"MM")])),
            ("p-split.tar.zst".into(), zst_archive(&[("rel/split/s1", &[b's'; 3000][..]), ("rel/split/s2", b"S2")])),
            ("p-tree.tar.zst".into(), zst_archive(&[("rel/pplacer/tree", b"TREE")])),
        ]
    }

    /// A multi-part catalog entry for `parts` (as served at `base`), loaded through the catalog
    /// so URLs, size and the combined MD5 are filled in the way `dblock` does it.
    fn multi_entry(dir: &Path, base: &str, parts: &[(String, Vec<u8>)], sha256: bool) -> Entry {
        multi_entry_id(dir, "mydb", base, parts, sha256)
    }

    fn multi_entry_id(dir: &Path, id: &str, base: &str, parts: &[(String, Vec<u8>)], sha256: bool) -> Entry {
        let parts: Vec<_> = parts
            .iter()
            .map(|(name, body)| {
                let mut p = serde_json::json!({"file": name, "size": body.len(), "md5": hex(&Md5::digest(body))});
                if sha256 {
                    p["sha256"] = hex(&Sha256::digest(body)).into();
                }
                p
            })
            .collect();
        let json = serde_json::json!([{
            "id": id, "name": "t", "version": "1", "tool": "t", "format": "tar.zst", "base_url": base,
            "parts": parts, "extracted": 100, "md5_source": "test", "env": "MYDB", "compat": null,
            "licence": "x", "verify_dirs": {"rel/markers": hex(&Sha1::digest(b"AAA"))}
        }]);
        let cat = dir.join("cat.json");
        fs::write(&cat, json.to_string()).unwrap();
        let entries = crate::catalog::load(Some(&cat)).unwrap();
        crate::catalog::find(&entries, id).unwrap().clone()
    }

    #[test]
    fn multi_part_entry_unpacks_into_one_folder_in_both_modes() {
        let files = three_parts();
        let want = ["rel/markers/a.hmm", "rel/masks/m.txt", "rel/pplacer/tree", "rel/split/s1", "rel/split/s2"];
        for stream in [false, true] {
            let (base, log) = serve_files(files.clone());
            let dir = tempfile::tempdir().unwrap();
            let e = multi_entry(dir.path(), &base, &files, true);
            assert_eq!(e.size, files.iter().map(|(_, b)| b.len() as u64).sum::<u64>());
            let store = dir.path().join("store");
            let (l, _) = obtain(&store, &e, &Opts::new(stream, false, true, 1))
                .unwrap_or_else(|err| panic!("stream={stream}: {err:#}"));
            assert_eq!(listing(&target_dir(&store, &e)), want, "stream={stream}");
            assert_eq!(l.files, 5);
            assert_eq!(l.archive_bytes, e.size);
            assert_eq!(l.archive_md5, e.md5);
            assert_eq!(l.parts.len(), 3);
            assert_eq!(l.parts[1].file, "p-split.tar.zst");
            assert_eq!(l.parts[1].url, format!("{base}/p-split.tar.zst"));
            assert!(l.parts.iter().all(|p| p.sha256.is_some()));
            assert!(Path::new(&l.data_path).ends_with("mydb/rel"), "{}", l.data_path);
            assert_eq!(log.lock().unwrap().len(), 3, "one request per part");
            // Archives are removed once unpacked, and the store reuses the folder next time.
            assert!(fs::read_dir(store.join(".partial")).map(|d| d.count() == 0).unwrap_or(true));
            let (again, reused) = obtain(&store, &e, &opts()).unwrap();
            assert!(reused && again.manifest_md5 == l.manifest_md5);
            // The lock file round-trips the parts.
            let mut lock = project::Lock::default();
            lock.upsert(l.clone());
            lock.save(dir.path()).unwrap();
            assert_eq!(project::Lock::load(dir.path()).unwrap().databases[0].parts, l.parts);
        }
    }

    #[test]
    fn corrupt_part_keeps_nothing_and_the_rest_resumes() {
        let good = three_parts();
        let mut bad = good.clone();
        // Same size, one byte flipped: only the checksums can tell.
        let last = bad[1].1.len() - 1;
        bad[1].1[last] ^= 0xff;
        for sha256 in [true, false] {
            let (base, log) = serve_files(bad.clone());
            let dir = tempfile::tempdir().unwrap();
            let e = multi_entry(dir.path(), &base, &good, sha256);
            let store = dir.path().join("store");
            let err = format!("{:#}", obtain(&store, &e, &opts()).unwrap_err());
            assert!(err.contains("checksum mismatch for mydb (part p-split.tar.zst)"), "{err}");
            assert!(err.contains("md5") && err.contains("sha256") == sha256, "{err}");
            assert!(!target_dir(&store, &e).exists(), "kept a database with a corrupt part");
            assert!(!store.join(".tmp").exists(), "temporary folder left behind");
            assert_eq!(*log.lock().unwrap(), ["p-core.tar.zst", "p-split.tar.zst"], "stopped at the bad part");
            // The good part stays downloaded; the bad one is deleted.
            let sources = e.sources();
            assert!(partial_path(&store, &e, &sources[0]).is_file());
            assert!(!partial_path(&store, &e, &sources[1]).exists());

            // Streamed: also nothing kept.
            let err = format!("{:#}", obtain(&store, &e, &Opts::new(true, false, true, 1)).unwrap_err());
            assert!(err.contains("part p-split.tar.zst") && err.contains("Nothing was kept"), "{err}");
            assert!(!target_dir(&store, &e).exists());
        }

        // Fixed upstream: the next fetch re-reads part 1 from disk instead of downloading it,
        // resumes part 2 from where an interrupted download stopped, and gets part 3.
        let (base, log) = serve_files(good.clone());
        let dir = tempfile::tempdir().unwrap();
        let e = multi_entry(dir.path(), &base, &good, true);
        let store = dir.path().join("store");
        let sources = e.sources();
        fs::create_dir_all(store.join(".partial")).unwrap();
        fs::write(partial_path(&store, &e, &sources[0]), &good[0].1).unwrap();
        fs::write(partial_path(&store, &e, &sources[1]), &good[1].1[..100]).unwrap();
        let p = plan(&store, &e, &opts()).unwrap();
        assert_eq!(p.resumable, good[0].1.len() as u64 + 100);
        let (l, _) = obtain(&store, &e, &opts()).unwrap();
        assert_eq!(*log.lock().unwrap(), ["p-split.tar.zst@100", "p-tree.tar.zst"]);
        assert_eq!(l.files, 5);
        assert!(check_dirs(&target_dir(&store, &e), &e.verify_dirs).unwrap().is_empty());
    }

    #[test]
    fn stale_partials_are_downloaded_again_in_the_same_run() {
        // Partials left by an earlier release with the same file names: one complete but different,
        // one a wrong prefix. Each fails its check once, then downloads from the start.
        let good = three_parts();
        let (base, log) = serve_files(good.clone());
        let dir = tempfile::tempdir().unwrap();
        let e = multi_entry(dir.path(), &base, &good, true);
        let store = dir.path().join("store");
        let sources = e.sources();
        fs::create_dir_all(store.join(".partial")).unwrap();
        let mut stale = good[0].1.clone();
        stale[0] ^= 0xff;
        fs::write(partial_path(&store, &e, &sources[0]), &stale).unwrap();
        fs::write(partial_path(&store, &e, &sources[1]), vec![b'x'; 100]).unwrap();
        let (l, _) = obtain(&store, &e, &opts()).unwrap_or_else(|err| panic!("{err:#}"));
        assert_eq!(*log.lock().unwrap(), ["p-core.tar.zst", "p-split.tar.zst@100", "p-split.tar.zst", "p-tree.tar.zst"]);
        assert_eq!(l.files, 5);
    }

    #[test]
    fn multi_part_folder_hash_mismatch_keeps_nothing() {
        let files = three_parts();
        let (base, _) = serve_files(files.clone());
        let dir = tempfile::tempdir().unwrap();
        let mut e = multi_entry(dir.path(), &base, &files, true);
        e.verify_dirs.insert("rel/pplacer".into(), "0".repeat(40));
        let store = dir.path().join("store");
        let err = format!("{:#}", obtain(&store, &e, &opts()).unwrap_err());
        assert!(err.contains("folder hash mismatch") && err.contains("rel/pplacer"), "{err}");
        assert!(!target_dir(&store, &e).exists());
    }

    #[test]
    fn plan_sums_the_parts() {
        let dir = tempfile::tempdir().unwrap();
        let files = three_parts();
        let mut e = multi_entry(dir.path(), "http://127.0.0.1:9/x", &files, false);
        e.extracted = 1_000;
        let p = plan(&dir.path().join("store"), &e, &opts()).unwrap();
        assert_eq!(p.need, e.size + 1_000);
        assert_eq!(e.on_disk(), 1_000);
    }

    /// Loose files (AMRFinderPlus's layout): each part is kept as is, under its name.
    fn loose_entry(dir: &Path, base: &str, files: &[(String, Vec<u8>)], prepare: Option<&str>) -> Entry {
        let parts: Vec<_> = files
            .iter()
            .map(|(name, body)| serde_json::json!({"file": name, "size": body.len(), "md5": hex(&Md5::digest(body)), "sha256": hex(&Sha256::digest(body))}))
            .collect();
        let mut json = serde_json::json!({
            "id": "loose", "name": "t", "version": "1", "tool": "t", "format": "file", "base_url": base,
            "parts": parts, "extracted": 50, "md5_source": "test", "env": "MYDB", "compat": null, "licence": "x"
        });
        if let Some(p) = prepare {
            json["prepare"] = p.into();
        }
        let cat = dir.join("cat.json");
        fs::write(&cat, serde_json::json!([json]).to_string()).unwrap();
        let entries = crate::catalog::load(Some(&cat)).unwrap();
        crate::catalog::find(&entries, "loose").unwrap().clone()
    }

    #[test]
    fn loose_files_are_kept_as_they_are() {
        let files: Vec<(String, Vec<u8>)> =
            vec![("AMRProt.fa".into(), b">p1\nMKV\n".to_vec()), ("version.txt".into(), b"2026-10-29.1\n".to_vec())];
        // --stream can't apply to plain files; it falls back to downloading them.
        for stream in [false, true] {
            let (base, log) = serve_files(files.clone());
            let dir = tempfile::tempdir().unwrap();
            let e = loose_entry(dir.path(), &base, &files, None);
            assert_eq!((e.size, e.on_disk()), (21, 50));
            let store = dir.path().join("store");
            let (l, _) = obtain(&store, &e, &Opts::new(stream, false, true, 1))
                .unwrap_or_else(|err| panic!("stream={stream}: {err:#}"));
            let db = target_dir(&store, &e);
            assert_eq!(listing(&db), ["AMRProt.fa", "version.txt"]);
            assert_eq!(fs::read(db.join("version.txt")).unwrap(), b"2026-10-29.1\n");
            assert_eq!(Path::new(&l.data_path), db, "the folder itself, not one file in it");
            assert_eq!((l.parts.len(), l.prepared.as_deref()), (2, None));
            assert_eq!(log.lock().unwrap().len(), 2);
        }
        // A corrupt file keeps nothing.
        let mut bad = files.clone();
        bad[1].1[0] = b'3';
        let (base, _) = serve_files(bad);
        let dir = tempfile::tempdir().unwrap();
        let e = loose_entry(dir.path(), &base, &files, None);
        let err = format!("{:#}", obtain(&dir.path().join("store"), &e, &opts()).unwrap_err());
        assert!(err.contains("checksum mismatch") && err.contains("version.txt"), "{err}");
        assert!(!target_dir(&dir.path().join("store"), &e).exists());
    }

    #[cfg(unix)]
    #[test]
    fn prepare_runs_on_the_folder_and_reports_its_version() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let prog = dir.path().join("amrfinder_index");
        fs::write(&prog, "#!/bin/sh\n[ \"$1\" = --version ] && { echo 4.2.7; exit 0; }\n[ -f \"$1/AMRProt.fa\" ] || { echo no db >&2; exit 1; }\necho idx > \"$1/AMRProt.fa.phr\"\n").unwrap();
        fs::set_permissions(&prog, fs::Permissions::from_mode(0o755)).unwrap();
        let db = dir.path().join("db");
        fs::create_dir(&db).unwrap();
        let err = format!("{:#}", prepare(&prog, &db).unwrap_err());
        assert!(err.contains("amrfinder_index failed") && err.contains("no db"), "{err}");
        fs::write(db.join("AMRProt.fa"), ">p\nM\n").unwrap();
        assert_eq!(prepare(&prog, &db).unwrap(), "amrfinder_index 4.2.7");
        assert!(db.join("AMRProt.fa.phr").is_file());
        let err = format!("{:#}", prepare(&dir.path().join("missing"), &db).unwrap_err());
        assert!(err.contains("could not be run") && err.contains("ncbi-amrfinderplus"), "{err}");
    }

    /// Without amrfinder_index on PATH, a database that needs it is refused before anything is
    /// downloaded (it used to be downloaded, verified, then thrown away).
    #[test]
    fn missing_prepare_program_is_refused_before_downloading() {
        if find_program("amrfinder_index").is_some() {
            return; // AMRFinderPlus is installed here; the error can't be provoked.
        }
        let files: Vec<(String, Vec<u8>)> = vec![("AMRProt.fa".into(), b">p1\nMKV\n".to_vec())];
        let dir = tempfile::tempdir().unwrap();
        // nothing listens on port 1: a download attempt would fail with a connection error instead
        let e = loose_entry(dir.path(), "http://127.0.0.1:1", &files, Some("amrfinder_index"));
        assert_eq!(missing_prepare(&e), Some("amrfinder_index"));
        let store = dir.path().join("store");
        let err = format!("{:#}", obtain(&store, &e, &opts()).unwrap_err());
        assert!(err.contains("amrfinder_index is needed to prepare loose") && err.contains("nothing was downloaded"), "{err}");
        assert!(!target_dir(&store, &e).exists());
        assert!(!store.join(".tmp").exists(), "temporary folder left behind");
    }

    #[test]
    fn programs_are_found_on_path_or_by_path() {
        assert!(find_program("no-such-program-dblock-test").is_none());
        let dir = tempfile::tempdir().unwrap();
        let prog = dir.path().join("tool");
        fs::write(&prog, "").unwrap();
        assert_eq!(find_program(prog.to_str().unwrap()), Some(prog));
        assert!(find_program(dir.path().join("absent").to_str().unwrap()).is_none());
    }

    fn paced(files: &[(String, Vec<u8>)], ms: u64) -> Vec<(String, u64)> {
        files.iter().map(|(n, _)| (n.clone(), ms)).collect()
    }

    #[test]
    fn parts_download_side_by_side_up_to_the_job_limit() {
        let files = three_parts();
        let want = ["rel/markers/a.hmm", "rel/masks/m.txt", "rel/pplacer/tree", "rel/split/s1", "rel/split/s2"];
        // (jobs, stream, most downloads at once): streamed parts go one after another.
        for (jobs, stream, most) in [(1, false, 1), (2, false, 2), (4, false, 3), (4, true, 1)] {
            let srv = serve_paced(files.clone(), paced(&files, 150));
            let dir = tempfile::tempdir().unwrap();
            let e = multi_entry(dir.path(), &srv.base, &files, true);
            let store = dir.path().join("store");
            let (l, _) = obtain(&store, &e, &Opts::new(stream, false, true, jobs)).unwrap_or_else(|err| panic!("jobs={jobs}: {err:#}"));
            assert_eq!(listing(&target_dir(&store, &e)), want, "jobs={jobs} stream={stream}");
            assert_eq!(srv.peak.load(Ordering::SeqCst), most, "jobs={jobs} stream={stream}");
            assert_eq!(srv.log.lock().unwrap().len(), 3, "one request per part");
            // The lock lists the parts in catalog order, whatever order they finished in.
            let names: Vec<_> = l.parts.iter().map(|p| p.file.as_str()).collect();
            assert_eq!(names, ["p-core.tar.zst", "p-split.tar.zst", "p-tree.tar.zst"]);
            assert!(l.parts.iter().all(|p| p.url == format!("{}/{}", srv.base, p.file)));
            assert!(check_dirs(&target_dir(&store, &e), &e.verify_dirs).unwrap().is_empty());
            assert!(fs::read_dir(store.join(".partial")).map(|d| d.count() == 0).unwrap_or(true));
        }
    }

    #[test]
    fn loose_files_download_side_by_side() {
        let files: Vec<(String, Vec<u8>)> = (0..6).map(|i| (format!("f{i}.txt"), format!("file {i}\n").repeat(50).into_bytes())).collect();
        let srv = serve_paced(files.clone(), paced(&files, 100));
        let dir = tempfile::tempdir().unwrap();
        let e = loose_entry(dir.path(), &srv.base, &files, None);
        let store = dir.path().join("store");
        let (l, _) = obtain(&store, &e, &Opts::new(false, false, true, 4)).unwrap_or_else(|err| panic!("{err:#}"));
        assert_eq!(srv.peak.load(Ordering::SeqCst), 4, "four at a time, never more");
        assert_eq!(listing(&target_dir(&store, &e)).len(), 6);
        assert_eq!(fs::read(target_dir(&store, &e).join("f5.txt")).unwrap(), files[5].1);
        assert_eq!(l.parts.len(), 6);
    }

    #[test]
    fn corrupt_part_among_parallel_downloads_keeps_nothing_and_the_rest_resumes() {
        let good = three_parts();
        let mut bad = good.clone();
        let last = bad[1].1.len() - 1;
        bad[1].1[last] ^= 0xff;
        // Part 1 arrives at once; part 2 (corrupt) takes ~300 ms; part 3 is slower and still
        // coming when part 2 fails.
        let srv = serve_paced(bad, vec![("p-split.tar.zst".into(), 150), ("p-tree.tar.zst".into(), 400)]);
        let dir = tempfile::tempdir().unwrap();
        let e = multi_entry(dir.path(), &srv.base, &good, true);
        let store = dir.path().join("store");
        let err = format!("{:#}", obtain(&store, &e, &Opts::new(false, false, true, 3)).unwrap_err());
        assert!(err.contains("checksum mismatch for mydb (part p-split.tar.zst)"), "{err}");
        assert!(!err.contains("another part failed"), "reported the stopped download, not the failure: {err}");
        assert!(!target_dir(&store, &e).exists(), "kept a database with a corrupt part");
        assert!(!store.join(".tmp").exists(), "temporary folder left behind");
        assert_eq!(srv.log.lock().unwrap().len(), 3, "all three started");
        let sources = e.sources();
        // Part 1 is all there, the corrupt part is gone, and part 3 stopped part way.
        assert_eq!(fs::metadata(partial_path(&store, &e, &sources[0])).unwrap().len(), good[0].1.len() as u64);
        assert!(!partial_path(&store, &e, &sources[1]).exists());
        let tree = fs::metadata(partial_path(&store, &e, &sources[2])).unwrap().len();
        assert!(tree > 0 && tree < good[2].1.len() as u64, "part 3 has {tree} of {} bytes", good[2].1.len());

        // Fixed upstream: part 1 is re-read from disk, part 2 downloaded again, part 3 resumed.
        let srv = serve_paced(good.clone(), paced(&good, 50));
        let e = multi_entry(dir.path(), &srv.base, &good, true);
        let (l, _) = obtain(&store, &e, &Opts::new(false, false, true, 3)).unwrap_or_else(|err| panic!("{err:#}"));
        let mut log = srv.log.lock().unwrap().clone();
        log.sort();
        assert_eq!(log, ["p-split.tar.zst".to_string(), format!("p-tree.tar.zst@{tree}")]);
        assert_eq!(l.files, 5);
        assert!(check_dirs(&target_dir(&store, &e), &e.verify_dirs).unwrap().is_empty());
        assert!(fs::read_dir(store.join(".partial")).map(|d| d.count() == 0).unwrap_or(true));
    }

    #[test]
    fn databases_fetched_side_by_side_share_the_job_limit() {
        let files = three_parts();
        let srv = serve_paced(files.clone(), paced(&files, 150));
        let dir = tempfile::tempdir().unwrap();
        let (da, db) = (dir.path().join("a"), dir.path().join("b"));
        fs::create_dir_all(&da).unwrap();
        fs::create_dir_all(&db).unwrap();
        let a = multi_entry_id(&da, "dba", &srv.base, &files, true);
        let b = multi_entry_id(&db, "dbb", &srv.base, &files, true);
        let store = dir.path().join("store");
        let opts = Opts::new(false, false, true, 2);
        std::thread::scope(|s| {
            let ha = s.spawn(|| obtain(&store, &a, &opts));
            let hb = s.spawn(|| obtain(&store, &b, &opts));
            for h in [ha, hb] {
                let (l, reused) = h.join().unwrap().unwrap_or_else(|err| panic!("{err:#}"));
                assert!(!reused && l.files == 5);
            }
        });
        assert_eq!(srv.log.lock().unwrap().len(), 6);
        assert_eq!(srv.peak.load(Ordering::SeqCst), 2, "two downloads at a time over both databases");
        assert_eq!(*opts.space.claimed.lock().unwrap(), (0, 0), "space claims given back");
    }

    #[test]
    fn a_fetch_waits_while_others_have_claimed_the_space_it_needs() {
        let dir = tempfile::tempdir().unwrap();
        let files = three_parts();
        let e = multi_entry(dir.path(), "http://127.0.0.1:9/x", &files, false);
        let store = dir.path().join("store");
        let opts = Opts::new(false, false, true, 2);
        // A little claimed by another fetch: the plan doesn't change, so no waiting.
        *opts.space.claimed.lock().unwrap() = (1_000, 1);
        let (p, claim) = plan_claim(&store, &e, &opts).unwrap();
        assert_eq!(p.mode, Mode::DownloadThenExtract);
        assert_eq!(*opts.space.claimed.lock().unwrap(), (1_000 + p.need, 2));
        drop(claim);
        // All of it claimed: wait until the other fetch is done, then plan as if alone.
        *opts.space.claimed.lock().unwrap() = (u64::MAX / 2, 1);
        std::thread::scope(|s| {
            let h = s.spawn(|| plan_claim(&store, &e, &opts).map(|(p, c)| (p.mode, c.bytes)));
            std::thread::sleep(Duration::from_millis(300));
            assert!(!h.is_finished(), "planned against space another fetch had claimed");
            *opts.space.claimed.lock().unwrap() = (0, 0);
            opts.space.released.notify_all();
            let (mode, bytes) = h.join().unwrap().unwrap();
            assert_eq!((mode, bytes), (Mode::DownloadThenExtract, plan(&store, &e, &opts).unwrap().need));
        });
        assert_eq!(*opts.space.claimed.lock().unwrap(), (0, 0));
    }
}
