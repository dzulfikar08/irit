//! Long-read views (Oxford Nanopore, PacBio): read lengths on a log scale with N50/N90, and the
//! mean quality of each read from its error probabilities, overall and by read length.
//!
//! Everything is accumulated in fixed-size tables while the file streams past (lengths come from
//! the length histogram the FastQC modules keep anyway), so memory doesn't grow with the file.

use serde::Serialize;

/// Length bins per factor of ten: bin `k` holds lengths from 10^(k/10) up to 10^((k+1)/10).
pub const BINS_PER_DECADE: usize = 10;
/// Bins up to 10^9 bp.
const LBINS: usize = 9 * BINS_PER_DECADE;
/// Read quality is kept to 0.1 Phred (QSTEP), from Q0 to Q94.
const QSTEP: f64 = 0.1;
const QBINS: usize = 941;

/// Reads with a median length above this are treated as long reads.
pub const AUTO_MEDIAN: f64 = 1000.0;

/// Error probability of each Phred score 0..=93.
pub fn error_table() -> [f64; 94] {
    std::array::from_fn(|q| 10f64.powf(-(q as f64) / 10.0))
}

/// The bin of a read of `len` ≥ 1 bases.
pub fn len_bin(len: usize) -> usize {
    // The small offset keeps exact powers of ten (10^(k/10) for k a multiple of 10) in their own
    // bin despite rounding in log10; no other bin edge is an integer.
    (((len as f64).log10() * BINS_PER_DECADE as f64 + 1e-9).floor() as usize).min(LBINS - 1)
}

/// Shortest and longest length in bin `k`.
pub fn bin_span(k: usize) -> (usize, usize) {
    let edge = |k: usize| 10f64.powf(k as f64 / BINS_PER_DECADE as f64);
    let lo = if k.is_multiple_of(BINS_PER_DECADE) { edge(k).round() } else { edge(k).ceil() };
    let next = k + 1;
    let hi = if next.is_multiple_of(BINS_PER_DECADE) { edge(next).round() } else { edge(next).ceil() };
    (lo as usize, hi as usize - 1)
}

/// The mean quality of a read from the sum of its bases' error probabilities: the Phred score of
/// the mean error probability (what basecallers report as a read's mean Q), not the mean of the
/// scores, which overstates it.
pub fn read_q(error_sum: f64, len: usize) -> f64 {
    -10.0 * (error_sum / len as f64).log10()
}

pub struct LongAcc {
    /// Reads per 0.1-Phred bin of read quality.
    qhist: Vec<u64>,
    q_sum: f64,
    q_reads: u64,
    q10: (u64, u64),
    q20: (u64, u64),
    /// Per length bin: reads with qualities and the sum of their read qualities.
    bin_q: Vec<(u64, f64)>,
}

impl Default for LongAcc {
    fn default() -> Self {
        LongAcc { qhist: vec![0; QBINS], q_sum: 0.0, q_reads: 0, q10: (0, 0), q20: (0, 0), bin_q: vec![(0, 0.0); LBINS] }
    }
}

impl LongAcc {
    /// A read of `len` ≥ 1 bases whose error probabilities sum to `error_sum`.
    pub fn add_quality(&mut self, len: usize, error_sum: f64) {
        let q = read_q(error_sum, len);
        self.q_sum += q;
        self.q_reads += 1;
        // A read of all Q20 bases comes out a hair under 20 in floating point; it counts as Q20.
        let qe = q + 1e-9;
        self.qhist[((qe * 10.0) as usize).min(QBINS - 1)] += 1;
        for (t, acc) in [(10.0, &mut self.q10), (20.0, &mut self.q20)] {
            if qe >= t {
                acc.0 += 1;
                acc.1 += len as u64;
            }
        }
        let b = &mut self.bin_q[len_bin(len)];
        b.0 += 1;
        b.1 += q;
    }

