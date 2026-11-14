//! Assembly statistics computed from a FASTA file, counted the way QUAST 5 reports them.
//!
//! QUAST (quast_libs/basic_stats.py) leaves out contigs shorter than `--min-contig` (500 bp by
//! default) before it counts anything except the `(>= 0 bp)` rows. The same 500 bp default is used
//! here, so a QUAST report on the same assembly gives the same numbers and isotable doesn't see a
//! conflict between the two. Ns per 100 kbp is QUAST's `# N's per 100 kbp`: N bases over the total
//! length. GC is G+C over A+C+G+T (QUAST divides by the length without Ns; the two differ only when
//! an assembly holds other IUPAC codes, which assemblers rarely write).

use anyhow::{Context, Result, bail};
use std::io::{BufRead, Read};
use std::path::Path;

/// QUAST's default `--min-contig`.
pub const QUAST_MIN_CONTIG: u64 = 500;

#[derive(Debug, Clone, PartialEq)]
pub struct AssemblyStats {
    /// Shortest contig counted.
    pub min_contig: u64,
    /// Every sequence in the file, whatever its length.
    pub contigs_all: u64,
    pub length_all: u64,
    /// Contigs of `min_contig` or more, and their total length.
    pub contigs: u64,
    pub length: u64,
    /// Contigs of 1000 bp or more (QUAST's `# contigs (>= 1000 bp)`).
    pub contigs_1kb: u64,
    pub largest: u64,
    pub n50: u64,
    pub l50: u64,
    pub n90: u64,
    pub l90: u64,
    /// G+C over A+C+G+T, %; `None` without any A, C, G or T.
    pub gc: Option<f64>,
    /// N bases per 100,000 bp of `length`.
    pub ns_per_100kbp: f64,
}

/// Length, G+C, A+C+G+T and N count of each sequence in a FASTA file (plain or gzip).
fn contigs(path: &Path) -> Result<Vec<[u64; 4]>> {
    let file = std::fs::File::open(path).with_context(|| format!("{}", path.display()))?;
    let mut head = [0u8; 2];
    let gz = std::fs::File::open(path)?.read(&mut head)? == 2 && head == [0x1f, 0x8b];
    let reader: Box<dyn BufRead> = if gz {
        Box::new(std::io::BufReader::new(flate2::read::MultiGzDecoder::new(file)))
    } else {
        Box::new(std::io::BufReader::new(file))
    };
    let mut out = Vec::new();
    let mut cur: Option<[u64; 4]> = None;
    for line in reader.lines() {
        let line = line?;
        if line.starts_with('>') {
            out.extend(cur.replace([0; 4]));
        } else if let Some(c) = cur.as_mut() {
            let seq = line.trim_end().as_bytes();
            c[0] += seq.len() as u64;
            for b in seq {
                match b.to_ascii_uppercase() {
                    b'G' | b'C' => {
                        c[1] += 1;
                        c[2] += 1;
                    }
                    b'A' | b'T' => c[2] += 1,
                    b'N' => c[3] += 1,
                    _ => {}
                }
            }
        } else if !line.trim().is_empty() {
            bail!("not FASTA: sequence before the first > line");
        }
    }
    out.extend(cur);
    Ok(out)
}

/// The length at which the contigs, longest first, reach `pct` % of `total`, and how many it took.
fn nx(sorted: &[u64], total: u64, pct: u64) -> (u64, u64) {
    let mut acc = 0;
    for (i, &l) in sorted.iter().enumerate() {
        acc += l;
        if acc * 100 >= total * pct {
            return (l, i as u64 + 1);
        }
    }
    (0, 0)
}

pub fn stats(path: &Path, min_contig: u64) -> Result<AssemblyStats> {
    let all = contigs(path)?;
    let mut kept: Vec<[u64; 4]> = all.iter().copied().filter(|c| c[0] >= min_contig).collect();
    kept.sort_unstable_by_key(|c| std::cmp::Reverse(c[0]));
    let lens: Vec<u64> = kept.iter().map(|c| c[0]).collect();
    let length: u64 = lens.iter().sum();
    let (n50, l50) = nx(&lens, length, 50);
    let (n90, l90) = nx(&lens, length, 90);
    let (gc, acgt, ns) = kept.iter().fold((0, 0, 0), |(g, a, n), c| (g + c[1], a + c[2], n + c[3]));
    Ok(AssemblyStats {
        min_contig,
        contigs_all: all.len() as u64,
        length_all: all.iter().map(|c| c[0]).sum(),
        contigs: kept.len() as u64,
        length,
        contigs_1kb: all.iter().filter(|c| c[0] >= 1000).count() as u64,
        largest: lens.first().copied().unwrap_or(0),
        n50,
        l50,
        n90,
        l90,
        // two decimals, as QUAST prints them
        gc: (acgt > 0).then(|| (gc as f64 * 10000.0 / acgt as f64).round() / 100.0),
        ns_per_100kbp: if length > 0 { (ns as f64 * 100000.0 / length as f64 * 100.0).round() / 100.0 } else { 0.0 },
    })
}

