//! Random access into a gzip file, after Mark Adler's `zran.c` (zlib's examples).
//!
//! While a gzip stream is decompressed once, front to back, we note "checkpoints" at deflate
//! block boundaries: the compressed position (byte and bit), the uncompressed position, and the
//! 32 KiB of output before it (the window back-references can reach). From a checkpoint, a raw
//! inflater primed with those few bits and that window can resume decompression, so any
//! uncompressed range can be read by fetching the compressed bytes from the checkpoint before
//! it. Checkpoints are kept only around one tar member (`sketches.db`), at a chosen spacing.
//!
//! The inflater is zlib-rs (`libz-rs-sys`), a Rust port of zlib with zlib's C API.

use anyhow::{Context, Result, bail, ensure};
use libz_rs_sys as z;
use md5::{Digest, Md5};
use std::cell::{Cell, RefCell};
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;
use std::rc::Rc;
use std::sync::Mutex;

pub const WINDOW: usize = 32768;
const MAGIC: &[u8; 8] = b"IRITGZ01";
const HEADER: u64 = 24;
const ENTRY: u64 = 24;

/// A zlib stream that stays at one address (zlib keeps a pointer back to it).
struct Inflater(Box<z::z_stream>);

impl Inflater {
    /// `window_bits`: 47 = gzip or zlib header, auto-detected; -15 = raw deflate.
    fn new(window_bits: i32) -> Result<Self> {
        // SAFETY: an all-zero z_stream (no allocator functions, no buffers) is the documented
        // starting state for inflateInit2.
        let mut s: Box<z::z_stream> = Box::new(unsafe { std::mem::zeroed() });
        let ret = unsafe {
            z::inflateInit2_(
                &mut *s,
                window_bits,
                z::zlibVersion(),
                std::mem::size_of::<z::z_stream>() as i32,
            )
        };
        ensure!(ret == z::Z_OK, "inflateInit2 failed ({ret})");
        Ok(Inflater(s))
    }

    /// One inflate call over `input` into `output`; returns (consumed, produced, return code).
    fn run(&mut self, input: &[u8], output: &mut [u8], flush: i32) -> (usize, usize, i32) {
        let s = &mut *self.0;
        s.next_in = input.as_ptr() as *mut u8;
        s.avail_in = input.len() as u32;
        s.next_out = output.as_mut_ptr();
        s.avail_out = output.len() as u32;
        // SAFETY: next_in/next_out point at live slices of the given lengths for this call.
        let ret = unsafe { z::inflate(s, flush) };
        let consumed = input.len() - s.avail_in as usize;
        let produced = output.len() - s.avail_out as usize;
        s.next_in = std::ptr::null_mut();
        s.next_out = std::ptr::null_mut();
        s.avail_in = 0;
        s.avail_out = 0;
        (consumed, produced, ret)
    }

    fn data_type(&self) -> i32 {
        self.0.data_type
    }

    fn reset(&mut self) -> Result<()> {
        let ret = unsafe { z::inflateReset(&mut *self.0) };
        ensure!(ret == z::Z_OK, "inflateReset failed ({ret})");
        Ok(())
    }

    fn prime(&mut self, bits: i32, value: i32) -> Result<()> {
        let ret = unsafe { z::inflatePrime(&mut *self.0, bits, value) };
        ensure!(ret == z::Z_OK, "inflatePrime failed ({ret})");
        Ok(())
    }

    fn set_dictionary(&mut self, dict: &[u8]) -> Result<()> {
        let ret =
            unsafe { z::inflateSetDictionary(&mut *self.0, dict.as_ptr(), dict.len() as u32) };
        ensure!(ret == z::Z_OK, "inflateSetDictionary failed ({ret})");
        Ok(())
    }
}

impl Drop for Inflater {
    fn drop(&mut self) {
        unsafe {
            z::inflateEnd(&mut *self.0);
        }
    }
}

/// A place decompression can restart from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    /// Compressed offset of the first full byte after the checkpoint.
    pub in_off: u64,
    /// Bits of the byte before `in_off` that belong after the checkpoint (0-7).
    pub bits: u8,
    /// Uncompressed offset.
    pub out_off: u64,
    pub window: Vec<u8>,
}

/// Which uncompressed range to keep checkpoints for; set by the tar reader once it has seen the
/// member's header.
pub type Target = Rc<Cell<Option<(u64, u64)>>>;

