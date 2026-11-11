//! Sorting and filtering rows: per-row figures over the shown columns, the filter that decides
//! which rows are shown, and the orders rows can be sorted in.
//!
//! Identity is the share of a row's residues that equal the target's residue in the same column,
//! counted over the shown columns where both have a residue; the target is the column consensus
//! or a reference row. Gap share is the share of the shown columns that are gaps in the row.
//! Length is the ungapped sequence length (every column).

use rayon::prelude::*;

use crate::Alignment;
use crate::alphabet::GAP;
use crate::view::{ColumnMap, RowView};

/// What rows can be sorted by, besides the file's and the tree's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortKey {
    /// A to Z.
    Name,
    /// Most identical to the consensus first.
    Identity,
    /// Most identical to the reference row first.
    Reference,
    /// Fewest gaps first.
    Gaps,
    /// Longest ungapped sequence first.
    Length,
}

impl SortKey {
    pub const ALL: [SortKey; 5] = [SortKey::Name, SortKey::Identity, SortKey::Reference, SortKey::Gaps, SortKey::Length];

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "Name",
            SortKey::Identity => "Identity to consensus",
            SortKey::Reference => "Identity to reference",
            SortKey::Gaps => "Gap share",
            SortKey::Length => "Length",
        }
    }

    /// Whether the key needs [`RowStats`].
    pub fn needs_stats(self) -> bool {
        self != SortKey::Name
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sort {
    pub key: SortKey,
    /// The other way round (Z to A, least identical first, ...).
    pub reverse: bool,
}

/// A name filter: a case-insensitive substring, or a regular expression written `/…/`.
#[derive(Clone, Debug)]
pub enum NameQuery {
    Substring(String),
    Regex(regex::Regex),
}

impl NameQuery {
    /// `None` for empty text; an error message for a bad regular expression.
    pub fn parse(text: &str) -> Result<Option<NameQuery>, String> {
        let t = text.trim();
        if t.is_empty() {
            return Ok(None);
        }
        if let Some(re) = t.strip_prefix('/').and_then(|r| r.strip_suffix('/')).filter(|r| !r.is_empty()) {
            return regex::RegexBuilder::new(re)
                .case_insensitive(true)
                .size_limit(1 << 22)
                .build()
                .map(|r| Some(NameQuery::Regex(r)))
                .map_err(|e| format!("Bad regular expression: {}", e.to_string().lines().last().unwrap_or("")));
        }
        Ok(Some(NameQuery::Substring(t.to_lowercase())))
    }

    /// Whether the name or the accession matches.
    pub fn matches(&self, name: &str, accession: Option<&str>) -> bool {
        let one = |s: &str| match self {
            NameQuery::Substring(q) => {
                if s.is_ascii() && q.is_ascii() {
                    s.as_bytes().windows(q.len()).any(|w| w.eq_ignore_ascii_case(q.as_bytes()))
                } else {
                    s.to_lowercase().contains(q.as_str())
                }
            }
            NameQuery::Regex(r) => r.is_match(s),
        };
        one(name) || accession.is_some_and(one)
    }
}

/// Which rows are shown. Every condition given must hold.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    pub name: Option<NameQuery>,
    /// Minimum identity (0–1) to the consensus, or to the reference row when one is set.
    pub min_identity: Option<f32>,
    /// Maximum gap share (0–1) over the shown columns.
    pub max_gaps: Option<f32>,
    /// Only these alignment rows (a tree clade), sorted.
    pub clade: Option<std::sync::Arc<Vec<u32>>>,
}

impl Filter {
    pub fn is_active(&self) -> bool {
        self.name.is_some() || self.min_identity.is_some() || self.max_gaps.is_some() || self.clade.is_some()
    }

    pub fn needs_stats(&self) -> bool {
        self.min_identity.is_some() || self.max_gaps.is_some()
    }

