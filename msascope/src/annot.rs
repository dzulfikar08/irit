//! Stockholm column and residue annotation: `#=GC <tag> <line>` (one character per column, e.g.
//! SS_cons, seq_cons, RF, PP_cons) and `#=GR <sequence> <tag> <line>` (one character per column of
//! one sequence, e.g. SS, PP, AS). Each line is kept as one byte per alignment column, so it lines
//! up with the alignment's own columns and follows hidden columns through the column map.

use std::collections::HashMap;

/// How a track is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TrackKind {
    /// Secondary structure (`SS_cons`, `SS`): helix and strand bands, base pairs, coil.
    Structure,
    /// Posterior probability (`PP_cons`, `PP`): `0`–`9` and `*`, drawn as a bar.
    Probability,
    /// Anything else (`seq_cons`, `RF`, `AS`, …): its characters as text.
    Text,
}

impl TrackKind {
    pub fn of(tag: &str) -> TrackKind {
        match tag {
            "SS_cons" | "SS" => TrackKind::Structure,
            "PP_cons" | "PP" => TrackKind::Probability,
            _ => TrackKind::Text,
        }
    }
}

/// One `#=GC` line, joined across blocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub tag: Box<str>,
    /// One byte per alignment column.
    pub data: Box<[u8]>,
}

impl Track {
    pub fn kind(&self) -> TrackKind {
        TrackKind::of(&self.tag)
    }
}

/// One `#=GR` line of one sequence, joined across blocks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowAnnot {
    /// Alignment row.
    pub row: u32,
    pub tag: Box<str>,
    /// One byte per alignment column.
    pub data: Box<[u8]>,
}

/// Secondary structure class of one `SS_cons` / `SS` character.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ss {
    /// H (α), G (3₁₀), I (π).
    Helix,
    /// E (strand), B (bridge).
    Strand,
    /// An RNA base pair, opening (`<`, `(`, `[`, `{`) or closing.
    PairOpen,
    PairClose,
    /// Turns, bends, loops and other unpaired positions (T, S, C, `,`, `_`, `-`, `:`, `~`, …).
    Coil,
    /// No annotation (`.`, space).
    None,
}

pub fn ss_class(b: u8) -> Ss {
    match b {
        b'H' | b'G' | b'I' => Ss::Helix,
        b'E' | b'B' => Ss::Strand,
        b'<' | b'(' | b'[' | b'{' => Ss::PairOpen,
        b'>' | b')' | b']' | b'}' => Ss::PairClose,
        b'.' | b' ' => Ss::None,
        // Pseudoknots in Rfam (A–Z paired with a–z) other than the letters above.
        b'A'..=b'Z' => Ss::PairOpen,
        b'a'..=b'z' => Ss::PairClose,
        _ => Ss::Coil,
    }
}

/// Words for a structure character, for the hover card.
pub fn ss_words(b: u8) -> &'static str {
    match b {
        b'H' => "α-helix",
        b'G' => "3₁₀ helix",
        b'I' => "π-helix",
        b'E' => "strand",
        b'B' => "β-bridge",
        b'T' => "turn",
        b'S' => "bend",
        b'C' => "coil",
        b'.' | b' ' => "none",
        _ => match ss_class(b) {
            Ss::PairOpen => "paired (opens)",
            Ss::PairClose => "paired (closes)",
            _ => "unpaired",
        },
    }
}

/// A posterior probability character as 0–1: `0` is 0–5%, `1` 5–15%, … `9` 85–95% (the middle of
/// each range), `*` 95–100%. `None` for anything else (gaps).
pub fn pp_value(b: u8) -> Option<f32> {
    match b {
        b'0' => Some(0.025),
        b'1'..=b'9' => Some((b - b'0') as f32 / 10.0),
        b'*' => Some(0.975),
        _ => None,
    }
}

/// Words for a probability character, for the hover card.
pub fn pp_words(b: u8) -> String {
    match b {
        b'0' => "0–5%".into(),
        b'1'..=b'9' => {
            let d = (b - b'0') as u32 * 10;
            format!("{}–{}%", d - 5, d + 5)
        }
        b'*' => "95–100%".into(),
        _ => "none".into(),
    }
}

/// Residue annotation (`#=GR`) kept per file, at most: lines past it are left out with a warning.
/// One byte per column per line, so a file with a `#=GR PP` line for every sequence holds as much
/// again as a protein alignment's residues.
pub const GR_BUDGET: usize = 64 << 20;