/// Decompresses a gzip stream as a `Read`, noting checkpoints on the way.
pub struct Scanner<R: Read> {
    input: R,
    inbuf: Vec<u8>,
    in_pos: usize,
    in_len: usize,
    inf: Inflater,
    outbuf: Vec<u8>,
    out_pos: usize,
    out_len: usize,
    ring: Vec<u8>,
    ring_pos: usize,
    totin: u64,
    totout: u64,
    last: Option<u64>,
    spacing: u64,
    target: Target,
    recent: Vec<Checkpoint>,
    pub checkpoints: Rc<RefCell<Vec<Checkpoint>>>,
    md5: Md5,
    pub members: u64,
    finished: bool,
}

impl<R: Read> Scanner<R> {
    pub fn new(input: R, spacing: u64, target: Target) -> Result<Self> {
        ensure!(spacing >= 1 << 16, "spacing must be at least 64 KiB");
        Ok(Scanner {
            input,
            inbuf: vec![0; 1 << 20],
            in_pos: 0,
            in_len: 0,
            inf: Inflater::new(47)?,
            outbuf: vec![0; 1 << 16],
            out_pos: 0,
            out_len: 0,
            ring: vec![0; WINDOW],
            ring_pos: 0,
            totin: 0,
            totout: 0,
            last: None,
            spacing,
            target,
            recent: Vec::new(),
            checkpoints: Rc::new(RefCell::new(Vec::new())),
            md5: Md5::new(),
            members: 0,
            finished: false,
        })
    }

    pub fn compressed_size(&self) -> u64 {
        self.totin
    }

    pub fn md5_hex(&self) -> String {
        super::screen::hex(&self.md5.clone().finalize())
    }

    fn window(&self) -> Vec<u8> {
        let n = (self.totout as usize).min(WINDOW);
        let mut w = Vec::with_capacity(n);
        if self.totout as usize >= WINDOW {
            w.extend_from_slice(&self.ring[self.ring_pos..]);
            w.extend_from_slice(&self.ring[..self.ring_pos]);
        } else {
            w.extend_from_slice(&self.ring[..n]);
        }
        w
    }

    fn fill_input(&mut self) -> Result<bool> {
        if self.in_pos < self.in_len {
            return Ok(true);
        }
        let n = loop {
            match self.input.read(&mut self.inbuf) {
                Ok(n) => break n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e.into()),
            }
        };
        self.md5.update(&self.inbuf[..n]);
        self.in_pos = 0;
        self.in_len = n;
        Ok(n > 0)
    }

    fn note_checkpoint(&mut self) {
        let due = self.last.is_none_or(|l| self.totout - l >= self.spacing);
        if !due {
            return;
        }
        self.last = Some(self.totout);
        let cp = Checkpoint {
            in_off: self.totin,
            bits: (self.inf.data_type() & 7) as u8,
            out_off: self.totout,
            window: self.window(),
        };
        match self.target.get() {
            None => {
                // Range not known yet: remember the last few.
                self.recent.push(cp);
                if self.recent.len() > 4 {
                    self.recent.remove(0);
                }
            }
            Some((start, end)) => {
                let mut all = self.checkpoints.borrow_mut();
                if all.is_empty() {
                    // The latest remembered checkpoint at or before the start.
                    if let Some(p) = self.recent.iter().rev().find(|c| c.out_off <= start) {
                        all.push(p.clone());
                    }
                    for p in self.recent.drain(..) {
                        if p.out_off > start && p.out_off < end {
                            all.push(p);
                        }
                    }
                }
                if cp.out_off <= start {
                    all.clear();
                    all.push(cp);
                } else if cp.out_off < end {
                    all.push(cp);
                } else if all.last().is_some_and(|l| l.out_off < end) {
                    // One past the end: bounds the compressed bytes of the last range.
                    all.push(cp);
                }
            }
        }
    }

    /// Decompress some more into `outbuf`. Returns false at the end of the input.
    fn step(&mut self) -> Result<bool> {
        loop {
            if self.finished {
                return Ok(false);
            }
            if !self.fill_input()? {
                bail!("the gzip data ends early (after {} bytes)", self.totin);
            }
            let (c, p, ret) = {
                let input = &self.inbuf[self.in_pos..self.in_len];
                let out = &mut self.outbuf[..];
                self.inf.run(input, out, z::Z_BLOCK)
            };
            self.in_pos += c;
            self.totin += c as u64;
            self.totout += p as u64;
            let mut fresh = &self.outbuf[..p];
            if fresh.len() > WINDOW {
                fresh = &fresh[fresh.len() - WINDOW..];
            }
            while !fresh.is_empty() {
                let n = fresh.len().min(WINDOW - self.ring_pos);
                self.ring[self.ring_pos..self.ring_pos + n].copy_from_slice(&fresh[..n]);
                self.ring_pos = (self.ring_pos + n) % WINDOW;
                fresh = &fresh[n..];
            }
            self.out_pos = 0;
            self.out_len = p;
            match ret {
                z::Z_OK | z::Z_BUF_ERROR => {}
                z::Z_STREAM_END => {
                    self.members += 1;
                    // Another gzip member may follow.
                    if self.fill_input()? {
                        self.inf.reset()?;
                    } else {
                        self.finished = true;
                    }
                }
                e => bail!("corrupt gzip data near byte {} ({e})", self.totin),
            }
            // At a block boundary that is not the end of the last block.
            if ret != z::Z_STREAM_END && self.inf.data_type() & 0xc0 == 0x80 {
                self.note_checkpoint();
            }
            if p > 0 || self.finished {
                return Ok(p > 0);
            }
        }
    }
}

