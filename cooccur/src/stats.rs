//! Compositional statistics: zero replacement, centred log-ratio, proportionality (rho) and
//! Spearman's correlation on CLR values, sequential permutation p-values and Benjamini-Hochberg q.

use std::sync::atomic::{AtomicUsize, Ordering};

/// The per-pair statistic.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// Proportionality rho on CLR values (Lovell et al. 2015; Erb & Notredame 2016).
    Rho,
    /// Spearman's rank correlation of CLR values.
    Spearman,
}

/// Which side of the null counts as extreme.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Alternative {
    TwoSided,
    Greater,
}

/// Replaces zeros with `pseudocount`; returns how many were replaced.
pub fn replace_zeros(values: &mut [Vec<f64>], pseudocount: f64) -> usize {
    let mut n = 0;
    for v in values.iter_mut().flatten() {
        if *v == 0.0 {
            *v = pseudocount;
            n += 1;
        }
    }
    n
}

/// The smallest non-zero value of the table (propr's zero replacement), if any.
pub fn smallest_nonzero(values: &[Vec<f64>]) -> Option<f64> {
    values.iter().flatten().copied().filter(|&v| v > 0.0).reduce(f64::min)
}

/// Log abundances (zeros replaced) of the kept genomes, and each sample's mean log: the centred
/// log-ratio (CLR; Aitchison 1986) of genome i in sample s is `rows[i][s] - mean[s]`.
#[derive(Clone, Debug)]
pub struct Logs {
    pub rows: Vec<Vec<f64>>,
    pub mean: Vec<f64>,
}

impl Logs {
    /// `values` is genomes × samples, without zeros.
    pub fn new(values: &[Vec<f64>]) -> Self {
        let rows: Vec<Vec<f64>> = values.iter().map(|r| r.iter().map(|v| v.ln()).collect()).collect();
        let n = rows.first().map_or(0, Vec::len);
        let mut mean = vec![0.0; n];
        for r in &rows {
            for (m, l) in mean.iter_mut().zip(r) {
                *m += l;
            }
        }
        for m in &mut mean {
            *m /= rows.len() as f64;
        }
        Logs { rows, mean }
    }

    /// Sum of squared deviations of genome i's CLR values (0: the same CLR in every sample).
    pub fn clr_ss(&self, i: usize) -> f64 {
        let c: Vec<f64> = self.rows[i].iter().zip(&self.mean).map(|(l, m)| l - m).collect();
        let mu = c.iter().sum::<f64>() / c.len() as f64;
        c.iter().map(|v| (v - mu) * (v - mu)).sum()
    }

    /// The same genomes without those not in `keep`, the CLR reference recomputed.
    pub fn subset(&self, keep: &[usize]) -> Self {
        let values: Vec<Vec<f64>> = keep.iter().map(|&i| self.rows[i].iter().map(|l| l.exp()).collect()).collect();
        Logs::new(&values)
    }
}

/// Scratch space for one thread.
#[derive(Default)]
pub struct Scratch {
    x: Vec<f64>,
    y: Vec<f64>,
    rx: Vec<f64>,
    ry: Vec<f64>,
    idx: Vec<usize>,
}

/// Average ranks of `x` into `out`, ties sharing the mean of their ranks.
fn ranks_into(x: &[f64], idx: &mut Vec<usize>, out: &mut Vec<f64>) {
    idx.clear();
    idx.extend(0..x.len());
    idx.sort_unstable_by(|&a, &b| x[a].total_cmp(&x[b]));
    out.clear();
    out.resize(x.len(), 0.0);
    let mut i = 0;
    while i < idx.len() {
        let mut j = i;
        while j + 1 < idx.len() && x[idx[j + 1]] == x[idx[i]] {
            j += 1;
        }
        let avg = (i + j) as f64 / 2.0 + 1.0;
        for &t in &idx[i..=j] {
            out[t] = avg;
        }
        i = j + 1;
    }
}

