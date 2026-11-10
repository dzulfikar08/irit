//! A compact copy of skani's marker screen.
//!
//! `skani search` passes a reference to the full ANI step when the query and the reference share
//! at least `max(1, floor(0.8^21 × min(|Q|, |R|)))` marker k-mers (`check_markers_quickly` in
//! skani 0.3.1's `src/screen.rs`; with more than 50 queries it uses an inverted index and a
//! strict `>`, which passes a subset of the same references). `|Q|` and `|R|` are the sizes of the
//! two marker sets.
//!
//! The index keeps, for every reference, its exact marker count and a 32-bit fingerprint of each
//! marker, Rice-coded. A query marker that the reference has always has a matching fingerprint,
//! so the count of matching fingerprints is never below the true shared count, and every
//! reference skani would screen in is kept (a few extra may pass; skani screens them again).
//!
//! Files in an index folder: `meta.json`, `refs.bin` (one fixed-size row per reference, in
//! `index.db` order) and `markers.idx` (the Rice-coded fingerprints).

use super::skani::{self, MarkersFile, Params};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File};
use std::io::{BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "irit-ani-screen/1";
/// skani's default `search` screen (`SEARCH_ANI_CUTOFF_DEFAULT`), which GTDB-Tk uses.
pub const SCREEN_IDENTITY: f64 = 0.80;
/// skani's marker k-mer length for DNA (`K_MARKER_DNA`).
pub const MARKER_K: i32 = 21;
const MARKERS_MAGIC: &[u8; 8] = b"IRITMK01";
const ROW: usize = 41;

/// Does a reference pass skani's marker screen? `shared` may overcount (never undercount).
pub fn passes(shared: u64, query_markers: usize, ref_markers: usize) -> bool {
    let min_card = query_markers.min(ref_markers);
    if min_card == 0 {
        return false;
    }
    // The same arithmetic as skani: f64 product, truncated, at least 1.
    let ratio = (SCREEN_IDENTITY.powi(MARKER_K) * min_card as f64) as usize;
    shared as usize >= ratio.max(1)
}

/// The fingerprint of a marker k-mer: the top 32 bits of splitmix64's finaliser.
pub fn fingerprint(kmer: u64) -> u32 {
    let mut x = kmer;
    x ^= x >> 30;
    x = x.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x ^= x >> 27;
    x = x.wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^= x >> 31;
    (x >> 32) as u32
}

/// Index metadata (`meta.json`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Meta {
    pub format: String,
    /// GTDB release the database belongs to, as given when building (`r232`).
    pub gtdb_release: Option<String>,
    /// skani version that wrote the database (from its `metadata.tsv`), e.g. `0.3.1`.
    pub skani_version: Option<String>,
    /// The database's own `metadata.tsv` lines (MD5s of its three files and so on).
    pub skani_metadata: BTreeMap<String, String>,
    /// The serialised skani `SketchParams` every record starts with, in hex.
    pub params_hex: String,
    pub sketch_c: u64,
    pub marker_c: u64,
    pub k: u64,
    pub refs: u64,
    pub markers_total: u64,
    pub sketches_db_size: u64,
    pub fingerprint: String,
    pub screen_identity: f64,
    pub marker_k: i32,
    /// How `refs.bin` stores each record's hash.
    pub record_hash: String,
    pub built_by: String,
    /// Where the full sketches can be range-read from GTDB's own archive, when known.
    #[serde(default)]
    pub gzip_source: Option<GzipSource>,
}

/// GTDB's archive as a source of sketches, with the seek index `gz-scan` made.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GzipSource {
    pub url: String,
    pub md5: String,
    pub compressed_size: u64,
    /// Path of `sketches.db` inside the archive.
    pub member: String,
    /// Offset of `sketches.db`'s first byte in the uncompressed tar stream.
    pub member_offset: u64,
    pub member_size: u64,
    /// The seek index file, relative to the index folder.
    pub checkpoints: String,
    pub spacing: u64,
    pub gzip_members: u64,
}

impl Meta {
    pub fn params(&self) -> Result<Params> {
        let raw = hex_decode(&self.params_hex)?;
        skani::read_params(&mut skani::Bin::new(&raw[..]))
    }
}

/// One reference: where its record is in `sketches.db`, its marker count, where its
/// fingerprints are, and the first 16 bytes of the SHA-256 of its record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefRow {
    pub offset: u64,
    pub length: u32,
    pub markers: u32,
    pub idx_offset: u64,
    pub rice_k: u8,
    pub sha: [u8; 16],
}

