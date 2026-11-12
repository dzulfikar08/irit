//! Where full reference sketches are read from: byte ranges of `sketches.db`.
//!
//! - a local `sketches.db` (or any copy of it on disk);
//! - an HTTP(S) URL of an unpacked `sketches.db`, read with Range requests;
//! - the same split into numbered parts (`...sketches.db.{part}`, fixed part size), the shape a
//!   GitHub Release needs (assets are limited to 2 GiB);
//! - GTDB's own `.tar.gz`, through a gzip seek index (see `gz.rs`).

use super::gz::GzIndex;
use anyhow::{Context, Result, bail, ensure};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Duration;

/// Default number of range requests in flight at once.
pub const DEFAULT_JOBS: usize = 4;
/// Upper limit on requests in flight, to keep the load on one server modest.
pub const MAX_JOBS: usize = 16;

/// How failed HTTP requests are retried: connection errors, timeouts, short bodies and the
/// statuses 408, 429, 500, 502, 503 and 504. Other answers (a 200 that ignores the range, 404,
/// 416) fail at once. The wait doubles from `base` up to `max`, with jitter, and a `Retry-After`
/// header (in seconds) is honoured up to `max`.
///
/// A request that stalls is cut off and retried: the server must start answering within `stall`,
/// and the body must arrive within `stall` plus its size at `min_rate` bytes per second.
#[derive(Debug, Clone, Copy)]
pub struct Retry {
    /// Attempts in all, including the first.
    pub attempts: u32,
    pub base: Duration,
    pub max: Duration,
    pub stall: Duration,
    pub min_rate: u64,
}

impl Default for Retry {
    fn default() -> Self {
        Retry {
            attempts: 5,
            base: Duration::from_secs(2),
            max: Duration::from_secs(60),
            stall: Duration::from_secs(60),
            min_rate: 32 << 10,
        }
    }
}

impl Retry {
    /// The wait before attempt `n` (1-based retries): `base * 2^(n-1)`, capped, plus up to 25%
    /// jitter so parallel workers don't retry in step.
    fn delay(&self, n: u32, retry_after: Option<Duration>) -> Duration {
        let exp = self
            .base
            .saturating_mul(1u32 << (n.saturating_sub(1)).min(16))
            .min(self.max);
        let d = retry_after.map_or(exp, |r| r.min(self.max).max(exp));
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |t| t.subsec_nanos());
        d + d.mul_f64(f64::from(nanos % 1000) / 4000.0)
    }
}

/// Fetches `[start, end)` of one file. Shared between fetch threads.
pub trait Fetch: Send + Sync {
    fn get(&self, start: u64, end: u64) -> Result<Vec<u8>>;
    fn describe(&self) -> String;
}

pub struct LocalFile(pub PathBuf);

impl Fetch for LocalFile {
    fn get(&self, start: u64, end: u64) -> Result<Vec<u8>> {
        let mut f = File::open(&self.0).with_context(|| format!("opening {}", self.0.display()))?;
        f.seek(SeekFrom::Start(start))?;
        let mut v = vec![0u8; (end - start) as usize];
        f.read_exact(&mut v)
            .with_context(|| format!("reading {}..{} of {}", start, end, self.0.display()))?;
        Ok(v)
    }

    fn describe(&self) -> String {
        self.0.display().to_string()
    }
}

/// An HTTP client for Range requests; one connection pool shared by all fetch threads.
#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    retry: Retry,
}