    /// The report section, from the length histogram (sorted by length) and this accumulator.
    /// `quality` is false when there are no Phred+33 qualities to read.
    pub fn finish(&self, lengths: &[(usize, u64)], detected: &'static str, quality: bool) -> LongReads {
        let reads: u64 = lengths.iter().map(|l| l.1).sum();
        let bases: u64 = lengths.iter().map(|&(l, c)| l as u64 * c).sum();
        let mut bins: Vec<LenBin> = Vec::new();
        let mut per_bin = vec![(0u64, 0u64); LBINS];
        for &(l, c) in lengths.iter().filter(|l| l.0 > 0) {
            let b = &mut per_bin[len_bin(l)];
            b.0 += c;
            b.1 += l as u64 * c;
        }
        if let (Some(first), Some(last)) = (per_bin.iter().position(|b| b.0 > 0), per_bin.iter().rposition(|b| b.0 > 0)) {
            for (k, &(n, b)) in per_bin.iter().enumerate().take(last + 1).skip(first) {
                let (min, max) = bin_span(k);
                let (qn, qs) = self.bin_q[k];
                bins.push(LenBin { min, max, reads: n, bases: b, mean_read_quality: (quality && qn > 0).then(|| round2(qs / qn as f64)) });
            }
        }
        let read_quality = (quality && self.q_reads > 0).then(|| {
            let mut dist: Vec<(u32, u64)> = Vec::new();
            for (i, &c) in self.qhist.iter().enumerate().filter(|(_, c)| **c > 0) {
                let q = (i / 10) as u32;
                match dist.last_mut() {
                    Some(d) if d.0 == q => d.1 += c,
                    _ => dist.push((q, c)),
                }
            }
            let n = self.q_reads;
            ReadQuality {
                reads: n,
                mean: round2(self.q_sum / n as f64),
                median: round2(hist_median(&self.qhist) as f64 * QSTEP),
                distribution: dist,
                q10_reads: self.q10.0,
                q10_percent: pct(self.q10.0, n),
                q10_bases_percent: pct(self.q10.1, bases),
                q20_reads: self.q20.0,
                q20_percent: pct(self.q20.0, n),
                q20_bases_percent: pct(self.q20.1, bases),
            }
        });
        LongReads {
            detected,
            reads,
            bases,
            mean_length: if reads == 0 { 0.0 } else { round2(bases as f64 / reads as f64) },
            median_length: median_length(lengths),
            n50: n_x(lengths, 50),
            n90: n_x(lengths, 90),
            longest: lengths.last().map_or(0, |l| l.0),
            length_bins: bins,
            read_quality,
        }
    }
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn pct(a: u64, b: u64) -> f64 {
    if b == 0 { 0.0 } else { round2(a as f64 * 100.0 / b as f64) }
}

/// Index of the bin holding the middle read (the lower one of the two middle reads).
fn hist_median(h: &[u64]) -> usize {
    let n: u64 = h.iter().sum();
    let target = n.div_ceil(2).max(1);
    let mut cum = 0;
    for (i, &c) in h.iter().enumerate() {
        cum += c;
        if cum >= target {
            return i;
        }
    }
    0
}

/// Median read length (the mean of the two middle reads for an even count); 0 for no reads.
pub fn median_length(lengths: &[(usize, u64)]) -> f64 {
    let n: u64 = lengths.iter().map(|l| l.1).sum();
    if n == 0 {
        return 0.0;
    }
    // 0-based ranks of the middle read(s).
    let (a, b) = if n % 2 == 1 { (n / 2, n / 2) } else { (n / 2 - 1, n / 2) };
    let at = |rank: u64| {
        let mut cum = 0;
        for &(l, c) in lengths {
            cum += c;
            if cum > rank {
                return l;
            }
        }
        0
    };
    (at(a) + at(b)) as f64 / 2.0
}

/// Nx: the length L such that reads of length L or more hold at least x% of all bases (N50 for
/// x = 50). `None` when there are no bases.
pub fn n_x(lengths: &[(usize, u64)], x: u64) -> Option<usize> {
    let total: u128 = lengths.iter().map(|&(l, c)| l as u128 * c as u128).sum();
    if total == 0 {
        return None;
    }
    let mut cum: u128 = 0;
    for &(l, c) in lengths.iter().rev() {
        cum += l as u128 * c as u128;
        if cum * 100 >= total * x as u128 {
            return Some(l);
        }
    }
    None
}

#[derive(Serialize)]
pub struct LongReads {
    /// Why this section is there: `"auto"` (median read length above 1,000 bp) or `"--long"`.
    pub detected: &'static str,
    pub reads: u64,
    pub bases: u64,
    pub mean_length: f64,
    pub median_length: f64,
    pub n50: Option<usize>,
    pub n90: Option<usize>,
    pub longest: usize,
    /// Reads and bases per log-scale length bin, from the first bin with reads to the last.
    pub length_bins: Vec<LenBin>,
    /// `None` for FASTA input or Phred+64 qualities.
    pub read_quality: Option<ReadQuality>,
}

#[derive(Serialize)]
pub struct LenBin {
    /// Shortest and longest length in the bin (bp, inclusive).
    pub min: usize,
    pub max: usize,
    pub reads: u64,
    pub bases: u64,
    /// Mean of the read qualities of the reads in the bin.
    pub mean_read_quality: Option<f64>,
}

#[derive(Serialize)]
pub struct ReadQuality {
    /// Reads with qualities (empty reads have none).
    pub reads: u64,
    /// Mean and median of the read qualities (median to 0.1).
    pub mean: f64,
    pub median: f64,
    /// (whole Phred score, reads whose quality is at least it and below the next).
    pub distribution: Vec<(u32, u64)>,
    pub q10_reads: u64,
    pub q10_percent: f64,
    /// Bases in reads of quality ≥ Q10, as a percent of all bases.
    pub q10_bases_percent: f64,
    pub q20_reads: u64,
    pub q20_percent: f64,
    pub q20_bases_percent: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hist(lens: &[usize]) -> Vec<(usize, u64)> {
        let mut m = std::collections::BTreeMap::new();
        for &l in lens {
            *m.entry(l).or_insert(0u64) += 1;
        }
        m.into_iter().collect()
    }

