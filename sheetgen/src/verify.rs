//! `sheetgen verify`: check read files before an analysis starts.
//!
//! Each file is read once. One thread reads it in 1 MiB blocks and hands every block to three
//! others: MD5, SHA-256, and a checker that decompresses gzip (every member, CRC and length
//! included) and walks the FASTQ records. Memory use doesn't depend on file or read size.
//! Expected checksums come from `md5sum`-style files or from ENA's file report (opt-in network).

use anyhow::{Context, Result, bail};
use clap::Args;
use flate2::{Decompress, FlushDecompress, Status};
use md5::{Digest, Md5};
use regex::Regex;
use serde_json::{Value, json};
use sha2::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

#[derive(Args)]
pub struct VerifyArgs {
    /// Read files, or folders to scan (recursively) for FASTQ files.
    paths: Vec<PathBuf>,

    /// Also check the FASTQ files named in this sample sheet (CSV or TSV). Repeatable.
    #[arg(long = "sheet", value_name = "FILE")]
    sheets: Vec<PathBuf>,

    /// Expected MD5s in `md5sum` format (`<md5>  <file>`). Repeatable.
    #[arg(long = "md5", value_name = "FILE")]
    md5: Vec<PathBuf>,

    /// Look up expected MD5s and sizes at ENA for the run accession (ERR/SRR/DRR) in each file
    /// name. Uses the network; nothing is fetched without this flag.
    #[arg(long)]
    ena: bool,

    /// Run accession for a file whose name has none, e.g. `--run AHB29_ont.fastq.gz=ERR15424439`.
    /// Repeatable.
    #[arg(long = "run", value_name = "FILE=ACCESSION", requires = "ena")]
    runs: Vec<String>,

    /// Print the report as JSON on stdout.
    #[arg(long)]
    json: bool,

    /// Files to check at the same time (each uses about four threads).
    #[arg(short, long, default_value_t = 1)]
    jobs: usize,
}

const BLOCK: usize = 1 << 20;
const DEPTH: usize = 4;

type Block = Arc<Vec<u8>>;

/// The first thing wrong with a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub what: String,
    /// Offset in the file as stored (compressed bytes for gzip) where the problem was found.
    pub byte_offset: Option<u64>,
    /// 1-based FASTQ record being read when the problem was found.
    pub record: Option<u64>,
    /// 1-based line of the decompressed FASTQ text.
    pub line: Option<u64>,
}

impl Problem {
    fn describe(&self) -> String {
        let mut at = Vec::new();
        if let Some(b) = self.byte_offset {
            at.push(format!("byte {b}"));
        }
        if let Some(r) = self.record {
            at.push(format!("record {r}"));
        }
        if let Some(l) = self.line {
            at.push(format!("line {l}"));
        }
        if at.is_empty() { self.what.clone() } else { format!("{} (at {})", self.what, at.join(", ")) }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Gzip,
    Plain,
    /// `.bz2`/`.zst`: hashed, structure not checked.
    Unchecked,
}

impl Kind {
    fn of(path: &Path) -> Kind {
        let name = path.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if name.ends_with(".gz") {
            Kind::Gzip
        } else if name.ends_with(".bz2") || name.ends_with(".zst") {
            Kind::Unchecked
        } else {
            Kind::Plain
        }
    }

    fn name(self) -> &'static str {
        match self {
            Kind::Gzip => "gzip",
            Kind::Plain => "plain",
            Kind::Unchecked => "not checked",
        }
    }
}

/// Streaming FASTQ structure check: four-line records, `@` header, `+` separator, sequence and
/// quality of equal length. Keeps only counters, so memory doesn't grow with read length.
#[derive(Default)]
pub struct FastqCheck {
    /// Line of the current record, 0..4.
    part: u8,
    /// Bytes of the current line seen so far, and its first and last byte.
    len: u64,
    first: u8,
    last: u8,
    seq_len: u64,
    /// Completed lines, and blank lines held back (allowed only at the end of the file).
    lines: u64,
    blank: u64,
    blank_start: u64,
    /// Decompressed bytes fed so far, and the offset where the current line starts.
    pub fed: u64,
    line_start: u64,
    pub reads: u64,
    pub bases: u64,
}

impl FastqCheck {
    pub fn feed(&mut self, mut data: &[u8]) -> Result<(), Problem> {
        let mut at = self.fed;
        self.fed += data.len() as u64;
        while !data.is_empty() {
            match memchr::memchr(b'\n', data) {
                Some(i) => {
                    self.extend(&data[..i]);
                    self.end_line()?;
                    data = &data[i + 1..];
                    at += i as u64 + 1;
                    self.line_start = at;
                }
                None => {
                    self.extend(data);
                    break;
                }
            }
        }
        Ok(())
    }

    fn extend(&mut self, part: &[u8]) {
        let (Some(&first), Some(&last)) = (part.first(), part.last()) else { return };
        if self.len == 0 {
            self.first = first;
        }
        self.len += part.len() as u64;
        self.last = last;
    }

    fn problem(&self, what: String) -> Problem {
        // Plain files: the bad line's offset. Gzip callers replace it with the compressed offset.
        Problem { what, byte_offset: Some(self.line_start), record: Some(self.reads + 1), line: Some(self.lines + self.blank + 1) }
    }

