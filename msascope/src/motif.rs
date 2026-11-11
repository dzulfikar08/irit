//! Motif search in the residues of every sequence, gaps ignored, so a motif matches across gaps in
//! the alignment. Three ways to write one:
//!
//! - **Plain**: letters, case-insensitive (`HGK`). For nucleotides the IUPAC codes match their
//!   bases (`R` = A or G, `N` any) and T and U match each other; for protein `X` matches any
//!   residue.
//! - **PROSITE**: elements joined by `-`: a residue, `x` (any), `[ST]` (one of), `{P}` (not P),
//!   each optionally repeated `(3)` or `(2,4)`; `<` at the start or `>` at the end anchors to the
//!   sequence's ends (`C-x(2,4)-C-x(3)-[LIVMFYWC]`). A pattern with `-`, brackets or parentheses
//!   is read this way.
//! - **Regular expression** between slashes (`/C.{2,4}C/`), on the ungapped residues in upper case.
//!
//! Matches don't overlap: after a match the search goes on after its last residue.

use std::sync::atomic::{AtomicBool, Ordering};

use rayon::prelude::*;

use crate::Alignment;
use crate::alphabet::{GAP, Kind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    Plain,
    Prosite,
    Regex,
}

#[derive(Clone, Debug)]
pub struct Motif {
    pub syntax: Syntax,
    /// The regular expression the motif became.
    pub pattern: String,
    re: regex::bytes::Regex,
}

/// The bases an IUPAC code stands for (T and U both).
fn iupac(b: u8) -> Option<&'static str> {
    Some(match b {
        b'A' => "A",
        b'C' => "C",
        b'G' => "G",
        b'T' | b'U' => "TU",
        b'R' => "AGR",
        b'Y' => "CTUY",
        b'S' => "CGS",
        b'W' => "ATUW",
        b'K' => "GTUK",
        b'M' => "ACM",
        b'B' => "CGTUB",
        b'D' => "AGTUD",
        b'H' => "ACTUH",
        b'V' => "ACGV",
        b'N' => "ACGTURYSWKMBDHVN",
        _ => return None,
    })
}

/// One residue of a plain motif, as a regex piece.
fn residue(b: u8, kind: Kind) -> Result<String, String> {
    let b = b.to_ascii_uppercase();
    if !b.is_ascii_uppercase() && b != b'*' {
        return Err(format!("'{}' is not a residue", b as char));
    }
    Ok(match kind {
        Kind::Nucleotide => match iupac(b) {
            Some(s) if s.len() == 1 => s.to_string(),
            Some(s) => format!("[{s}]"),
            None if b == b'X' => ".".into(),
            None => return Err(format!("'{}' is not a nucleotide code", b as char)),
        },
        Kind::Protein if b == b'X' => ".".into(),
        Kind::Protein if b == b'*' => r"\*".into(),
        Kind::Protein => (b as char).to_string(),
    })
}

/// Residues allowed in a PROSITE set (`[...]` or `{...}`).
fn set_body(body: &str) -> Result<String, String> {
    let mut out = String::new();
    for c in body.chars() {
        match c {
            'A'..='Z' | 'a'..='z' => out.push(c.to_ascii_uppercase()),
            _ => return Err(format!("'{c}' in a set: only residue letters can go in [ ] or {{ }}")),
        }
    }
    if out.is_empty() {
        return Err("Empty set".into());
    }
    Ok(out)
}

/// A PROSITE pattern as a regular expression.
pub fn prosite_to_regex(p: &str) -> Result<String, String> {
    let p = p.trim().trim_end_matches('.');
    let parts: Vec<&str> = p.split('-').collect();
    let mut out = String::new();
    for (i, part) in parts.iter().enumerate() {
        let mut e = part.trim();
        if e.is_empty() {
            return Err("Empty element between '-'".into());
        }
        if i == 0
            && let Some(rest) = e.strip_prefix('<')
        {
            out.push('^');
            e = rest;
        }
        let mut end = false;
        if i == parts.len() - 1
            && let Some(rest) = e.strip_suffix('>')
        {
            end = true;
            e = rest;
        }
        let (core, rep) = match e.find('(') {
            Some(k) => {
                let r = e[k + 1..].strip_suffix(')').ok_or_else(|| format!("Missing ')' in '{part}'"))?;
                (&e[..k], Some(r))
            }
            None => (e, None),
        };
        let piece = if core.eq_ignore_ascii_case("x") {
            ".".to_string()
        } else if let Some(b) = core.strip_prefix('[').and_then(|c| c.strip_suffix(']')) {
            // `>` inside a set (end of sequence allowed) is dropped.
            format!("[{}]", set_body(&b.replace('>', ""))?)
        } else if let Some(b) = core.strip_prefix('{').and_then(|c| c.strip_suffix('}')) {
            format!("[^{}]", set_body(b)?)
        } else if core.len() == 1 && core.as_bytes()[0].is_ascii_alphabetic() {
            core.to_ascii_uppercase()
        } else {
            return Err(format!("Can't read '{part}': expected a residue, x, [..] or {{..}}"));
        };
        out.push_str(&piece);
        if let Some(r) = rep {
            let ok = |s: &str| !s.trim().is_empty() && s.trim().bytes().all(|b| b.is_ascii_digit());
            match r.split_once(',') {
                Some((a, b)) if ok(a) && ok(b) => out.push_str(&format!("{{{},{}}}", a.trim(), b.trim())),
                None if ok(r) => out.push_str(&format!("{{{}}}", r.trim())),
                _ => return Err(format!("Bad repeat '({r})': use (3) or (2,4)")),
            }
        }
        if end {
            out.push('$');
        }
    }
    Ok(out)
}

