//! Readers for aligned FASTA (plain or gzip), Clustal and Stockholm.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

use anyhow::{Context, Result, bail};
use rayon::prelude::*;

use crate::Alignment;
use crate::alphabet::{Alphabet, Census, Kind, SeqType};
use crate::matrix::{Matrix, encode_at, encode_row};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Fasta,
    Clustal,
    Stockholm,
}

const BOM: &[u8] = b"\xEF\xBB\xBF";

fn strip_bom(b: &[u8]) -> &[u8] {
    b.strip_prefix(BOM).unwrap_or(b)
}

pub fn detect_format(head: &[u8]) -> Result<Format> {
    let head = strip_bom(head);
    let start = head.iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(head.len());
    let head = &head[start..];
    if head.starts_with(b">") {
        Ok(Format::Fasta)
    } else if head.starts_with(b"# STOCKHOLM") {
        Ok(Format::Stockholm)
    } else if [&b"CLUSTAL"[..], b"MUSCLE", b"PROBCONS"].iter().any(|p| head.starts_with(p)) {
        Ok(Format::Clustal)
    } else {
        bail!("unrecognised format: expected aligned FASTA ('>'), Clustal ('CLUSTAL') or Stockholm ('# STOCKHOLM')")
    }
}

/// Bytes of FASTA read per batch; each batch's records are encoded in parallel.
const CHUNK: usize = 64 << 20;

/// Residues (gaps excluded) read across the first rows to choose the alphabet.
const SAMPLE: usize = 64 << 10;

type Reader<'a> = Box<dyn BufRead + 'a>;

/// Lower case and `.` mark insert states (A2M, Stockholm from HMMER and Pfam).
#[inline]
fn insert_mark(b: u8) -> bool {
    b.is_ascii_lowercase() || b == b'.'
}

/// Set bit `start + k` of `bits` for every residue or gap `k` of `seq` that is not an insert mark
/// (upper case, `-`, `~`, `*`, `?`), growing `bits` as needed. Whitespace and digits are skipped,
/// as the encoder skips them. A column is an insert column when no row sets its bit.
fn match_marks(seq: &[u8], start: usize, bits: &mut Vec<u64>) {
    let mut k = start;
    for &b in seq {
        if b.is_ascii_whitespace() || b.is_ascii_digit() {
            continue;
        }
        if !insert_mark(b) {
            if bits.len() <= k / 64 {
                bits.resize(k / 64 + 1, 0);
            }
            bits[k / 64] |= 1 << (k % 64);
        }
        k += 1;
    }
}

/// Insert columns from match marks: the columns in `0..cols` no row marked as a match.
fn inserts_from(marks: &[u64], cols: usize) -> Vec<u64> {
    let mut v: Vec<u64> = (0..cols.div_ceil(64)).map(|i| !marks.get(i).copied().unwrap_or(0)).collect();
    if let Some(last) = v.last_mut()
        && !cols.is_multiple_of(64)
    {
        *last &= (1u64 << (cols % 64)) - 1;
    }
    if v.iter().all(|&w| w == 0) { Vec::new() } else { v }
}

/// Insert columns from a Stockholm `#=GC RF` line: `.`, `-`, `~` and `_` are inserts, anything
/// else (`x`, or residues) a match column.
fn inserts_from_rf(rf: &[u8]) -> Vec<u64> {
    let mut v = vec![0u64; rf.len().div_ceil(64)];
    for (c, &b) in rf.iter().enumerate() {
        if matches!(b, b'.' | b'-' | b'~' | b'_') {
            v[c / 64] |= 1 << (c % 64);
        }
    }
    if v.iter().all(|&w| w == 0) { Vec::new() } else { v }
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Fasta => "FASTA",
            Format::Clustal => "Clustal",
            Format::Stockholm => "Stockholm",
        }
    }
}

/// The format of a file and whether it is gzipped, from its first few kilobytes.
pub fn sniff(path: &Path) -> Result<(Format, bool)> {
    let mut f = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut head = vec![0u8; 4096];
    let n = f.read(&mut head)?;
    let gz = n >= 2 && head[..2] == [0x1f, 0x8b];
    if gz {
        let mut d = flate2::read::MultiGzDecoder::new(File::open(path)?);
        let mut n = 0;
        while n < head.len() {
            match d.read(&mut head[n..])? {
                0 => break,
                k => n += k,
            }
        }
        head.truncate(n);
    } else {
        head.truncate(n);
    }
    Ok((detect_format(&head)?, gz))
}

/// What a load has done so far, shared with the thread showing it, and the flag that cancels it.
/// The reader counts bytes on every read (at most [`READ_MAX`] at a time) and checks the flag
/// there, so a cancelled load stops within one read or, for FASTA, after encoding the batch in
/// hand.
#[derive(Debug, Default)]
pub struct Progress {
    /// Bytes read from the file in this pass (compressed bytes for gzip).
    bytes: AtomicU64,
    /// The file's size in bytes; 0 when unknown.
    total: AtomicU64,
    /// Sequences read so far.
    rows: AtomicU64,
    /// Pass over the file: Clustal and Stockholm are read twice (names, then residues).
    pass: AtomicU8,
    stage: AtomicU8,
    cancel: AtomicBool,
}

/// The step a load is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Read,
    Columns,
    Overview,
}

/// A copy of [`Progress`] at one moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub bytes: u64,
    pub total: u64,
    pub rows: u64,
    pub pass: u8,
    pub stage: Stage,
}

impl Snapshot {
    /// Share of the file read in this pass, 0–1 (0 when the size is unknown).
    pub fn fraction(&self) -> f64 {
        if self.total == 0 { 0.0 } else { (self.bytes as f64 / self.total as f64).min(1.0) }
    }
}

impl Progress {
    pub fn new(total: u64) -> Self {
        let p = Progress::default();
        p.total.store(total, Ordering::Relaxed);
        p
    }

    /// Ask the load to stop; it returns [`Cancelled`] at its next check.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// `Err(Cancelled)` once [`Progress::cancel`] was called.
    pub fn check(&self) -> Result<()> {
        if self.is_cancelled() { Err(Cancelled.into()) } else { Ok(()) }
    }

    pub fn set_stage(&self, s: Stage) {
        self.stage.store(s as u8, Ordering::Relaxed);
    }