    #[test]
    fn nx_edge_cases() {
        assert_eq!(n_x(&[], 50), None);
        assert_eq!(n_x(&hist(&[0, 0]), 50), None);
        assert_eq!(n_x(&hist(&[7]), 50), Some(7));
        assert_eq!(n_x(&hist(&[7]), 90), Some(7));
        // 2, 3, 4, 5, 6 (20 bases): 6 + 5 = 11 ≥ 10 → N50 5; 6+5+4+3 = 18 ≥ 18 → N90 3.
        assert_eq!(n_x(&hist(&[2, 3, 4, 5, 6]), 50), Some(5));
        assert_eq!(n_x(&hist(&[2, 3, 4, 5, 6]), 90), Some(3));
        // Exactly half in the longest read: 10 of 20 bases → N50 is that read.
        assert_eq!(n_x(&hist(&[10, 5, 5]), 50), Some(10));
        // Ties: many reads of one length.
        assert_eq!(n_x(&hist(&[100, 100, 100, 1]), 50), Some(100));
        // One huge read dominates.
        assert_eq!(n_x(&hist(&[1_000_000, 10, 10, 10]), 90), Some(1_000_000));
        // No overflow with large totals.
        assert_eq!(n_x(&[(4_000_000_000, u64::MAX / 8_000_000_000)], 50), Some(4_000_000_000));
    }

    #[test]
    fn median_lengths() {
        assert_eq!(median_length(&[]), 0.0);
        assert_eq!(median_length(&hist(&[42])), 42.0);
        assert_eq!(median_length(&hist(&[1, 2, 3])), 2.0);
        assert_eq!(median_length(&hist(&[1, 2, 3, 10])), 2.5);
        assert_eq!(median_length(&hist(&[5, 5, 5, 9])), 5.0);
    }

    #[test]
    fn log_bins() {
        assert_eq!((len_bin(1), len_bin(9), len_bin(10), len_bin(1000), len_bin(999)), (0, 9, 10, 30, 29));
        assert_eq!(bin_span(0), (1, 1));
        assert_eq!(bin_span(10), (10, 12));
        assert_eq!(bin_span(30), (1000, 1258));
        assert_eq!(bin_span(29), (795, 999));
        // Every length falls inside its bin's span, and spans tile the lengths without gaps.
        for len in 1..=200_000usize {
            let (lo, hi) = bin_span(len_bin(len));
            assert!(lo <= len && len <= hi, "{len}: {lo}-{hi}");
        }
        for k in 0..LBINS - 1 {
            assert!(bin_span(k).1 + 1 >= bin_span(k + 1).0);
        }
    }

    #[test]
    fn read_quality_uses_error_probabilities() {
        let e = error_table();
        // All Q20: Q20.
        assert!((read_q(e[20] * 10.0, 10) - 20.0).abs() < 1e-9);
        // Half Q10, half Q30: mean error (0.1 + 0.001) / 2 → Q12.97, not the score mean Q20.
        let q = read_q(e[10] + e[30], 2);
        assert!((q - 12.967).abs() < 1e-3, "{q}");
        let mut a = LongAcc::default();
        a.add_quality(2, e[10] + e[30]);
        a.add_quality(10, e[20] * 10.0);
        a.add_quality(4, e[5] * 4.0);
        let r = a.finish(&hist(&[2, 10, 4]), "auto", true);
        let rq = r.read_quality.unwrap();
        assert_eq!((rq.reads, rq.q10_reads, rq.q20_reads), (3, 2, 1));
        assert_eq!(rq.q10_percent, 66.67);
        assert_eq!(rq.q20_bases_percent, 62.5); // 10 of 16 bases
        assert_eq!(rq.median, 12.9); // the middle read, to 0.1 below
        assert_eq!(rq.mean, round2((q + 20.0 + 5.0) / 3.0));
        assert_eq!(rq.distribution, vec![(5, 1), (12, 1), (20, 1)]);
        assert_eq!((r.reads, r.bases, r.longest, r.n50, r.median_length), (3, 16, 10, Some(10), 4.0));
        // Bins from 2 bp to 10 bp, with each bin's mean read quality.
        assert_eq!(r.length_bins.first().map(|b| (b.min, b.reads)), Some((2, 1)));
        assert_eq!(r.length_bins.last().map(|b| (b.min, b.max, b.mean_read_quality)), Some((10, 12, Some(20.0))));
    }

    #[test]
    fn empty_and_fasta() {
        let a = LongAcc::default();
        let r = a.finish(&[], "--long", true);
        assert_eq!((r.reads, r.bases, r.n50, r.longest, r.median_length), (0, 0, None, 0, 0.0));
        assert!(r.length_bins.is_empty() && r.read_quality.is_none());
        // Empty reads only: counted, no bins, no N50.
        let r = a.finish(&[(0, 3)], "--long", true);
        assert_eq!((r.reads, r.n50, r.length_bins.len()), (3, None, 0));
        // FASTA: lengths, no quality.
        let r = a.finish(&hist(&[5000, 7000]), "auto", false);
        assert!(r.read_quality.is_none() && r.length_bins.iter().all(|b| b.mean_read_quality.is_none()));
        assert_eq!(r.n50, Some(7000));
    }
}