impl<R: Read> Read for Scanner<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.out_pos == self.out_len && !self.step().map_err(std::io::Error::other)? {
            return Ok(0);
        }
        let n = buf.len().min(self.out_len - self.out_pos);
        buf[..n].copy_from_slice(&self.outbuf[self.out_pos..self.out_pos + n]);
        self.out_pos += n;
        Ok(n)
    }
}

/// A member of the tar stream.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Member {
    pub path: String,
    pub offset: u64,
    pub size: u64,
}

/// What a scan found.
pub struct ScanResult {
    pub members: Vec<Member>,
    pub target: Option<Member>,
    pub checkpoints: Vec<Checkpoint>,
    pub compressed_size: u64,
    pub md5: String,
    pub gzip_members: u64,
}

/// Read a `.tar.gz` stream to the end. Lists its members and keeps checkpoints every `spacing`
/// uncompressed bytes across the member whose path ends with `member_suffix`.
pub fn scan<R: Read>(input: R, spacing: u64, member_suffix: &str) -> Result<ScanResult> {
    let target: Target = Rc::new(Cell::new(None));
    let scanner = Scanner::new(input, spacing, target.clone())?;
    let checkpoints = scanner.checkpoints.clone();
    let mut ar = tar::Archive::new(scanner);
    let mut members = Vec::new();
    let mut found = None;
    for e in ar.entries().context("reading the tar stream")? {
        let e = e.context("reading the tar stream")?;
        let path = e.path()?.to_string_lossy().into_owned();
        let m = Member {
            path: path.clone(),
            offset: e.raw_file_position(),
            size: e.size(),
        };
        if found.is_none() && path.ends_with(member_suffix) && m.size > 0 {
            target.set(Some((m.offset, m.offset + m.size)));
            found = Some(m.clone());
        }
        members.push(m);
    }
    let mut scanner = ar.into_inner();
    // Drain whatever follows the tar end marker so the hash covers the whole file.
    std::io::copy(&mut scanner, &mut std::io::sink())?;
    let cps = std::mem::take(&mut *checkpoints.borrow_mut());
    Ok(ScanResult {
        members,
        target: found,
        checkpoints: cps,
        compressed_size: scanner.compressed_size(),
        md5: scanner.md5_hex(),
        gzip_members: scanner.members,
    })
}

/// Write checkpoints (every `every`-th, plus the last) to a seek index file.
pub fn write_index(path: &Path, cps: &[Checkpoint], spacing: u64, every: usize) -> Result<usize> {
    let every = every.max(1);
    let mut keep: Vec<&Checkpoint> = cps.iter().step_by(every).collect();
    if let Some(last) = cps.last()
        && keep.last().is_none_or(|k| *k != last)
    {
        keep.push(last);
    }
    let mut w = BufWriter::new(File::create(path)?);
    w.write_all(MAGIC)?;
    w.write_all(&(spacing * every as u64).to_le_bytes())?;
    w.write_all(&(keep.len() as u64).to_le_bytes())?;
    for c in &keep {
        w.write_all(&c.in_off.to_le_bytes())?;
        w.write_all(&c.out_off.to_le_bytes())?;
        w.write_all(&[c.bits, 0, 0, 0])?;
        w.write_all(&(c.window.len() as u32).to_le_bytes())?;
    }
    for c in &keep {
        let mut slot = c.window.clone();
        slot.resize(WINDOW, 0);
        w.write_all(&slot)?;
    }
    w.flush()?;
    Ok(keep.len())
}