impl RefRow {
    fn to_bytes(self) -> [u8; ROW] {
        let mut b = [0u8; ROW];
        b[0..8].copy_from_slice(&self.offset.to_le_bytes());
        b[8..12].copy_from_slice(&self.length.to_le_bytes());
        b[12..16].copy_from_slice(&self.markers.to_le_bytes());
        b[16..24].copy_from_slice(&self.idx_offset.to_le_bytes());
        b[24] = self.rice_k;
        b[25..41].copy_from_slice(&self.sha);
        b
    }

    fn from_bytes(b: &[u8]) -> Self {
        RefRow {
            offset: u64::from_le_bytes(b[0..8].try_into().unwrap()),
            length: u32::from_le_bytes(b[8..12].try_into().unwrap()),
            markers: u32::from_le_bytes(b[12..16].try_into().unwrap()),
            idx_offset: u64::from_le_bytes(b[16..24].try_into().unwrap()),
            rice_k: b[24],
            sha: b[25..41].try_into().unwrap(),
        }
    }
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

pub fn hex_decode(s: &str) -> Result<Vec<u8>> {
    ensure!(s.len().is_multiple_of(2), "odd-length hex");
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).context("bad hex"))
        .collect()
}

// ---- Rice coding of sorted fingerprints ----

struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitWriter {
    fn new() -> Self {
        BitWriter {
            out: Vec::new(),
            acc: 0,
            n: 0,
        }
    }

    /// Append the low `bits` bits of `v` (bits <= 32).
    fn put(&mut self, v: u64, bits: u32) {
        if bits == 0 {
            return;
        }
        self.acc |= (v & ((1u64 << bits) - 1)) << self.n;
        self.n += bits;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u64,
    n: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitReader {
            data,
            pos: 0,
            acc: 0,
            n: 0,
        }
    }

    fn refill(&mut self) {
        while self.n <= 56 {
            let byte = self.data.get(self.pos).copied().unwrap_or(0);
            self.pos += 1;
            self.acc |= (byte as u64) << self.n;
            self.n += 8;
        }
    }

    fn get(&mut self, bits: u32) -> u64 {
        if bits == 0 {
            return 0;
        }
        self.refill();
        let v = self.acc & ((1u64 << bits) - 1);
        self.acc >>= bits;
        self.n -= bits;
        v
    }

    /// Count 1-bits up to the next 0-bit (consumed).
    fn unary(&mut self) -> Result<u64> {
        let mut q = 0u64;
        loop {
            self.refill();
            let ones = self.acc.trailing_ones();
            if ones < self.n {
                self.acc >>= ones + 1;
                self.n -= ones + 1;
                return Ok(q + ones as u64);
            }
            q += self.n as u64;
            self.acc = 0;
            self.n = 0;
            ensure!(self.pos <= self.data.len() + 8, "corrupt fingerprint block");
        }
    }
}

fn rice_k(n: usize) -> u8 {
    if n == 0 {
        return 0;
    }
    let mean = (1u64 << 32) as f64 / n as f64;
    let k = (mean * std::f64::consts::LN_2).log2().floor();
    k.clamp(0.0, 31.0) as u8
}

/// Rice-code sorted values (gaps from 0).
fn encode(sorted: &[u32], k: u8) -> Vec<u8> {
    let mut w = BitWriter::new();
    let mut prev = 0u64;
    for &v in sorted {
        let gap = v as u64 - prev;
        prev = v as u64;
        let mut q = gap >> k;
        while q >= 32 {
            w.put(u32::MAX as u64, 32);
            q -= 32;
        }
        w.put((1u64 << q) - 1, q as u32);
        w.put(0, 1);
        w.put(gap, k as u32);
    }
    w.finish()
}

fn decode(data: &[u8], n: usize, k: u8, out: &mut Vec<u32>) -> Result<()> {
    out.clear();
    let mut r = BitReader::new(data);
    let mut prev = 0u64;
    for _ in 0..n {
        let q = r.unary()?;
        let gap = (q << k) | r.get(k as u32);
        prev += gap;
        ensure!(prev <= u32::MAX as u64, "corrupt fingerprint block");
        out.push(prev as u32);
    }
    Ok(())
}

// ---- Building ----

