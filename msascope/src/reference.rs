//! A reference row: one sequence the ruler can count by, and that Differences, the identity track
//! and the density view can compare with instead of the consensus.

use crate::Alignment;
use crate::alphabet::{GAP, NCODES};
use crate::stats::ColumnStats;
use crate::view::{ColumnMap, aln_col};

pub struct Reference {
    /// Alignment row.
    pub row: usize,
    /// The row's code in every alignment column.
    pub codes: Vec<u8>,
    /// Residues of the row before each column (`cols + 1` entries; the last is the length).
    before: Vec<u32>,
    /// The number of its first residue: the start of the name's `/start-end`, else 1.
    pub start: usize,
}

impl Reference {
    pub fn new(aln: &Alignment, row: usize) -> Reference {
        let mut codes = vec![GAP; aln.cols()];
        aln.matrix.decode_row(row, 0, &mut codes);
        let mut before = Vec::with_capacity(codes.len() + 1);
        let mut n = 0u32;
        for &c in &codes {
            before.push(n);
            n += (c != GAP) as u32;
        }
        before.push(n);
        Reference {
            row,
            codes,
            before,
            start: crate::name_range(&aln.names[row]).map_or(1, |(a, _)| a),
        }
    }

    /// Residues in the row.
    pub fn len(&self) -> usize {
        *self.before.last().unwrap() as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The reference's own number of its residue in alignment column `c` (from its `/start`);
    /// `None` where the reference has a gap.
    pub fn position(&self, c: usize) -> Option<usize> {
        (self.codes[c] != GAP).then(|| self.start + self.before[c] as usize)
    }

    /// The alignment column of the reference's residue numbered `pos`; `None` outside its range.
    pub fn column_of(&self, pos: usize) -> Option<usize> {
        let i = pos.checked_sub(self.start)?;
        if i >= self.len() {
            return None;
        }
        // The first column with i + 1 residues up to and including it.
        Some(self.before.partition_point(|&b| (b as usize) <= i) - 1)
    }

    /// Share of all rows carrying the reference's residue in column `c`, from the column's
    /// shares (to 1/255); 0 where the reference has a gap.
    pub fn identity(&self, stats: &ColumnStats, c: usize) -> f32 {
        let code = self.codes[c];
        if code == GAP {
            0.0
        } else {
            stats.shares[c][code as usize & (NCODES - 1)] as f32 / 255.0
        }
    }

    pub fn bytes(&self) -> usize {
        self.codes.len() + self.before.len() * 4
    }
}

/// Ruler marks counting the reference's residues, for view columns `range`: `(view column,
/// number, major)` where the reference has a residue whose number is a multiple of `step`
/// (major: labelled) or of `minor` (a minor tick; 0 for none), and, with `first`, its first
/// residue in the range as a major mark.
pub fn ruler_marks(
    r: &Reference,
    map: Option<&ColumnMap>,
    range: std::ops::Range<usize>,
    step: usize,
    minor: usize,
    first: bool,
) -> Vec<(usize, usize, bool)> {
    let mut out = Vec::new();
    let mut first = first;
    for v in range {
        let Some(p) = r.position(aln_col(map, v)) else {
            continue;
        };
        if p.is_multiple_of(step) || first {
            out.push((v, p, true));
        } else if minor > 0 && p.is_multiple_of(minor) {
            out.push((v, p, false));
        }
        first = false;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_bytes;

    fn aln() -> Alignment {
        // columns:         0123456789
        parse_bytes(
            b"# STOCKHOLM 1.0\nR/11-16 -AC..DE-FG\nx       AAC..DQ-FG\ny       -AT.aDE-F-\n//\n",
        )
        .unwrap()
    }

    #[test]
    fn positions_follow_the_name_range() {
        let a = aln();
        let r = Reference::new(&a, 0);
        assert_eq!((r.start, r.len()), (11, 6));
        assert_eq!(r.position(0), None);
        assert_eq!(r.position(1), Some(11));
        assert_eq!(r.position(5), Some(13));
        assert_eq!(r.position(9), Some(16));
        assert_eq!(r.position(7), None);
        for pos in 11..=16 {
            assert_eq!(r.position(r.column_of(pos).unwrap()), Some(pos));
        }
        assert_eq!(r.column_of(10), None);
        assert_eq!(r.column_of(17), None);
        // without a range, from 1
        let r = Reference::new(&a, 1);
        assert_eq!(
            (r.start, r.position(0), r.column_of(3)),
            (1, Some(1), Some(2))
        );
    }

    #[test]
    fn identity_to_the_reference() {
        let a = aln();
        let s = ColumnStats::compute(&a);
        let r = Reference::new(&a, 0);
        // column 2: C, C, T → 2 of 3; column 6: E, Q, E → 2 of 3; column 0: gap → 0
        let near = |x: f32, y: f32| (x - y).abs() < 1.0 / 255.0 + 1e-6;
        assert!(near(r.identity(&s, 2), 2.0 / 3.0));
        assert!(near(r.identity(&s, 6), 2.0 / 3.0));
        assert_eq!(r.identity(&s, 0), 0.0);
        assert!(near(r.identity(&s, 1), 1.0));
    }

    #[test]
    fn ruler_counts_reference_residues_through_the_column_map() {
        let a = aln();
        let s = ColumnStats::compute(&a);
        let r = Reference::new(&a, 0);
        // every 2nd residue: 12 (col 2), 14 (col 6), 16 (col 9)
        let majors = |m: Vec<(usize, usize, bool)>| {
            m.into_iter()
                .filter(|x| x.2)
                .map(|x| (x.0, x.1))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            majors(ruler_marks(&r, None, 0..a.cols(), 2, 0, false)),
            [(2, 12), (6, 14), (9, 16)]
        );
        // with the first residue in view
        assert_eq!(
            majors(ruler_marks(&r, None, 3..a.cols(), 2, 0, true)),
            [(5, 13), (6, 14), (9, 16)]
        );
        // labels every 4th, minor ticks every 2nd
        assert_eq!(
            ruler_marks(&r, None, 0..a.cols(), 4, 2, false),
            [(2, 12, true), (6, 14, false), (9, 16, true)]
        );
        // inserts (columns 3 and 4) hidden: the same residues, at view columns shifted by 2
        let m = ColumnMap::hiding(&a, &s, true, None).unwrap();
        assert_eq!(m.visible, [0, 1, 2, 5, 6, 7, 8, 9]);
        assert_eq!(
            majors(ruler_marks(&r, Some(&m), 0..m.len(), 2, 0, false)),
            [(2, 12), (4, 14), (7, 16)]
        );
    }
}