    /// One flag per alignment row; `None` when nothing is filtered. `stats` must be given when
    /// [`Filter::needs_stats`].
    pub fn keep(&self, aln: &Alignment, stats: Option<&RowStats>) -> Option<Vec<bool>> {
        if !self.is_active() {
            return None;
        }
        let ident = stats.map(|s| s.reference.as_deref().unwrap_or(&s.identity));
        let mut keep: Vec<bool> = (0..aln.rows())
            .into_par_iter()
            .map(|r| {
                self.name.as_ref().is_none_or(|q| q.matches(&aln.names[r], aln.accession(r)))
                    && self.min_identity.is_none_or(|m| ident.is_some_and(|i| i[r] >= m - 1e-6))
                    && self.max_gaps.is_none_or(|m| stats.is_some_and(|s| s.gaps[r] <= m + 1e-6))
            })
            .collect();
        if let Some(c) = &self.clade {
            let mut inside = vec![false; aln.rows()];
            for &r in c.iter() {
                inside[r as usize] = true;
            }
            keep.iter_mut().zip(inside).for_each(|(k, i)| *k &= i);
        }
        Some(keep)
    }
}

/// Per-row figures over the shown columns.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowStats {
    /// Ungapped length (all columns).
    pub residues: Vec<u32>,
    /// Gap share of the shown columns, 0–1.
    pub gaps: Vec<f32>,
    /// Identity to the column consensus, 0–1.
    pub identity: Vec<f32>,
    /// Identity to the reference row, when one was given.
    pub reference: Option<Vec<f32>>,
}

impl RowStats {
    /// Figures for every row over the columns `map` shows (all without a map), comparing with
    /// `consensus` and, when given, alignment row `reference`. Rows are done in parallel.
    pub fn compute(aln: &Alignment, consensus: &[u8], map: Option<&ColumnMap>, reference: Option<usize>) -> RowStats {
        let cols = aln.cols();
        let shown: Vec<u32> = map.map_or_else(|| (0..cols as u32).collect(), |m| m.visible.clone());
        let refrow: Option<Vec<u8>> = reference.map(|r| {
            let mut b = vec![0u8; cols];
            aln.matrix.decode_row(r, 0, &mut b);
            b
        });
        let n = shown.len().max(1) as f32;
        let per: Vec<(u32, f32, f32, f32)> = (0..aln.rows())
            .into_par_iter()
            .map_init(
                || vec![0u8; cols],
                |buf, r| {
                    aln.matrix.decode_row(r, 0, buf);
                    let residues = buf.iter().filter(|&&c| c != GAP).count() as u32;
                    let (mut gaps, mut same, mut both, mut rsame, mut rboth) = (0u32, 0u32, 0u32, 0u32, 0u32);
                    for &c in &shown {
                        let code = buf[c as usize];
                        if code == GAP {
                            gaps += 1;
                            continue;
                        }
                        let k = consensus[c as usize];
                        if k != GAP {
                            both += 1;
                            same += (k == code) as u32;
                        }
                        if let Some(rr) = &refrow {
                            let k = rr[c as usize];
                            if k != GAP {
                                rboth += 1;
                                rsame += (k == code) as u32;
                            }
                        }
                    }
                    let share = |a: u32, b: u32| if b == 0 { 0.0 } else { a as f32 / b as f32 };
                    (residues, gaps as f32 / n, share(same, both), share(rsame, rboth))
                },
            )
            .collect();
        RowStats {
            residues: per.iter().map(|p| p.0).collect(),
            gaps: per.iter().map(|p| p.1).collect(),
            identity: per.iter().map(|p| p.2).collect(),
            reference: reference.map(|_| per.iter().map(|p| p.3).collect()),
        }
    }
}

/// `rows` sorted by `sort` (stable: ties keep their order in `rows`). `stats` must be given for
/// keys that [`SortKey::needs_stats`].
pub fn sort_rows(aln: &Alignment, mut rows: Vec<u32>, sort: Sort, stats: Option<&RowStats>) -> Vec<u32> {
    use std::cmp::Ordering;
    let f = |v: &[f32], a: u32, b: u32| v[a as usize].total_cmp(&v[b as usize]);
    let cmp = |a: &u32, b: &u32| -> Ordering {
        let (a, b) = (*a, *b);
        let o = match (sort.key, stats) {
            (SortKey::Name, _) => natural(&aln.names[a as usize], &aln.names[b as usize]),
            (SortKey::Identity, Some(s)) => f(&s.identity, b, a),
            (SortKey::Reference, Some(s)) => f(s.reference.as_deref().unwrap_or(&s.identity), b, a),
            (SortKey::Gaps, Some(s)) => f(&s.gaps, a, b),
            (SortKey::Length, Some(s)) => s.residues[b as usize].cmp(&s.residues[a as usize]),
            _ => Ordering::Equal,
        };
        if sort.reverse { o.reverse() } else { o }
    };
    rows.par_sort_by(cmp);
    rows
}