fn pearson(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len() as f64;
    let (mx, my) = (x.iter().sum::<f64>() / n, y.iter().sum::<f64>() / n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (a, b) in x.iter().zip(y) {
        sxy += (a - mx) * (b - my);
        sxx += (a - mx) * (a - mx);
        syy += (b - my) * (b - my);
    }
    sxy / (sxx * syy).sqrt()
}

/// Genomes a and b relative to the other genomes of each sample: (a's log minus the mean log of
/// every genome but b, b's log minus that same mean). Both are unchanged when a sample is scaled
/// (sequencing depth, closure to 100 %).
pub fn relative(l: &Logs, i: usize, j: usize) -> (Vec<f64>, Vec<f64>) {
    let k = l.rows.len() as f64;
    let (la, lb) = (&l.rows[i], &l.rows[j]);
    // mean log of the genomes other than b
    let h: Vec<f64> = l.mean.iter().zip(lb).map(|(m, b)| (k * m - b) / (k - 1.0)).collect();
    (la.iter().zip(&h).map(|(a, h)| a - h).collect(), lb.iter().zip(&h).map(|(b, h)| b - h).collect())
}

/// The statistic of a and b from `relative`, with b's values `r` in their own order or permuted.
/// With b at r, each sample's mean log is h + r/k, so clr(a) = arel - r/k and clr(b) = r (k-1)/k:
/// moving b moves the CLR reference, and the slight negative covariance the CLR puts between
/// independent genomes (about -1/(k-1)) is part of the null too.
/// rho = 1 - var(clr a - clr b) / (var(clr a) + var(clr b)) (Lovell et al. 2015, with the CLR as
/// reference: Erb & Notredame 2016).
pub fn pair_stat(arel: &[f64], r: &[f64], k: usize, method: Method, sc: &mut Scratch) -> f64 {
    let kf = k as f64;
    let shrink = (kf - 1.0) / kf;
    match method {
        Method::Rho => {
            let n = arel.len() as f64;
            let (mut sx, mut sxx, mut sy, mut syy, mut sd, mut sdd) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
            for (a, b) in arel.iter().zip(r) {
                let (x, y, d) = (a - b / kf, b * shrink, a - b);
                sx += x;
                sxx += x * x;
                sy += y;
                syy += y * y;
                sd += d;
                sdd += d * d;
            }
            let (vx, vy, vd) = (sxx - sx * sx / n, syy - sy * sy / n, (sdd - sd * sd / n).max(0.0));
            1.0 - vd / (vx + vy)
        }
        Method::Spearman => {
            sc.x.clear();
            sc.y.clear();
            for (a, b) in arel.iter().zip(r) {
                sc.x.push(a - b / kf);
                sc.y.push(b * shrink);
            }
            ranks_into(&sc.x, &mut sc.idx, &mut sc.rx);
            ranks_into(&sc.y, &mut sc.idx, &mut sc.ry);
            pearson(&sc.rx, &sc.ry)
        }
    }
}

/// The observed statistic of genomes i and j.
pub fn statistic(l: &Logs, i: usize, j: usize, method: Method) -> f64 {
    let (arel, r) = relative(l, i, j);
    pair_stat(&arel, &r, l.rows.len(), method, &mut Scratch::default())
}

/// SplitMix64 (Steele, Lea & Flood 2014): small, fast, and enough for shuffling.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in 0..n (multiply-shift; bias below n / 2^64).
    fn below(&mut self, n: usize) -> usize {
        ((self.next_u64() as u128 * n as u128) >> 64) as usize
    }

    fn shuffle(&mut self, x: &mut [f64]) {
        for i in (1..x.len()).rev() {
            let j = self.below(i + 1);
            x.swap(i, j);
        }
    }
}

/// The RNG seed of pair (i, j): the same for every thread count and every order of work.
pub fn pair_seed(seed: u64, i: usize, j: usize) -> u64 {
    let mut r = Rng::new(seed ^ ((i as u64) << 32 | j as u64).wrapping_mul(0xD6E8_FEB8_6659_FD93));
    r.next_u64()
}