/// A seek index on disk: the table is read up front, windows when used.
pub struct GzIndex {
    file: Mutex<File>,
    pub spacing: u64,
    pub points: Vec<(u64, u64, u8, u32)>, // in_off, out_off, bits, window length
}

impl GzIndex {
    pub fn open(path: &Path) -> Result<Self> {
        let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mut head = [0u8; HEADER as usize];
        f.read_exact(&mut head)?;
        ensure!(
            &head[..8] == MAGIC,
            "{}: not a gzip seek index",
            path.display()
        );
        let spacing = u64::from_le_bytes(head[8..16].try_into().unwrap());
        let n = u64::from_le_bytes(head[16..24].try_into().unwrap());
        ensure!(n < 1 << 32, "{}: corrupt", path.display());
        let mut table = vec![0u8; (n * ENTRY) as usize];
        f.read_exact(&mut table)?;
        let points = table
            .as_chunks::<{ ENTRY as usize }>()
            .0
            .iter()
            .map(|e| {
                (
                    u64::from_le_bytes(e[0..8].try_into().unwrap()),
                    u64::from_le_bytes(e[8..16].try_into().unwrap()),
                    e[16],
                    u32::from_le_bytes(e[20..24].try_into().unwrap()),
                )
            })
            .collect::<Vec<_>>();
        ensure!(
            points
                .windows(2)
                .all(|w| w[0].1 < w[1].1 && w[0].0 <= w[1].0),
            "{}: checkpoints out of order",
            path.display()
        );
        Ok(GzIndex {
            file: Mutex::new(f),
            spacing,
            points,
        })
    }

    /// Write every `every`-th checkpoint (and the last) to a new index file.
    pub fn copy_every(&self, path: &Path, every: usize) -> Result<usize> {
        let mut cps = Vec::with_capacity(self.points.len());
        for (i, p) in self.points.iter().enumerate() {
            cps.push(Checkpoint {
                in_off: p.0,
                bits: p.2,
                out_off: p.1,
                window: if i % every.max(1) == 0 || i + 1 == self.points.len() {
                    self.window(i)?
                } else {
                    Vec::new()
                },
            });
        }
        write_index(path, &cps, self.spacing, every)
    }

    /// The last checkpoint at or before uncompressed offset `out`.
    pub fn before(&self, out: u64) -> Option<usize> {
        let i = self.points.partition_point(|p| p.1 <= out);
        i.checked_sub(1)
    }

    /// The first checkpoint at or after `out`.
    pub fn after(&self, out: u64) -> Option<usize> {
        let i = self.points.partition_point(|p| p.1 < out);
        (i < self.points.len()).then_some(i)
    }

    fn window(&self, i: usize) -> Result<Vec<u8>> {
        let n = self.points.len() as u64;
        let mut f = self.file.lock().unwrap_or_else(|e| e.into_inner());
        f.seek(SeekFrom::Start(
            HEADER + n * ENTRY + i as u64 * WINDOW as u64,
        ))?;
        let mut w = vec![0u8; self.points[i].3 as usize];
        f.read_exact(&mut w)?;
        Ok(w)
    }

    /// The compressed byte range to fetch to resume at checkpoint `i` and run up to
    /// uncompressed offset `end`: `[from, to)`, given the compressed file size.
    pub fn compressed_span(&self, i: usize, end: u64, compressed_size: u64) -> (u64, u64) {
        let (in_off, _, bits, _) = self.points[i];
        let from = in_off - u64::from(bits > 0);
        let to = self.after(end).map_or(compressed_size, |j| {
            (self.points[j].0 + 1).min(compressed_size)
        });
        (from, to.max(in_off))
    }

