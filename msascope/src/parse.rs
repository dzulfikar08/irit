//! Readers for aligned FASTA (plain or gzip), Clustal and Stockholm.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

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

/// Load an alignment, decompressing gzip transparently.
pub fn load(path: &Path) -> Result<Alignment> {
    load_as(path, None)
}

/// [`load`] with the residue type given instead of guessed (`--type`).
pub fn load_as(path: &Path, ty: Option<SeqType>) -> Result<Alignment> {
    let open = || -> Result<Reader<'static>> {
        let f = || File::open(path).with_context(|| format!("opening {}", path.display()));
        let mut magic = [0u8; 2];
        let gz = f()?.read(&mut magic)? == 2 && magic == [0x1f, 0x8b];
        Ok(if gz {
            Box::new(BufReader::with_capacity(1 << 16, flate2::read::MultiGzDecoder::new(f()?)))
        } else {
            Box::new(BufReader::with_capacity(1 << 16, f()?))
        })
    };
    read(&open, ty).with_context(|| format!("reading {}", path.display()))
}

pub fn parse_bytes(data: &[u8]) -> Result<Alignment> {
    parse_bytes_as(data, None)
}

pub fn parse_bytes_as(data: &[u8], ty: Option<SeqType>) -> Result<Alignment> {
    read(&|| Ok(Box::new(data) as Reader), ty)
}

/// Parse from `open`, which must return the same content each time: Clustal and Stockholm are read
/// twice, and FASTA again in the rare case that the residue type changes after rows were packed.
fn read<'a>(open: &dyn Fn() -> Result<Reader<'a>>, ty: Option<SeqType>) -> Result<Alignment> {
    let mut r = open()?;
    match detect_format(r.fill_buf()?)? {
        Format::Fasta => {
            let mut forced = None;
            loop {
                let res = fasta(r, CHUNK, forced, ty);
                let Some(a) = res.as_ref().err().and_then(|e| e.downcast_ref::<Restart>()).map(|r| r.0) else { return res };
                forced = Some(a);
                r = open()?;
            }
        }
        f => interleaved(open, f, ty),
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
        if self.alpha.kind == Kind::Nucleotide && !self.question {
            self.question = recs.par_iter().any(|(_, s)| memchr::memchr(b'?', s).is_some());
        }
        self.names.par_extend(recs.par_iter().map(|(h, _)| lossy(h)));
        Ok(())
    }

    fn finish(self) -> Result<Alignment> {
        let Some(matrix) = self.matrix else { bail!("no sequences found") };
        Ok(Alignment { names: self.names, matrix, alpha: self.alpha })
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
fn fasta(mut r: impl Read, mut chunk: usize, forced: Option<&'static Alphabet>, ty: Option<SeqType>) -> Result<Alignment> {
    let mut b = Builder::new(forced, ty);
    let mut buf: Vec<u8> = Vec::new();
    let mut first = true;
    loop {
        let want = chunk.saturating_sub(buf.len()) as u64;
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
/// Read twice so nothing but the packed matrix is held: the first pass finds the names, lengths and
/// residue type, the second encodes each line straight into its row.
fn interleaved<'a>(open: &dyn Fn() -> Result<Reader<'a>>, format: Format, ty: Option<SeqType>) -> Result<Alignment> {
    let mut index: HashMap<Vec<u8>, usize> = HashMap::new();
    let mut names: Vec<Vec<u8>> = Vec::new();
    let mut lens: Vec<usize> = Vec::new();
    // Block number (from 1) in which each row was last seen.
    let mut seen: Vec<usize> = Vec::new();
    let (mut block, mut in_block) = (1usize, false);
    let mut census = Census::default();
    lines(open()?, format, |ln, l| {
        match l {
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
    let mut m = Matrix::empty(cols, alpha.bits);
    let mut rows = m.grow(names.len());
    let mut off = vec![0usize; names.len()];
    lines(open()?, format, |_, l| {
        if let Line::Seq(name, seq) = l {
            let i = *index.get(name).context("input changed while reading")?;
            off[i] += encode_at(alpha, seq, rows[i], off[i]).map_err(|b| match ty {
                Some(t) => forced_char(b, &lossy(name), t),
                None => bad_char(b, &lossy(name)),
            })?;
        }
        Ok(())
    })?;
    if off.iter().any(|&n| n != cols) {
        bail!("input changed while reading");
    }
    drop(rows);
    Ok(Alignment { names: names.iter().map(|n| lossy(n)).collect(), matrix: m, alpha })
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
                    match fasta(text, chunk, forced, None) {
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
            let e = fasta(&b">a\nACGT\n>b\nACGE\n"[..], chunk, None, Some(SeqType::Dna)).err().unwrap();
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
        assert!(fasta(&b">a\nACGT\n>b\nACG\n"[..], 4, None, None).is_err());
        assert!(parse_bytes(b">a\n\n>b\nAC\n").is_err());
        assert!(parse_bytes(b">a\nAC%T\n").is_err());
        assert!(fasta(&b">a\nACGT\n>b\nAC%T\n"[..], 4, None, None).is_err());
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
}