/// Settings of the sequential permutation test (Besag & Clifford 1991): a pair stops once
/// `stop_after` (h) permuted statistics were at least as extreme as the observed one (p = h / l
/// after l permutations), or at the cap (p = (g + 1) / (cap + 1) for g < h exceedances). The cap
/// starts at `start`; pairs that reach it and don't pass the FDR are extended tenfold, up to `max`.
#[derive(Clone, Copy, Debug)]
pub struct PermTest {
    pub method: Method,
    pub start: usize,
    pub max: usize,
    pub stop_after: usize,
    pub alternative: Alternative,
}

/// The permutation p-value of pair (i, j) and the permutations used. The null is that b's
/// abundance relative to the other genomes is unrelated to a's: b's log-ratio to the geometric
/// mean of the other genomes is shuffled over the samples and both CLRs recomputed (`pair_stat`).
/// Returns p, permutations used, and whether the cap ended them. The RNG stream is pair_seed(seed, i, j).
pub fn perm_p(l: &Logs, (i, j): (usize, usize), t: PermTest, cap: usize, seed: u64, sc: &mut Scratch) -> (f64, usize, bool) {
    let method = t.method;
    let observed = statistic(l, i, j, method);
    let mut rng = Rng::new(pair_seed(seed, i, j));
    let eps = 1e-10 * observed.abs().max(1.0);
    let extreme = |s: f64| match t.alternative {
        Alternative::TwoSided => s.abs() >= observed.abs() - eps,
        Alternative::Greater => s >= observed - eps,
    };
    let (arel, mut r) = relative(l, i, j);
    let k = l.rows.len();
    let mut hits = 0;
    for n in 1..=cap {
        rng.shuffle(&mut r);
        if extreme(pair_stat(&arel, &r, k, method, sc)) {
            hits += 1;
            if hits == t.stop_after {
                return (hits as f64 / n as f64, n, false);
            }
        }
    }
    ((hits + 1) as f64 / (cap + 1) as f64, cap, true)
}

/// Benjamini-Hochberg (1995) adjusted p-values (q), in the input order.
pub fn bh(p: &[f64]) -> Vec<f64> {
    let m = p.len();
    let mut idx: Vec<usize> = (0..m).collect();
    idx.sort_by(|&a, &b| p[b].total_cmp(&p[a]));
    let mut q = vec![0.0; m];
    let mut running = 1.0f64;
    for (r, &i) in idx.iter().enumerate() {
        let rank = m - r;
        running = running.min(p[i] * m as f64 / rank as f64);
        q[i] = running.min(1.0);
    }
    q
}

/// One tested pair (row indices into `Logs`).
#[derive(Clone, Debug)]
pub struct Pair {
    pub i: usize,
    pub j: usize,
    pub stat: f64,
    pub p: f64,
    pub perms: usize,
    /// The permutation cap ended this pair (fewer than `stop_after` exceedances).
    pub capped: bool,
    pub q: f64,
}

/// Runs the permutation test of `todo` (pair index into `pairs`) with `cap`, over `threads`.
fn run(l: &Logs, test: PermTest, cap: usize, seed: u64, threads: usize, pairs: &mut [Pair], todo: &[usize]) {
    // Small chunks when few pairs are left (the extended ones), so every thread gets some.
    let chunk = (todo.len() / (threads.max(1) * 8)).clamp(1, 64);
    let next = AtomicUsize::new(0);
    let shared: &[Pair] = pairs;
    let done: Vec<(usize, f64, usize, bool)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads.max(1))
            .map(|_| {
                s.spawn(|| {
                    let mut out = Vec::new();
                    let mut sc = Scratch::default();
                    loop {
                        let c = next.fetch_add(1, Ordering::Relaxed) * chunk;
                        if c >= todo.len() {
                            break;
                        }
                        for &t in &todo[c..(c + chunk).min(todo.len())] {
                            let p = &shared[t];
                            let (pv, n, capped) = perm_p(l, (p.i, p.j), test, cap, seed, &mut sc);
                            out.push((t, pv, n, capped));
                        }
                    }
                    out
                })
            })
            .collect();
        workers.into_iter().flat_map(|w| w.join().expect("worker thread")).collect()
    });
    for (t, pv, n, capped) in done {
        pairs[t].p = pv;
        pairs[t].perms = n;
        pairs[t].capped = capped;
    }
}

