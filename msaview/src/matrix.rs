//! Row-major residue matrix: 4 bits per nucleotide (two per byte, low nibble first) or 8 bits per amino acid.
//! Rows live in fixed blocks so the matrix can grow while a file streams in without reallocating
//! (and briefly doubling) gigabytes.

use crate::alphabet::{Alphabet, NCODES};

const BLOCK_SHIFT: usize = 10;
const BLOCK_ROWS: usize = 1 << BLOCK_SHIFT;

pub struct Matrix {
    pub rows: usize,
    pub cols: usize,
    pub bits: u32,
    stride: usize,
    blocks: Vec<Vec<u8>>,
}

impl Matrix {
    pub fn empty(cols: usize, bits: u32) -> Self {
        Matrix { rows: 0, cols, bits, stride: stride(cols, bits), blocks: Vec::new() }
    }

    pub fn bytes(&self) -> usize {
        self.rows * self.stride
    }

    #[inline]
    pub fn row_bytes(&self, r: usize) -> &[u8] {
        let o = (r & (BLOCK_ROWS - 1)) * self.stride;
        &self.blocks[r >> BLOCK_SHIFT][o..o + self.stride]
    }

    #[inline]
    pub fn get(&self, r: usize, c: usize) -> u8 {
        let row = self.row_bytes(r);
        if self.bits == 4 { (row[c >> 1] >> ((c & 1) * 4)) & 15 } else { row[c] }
    }

    /// Append `n` zeroed rows and return them, for filling in parallel.
    pub fn grow(&mut self, n: usize) -> Vec<&mut [u8]> {
        let (start, end) = (self.rows, self.rows + n);
        while self.blocks.len() * BLOCK_ROWS < end {
            self.blocks.push(Vec::new());
        }
        for b in start >> BLOCK_SHIFT..end.div_ceil(BLOCK_ROWS) {
            let used = (end - b * BLOCK_ROWS).min(BLOCK_ROWS);
            self.blocks[b].resize(used * self.stride, 0);
        }
        self.rows = end;
        let stride = self.stride.max(1);
        let first = start >> BLOCK_SHIFT;
        self.blocks[first..]
            .iter_mut()
            .enumerate()
            .flat_map(|(i, blk)| blk.chunks_exact_mut(stride).enumerate().map(move |(j, row)| ((first + i) * BLOCK_ROWS + j, row)))
            .filter(|(r, _)| *r >= start)
            .map(|(_, row)| row)
            .collect()
    }

    pub fn push_row(&mut self) -> &mut [u8] {
        self.grow(1).pop().unwrap()
    }

    /// Drop the last row (after a failed encode).
    pub fn pop_row(&mut self) {
        self.rows -= 1;
        let b = self.rows >> BLOCK_SHIFT;
        self.blocks[b].truncate((self.rows - b * BLOCK_ROWS) * self.stride);
    }

    /// Decode `out.len()` codes of row `r` starting at column `c0`, one per byte.
    pub fn decode_row(&self, r: usize, c0: usize, out: &mut [u8]) {
        let row = self.row_bytes(r);
        if self.bits == 8 {
            out.copy_from_slice(&row[c0..c0 + out.len()]);
        } else {
            for (i, o) in out.iter_mut().enumerate() {
                let c = c0 + i;
                *o = (row[c >> 1] >> ((c & 1) * 4)) & 15;
            }
        }
    }

    /// Re-encode as `to` (8 bits per residue) by symbol, when a later row needs a wider alphabet.
    /// Lossless except that a `?` stored as N in a nucleotide alphabet stays N: the parser reads the
    /// input again instead when that would matter.
    pub fn convert(&self, from: &Alphabet, to: &Alphabet) -> Matrix {
        assert_eq!(to.bits, 8);
        let map: Vec<u8> = (0..NCODES as u8).map(|c| to.encode(from.symbol(c)).unwrap_or(0)).collect();
        let mut m = Matrix::empty(self.cols, 8);
        let mut buf = vec![0u8; self.cols];
        for (r, row) in m.grow(self.rows).into_iter().enumerate() {
            self.decode_row(r, 0, &mut buf);
            for (o, &c) in row.iter_mut().zip(&buf) {
                *o = map[c as usize];
            }
        }
        m
    }
}

