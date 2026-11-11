//! Selections (rows, columns, blocks) in alignment coordinates, writing them out as FASTA or plain
//! text, and moving the cell cursor.

use std::io::{self, Write};

use crate::Alignment;
use crate::alphabet::GAP;
use crate::view::{ColumnMap, RowSrc, RowView, row_src};

/// A set of `u32` as sorted, disjoint, non-touching half-open ranges.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RangeSet {
    r: Vec<(u32, u32)>,
}

impl RangeSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// From values in ascending order (duplicates allowed).
    pub fn from_sorted(it: impl IntoIterator<Item = u32>) -> Self {
        let mut r: Vec<(u32, u32)> = Vec::new();
        for x in it {
            match r.last_mut() {
                Some(l) if x <= l.1 => l.1 = l.1.max(x + 1),
                _ => r.push((x, x + 1)),
            }
        }
        RangeSet { r }
    }

    pub fn ranges(&self) -> &[(u32, u32)] {
        &self.r
    }

    pub fn is_empty(&self) -> bool {
        self.r.is_empty()
    }

    /// Number of values.
    pub fn len(&self) -> usize {
        self.r.iter().map(|&(a, b)| (b - a) as usize).sum()
    }

    pub fn clear(&mut self) {
        self.r.clear();
    }

    pub fn first(&self) -> Option<u32> {
        self.r.first().map(|r| r.0)
    }

    pub fn last(&self) -> Option<u32> {
        self.r.last().map(|r| r.1 - 1)
    }

    #[inline]
    pub fn contains(&self, x: u32) -> bool {
        let i = self.r.partition_point(|r| r.1 <= x);
        self.r.get(i).is_some_and(|r| r.0 <= x)
    }

    /// Add `a..b`.
    pub fn add(&mut self, a: u32, b: u32) {
        if a >= b {
            return;
        }
        // Ranges that touch or overlap a..b merge with it.
        let i = self.r.partition_point(|r| r.1 < a);
        let j = self.r.partition_point(|r| r.0 <= b);
        let (lo, hi) = if i < j { (self.r[i].0.min(a), self.r[j - 1].1.max(b)) } else { (a, b) };
        self.r.splice(i..j, [(lo, hi)]);
    }

    /// Remove `a..b`.
    pub fn remove(&mut self, a: u32, b: u32) {
        if a >= b {
            return;
        }
        let mut out = Vec::with_capacity(self.r.len() + 1);
        for &(x, y) in &self.r {
            if y <= a || x >= b {
                out.push((x, y));
                continue;
            }
            if x < a {
                out.push((x, a));
            }
            if y > b {
                out.push((b, y));
            }
        }
        self.r = out;
    }

    /// Add `x` if absent, remove it if present.
    pub fn toggle(&mut self, x: u32) {
        if self.contains(x) { self.remove(x, x + 1) } else { self.add(x, x + 1) }
    }

    /// Add every value of another set.
    pub fn union(&mut self, other: &RangeSet) {
        for &(a, b) in &other.r {
            self.add(a, b);
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        self.r.iter().flat_map(|&(a, b)| a..b)
    }
}

/// What is selected, in alignment rows and columns. A cell is selected when its row and column
/// are: rows only select whole rows, columns only whole columns, both a block.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    pub rows: RangeSet,
    pub cols: RangeSet,
}