fn set_q(pairs: &mut [Pair]) {
    let q = bh(&pairs.iter().map(|p| p.p).collect::<Vec<_>>());
    for (p, q) in pairs.iter_mut().zip(q) {
        p.q = q;
    }
}

/// Every pair i < j: statistic, permutation p and BH q. Pairs that reach the permutation cap with
/// q above `fdr` are rerun with ten times the cap (up to `test.max`). Each pair has its own RNG
/// stream from `seed`, so a rerun repeats its first permutations and continues, and the output is
/// the same on any number of threads. A pair that stopped on `stop_after` exceedances has the p it
/// would have under any larger cap. Lowering the p of pairs at the cap (the smallest p values)
/// leaves every other pair's q unchanged.
pub fn all_pairs(l: &Logs, test: PermTest, fdr: f64, seed: u64, threads: usize) -> Vec<Pair> {
    let k = l.rows.len();
    let mut pairs: Vec<Pair> = (0..k)
        .flat_map(|i| (i + 1..k).map(move |j| (i, j)))
        .map(|(i, j)| Pair { i, j, stat: statistic(l, i, j, test.method), p: 1.0, perms: 0, capped: false, q: 1.0 })
        .collect();
    let mut cap = test.start.min(test.max).max(1);
    let mut todo: Vec<usize> = (0..pairs.len()).collect();
    loop {
        run(l, test, cap, seed, threads, &mut pairs, &todo);
        set_q(&mut pairs);
        if cap >= test.max {
            break;
        }
        todo = (0..pairs.len()).filter(|&t| pairs[t].capped && pairs[t].q > fdr).collect();
        if todo.is_empty() {
            break;
        }
        cap = cap.saturating_mul(10).min(test.max);
    }
    pairs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn rho_by_hand() {
        // genomes (1, 2, 4), (2, 8, 4), (4, 4, 4): sample means of the logs are ln2, ln4, ln4,
        // so the CLRs are a = (-ln2, -ln2, 0), b = (0, ln2, 0) in units of ln2.
        // var(a) = 1/3, var(b) = 1/3, var(a - b) = var(-1, -2, 0) = 1 (sample variances, × ln2²).
        let l = Logs::new(&[vec![1.0, 2.0, 4.0], vec![2.0, 8.0, 4.0], vec![4.0, 4.0, 4.0]]);
        let expect = 1.0 - 1.0 / (1.0 / 3.0 + 1.0 / 3.0);
        assert!(close(statistic(&l, 0, 1, Method::Rho), expect, 1e-12), "{}", statistic(&l, 0, 1, Method::Rho));
        assert!(close(l.clr_ss(2), 2.0 * 2f64.ln().powi(2) * (1.0 / 3.0), 1e-12));
        // a genome proportional to another (here 3 ×) has rho 1
        let p = Logs::new(&[vec![1.0, 5.0, 2.0, 9.0], vec![3.0, 15.0, 6.0, 27.0], vec![4.0, 1.0, 7.0, 2.0]]);
        assert!(close(statistic(&p, 0, 1, Method::Rho), 1.0, 1e-12));
    }

    #[test]
    fn ranks_average_ties() {
        let (mut idx, mut out) = (Vec::new(), Vec::new());
        ranks_into(&[3.0, 1.0, 3.0, 2.0], &mut idx, &mut out);
        assert_eq!(out, [3.5, 1.0, 3.5, 2.0]);
    }

    #[test]
    fn spearman_of_monotone_rows() {
        // CLR ranks: genome 1 is genome 0 squared, so their CLRs rise together; genome 2 falls
        let l = Logs::new(&[vec![1.0, 2.0, 5.0, 9.0, 20.0], vec![1.0, 4.0, 25.0, 81.0, 400.0], vec![9.0, 7.0, 5.0, 3.0, 1.0]]);
        assert!(close(statistic(&l, 0, 1, Method::Spearman), 1.0, 1e-12));
        assert!(close(statistic(&l, 0, 2, Method::Spearman), -1.0, 1e-12));
    }

    #[test]
    fn bh_matches_r_p_adjust() {
        // p.adjust(c(0.01, 0.04, 0.03, 0.2, 0.5), "BH") = 0.05 0.0666667 0.0666667 0.25 0.5
        let q = bh(&[0.01, 0.04, 0.03, 0.2, 0.5]);
        let want = [0.05, 0.2 / 3.0, 0.2 / 3.0, 0.25, 0.5];
        for (a, b) in q.iter().zip(want) {
            assert!(close(*a, b, 1e-12), "{q:?}");
        }
    }

    #[test]
    fn permutation_p_is_reproducible_and_stops_early() {
        let x = vec![
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0],
            vec![5.0, 3.0, 8.0, 1.0, 2.0, 7.0, 4.0, 6.0],
            vec![2.0, 2.0, 9.0, 3.0, 1.0, 4.0, 6.0, 2.0],
            vec![2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0],
        ];
        let l = Logs::new(&x);
        let t = PermTest { method: Method::Rho, start: 10_000, max: 10_000, stop_after: 10, alternative: Alternative::TwoSided };
        let mut sc = Scratch::default();
        let (p1, n1, _) = perm_p(&l, (0, 1), t, 10_000, 7, &mut sc);
        let (p2, n2, _) = perm_p(&l, (0, 1), t, 10_000, 7, &mut sc);
        assert_eq!((p1, n1), (p2, n2));
        assert!(n1 < 10_000 && p1 > 0.05, "unrelated rows should stop early with a large p: p={p1} after {n1}");
        // proportional genomes (0 and 3): only the identity permutation is as extreme, so it runs to the cap
        let s = statistic(&l, 0, 3, Method::Rho);
        assert!(close(s, 1.0, 1e-12));
        let (p, n, capped) = perm_p(&l, (0, 3), t, 10_000, 1, &mut sc);
        assert!(capped);
        assert_eq!(n, 10_000);
        assert!(p < 0.002, "p={p}");
    }

    #[test]
    fn thread_count_does_not_change_results() {
        let mut r = Rng::new(3);
        let x: Vec<Vec<f64>> = (0..12).map(|_| (0..15).map(|_| (r.next_u64() % 1000) as f64 + 1.0).collect()).collect();
        let l = Logs::new(&x);
        for method in [Method::Rho, Method::Spearman] {
            let t = PermTest { method, start: 100, max: 100_000, stop_after: 10, alternative: Alternative::TwoSided };
            let one = all_pairs(&l, t, 0.05, 42, 1);
            let four = all_pairs(&l, t, 0.05, 42, 4);
            assert_eq!(one.len(), 66);
            for (a, b) in one.iter().zip(&four) {
                assert_eq!((a.i, a.j, a.stat, a.p, a.perms, a.q), (b.i, b.j, b.stat, b.p, b.perms, b.q));
            }
        }
    }

    #[test]
    fn extending_the_cap_matches_starting_with_it() {
        // genome 1 is proportional to genome 0 up to noise: it reaches every cap and is extended
        let mut r = Rng::new(9);
        let mut x: Vec<Vec<f64>> = (0..10).map(|_| (0..20).map(|_| (r.next_u64() % 1000) as f64 + 1.0).collect()).collect();
        x[1] = x[0].iter().map(|v| v * 3.0 * (1.0 + (r.next_u64() % 100) as f64 / 1000.0)).collect();
        let l = Logs::new(&x);
        let t = PermTest { method: Method::Rho, start: 100, max: 100_000, stop_after: 10, alternative: Alternative::TwoSided };
        let grown = all_pairs(&l, t, 0.05, 5, 3);
        let flat = all_pairs(&l, PermTest { start: 100_000, ..t }, 0.05, 5, 3);
        let p01 = grown.iter().find(|p| (p.i, p.j) == (0, 1)).unwrap();
        assert!(p01.perms > 100 && p01.q <= 0.05, "{p01:?}");
        assert!(flat.iter().find(|p| (p.i, p.j) == (0, 1)).unwrap().q <= 0.05);
        for (a, b) in grown.iter().zip(&flat) {
            if !a.capped {
                assert_eq!((a.p, a.perms), (b.p, b.perms), "pair {} {}", a.i, a.j);
            }
        }
    }
}