impl Motif {
    /// Read a motif for an alignment of residue type `kind`.
    pub fn compile(text: &str, kind: Kind) -> Result<Motif, String> {
        let t = text.trim();
        if t.is_empty() {
            return Err("Empty motif".into());
        }
        let (syntax, pattern) = if let Some(re) = t.strip_prefix('/').and_then(|r| r.strip_suffix('/')).filter(|r| !r.is_empty()) {
            (Syntax::Regex, format!("(?i){re}"))
        } else if t.contains(['-', '[', ']', '{', '}', '(', ')', '<', '>']) {
            (Syntax::Prosite, prosite_to_regex(t)?)
        } else {
            let mut p = String::new();
            for b in t.bytes() {
                if b.is_ascii_whitespace() {
                    continue;
                }
                p += &residue(b, kind)?;
            }
            (Syntax::Plain, p)
        };
        let re = regex::bytes::RegexBuilder::new(&pattern)
            .unicode(false)
            .size_limit(1 << 24)
            .build()
            .map_err(|e| format!("Bad pattern: {}", e.to_string().lines().last().unwrap_or("")))?;
        Ok(Motif { syntax, pattern, re })
    }

    /// Matches in one ungapped sequence, as (start, end) residue indices, end exclusive.
    pub fn find(&self, residues: &[u8]) -> Vec<(usize, usize)> {
        self.re.find_iter(residues).filter(|m| m.end() > m.start()).map(|m| (m.start(), m.end())).collect()
    }
}

/// One match: its row, the alignment columns of its first and last residue, and the residue index
/// (from 0, in the ungapped sequence) of its first residue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub row: u32,
    pub start: u32,
    pub end: u32,
    pub residue: u32,
}

#[derive(Clone, Debug, Default)]
pub struct Hits {
    /// Sorted by row, then column; at most the limit given to [`search`].
    pub hits: Vec<Hit>,
    /// Matches found, including those past the limit.
    pub total: usize,
    /// Sequences with at least one match.
    pub rows: usize,
}

impl Hits {
    /// Indices into `hits` of the matches in alignment row `r`.
    pub fn in_row(&self, r: u32) -> std::ops::Range<usize> {
        let a = self.hits.partition_point(|h| h.row < r);
        let b = self.hits.partition_point(|h| h.row <= r);
        a..b
    }
}

/// Rows searched together; matches are kept in row order, and once the limit is reached the
/// rest are only counted, so memory stays near the limit (plus one step's matches). Each step
/// waits for its slowest thread, so steps are large.
const STEP: usize = 16_384;