impl Selection {
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty() && self.cols.is_empty()
    }

    pub fn clear(&mut self) {
        self.rows.clear();
        self.cols.clear();
    }

    /// Whether row `r` has selected cells.
    pub fn row_selected(&self, r: u32) -> bool {
        !self.is_empty() && (self.rows.is_empty() || self.rows.contains(r))
    }

    pub fn col_selected(&self, c: u32) -> bool {
        !self.is_empty() && (self.cols.is_empty() || self.cols.contains(c))
    }

    pub fn cell_selected(&self, r: u32, c: u32) -> bool {
        self.row_selected(r) && self.col_selected(c)
    }

    /// "1 row", "3 rows × 12 columns", "12 columns": the size, given the alignment's.
    pub fn describe(&self, rows: usize, cols: usize) -> String {
        let g = crate::view::group_digits;
        let n = |k: usize, one: &str, many: &str| format!("{} {}", g(k), if k == 1 { one } else { many });
        match (self.rows.is_empty(), self.cols.is_empty()) {
            (true, true) => String::new(),
            (false, true) => n(self.rows.len(), "row", "rows"),
            (true, false) => n(self.cols.len(), "column", "columns"),
            (false, false) if self.rows.len() == rows && self.cols.len() == cols => "all".into(),
            (false, false) => format!("{} × {}", n(self.rows.len(), "row", "rows"), n(self.cols.len(), "column", "columns")),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExportOptions {
    /// Write the hidden columns inside the selection too (default: only what is shown).
    pub include_hidden: bool,
    /// Leave out gaps, giving the ungapped sequences.
    pub strip_gaps: bool,
    /// `>name` lines (FASTA); otherwise residues only, one row per line.
    pub fasta: bool,
}

/// What gets written: rows in view order, and alignment columns.
pub struct Extract {
    pub rows: Vec<u32>,
    pub cols: Vec<u32>,
}

impl Extract {
    /// The selected rows in the order shown (`rows` view, clades expanded; file order without a
    /// view) and the selected columns, minus hidden ones unless asked. `sel` `None`: everything.
    pub fn new(aln: &Alignment, sel: Option<&Selection>, view: Option<&RowView>, map: Option<&ColumnMap>, include_hidden: bool) -> Extract {
        let order: Vec<u32> = view.map_or_else(|| (0..aln.rows() as u32).collect(), RowView::expanded);
        let rows = order.into_iter().filter(|&r| sel.is_none_or(|s| s.row_selected(r))).collect();
        let shown = |c: u32| include_hidden || map.is_none_or(|m| m.view_of(c as usize).is_ok());
        let cols = (0..aln.cols() as u32).filter(|&c| sel.is_none_or(|s| s.col_selected(c)) && shown(c)).collect();
        Extract { rows, cols }
    }

    /// Bytes [`Extract::write`] produces, at most (gaps not stripped).
    pub fn size(&self, aln: &Alignment, fasta: bool) -> usize {
        let names: usize = if fasta { self.rows.iter().map(|&r| aln.names[r as usize].len() + 2).sum() } else { 0 };
        names + self.rows.len() * (self.cols.len() + 1)
    }

    pub fn write(&self, aln: &Alignment, opts: ExportOptions, out: &mut impl Write) -> io::Result<()> {
        let mut buf = vec![0u8; aln.cols()];
        let mut line = Vec::with_capacity(self.cols.len() + 1);
        for &r in &self.rows {
            if opts.fasta {
                writeln!(out, ">{}", aln.names[r as usize])?;
            }
            RowSrc::Seq(r as usize).decode(aln, &mut buf);
            line.clear();
            for &c in &self.cols {
                let code = buf[c as usize];
                if code == GAP {
                    if !opts.strip_gaps {
                        line.push(b'-');
                    }
                } else {
                    line.push(aln.symbol(code));
                }
            }
            line.push(b'\n');
            out.write_all(&line)?;
        }
        Ok(())
    }
}

/// Copying more than this goes to a file instead of the clipboard.
pub const CLIPBOARD_MAX: usize = 64 << 20;

/// Move a cursor at (row, column) by (dr, dc), staying inside `rows` × `cols`.
pub fn step(cur: (usize, usize), d: (isize, isize), rows: usize, cols: usize) -> (usize, usize) {
    if rows == 0 || cols == 0 {
        return (0, 0);
    }
    let mv = |x: usize, dx: isize, n: usize| (x as isize).saturating_add(dx).clamp(0, n as isize - 1) as usize;
    (mv(cur.0, d.0, rows), mv(cur.1, d.1, cols))
}

/// From view column `v` of view row `row`, the next view column in direction `dir` (±1) that
/// starts a run of residues, or the edge when there is none (Ctrl+arrow).
pub fn next_residue(aln: &Alignment, view: Option<&RowView>, map: Option<&ColumnMap>, row: usize, v: usize, dir: isize, cols: usize) -> usize {
    if cols == 0 {
        return 0;
    }
    let src = row_src(view, row);
    let res = |v: usize| src.get(aln, crate::view::aln_col(map, v)) != GAP;
    let mut i = v as isize;
    let end = if dir > 0 { cols as isize - 1 } else { 0 };
    // Skip the rest of the current run, then the gaps after it.
    while i != end && res(i as usize) {
        i += dir;
    }
    while i != end && !res(i as usize) {
        i += dir;
    }
    if dir < 0 && i != end {
        // Moving left: to the start of the run found.
        while i > 0 && res(i as usize - 1) {
            i -= 1;
        }
    }
    i as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_bytes;
    use crate::stats::ColumnStats;

    #[test]
    fn range_set_ops() {
        let mut s = RangeSet::new();
        s.add(5, 8);
        s.add(1, 2);
        s.add(8, 10); // touches: merges
        assert_eq!(s.ranges(), [(1, 2), (5, 10)]);
        s.add(0, 6);
        assert_eq!(s.ranges(), [(0, 10)]);
        s.remove(3, 5);
        assert_eq!(s.ranges(), [(0, 3), (5, 10)]);
        assert!(s.contains(2) && !s.contains(3) && s.contains(9) && !s.contains(10));
        s.toggle(4);
        s.toggle(9);
        assert_eq!(s.ranges(), [(0, 3), (4, 9)]);
        assert_eq!(s.len(), 8);
        assert_eq!((s.first(), s.last()), (Some(0), Some(8)));
        assert_eq!(RangeSet::from_sorted([1, 2, 2, 3, 7, 9, 10]).ranges(), [(1, 4), (7, 8), (9, 11)]);
        let mut u = RangeSet::from_sorted([20]);
        u.union(&s);
        assert_eq!(u.iter().count(), 9);
        s.add(4, 4); // empty: no change
        s.remove(100, 200);
        assert_eq!(s.len(), 8);
    }

    #[test]
    fn selection_model() {
        let mut s = Selection::default();
        assert!(!s.cell_selected(0, 0));
        s.rows.add(1, 3);
        assert!(s.cell_selected(2, 99) && !s.cell_selected(0, 0)); // whole rows
        assert_eq!(s.describe(10, 100), "2 rows");
        s.cols.add(4, 5);
        assert!(s.cell_selected(2, 4) && !s.cell_selected(2, 5)); // block
        assert_eq!(s.describe(10, 100), "2 rows × 1 column");
        s.rows.clear();
        assert!(s.cell_selected(9, 4)); // whole column
        assert_eq!(s.describe(10, 100), "1 column");
    }

    /// FASTA of a block, with and without the hidden columns inside it, gaps kept or stripped,
    /// rows in the order shown.
    #[test]
    fn export_respects_hidden_columns() {
        let aln = parse_bytes(b"# STOCKHOLM 1.0\na/1-5 AC.gT-D\nb/2-4 -C..TGD\nc/1-3 MA..-KD\n//\n").unwrap();
        let stats = ColumnStats::compute(&aln);
        // columns 2 and 3 are inserts; hide them
        let map = ColumnMap::hiding(&aln, &stats, true, None).unwrap();
        assert_eq!(map.visible, [0, 1, 4, 5, 6]);
        let mut sel = Selection::default();
        sel.rows.add(0, 2);
        sel.cols.add(1, 6); // columns 2..6 in the alignment's numbering, two of them hidden
        let text = |sel: Option<&Selection>, view: Option<&RowView>, opts: ExportOptions| {
            let e = Extract::new(&aln, sel, view, Some(&map), opts.include_hidden);
            let mut out = Vec::new();
            e.write(&aln, opts, &mut out).unwrap();
            assert!(out.len() <= e.size(&aln, opts.fasta));
            String::from_utf8(out).unwrap()
        };
        let fasta = ExportOptions { fasta: true, ..Default::default() };
        assert_eq!(text(Some(&sel), None, fasta), ">a/1-5\nCT-\n>b/2-4\nCTG\n");
        assert_eq!(text(Some(&sel), None, ExportOptions { include_hidden: true, ..fasta }), ">a/1-5\nC-GT-\n>b/2-4\nC--TG\n");
        assert_eq!(text(Some(&sel), None, ExportOptions { strip_gaps: true, ..Default::default() }), "CT\nCTG\n");
        // everything, rows in a tree order (c, a, b)
        let view = RowView::new(vec![2, 0, 1], 3, 3, Vec::new(), Vec::new(), Vec::new());
        assert_eq!(text(None, Some(&view), ExportOptions::default()), "MA-KD\nACT-D\n-CTGD\n");
        // columns only: every row
        let mut cols = Selection::default();
        cols.cols.add(6, 7);
        assert_eq!(text(Some(&cols), None, ExportOptions::default()), "D\nD\nD\n");
    }

    #[test]
    fn copy_size_guard() {
        let aln = parse_bytes(b">name\nACGT\n>n2\nAC-T\n").unwrap();
        let e = Extract::new(&aln, None, None, None, false);
        assert_eq!(e.size(&aln, true), (4 + 2) + (2 + 2) + 2 * 5);
        assert_eq!(e.size(&aln, false), 10);
        assert!(e.size(&aln, true) < CLIPBOARD_MAX);
    }

    #[test]
    fn cursor_clamps_and_jumps() {
        assert_eq!(step((0, 0), (-1, -1), 10, 20), (0, 0));
        assert_eq!(step((9, 19), (1, 1), 10, 20), (9, 19));
        assert_eq!(step((5, 5), (100, -100), 10, 20), (9, 0));
        assert_eq!(step((5, 5), (isize::MIN, isize::MAX), 10, 20), (0, 19));
        assert_eq!(step((3, 3), (0, 0), 0, 0), (0, 0));
        //                      0123456789
        let aln = parse_bytes(b">a\nAC--GT---T\n").unwrap();
        let n = aln.cols();
        assert_eq!(next_residue(&aln, None, None, 0, 0, 1, n), 4); // over "AC" and the gap to G
        assert_eq!(next_residue(&aln, None, None, 0, 4, 1, n), 9);
        assert_eq!(next_residue(&aln, None, None, 0, 9, 1, n), 9); // at the edge
        assert_eq!(next_residue(&aln, None, None, 0, 9, -1, n), 4); // back to the start of "GT"
        assert_eq!(next_residue(&aln, None, None, 0, 4, -1, n), 0);
        assert_eq!(next_residue(&aln, None, None, 0, 2, 1, n), 4); // from a gap
        // through a column map: column 4 (G) hidden, so from view 0 the next run starts at view 4 (T)
        let map = ColumnMap::build(n, |c| c == 4).unwrap();
        assert_eq!(next_residue(&aln, None, Some(&map), 0, 0, 1, map.len()), 4);
    }
}
