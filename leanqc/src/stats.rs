//! Single-pass accumulation of read statistics, then summarising into a serialisable report.

use memchr::memmem::Finder;
use serde::Serialize;
use std::collections::HashMap;

/// Per-position statistics stop here; long reads still count in every whole-read figure.
pub const MAX_POS: usize = 1000;
/// Phred+33 characters `!`..=`~`.
const QBINS: usize = 94;
/// Sequences longer than this are keyed on their first 50 bases for duplication, as FastQC does.
const DUP_TRIM_ABOVE: usize = 75;
const DUP_KEY_LEN: usize = 50;
/// Per-tile tracking switches off above this many tiles (FastQC does the same).
const MAX_TILES: usize = 2500;

pub const ADAPTERS: &[(&str, &[u8])] = &[
    ("Illumina Universal", b"AGATCGGAAGAG"),
    ("Nextera", b"CTGTCTCTTATA"),
    ("Illumina Small RNA 3'", b"TGGAATTCTCGG"),
    ("PolyA", b"AAAAAAAAAAAA"),
    ("PolyG", b"GGGGGGGGGGGG"),
];

pub struct Acc {
    reads: u64,
    bases: u64,
    min_len: usize,
    max_len: usize,
    zero_len: u64,
    len_hist: HashMap<usize, u64>,
    gc_hist: [f64; 101],
    base_tot: [u64; 5],
    qual_tot: [u64; QBINS],
    read_meanq: [u64; QBINS],
    pos_qual: Vec<[u64; QBINS]>,
    pos_base: Vec<[u64; 5]>,
    adapter_start: Vec<Vec<u64>>,
    finders: Vec<Finder<'static>>,
    min_qc: u8,
    max_qc: u8,
    sample: HashMap<Box<[u8]>, u32>,
    sampled: u64,
    sample_limit: u64,
    tiles: Tiles,
}

/// Mean quality per flowcell tile and position, for spotting bad patches of the flowcell.
enum Tiles {
    Unknown,
    On(HashMap<u32, TileAcc>),
    Off(&'static str),
}

#[derive(Default)]
struct TileAcc {
    sum: Vec<u64>,
    n: Vec<u64>,
}

/// Tile number from an Illumina read header, using FastQC's rule: split the whole header on
/// `:`; with 7+ fields (CASAVA 1.8+, `@M00:RUN:FC:LANE:TILE:X:Y 1:N:0:...`) the tile is field 5,
/// with 5–6 fields (older `@HWI:LANE:TILE:X:Y#IDX/1`) it is field 3.
pub fn parse_tile(id: &[u8]) -> Option<u32> {
    let colons = memchr::memchr_iter(b':', id).count();
    let field = match colons + 1 {
        n if n >= 7 => 4,
        n if n >= 5 => 2,
        _ => return None,
    };
    let raw = id.split(|&c| c == b':').nth(field)?;
    std::str::from_utf8(raw).ok()?.parse().ok()
}

/// Add one read to the 0–100% GC histogram. A read of `called` bases can only take
/// `called + 1` GC values, so putting it in a single percent bin leaves regular gaps
/// and spikes (75 bp reads fill 76 of 101 bins). Instead the read's value range,
/// ±half a base, is shared across the bins it overlaps, as FastQC does.
fn spread_gc(hist: &mut [f64; 101], gc: usize, called: usize) {
    let width = 100.0 / called as f64;
    let lo = ((gc as f64 - 0.5) * width).max(-0.5);
    let hi = ((gc as f64 + 0.5) * width).min(100.5);
    let first = (lo + 0.5).floor().max(0.0) as usize;
    let last = ((hi + 0.5).ceil() as usize).min(101);
    for (i, slot) in hist.iter_mut().enumerate().take(last).skip(first) {
        let overlap = hi.min(i as f64 + 0.5) - lo.max(i as f64 - 0.5);
        if overlap > 0.0 {
            *slot += overlap / (hi - lo);
        }
    }
}

#[inline]
fn base_index(b: u8) -> usize {
    match b {
        b'A' | b'a' => 0,
        b'C' | b'c' => 1,
        b'G' | b'g' => 2,
        b'T' | b't' => 3,
        _ => 4,
    }
}

impl Acc {
    fn add_tile(&mut self, id: &[u8], qual: &[u8], npos: usize) {
        let tile = parse_tile(id);
        let map = match (&mut self.tiles, tile) {
            (Tiles::Off(_), _) => return,
            (Tiles::Unknown, None) => {
                self.tiles = Tiles::Off("read names carry no Illumina tile number");
                return;
            }
            (Tiles::Unknown, Some(_)) => {
                self.tiles = Tiles::On(HashMap::new());
                let Tiles::On(m) = &mut self.tiles else { unreachable!() };
                m
            }
            (Tiles::On(_), None) => return, // an odd header among good ones: skip that read
            (Tiles::On(m), Some(_)) => m,
        };
        let tile = tile.unwrap();
        if !map.contains_key(&tile) && map.len() >= MAX_TILES {
            self.tiles = Tiles::Off("more than 2,500 tiles");
            return;
        }
        let t = map.entry(tile).or_default();
        if t.sum.len() < npos {
            t.sum.resize(npos, 0);
            t.n.resize(npos, 0);
        }
        for (i, &c) in qual.iter().take(npos).enumerate() {
            t.sum[i] += c.saturating_sub(33) as u64;
            t.n[i] += 1;
        }
    }