/// Search every row (in parallel). Stops with `None` when `cancel` is set. The first `limit`
/// matches (by row) are kept; all are counted.
pub fn search(aln: &Alignment, motif: &Motif, limit: usize, cancel: &AtomicBool) -> Option<Hits> {
    let cols = aln.cols();
    let mut out = Hits::default();
    for start in (0..aln.rows()).step_by(STEP) {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        let keep = out.hits.len() < limit;
        let per_row: Vec<(usize, Vec<Hit>)> = (start..(start + STEP).min(aln.rows()))
            .into_par_iter()
            .map_init(
                || (vec![0u8; cols], Vec::with_capacity(cols), Vec::<u32>::with_capacity(cols)),
                |(buf, res, at), r| {
                    aln.matrix.decode_row(r, 0, buf);
                    res.clear();
                    at.clear();
                    for (c, &code) in buf.iter().enumerate() {
                        if code != GAP {
                            res.push(aln.symbol(code));
                            at.push(c as u32);
                        }
                    }
                    if !keep {
                        return (motif.find(res).len(), Vec::new());
                    }
                    let v: Vec<Hit> = motif.find(res).into_iter().map(|(s, e)| Hit { row: r as u32, start: at[s], end: at[e - 1], residue: s as u32 }).collect();
                    (v.len(), v)
                },
            )
            .collect();
        for (n, v) in per_row {
            if n == 0 {
                continue;
            }
            out.rows += 1;
            out.total += n;
            let room = limit.saturating_sub(out.hits.len());
            out.hits.extend(v.into_iter().take(room));
        }
    }
    if cancel.load(Ordering::Relaxed) {
        return None;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_bytes;

    fn spans(m: &Motif, s: &str) -> Vec<(usize, usize)> {
        m.find(s.as_bytes())
    }

    #[test]
    fn prosite_translation() {
        assert_eq!(prosite_to_regex("C-x(2,4)-C-x(3)-[LIVMFYWC].").unwrap(), "C.{2,4}C.{3}[LIVMFYWC]");
        assert_eq!(prosite_to_regex("<M-{P}-x-[ST>]").unwrap(), "^M[^P].[ST]");
        assert_eq!(prosite_to_regex("a-b>").unwrap(), "AB$");
        assert!(prosite_to_regex("C--C").is_err());
        assert!(prosite_to_regex("C-x(2").is_err());
        assert!(prosite_to_regex("C-x(a)").is_err());
        assert!(prosite_to_regex("C-[1]").is_err());
        assert!(prosite_to_regex("CC-x").is_err());
    }

    #[test]
    fn syntaxes() {
        let m = Motif::compile("C-x(2,4)-C", Kind::Protein).unwrap();
        assert_eq!(m.syntax, Syntax::Prosite);
        assert_eq!(spans(&m, "ACAACAAAAAC"), [(1, 5)]);
        let m = Motif::compile("hgk", Kind::Protein).unwrap();
        assert_eq!((m.syntax, spans(&m, "AHGKHGK")), (Syntax::Plain, vec![(1, 4), (4, 7)]));
        let m = Motif::compile("/H.K/", Kind::Protein).unwrap();
        assert_eq!((m.syntax, spans(&m, "XHGKX")), (Syntax::Regex, vec![(1, 4)]));
        // IUPAC for nucleotides; T matches U.
        let m = Motif::compile("GRN", Kind::Nucleotide).unwrap();
        assert_eq!(spans(&m, "TGAC"), [(1, 4)]);
        assert_eq!(spans(&m, "GCA"), []);
        let m = Motif::compile("AT", Kind::Nucleotide).unwrap();
        assert_eq!(spans(&m, "GAU"), [(1, 3)]);
        // X is any residue in protein.
        assert_eq!(spans(&Motif::compile("CXC", Kind::Protein).unwrap(), "CAC"), [(0, 3)]);
        assert!(Motif::compile("C1", Kind::Protein).is_err());
        assert!(Motif::compile("/(/", Kind::Protein).is_err());
        assert!(Motif::compile("", Kind::Protein).is_err());
    }

    #[test]
    fn matches_across_gaps() {
        let a = parse_bytes(b">s0\nAC--G-TAC\n>s1\nACGTACGTA\n>s2\n--A-CG---\n").unwrap();
        let m = Motif::compile("ACG", Kind::Nucleotide).unwrap();
        let h = search(&a, &m, 100, &AtomicBool::new(false)).unwrap();
        assert_eq!(h.total, 4);
        assert_eq!(h.rows, 3);
        assert_eq!(h.hits[0], Hit { row: 0, start: 0, end: 4, residue: 0 });
        assert_eq!(h.hits[1], Hit { row: 1, start: 0, end: 2, residue: 0 });
        assert_eq!(h.hits[2], Hit { row: 1, start: 4, end: 6, residue: 4 });
        assert_eq!(h.hits[3], Hit { row: 2, start: 2, end: 5, residue: 0 });
        assert_eq!(h.in_row(1), 1..3);
        assert_eq!(h.in_row(5), 4..4);
        // The limit keeps the first hits, counts all.
        let h = search(&a, &m, 2, &AtomicBool::new(false)).unwrap();
        assert_eq!((h.hits.len(), h.total), (2, 4));
        assert!(search(&a, &m, 2, &AtomicBool::new(true)).is_none());
    }

    #[test]
    fn limit_keeps_the_first_rows_across_steps() {
        let text: String = (0..40_000).map(|i| format!(">s{i}\nTACGTACGT\n")).collect();
        let a = parse_bytes(text.as_bytes()).unwrap();
        let m = Motif::compile("ACG", Kind::Nucleotide).unwrap();
        let h = search(&a, &m, 30_000, &AtomicBool::new(false)).unwrap();
        assert_eq!((h.total, h.rows, h.hits.len()), (80_000, 40_000, 30_000));
        assert!(h.hits.windows(2).all(|w| (w[0].row, w[0].start) < (w[1].row, w[1].start)));
        assert_eq!(h.hits.last().unwrap().row, 14_999);
    }
}