impl Client {
    pub fn new(retry: Retry) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .user_agent(concat!("irit-gtdbprep/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Client { agent, retry }
    }

    /// Bytes `[start, end)` of `url`, retried as [`Retry`] says.
    pub fn get(&self, url: &str, start: u64, end: u64) -> Result<Vec<u8>> {
        ensure!(end > start, "empty range");
        let mut last = None;
        let mut wait = None;
        for attempt in 0..self.retry.attempts.max(1) {
            if attempt > 0 {
                let d = self.retry.delay(attempt, wait.take());
                if let Some(e) = &last {
                    eprintln!(
                        "gtdbprep: bytes {start}..{end}: {e:#}; retrying in {:.1} s",
                        d.as_secs_f64()
                    );
                }
                std::thread::sleep(d);
            }
            let body_time = self.retry.stall
                + Duration::from_secs((end - start) / self.retry.min_rate.max(1));
            let res = self
                .agent
                .get(url)
                .header("Range", format!("bytes={}-{}", start, end - 1))
                .config()
                .timeout_recv_response(Some(self.retry.stall))
                .timeout_recv_body(Some(body_time))
                .build()
                .call();
            let mut resp = match res {
                Ok(r) => r,
                Err(e) => {
                    last = Some(anyhow::Error::from(e));
                    continue;
                }
            };
            let status = resp.status().as_u16();
            if matches!(status, 408 | 429 | 500 | 502 | 503 | 504) {
                wait = resp
                    .headers()
                    .get("retry-after")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .map(Duration::from_secs);
                last = Some(anyhow::anyhow!("{url} answered {status}"));
                continue;
            }
            if status != 206 {
                bail!("{url} answered {status} to a Range request (it must support byte ranges)");
            }
            let want = format!("bytes {}-{}/", start, end - 1);
            let got = resp
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            ensure!(
                got.starts_with(&want),
                "{url}: asked for {want}..., got Content-Range '{got}'"
            );
            match resp
                .body_mut()
                .with_config()
                .limit(end - start + 1)
                .read_to_vec()
            {
                Ok(v) if v.len() as u64 == end - start => return Ok(v),
                Ok(v) => {
                    last = Some(anyhow::anyhow!(
                        "{url}: got {} bytes for a {}-byte range",
                        v.len(),
                        end - start
                    ))
                }
                Err(e) => last = Some(e.into()),
            }
        }
        Err(last.unwrap()).with_context(|| format!("fetching bytes {start}..{end} of {url}"))
    }
}

pub struct Http {
    pub url: String,
    client: Client,
}

impl Fetch for Http {
    fn get(&self, start: u64, end: u64) -> Result<Vec<u8>> {
        self.client.get(&self.url, start, end)
    }

    fn describe(&self) -> String {
        self.url.clone()
    }
}

fn is_url(location: &str) -> bool {
    location.starts_with("http://") || location.starts_with("https://")
}

fn fetcher(location: &str, client: &Client) -> Box<dyn Fetch> {
    if is_url(location) {
        Box::new(Http {
            url: location.to_string(),
            client: client.clone(),
        })
    } else {
        Box::new(LocalFile(PathBuf::from(location)))
    }
}

pub enum Kind {
    /// `sketches.db` itself, local or on a server.
    Plain(Box<dyn Fetch>),
    /// `sketches.db` cut into parts of `part_size` bytes; `{part}` in the template is the part
    /// number, zero-padded to three digits.
    Parts { template: String, part_size: u64 },
    /// GTDB's archive with a seek index; `member_offset` is where `sketches.db` starts in the
    /// uncompressed tar stream.
    Gzip {
        fetch: Box<dyn Fetch>,
        index: GzIndex,
        member_offset: u64,
        compressed_size: u64,
    },
}

/// One request's worth of work: the byte spans to fetch (concatenated), and which of the asked
/// ranges they hold.
struct Job {
    /// `(part number or None for the single file, start, end)`.
    spans: Vec<(Option<u64>, u64, u64)>,
    /// Indices into the asked ranges. Plain and parts jobs hold exactly one.
    ranges: Vec<usize>,
    /// For the gzip source: the checkpoint to resume at.
    checkpoint: Option<usize>,
}

pub struct Source {
    pub kind: Kind,
    client: Client,
    /// Requests in flight at once (1 = one at a time).
    pub jobs: usize,
    /// Bytes transferred (compressed bytes, for the gzip source).
    pub fetched: AtomicU64,
    pub requests: AtomicU64,
}

impl Source {
    pub fn plain(location: &str) -> Self {
        let client = Client::new(Retry::default());
        Self::new(Kind::Plain(fetcher(location, &client)), client)
    }