/// Segments are one whitespace-free token of a line (the reader splits lines on whitespace).
///
/// What the first pass of a Stockholm file collects: the `#=GC` lines (joined), and the length
/// of each `#=GR` line (their characters are read in the second pass, once the alignment's width
/// is known, into buffers of exactly that size).
#[derive(Default)]
pub(crate) struct Collector {
    gc: Vec<(Vec<u8>, Vec<u8>)>,
    /// `#=GR` lines by `name \0 tag`, as indices into `gr`.
    gr_index: HashMap<Vec<u8>, usize>,
    /// Name, tag and length so far, in the order of first appearance.
    gr: Vec<(Vec<u8>, Vec<u8>, usize)>,
    key: Vec<u8>,
}

fn gr_key<'k>(key: &'k mut Vec<u8>, name: &[u8], tag: &[u8]) -> &'k [u8] {
    key.clear();
    key.extend_from_slice(name);
    key.push(0);
    key.extend_from_slice(tag);
    key
}


impl Collector {
    pub(crate) fn gc(&mut self, tag: &[u8], seg: &[u8]) {
        let i = match self.gc.iter().position(|(t, _)| t == tag) {
            Some(i) => i,
            None => {
                self.gc.push((tag.to_vec(), Vec::new()));
                self.gc.len() - 1
            }
        };
        self.gc[i].1.extend_from_slice(seg);
    }

    pub(crate) fn gr(&mut self, name: &[u8], tag: &[u8], seg: &[u8]) {
        let k = gr_key(&mut self.key, name, tag);
        let i = match self.gr_index.get(k) {
            Some(&i) => i,
            None => {
                self.gr_index.insert(k.to_vec(), self.gr.len());
                self.gr.push((name.to_vec(), tag.to_vec(), 0));
                self.gr.len() - 1
            }
        };
        self.gr[i].2 += seg.len();
    }

    /// The `#=GC` tracks that cover exactly `cols` columns, the plan for reading the `#=GR`
    /// lines that do (within `budget` bytes), and a warning for each line left out: a different
    /// length, a `#=GR` line for a sequence that isn't in the alignment, lines over the budget.
    pub(crate) fn finish(self, cols: usize, row_of: impl Fn(&[u8]) -> Option<usize>, budget: usize) -> (Vec<Track>, GrFill, Vec<String>) {
        let s = |b: &[u8]| String::from_utf8_lossy(b).into_owned();
        let mut warnings = Vec::new();
        let mut gc = Vec::new();
        for (tag, data) in self.gc {
            if data.len() == cols {
                gc.push(Track { tag: s(&tag).into(), data: data.into() });
            } else {
                warnings.push(format!("#=GC {} has {} columns, the alignment {}; left out", s(&tag), data.len(), cols));
            }
        }
        let mut fill = GrFill::default();
        let (mut used, mut over) = (0usize, 0usize);
        for (name, tag, len) in self.gr {
            let Some(row) = row_of(&name) else {
                warnings.push(format!("#=GR {} {}: no sequence of that name; left out", s(&name), s(&tag)));
                continue;
            };
            if len != cols {
                warnings.push(format!("#=GR {} {} has {} columns, the alignment {}; left out", s(&name), s(&tag), len, cols));
                continue;
            }
            if used + cols > budget {
                over += 1;
                continue;
            }
            used += cols;
            let k = gr_key(&mut fill.key, &name, &tag).to_vec();
            fill.index.insert(k, fill.lines.len());
            fill.lines.push((row as u32, s(&tag).into(), Vec::with_capacity(cols)));
        }
        if over > 0 {
            warnings.push(format!("#=GR: {over} lines left out, over the {} MiB kept for residue annotation", budget >> 20));
        }
        (gc, fill, warnings)
    }
}

/// The second pass over `#=GR` lines: their characters, into buffers of the alignment's width.
#[derive(Default)]
pub(crate) struct GrFill {
    index: HashMap<Vec<u8>, usize>,
    lines: Vec<(u32, Box<str>, Vec<u8>)>,
    key: Vec<u8>,
}

impl GrFill {
    pub(crate) fn add(&mut self, name: &[u8], tag: &[u8], seg: &[u8]) {
        if self.lines.is_empty() {
            return;
        }
        if let Some(&i) = self.index.get(gr_key(&mut self.key, name, tag)) {
            self.lines[i].2.extend_from_slice(seg);
        }
    }

