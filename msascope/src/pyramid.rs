//! Downsampled summaries for zoomed-out views. Level `k` covers blocks of `8 << k` × `8 << k` cells
//! and stores, per block, the most common code and the share of the block's cells (gaps included)
//! holding a residue that differs from its column's consensus (N/X never count). Level 0 is counted
//! from the cells; higher levels combine 2×2 blocks of the level below, each weighted by the cells it
//! covers (so short edge blocks count less): the majority is the majority code with the most weight,
//! which approximates the true majority, and the difference share is exact up to rounding. Ties go to
//! a residue rather than the gap, then to the lower code.

use rayon::prelude::*;

use crate::Alignment;
use crate::alphabet::{GAP, Kind, NCODES};
use crate::view::{RowView, row_src, view_rows};

pub const BASE_SHIFT: u32 = 3;

pub struct Level {
    pub shift: u32,
    pub rows: usize,
    pub cols: usize,
    pub majority: Vec<u8>,
    pub diff: Vec<u8>,
}

impl Level {
    #[inline]
    pub fn at(&self, br: usize, bc: usize) -> (u8, u8) {
        let i = br * self.cols + bc;
        (self.majority[i], self.diff[i])
    }

    pub fn bytes(&self) -> usize {
        self.majority.len() + self.diff.len()
    }
}

pub struct Pyramid {
    pub levels: Vec<Level>,
}

/// Codes that mean "no information" and never count as a difference.
pub fn missing_code(kind: Kind) -> u8 {
    match kind {
        Kind::Nucleotide => 15, // N
        Kind::Protein => 23,    // X
    }
}

impl Pyramid {
    pub fn build(aln: &Alignment, consensus: &[u8]) -> Self {
        Self::build_mapped(aln, consensus, None, None)
    }

    /// The pyramid of the alignment with only the columns in `map` (ascending alignment columns),
    /// as if the others had been removed, and the rows of `rows` in its order (clades as their
    /// consensus rows); `None` keeps every column / the file's rows.
    pub fn build_mapped(
        aln: &Alignment,
        consensus: &[u8],
        map: Option<&[u32]>,
        rows: Option<&RowView>,
    ) -> Self {
        let cols = map.map_or(aln.cols(), <[u32]>::len);
        let nrows = view_rows(rows, aln.rows());
        let mut levels = vec![base_level(aln, consensus, map, rows)];
        while let Some(l) = levels.last()
            && (l.rows > 1 || l.cols > 1)
        {
            levels.push(halve(l, nrows, cols));
        }
        Pyramid { levels }
    }

    pub fn bytes(&self) -> usize {
        self.levels.iter().map(Level::bytes).sum()
    }

    /// The coarsest level whose blocks are no larger than `cells` cells on a side.
    pub fn level_for(&self, cells: f64) -> Option<&Level> {
        let k = (cells.log2().floor() as i64) - BASE_SHIFT as i64;
        (k >= 0).then(|| &self.levels[(k as usize).min(self.levels.len() - 1)])
    }
}

fn base_level(
    aln: &Alignment,
    consensus: &[u8],
    map: Option<&[u32]>,
    view: Option<&RowView>,
) -> Level {
    let b = 1usize << BASE_SHIFT;
    let rows = view_rows(view, aln.rows());
    let cols = map.map_or(aln.cols(), <[u32]>::len);
    let (brows, bcols) = (rows.div_ceil(b), cols.div_ceil(b));
    let missing = missing_code(aln.kind());
    let mut majority = vec![0u8; brows * bcols];
    let mut diff = vec![0u8; brows * bcols];
    majority
        .par_chunks_mut(bcols.max(1))
        .zip(diff.par_chunks_mut(bcols.max(1)))
        .enumerate()
        .for_each(|(br, (maj, dif))| {
            let mut hist = vec![0u8; bcols * NCODES];
            let mut ndiff = vec![0u8; bcols];
            let mut buf = vec![0u8; aln.cols()];
            let r1 = ((br + 1) * b).min(rows);
            let mut count = |v: usize, code: u8, cons: u8| {
                hist[(v >> BASE_SHIFT) * NCODES + code as usize] += 1;
                ndiff[v >> BASE_SHIFT] += (code != GAP && code != missing && code != cons) as u8;
            };
            for r in br * b..r1 {
                row_src(view, r).decode(aln, &mut buf);
                match map {
                    None => buf
                        .iter()
                        .enumerate()
                        .for_each(|(c, &code)| count(c, code, consensus[c])),
                    Some(m) => m
                        .iter()
                        .enumerate()
                        .for_each(|(v, &c)| count(v, buf[c as usize], consensus[c as usize])),
                }
            }
            let h = r1 - br * b;
            for bc in 0..bcols {
                maj[bc] = argmax(&hist[bc * NCODES..(bc + 1) * NCODES]);
                let w = (cols - bc * b).min(b);
                dif[bc] = (ndiff[bc] as usize * 255 / (w * h)) as u8;
            }
        });
    Level {
        shift: BASE_SHIFT,
        rows: brows,
        cols: bcols,
        majority,
        diff,
    }
}