/// Statistics from a build.
#[derive(Debug, Clone, Default)]
pub struct BuildStats {
    pub refs: u64,
    pub markers_total: u64,
    pub markers_max: u64,
    pub markers_bin_bytes: u64,
    pub markers_idx_bytes: u64,
    pub refs_bin_bytes: u64,
    pub sketch_bytes_max: u64,
}

fn skani_metadata(db: &Path) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    if let Ok(text) = fs::read_to_string(db.join("metadata.tsv")) {
        for line in text.lines() {
            if let Some((k, v)) = line.split_once(':') {
                m.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
    }
    m
}

/// Build a screen index for the skani database in `db` (a folder with `markers.bin`,
/// `index.db` and `sketches.db`). Reads each file once, front to back; memory stays small.
pub fn build(db: &Path, out: &Path, release: Option<&str>) -> Result<BuildStats> {
    fs::create_dir_all(out)?;
    let index = skani::read_index(&db.join("index.db"))?;
    let mut markers = MarkersFile::open(&db.join("markers.bin"))?;
    ensure!(
        markers.count == index.len() as u64,
        "markers.bin has {} sketches, index.db {}",
        markers.count,
        index.len()
    );
    ensure!(
        !markers.params.use_aa,
        "amino-acid skani databases are not supported"
    );
    let params = markers.params.clone();
    let mut stats = BuildStats {
        refs: index.len() as u64,
        markers_bin_bytes: fs::metadata(db.join("markers.bin"))?.len(),
        ..Default::default()
    };

    let mut idx = BufWriter::new(File::create(out.join("markers.idx"))?);
    idx.write_all(MARKERS_MAGIC)?;
    let mut idx_pos = MARKERS_MAGIC.len() as u64;
    let mut rows = Vec::with_capacity(index.len());
    let mut fps = Vec::new();
    for (i, entry) in index.iter().enumerate() {
        let m = markers
            .next_sketch()?
            .with_context(|| format!("markers.bin ended at sketch {i}"))?;
        ensure!(
            m.file_name == entry.file_name,
            "sketch {i} is {} in markers.bin but {} in index.db",
            m.file_name,
            entry.file_name
        );
        fps.clear();
        fps.extend(m.markers.iter().map(|&x| fingerprint(x)));
        fps.sort_unstable();
        let k = rice_k(fps.len());
        let block = encode(&fps, k);
        idx.write_all(&block)?;
        ensure!(entry.length <= u32::MAX as u64, "sketch {i} is over 4 GiB");
        rows.push(RefRow {
            offset: entry.offset,
            length: entry.length as u32,
            markers: m.markers.len() as u32,
            idx_offset: idx_pos,
            rice_k: k,
            sha: [0; 16],
        });
        idx_pos += block.len() as u64;
        stats.markers_total += m.markers.len() as u64;
        stats.markers_max = stats.markers_max.max(m.markers.len() as u64);
        stats.sketch_bytes_max = stats.sketch_bytes_max.max(entry.length);
    }
    ensure!(
        markers.next_sketch()?.is_none(),
        "markers.bin has more sketches than it says"
    );
    idx.flush()?;
    stats.markers_idx_bytes = idx_pos;

    // Hash every record, reading sketches.db front to back.
    let sk_path = db.join("sketches.db");
    let sk_size = fs::metadata(&sk_path)?.len();
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|&i| rows[i].offset);
    let mut f = BufReader::with_capacity(8 << 20, File::open(&sk_path)?);
    let mut pos = 0u64;
    let mut buf = Vec::new();
    for i in order {
        let r = rows[i];
        ensure!(
            r.offset + r.length as u64 <= sk_size,
            "sketch {i} lies beyond the end of sketches.db"
        );
        if r.offset != pos {
            f.seek(SeekFrom::Start(r.offset))?;
        }
        buf.resize(r.length as usize, 0);
        f.read_exact(&mut buf)?;
        pos = r.offset + r.length as u64;
        let name = skani::record_name(&buf, &params)
            .with_context(|| format!("sketch {i} in sketches.db"))?;
        ensure!(
            name == index[i].file_name,
            "sketch {i} in sketches.db is {name}, index.db says {}",
            index[i].file_name
        );
        rows[i].sha = Sha256::digest(&buf)[..16].try_into().unwrap();
    }

    let mut rb = BufWriter::new(File::create(out.join("refs.bin"))?);
    for r in &rows {
        rb.write_all(&r.to_bytes())?;
    }
    rb.flush()?;
    stats.refs_bin_bytes = (rows.len() * ROW) as u64;

    let md = skani_metadata(db);
    let meta = Meta {
        format: FORMAT.into(),
        gtdb_release: release.map(str::to_string),
        skani_version: md.get("skani_version").cloned(),
        skani_metadata: md,
        params_hex: hex(&params.raw),
        sketch_c: params.c,
        marker_c: params.marker_c,
        k: params.k,
        refs: rows.len() as u64,
        markers_total: stats.markers_total,
        sketches_db_size: sk_size,
        fingerprint: "splitmix64-hi32".into(),
        screen_identity: SCREEN_IDENTITY,
        marker_k: MARKER_K,
        record_hash: "sha256, first 16 bytes".into(),
        built_by: format!("gtdbprep {}", env!("CARGO_PKG_VERSION")),
        gzip_source: None,
    };
    write_meta(out, &meta)?;
    Ok(stats)
}