/// Case-insensitive order in which digit runs compare as numbers (`seq2` before `seq10`).
pub fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let (si, sj) = (i, j);
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let (x, y) = (trim_zeros(&a[si..i]), trim_zeros(&b[sj..j]));
            let o = x.len().cmp(&y.len()).then_with(|| x.cmp(y));
            if o != Ordering::Equal {
                return o;
            }
        } else {
            let o = a[i].to_ascii_lowercase().cmp(&b[j].to_ascii_lowercase());
            if o != Ordering::Equal {
                return o;
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j)).then_with(|| a.cmp(b))
}

fn trim_zeros(d: &[u8]) -> &[u8] {
    let k = d.iter().position(|&c| c != b'0').unwrap_or(d.len().saturating_sub(1));
    &d[k..]
}

/// The rows shown in file order (`keep`), or sorted (`sort`), as a row view without clades.
pub fn flat_view(aln: &Alignment, keep: Option<&[bool]>, sort: Option<Sort>, stats: Option<&RowStats>) -> RowView {
    let rows: Vec<u32> = (0..aln.rows() as u32).filter(|&r| keep.is_none_or(|k| k[r as usize])).collect();
    let rows = match sort {
        Some(s) => sort_rows(aln, rows, s, stats),
        None => rows,
    };
    let n = rows.len() as u32;
    RowView::new(rows, aln.rows(), n, Vec::new(), Vec::new(), Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_bytes;

    fn aln() -> Alignment {
        // Consensus: A C G T A (row 0 equals it). Row 3 has an accession-like name.
        parse_bytes(b">seq10/1-5\nACGTA\n>seq2\nAC-TT\n>Seq1\n--GTA\n>x_P12345\nTTTTT\n").unwrap()
    }

    #[test]
    fn natural_order() {
        let mut v = vec!["seq10", "Seq1", "seq2", "seq02a", "a"];
        v.sort_by(|a, b| natural(a, b));
        assert_eq!(v, ["a", "Seq1", "seq2", "seq02a", "seq10"]);
    }

    #[test]
    fn stats_over_shown_columns() {
        let a = aln();
        let cons = crate::stats::ColumnStats::compute(&a).consensus;
        let s = RowStats::compute(&a, &cons, None, Some(3));
        assert_eq!(s.residues, [5, 4, 3, 5]);
        assert_eq!(s.gaps, [0.0, 0.2, 0.4, 0.0]);
        assert_eq!(s.identity[0], 1.0);
        assert_eq!(s.identity[1], 0.75); // A C T match, T vs A not
        assert_eq!(s.reference.as_ref().unwrap()[3], 1.0);
        assert_eq!(s.reference.as_ref().unwrap()[0], 0.2);
        // Hiding columns 0–1 changes the gap share of row 2.
        let m = ColumnMap { visible: vec![2, 3, 4] };
        let s = RowStats::compute(&a, &cons, Some(&m), None);
        assert_eq!(s.gaps[2], 0.0);
        assert_eq!(s.gaps[1], 1.0 / 3.0);
    }

    #[test]
    fn sorts() {
        let a = aln();
        let cons = crate::stats::ColumnStats::compute(&a).consensus;
        let s = RowStats::compute(&a, &cons, None, Some(3));
        let all = || (0..4u32).collect::<Vec<_>>();
        let by = |key, reverse| sort_rows(&a, all(), Sort { key, reverse }, Some(&s));
        assert_eq!(by(SortKey::Name, false), [2, 1, 0, 3]);
        assert_eq!(by(SortKey::Name, true), [3, 0, 1, 2]);
        assert_eq!(by(SortKey::Gaps, false), [0, 3, 1, 2]);
        assert_eq!(by(SortKey::Length, false), [0, 3, 1, 2]);
        assert_eq!(by(SortKey::Identity, false)[0], 0);
        assert_eq!(by(SortKey::Reference, false)[0], 3);
    }

    #[test]
    fn filters_compose() {
        let a = aln();
        let cons = crate::stats::ColumnStats::compute(&a).consensus;
        let s = RowStats::compute(&a, &cons, None, None);
        let f = Filter { name: NameQuery::parse("SEQ").unwrap(), ..Default::default() };
        assert_eq!(f.keep(&a, None).unwrap(), [true, true, true, false]);
        let f = Filter { name: NameQuery::parse("/^seq\\d$/").unwrap(), max_gaps: Some(0.2), ..Default::default() };
        assert_eq!(f.keep(&a, Some(&s)).unwrap(), [false, true, false, false]);
        let f = Filter { min_identity: Some(0.9), ..Default::default() };
        assert_eq!(f.keep(&a, Some(&s)).unwrap(), [true, false, true, false]);
        let f = Filter { clade: Some(std::sync::Arc::new(vec![1, 3])), name: NameQuery::parse("p123").unwrap(), ..Default::default() };
        assert_eq!(f.keep(&a, None).unwrap(), [false, false, false, true]);
        assert!(Filter::default().keep(&a, None).is_none());
        assert!(NameQuery::parse("/[/").is_err());
        assert!(NameQuery::parse("  ").unwrap().is_none());
    }

    #[test]
    fn flat_view_hides_and_sorts() {
        let a = aln();
        let keep = [true, false, true, true];
        let v = flat_view(&a, Some(&keep), Some(Sort { key: SortKey::Name, reverse: false }), None);
        assert_eq!(v.rows, [2, 0, 3]);
        assert_eq!(v.shown(1), None);
        assert_eq!(v.shown(3), Some(2));
        assert_eq!(v.expanded(), [2, 0, 3]);
    }

    /// Export and copy take what is shown: a filter's hidden rows drop out of a selection, the
    /// sort order is kept, and hidden columns stay out.
    #[test]
    fn export_uses_what_is_shown() {
        use crate::select::{Extract, Selection};
        let a = aln();
        let cons = crate::stats::ColumnStats::compute(&a).consensus;
        let map = ColumnMap { visible: vec![0, 2, 3, 4] };
        let s = RowStats::compute(&a, &cons, Some(&map), None);
        // Gaps over shown columns: row 1 (AC-TT) has 1 of 4, row 2 (--GTA) 1 of 4.
        let f = Filter { max_gaps: Some(0.0), ..Default::default() };
        let keep = f.keep(&a, Some(&s)).unwrap();
        assert_eq!(keep, [true, false, false, true]);
        let v = flat_view(&a, Some(&keep), Some(Sort { key: SortKey::Name, reverse: true }), None);
        assert_eq!(v.rows, [3, 0]);
        let mut sel = Selection::default();
        sel.rows.add(0, 4); // every row selected, two of them hidden
        let e = Extract::new(&a, Some(&sel), Some(&v), Some(&map), false);
        assert_eq!(e.rows, [3, 0]);
        assert_eq!(e.cols, [0, 2, 3, 4]);
        // In tree order with the same filter.
        let t = crate::tree::Tree::parse("((Seq1,seq2),(seq10/1-5,x_P12345));").unwrap();
        let ix = crate::tree::TreeIndex::new(t, &a.names, &[]);
        let (tv, tr) = crate::tree::row_view_kept(&a, &ix, &vec![false; ix.tree.len()], Some(&keep));
        assert_eq!(tv.expanded(), [0, 3]);
        assert!(tr.unwrap().y[1].is_nan()); // the clade of Seq1 and seq2 is gone
        assert_eq!(Extract::new(&a, Some(&sel), Some(&tv), Some(&map), false).rows, [0, 3]);
    }
}