    pub fn set_rows(&self, n: usize) {
        self.rows.store(n as u64, Ordering::Relaxed);
    }

    /// A new pass over the file: bytes count from 0 again.
    fn start_pass(&self) {
        self.bytes.store(0, Ordering::Relaxed);
        self.pass.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self) -> Snapshot {
        let stage = match self.stage.load(Ordering::Relaxed) {
            1 => Stage::Columns,
            2 => Stage::Overview,
            _ => Stage::Read,
        };
        Snapshot {
            bytes: self.bytes.load(Ordering::Relaxed),
            total: self.total.load(Ordering::Relaxed),
            rows: self.rows.load(Ordering::Relaxed),
            pass: self.pass.load(Ordering::Relaxed).max(1),
            stage,
        }
    }
}

/// The error a cancelled load returns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cancelled;

impl std::fmt::Display for Cancelled {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

/// Whether `e` (or an error it wraps, I/O errors included) is [`Cancelled`].
pub fn is_cancelled(e: &anyhow::Error) -> bool {
    e.chain().any(|c| c.is::<Cancelled>() || c.downcast_ref::<std::io::Error>().and_then(|io| io.get_ref()).is_some_and(|i| i.is::<Cancelled>()))
}

/// Largest single read while a load reports progress, so cancel is noticed quickly.
pub const READ_MAX: usize = 256 << 10;

/// A reader that adds what it reads to a [`Progress`] and fails with [`Cancelled`] once the load
/// is cancelled. Reads are cut to [`READ_MAX`].
struct Counting<'p, R> {
    inner: R,
    progress: &'p Progress,
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.progress.is_cancelled() {
            return Err(std::io::Error::other(Cancelled));
        }
        let k = buf.len().min(READ_MAX);
        let n = self.inner.read(&mut buf[..k])?;
        self.progress.bytes.fetch_add(n as u64, Ordering::Relaxed);
        Ok(n)
    }
}

/// Load an alignment, decompressing gzip transparently.
pub fn load(path: &Path) -> Result<Alignment> {
    load_as(path, None)
}

/// [`load`] with the residue type given instead of guessed (`--type`).
pub fn load_as(path: &Path, ty: Option<SeqType>) -> Result<Alignment> {
    load_with(path, ty, None)
}

/// [`load_as`] reporting to `progress`, and stopping with [`Cancelled`] when it is cancelled.
pub fn load_with(path: &Path, ty: Option<SeqType>, progress: Option<&Progress>) -> Result<Alignment> {
    let open = || -> Result<Reader<'_>> {
        let f = || File::open(path).with_context(|| format!("opening {}", path.display()));
        let mut magic = [0u8; 2];
        let gz = f()?.read(&mut magic)? == 2 && magic == [0x1f, 0x8b];
        let raw: Box<dyn Read + '_> = match progress {
            Some(p) => {
                p.start_pass();
                Box::new(Counting { inner: f()?, progress: p })
            }
            None => Box::new(f()?),
        };
        Ok(if gz {
            Box::new(BufReader::with_capacity(1 << 16, flate2::read::MultiGzDecoder::new(raw)))
        } else {
            Box::new(BufReader::with_capacity(1 << 16, raw))
        })
    };
    read(&open, ty, progress).with_context(|| format!("reading {}", path.display()))
}

pub fn parse_bytes(data: &[u8]) -> Result<Alignment> {
    parse_bytes_as(data, None)
}

pub fn parse_bytes_as(data: &[u8], ty: Option<SeqType>) -> Result<Alignment> {
    read(&|| Ok(Box::new(data) as Reader), ty, None)
}

/// [`parse_bytes`] reporting to `progress` (for tests of progress and cancel).
pub fn parse_bytes_with(data: &[u8], progress: &Progress) -> Result<Alignment> {
    read(
        &|| {
            progress.start_pass();
            Ok(Box::new(BufReader::with_capacity(1 << 16, Counting { inner: data, progress })) as Reader)
        },
        None,
        Some(progress),
    )
}

/// Parse from `open`, which must return the same content each time: Clustal and Stockholm are read
/// twice, and FASTA again in the rare case that the residue type changes after rows were packed.
fn read<'a>(open: &dyn Fn() -> Result<Reader<'a>>, ty: Option<SeqType>, progress: Option<&Progress>) -> Result<Alignment> {
    let mut r = open()?;
    match detect_format(r.fill_buf()?)? {
        Format::Fasta => {
            let mut forced = None;
            loop {
                let res = fasta(r, CHUNK, forced, ty, progress);
                let Some(a) = res.as_ref().err().and_then(|e| e.downcast_ref::<Restart>()).map(|r| r.0) else { return res };
                forced = Some(a);
                r = open()?;
            }
        }
        f => interleaved(r, open, f, ty, progress),
    }
}

/// The residue type changed after earlier batches were packed and they can't be converted without
/// loss (a `?` stored as N would become asparagine): read the input again with this alphabet.
#[derive(Debug)]
struct Restart(&'static Alphabet);

impl std::fmt::Display for Restart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "residue type changed to {:?}", self.0)
    }
}

impl std::error::Error for Restart {}

/// Accumulates FASTA rows. The alphabet comes from a sample of residues across the first rows and
/// widens (DNA or RNA to DNA/RNA with T and U apart, or to protein) if a later row needs it.
struct Builder {
    alpha: &'static Alphabet,
    forced: bool,
    /// Residue type from `--type`: the alphabet may still widen within nucleotides, never across.
    ty: Option<SeqType>,
    names: Vec<String>,
    matrix: Option<Matrix>,
    /// Some packed row came from a `?`, stored as N in nucleotide alphabets.
    question: bool,
    /// Columns some row marked as a match ([`match_marks`]); `None` once a row with no insert
    /// mark at all was seen, which makes every column a match column.
    matches: Option<Vec<u64>>,
}

type Record<'a> = (&'a [u8], &'a [u8]);

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b.trim_ascii()).into_owned()
}

impl Builder {
    fn new(forced: Option<&'static Alphabet>, ty: Option<SeqType>) -> Self {
        Builder {
            alpha: forced.unwrap_or(&crate::alphabet::DNA),
            forced: forced.is_some(),
            ty,
            names: Vec::new(),
            matrix: None,
            question: false,
            matches: Some(Vec::new()),
        }
    }