pub fn stride(cols: usize, bits: u32) -> usize {
    if bits == 4 { cols.div_ceil(2) } else { cols }
}

/// Encode raw sequence bytes into a zeroed packed row. Whitespace and digits are skipped.
/// Returns the residue count (which may exceed the row), or the first byte the alphabet rejects.
pub fn encode_row(a: &Alphabet, seq: &[u8], out: &mut [u8]) -> Result<usize, u8> {
    encode_at(a, seq, out, 0)
}

/// Like [`encode_row`], writing from column `start` (for interleaved formats, one block at a time).
pub fn encode_at(a: &Alphabet, seq: &[u8], out: &mut [u8], start: usize) -> Result<usize, u8> {
    let nibbles = a.bits == 4;
    let cap = if nibbles { out.len() * 2 } else { out.len() };
    let mut n = start;
    for &b in seq {
        let code = match a.encode(b) {
            Some(c) => c,
            None if b.is_ascii_whitespace() || b.is_ascii_digit() => continue,
            None => return Err(b),
        };
        if n < cap {
            if nibbles {
                out[n >> 1] |= code << ((n & 1) * 4);
            } else {
                out[n] = code;
            }
        }
        n += 1;
    }
    Ok(n - start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alphabet::{DNA, NT_MIXED, PROTEIN, RNA};

    #[test]
    fn packs_nibbles() {
        let mut m = Matrix::empty(5, 4);
        assert_eq!(encode_row(&DNA, b"AC-gN\r", m.push_row()), Ok(5));
        let s: Vec<u8> = (0..5).map(|c| DNA.symbol(m.get(0, c))).collect();
        assert_eq!(s, b"AC-GN");
        assert_eq!(m.bytes(), 3);
    }

    #[test]
    fn rejects_and_converts() {
        let mut m = Matrix::empty(3, 4);
        assert_eq!(encode_row(&DNA, b"AE-", m.push_row()), Err(b'E'));
        m.pop_row();
        assert_eq!(m.rows, 0);
        encode_row(&DNA, b"AT-", m.push_row()).unwrap();
        let p = m.convert(&DNA, &PROTEIN);
        let s: Vec<u8> = (0..3).map(|c| PROTEIN.symbol(p.get(0, c))).collect();
        assert_eq!(s, b"AT-");
    }

    #[test]
    fn converts_rna_by_symbol() {
        // U stays U (selenocysteine in protein, U next to T in the mixed alphabet), not T.
        let mut m = Matrix::empty(4, 4);
        encode_row(&RNA, b"ACGU", m.push_row()).unwrap();
        for to in [&PROTEIN, &NT_MIXED] {
            let p = m.convert(&RNA, to);
            let s: Vec<u8> = (0..4).map(|c| to.symbol(p.get(0, c))).collect();
            assert_eq!(s, b"ACGU");
        }
    }

    #[test]
    fn encodes_at_offset() {
        let mut m = Matrix::empty(5, 4);
        let row = m.push_row();
        assert_eq!(encode_at(&DNA, b"AC", row, 0), Ok(2));
        assert_eq!(encode_at(&DNA, b"G-T", row, 2), Ok(3));
        let s: Vec<u8> = (0..5).map(|c| DNA.symbol(m.get(0, c))).collect();
        assert_eq!(s, b"ACG-T");
    }

    #[test]
    fn grows_across_blocks() {
        let mut m = Matrix::empty(3, 8);
        for (i, row) in m.grow(BLOCK_ROWS - 1).into_iter().enumerate() {
            row[0] = i as u8;
        }
        let rows = m.grow(3);
        assert_eq!(rows.len(), 3);
        for (i, row) in rows.into_iter().enumerate() {
            row[1] = 7 + i as u8;
        }
        assert_eq!(m.rows, BLOCK_ROWS + 2);
        assert_eq!(m.get(5, 0), 5);
        assert_eq!((m.get(BLOCK_ROWS - 1, 1), m.get(BLOCK_ROWS, 1), m.get(BLOCK_ROWS + 1, 1)), (7, 8, 9));
    }
}