pub fn write_meta(dir: &Path, meta: &Meta) -> Result<()> {
    fs::write(
        dir.join("meta.json"),
        serde_json::to_string_pretty(meta)? + "\n",
    )?;
    Ok(())
}

// ---- Reading and screening ----

pub struct ScreenIndex {
    pub dir: PathBuf,
    pub meta: Meta,
    pub params: Params,
    pub refs: Vec<RefRow>,
}

impl ScreenIndex {
    pub fn open(dir: &Path) -> Result<Self> {
        let meta: Meta = serde_json::from_str(
            &fs::read_to_string(dir.join("meta.json"))
                .with_context(|| format!("reading {}/meta.json", dir.display()))?,
        )
        .context("meta.json")?;
        if meta.format != FORMAT {
            bail!(
                "{}: index format {} (this gtdbprep reads {FORMAT})",
                dir.display(),
                meta.format
            );
        }
        let params = meta.params()?;
        let raw = fs::read(dir.join("refs.bin"))?;
        ensure!(
            raw.len() == meta.refs as usize * ROW,
            "refs.bin has the wrong size for {} references",
            meta.refs
        );
        let refs = raw
            .as_chunks::<ROW>()
            .0
            .iter()
            .map(|b| RefRow::from_bytes(b))
            .collect();
        Ok(ScreenIndex {
            dir: dir.to_path_buf(),
            meta,
            params,
            refs,
        })
    }