/// Most common code; ties go to a residue over the gap, then to the lower code.
fn argmax(h: &[u8]) -> u8 {
    let mut best = GAP as usize;
    for (i, &v) in h.iter().enumerate().skip(1) {
        if v > h[best] || (v > 0 && v == h[best] && best == GAP as usize) {
            best = i;
        }
    }
    best as u8
}

/// Combine 2×2 blocks of `l`. `rows` × `cols` is the alignment, for the cells each block covers.
fn halve(l: &Level, rows_total: usize, cols_total: usize) -> Level {
    let (rows, cols) = (l.rows.div_ceil(2), l.cols.div_ceil(2));
    let side = 1usize << l.shift;
    let extent = |i: usize, total: usize| (total - i * side).min(side);
    let mut majority = vec![0u8; rows * cols];
    let mut diff = vec![0u8; rows * cols];
    majority
        .par_chunks_mut(cols)
        .zip(diff.par_chunks_mut(cols))
        .enumerate()
        .for_each(|(r, (maj, dif))| {
            for c in 0..cols {
                // (code, weight) of each distinct child majority.
                let mut votes = [(0u8, 0usize); 4];
                let (mut n, mut dsum, mut area) = (0usize, 0usize, 0usize);
                for (dr, dc) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
                    let (rr, cc) = (2 * r + dr, 2 * c + dc);
                    if rr < l.rows && cc < l.cols {
                        let (m, d) = l.at(rr, cc);
                        let w = extent(rr, rows_total) * extent(cc, cols_total);
                        dsum += d as usize * w;
                        area += w;
                        match votes[..n].iter_mut().find(|v| v.0 == m) {
                            Some(v) => v.1 += w,
                            None => {
                                votes[n] = (m, w);
                                n += 1;
                            }
                        }
                    }
                }
                maj[c] = votes[..n]
                    .iter()
                    .max_by_key(|&&(k, w)| (w, k != GAP, std::cmp::Reverse(k)))
                    .unwrap()
                    .0;
                dif[c] = ((dsum + area / 2) / area) as u8;
            }
        });
    Level {
        shift: l.shift + 1,
        rows,
        cols,
        majority,
        diff,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_bytes;
    use crate::stats::ColumnStats;

    fn fasta(rows: &[String]) -> Vec<u8> {
        rows.iter()
            .enumerate()
            .flat_map(|(i, s)| format!(">s{i}\n{s}\n").into_bytes())
            .collect()
    }

    #[test]
    fn blocks_and_levels() {
        // 20 x 20: left 8 columns all A, rest C; row 0 has a T at column 9.
        let rows: Vec<String> = (0..20)
            .map(|r| {
                (0..20)
                    .map(|c| {
                        if c < 8 {
                            'A'
                        } else if r == 0 && c == 9 {
                            'T'
                        } else {
                            'C'
                        }
                    })
                    .collect()
            })
            .collect();
        let a = parse_bytes(&fasta(&rows)).unwrap();
        let s = ColumnStats::compute(&a);
        let p = Pyramid::build(&a, &s.consensus);
        let l0 = &p.levels[0];
        assert_eq!((l0.rows, l0.cols), (3, 3));
        let sym = |code| a.symbol(code);
        assert_eq!(sym(l0.at(0, 0).0), b'A');
        assert_eq!(sym(l0.at(0, 1).0), b'C');
        assert_eq!(l0.at(0, 1).1, (255 / 64) as u8);
        assert_eq!(l0.at(1, 1).1, 0);
        // Edge block: 4 rows x 4 columns.
        assert_eq!(sym(l0.at(2, 2).0), b'C');
        assert_eq!(p.levels.last().unwrap().rows, 1);
        assert_eq!(p.levels.len(), 3);
        assert!(p.level_for(4.0).is_none());
        assert_eq!(p.level_for(8.0).unwrap().shift, 3);
        assert_eq!(p.level_for(31.0).unwrap().shift, 4);
        assert_eq!(p.level_for(1e9).unwrap().shift, 5);
    }

    /// A mapped pyramid equals the pyramid of the alignment with the hidden columns removed.
    #[test]
    fn mapped_equals_removed() {
        let rows: Vec<String> = (0..37)
            .map(|r| {
                (0..53)
                    .map(|c| b"ACGT-"[(r * 7 + c * 3 + r * c) % 5] as char)
                    .collect()
            })
            .collect();
        let a = parse_bytes(&fasta(&rows)).unwrap();
        let s = ColumnStats::compute(&a);
        let map: Vec<u32> = (0..53u32).filter(|c| c % 3 != 1).collect();
        let p = Pyramid::build_mapped(&a, &s.consensus, Some(&map), None);
        let kept: Vec<String> = rows
            .iter()
            .map(|r| {
                map.iter()
                    .map(|&c| r.as_bytes()[c as usize] as char)
                    .collect()
            })
            .collect();
        let b = parse_bytes(&fasta(&kept)).unwrap();
        let cons: Vec<u8> = map.iter().map(|&c| s.consensus[c as usize]).collect();
        let q = Pyramid::build(&b, &cons);
        assert_eq!(p.levels.len(), q.levels.len());
        for (x, y) in p.levels.iter().zip(&q.levels) {
            assert_eq!(
                (x.rows, x.cols, &x.majority, &x.diff),
                (y.rows, y.cols, &y.majority, &y.diff)
            );
        }
    }

    #[test]
    fn ties_prefer_residues() {
        let mut h = [0u8; NCODES];
        assert_eq!(argmax(&h), GAP);
        h[GAP as usize] = 4;
        h[3] = 4;
        h[2] = 4;
        assert_eq!(argmax(&h), 2);
        h[GAP as usize] = 5;
        assert_eq!(argmax(&h), GAP);
    }

    #[test]
    fn upper_levels_weight_by_area() {
        // 8 rows × 9 columns: level 0 is a full 8×8 block and an 8×1 edge block.
        // The full block is all gaps; the edge column is A with one T.
        let rows: Vec<String> = (0..8)
            .map(|r| format!("--------{}", if r == 0 { 'T' } else { 'A' }))
            .collect();
        let a = parse_bytes(&fasta(&rows)).unwrap();
        let s = ColumnStats::compute(&a);
        let p = Pyramid::build(&a, &s.consensus);
        let (l0, l1) = (&p.levels[0], &p.levels[1]);
        assert_eq!(l0.at(0, 1).1, (255 / 8) as u8); // 1 differing cell of 8
        // Level 1: 1 differing cell of 72, not the mean of the two blocks' shares.
        assert_eq!(l1.at(0, 0).1, ((255 / 8 * 8 + 36) / 72) as u8);
        // Gap block weighs 64 cells against 8: gap wins on weight.
        assert_eq!(l1.at(0, 0).0, GAP);
        // Equal weights (8 × 16: half gap, half A): the residue wins the tie.
        let rows: Vec<String> = (0..8)
            .map(|_| format!("{}{}", "-".repeat(8), "A".repeat(8)))
            .collect();
        let a = parse_bytes(&fasta(&rows)).unwrap();
        let s = ColumnStats::compute(&a);
        let p = Pyramid::build(&a, &s.consensus);
        assert_eq!(a.symbol(p.levels[1].at(0, 0).0), b'A');
    }
}
