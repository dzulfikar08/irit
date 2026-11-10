//! Where full reference sketches are read from: byte ranges of `sketches.db`.
//!
//! - a local `sketches.db` (or any copy of it on disk);
//! - an HTTP(S) URL of an unpacked `sketches.db`, read with Range requests;
//! - the same split into numbered parts (`...sketches.db.{part}`, fixed part size), the shape a
//!   GitHub Release needs (assets are limited to 2 GiB);
//! - GTDB's own `.tar.gz`, through a gzip seek index (see `gz.rs`).

use super::gz::GzIndex;
use anyhow::{Context, Result, bail, ensure};
use std::cell::Cell;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;
use std::time::Duration;

/// Fetches `[start, end)` of one file.
pub trait Fetch {
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

pub struct Http {
    pub url: String,
    agent: ureq::Agent,
}

impl Http {
    pub fn new(url: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_global(Some(Duration::from_secs(600)))
            .user_agent(concat!("irit-gtdbprep/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Http {
            url: url.to_string(),
            agent,
        }
    }
}

impl Fetch for Http {
    fn get(&self, start: u64, end: u64) -> Result<Vec<u8>> {
        ensure!(end > start, "empty range");
        let mut last = None;
        for attempt in 0..4 {
            if attempt > 0 {
                std::thread::sleep(Duration::from_secs(2 << attempt));
            }
            let res = self
                .agent
                .get(&self.url)
                .header("Range", format!("bytes={}-{}", start, end - 1))
                .call();
            let mut resp = match res {
                Ok(r) => r,
                Err(e) => {
                    last = Some(anyhow::Error::from(e));
                    continue;
                }
            };
            let status = resp.status().as_u16();
            if status != 206 {
                bail!(
                    "{} answered {status} to a Range request (it must support byte ranges)",
                    self.url
                );
            }
            let want = format!("bytes {}-{}/", start, end - 1);
            let got = resp
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("");
            ensure!(
                got.starts_with(&want),
                "{}: asked for {want}..., got Content-Range '{got}'",
                self.url
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
                        "{}: got {} bytes for a {}-byte range",
                        self.url,
                        v.len(),
                        end - start
                    ))
                }
                Err(e) => last = Some(e.into()),
            }
        }
        Err(last.unwrap()).with_context(|| format!("fetching bytes {start}..{end} of {}", self.url))
    }

    fn describe(&self) -> String {
        self.url.clone()
    }
}

fn fetcher(location: &str) -> Box<dyn Fetch> {
    if location.starts_with("http://") || location.starts_with("https://") {
        Box::new(Http::new(location))
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

pub struct Source {
    pub kind: Kind,
    /// Bytes transferred (compressed bytes, for the gzip source).
    pub fetched: Cell<u64>,
    pub requests: Cell<u64>,
}

impl Source {
    pub fn plain(location: &str) -> Self {
        Self::new(Kind::Plain(fetcher(location)))
    }

    pub fn parts(template: &str, part_size: u64) -> Result<Self> {
        ensure!(
            template.contains("{part}"),
            "a parts source needs {{part}} in its URL"
        );
        ensure!(part_size > 0, "part size must be positive");
        Ok(Self::new(Kind::Parts {
            template: template.to_string(),
            part_size,
        }))
    }

    pub fn gzip(location: &str, index: GzIndex, member_offset: u64, compressed_size: u64) -> Self {
        Self::new(Kind::Gzip {
            fetch: fetcher(location),
            index,
            member_offset,
            compressed_size,
        })
    }

    fn new(kind: Kind) -> Self {
        Source {
            kind,
            fetched: Cell::new(0),
            requests: Cell::new(0),
        }
    }

    pub fn describe(&self) -> String {
        match &self.kind {
            Kind::Plain(f) => f.describe(),
            Kind::Parts { template, .. } => template.clone(),
            Kind::Gzip { fetch, .. } => format!("{} (gzip seek index)", fetch.describe()),
        }
    }

    fn count(&self, bytes: u64) {
        self.fetched.set(self.fetched.get() + bytes);
        self.requests.set(self.requests.get() + 1);
    }

    /// Read `(offset, length)` ranges of `sketches.db`. Results come back in the order asked.
    pub fn read(&self, ranges: &[(u64, u64)]) -> Result<Vec<Vec<u8>>> {
        match &self.kind {
            Kind::Plain(f) => ranges
                .iter()
                .map(|&(o, l)| {
                    let v = f.get(o, o + l)?;
                    self.count(l);
                    Ok(v)
                })
                .collect(),
            Kind::Parts {
                template,
                part_size,
            } => ranges
                .iter()
                .map(|&(o, l)| {
                    let mut v = Vec::with_capacity(l as usize);
                    let mut pos = o;
                    while pos < o + l {
                        let part = pos / part_size;
                        let start = pos % part_size;
                        let end = (o + l - part * part_size).min(*part_size);
                        let url = template.replace("{part}", &format!("{part:03}"));
                        v.extend(fetcher(&url).get(start, end)?);
                        self.count(end - start);
                        pos = part * part_size + end;
                    }
                    Ok(v)
                })
                .collect(),
            Kind::Gzip {
                fetch,
                index,
                member_offset,
                compressed_size,
            } => {
                let mut order: Vec<usize> = (0..ranges.len()).collect();
                order.sort_by_key(|&i| ranges[i].0);
                let mut out = vec![Vec::new(); ranges.len()];
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
                    let data = fetch.get(from, to)?;
                    self.count(to - from);
                    let want: Vec<(u64, u64)> = group
                        .iter()
                        .map(|&g| (ranges[g].0 + member_offset, ranges[g].1))
                        .collect();
                    let got = index.extract(cp, &data, &want)?;
                    for (g, v) in group.iter().zip(got) {
                        out[*g] = v;
                    }
                    k += group.len();
                }
                Ok(out)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::gz::{self, tests::sample, tests::tar_gz};
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::Arc;

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
        assert!(src.fetched.get() > 0 && src.fetched.get() < gzb.len() as u64);
    }
}