    /// Screen each query's marker set; returns, per query, the indices of the references that
    /// pass (ascending). One sequential pass over `markers.idx`, whatever the number of queries.
    pub fn screen(&self, queries: &[Vec<u64>]) -> Result<Vec<Vec<u32>>> {
        // fingerprint -> [(query, how many of its markers have this fingerprint)]
        let mut by_fp: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
        let mut seen = vec![0u64; 1 << 18]; // a bitmap over the top 24 bits
        for (q, markers) in queries.iter().enumerate() {
            let mut fps: Vec<u32> = markers.iter().map(|&m| fingerprint(m)).collect();
            fps.sort_unstable();
            for run in fps.chunk_by(|a, b| a == b) {
                by_fp
                    .entry(run[0])
                    .or_default()
                    .push((q as u32, run.len() as u32));
                let t = (run[0] >> 8) as usize;
                seen[t >> 6] |= 1 << (t & 63);
            }
        }
        let mut out = vec![Vec::new(); queries.len()];
        let mut f = BufReader::with_capacity(8 << 20, File::open(self.dir.join("markers.idx"))?);
        let mut magic = [0u8; 8];
        f.read_exact(&mut magic)?;
        ensure!(&magic == MARKERS_MAGIC, "markers.idx: not a marker index");
        let end = fs::metadata(self.dir.join("markers.idx"))?.len();
        let mut block = Vec::new();
        let mut fps = Vec::new();
        let mut counts: HashMap<u32, u64> = HashMap::new();
        let mut pos = MARKERS_MAGIC.len() as u64;
        for (i, r) in self.refs.iter().enumerate() {
            let next = self.refs.get(i + 1).map_or(end, |n| n.idx_offset);
            ensure!(
                r.idx_offset == pos && next >= pos,
                "markers.idx is out of order"
            );
            block.resize((next - pos) as usize, 0);
            f.read_exact(&mut block)?;
            pos = next;
            decode(&block, r.markers as usize, r.rice_k, &mut fps)
                .with_context(|| format!("reference {i}"))?;
            counts.clear();
            for run in fps.chunk_by(|a, b| a == b) {
                let v = run[0];
                let t = (v >> 8) as usize;
                if seen[t >> 6] & (1 << (t & 63)) == 0 {
                    continue;
                }
                if let Some(hits) = by_fp.get(&v) {
                    for &(q, mult) in hits {
                        *counts.entry(q).or_default() += mult as u64;
                    }
                }
            }
            for (&q, &c) in &counts {
                if passes(c, queries[q as usize].len(), r.markers as usize) {
                    out[q as usize].push(i as u32);
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::super::skani::tests::fake_db;
    use super::*;

    #[test]
    fn rice_round_trip() {
        for n in [0usize, 1, 2, 7, 1000, 20_000] {
            let mut v: Vec<u32> = (0..n as u64).map(|i| fingerprint(i * 7919 + 13)).collect();
            v.push(u32::MAX);
            v.push(0);
            v.push(0);
            v.sort_unstable();
            let k = rice_k(v.len());
            let enc = encode(&v, k);
            let mut out = Vec::new();
            decode(&enc, v.len(), k, &mut out).unwrap();
            assert_eq!(out, v, "n = {n}");
        }
        // Tiny k with a huge gap (long unary run).
        let v = vec![5u32, 4_000_000_000];
        let enc = encode(&v, 0);
        let mut out = Vec::new();
        decode(&enc, 2, 0, &mut out).unwrap();
        assert_eq!(out, v);
    }

    #[test]
    fn threshold_matches_skani() {
        // 0.8^21 = 0.00922..., so 1,000 markers need 9 shared, 100 need 1.
        assert!(passes(9, 1000, 5000));
        assert!(!passes(8, 1000, 5000));
        assert!(passes(1, 100, 100));
        assert!(!passes(0, 100, 100));
        assert!(!passes(5, 0, 100));
        assert!(passes(46, 5000, 5000) && !passes(45, 5000, 5000));
    }

    /// The index screen must keep every reference a direct marker comparison keeps.
    #[test]
    fn screen_keeps_every_reference_skani_keeps() {
        let d = tempfile::tempdir().unwrap();
        let db = d.path().join("db");
        // 30 references with 1,500 markers each; reference r shares r * 2 markers with query 0.
        let query: Vec<u64> = (0..1500u64).map(|i| i * 1_000_003 + 1).collect();
        let refs: Vec<(String, Vec<u64>)> = (0..30u64)
            .map(|r| {
                let mut m: Vec<u64> = (0..1500u64)
                    .map(|i| (r + 1) * 1_000_000_000_000 + i * 17)
                    .collect();
                let n = (r as usize * 2).min(1500);
                m[..n].copy_from_slice(&query[..n]);
                (format!("ref{r}"), m)
            })
            .collect();
        let named: Vec<(&str, Vec<u64>)> =
            refs.iter().map(|(n, m)| (n.as_str(), m.clone())).collect();
        fake_db(&db, &named);
        let out = d.path().join("idx");
        let stats = build(&db, &out, Some("test")).unwrap();
        assert_eq!(stats.refs, 30);
        assert_eq!(stats.markers_total, 30 * 1500);
        let ix = ScreenIndex::open(&out).unwrap();
        let small: Vec<u64> = query[..50].to_vec();
        let got = ix.screen(&[query.clone(), small.clone(), vec![]]).unwrap();
        for (qi, q) in [&query, &small].iter().enumerate() {
            let want: Vec<u32> = refs
                .iter()
                .enumerate()
                .filter(|(_, (_, m))| {
                    let set: std::collections::HashSet<&u64> = m.iter().collect();
                    let shared = q.iter().filter(|x| set.contains(x)).count() as u64;
                    passes(shared, q.len(), m.len())
                })
                .map(|(i, _)| i as u32)
                .collect();
            for w in &want {
                assert!(got[qi].contains(w), "query {qi} lost reference {w}");
            }
            assert!(!want.is_empty());
        }
        // floor(0.00922 * 1500) = 13 shared needed: references 7 and up (14+ shared).
        assert_eq!(got[0], (7..30).collect::<Vec<u32>>());
        assert!(got[2].is_empty());
        // Record hashes match the bytes.
        let db_bytes = fs::read(db.join("sketches.db")).unwrap();
        let r = ix.refs[3];
        let rec = &db_bytes[r.offset as usize..r.offset as usize + r.length as usize];
        assert_eq!(&Sha256::digest(rec)[..16], &r.sha);
        assert_eq!(ix.params.c, 125);
    }
}