    pub fn parts(template: &str, part_size: u64) -> Result<Self> {
        ensure!(
            template.contains("{part}"),
            "a parts source needs {{part}} in its URL"
        );
        ensure!(part_size > 0, "part size must be positive");
        Ok(Self::new(
            Kind::Parts {
                template: template.to_string(),
                part_size,
            },
            Client::new(Retry::default()),
        ))
    }

    pub fn gzip(location: &str, index: GzIndex, member_offset: u64, compressed_size: u64) -> Self {
        let client = Client::new(Retry::default());
        Self::new(
            Kind::Gzip {
                fetch: fetcher(location, &client),
                index,
                member_offset,
                compressed_size,
            },
            client,
        )
    }

    fn new(kind: Kind, client: Client) -> Self {
        Source {
            kind,
            client,
            jobs: DEFAULT_JOBS,
            fetched: AtomicU64::new(0),
            requests: AtomicU64::new(0),
        }
    }

    /// Set how many requests may be in flight at once (clamped to 1..=[`MAX_JOBS`]).
    pub fn with_jobs(mut self, jobs: usize) -> Self {
        self.jobs = jobs.clamp(1, MAX_JOBS);
        self
    }

    /// Set the retry policy for HTTP sources.
    pub fn with_retry(mut self, retry: Retry) -> Self {
        self.client.retry = retry;
        let client = self.client.clone();
        let rewrap = |f: &mut Box<dyn Fetch>| {
            let d = f.describe();
            if is_url(&d) {
                *f = fetcher(&d, &client);
            }
        };
        match &mut self.kind {
            Kind::Plain(f) => rewrap(f),
            Kind::Gzip { fetch, .. } => rewrap(fetch),
            Kind::Parts { .. } => {}
        }
        self
    }

    pub fn describe(&self) -> String {
        match &self.kind {
            Kind::Plain(f) => f.describe(),
            Kind::Parts { template, .. } => template.clone(),
            Kind::Gzip { fetch, .. } => format!("{} (gzip seek index)", fetch.describe()),
        }
    }

    pub fn fetched_bytes(&self) -> u64 {
        self.fetched.load(Ordering::Relaxed)
    }

    pub fn request_count(&self) -> u64 {
        self.requests.load(Ordering::Relaxed)
    }

    /// Split the asked ranges into requests.
    fn plan(&self, ranges: &[(u64, u64)]) -> Result<Vec<Job>> {
        match &self.kind {
            Kind::Plain(_) => Ok(ranges
                .iter()
                .enumerate()
                .map(|(i, &(o, l))| Job {
                    spans: vec![(None, o, o + l)],
                    ranges: vec![i],
                    checkpoint: None,
                })
                .collect()),
            Kind::Parts { part_size, .. } => Ok(ranges
                .iter()
                .enumerate()
                .map(|(i, &(o, l))| {
                    let mut spans = Vec::new();
                    let mut pos = o;
                    while pos < o + l {
                        let part = pos / part_size;
                        let start = pos % part_size;
                        let end = (o + l - part * part_size).min(*part_size);
                        spans.push((Some(part), start, end));
                        pos = part * part_size + end;
                    }
                    Job {
                        spans,
                        ranges: vec![i],
                        checkpoint: None,
                    }
                })
                .collect()),
            Kind::Gzip {
                index,
                member_offset,
                compressed_size,
                ..
            } => {
                let mut order: Vec<usize> = (0..ranges.len()).collect();
                order.sort_by_key(|&i| ranges[i].0);
                let mut jobs = Vec::new();
                let mut k = 0;
                while k < order.len() {
                    // One fetch per run of ranges that share a checkpoint or lie closer
                    // together than the checkpoint spacing.
                    let first = ranges[order[k]].0 + member_offset;
                    let cp = index
                        .before(first)
                        .context("no checkpoint before a requested sketch")?;
                    let mut group = vec![order[k]];
                    let mut end = first + ranges[order[k]].1;
                    while let Some(&next) = order.get(k + group.len()) {
                        let s = ranges[next].0 + member_offset;
                        if index.before(s) == Some(cp) || s < end + index.spacing {
                            group.push(next);
                            end = end.max(s + ranges[next].1);
                        } else {
                            break;
                        }
                    }
                    let (from, to) = index.compressed_span(cp, end, *compressed_size);
                    k += group.len();
                    jobs.push(Job {
                        spans: vec![(None, from, to)],
                        ranges: group,
                        checkpoint: Some(cp),
                    });
                }
                Ok(jobs)
            }
        }
    }