    /// Decompress from checkpoint `i`, given the compressed bytes from `compressed_span`, and
    /// return the uncompressed `ranges` (absolute offsets, each at or after the checkpoint).
    pub fn extract(&self, i: usize, data: &[u8], ranges: &[(u64, u64)]) -> Result<Vec<Vec<u8>>> {
        let (_, out_off, bits, _) = self.points[i];
        let mut inf = Inflater::new(-15)?;
        let mut data = data;
        if bits > 0 {
            ensure!(!data.is_empty(), "no compressed data");
            inf.prime(bits as i32, (data[0] >> (8 - bits)) as i32)?;
            data = &data[1..];
        }
        inf.set_dictionary(&self.window(i)?)?;
        let mut outs: Vec<Vec<u8>> = ranges
            .iter()
            .map(|&(s, l)| {
                assert!(s >= out_off);
                Vec::with_capacity(l as usize)
            })
            .collect();
        let end = ranges.iter().map(|&(s, l)| s + l).max().unwrap_or(out_off);
        let mut pos = out_off;
        let mut buf = vec![0u8; 1 << 18];
        while pos < end {
            let (c, p, ret) = inf.run(data, &mut buf, z::Z_NO_FLUSH);
            data = &data[c..];
            for (k, &(s, l)) in ranges.iter().enumerate() {
                let a = s.max(pos);
                let b = (s + l).min(pos + p as u64);
                if a < b {
                    outs[k].extend_from_slice(&buf[(a - pos) as usize..(b - pos) as usize]);
                }
            }
            pos += p as u64;
            match ret {
                z::Z_OK => {}
                z::Z_BUF_ERROR if c == 0 && p == 0 => {
                    bail!("ran out of compressed data at uncompressed offset {pos}")
                }
                z::Z_BUF_ERROR => {}
                z::Z_STREAM_END if pos < end => {
                    bail!(
                        "reached the end of a gzip member at {pos}; multi-member reads are not supported"
                    )
                }
                z::Z_STREAM_END => {}
                e => bail!("corrupt deflate data after checkpoint {i} ({e})"),
            }
        }
        Ok(outs)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;

    /// Pseudo-random but compressible bytes.
    pub fn sample(n: usize, seed: u64) -> Vec<u8> {
        let mut x = seed;
        (0..n)
            .map(|i| {
                x = x
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                if i % 3 == 0 {
                    b"ACGT"[(x >> 62) as usize]
                } else {
                    (x >> 56) as u8
                }
            })
            .collect()
    }

    /// A tar.gz holding `before.bin`, `db/sketches.db` (= `payload`) and `after.txt`.
    pub fn tar_gz(payload: &[u8]) -> Vec<u8> {
        let mut b = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::new(6)));
        for (name, data) in [
            ("release/before.bin", sample(300_000, 1)),
            ("release/db/sketches.db", payload.to_vec()),
            ("release/after.txt", b"done\n".to_vec()),
        ] {
            let mut h = tar::Header::new_gnu();
            h.set_size(data.len() as u64);
            h.set_mode(0o644);
            h.set_cksum();
            b.append_data(&mut h, name, &data[..]).unwrap();
        }
        b.into_inner().unwrap().finish().unwrap()
    }

    #[test]
    fn checkpoints_give_back_any_range() {
        let payload = sample(3_000_000, 7);
        let gz = tar_gz(&payload);
        let r = scan(&gz[..], 1 << 16, "db/sketches.db").unwrap();
        let t = r.target.clone().unwrap();
        assert_eq!(t.size, payload.len() as u64);
        assert_eq!(r.members.len(), 3);
        assert_eq!(r.compressed_size, gz.len() as u64);
        assert_eq!(r.md5, super::super::screen::hex(&Md5::digest(&gz)));
        assert_eq!(r.gzip_members, 1);
        assert!(r.checkpoints.len() > 10, "{}", r.checkpoints.len());
        assert!(r.checkpoints[0].out_off <= t.offset);
        assert!(
            r.checkpoints.last().unwrap().out_off >= t.offset + t.size || r.checkpoints.len() > 1
        );

        let d = tempfile::tempdir().unwrap();
        for every in [1usize, 3] {
            let p = d.path().join(format!("gz{every}.idx"));
            write_index(&p, &r.checkpoints, 1 << 16, every).unwrap();
            let ix = GzIndex::open(&p).unwrap();
            for &(s, l) in &[
                (0u64, 10u64),
                (12_345, 70_000),
                (2_999_000, 1000),
                (1_500_000, 1),
            ] {
                let abs = t.offset + s;
                let i = ix.before(abs).unwrap();
                let (from, to) = ix.compressed_span(i, abs + l, gz.len() as u64);
                let got = ix
                    .extract(i, &gz[from as usize..to as usize], &[(abs, l)])
                    .unwrap();
                assert_eq!(
                    got[0],
                    &payload[s as usize..(s + l) as usize],
                    "every {every}, at {s}"
                );
            }
        }
    }

    #[test]
    fn concatenated_members_are_read_through() {
        let mut gz = Vec::new();
        for part in [sample(100_000, 3), sample(100_000, 4)] {
            let mut e = GzEncoder::new(Vec::new(), Compression::new(6));
            e.write_all(&part).unwrap();
            gz.extend(e.finish().unwrap());
        }
        let mut s = Scanner::new(&gz[..], 1 << 16, Rc::new(Cell::new(None))).unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).unwrap();
        assert_eq!(out.len(), 200_000);
        assert_eq!(s.members, 2);
    }
}
