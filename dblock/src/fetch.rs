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
//! Either way the contents land in a temporary folder and only take their final name once
//! the checksum matches, so a half-finished or corrupt database is never mistaken for a good one.

use crate::catalog::{Entry, Format, human};
use crate::project::{self, Locked};
use anyhow::{Context, Result, bail};
use md5::{Digest, Md5};
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

/// Already in the store, intact, and the same release the catalog describes?
pub fn existing(store: &Path, e: &Entry) -> Option<Locked> {
    let dir = target_dir(store, e);
    let marker = project::read_marker(&dir)?;
    (marker.archive_md5 == e.md5).then_some(marker)
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

pub fn fetch(store: &Path, e: &Entry, plan: &Plan, opts: &Opts) -> Result<Locked> {
    let tmp = store.join(format!(".tmp-{}", e.id));
    if tmp.exists() {
        fs::remove_dir_all(&tmp).with_context(|| format!("clearing leftover {}", tmp.display()))?;
    }
    fs::create_dir_all(&tmp)?;

    let mut last_err = None;
    let mut used_url = None;
    for url in &e.urls {
        let r = match plan.mode {
            Mode::DownloadThenExtract => download_then_extract(store, e, url, &tmp, opts),
            Mode::Stream => stream_extract(e, url, &tmp),
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
                    let _ = fs::remove_dir_all(&tmp);
                    return Err(err);
                }
                last_err = Some(err);
                let _ = fs::remove_dir_all(&tmp);
                fs::create_dir_all(&tmp)?;
            }
        }
    }
    let Some(url) = used_url else {
        let _ = fs::remove_dir_all(&tmp);
        return Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no URL worked")));
    };

    let target = target_dir(store, e);
    if target.exists() {
        bail!(
            "{} already exists but is a different download (checksum differs from the catalog); move or delete it, then fetch again",
            target.display()
        );
    }
    fs::rename(&tmp, &target).with_context(|| format!("moving into {}", target.display()))?;

    let (files, bytes, manifest_md5) = manifest(&target)?;
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
        data_path: data_path(&target, &e.env_points_to)?.display().to_string(),
        files,
        bytes,
        manifest_md5,
        env: e.env.clone(),
        licence: e.licence.clone(),
        non_commercial: e.non_commercial,
    };
    project::write_marker(&target, &locked)?;
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
        have = 0; // a different file was left here
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

/// Counted files, total bytes and a fingerprint of the folder's file list and sizes.
pub fn manifest(dir: &Path) -> Result<(u64, u64, String)> {
    let mut items = Vec::new();
    walk(dir, dir, &mut items)?;
    items.sort();
    let mut h = Md5::new();
    let mut bytes = 0;
    for (rel, size) in &items {
        h.update(format!("{rel}\t{size}\n").as_bytes());
        bytes += size;
    }
    Ok((items.len() as u64, bytes, hex(&h.finalize())))
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        let ft = entry.file_type()?;
        if ft.is_dir() {
            walk(root, &path, out)?;
        } else if ft.is_file() {
            let rel = path.strip_prefix(root)?.to_string_lossy().into_owned();
            if rel != project::MARKER {
                out.push((rel, entry.metadata()?.len()));
            }
        }
    }
    Ok(())
}

/// The path a tool should be given: the database folder (or the single folder inside it,
/// as GTDB-Tk's tarball wraps everything in `release232/`), or one file matched by `glob:*.ext`.
fn data_path(target: &Path, points_to: &str) -> Result<PathBuf> {
    if let Some(pattern) = points_to.strip_prefix("glob:") {
        let suffix = pattern.trim_start_matches('*');
        let mut items = Vec::new();
        walk(target, target, &mut items)?;
        let hits: Vec<_> = items.iter().filter(|(r, _)| r.ends_with(suffix)).collect();
        return match hits.as_slice() {
            [(one, _)] => Ok(target.join(one)),
            [] => bail!("no {pattern} file in {}", target.display()),
            _ => Ok(target.to_path_buf()),
        };
    }
    let inside: Vec<_> = fs::read_dir(target)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name() != project::MARKER)
        .collect();
    if let [only] = inside.as_slice()
        && only.file_type()?.is_dir() {
            return Ok(only.path());
        }
    Ok(target.to_path_buf())
}