    /// The lines, sorted by row; a line whose length changed between the passes is left out.
    pub(crate) fn finish(self, cols: usize) -> Vec<RowAnnot> {
        let mut gr: Vec<RowAnnot> = self.lines.into_iter().filter(|l| l.2.len() == cols).map(|(row, tag, data)| RowAnnot { row, tag, data: data.into() }).collect();
        gr.sort_by_key(|a| a.row);
        gr
    }
}

/// The `#=GR` lines of alignment row `row` (`gr` sorted by row).
pub fn row_annots(gr: &[RowAnnot], row: usize) -> &[RowAnnot] {
    let a = gr.partition_point(|x| (x.row as usize) < row);
    let b = gr.partition_point(|x| (x.row as usize) <= row);
    &gr[a..b]
}

/// Bytes held by the annotation.
pub fn bytes(gc: &[Track], gr: &[RowAnnot]) -> usize {
    gc.iter().map(|t| t.data.len() + t.tag.len()).sum::<usize>() + gr.iter().map(|t| t.data.len() + t.tag.len() + 4).sum::<usize>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classes() {
        assert_eq!(TrackKind::of("SS_cons"), TrackKind::Structure);
        assert_eq!(TrackKind::of("PP"), TrackKind::Probability);
        assert_eq!(TrackKind::of("seq_cons"), TrackKind::Text);
        let ss: Vec<Ss> = b"HGIEB<>().-T".iter().map(|&b| ss_class(b)).collect();
        use Ss::{Coil, Helix, PairClose, PairOpen, Strand};
        assert_eq!(ss, [Helix, Helix, Helix, Strand, Strand, PairOpen, PairClose, PairOpen, PairClose, Ss::None, Coil, PairOpen]);
        // 'T' (turn) is a capital letter, but means a turn in protein SS lines
        assert_eq!(ss_words(b'T'), "turn");
        assert_eq!(pp_value(b'*'), Some(0.975));
        assert_eq!(pp_value(b'7'), Some(0.7));
        assert_eq!(pp_value(b'.'), None);
        assert_eq!(pp_words(b'9'), "85–95%");
        assert_eq!(pp_words(b'0'), "0–5%");
    }

    #[test]
    fn collector_joins_blocks_and_checks_lengths() {
        let mut c = Collector::default();
        let first: [(&[u8], &[u8], &[u8]); 3] = [(b"a", b"PP", b"98"), (b"a", b"PP", b"7*."), (b"zz", b"SS", b"HHHHH")];
        c.gc(b"SS_cons", b"HHH");
        c.gc(b"seq_cons", b"ACD");
        c.gc(b"SS_cons", b"EE");
        c.gc(b"seq_cons", b"EF");
        c.gc(b"bad", b"xx");
        for (n, t, seg) in first {
            c.gr(n, t, seg);
        }
        let (gc, mut fill, w) = c.finish(5, |n| (n == b"a").then_some(1), GR_BUDGET);
        for (n, t, seg) in first {
            fill.add(n, t, seg);
        }
        let gr = fill.finish(5);
        assert_eq!(gc.len(), 2);
        assert_eq!(&*gc[0].tag, "SS_cons");
        assert_eq!(&*gc[0].data, b"HHHEE");
        assert_eq!(&*gc[1].data, b"ACDEF");
        assert_eq!(gr.len(), 1);
        assert_eq!((gr[0].row, &*gr[0].tag, &*gr[0].data), (1, "PP", &b"987*."[..]));
        assert_eq!(gr[0].data.len(), 5);
        assert_eq!(w.len(), 2);
        assert!(w[0].contains("#=GC bad has 2 columns, the alignment 5"), "{w:?}");
        assert!(w[1].contains("#=GR zz SS"), "{w:?}");
        assert_eq!(row_annots(&gr, 1).len(), 1);
        assert!(row_annots(&gr, 0).is_empty());
    }

    /// Lines past the budget are left out, with one warning that counts them.
    #[test]
    fn residue_annotation_budget() {
        let mut c = Collector::default();
        for n in [&b"a"[..], b"b", b"c"] {
            c.gr(n, b"PP", b"0123");
        }
        let (_, mut fill, w) = c.finish(4, |n| Some(n[0] as usize - b'a' as usize), 9);
        for n in [&b"a"[..], b"b", b"c"] {
            fill.add(n, b"PP", b"0123");
        }
        let gr = fill.finish(4);
        assert_eq!(gr.iter().map(|g| g.row).collect::<Vec<_>>(), [0, 1]);
        assert_eq!(w, ["#=GR: 1 lines left out, over the 0 MiB kept for residue annotation"]);
    }
}