    /// Append (name, raw sequence) records, encoding them in parallel.
    fn push(&mut self, recs: &[Record]) -> Result<()> {
        let Some((name, seq)) = recs.first() else { return Ok(()) };
        if self.matrix.is_none() {
            if !self.forced {
                let (mut census, mut left) = (Census::default(), SAMPLE);
                for (_, s) in recs {
                    left -= census.add(s, left);
                    if left == 0 {
                        break;
                    }
                }
                self.alpha = census.alphabet_as(self.ty);
            }
            let cols = loop {
                match encode_row(self.alpha, seq, &mut []) {
                    Ok(n) => break n,
                    Err(b) => self.alpha = widen(self.alpha, self.ty, b, &lossy(name))?,
                }
            };
            if cols == 0 {
                bail!("sequence '{}' is empty", lossy(name));
            }
            self.matrix = Some(Matrix::empty(cols, self.alpha.bits));
        }
        loop {
            let alpha = self.alpha;
            let m = self.matrix.as_mut().unwrap();
            let cols = m.cols;
            let rows = m.grow(recs.len());
            let first_err = rows
                .into_par_iter()
                .zip(recs.par_iter())
                .enumerate()
                .filter_map(|(i, (row, (_, seq)))| match encode_row(alpha, seq, row) {
                    Ok(n) if n == cols => None,
                    r => Some((i, r)),
                })
                .min_by_key(|e| e.0);
            let Some((i, r)) = first_err else { break };
            for _ in 0..recs.len() {
                m.pop_row();
            }
            let name = lossy(recs[i].0);
            match r {
                Err(b) => {
                    let to = widen(alpha, self.ty, b, &name)?;
                    if m.rows > 0 {
                        if to.kind == Kind::Protein && self.question {
                            return Err(Restart(to).into());
                        }
                        *m = m.convert(alpha, to);
                    } else {
                        *m = Matrix::empty(cols, to.bits);
                    }
                    self.alpha = to;
                }
                Ok(n) => bail!(not_aligned(&name, n, cols)),
            }
        }
        if self.matches.is_some() {
            // Most files have no insert marks: the first row without one settles it.
            if recs.par_iter().any(|(_, s)| !s.iter().any(|&b| insert_mark(b))) {
                self.matches = None;
            } else {
                let batch = recs
                    .par_iter()
                    .fold(Vec::new, |mut acc, (_, s)| {
                        match_marks(s, 0, &mut acc);
                        acc
                    })
                    .reduce(Vec::new, |mut a, b| {
                        if a.len() < b.len() {
                            a.resize(b.len(), 0);
                        }
                        a.iter_mut().zip(&b).for_each(|(x, y)| *x |= y);
                        a
                    });
                let m = self.matches.as_mut().unwrap();
                if m.len() < batch.len() {
                    m.resize(batch.len(), 0);
                }
                m.iter_mut().zip(&batch).for_each(|(x, y)| *x |= y);
            }
        }
        if self.alpha.kind == Kind::Nucleotide && !self.question {
            self.question = recs.par_iter().any(|(_, s)| memchr::memchr(b'?', s).is_some());
        }
        self.names.par_extend(recs.par_iter().map(|(h, _)| lossy(h)));
        Ok(())
    }

    fn finish(self) -> Result<Alignment> {
        let Some(matrix) = self.matrix else { bail!("no sequences found") };
        let inserts = self.matches.map_or_else(Vec::new, |m| inserts_from(&m, matrix.cols));
        Ok(Alignment { names: self.names, matrix, alpha: self.alpha, inserts, accessions: Vec::new(), newick: None, gc: Vec::new(), gr: Vec::new(), warnings: Vec::new() })
    }
}

fn bad_char(b: u8, name: &str) -> anyhow::Error {
    anyhow::anyhow!("sequence '{name}': unexpected character {:?}", b as char)
}

/// `alpha.widen(b)`, refusing to cross between nucleotides and protein when `--type` was given.
fn widen(alpha: &'static Alphabet, ty: Option<SeqType>, b: u8, name: &str) -> Result<&'static Alphabet> {
    let to = alpha.widen(b).ok_or_else(|| bad_char(b, name))?;
    match ty {
        Some(t) if t.kind() != to.kind => Err(forced_char(b, name, t)),
        _ => Ok(to),
    }
}

fn forced_char(b: u8, name: &str, ty: SeqType) -> anyhow::Error {
    anyhow::anyhow!("sequence '{name}': {:?} is not a {} residue (the type was set with --type)", b as char, format!("{ty:?}").to_lowercase())
}

fn not_aligned(name: &str, n: usize, cols: usize) -> String {
    format!("sequence '{name}' has {n} columns but the first has {cols}: not an alignment (sequences must have equal length)")
}

/// FASTA in batches of about `chunk` bytes: read, split at record starts, encode the batch in parallel.
/// Memory stays at the packed matrix plus one batch, for plain and gzip input alike.
fn fasta(mut r: impl Read, mut chunk: usize, forced: Option<&'static Alphabet>, ty: Option<SeqType>, progress: Option<&Progress>) -> Result<Alignment> {
    let mut b = Builder::new(forced, ty);
    let mut buf: Vec<u8> = Vec::new();
    let mut first = true;
    loop {
        let want = chunk.saturating_sub(buf.len()) as u64;
        // Room for the whole batch first: growing while reading doubles the buffer (to twice the
        // batch) for readers that can't say how much is left, such as the progress-counting one.
        buf.reserve_exact(want as usize);
        let eof = (&mut r).take(want).read_to_end(&mut buf)? < want as usize;
        if first {
            let skip = if buf.starts_with(BOM) { BOM.len() } else { 0 };
            let skip = skip + buf[skip..].iter().position(|b| !b.is_ascii_whitespace()).unwrap_or(buf.len() - skip);
            buf.drain(..skip);
            if !buf.is_empty() && buf[0] != b'>' {
                bail!("FASTA: data before the first '>' header");
            }
            first = buf.is_empty();
        }
        let cut = if eof { buf.len() } else { memchr::memmem::rfind(&buf, b"\n>").map_or(0, |p| p + 1) };
        if cut == 0 && !eof {
            chunk *= 2; // one record is larger than the batch
            continue;
        }
        let data = &buf[..cut];
        let mut starts: Vec<usize> = if data.is_empty() { Vec::new() } else { vec![0] };
        starts.extend(memchr::memmem::find_iter(data, b"\n>").map(|p| p + 1));
        let recs: Vec<Record> = starts
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let rec = &data[s + 1..starts.get(i + 1).copied().unwrap_or(data.len())];
                let nl = memchr::memchr(b'\n', rec).unwrap_or(rec.len());
                (&rec[..nl], &rec[nl..])
            })
            .collect();
        b.push(&recs)?;
        if let Some(p) = progress {
            p.set_rows(b.names.len());
            p.check()?;
        }
        buf.drain(..cut);
        if eof {
            break;
        }
    }
    b.finish()
}