    fn end_line(&mut self) -> Result<(), Problem> {
        let len = if self.len > 0 && self.last == b'\r' { self.len - 1 } else { self.len };
        if self.part == 0 && len == 0 {
            if self.blank == 0 {
                self.blank_start = self.line_start;
            }
            self.blank += 1;
            self.len = 0;
            return Ok(());
        }
        if self.blank > 0 {
            self.lines += self.blank - 1;
            self.blank = 0;
            self.line_start = self.blank_start;
            return Err(self.problem("blank line inside the file".into()));
        }
        match self.part {
            0 if self.first != b'@' => {
                let what = if self.reads == 0 && self.first == 0x1f {
                    "gzip data in a file not named .gz".to_string()
                } else {
                    format!("record header doesn't start with @ (starts with {:?})", self.first as char)
                };
                return Err(self.problem(what));
            }
            1 => self.seq_len = len,
            2 if self.first != b'+' => {
                return Err(self.problem(format!(
                    "third line of a record doesn't start with + (starts with {:?}); multi-line FASTQ is not supported",
                    self.first as char
                )));
            }
            3 if len != self.seq_len => {
                return Err(self.problem(format!("sequence is {} bases but quality is {} characters", self.seq_len, len)));
            }
            3 => {
                self.reads += 1;
                self.bases += self.seq_len;
            }
            _ => {}
        }
        self.part = (self.part + 1) % 4;
        self.lines += 1;
        self.len = 0;
        Ok(())
    }

    /// End of the data: a last line without a newline counts; a partial record is an error.
    pub fn finish(&mut self) -> Result<(), Problem> {
        if self.len > 0 {
            self.end_line()?;
        }
        if self.part != 0 {
            return Err(self.problem(format!("file ends inside a record (after line {} of 4)", self.part)));
        }
        Ok(())
    }
}

/// What the checker thread found.
#[derive(Debug, Default)]
pub struct Decoded {
    pub members: u64,
    pub reads: u64,
    pub bases: u64,
    pub uncompressed: u64,
    pub problem: Option<Problem>,
}

/// Inflate every gzip member (zlib checks each member's header, CRC-32 and length) and pass the
/// text to the FASTQ check. Stops at the first gzip problem but keeps draining `blocks`. After a
/// FASTQ problem it keeps inflating: damaged gzip data often decodes to garbage that breaks the
/// records before the member's CRC is reached, and then the gzip damage is the real cause.
pub fn check_gzip(blocks: impl IntoIterator<Item = Block>) -> Decoded {
    let mut out = vec![0u8; BLOCK];
    let mut fq = FastqCheck::default();
    let mut fq_problem: Option<Problem> = None;
    let mut d: Option<Decompress> = None;
    let mut member_start = 0u64;
    let mut members = 0u64;
    let mut uncompressed = 0u64;
    let mut problem = None;
    'blocks: for block in blocks {
        if problem.is_some() {
            continue;
        }
        let mut input = &block[..];
        loop {
            let dec = d.get_or_insert_with(|| Decompress::new_gzip(15));
            let (in0, out0) = (dec.total_in(), dec.total_out());
            let status = dec.decompress(input, &mut out, FlushDecompress::None);
            let used = (dec.total_in() - in0) as usize;
            let made = (dec.total_out() - out0) as usize;
            input = &input[used..];
            uncompressed += made as u64;
            if fq_problem.is_none()
                && let Err(mut p) = fq.feed(&out[..made])
            {
                p.byte_offset = Some(member_start + dec.total_in());
                fq_problem = Some(p);
            }
            let record = fq.reads + 1;
            match status {
                Ok(Status::StreamEnd) => {
                    members += 1;
                    member_start += dec.total_in();
                    d = None;
                }
                Ok(_) if used == 0 && made == 0 && !input.is_empty() => {
                    let at = Some(member_start + dec.total_in());
                    problem = Some(Problem { what: "gzip decoder made no progress".into(), byte_offset: at, record: Some(record), line: None });
                    continue 'blocks;
                }
                Ok(_) => {}
                Err(e) => {
                    let at = dec.total_in();
                    let msg = e.message().unwrap_or("invalid data").to_string();
                    let what = if members > 0 && at < 10 {
                        format!("data after gzip member {members} is not gzip ({msg})")
                    } else if members == 0 && at < 10 {
                        format!("not gzip data ({msg})")
                    } else if msg.contains("check") || msg.contains("length") {
                        format!("gzip member {} is damaged: {msg} (CRC-32/length are checked at the member's end)", members + 1)
                    } else {
                        format!("gzip member {} is damaged: {msg}", members + 1)
                    };
                    problem = Some(Problem { what, byte_offset: Some(member_start + at), record: Some(record), line: None });
                    continue 'blocks;
                }
            }
            // Done with this block once the input is used up and no output is pending.
            if input.is_empty() && made < out.len() {
                break;
            }
        }
    }
    if problem.is_none() {
        if let Some(dec) = &d {
            let at = Some(member_start + dec.total_in());
            let what = format!("file ends inside gzip member {} (truncated)", members + 1);
            problem = Some(Problem { what, byte_offset: at, record: Some(fq.reads + 1), line: None });
        } else if members == 0 {
            problem = Some(Problem { what: "empty file".into(), byte_offset: Some(0), record: None, line: None });
        }
    }
    let problem = match (problem, fq_problem) {
        // Gzip damage that had already garbled the reads: report the gzip damage, located where
        // it first showed (nearer the damage than where zlib noticed it).
        (Some(g), Some(f)) => Some(Problem {
            what: format!(
                "{} at byte {}; the decompressed reads already break at record {}: {}",
                g.what,
                g.byte_offset.unwrap_or(0),
                f.record.unwrap_or(0),
                f.what
            ),
            ..f
        }),
        (Some(g), None) => Some(g),
        (None, Some(f)) => Some(f),
        (None, None) => fq.finish().err().map(|mut p| {
            p.byte_offset = Some(member_start);
            p
        }),
    };
    Decoded { members, reads: fq.reads, bases: fq.bases, uncompressed, problem }
}

pub fn check_plain(blocks: impl IntoIterator<Item = Block>) -> Decoded {
    let mut fq = FastqCheck::default();
    let mut problem = None;
    for block in blocks {
        if problem.is_some() {
            continue;
        }
        if let Err(p) = fq.feed(&block) {
            problem = Some(p);
        }
    }
    if problem.is_none() {
        problem = fq.finish().err();
    }
    Decoded { members: 0, reads: fq.reads, bases: fq.bases, uncompressed: fq.fed, problem }
}

