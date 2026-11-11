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

/// The name `write_fasta` gives row `r` of `rows` split into `clades` clades.
pub fn row_name(r: usize, rows: usize, clades: usize) -> String {
    let k = r * clades.max(1) / rows.max(1);
    format!("seq{:07}|clade{k}", r + 1)
}

/// A random binary tree over `n` items written as Newick, with random branch lengths: each group
/// splits at a random point, without recursion. `leaf` writes item `i` (a name, or a subtree).
/// The root gets no branch length.
fn random_tree<W: Write>(n: usize, rng: &mut Rng, out: &mut W, leaf: &mut dyn FnMut(&mut W, usize, &mut Rng) -> io::Result<()>) -> io::Result<()> {
    let len = |rng: &mut Rng| 0.001 + rng.below(1000) as f64 / 10_000.0;
    // (lo, hi, mid, stage)
    let mut stack = vec![(0usize, n, 0usize, 0u8)];
    while let Some(&(lo, hi, mid, stage)) = stack.last() {
        let root = stack.len() == 1;
        match stage {
            0 if hi - lo == 1 => {
                stack.pop();
                leaf(out, lo, rng)?;
                if !root {
                    write!(out, ":{:.4}", len(rng))?;
                }
            }
            0 => {
                let mid = lo + 1 + rng.below(hi - lo - 1);
                *stack.last_mut().unwrap() = (lo, hi, mid, 1);
                out.write_all(b"(")?;
                stack.push((lo, mid, 0, 0));
            }
            1 => {
                stack.last_mut().unwrap().3 = 2;
                out.write_all(b",")?;
                stack.push((mid, hi, 0, 0));
            }
            _ => {
                stack.pop();
                out.write_all(b")")?;
                if !root {
                    write!(out, ":{:.4}", len(rng))?;
                }
            }
        }
    }
    Ok(())
}

/// A tree for the rows `write_fasta` makes with `rows` and `clades`: each clade's rows under one
/// random subtree (labelled `cladeK`), the clades joined by another. Leaf names match the FASTA
/// names.
pub fn write_newick<W: Write>(rows: usize, clades: usize, seed: u64, out: &mut W) -> io::Result<()> {
    if rows == 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "ROWS must be at least 1"));
    }
    let clades = clades.max(1);
    let mut rng = Rng::new(seed ^ 0x7EE);
    // Rows of clade k are those with r * clades / rows == k; some clades are empty when there
    // are more clades than rows.
    let start = |k: usize| (k * rows).div_ceil(clades);
    let groups: Vec<(usize, usize, usize)> = (0..clades).map(|k| (k, start(k), start(k + 1))).filter(|g| g.1 < g.2).collect();
    let mut clade = |out: &mut W, i: usize, rng: &mut Rng| -> io::Result<()> {
        let (k, a, b) = groups[i];
        random_tree(b - a, rng, out, &mut |out: &mut W, j: usize, _: &mut Rng| out.write_all(row_name(a + j, rows, clades).as_bytes()))?;
        if b - a > 1 {
            write!(out, "clade{k}")?;
        }
        Ok(())
    };
    random_tree(groups.len(), &mut rng, out, &mut clade)?;
    out.write_all(b";\n")
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
        writeln!(out, ">{}", row_name(r, spec.rows, clades.len()))?;
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
    fn tree_matches_rows() {
        for (rows, clades) in [(1, 32), (5, 2), (300, 7)] {
            let mut fa = Vec::new();
            write_fasta(&Spec { rows, cols: 20, kind: Kind::Nucleotide, clades, seed: 3 }, &mut fa).unwrap();
            let a = parse_bytes(&fa).unwrap();
            let mut nwk = Vec::new();
            write_newick(rows, clades, 3, &mut nwk).unwrap();
            let t = crate::tree::Tree::parse(std::str::from_utf8(&nwk).unwrap()).unwrap();
            let ix = crate::tree::TreeIndex::new(t, &a.names, &[]);
            assert_eq!((ix.matching.matched, ix.matching.leaves), (rows, rows), "{rows} rows, {clades} clades");
            // each clade's rows are together in tree order
            let clade_of = |r: u32| a.names[r as usize].split("|clade").nth(1).unwrap().to_string();
            let changes = ix.order.windows(2).filter(|w| clade_of(w[0]) != clade_of(w[1])).count();
            assert!(changes < clades.min(rows), "{changes}");
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
