//! Per-column consensus and conservation.

use rayon::prelude::*;

use crate::Alignment;
use crate::alphabet::{GAP, NCODES};

pub struct ColumnStats {
    /// Most frequent non-gap code per column (gap if the column is all gaps).
    pub consensus: Vec<u8>,
    /// Fraction of all rows that carry the consensus residue.
    pub identity: Vec<f32>,
    /// Fraction of rows that are gaps.
    pub gaps: Vec<f32>,
    /// Share of each code per column, 0–255 of the rows (rounded down). 32 bytes per column.
    pub shares: Vec<[u8; NCODES]>,
    /// Clustal X colouring per column ([`crate::scheme::clustal_mask`]); zeros for nucleotides.
    pub clustal: Vec<u32>,
}

/// Columns per task: 1024 × 32 u32 counters = 128 KB, which stays in L2. Splitting by rows instead
/// gave every thread a whole-width table (3.8 MB at 30 kb) and ran about 14× slower from cache misses.
/// Each task reduces its counters to (consensus, count, gaps) per column before returning, so the
/// full table never exists for the whole width.
const COL_BLOCK: usize = 1024;

impl ColumnStats {
    pub fn compute(aln: &Alignment) -> Self {
        let (rows, cols) = (aln.rows(), aln.cols());
        let m = &aln.matrix;
        let n = rows.max(1) as f32;
        let mut s = ColumnStats {
            consensus: vec![GAP; cols],
            identity: vec![0.0; cols],
            gaps: vec![0.0; cols],
            shares: vec![[0u8; NCODES]; cols],
            clustal: vec![0; cols],
        };
        let alpha = aln.alphabet();
        s.consensus
            .par_chunks_mut(COL_BLOCK)
            .zip(s.identity.par_chunks_mut(COL_BLOCK))
            .zip(s.gaps.par_chunks_mut(COL_BLOCK))
            .zip(s.shares.par_chunks_mut(COL_BLOCK))
            .zip(s.clustal.par_chunks_mut(COL_BLOCK))
            .enumerate()
            .for_each(|(blk, ((((cons, ident), gaps), shares), clustal))| {
                let (c0, w) = (blk * COL_BLOCK, cons.len());
                let mut acc = vec![0u32; w * NCODES];
                let mut buf = vec![0u8; w];
                for r in 0..rows {
                    m.decode_row(r, c0, &mut buf);
                    for (i, &code) in buf.iter().enumerate() {
                        acc[i * NCODES + code as usize] += 1;
                    }
                }
                for (i, h) in acc.as_chunks::<NCODES>().0.iter().enumerate() {
                    let (best, &cnt) = h.iter().enumerate().skip(1).max_by_key(|&(i, &v)| (v, std::cmp::Reverse(i))).unwrap();
                    if cnt > 0 {
                        cons[i] = best as u8;
                    }
                    ident[i] = cnt as f32 / n;
                    gaps[i] = h[GAP as usize] as f32 / n;
                    for (sh, &v) in shares[i].iter_mut().zip(h) {
                        *sh = (v as u64 * 255 / rows.max(1) as u64) as u8;
                    }
                    clustal[i] = crate::scheme::clustal_mask(alpha, h, rows as u32);
                }
            });
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_bytes;

    #[test]
    fn consensus() {
        let a = parse_bytes(b">a\nAC-T\n>b\nAG-T\n>c\nTG-A\n>d\nAG-A\n").unwrap();
        let s = ColumnStats::compute(&a);
        let cons: Vec<u8> = s.consensus.iter().map(|&c| a.symbol(c)).collect();
        assert_eq!(cons, b"AG-A"); // T/A tie in the last column goes to the lower code (A)
        assert_eq!(s.identity, [0.75, 0.75, 0.0, 0.5]);
        assert_eq!(s.gaps, [0.0, 0.0, 1.0, 0.0]);
        assert_eq!(s.shares[0][a.alphabet().encode(b'A').unwrap() as usize], 191); // 3 of 4
        assert_eq!(s.shares[2][GAP as usize], 255);
        assert!(s.clustal.iter().all(|&m| m == 0)); // DNA
    }

    #[test]
    fn spans_column_blocks() {
        // Columns on both sides of a block boundary, with a partial last block.
        let cols = COL_BLOCK + 3;
        let row = |c: char| -> String { (0..cols).map(|i| if i == COL_BLOCK + 1 { c } else { 'A' }).collect() };
        let text = format!(">a\n{}\n>b\n{}\n>c\n{}\n", row('G'), row('G'), row('-'));
        let a = parse_bytes(text.as_bytes()).unwrap();
        let s = ColumnStats::compute(&a);
        assert_eq!(s.consensus.len(), cols);
        assert_eq!(a.symbol(s.consensus[COL_BLOCK - 1]), b'A');
        assert_eq!(a.symbol(s.consensus[COL_BLOCK + 1]), b'G');
        assert!((s.identity[COL_BLOCK + 1] - 2.0 / 3.0).abs() < 1e-6);
        assert!((s.gaps[COL_BLOCK + 1] - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(s.identity[cols - 1], 1.0);
    }
}