    pub fn new(sample_limit: u64) -> Self {
        Acc {
            reads: 0,
            bases: 0,
            min_len: usize::MAX,
            max_len: 0,
            zero_len: 0,
            len_hist: HashMap::new(),
            gc_hist: [0.0; 101],
            base_tot: [0; 5],
            qual_tot: [0; QBINS],
            read_meanq: [0; QBINS],
            pos_qual: Vec::new(),
            pos_base: Vec::new(),
            adapter_start: vec![vec![0; MAX_POS]; ADAPTERS.len()],
            finders: ADAPTERS.iter().map(|(_, s)| Finder::new(*s)).collect(),
            min_qc: u8::MAX,
            max_qc: 0,
            sample: HashMap::new(),
            sampled: 0,
            sample_limit,
            tiles: Tiles::Unknown,
        }
    }

    pub fn add(&mut self, id: &[u8], seq: &[u8], qual: Option<&[u8]>) {
        let len = seq.len();
        self.reads += 1;
        self.bases += len as u64;
        self.min_len = self.min_len.min(len);
        self.max_len = self.max_len.max(len);
        *self.len_hist.entry(len).or_default() += 1;
        if len == 0 {
            self.zero_len += 1;
            return;
        }
        let npos = len.min(MAX_POS);
        if self.pos_base.len() < npos {
            self.pos_base.resize(npos, [0; 5]);
            if qual.is_some() {
                self.pos_qual.resize(npos, [0; QBINS]);
            }
        }

        let mut gc = 0usize;
        let mut n = 0usize;
        for (i, &b) in seq.iter().enumerate() {
            let k = base_index(b);
            self.base_tot[k] += 1;
            gc += (k == 1 || k == 2) as usize;
            n += (k == 4) as usize;
            if i < MAX_POS {
                self.pos_base[i][k] += 1;
            }
        }
        if let Some(q) = qual {
            let mut qsum = 0u64;
            for (i, &c) in q.iter().enumerate().take(len) {
                self.min_qc = self.min_qc.min(c);
                self.max_qc = self.max_qc.max(c);
                let bin = (c.saturating_sub(33) as usize).min(QBINS - 1);
                self.qual_tot[bin] += 1;
                qsum += bin as u64;
                if i < MAX_POS {
                    self.pos_qual[i][bin] += 1;
                }
            }
            self.read_meanq[(qsum / len as u64) as usize] += 1;
            self.add_tile(id, q, len.min(MAX_POS));
        } else if matches!(self.tiles, Tiles::Unknown) {
            self.tiles = Tiles::Off("no quality scores");
        }
        let called = len - n;
        if called > 0 {
            spread_gc(&mut self.gc_hist, gc, called);
        }
        for (k, f) in self.finders.iter().enumerate() {
            if let Some(p) = f.find(seq) {
                self.adapter_start[k][p.min(MAX_POS - 1)] += 1;
            }
        }
        if self.sampled < self.sample_limit {
            self.sampled += 1;
            let key = if len > DUP_TRIM_ABOVE { &seq[..DUP_KEY_LEN] } else { seq };
            *self.sample.entry(key.into()).or_default() += 1;
        }
    }
}

#[derive(Serialize)]
pub struct Report {
    pub file: String,
    pub reads: u64,
    pub bases: u64,
    pub min_len: usize,
    pub max_len: usize,
    pub mean_len: f64,
    pub gc_percent: f64,
    pub n_percent: f64,
    /// `None` for FASTA input.
    pub q20_percent: Option<f64>,
    pub q30_percent: Option<f64>,
    pub phred_offset: Option<u8>,
    pub positions_capped_at: Option<usize>,
    pub per_position: Vec<PosStat>,
    /// (mean Phred of a read, number of reads)
    pub read_mean_quality: Vec<(u32, u64)>,
    /// Reads per GC percent, index 0..=100 (fractional: see `spread_gc`).
    pub gc_distribution: Vec<f64>,
    pub length_distribution: Vec<(usize, u64)>,
    pub adapters: Vec<AdapterStat>,
    pub duplication: Duplication,
    pub overrepresented: Vec<Overrep>,
    pub per_tile: PerTile,
    pub checks: Vec<Check>,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum PerTile {
    Measured {
        tiles: Vec<u32>,
        /// `deviation[t][p]`: tile t's mean quality at position p+1 minus the mean over tiles.
        deviation: Vec<Vec<f32>>,
        worst_tile: u32,
        worst_position: usize,
        worst_deviation: f64,
    },
    Skipped {
        reason: &'static str,
    },
}

#[derive(Serialize)]
pub struct PosStat {
    pub pos: usize,
    pub mean_q: Option<f64>,
    pub p10: Option<u32>,
    pub p25: Option<u32>,
    pub median: Option<u32>,
    pub p75: Option<u32>,
    pub p90: Option<u32>,
    /// A, C, G, T, N as percent of reads covering this position.
    pub base_percent: [f64; 5],
}

#[derive(Serialize)]
pub struct AdapterStat {
    pub name: &'static str,
    /// Percent of reads with the adapter starting at or before each position.
    pub cumulative_percent: Vec<f64>,
    pub max_percent: f64,
}

#[derive(Serialize)]
pub struct Duplication {
    pub reads_sampled: u64,
    pub distinct: u64,
    pub remaining_if_deduplicated_percent: f64,
}

#[derive(Serialize)]
pub struct Overrep {
    pub sequence: String,
    pub count: u32,
    pub percent: f64,
    pub possible_source: Option<&'static str>,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Warn,
    Fail,
}

#[derive(Serialize)]
pub struct Check {
    pub name: &'static str,
    pub status: Status,
    pub detail: String,
}

fn pct(a: u64, b: u64) -> f64 {
    if b == 0 { 0.0 } else { a as f64 * 100.0 / b as f64 }
}

/// Value at quantile `q` of a histogram (bins are integer scores).
fn quantile(hist: &[u64], total: u64, q: f64, shift: usize) -> Option<u32> {
    if total == 0 {
        return None;
    }
    let target = (q * total as f64).ceil().max(1.0) as u64;
    let mut cum = 0;
    for (i, &c) in hist.iter().enumerate() {
        cum += c;
        if cum >= target {
            return Some(i.saturating_sub(shift) as u32);
        }
    }
    None
}

/// Standard normal CDF (Abramowitz–Stegun 7.1.26 erf, error < 1.5e-7).
fn norm_cdf(x: f64) -> f64 {
    let z = x.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.327_591_1 * z);
    let poly = t * (0.254_829_592 + t * (-0.284_496_736 + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    let erf = 1.0 - poly * (-z * z).exp();
    0.5 * (1.0 + if x < 0.0 { -erf } else { erf })
}

impl Acc {
    pub fn finish(self, file: String) -> Report {
        let has_qual = self.max_qc > 0;
        // Phred+64 data (old Illumina) never uses characters below '@'.
        let shift = if has_qual && self.min_qc >= 64 { 31 } else { 0 };
        let offset = has_qual.then_some(if shift > 0 { 64 } else { 33 });
        let q_at_least = |t: usize| -> u64 { self.qual_tot.iter().skip(t + shift).sum() };
        let qual_bases: u64 = self.qual_tot.iter().sum();

        let per_position: Vec<PosStat> = self
            .pos_base
            .iter()
            .enumerate()
            .map(|(i, bc)| {
                let cover: u64 = bc.iter().sum();
                let (mean_q, qs) = match self.pos_qual.get(i) {
                    Some(h) => {
                        let tot: u64 = h.iter().sum();
                        let sum: u64 = h.iter().enumerate().map(|(q, &c)| q as u64 * c).sum();
                        let mean = (tot > 0).then(|| sum as f64 / tot as f64 - shift as f64);
                        let qv = [0.10, 0.25, 0.50, 0.75, 0.90].map(|q| quantile(h, tot, q, shift));
                        (mean, qv)
                    }
                    None => (None, [None; 5]),
                };
                PosStat {
                    pos: i + 1,
                    mean_q,
                    p10: qs[0],
                    p25: qs[1],
                    median: qs[2],
                    p75: qs[3],
                    p90: qs[4],
                    base_percent: bc.map(|c| pct(c, cover)),
                }
            })
            .collect();

        let read_mean_quality = self
            .read_meanq
            .iter()
            .enumerate()
            .filter(|(_, c)| **c > 0)
            .map(|(q, &c)| (q.saturating_sub(shift) as u32, c))
            .collect();

        let mut length_distribution: Vec<_> = self.len_hist.into_iter().collect();
        length_distribution.sort_unstable();

        let shown = self.max_len.min(MAX_POS);
        let adapters = ADAPTERS
            .iter()
            .zip(&self.adapter_start)
            .map(|((name, _), starts)| {
                let mut cum = 0;
                let cumulative_percent: Vec<f64> = starts[..shown]
                    .iter()
                    .map(|&c| {
                        cum += c;
                        pct(cum, self.reads)
                    })
                    .collect();
                let max_percent = cumulative_percent.last().copied().unwrap_or(0.0);
                AdapterStat { name, cumulative_percent, max_percent }
            })
            .collect();

        let distinct = self.sample.len() as u64;
        let duplication = Duplication {
            reads_sampled: self.sampled,
            distinct,
            remaining_if_deduplicated_percent: pct(distinct, self.sampled),
        };
        let mut over: Vec<_> = self
            .sample
            .iter()
            .filter(|(_, c)| (**c as f64) * 1000.0 > self.sampled as f64)
            .map(|(s, &c)| Overrep {
                sequence: String::from_utf8_lossy(s).into_owned(),
                count: c,
                percent: pct(c as u64, self.sampled),
                possible_source: ADAPTERS
                    .iter()
                    .find(|(_, a)| memchr::memmem::find(s, a).is_some())
                    .map(|(n, _)| *n),
            })
            .collect();
        over.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.sequence.cmp(&b.sequence)));
        over.truncate(20);

        let per_tile = match self.tiles {
            Tiles::On(map) if !map.is_empty() => tile_summary(map),
            Tiles::On(_) | Tiles::Unknown => PerTile::Skipped { reason: "no reads" },
            Tiles::Off(reason) => PerTile::Skipped { reason },
        };
        let called = self.base_tot[..4].iter().sum::<u64>();
        let mut r = Report {
            file,
            reads: self.reads,
            bases: self.bases,
            min_len: if self.reads == 0 { 0 } else { self.min_len },
            max_len: self.max_len,
            mean_len: if self.reads == 0 { 0.0 } else { self.bases as f64 / self.reads as f64 },
            gc_percent: pct(self.base_tot[1] + self.base_tot[2], called),
            n_percent: pct(self.base_tot[4], self.bases),
            q20_percent: has_qual.then(|| pct(q_at_least(20), qual_bases)),
            q30_percent: has_qual.then(|| pct(q_at_least(30), qual_bases)),
            phred_offset: offset,
            positions_capped_at: (self.max_len > MAX_POS).then_some(MAX_POS),
            per_position,
            read_mean_quality,
            gc_distribution: self.gc_hist.to_vec(),
            length_distribution,
            adapters,
            duplication,
            overrepresented: over,
            per_tile,
            checks: Vec::new(),
        };
        r.checks = checks(&r, self.zero_len);
        r
    }
}

fn grade(value: f64, warn: f64, fail: f64, higher_is_bad: bool) -> Status {
    let bad = |t: f64| if higher_is_bad { value > t } else { value < t };
    if bad(fail) {
        Status::Fail
    } else if bad(warn) {
        Status::Warn
    } else {
        Status::Pass
    }
}

/// Thresholds follow FastQC's defaults so results read the same way.
fn checks(r: &Report, zero_len: u64) -> Vec<Check> {
    let mut out = Vec::new();
    if r.phred_offset.is_some() {
        let worst_median = r.per_position.iter().filter_map(|p| p.median).min().unwrap_or(0) as f64;
        let worst_lq = r.per_position.iter().filter_map(|p| p.p25).min().unwrap_or(0) as f64;
        let status = grade(worst_median, 25.0, 20.0, false).max(grade(worst_lq, 10.0, 5.0, false));
        out.push(Check {
            name: "Per-position quality",
            status,
            detail: format!("lowest median Q{worst_median:.0}, lowest lower quartile Q{worst_lq:.0}"),
        });
        let mode = r.read_mean_quality.iter().max_by_key(|(_, c)| *c).map(|(q, _)| *q).unwrap_or(0) as f64;
        out.push(Check {
            name: "Per-read quality",
            status: grade(mode, 27.0, 20.0, false),
            detail: format!("most common mean read quality Q{mode:.0}"),
        });
    }
    let worst_diff = r
        .per_position
        .iter()
        .map(|p| {
            let b = p.base_percent;
            (b[0] - b[3]).abs().max((b[1] - b[2]).abs())
        })
        .fold(0.0, f64::max);
    out.push(Check {
        name: "Per-position base content",
        status: grade(worst_diff, 10.0, 20.0, true),
        detail: format!("largest A/T or G/C imbalance {worst_diff:.1}%"),
    });
    out.push(gc_check(&r.gc_distribution));
    let worst_n = r.per_position.iter().map(|p| p.base_percent[4]).fold(0.0, f64::max);
    out.push(Check {
        name: "N content",
        status: grade(worst_n, 5.0, 20.0, true),
        detail: format!("highest N rate at a position {worst_n:.2}%"),
    });
    out.push(Check {
        name: "Read length",
        status: if zero_len > 0 { Status::Fail } else if r.min_len != r.max_len { Status::Warn } else { Status::Pass },
        detail: if zero_len > 0 {
            format!("{zero_len} empty reads")
        } else {
            format!("{}–{} bp", r.min_len, r.max_len)
        },
    });
    if let PerTile::Measured { worst_tile, worst_position, worst_deviation, tiles, .. } = &r.per_tile {
        out.push(Check {
            name: "Per-tile quality",
            status: grade(*worst_deviation, -5.0, -10.0, false),
            detail: format!(
                "{} tiles; worst: tile {worst_tile} at position {worst_position}, {:.1} below the average",
                tiles.len(),
                -worst_deviation
            ),
        });
    }
    let rem = r.duplication.remaining_if_deduplicated_percent;
    out.push(Check {
        name: "Duplication",
        status: grade(rem, 80.0, 50.0, false),
        detail: format!(
            "{rem:.1}% of the first {} reads would remain after deduplication",
            r.duplication.reads_sampled
        ),
    });
    let top = r.overrepresented.first().map(|o| o.percent).unwrap_or(0.0);
    out.push(Check {
        name: "Overrepresented sequences",
        status: grade(top, 0.1, 1.0, true),
        detail: match r.overrepresented.len() {
            1 => "1 sequence above 0.1%".into(),
            n => format!("{n} sequences above 0.1%"),
        },
    });
    let (aname, amax) = r
        .adapters
        .iter()
        .map(|a| (a.name, a.max_percent))
        .fold(("none", 0.0), |acc, x| if x.1 > acc.1 { x } else { acc });
    out.push(Check {
        name: "Adapter content",
        status: grade(amax, 5.0, 10.0, true),
        detail: format!("{aname}: {amax:.2}% of reads"),
    });
    out
}

/// Each tile's mean quality per position, relative to the mean over all tiles at that position.
fn tile_summary(map: HashMap<u32, TileAcc>) -> PerTile {
    let mut tiles: Vec<_> = map.into_iter().collect();
    tiles.sort_unstable_by_key(|(t, _)| *t);
    let npos = tiles.iter().map(|(_, a)| a.sum.len()).max().unwrap_or(0);
    let means: Vec<Vec<Option<f64>>> = tiles
        .iter()
        .map(|(_, a)| (0..npos).map(|p| (a.n.get(p).copied().unwrap_or(0) > 0).then(|| a.sum[p] as f64 / a.n[p] as f64)).collect())
        .collect();
    let avg: Vec<f64> = (0..npos)
        .map(|p| {
            let v: Vec<f64> = means.iter().filter_map(|m| m[p]).collect();
            if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
        })
        .collect();
    let (mut worst, mut worst_tile, mut worst_position) = (0.0f64, tiles[0].0, 1);
    let deviation = means
        .iter()
        .zip(&tiles)
        .map(|(m, (tile, _))| {
            (0..npos)
                .map(|p| {
                    let d = m[p].map(|v| v - avg[p]).unwrap_or(0.0);
                    if d < worst {
                        (worst, worst_tile, worst_position) = (d, *tile, p + 1);
                    }
                    (d * 100.0).round() as f32 / 100.0
                })
                .collect()
        })
        .collect();
    PerTile::Measured {
        tiles: tiles.iter().map(|(t, _)| *t).collect(),
        deviation,
        worst_tile,
        worst_position,
        worst_deviation: worst,
    }
}

/// Compare the per-read GC histogram with a normal curve of the same mean and spread.
fn gc_check(hist: &[f64]) -> Check {
    let n: f64 = hist.iter().sum();
    if n == 0.0 {
        return Check { name: "Per-read GC", status: Status::Pass, detail: "no reads".into() };
    }
    let mean = hist.iter().enumerate().map(|(i, &c)| i as f64 * c).sum::<f64>() / n;
    let var = hist.iter().enumerate().map(|(i, &c)| (i as f64 - mean).powi(2) * c).sum::<f64>() / n;
    let sd = var.sqrt().max(0.5);
    let deviation: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let x = i as f64;
            let expected = n * (norm_cdf((x + 0.5 - mean) / sd) - norm_cdf((x - 0.5 - mean) / sd));
            (c - expected).abs()
        })
        .sum::<f64>()
        * 100.0
        / n;
    Check {
        name: "Per-read GC",
        status: grade(deviation, 15.0, 30.0, true),
        detail: format!("mean {mean:.1}%, {deviation:.1}% of reads off a normal curve"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_counts() {
        let mut a = Acc::new(10);
        a.add(b"r", b"ACGTN", Some(b"IIII#")); // Q40 x4, Q2
        a.add(b"r", b"GGCC", Some(b"5555")); // Q20
        let r = a.finish("t".into());
        assert_eq!(r.reads, 2);
        assert_eq!(r.bases, 9);
        assert_eq!(r.phred_offset, Some(33));
        assert!((r.gc_percent - 75.0).abs() < 1e-9); // 6 of 8 called bases
        assert!((r.q20_percent.unwrap() - 800.0 / 9.0).abs() < 1e-9);
        assert_eq!(r.per_position[0].base_percent[0], 50.0);
        assert_eq!(r.duplication.distinct, 2);
    }

    #[test]
    fn finds_adapters() {
        let mut a = Acc::new(10);
        a.add(b"r", b"TTTTAGATCGGAAGAGCACAC", None);
        a.add(b"r", b"TTTTTTTTTTTTTTTTTTTTT", None);
        let r = a.finish("t".into());
        assert_eq!(r.adapters[0].cumulative_percent[3], 0.0);
        assert_eq!(r.adapters[0].cumulative_percent[4], 50.0);
        assert_eq!(r.q30_percent, None);
    }

    #[test]
    fn gc_spread_keeps_one_read_per_read_and_fills_gaps() {
        let mut h = [0.0; 101];
        for g in 0..=75 {
            spread_gc(&mut h, g, 75);
        }
        assert!((h.iter().sum::<f64>() - 76.0).abs() < 1e-9);
        assert!(h.iter().all(|&v| v > 0.0), "every bin gets some weight");
        let mut one = [0.0; 101];
        spread_gc(&mut one, 1, 2); // exactly 50%
        assert!((one[50] - 1.0 / 50.0).abs() < 1e-9 && (one.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn tile_numbers_from_both_header_styles() {
        assert_eq!(parse_tile(b"M00123:45:000-ABC:1:1101:15589:1331 1:N:0:ATCACG"), Some(1101));
        assert_eq!(parse_tile(b"ERR064912.1 HS7_6949:5:1306:14580:154749#15/1"), Some(1306));
        assert_eq!(parse_tile(b"read_42"), None);
        assert_eq!(parse_tile(b"SRR1.1 1 length=100"), None);
    }

    #[test]
    fn flags_a_bad_tile() {
        let mut a = Acc::new(10);
        for _ in 0..50 {
            a.add(b"I:1:FC:1:1101:1:1", b"ACGT", Some(b"IIII")); // Q40
            a.add(b"I:1:FC:1:1102:1:1", b"ACGT", Some(b"IIII"));
            a.add(b"I:1:FC:1:1103:1:1", b"ACGT", Some(b"II++")); // Q10 at the end
        }
        let r = a.finish("t".into());
        let PerTile::Measured { worst_tile, worst_position, worst_deviation, .. } = r.per_tile else { panic!() };
        assert_eq!((worst_tile, worst_position), (1103, 3));
        assert!((worst_deviation + 20.0).abs() < 1e-9); // 10 - mean(40,40,10)
        assert_eq!(r.checks.iter().find(|c| c.name == "Per-tile quality").unwrap().status, Status::Fail);
    }

    #[test]
    fn cdf() {
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-7);
        assert!((norm_cdf(1.96) - 0.975).abs() < 1e-3);
    }
}