impl AssemblyStats {
    /// Column names and values, in the order `isotable assembly-stats` prints them.
    pub fn columns(&self) -> Vec<(&'static str, String)> {
        let two = |x: f64| format!("{x:.2}");
        vec![
            ("min_contig", self.min_contig.to_string()),
            ("contigs", self.contigs.to_string()),
            ("total_length", self.length.to_string()),
            ("largest_contig", self.largest.to_string()),
            ("n50", self.n50.to_string()),
            ("l50", self.l50.to_string()),
            ("n90", self.n90.to_string()),
            ("l90", self.l90.to_string()),
            ("gc_percent", self.gc.map(two).unwrap_or_default()),
            ("ns_per_100kbp", two(self.ns_per_100kbp)),
            ("contigs_1kb", self.contigs_1kb.to_string()),
            ("contigs_all", self.contigs_all.to_string()),
            ("length_all", self.length_all.to_string()),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fasta(dir: &Path, name: &str, contigs: &[(usize, &str)]) -> std::path::PathBuf {
        let mut s = String::new();
        for (i, (n, base)) in contigs.iter().enumerate() {
            s += &format!(">c{i}\n{}\n", base.repeat(*n));
        }
        let p = dir.join(name);
        std::fs::write(&p, s).unwrap();
        p
    }

    /// Worked by hand. Contigs 5000, 3000, 2000 (all N), 1000, 600, 500 and 100 bp; at 500 bp the
    /// 100 is left out. Total 12,100; half is 6,050: 5000 (5,000), then 3000 (8,000), so N50 3000,
    /// L50 2. 90% is 10,890: +2000 = 10,000, +1000 = 11,000, so N90 1000, L90 4. At 501 bp the
    /// 500 goes too: total 11,600, half 5,800 and 90% 10,440 give the same N50/L50/N90/L90.
    #[test]
    fn n50_l50_n90_by_hand() {
        let d = tempfile::tempdir().unwrap();
        let p = fasta(d.path(), "a.fa", &[(600, "A"), (5000, "G"), (100, "C"), (1000, "T"), (3000, "A"), (500, "C"), (2000, "N")]);
        let s = stats(&p, QUAST_MIN_CONTIG).unwrap();
        assert_eq!((s.contigs, s.length, s.largest), (6, 12100, 5000));
        assert_eq!((s.n50, s.l50, s.n90, s.l90), (3000, 2, 1000, 4));
        assert_eq!((s.contigs_all, s.length_all, s.contigs_1kb), (7, 12200, 4));
        // 2000 N in 12,100 bp
        assert_eq!(s.ns_per_100kbp, 16528.93);
        // G+C: 5000 G + 500 C over 10,100 A/C/G/T
        assert_eq!(s.gc, Some(54.46));
        let s = stats(&p, 501).unwrap();
        assert_eq!((s.contigs, s.length, s.n50, s.l50, s.n90, s.l90), (5, 11600, 3000, 2, 1000, 4));
        let s = stats(&p, 0).unwrap();
        assert_eq!((s.contigs, s.length), (7, 12200));
    }

    #[test]
    fn gzip_empty_and_not_fasta() {
        let d = tempfile::tempdir().unwrap();
        let plain = fasta(d.path(), "a.fa", &[(700, "ACGT"), (900, "GC")]);
        let gz = d.path().join("a.fa.gz");
        let mut enc = flate2::write::GzEncoder::new(std::fs::File::create(&gz).unwrap(), flate2::Compression::fast());
        std::io::Write::write_all(&mut enc, &std::fs::read(&plain).unwrap()).unwrap();
        enc.finish().unwrap();
        assert_eq!(stats(&gz, 500).unwrap(), stats(&plain, 500).unwrap());
        let s = stats(&fasta(d.path(), "short.fa", &[(100, "A")]), 500).unwrap();
        assert_eq!((s.contigs, s.length, s.n50, s.l50, s.gc, s.contigs_all), (0, 0, 0, 0, None, 1));
        std::fs::write(d.path().join("x.fa"), "ACGT\n").unwrap();
        assert!(stats(&d.path().join("x.fa"), 500).is_err());
    }
}