enum Line<'a> {
    Seq(&'a [u8], &'a [u8]),
    /// Blank line or Clustal consensus line: the end of a block.
    Break,
    /// Stockholm `#=GC <tag> <line>`: one block's part of a column annotation (RF among them).
    Gc(&'a [u8], &'a [u8]),
    /// Stockholm `#=GR <name> <tag> <line>`: one block's part of a residue annotation.
    Gr(&'a [u8], &'a [u8], &'a [u8]),
    /// Stockholm `#=GS <name> AC <accession>`.
    Accession(&'a [u8], &'a [u8]),
    /// Stockholm `#=GF NH`: part of a Newick tree.
    Tree(&'a [u8]),
}

/// Call `f` with the 1-based line number for every sequence line and block break of a Clustal or
/// Stockholm file, skipping blank lines before the header, the header itself and markup.
fn lines(mut r: impl BufRead, format: Format, mut f: impl FnMut(usize, Line) -> Result<()>) -> Result<()> {
    let mut buf = Vec::new();
    let (mut ln, mut header) = (0usize, true);
    loop {
        buf.clear();
        if r.read_until(b'\n', &mut buf)? == 0 {
            return Ok(());
        }
        ln += 1;
        let mut line = if ln == 1 { strip_bom(&buf) } else { &buf[..] };
        line = line.strip_suffix(b"\n").unwrap_or(line);
        line = line.strip_suffix(b"\r").unwrap_or(line);
        let blank = line.trim_ascii().is_empty();
        if header {
            header = blank;
            continue;
        }
        match format {
            _ if blank => f(ln, Line::Break)?,
            Format::Stockholm if line.starts_with(b"//") => return Ok(()),
            Format::Stockholm if line.starts_with(b"#=GC") => {
                let mut parts = line.split(|b| b.is_ascii_whitespace()).filter(|p| !p.is_empty());
                if let (Some(_), Some(tag), Some(seg)) = (parts.next(), parts.next(), parts.next()) {
                    f(ln, Line::Gc(tag, seg))?;
                }
            }
            Format::Stockholm if line.starts_with(b"#=GR") => {
                let mut parts = line.split(|b| b.is_ascii_whitespace()).filter(|p| !p.is_empty());
                if let (Some(_), Some(name), Some(tag), Some(seg)) = (parts.next(), parts.next(), parts.next(), parts.next()) {
                    f(ln, Line::Gr(name, tag, seg))?;
                }
            }
            Format::Stockholm if line.starts_with(b"#=GS") => {
                let mut parts = line.split(|b| b.is_ascii_whitespace()).filter(|p| !p.is_empty());
                if let (Some(_), Some(name), Some(b"AC"), Some(acc)) = (parts.next(), parts.next(), parts.next(), parts.next()) {
                    f(ln, Line::Accession(name, acc))?;
                }
            }
            Format::Stockholm if line.starts_with(b"#=GF") => {
                let rest = line[4..].trim_ascii_start();
                if let Some(nh) = rest.strip_prefix(b"NH") {
                    f(ln, Line::Tree(nh.trim_ascii()))?;
                }
            }
            Format::Stockholm if line.starts_with(b"#") => {}
            // Consensus lines in Clustal start with whitespace.
            Format::Clustal if line[0].is_ascii_whitespace() => f(ln, Line::Break)?,
            _ => {
                let mut parts = line.split(|b| b.is_ascii_whitespace()).filter(|p| !p.is_empty());
                let (Some(name), Some(seq)) = (parts.next(), parts.next()) else {
                    bail!("line {ln}: expected 'name sequence'");
                };
                f(ln, Line::Seq(name, seq))?;
            }
        }
    }
}

/// Clustal and Stockholm: blocks of `name sequence` lines, the same names repeated in each block.
/// Read twice so nothing but the packed matrix is held: the first pass (from `first`) finds the
/// names, lengths and residue type, the second (from `open`) encodes each line straight into its row.
fn interleaved<'a>(first: Reader<'a>, open: &dyn Fn() -> Result<Reader<'a>>, format: Format, ty: Option<SeqType>, progress: Option<&Progress>) -> Result<Alignment> {
    let mut index: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut names: Vec<Vec<u8>> = Vec::new();
    let mut lens: Vec<usize> = Vec::new();
    // Block number (from 1) in which each row was last seen.
    let mut seen: Vec<usize> = Vec::new();
    let (mut block, mut in_block) = (1usize, false);
    let mut census = Census::default();
    let mut marks: Vec<u64> = Vec::new();
    let mut rf: Vec<u8> = Vec::new();
    let mut acc: HashMap<Vec<u8>, Box<str>> = HashMap::new();
    let mut nh: Vec<u8> = Vec::new();
    let mut notes = crate::annot::Collector::default();
    lines(first, format, |ln, l| {
        match l {
            Line::Tree(t) => nh.extend_from_slice(t),
            Line::Gc(tag, seg) => {
                if tag == b"RF" {
                    rf.extend(seg.iter().filter(|b| !b.is_ascii_whitespace()));
                }
                notes.gc(tag, seg);
            }
            Line::Gr(name, tag, seg) => notes.gr(name, tag, seg),
            Line::Accession(name, a) => {
                acc.insert(name.to_vec(), lossy(a).into_boxed_str());
            }
            Line::Break => {
                block += in_block as usize;
                in_block = false;
            }
            Line::Seq(name, seq) => {
                in_block = true;
                let i = match index.get(name) {
                    Some(&i) => i,
                    None => {
                        index.insert(name.to_vec(), names.len());
                        names.push(name.to_vec());
                        lens.push(0);
                        seen.push(0);
                        if let Some(p) = progress {
                            p.set_rows(names.len());
                        }
                        names.len() - 1
                    }
                };
                if seen[i] == block {
                    bail!(
                        "line {ln}: sequence '{}' appears twice in the same block (blocks must be separated by a blank line)",
                        lossy(name)
                    );
                }
                seen[i] = block;
                match_marks(seq, lens[i], &mut marks);
                lens[i] += seq.iter().filter(|b| !b.is_ascii_whitespace() && !b.is_ascii_digit()).count();
                census.add(seq, usize::MAX);
            }
        }
        Ok(())
    })?;
    let Some(&cols) = lens.first() else { bail!("no sequences found") };
    if cols == 0 {
        bail!("sequence '{}' is empty", lossy(&names[0]));
    }
    if let Some(i) = lens.iter().position(|&n| n != cols) {
        bail!(not_aligned(&lossy(&names[i]), lens[i], cols));
    }
    let alpha = census.alphabet_as(ty);
    let (gc, mut gr_fill, warnings) = notes.finish(cols, |n| index.get(n).copied(), crate::annot::GR_BUDGET);
    let mut m = Matrix::empty(cols, alpha.bits);
    let mut rows = m.grow(names.len());
    let mut off = vec![0usize; names.len()];
    lines(open()?, format, |_, l| {
        match l {
            Line::Seq(name, seq) => {
                let i = *index.get(name).context("input changed while reading")?;
                off[i] += encode_at(alpha, seq, rows[i], off[i]).map_err(|b| match ty {
                    Some(t) => forced_char(b, &lossy(name), t),
                    None => bad_char(b, &lossy(name)),
                })?;
            }
            Line::Gr(name, tag, seg) => gr_fill.add(name, tag, seg),
            _ => {}
        }
        Ok(())
    })?;
    if off.iter().any(|&n| n != cols) {
        bail!("input changed while reading");
    }
    drop(rows);
    // RF wins over the marks when it covers every column.
    let inserts = if rf.len() == cols { inserts_from_rf(&rf) } else { inserts_from(&marks, cols) };
    let accessions = if acc.is_empty() { Vec::new() } else { names.iter().map(|n| acc.get(n).cloned()).collect() };
    let newick = (!nh.is_empty()).then(|| lossy(&nh));
    let gr = gr_fill.finish(cols);
    Ok(Alignment { names: names.iter().map(|n| lossy(n)).collect(), matrix: m, alpha, inserts, accessions, newick, gc, gr, warnings })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::alphabet::{DNA, NT_MIXED, PROTEIN, RNA};

    fn seqs(a: &Alignment) -> Vec<String> {
        (0..a.rows()).map(|r| (0..a.cols()).map(|c| a.symbol(a.matrix.get(r, c)) as char).collect()).collect()
    }

    /// Parse whole, and FASTA also in tiny batches so the type is first chosen from one row and
    /// then widened (with the input re-read through `read` when needed).
    fn all_ways(text: &[u8]) -> Vec<Alignment> {
        let mut v = vec![parse_bytes(text).unwrap()];
        if detect_format(text).unwrap() == Format::Fasta {
            for chunk in [1, 3, 8, 13] {
                let mut forced = None;
                let a = loop {
                    match fasta(text, chunk, forced, None, None) {
                        Err(e) if e.downcast_ref::<Restart>().is_some() => forced = Some(e.downcast_ref::<Restart>().unwrap().0),
                        r => break r.unwrap(),
                    }
                };
                v.push(a);
            }
        }
        v
    }

    #[test]
    fn fasta_wrapped_and_crlf() {
        let a = parse_bytes(b"\n>s1 desc\r\nACG\r\nT-\r\n>s2\nAC\nGTT\n").unwrap();
        assert_eq!(a.names, ["s1 desc", "s2"]);
        assert_eq!(seqs(&a), ["ACGT-", "ACGTT"]);
        assert_eq!(a.alpha, &DNA);
        assert_eq!(a.matrix.bits, 4);
    }

    #[test]
    fn small_batches_match_one_batch() {
        let text = b">a\nMKV-L\n>b\nMKVAL\n>c\nMK\nVAL\n";
        let p = parse_bytes(text).unwrap();
        for s in all_ways(text) {
            assert_eq!(seqs(&p), seqs(&s));
            assert_eq!(p.names, s.names);
        }
        assert_eq!(p.kind(), Kind::Protein);
        assert_eq!(p.matrix.bits, 8);
    }

    #[test]
    fn switches_to_protein_late() {
        for a in all_ways(b">a\nACGTA\n>b\nACGTE\n") {
            assert_eq!(a.kind(), Kind::Protein);
            assert_eq!(seqs(&a), ["ACGTA", "ACGTE"]);
        }
    }

    #[test]
    fn type_from_sample_across_rows() {
        // A gappy first row with ambiguity codes no longer makes the whole alignment protein.
        let mut text = b">a\n--ACRYKMSW-ACGT---\n".to_vec();
        for _ in 0..4 {
            text.extend_from_slice(b">b\nAAACGTACGTACGTACGT\n");
        }
        let a = parse_bytes(&text).unwrap();
        assert_eq!((a.alpha, a.matrix.bits), (&DNA, 4));
        assert_eq!(seqs(&a)[0], "--ACRYKMSW-ACGT---");
        // Rows after the sample still widen to protein.
        let mut late = b">a\nACGTACGT\n".repeat(SAMPLE / 8 + 1);
        late.extend_from_slice(b">z\nMKVLEACG\n");
        assert_eq!(parse_bytes(&late).unwrap().kind(), Kind::Protein);
    }

    #[test]
    fn switch_keeps_residues() {
        // `?` packed as N must become X, not asparagine; U must stay U (selenocysteine), not T.
        for a in all_ways(b">a\nACGTACGTAC?A\n>b\nMKVLEMKVLEMK\n") {
            assert_eq!(seqs(&a), ["ACGTACGTACXA", "MKVLEMKVLEMK"]);
        }
        for a in all_ways(b">a\nACGUACGU\n>b\nMKVLEMKV\n") {
            assert_eq!(seqs(&a), ["ACGUACGU", "MKVLEMKV"]);
        }
    }

    /// Reviewer repros: realistic proteins with no protein-only letter came out as DNA.
    #[test]
    fn low_complexity_proteins_are_protein() {
        let cases: [&[u8]; 3] = [
            b">fib1\nGAGAGSGAGAGSGAGAGYGAGAGS\n>fib2\nGAGAGSGAGAGSGAGAGSGAGAGS\n>fib3\nGAGAGYGAGAGSGAGAGSGAGAGT\n",
            b">grp1\nMGSGGGRGGGSGGRGYGGGGRGGG\n>grp2\nMGSGGGRGGGNGGRGYGGGGRGGG\n",
            b">p1\nGCKGKGAKCSRTMYDCCTGSCRSGKC\n>p2\nGCKGKGAKCSRTMYDCCTGSCRSGKC\n",
        ];
        for text in cases {
            for a in all_ways(text) {
                assert_eq!(a.alpha, &PROTEIN, "{}", String::from_utf8_lossy(text));
            }
        }
        // (AAT)n is valid DNA too; the override makes it protein
        let afgp = b">afgp1\nAATAATAATAATAATAATAATAAT\n>afgp2\nAATAATAATAATAATAATAATAAT\n";
        assert_eq!(parse_bytes(afgp).unwrap().alpha, &DNA);
        let a = parse_bytes_as(afgp, Some(SeqType::Protein)).unwrap();
        assert_eq!((a.alpha, seqs(&a)[0].as_str()), (&PROTEIN, "AATAATAATAATAATAATAATAAT"));
    }

    #[test]
    fn type_override() {
        let sto = b"# STOCKHOLM 1.0\nx GAGAGS\ny GAGAGS\n//\n";
        assert_eq!(parse_bytes(sto).unwrap().alpha, &PROTEIN);
        assert_eq!(parse_bytes_as(sto, Some(SeqType::Dna)).unwrap().alpha, &DNA);
        assert_eq!(parse_bytes_as(b">a\nGAGAGS\n", Some(SeqType::Rna)).unwrap().alpha, &RNA);
        // a forced nucleotide type never widens to protein; it reports the residue instead
        let e = parse_bytes_as(b">a\nACGT\n>b\nACGE\n", Some(SeqType::Dna)).err().unwrap();
        assert!(format!("{e:#}").contains("--type"), "{e:#}");
        let e = parse_bytes_as(b"CLUSTAL\n\nx ACGE\n", Some(SeqType::Dna)).err().unwrap();
        assert!(format!("{e:#}").contains("--type"), "{e:#}");
        for chunk in [1, 8] {
            let e = fasta(&b">a\nACGT\n>b\nACGE\n"[..], chunk, None, Some(SeqType::Dna), None).err().unwrap();
            assert!(e.to_string().contains("--type"), "{e}");
        }
    }

    /// Hard-masked X and '*' no longer turn DNA into protein, and are stored as written.
    #[test]
    fn masked_nucleotides_stay_nucleotides() {
        for a in all_ways(b">a\nACGTXXXXACGT\n>b\nACGTACGTACGT\n>c\nACGTACGTACG*\n") {
            assert_eq!((a.kind(), a.alpha.name), (Kind::Nucleotide, "DNA"));
            assert_eq!(seqs(&a), ["ACGTXXXXACGT", "ACGTACGTACGT", "ACGTACGTACG*"]);
        }
        for a in all_ways(b">a\nACGUXXACGU\n>b\nACGUACGUAC\n") {
            assert_eq!(a.alpha.name, "RNA");
            assert_eq!(seqs(&a), ["ACGUXXACGU", "ACGUACGUAC"]);
        }
        let a = parse_bytes(b"# STOCKHOLM 1.0\nx ACGTXXACGT\ny ACGTACGTAC\n//\n").unwrap();
        assert_eq!((a.kind(), seqs(&a)[0].as_str()), (Kind::Nucleotide, "ACGTXXACGT"));
        // a protein with a stop stays protein
        for a in all_ways(b">a\nMKVLE*\n>b\nMKVLA*\n") {
            assert_eq!(a.alpha, &PROTEIN);
        }
    }

    #[test]
    fn stockholm_without_blank_line_between_blocks() {
        let e = parse_bytes(b"# STOCKHOLM 1.0\na ACGU\nb ACGU\na AC\nb A-\n//\n").err().unwrap();
        assert!(format!("{e:#}").contains("blank line"), "{e:#}");
    }

    #[test]
    fn mixed_t_and_u_kept_apart() {
        for a in all_ways(b">a\nACGU\n>b\nACGT\n>c\nAC-?\n") {
            assert_eq!(a.alpha, &NT_MIXED);
            assert_eq!(a.matrix.bits, 8);
            assert_eq!(seqs(&a), ["ACGU", "ACGT", "AC-N"]);
        }
        let a = parse_bytes(b"# STOCKHOLM 1.0\nx ACGU\ny ACGT\n//\n").unwrap();
        assert_eq!(seqs(&a), ["ACGU", "ACGT"]);
    }

    #[test]
    fn rejects_unaligned_and_garbage() {
        let e = parse_bytes(b">a\nACGT\n>b\nACG\n").err().unwrap().to_string();
        assert!(e.contains("not an alignment"), "{e}");
        assert!(fasta(&b">a\nACGT\n>b\nACG\n"[..], 4, None, None, None).is_err());
        assert!(parse_bytes(b">a\n\n>b\nAC\n").is_err());
        assert!(parse_bytes(b">a\nAC%T\n").is_err());
        assert!(fasta(&b">a\nACGT\n>b\nAC%T\n"[..], 4, None, None, None).is_err());
        assert!(parse_bytes(b"hello").is_err());
    }

    #[test]
    fn rna() {
        let a = parse_bytes(b">a\nACGU-U\n").unwrap();
        assert_eq!((a.alpha, a.matrix.bits), (&RNA, 4));
        assert_eq!(seqs(&a), ["ACGU-U"]);
    }

    #[test]
    fn clustal() {
        let text = b"CLUSTAL W (1.83) multiple sequence alignment\n\n\
            seq1      MKV-LA 5\nseq2      MKVALA 6\n          *** **\n\n\
            seq1      GG\nseq2      G-\n";
        let a = parse_bytes(text).unwrap();
        assert_eq!(a.names, ["seq1", "seq2"]);
        assert_eq!(seqs(&a), ["MKV-LAGG", "MKVALAG-"]);
        assert_eq!(a.alpha, &PROTEIN);
    }

    #[test]
    fn clustal_blank_first_line_and_bom() {
        let body = b"CLUSTAL W\n\nseq1 AC-T\nseq2 ACGT\n\nseq1 GG\nseq2 G-\n";
        for prefix in [&b"\n\n"[..], b"\r\n", b"\xEF\xBB\xBF", b"\xEF\xBB\xBF\n"] {
            let text = [prefix, &body[..]].concat();
            let a = parse_bytes(&text).unwrap_or_else(|e| panic!("{prefix:?}: {e}"));
            assert_eq!(seqs(&a), ["AC-TGG", "ACGTG-"]);
            assert_eq!(a.alpha, &DNA);
        }
    }

    #[test]
    fn bom_fasta_and_stockholm() {
        let a = parse_bytes(b"\xEF\xBB\xBF>a\nACGT\n").unwrap();
        assert_eq!((a.names[0].as_str(), seqs(&a)[0].as_str()), ("a", "ACGT"));
        let a = parse_bytes(b"\xEF\xBB\xBF# STOCKHOLM 1.0\nx AC\n//\n").unwrap();
        assert_eq!(seqs(&a), ["AC"]);
    }

    #[test]
    fn duplicate_name_in_block() {
        for (text, ln) in [(&b"# STOCKHOLM 1.0\nx AC\nx AG\ny AC\n//\n"[..], 3), (b"CLUSTAL\n\nx AC\nx AG\ny AC\n", 4)] {
            let e = parse_bytes(text).err().unwrap().to_string();
            assert!(e.contains("'x' appears twice") && e.contains(&format!("line {ln}:")), "{e}");
        }
        // A row missing from a block is still reported by name.
        let e = parse_bytes(b"CLUSTAL\n\nx AC\ny AC\n\nx GG\n").err().unwrap().to_string();
        assert!(e.contains("'y' has 2 columns"), "{e}");
    }

    #[test]
    fn stockholm() {
        let text = b"# STOCKHOLM 1.0\n#=GF ID test\nx/1-4 AC.G\ny/1-4 ACUG\n#=GC SS_cons ....\n\nx/1-4 U\ny/1-4 -\n//\n";
        let a = parse_bytes(text).unwrap();
        assert_eq!(a.names, ["x/1-4", "y/1-4"]);
        assert_eq!(seqs(&a), ["AC-GU", "ACUG-"]);
        assert_eq!(a.alpha, &RNA);
        // Odd block widths land on the right nibble.
        let a = parse_bytes(b"# STOCKHOLM 1.0\nx A\ny C\n\nx CGT\ny GTA\n\nx A-\ny -A\n//\n").unwrap();
        assert_eq!(seqs(&a), ["ACGTA-", "CGTA-A"]);
    }

    fn insert_cols(a: &Alignment) -> Vec<usize> {
        (0..a.cols()).filter(|&c| a.is_insert(c)).collect()
    }

    /// Pfam/HMMER Stockholm: match columns are upper case with `-` gaps, insert columns lower case
    /// with `.` gaps. Lower case is stored as upper case and `.` as a gap; the columns are kept.
    #[test]
    fn stockholm_inserts_from_case() {
        let text = b"# STOCKHOLM 1.0\n#=GS x/1-6 AC P12345.1\nx/1-6 AC.gT\ny/3-7 A-tGT\n\nx/1-6 .a\ny/3-7 ..\n//\n";
        let a = parse_bytes(text).unwrap();
        assert_eq!(seqs(&a), ["AC-GT-A", "A-TGT--"]);
        // column 2 has '.' and 't'; column 5 '.' and '.'; column 6 'a' and '.'
        assert_eq!(insert_cols(&a), [2, 5, 6]);
        assert_eq!(a.insert_count(), 3);
        assert_eq!(a.accession(0), Some("P12345.1"));
        assert_eq!(a.accession(1), None);
        // Pfam seed style: '.' gaps, upper case residues, no inserts
        let seed = b"# STOCKHOLM 1.0\nx ACD.E\ny AC..E\n//\n";
        let a = parse_bytes(seed).unwrap();
        assert_eq!(insert_cols(&a), [3]); // only the all-'.' column
        assert!(a.accessions.is_empty());
    }

    #[test]
    fn stockholm_rf_wins() {
        // RF says columns 0 and 3 are inserts, although the residues are upper case there.
        let text = b"# STOCKHOLM 1.0\nx ACGT\ny AC.t\n#=GC RF .xx.\n\nx A\ny a\n#=GC RF x\n//\n";
        let a = parse_bytes(text).unwrap();
        assert_eq!(insert_cols(&a), [0, 3]);
        // an RF line that doesn't cover the alignment is ignored
        let text = b"# STOCKHOLM 1.0\nx ACGt\ny AC.t\n#=GC RF xx\n//\n";
        let a = parse_bytes(text).unwrap();
        assert_eq!(insert_cols(&a), [3]);
        assert!(a.gc.is_empty());
        assert_eq!(a.warnings, ["#=GC RF has 2 columns, the alignment 4; left out"]);
    }

    /// `#=GC` and `#=GR` lines: every tag is kept, joined across blocks in file order; lines that
    /// don't cover the alignment (or name no sequence) are left out with a warning.
    #[test]
    fn stockholm_annotation() {
        let text = b"# STOCKHOLM 1.0\n\
            x/1-5 ACD\n#=GR x/1-5 SS HHE\n#=GR x/1-5 PP 9*8\ny ACE\n\
            #=GC SS_cons HHE\n#=GC seq_cons ACd\n#=GC PP_cons 999\n\n\
            x/1-5 EF\n#=GR x/1-5 SS E.\n#=GR x/1-5 PP 7.\ny E-\n#=GR y PP 12\n\
            #=GC SS_cons EC\n#=GC seq_cons EF\n#=GC PP_cons 9\n#=GR nobody SS HH\n//\n";
        let a = parse_bytes(text).unwrap();
        let tags: Vec<&str> = a.gc.iter().map(|t| &*t.tag).collect();
        assert_eq!(tags, ["SS_cons", "seq_cons"]);
        assert_eq!(&*a.gc[0].data, b"HHEEC");
        assert_eq!(&*a.gc[1].data, b"ACdEF");
        // y's PP line covers 2 of 5 columns: left out
        assert_eq!(a.gr.len(), 2);
        let x: Vec<(&str, &[u8])> = crate::annot::row_annots(&a.gr, 0).iter().map(|g| (&*g.tag, &*g.data)).collect();
        assert_eq!(x, [("SS", &b"HHEE."[..]), ("PP", &b"9*87."[..])]);
        assert!(crate::annot::row_annots(&a.gr, 1).is_empty());
        assert!(a.warnings.iter().any(|w| w == "#=GR y PP has 2 columns, the alignment 5; left out"));
        assert_eq!(a.warnings.len(), 3, "{:?}", a.warnings);
        assert!(a.warnings[0].starts_with("#=GC PP_cons has 4 columns"));
        assert!(a.warnings.iter().any(|w| w.starts_with("#=GR nobody SS: no sequence")));
        // FASTA has none
        let f = parse_bytes(b">a\nAC\n").unwrap();
        assert!(f.gc.is_empty() && f.gr.is_empty() && f.warnings.is_empty());
    }

    /// A2M-style FASTA, in one batch and in tiny batches.
    #[test]
    fn a2m_fasta_inserts() {
        let text = b">a\nAC.gT\n>b\nA-tG.\n>c\nACaGT\n";
        for a in all_ways(text) {
            assert_eq!(seqs(&a), ["AC-GT", "A-TG-", "ACAGT"]);
            assert_eq!(insert_cols(&a), [2]);
        }
        // a row with no insert marks makes every column a match column
        for a in all_ways(b">a\nAC.gT\n>b\nACGGT\n") {
            assert!(a.inserts.is_empty());
        }
        // plain upper-case FASTA: none
        assert!(parse_bytes(b">a\nACGT\n>b\nAC-T\n").unwrap().inserts.is_empty());
    }

    #[test]
    fn gzip_and_file() {
        use std::io::Write;
        let dir = std::env::temp_dir().join(format!("msascope-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = Vec::new();
        for (name, text) in [("a.fa", &b">a\nAC-T\n>b\nACGT\n"[..]), ("a.sto", b"# STOCKHOLM 1.0\na AC\nb AC\n\na -T\nb GT\n//\n")] {
            let plain = dir.join(name);
            std::fs::write(&plain, text).unwrap();
            let gz = dir.join(format!("{name}.gz"));
            let mut e = flate2::write::GzEncoder::new(File::create(&gz).unwrap(), flate2::Compression::fast());
            e.write_all(text).unwrap();
            e.finish().unwrap();
            let (p, z) = (load(&plain).unwrap(), load(&gz).unwrap());
            assert_eq!(seqs(&p), seqs(&z));
            a.push(seqs(&p));
        }
        assert_eq!(a[0], a[1]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// A FASTA of `rows` rows × `cols` columns, and the same as Stockholm.
    fn big(rows: usize, cols: usize) -> (Vec<u8>, Vec<u8>) {
        let (mut fa, mut sto) = (Vec::new(), b"# STOCKHOLM 1.0\n".to_vec());
        for r in 0..rows {
            let seq: String = (0..cols).map(|c| b"ACGT-"[(r * 7 + c * 3) % 5] as char).collect();
            fa.extend_from_slice(format!(">s{r}\n{seq}\n").as_bytes());
            sto.extend_from_slice(format!("s{r} {seq}\n").as_bytes());
        }
        sto.extend_from_slice(b"//\n");
        (fa, sto)
    }

    /// Reads `inner`, and cancels `p` once more than `after` bytes have gone through.
    struct CancelAfter<'a> {
        inner: &'a [u8],
        p: &'a Progress,
        after: usize,
        done: usize,
    }

    impl Read for CancelAfter<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.done += n;
            if self.done > self.after {
                self.p.cancel();
            }
            Ok(n)
        }
    }

    #[test]
    fn progress_counts_bytes_and_rows() {
        let (fa, sto) = big(2000, 300);
        for (data, passes) in [(&fa, 1), (&sto, 2)] {
            let p = Progress::new(data.len() as u64);
            let a = parse_bytes_with(data, &p).unwrap();
            let s = p.snapshot();
            assert_eq!((a.rows(), s.rows, s.bytes, s.pass), (2000, 2000, data.len() as u64, passes));
            assert_eq!(s.fraction(), 1.0);
        }
    }

    #[test]
    fn progress_is_monotone() {
        let (fa, _) = big(20_000, 400);
        let p = Progress::new(fa.len() as u64);
        let seen = std::thread::scope(|sc| {
            let h = sc.spawn(|| parse_bytes_with(&fa, &p).map(|a| a.rows()));
            let mut seen = Vec::new();
            while !h.is_finished() {
                seen.push(p.snapshot());
                std::thread::yield_now();
            }
            assert_eq!(h.join().unwrap().unwrap(), 20_000);
            seen
        });
        for w in seen.windows(2) {
            assert!(w[1].bytes >= w[0].bytes && w[1].rows >= w[0].rows, "{w:?}");
        }
    }

    #[test]
    fn cancel_stops_within_one_read() {
        let (fa, sto) = big(4000, 500);
        for data in [&fa, &sto] {
            let p = Progress::new(data.len() as u64);
            let open = || -> Result<Reader> {
                p.start_pass();
                let r = CancelAfter { inner: data, p: &p, after: 200_000, done: 0 };
                Ok(Box::new(BufReader::with_capacity(1 << 16, Counting { inner: r, progress: &p })))
            };
            let e = read(&open, None, Some(&p)).err().expect("cancelled");
            assert!(is_cancelled(&e), "{e:#}");
            // Stopped at the first read after the flag: at most one read past it.
            let s = p.snapshot();
            assert!(s.bytes <= (200_000 + READ_MAX) as u64 && s.bytes < data.len() as u64 && s.pass == 1, "{s:?}");
        }
        // Cancelled before starting.
        let p = Progress::new(0);
        p.cancel();
        assert!(is_cancelled(&parse_bytes_with(&fa, &p).err().unwrap()));
        assert!(!is_cancelled(&anyhow::anyhow!("other")));
    }
}