    /// Fetch one job's bytes (runs on a fetch thread).
    fn fetch(&self, job: &Job) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        for &(part, start, end) in &job.spans {
            let v = match (&self.kind, part) {
                (Kind::Plain(f), None) | (Kind::Gzip { fetch: f, .. }, None) => f.get(start, end)?,
                (Kind::Parts { template, .. }, Some(p)) => {
                    let url = template.replace("{part}", &format!("{p:03}"));
                    if is_url(&url) {
                        self.client.get(&url, start, end)?
                    } else {
                        LocalFile(PathBuf::from(url)).get(start, end)?
                    }
                }
                _ => unreachable!("a job planned for another kind of source"),
            };
            self.fetched.fetch_add(end - start, Ordering::Relaxed);
            self.requests.fetch_add(1, Ordering::Relaxed);
            if job.spans.len() == 1 {
                return Ok(v);
            }
            out.extend(v);
        }
        Ok(out)
    }

    /// Turn one job's bytes into the asked ranges it holds (runs on the calling thread).
    fn finish(
        &self,
        job: &Job,
        data: Vec<u8>,
        ranges: &[(u64, u64)],
    ) -> Result<Vec<(usize, Vec<u8>)>> {
        match &self.kind {
            Kind::Gzip {
                index,
                member_offset,
                ..
            } => {
                let want: Vec<(u64, u64)> = job
                    .ranges
                    .iter()
                    .map(|&g| (ranges[g].0 + member_offset, ranges[g].1))
                    .collect();
                let got = index.extract(job.checkpoint.unwrap(), &data, &want)?;
                Ok(job.ranges.iter().copied().zip(got).collect())
            }
            _ => Ok(vec![(job.ranges[0], data)]),
        }
    }

    /// Read `(offset, length)` ranges of `sketches.db` with up to [`Source::jobs`] requests in
    /// flight, and hand each range to `sink` (with its index in `ranges`) as soon as it arrives,
    /// in no particular order. At most about twice `jobs` requests' bytes are held in memory at
    /// once. Stops at the first error (after the requests in flight return).
    pub fn read_each(
        &self,
        ranges: &[(u64, u64)],
        mut sink: impl FnMut(usize, Vec<u8>) -> Result<()>,
    ) -> Result<()> {
        let plan = self.plan(ranges)?;
        if plan.is_empty() {
            return Ok(());
        }
        let workers = self.jobs.clamp(1, MAX_JOBS).min(plan.len());
        let next = AtomicUsize::new(0);
        let stop = AtomicBool::new(false);
        std::thread::scope(|s| {
            let (tx, rx) = mpsc::sync_channel::<(usize, Result<Vec<u8>>)>(workers);
            for _ in 0..workers {
                let tx = tx.clone();
                let (plan, next, stop) = (&plan, &next, &stop);
                s.spawn(move || {
                    while !stop.load(Ordering::Relaxed) {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= plan.len() {
                            break;
                        }
                        let r = self.fetch(&plan[i]);
                        if r.is_err() {
                            // Start no more requests; the ones in flight finish.
                            stop.store(true, Ordering::Relaxed);
                        }
                        if tx.send((i, r)).is_err() {
                            break;
                        }
                    }
                });
            }
            drop(tx);
            let mut result = Ok(());
            for (i, r) in rx.iter() {
                let step = r.and_then(|data| {
                    for (k, v) in self.finish(&plan[i], data, ranges)? {
                        sink(k, v)?;
                    }
                    Ok(())
                });
                if let Err(e) = step {
                    stop.store(true, Ordering::Relaxed);
                    result = Err(e);
                    break;
                }
            }
            // Dropping `rx` here unblocks any worker waiting to send.
            result
        })
    }

