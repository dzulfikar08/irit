//! Synthetic alignments for testing: a random reference, clades with shared mutations, and per-sequence
//! substitutions, deletions, N runs and ragged ends (like a viral surveillance alignment).

use std::io::{self, Write};

use crate::alphabet::Kind;

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    pub fn next_u64(&mut self) -> u64 {
        // splitmix64
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    pub fn below(&mut self, n: usize) -> usize {
        (((self.next_u64() >> 32) * n as u64) >> 32) as usize
    }
}

pub struct Spec {
    pub rows: usize,
    pub cols: usize,
    pub kind: Kind,
    pub clades: usize,
    pub seed: u64,
}

/// Fails with `InvalidInput` for zero rows or columns (an empty file is not an alignment).
pub fn write_fasta(spec: &Spec, out: &mut impl Write) -> io::Result<()> {
    if spec.rows == 0 || spec.cols == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "ROWS and COLS must be at least 1"));
    }
    let letters: &[u8] = match spec.kind {
        Kind::Nucleotide => b"ACGT",
        Kind::Protein => b"ACDEFGHIKLMNPQRSTVWY",
    };
    let unknown = if spec.kind == Kind::Nucleotide { b'N' } else { b'X' };
    let mut rng = Rng::new(spec.seed);
    let cols = spec.cols;
    let pick = |rng: &mut Rng| letters[rng.below(letters.len())];
    let reference: Vec<u8> = (0..cols).map(|_| pick(&mut rng)).collect();
    let clades: Vec<Vec<u8>> = (0..spec.clades.max(1))
        .map(|_| {
            let mut s = reference.clone();
            for _ in 0..cols / 200 {
                let i = rng.below(cols);
                s[i] = pick(&mut rng);
            }
            s
        })
        .collect();
    let mut row = vec![0u8; cols];
    for r in 0..spec.rows {
        let k = r * clades.len() / spec.rows.max(1);
        row.copy_from_slice(&clades[k]);
        for _ in 0..1 + rng.below(cols / 1000 + 2) {
            let i = rng.below(cols);
            row[i] = pick(&mut rng);
        }
        if rng.below(4) == 0 {
            let (i, n) = (rng.below(cols), 1 + rng.below(30));
            row[i..(i + n).min(cols)].fill(b'-');
        }
        if rng.below(3) == 0 {
            let (i, n) = (rng.below(cols), 1 + rng.below(cols / 50 + 1));
            row[i..(i + n).min(cols)].fill(unknown);
        }
        let lead = rng.below(cols / 300 + 1);
        let trail = rng.below(cols / 300 + 1);
        row[..lead].fill(b'-');
        row[cols - trail.min(cols)..].fill(b'-');
        writeln!(out, ">seq{:07}|clade{k}", r + 1)?;
        out.write_all(&row)?;
        out.write_all(b"\n")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_bytes;

    #[test]
    fn generated_alignment_parses() {
        for kind in [Kind::Nucleotide, Kind::Protein] {
            let mut buf = Vec::new();
            write_fasta(&Spec { rows: 50, cols: 400, kind, clades: 4, seed: 1 }, &mut buf).unwrap();
            let a = parse_bytes(&buf).unwrap();
            assert_eq!((a.rows(), a.cols(), a.kind()), (50, 400, kind));
            assert_eq!(a.names[49], "seq0000050|clade3");
        }
    }

    #[test]
    fn refuses_empty_and_handles_tiny() {
        for (rows, cols) in [(0, 10), (10, 0), (0, 0)] {
            let mut buf = Vec::new();
            let e = write_fasta(&Spec { rows, cols, kind: Kind::Nucleotide, clades: 4, seed: 1 }, &mut buf).unwrap_err();
            assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
            assert!(buf.is_empty());
        }
        let mut buf = Vec::new();
        write_fasta(&Spec { rows: 3, cols: 1, kind: Kind::Nucleotide, clades: 32, seed: 1 }, &mut buf).unwrap();
        assert_eq!(parse_bytes(&buf).unwrap().cols(), 1);
    }
}