/// Hashes and structure of one file.
#[derive(Debug)]
pub struct Checked {
    pub bytes: u64,
    pub md5: String,
    pub sha256: String,
    pub kind_name: &'static str,
    pub decoded: Decoded,
    pub seconds: f64,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Read `path` once, hashing and checking it on separate threads.
pub fn check_file(path: &Path) -> Result<Checked> {
    let start = Instant::now();
    let mut file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let kind = Kind::of(path);
    let (md5_tx, md5_rx) = sync_channel::<Block>(DEPTH);
    let (sha_tx, sha_rx) = sync_channel::<Block>(DEPTH);
    let (dec_tx, dec_rx) = sync_channel::<Block>(DEPTH);
    std::thread::scope(|s| {
        let md5 = s.spawn(move || {
            let mut h = Md5::new();
            for b in md5_rx {
                h.update(&b[..]);
            }
            hex(&h.finalize())
        });
        let sha = s.spawn(move || {
            let mut h = Sha256::new();
            for b in sha_rx {
                h.update(&b[..]);
            }
            hex(&h.finalize())
        });
        let dec = s.spawn(move || match kind {
            Kind::Gzip => check_gzip(dec_rx),
            Kind::Plain => check_plain(dec_rx),
            Kind::Unchecked => {
                dec_rx.into_iter().for_each(drop);
                Decoded::default()
            }
        });
        let sent = send_blocks(&mut file, [&md5_tx, &sha_tx, &dec_tx]);
        drop((md5_tx, sha_tx, dec_tx));
        let (md5, sha256, mut decoded) = (md5.join().unwrap(), sha.join().unwrap(), dec.join().unwrap());
        let bytes = match sent {
            Ok(n) => n,
            Err((n, e)) => {
                decoded.problem = Some(Problem { what: format!("read error: {e}"), byte_offset: Some(n), record: None, line: None });
                n
            }
        };
        Ok(Checked { bytes, md5, sha256, kind_name: kind.name(), decoded, seconds: start.elapsed().as_secs_f64() })
    })
}

/// Read the file in blocks and send each to every receiver. Returns the bytes read.
fn send_blocks(file: &mut std::fs::File, txs: [&SyncSender<Block>; 3]) -> std::result::Result<u64, (u64, std::io::Error)> {
    let mut total = 0u64;
    loop {
        let mut buf = vec![0u8; BLOCK];
        let mut n = 0;
        while n < BLOCK {
            match file.read(&mut buf[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => return Err((total + n as u64, e)),
            }
        }
        if n == 0 {
            return Ok(total);
        }
        buf.truncate(n);
        total += n as u64;
        let block = Arc::new(buf);
        for tx in txs {
            // A receiver only goes away by panicking; the join reports that.
            let _ = tx.send(block.clone());
        }
        if n < BLOCK {
            return Ok(total);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Expected checksums

#[derive(Debug, Clone, PartialEq)]
pub struct Expected {
    pub md5: String,
    pub bytes: Option<u64>,
    /// Where the value came from, e.g. `MD5SUMS` or `ENA ERR15424439`.
    pub source: String,
}

/// One line of an `md5sum` file: GNU (`<md5>  name`, `<md5> *name`) or BSD (`MD5 (name) = <md5>`).
pub fn parse_md5_line(line: &str) -> Option<(String, String)> {
    static BSD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^MD5 ?\((.+)\) ?= ?([0-9A-Fa-f]{32})$").unwrap());
    let line = line.trim_end_matches(['\r', '\n']);
    if line.trim().is_empty() || line.starts_with('#') {
        return None;
    }
    if let Some(c) = BSD.captures(line) {
        return Some((c[2].to_ascii_lowercase(), c[1].to_string()));
    }
    let (hash, rest) = line.split_once(char::is_whitespace)?;
    let hash = hash.trim_start_matches('\\');
    if hash.len() != 32 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let name = rest.strip_prefix([' ', '*']).unwrap_or(rest).trim_start_matches(['*', ' ']);
    Some((hash.to_ascii_lowercase(), name.to_string()))
}

/// Entries of the `md5sum` files: by path (resolved against the file's folder) and by name.
#[derive(Default)]
struct Md5Lists {
    by_path: BTreeMap<PathBuf, Expected>,
    by_name: BTreeMap<String, Vec<Expected>>,
    /// Every path listed, to report the ones that weren't checked.
    listed: BTreeSet<PathBuf>,
}

impl Md5Lists {
    fn load(files: &[PathBuf]) -> Result<Md5Lists> {
        let mut lists = Md5Lists::default();
        for f in files {
            let text = std::fs::read_to_string(f).with_context(|| format!("reading {}", f.display()))?;
            let dir = f.parent().map(Path::to_path_buf).unwrap_or_default();
            let dir = dir.canonicalize().unwrap_or(dir);
            let mut n = 0;
            for line in text.lines() {
                let Some((md5, name)) = parse_md5_line(line) else { continue };
                n += 1;
                let e = Expected { md5, bytes: None, source: f.display().to_string() };
                let path = normalize(&dir.join(&name));
                let base = Path::new(&name).file_name().map(|b| b.to_string_lossy().into_owned()).unwrap_or(name);
                lists.by_name.entry(base).or_default().push(e.clone());
                lists.listed.insert(path.clone());
                lists.by_path.insert(path, e);
            }
            if n == 0 {
                bail!("{}: no `<md5>  <file>` lines found", f.display());
            }
        }
        Ok(lists)
    }

    fn find(&self, path: &Path) -> Option<Expected> {
        if let Some(e) = self.by_path.get(path) {
            return Some(e.clone());
        }
        let name = path.file_name()?.to_string_lossy();
        match self.by_name.get(name.as_ref())?.as_slice() {
            [one] => Some(one.clone()),
            // Same name listed several times with one MD5 (copies) is fine; otherwise ambiguous.
            many if many.iter().all(|e| e.md5 == many[0].md5) => Some(many[0].clone()),
            _ => None,
        }
    }
}

/// Remove `.` and `..` without touching the file system.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// One FASTQ file of a run, from ENA's file report.
#[derive(Debug, Clone, PartialEq)]
pub struct EnaFile {
    pub run: String,
    pub name: String,
    pub url: String,
    pub md5: String,
    pub bytes: Option<u64>,
}

/// ENA's file report for a run: its FASTQ files with MD5s and sizes (tab-separated, one line
/// per run, `;` between the files of a run).
pub fn ena_url(run: &str) -> String {
    format!(
        "https://www.ebi.ac.uk/ena/portal/api/filereport?accession={run}&result=read_run&fields=run_accession,fastq_ftp,fastq_md5,fastq_bytes&format=tsv"
    )
}

pub fn parse_filereport(tsv: &str) -> Result<Vec<EnaFile>> {
    let mut lines = tsv.lines().filter(|l| !l.trim().is_empty());
    let Some(header) = lines.next() else { bail!("empty ENA response") };
    let cols: Vec<&str> = header.split('\t').map(str::trim).collect();
    let col = |name: &str| cols.iter().position(|c| *c == name).with_context(|| format!("ENA response has no `{name}` column: {header}"));
    let (run_i, ftp_i, md5_i, bytes_i) = (col("run_accession")?, col("fastq_ftp")?, col("fastq_md5")?, col("fastq_bytes")?);
    let mut out = Vec::new();
    for line in lines {
        let cells: Vec<&str> = line.split('\t').collect();
        let cell = |i: usize| cells.get(i).map_or("", |c| c.trim());
        let urls: Vec<&str> = cell(ftp_i).split(';').filter(|s| !s.is_empty()).collect();
        let md5s: Vec<&str> = cell(md5_i).split(';').collect();
        let sizes: Vec<&str> = cell(bytes_i).split(';').collect();
        if urls.is_empty() {
            continue;
        }
        if md5s.len() != urls.len() {
            bail!("ENA response for {} lists {} files but {} MD5s", cell(run_i), urls.len(), md5s.len());
        }
        for (i, url) in urls.iter().enumerate() {
            let name = url.rsplit('/').next().unwrap_or(url).to_string();
            out.push(EnaFile {
                run: cell(run_i).to_string(),
                name,
                url: url.to_string(),
                md5: md5s[i].trim().to_ascii_lowercase(),
                bytes: sizes.get(i).and_then(|s| s.trim().parse().ok()),
            });
        }
    }
    Ok(out)
}

/// The run accession in a file name: `ERR044595_1.fastq.gz` → `ERR044595`.
pub fn run_in_name(name: &str) -> Option<String> {
    static RUN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[^A-Za-z0-9])([EDS]RR[0-9]{6,})(?:[^0-9]|$)").unwrap());
    RUN.captures(name).map(|c| c[1].to_string())
}

/// Which of a run's ENA files a local file is: same name, else the same mate (`_1`/`_R1` →
/// ENA's `_1`), else the run's only file.
pub fn match_ena<'a>(local_name: &str, run: &str, files: &'a [EnaFile]) -> Option<&'a EnaFile> {
    let files: Vec<&EnaFile> = files.iter().filter(|f| f.run == run).collect();
    if let Some(f) = files.iter().find(|f| f.name == local_name) {
        return Some(f);
    }
    let stem = |n: &str| crate::FASTQ_EXT.find(n).map_or(n.to_string(), |m| n[..m.start()].to_string());
    let want = match crate::classify(&stem(local_name), true).mate {
        Some(crate::Mate::R1) => format!("{run}_1"),
        Some(crate::Mate::R2) => format!("{run}_2"),
        _ => run.to_string(),
    };
    if let Some(f) = files.iter().find(|f| stem(&f.name) == want) {
        return Some(f);
    }
    match files.as_slice() {
        [one] => Some(one),
        _ => None,
    }
}

fn fetch_ena(run: &str) -> Result<Vec<EnaFile>> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .user_agent(concat!("sheetgen/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let url = ena_url(run);
    let mut last = None;
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(Duration::from_secs(2 * attempt));
        }
        match agent.get(&url).call() {
            Ok(mut resp) => {
                let body = resp.body_mut().read_to_string().with_context(|| format!("reading {url}"))?;
                return parse_filereport(&body).with_context(|| format!("parsing {url}"));
            }
            Err(e) => last = Some(e),
        }
    }
    Err(anyhow::anyhow!("{}", last.unwrap())).with_context(|| format!("looking up {run} at ENA ({url})"))
}

// ---------------------------------------------------------------------------------------------
// Inputs

/// FASTQ files named in a sample sheet: every cell that looks like a FASTQ path. Relative paths
/// are taken from the current folder, as pipelines do. URLs are skipped (noted).
fn sheet_files(sheet: &Path, notes: &mut Vec<String>) -> Result<Vec<PathBuf>> {
    let text = std::fs::read_to_string(sheet).with_context(|| format!("reading {}", sheet.display()))?;
    let tab = sheet.extension().is_some_and(|e| e.eq_ignore_ascii_case("tsv"))
        || text.lines().next().is_some_and(|h| h.contains('\t') && !h.contains(','));
    let mut out = Vec::new();
    for line in text.lines().skip(1) {
        for cell in line.split(if tab { '\t' } else { ',' }) {
            let cell = cell.trim().trim_matches('"');
            if !crate::FASTQ_EXT.is_match(cell) {
                continue;
            }
            if cell.contains("://") {
                notes.push(format!("skipped {cell} (a URL; download it first)"));
                continue;
            }
            out.push(PathBuf::from(cell));
        }
    }
    if out.is_empty() {
        bail!("{}: no FASTQ paths in the sheet", sheet.display());
    }
    Ok(out)
}

fn collect(args: &VerifyArgs, notes: &mut Vec<String>) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for p in &args.paths {
        if p.is_dir() {
            let before = paths.len();
            for e in walkdir::WalkDir::new(p).follow_links(true).sort_by_file_name() {
                let e = e.with_context(|| format!("scanning {}", p.display()))?;
                if e.file_type().is_file() && crate::FASTQ_EXT.is_match(&e.file_name().to_string_lossy()) {
                    paths.push(e.path().to_path_buf());
                }
            }
            if paths.len() == before {
                notes.push(format!("no FASTQ files under {}", p.display()));
            }
        } else {
            paths.push(p.clone());
        }
    }
    for s in &args.sheets {
        paths.extend(sheet_files(s, notes)?);
    }
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    for p in paths {
        let abs = p.canonicalize().with_context(|| format!("cannot open {}", p.display()))?;
        if seen.insert(abs.clone()) {
            out.push(abs);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------------------------
// Report

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Ok,
    NotListed,
    Mismatch,
    Corrupt,
}

impl Verdict {
    fn name(self) -> &'static str {
        match self {
            Verdict::Ok => "ok",
            Verdict::NotListed => "not listed",
            Verdict::Mismatch => "mismatch",
            Verdict::Corrupt => "corrupt",
        }
    }
}

/// Corrupt beats mismatch beats not listed. Without any checksum source, unlisted files are ok.
pub fn verdict(c: &Checked, expected: Option<&Expected>, have_source: bool) -> Verdict {
    if c.decoded.problem.is_some() {
        return Verdict::Corrupt;
    }
    match expected {
        Some(e) if e.md5 != c.md5 || e.bytes.is_some_and(|b| b != c.bytes) => Verdict::Mismatch,
        Some(_) => Verdict::Ok,
        None if have_source => Verdict::NotListed,
        None => Verdict::Ok,
    }
}

struct Row {
    path: PathBuf,
    checked: Result<Checked>,
    expected: Option<Expected>,
    note: Option<String>,
}

fn row_json(r: &Row, have_source: bool) -> Value {
    let (expected_md5, expected_bytes, source) = match &r.expected {
        Some(e) => (json!(e.md5), json!(e.bytes), json!(e.source)),
        None => (Value::Null, Value::Null, Value::Null),
    };
    match &r.checked {
        Err(e) => json!({
            "path": r.path, "status": "corrupt", "problem": {"what": format!("{e:#}")},
            "expected_md5": expected_md5, "expected_bytes": expected_bytes, "expected_from": source,
        }),
        Ok(c) => {
            let v = verdict(c, r.expected.as_ref(), have_source);
            let problem = c.decoded.problem.as_ref().map(|p| {
                json!({"what": p.what, "byte_offset": p.byte_offset, "record": p.record, "line": p.line})
            });
            json!({
                "path": r.path,
                "status": v.name(),
                "bytes": c.bytes,
                "md5": c.md5,
                "sha256": c.sha256,
                "expected_md5": expected_md5,
                "expected_bytes": expected_bytes,
                "expected_from": source,
                "compression": c.kind_name,
                "gzip_members": if c.kind_name == "gzip" { json!(c.decoded.members) } else { Value::Null },
                "uncompressed_bytes": c.decoded.uncompressed,
                "reads": c.decoded.reads,
                "bases": c.decoded.bases,
                "problem": problem,
                "note": r.note,
                "seconds": (c.seconds * 100.0).round() / 100.0,
            })
        }
    }
}

fn row_text(r: &Row, have_source: bool) -> String {
    let path = r.path.display();
    let c = match &r.checked {
        Err(e) => return format!("CORRUPT     {path}: {e:#}"),
        Ok(c) => c,
    };
    let v = verdict(c, r.expected.as_ref(), have_source);
    let mut detail = Vec::new();
    match (&c.decoded.problem, v) {
        (Some(p), _) => detail.push(p.describe()),
        (None, _) if c.kind_name == "not checked" => detail.push(format!("{} bytes, structure not checked (.bz2/.zst)", c.bytes)),
        (None, _) => detail.push(format!("{} reads, {} bases", c.decoded.reads, c.decoded.bases)),
    }
    match &r.expected {
        Some(e) if e.md5 != c.md5 => detail.push(format!("md5 {} but {} lists {}", c.md5, e.source, e.md5)),
        Some(e) if e.bytes.is_some_and(|b| b != c.bytes) => {
            detail.push(format!("{} bytes but {} lists {}", c.bytes, e.source, e.bytes.unwrap()))
        }
        Some(e) => detail.push(format!("md5 matches {}", e.source)),
        None => detail.push(format!("md5 {}", c.md5)),
    }
    if let Some(n) = &r.note {
        detail.push(n.clone());
    }
    let label = match v {
        Verdict::Ok => "ok",
        Verdict::NotListed => "NOT LISTED",
        Verdict::Mismatch => "MISMATCH",
        Verdict::Corrupt => "CORRUPT",
    };
    format!("{label:<11} {path}: {}", detail.join("; "))
}

/// Run `sheetgen verify`. Returns whether every file passed.
pub fn run(args: VerifyArgs) -> Result<bool> {
    let mut notes = Vec::new();
    if args.paths.is_empty() && args.sheets.is_empty() {
        bail!("give read files, folders, or --sheet FILE");
    }
    let files = collect(&args, &mut notes)?;
    if files.is_empty() {
        bail!("no FASTQ files to check");
    }
    let md5s = Md5Lists::load(&args.md5)?;
    let have_source = !args.md5.is_empty() || args.ena;

    // Run accessions: from `--run NAME=ACC`, else from the file name.
    let mut run_of: BTreeMap<String, String> = BTreeMap::new();
    for r in &args.runs {
        let (name, acc) = r.split_once('=').with_context(|| format!("--run expects FILE=ACCESSION, got `{r}`"))?;
        let name = Path::new(name.trim()).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        run_of.insert(name, acc.trim().to_string());
    }
    let mut rows: Vec<Row> = Vec::new();
    let mut ena: BTreeMap<String, Result<Vec<EnaFile>, String>> = BTreeMap::new();
    for path in &files {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let mut row = Row { path: path.clone(), checked: Err(anyhow::anyhow!("not checked")), expected: md5s.find(path), note: None };
        if args.ena && row.expected.is_none() {
            match run_of.get(&name).cloned().or_else(|| run_in_name(&name)) {
                None => row.note = Some("no run accession in the name (use --run FILE=ACCESSION)".into()),
                Some(run) => {
                    let got = ena.entry(run.clone()).or_insert_with(|| {
                        eprintln!("sheetgen verify: looking up {run} at ENA");
                        fetch_ena(&run).map_err(|e| format!("{e:#}"))
                    });
                    match got {
                        Err(e) => bail!("{e}"),
                        Ok(list) => match match_ena(&name, &run, list) {
                            Some(f) => {
                                row.expected = Some(Expected { md5: f.md5.clone(), bytes: f.bytes, source: format!("ENA {run} ({})", f.name) })
                            }
                            None if list.iter().all(|f| f.run != run) => row.note = Some(format!("ENA has no FASTQ files for {run}")),
                            None => {
                                let names: Vec<_> = list.iter().filter(|f| f.run == run).map(|f| f.name.as_str()).collect();
                                row.note = Some(format!("can't tell which of ENA's files for {run} this is: {}", names.join(", ")))
                            }
                        },
                    }
                }
            }
        }
        rows.push(row);
    }

    // Check the files, `jobs` at a time.
    let start = Instant::now();
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<Result<Checked>>>> = Mutex::new((0..rows.len()).map(|_| None).collect());
    let print = Mutex::new(());
    let jobs = args.jobs.clamp(1, rows.len());
    std::thread::scope(|s| {
        for _ in 0..jobs {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some(row) = rows.get(i) else { break };
                    let checked = check_file(&row.path);
                    if !args.json {
                        let shown = Row { path: row.path.clone(), checked, expected: row.expected.clone(), note: row.note.clone() };
                        let _g = print.lock().unwrap();
                        println!("{}", row_text(&shown, have_source));
                        results.lock().unwrap()[i] = Some(shown.checked);
                    } else {
                        results.lock().unwrap()[i] = Some(checked);
                    }
                }
            });
        }
    });
    for (row, c) in rows.iter_mut().zip(results.into_inner().unwrap()) {
        row.checked = c.unwrap();
    }
    let seconds = start.elapsed().as_secs_f64();

    let checked_paths: BTreeSet<&PathBuf> = rows.iter().map(|r| &r.path).collect();
    let unchecked: Vec<String> = md5s
        .listed
        .iter()
        .filter(|p| !checked_paths.contains(p))
        .map(|p| p.display().to_string())
        .collect();

    let mut counts: BTreeMap<Verdict, usize> = BTreeMap::new();
    let mut total_bytes = 0u64;
    for r in &rows {
        let v = match &r.checked {
            Ok(c) => {
                total_bytes += c.bytes;
                verdict(c, r.expected.as_ref(), have_source)
            }
            Err(_) => Verdict::Corrupt,
        };
        *counts.entry(v).or_default() += 1;
    }
    let all_ok = counts.keys().all(|v| *v == Verdict::Ok);
    let mb_s = total_bytes as f64 / 1e6 / seconds.max(1e-9);

    if args.json {
        let summary: serde_json::Map<String, Value> = [Verdict::Ok, Verdict::Mismatch, Verdict::Corrupt, Verdict::NotListed]
            .iter()
            .map(|v| (v.name().replace(' ', "_"), json!(counts.get(v).copied().unwrap_or(0))))
            .collect();
        let report = json!({
            "ok": all_ok,
            "files": rows.iter().map(|r| row_json(r, have_source)).collect::<Vec<_>>(),
            "summary": summary,
            "listed_not_checked": unchecked,
            "notes": notes,
            "bytes": total_bytes,
            "seconds": (seconds * 100.0).round() / 100.0,
        });
        println!("{}", serde_json::to_string_pretty(&report)?);
    }
    for n in &notes {
        eprintln!("  note: {n}");
    }
    if !unchecked.is_empty() {
        eprintln!("  note: listed in --md5 but not checked: {}", unchecked.join(", "));
    }
    let parts: Vec<String> = counts.iter().map(|(v, n)| format!("{n} {}", v.name())).collect();
    eprintln!(
        "sheetgen verify: {} files: {}; {:.1} MB in {:.1} s ({:.0} MB/s)",
        rows.len(),
        parts.join(", "),
        total_bytes as f64 / 1e6,
        seconds,
        mb_s
    );
    Ok(all_ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn fastq(reads: usize, len: usize) -> Vec<u8> {
        let mut v = Vec::new();
        for i in 0..reads {
            let seq: String = (0..len).map(|j| ['A', 'C', 'G', 'T'][(i * 7 + j * 3) % 4]).collect();
            writeln!(v, "@r{i} test\n{seq}\n+\n{}", "I".repeat(len)).unwrap();
        }
        v
    }

    fn gz(data: &[u8], level: flate2::Compression) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), level);
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    fn write(dir: &Path, name: &str, data: &[u8]) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, data).unwrap();
        p
    }

    #[test]
    fn good_gzip_counts_reads_and_hashes() {
        let t = tempfile::tempdir().unwrap();
        // Over 1 MiB compressed at level 0, so records and members cross block boundaries.
        let text = fastq(5000, 150);
        let data = gz(&text, flate2::Compression::none());
        assert!(data.len() > BLOCK);
        let c = check_file(&write(t.path(), "x.fastq.gz", &data)).unwrap();
        assert_eq!(c.decoded.problem, None);
        assert_eq!((c.decoded.reads, c.decoded.bases, c.decoded.members), (5000, 750_000, 1));
        assert_eq!(c.decoded.uncompressed, text.len() as u64);
        assert_eq!(c.bytes, data.len() as u64);
        assert_eq!(c.md5, hex(&Md5::digest(&data)));
        assert_eq!(c.sha256, hex(&Sha256::digest(&data)));
    }

    #[test]
    fn truncated_gzip_is_corrupt() {
        let t = tempfile::tempdir().unwrap();
        let data = gz(&fastq(2000, 100), flate2::Compression::fast());
        for cut in [data.len() / 2, data.len() - 4] {
            let c = check_file(&write(t.path(), "x.fq.gz", &data[..cut])).unwrap();
            let p = c.decoded.problem.clone().expect("truncation found");
            assert!(p.what.contains("truncated"), "{p:?}");
            assert_eq!(p.byte_offset, Some(cut as u64));
            assert_eq!(verdict(&c, None, false), Verdict::Corrupt);
        }
    }

    #[test]
    fn flipped_byte_is_a_crc_error() {
        let t = tempfile::tempdir().unwrap();
        // Stored blocks: the flipped byte decodes fine and only the CRC-32 catches it.
        let mut data = gz(&fastq(100, 100), flate2::Compression::none());
        let mid = data.len() / 2;
        data[mid] ^= 0x04;
        let c = check_file(&write(t.path(), "x.fq.gz", &data)).unwrap();
        let p = c.decoded.problem.expect("CRC error found");
        assert!(p.what.contains("damaged") && p.what.contains("check"), "{p:?}");
        assert!(p.byte_offset.unwrap() > mid as u64, "{p:?}");
        // Compressed data: the flip breaks the deflate stream or the CRC, either way corrupt.
        let mut data = gz(&fastq(2000, 100), flate2::Compression::default());
        let mid = data.len() / 2;
        data[mid] ^= 0xff;
        let c = check_file(&write(t.path(), "y.fq.gz", &data)).unwrap();
        let p = c.decoded.problem.unwrap();
        assert!(p.what.contains("damaged") && p.byte_offset.is_some() && p.record.is_some(), "{p:?}");
    }

    #[test]
    fn multi_member_gzip_is_read_to_the_end() {
        let t = tempfile::tempdir().unwrap();
        let mut data = gz(&fastq(300, 80), flate2::Compression::fast());
        data.extend(gz(&fastq(200, 50), flate2::Compression::best()));
        data.extend(gz(b"", flate2::Compression::fast()));
        let c = check_file(&write(t.path(), "x.fastq.gz", &data)).unwrap();
        assert_eq!(c.decoded.problem, None);
        assert_eq!((c.decoded.members, c.decoded.reads, c.decoded.bases), (3, 500, 300 * 80 + 200 * 50));
        // A damaged second member is found even though the first one is fine.
        let first = gz(&fastq(300, 80), flate2::Compression::fast()).len();
        let mut bad = data.clone();
        bad[first + 1] = 0; // second member's magic
        let c = check_file(&write(t.path(), "y.fastq.gz", &bad)).unwrap();
        let p = c.decoded.problem.unwrap();
        assert!(p.what.contains("after gzip member 1"), "{p:?}");
        assert_eq!(c.decoded.reads, 300);
    }

    #[test]
    fn trailing_garbage_and_non_gzip_are_corrupt() {
        let t = tempfile::tempdir().unwrap();
        let mut data = gz(&fastq(10, 50), flate2::Compression::fast());
        data.extend(b"garbage after the end");
        let p = check_file(&write(t.path(), "x.fq.gz", &data)).unwrap().decoded.problem.unwrap();
        assert!(p.what.contains("not gzip"), "{p:?}");
        let p = check_file(&write(t.path(), "plain.fq.gz", &fastq(10, 50))).unwrap().decoded.problem.unwrap();
        assert!(p.what.starts_with("not gzip data"), "{p:?}");
        let p = check_file(&write(t.path(), "empty.fq.gz", b"")).unwrap().decoded.problem.unwrap();
        assert_eq!(p.what, "empty file");
        let p = check_file(&write(t.path(), "zipped.fastq", &data)).unwrap().decoded.problem.unwrap();
        assert!(p.what.contains("not named .gz"), "{p:?}");
    }

    #[test]
    fn plain_fastq_with_crlf_and_no_final_newline() {
        let t = tempfile::tempdir().unwrap();
        let c = check_file(&write(t.path(), "x.fastq", b"@a\r\nACGT\r\n+\r\nIIII\r\n@b\nAC\n+b\nII")).unwrap();
        assert_eq!(c.decoded.problem, None);
        assert_eq!((c.decoded.reads, c.decoded.bases), (2, 6));
        // Blank lines only at the end are fine.
        let c = check_file(&write(t.path(), "y.fq", b"@a\nACGT\n+\nIIII\n\n\n")).unwrap();
        assert_eq!(c.decoded.problem, None);
    }

    #[test]
    fn broken_fastq_records_are_located() {
        let t = tempfile::tempdir().unwrap();
        let rec1 = b"@a\nACGT\n+\nIIII\n";
        let cases: [(&[u8], &str, u64, u64); 5] = [
            (b"@b\nACGT\n+\nIII\n", "quality is 3", 2, 8),
            (b"b\nACGT\n+\nIIII\n", "doesn't start with @", 2, 5),
            (b"@b\nACGT\n-\nIIII\n", "doesn't start with +", 2, 7),
            (b"@b\nACGT\n", "ends inside a record", 2, 7),
            (b"\n@b\nACGT\n+\nIIII\n", "blank line", 2, 5),
        ];
        for (second, what, record, line) in cases {
            let mut data = rec1.to_vec();
            data.extend_from_slice(second);
            let c = check_file(&write(t.path(), "x.fastq", &data)).unwrap();
            let p = c.decoded.problem.unwrap_or_else(|| panic!("no problem found for {what}"));
            assert!(p.what.contains(what), "{p:?}");
            assert_eq!((p.record, p.line), (Some(record), Some(line)), "{what}: {p:?}");
            // The same record, gzipped: same record and line, offset in compressed bytes.
            let c = check_file(&write(t.path(), "x.fastq.gz", &gz(&data, flate2::Compression::fast()))).unwrap();
            let q = c.decoded.problem.unwrap();
            assert_eq!((q.what, q.record, q.line), (p.what, p.record, p.line));
        }
        // Plain files: the offset is where the bad line starts.
        let mut data = rec1.to_vec();
        data.extend_from_slice(b"@b\nACGT\n+\nIII\n");
        let p = check_file(&write(t.path(), "x.fq", &data)).unwrap().decoded.problem.unwrap();
        assert_eq!(p.byte_offset, Some(rec1.len() as u64 + 10));
    }

    #[test]
    fn md5_lists_and_mismatch() {
        assert_eq!(
            parse_md5_line("852775B2AA83D5FDBA426911E671A935  AF47_ont.fastq.gz"),
            Some(("852775b2aa83d5fdba426911e671a935".into(), "AF47_ont.fastq.gz".into()))
        );
        assert_eq!(
            parse_md5_line("852775b2aa83d5fdba426911e671a935 *sub/a b.fq.gz\r"),
            Some(("852775b2aa83d5fdba426911e671a935".into(), "sub/a b.fq.gz".into()))
        );
        assert_eq!(
            parse_md5_line("MD5 (x.fq.gz) = 852775b2aa83d5fdba426911e671a935"),
            Some(("852775b2aa83d5fdba426911e671a935".into(), "x.fq.gz".into()))
        );
        assert_eq!(parse_md5_line("# comment"), None);
        assert_eq!(parse_md5_line("not-a-hash  x.fq.gz"), None);

        let t = tempfile::tempdir().unwrap();
        let data = gz(&fastq(10, 50), flate2::Compression::fast());
        let good = write(t.path(), "good.fq.gz", &data);
        let bad = write(t.path(), "bad.fq.gz", &data);
        let other = write(t.path(), "other.fq.gz", &data);
        let md5 = hex(&Md5::digest(&data));
        let list = write(t.path(), "MD5SUMS", format!("{md5}  good.fq.gz\n{}  ./bad.fq.gz\n", "0".repeat(32)).as_bytes());
        let lists = Md5Lists::load(&[list]).unwrap();
        let dir = t.path().canonicalize().unwrap();
        let (g, b) = (lists.find(&dir.join("good.fq.gz")), lists.find(&dir.join("bad.fq.gz")));
        assert_eq!(verdict(&check_file(&good).unwrap(), g.as_ref(), true), Verdict::Ok);
        assert_eq!(verdict(&check_file(&bad).unwrap(), b.as_ref(), true), Verdict::Mismatch);
        assert_eq!(lists.find(&dir.join("other.fq.gz")), None);
        assert_eq!(verdict(&check_file(&other).unwrap(), None, true), Verdict::NotListed);
        assert_eq!(verdict(&check_file(&other).unwrap(), None, false), Verdict::Ok);
        // Right MD5, wrong size (ENA lists sizes): mismatch.
        let e = Expected { md5, bytes: Some(1), source: "ENA".into() };
        assert_eq!(verdict(&check_file(&good).unwrap(), Some(&e), true), Verdict::Mismatch);
    }

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)).unwrap()
    }

    #[test]
    fn ena_filereport_parsing_and_matching() {
        assert!(ena_url("ERR15424439").contains("accession=ERR15424439&result=read_run"));
        let paired = parse_filereport(&fixture("ena-filereport-ERR044595.tsv")).unwrap();
        assert_eq!(paired.len(), 2);
        assert_eq!(paired[1].name, "ERR044595_2.fastq.gz");
        assert_eq!(paired[1].md5, "4fc3ac2799fe9f072dcb3b14ec195804");
        assert_eq!(paired[1].bytes, Some(94427453));
        let m = |name: &str| match_ena(name, "ERR044595", &paired).map(|f| f.name.clone());
        assert_eq!(m("ERR044595_1.fastq.gz").as_deref(), Some("ERR044595_1.fastq.gz"));
        assert_eq!(m("ERR044595_R2.fq.gz").as_deref(), Some("ERR044595_2.fastq.gz"));
        assert_eq!(m("ERR044595.fastq.gz"), None);

        let single = parse_filereport(&fixture("ena-filereport-ERR15424439.tsv")).unwrap();
        let f = match_ena("AHB29_ont.fastq.gz", "ERR15424439", &single).unwrap();
        assert_eq!((f.md5.as_str(), f.bytes), ("6ff210436d629b9d64bb732028654a41", Some(567992870)));
        assert_eq!(match_ena("x.fq.gz", "ERR1", &single), None);

        // A run without FASTQ files at ENA: no rows; a malformed header is an error.
        assert!(parse_filereport("run_accession\tfastq_ftp\tfastq_md5\tfastq_bytes\nERR1\t\t\t\n").unwrap().is_empty());
        assert!(parse_filereport("run_accession\tsubmitted_ftp\n").is_err());

        assert_eq!(run_in_name("ERR044595_1.fastq.gz").as_deref(), Some("ERR044595"));
        assert_eq!(run_in_name("SRX1_SRR12345678.fastq.gz").as_deref(), Some("SRR12345678"));
        assert_eq!(run_in_name("DRR123456_R1.fq.gz").as_deref(), Some("DRR123456"));
        assert_eq!(run_in_name("AHB29_ont.fastq.gz"), None);
        assert_eq!(run_in_name("XERR044595.fq.gz"), None);
    }
}