    /// Read `(offset, length)` ranges of `sketches.db`. Results come back in the order asked.
    pub fn read(&self, ranges: &[(u64, u64)]) -> Result<Vec<Vec<u8>>> {
        let mut out = vec![Vec::new(); ranges.len()];
        self.read_each(ranges, |i, v| {
            out[i] = v;
            Ok(())
        })?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::super::gz::{self, tests::sample, tests::tar_gz};
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;

    /// A tiny HTTP/1.1 server that answers single Range requests for a few paths.
    fn serve(files: Vec<(String, Vec<u8>)>) -> String {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = l.local_addr().unwrap();
        let files = Arc::new(files);
        std::thread::spawn(move || {
            for s in l.incoming() {
                let mut s = s.unwrap();
                let files = files.clone();
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let mut b = [0u8; 1024];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = s.read(&mut b).unwrap();
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&b[..n]);
                    }
                    let req = String::from_utf8_lossy(&buf).to_string();
                    let path = req.split_whitespace().nth(1).unwrap().to_string();
                    let range = req
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("range: bytes=")
                                .map(str::to_string)
                        })
                        .unwrap();
                    let (a, z) = range.trim().split_once('-').unwrap();
                    let (a, z): (usize, usize) = (a.parse().unwrap(), z.parse().unwrap());
                    let Some((_, data)) = files.iter().find(|(p, _)| *p == path) else {
                        let _ = s.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\n\r\n");
                        return;
                    };
                    let z = z.min(data.len() - 1);
                    let body = &data[a..=z];
                    let head = format!(
                        "HTTP/1.1 206 Partial Content\r\ncontent-range: bytes {a}-{z}/{}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        data.len(),
                        body.len()
                    );
                    let _ = s.write_all(head.as_bytes());
                    let _ = s.write_all(body);
                });
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn plain_parts_and_gzip_sources_agree() {
        let db = sample(2_500_000, 11);
        let ranges = [
            (0u64, 100u64),
            (999_990, 20),
            (1_000_000, 70_000),
            (2_499_000, 1000),
        ];
        let want: Vec<Vec<u8>> = ranges
            .iter()
            .map(|&(o, l)| db[o as usize..(o + l) as usize].to_vec())
            .collect();

        let d = tempfile::tempdir().unwrap();
        let local = d.path().join("sketches.db");
        std::fs::write(&local, &db).unwrap();
        assert_eq!(
            Source::plain(local.to_str().unwrap())
                .read(&ranges)
                .unwrap(),
            want
        );

        let gzb = tar_gz(&db);
        let mut files = vec![
            ("/sketches.db".to_string(), db.clone()),
            ("/r.tar.gz".to_string(), gzb.clone()),
        ];
        for (i, chunk) in db.chunks(1_000_000).enumerate() {
            files.push((format!("/sketches.db.{i:03}"), chunk.to_vec()));
        }
        let base = serve(files);
        assert_eq!(
            Source::plain(&format!("{base}/sketches.db"))
                .read(&ranges)
                .unwrap(),
            want
        );
        let parts = Source::parts(&format!("{base}/sketches.db.{{part}}"), 1_000_000).unwrap();
        assert_eq!(parts.read(&ranges).unwrap(), want);
        assert!(
            Source::plain(&format!("{base}/missing"))
                .read(&ranges[..1])
                .is_err()
        );

        let r = gz::scan(&gzb[..], 1 << 16, "db/sketches.db").unwrap();
        let t = r.target.unwrap();
        let idx = d.path().join("gz.idx");
        gz::write_index(&idx, &r.checkpoints, 1 << 16, 2).unwrap();
        let src = Source::gzip(
            &format!("{base}/r.tar.gz"),
            GzIndex::open(&idx).unwrap(),
            t.offset,
            r.compressed_size,
        );
        assert_eq!(src.read(&ranges).unwrap(), want);
        assert!(src.fetched_bytes() > 0 && src.fetched_bytes() < gzb.len() as u64);
    }

    /// What the scripted server does with one request.
    #[derive(Clone, Copy)]
    enum Reply {
        /// Serve the range after a pause.
        Ok(u64),
        /// Answer this status, with `Retry-After: 0` for 503.
        Status(u16),
        /// Promise the full range but send only half and close.
        Short,
    }

    struct Script {
        base: String,
        /// Requests received, per range start.
        calls: Arc<std::sync::Mutex<std::collections::HashMap<u64, usize>>>,
        in_flight: Arc<AtomicUsize>,
        max_in_flight: Arc<AtomicUsize>,
    }

    /// An HTTP server for one file whose answer to each request is `decide(range start, how many
    /// requests that range has had before)`; it counts requests in flight.
    fn scripted(data: Vec<u8>, decide: fn(u64, usize) -> Reply) -> Script {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let calls = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_in_flight = Arc::new(AtomicUsize::new(0));
        let data = Arc::new(data);
        let (c, f, m) = (calls.clone(), in_flight.clone(), max_in_flight.clone());
        std::thread::spawn(move || {
            for s in l.incoming() {
                let mut s = s.unwrap();
                let (data, calls, in_flight, max_in_flight) =
                    (data.clone(), c.clone(), f.clone(), m.clone());
                std::thread::spawn(move || {
                    let mut buf = Vec::new();
                    let mut b = [0u8; 1024];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        let n = s.read(&mut b).unwrap_or(0);
                        if n == 0 {
                            return;
                        }
                        buf.extend_from_slice(&b[..n]);
                    }
                    let now = in_flight.fetch_add(1, Ordering::SeqCst) + 1;
                    max_in_flight.fetch_max(now, Ordering::SeqCst);
                    let req = String::from_utf8_lossy(&buf).to_ascii_lowercase();
                    let range = req
                        .lines()
                        .find_map(|l| l.strip_prefix("range: bytes=").map(str::to_string))
                        .unwrap();
                    let (a, z) = range.trim().split_once('-').unwrap();
                    let (a, z): (usize, usize) = (a.parse().unwrap(), z.parse().unwrap());
                    let seen = {
                        let mut c = calls.lock().unwrap();
                        let e = c.entry(a as u64).or_insert(0);
                        *e += 1;
                        *e - 1
                    };
                    let body = &data[a..=z];
                    let head = |n: usize| {
                        format!(
                            "HTTP/1.1 206 Partial Content\r\ncontent-range: bytes {a}-{z}/{}\r\ncontent-length: {n}\r\nconnection: close\r\n\r\n",
                            data.len()
                        )
                    };
                    let reply = decide(a as u64, seen);
                    if let Reply::Ok(ms) = reply {
                        std::thread::sleep(Duration::from_millis(ms));
                    }
                    // Counted out before answering: the client may send its next request as soon
                    // as the body arrives, before this thread runs again.
                    in_flight.fetch_sub(1, Ordering::SeqCst);
                    match reply {
                        Reply::Ok(_) => {
                            let _ = s.write_all(head(body.len()).as_bytes());
                            let _ = s.write_all(body);
                        }
                        Reply::Status(code) => {
                            let _ = s.write_all(
                                format!("HTTP/1.1 {code} X\r\nretry-after: 0\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
                                    .as_bytes(),
                            );
                        }
                        Reply::Short => {
                            let _ = s.write_all(head(body.len()).as_bytes());
                            let _ = s.write_all(&body[..body.len() / 2]);
                        }
                    }
                });
            }
        });
        Script {
            base,
            calls,
            in_flight,
            max_in_flight,
        }
    }

    fn fast_retry(attempts: u32) -> Retry {
        Retry {
            attempts,
            base: Duration::from_millis(5),
            max: Duration::from_millis(20),
            stall: Duration::from_millis(300),
            min_rate: 1 << 20,
        }
    }

    fn ranges_of(n: u64, len: u64) -> Vec<(u64, u64)> {
        (0..n).map(|i| (i * 1000 + 7, len)).collect()
    }

    #[test]
    fn parallel_requests_stay_within_the_limit_and_keep_the_order() {
        let db = sample(40_000, 3);
        let ranges = ranges_of(24, 900);
        let want: Vec<Vec<u8>> = ranges
            .iter()
            .map(|&(o, l)| db[o as usize..(o + l) as usize].to_vec())
            .collect();
        for jobs in [1usize, 3, 6] {
            let srv = scripted(db.clone(), |_, _| Reply::Ok(40));
            let src = Source::plain(&format!("{}/sketches.db", srv.base)).with_jobs(jobs);
            let t = std::time::Instant::now();
            assert_eq!(src.read(&ranges).unwrap(), want, "jobs = {jobs}");
            let max = srv.max_in_flight.load(Ordering::SeqCst);
            assert!(max <= jobs, "jobs = {jobs}: {max} requests in flight");
            if jobs > 1 {
                assert!(max > 1, "jobs = {jobs}: requests never overlapped");
            } else {
                // 24 requests one at a time, 40 ms each.
                assert!(t.elapsed() >= Duration::from_millis(24 * 40));
            }
            assert_eq!(src.request_count(), 24);
            assert_eq!(src.fetched_bytes(), 24 * 900);
            assert_eq!(srv.in_flight.load(Ordering::SeqCst), 0);
        }
        // Clamped to 1..=MAX_JOBS.
        assert_eq!(Source::plain("x").with_jobs(0).jobs, 1);
        assert_eq!(Source::plain("x").with_jobs(1000).jobs, MAX_JOBS);
    }

    #[test]
    fn transient_errors_are_retried() {
        let db = sample(30_000, 5);
        let ranges = ranges_of(12, 500);
        let want: Vec<Vec<u8>> = ranges
            .iter()
            .map(|&(o, l)| db[o as usize..(o + l) as usize].to_vec())
            .collect();
        // Every range: 503, then 429, then a cut-off body, then the data.
        let srv = scripted(db.clone(), |_, seen| match seen {
            0 => Reply::Status(503),
            1 => Reply::Status(429),
            2 => Reply::Short,
            _ => Reply::Ok(0),
        });
        let src = Source::plain(&format!("{}/sketches.db", srv.base))
            .with_jobs(4)
            .with_retry(fast_retry(4));
        assert_eq!(src.read(&ranges).unwrap(), want);
        let calls = srv.calls.lock().unwrap();
        assert_eq!(calls.len(), 12);
        assert!(calls.values().all(|&n| n == 4), "{calls:?}");
        // Only successful bodies count as fetched.
        assert_eq!(src.request_count(), 12);
    }

    #[test]
    fn a_stalled_request_is_cut_off_and_retried() {
        let db = sample(30_000, 5);
        let ranges = ranges_of(4, 500);
        // The first answer to each range comes after 5 s; the retry is immediate.
        let srv = scripted(db.clone(), |_, seen| Reply::Ok(if seen == 0 { 5000 } else { 0 }));
        let src = Source::plain(&format!("{}/sketches.db", srv.base))
            .with_jobs(4)
            .with_retry(fast_retry(3));
        let t = std::time::Instant::now();
        let got = src.read(&ranges).unwrap();
        assert!(t.elapsed() < Duration::from_secs(4), "{:?}", t.elapsed());
        assert_eq!(got[2], db[2007..2507].to_vec());
        assert!(srv.calls.lock().unwrap().values().all(|&n| n == 2));
    }

    #[test]
    fn retries_give_up_and_permanent_errors_fail_at_once() {
        let db = sample(30_000, 5);
        let ranges = ranges_of(6, 500);
        // Always 503: each range is tried exactly `attempts` times, then the read fails.
        let srv = scripted(db.clone(), |_, _| Reply::Status(503));
        let src = Source::plain(&format!("{}/sketches.db", srv.base))
            .with_jobs(1)
            .with_retry(fast_retry(3));
        let err = src.read(&ranges).unwrap_err();
        assert!(format!("{err:#}").contains("503"), "{err:#}");
        // Stopped at the first range that failed: one range, three attempts.
        assert_eq!(srv.calls.lock().unwrap().values().sum::<usize>(), 3);

        // 404 is not retried.
        let srv = scripted(db.clone(), |_, _| Reply::Status(404));
        let src = Source::plain(&format!("{}/sketches.db", srv.base))
            .with_jobs(1)
            .with_retry(fast_retry(5));
        assert!(src.read(&ranges[..1]).is_err());
        assert_eq!(srv.calls.lock().unwrap().values().sum::<usize>(), 1);
    }

    #[test]
    fn an_error_stops_the_other_workers() {
        let db = sample(200_000, 9);
        let ranges = ranges_of(150, 500);
        // Range 3 always fails; the rest are slow, so most are still unrequested by then.
        let srv = scripted(db, |start, _| {
            if start == 3007 {
                Reply::Status(404)
            } else {
                Reply::Ok(20)
            }
        });
        let src = Source::plain(&format!("{}/sketches.db", srv.base)).with_jobs(4);
        let mut got = 0;
        let err = src
            .read_each(&ranges, |_, _| {
                got += 1;
                Ok(())
            })
            .unwrap_err();
        assert!(format!("{err:#}").contains("404"));
        let asked: usize = srv.calls.lock().unwrap().values().sum();
        assert!(asked < 40, "{asked} of 150 ranges were requested after the error");
        assert!(got < 40);

        // An error from the sink (e.g. a checksum mismatch) stops the read too.
        let db = sample(200_000, 9);
        let srv = scripted(db, |_, _| Reply::Ok(20));
        let src = Source::plain(&format!("{}/sketches.db", srv.base)).with_jobs(4);
        let mut n = 0;
        let err = src
            .read_each(&ranges, |_, _| {
                n += 1;
                if n == 2 {
                    anyhow::bail!("bad sketch")
                }
                Ok(())
            })
            .unwrap_err();
        assert_eq!(err.to_string(), "bad sketch");
        assert!(srv.calls.lock().unwrap().values().sum::<usize>() < 40);
    }

    #[test]
    fn gzip_source_in_parallel_matches_one_at_a_time() {
        let db = sample(2_500_000, 13);
        let gzb = tar_gz(&db);
        let r = gz::scan(&gzb[..], 1 << 16, "db/sketches.db").unwrap();
        let t = r.target.unwrap();
        let d = tempfile::tempdir().unwrap();
        let idx = d.path().join("gz.idx");
        gz::write_index(&idx, &r.checkpoints, 1 << 16, 1).unwrap();
        let ranges: Vec<(u64, u64)> = (0..40u64).map(|i| (i * 61_001 % 2_400_000, 3000)).collect();
        let srv = scripted(gzb.clone(), |_, _| Reply::Ok(5));
        let one = Source::gzip(
            &format!("{}/r.tar.gz", srv.base),
            GzIndex::open(&idx).unwrap(),
            t.offset,
            r.compressed_size,
        )
        .with_jobs(1);
        let many = Source::gzip(
            &format!("{}/r.tar.gz", srv.base),
            GzIndex::open(&idx).unwrap(),
            t.offset,
            r.compressed_size,
        )
        .with_jobs(8);
        let a = one.read(&ranges).unwrap();
        let b = many.read(&ranges).unwrap();
        assert_eq!(a, b);
        for (&(o, l), v) in ranges.iter().zip(&a) {
            assert_eq!(&db[o as usize..(o + l) as usize], &v[..]);
        }
        assert_eq!(one.fetched_bytes(), many.fetched_bytes());
        assert_eq!(one.request_count(), many.request_count());
    }
}
