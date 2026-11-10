//! Single-pass accumulation of read statistics, then summarising into a serialisable report.
//!
//! The verdicts follow FastQC 0.12.1's modules: per-position modules are judged on FastQC's
//! linear position groups, per-read GC is compared with a normal curve centred on the
//! histogram's mode, and duplication tracks the first 100,000 distinct sequences through the
//! whole file and extrapolates.
//!
//! Portions of this file are translated from FastQC 0.12.1
//! (<https://www.bioinformatics.babraham.ac.uk/projects/fastqc/>), Copyright 2010-17 Simon
//! Andrews, Babraham Bioinformatics, licensed under the GNU GPL version 3 or any later version
//! (as is this file; see LICENSE). Translated from, in `uk.ac.babraham.FastQC`:
//! `Graphs.BaseGroup` (linear position groups), `Modules.GCModel.GCModel` (GC claims),
//! `Modules.OverRepresentedSeqs` and `Modules.DuplicationLevel` (duplicate tracking and the
//! corrected count), `Sequence.QualityEncoding.PhredEncoding` (encoding detection, changed),
//! `Modules.PerBaseQualityScores` / `Utilities.QualityCount` (percentiles),
//! `Modules.PerSequenceGCContent` (read truncation, normal-curve fit), `Modules.AdapterContent`
//! (cumulative adapter positions), `Modules.PerSequenceQualityScores` (mode),
//! `Modules.PerTileQualityScores` (tile field, sampling, deviations), and the defaults in
//! `Configuration/adapter_list.txt` and `Configuration/limits.txt`. Each translated item says
//! which FastQC code it ports.

use memchr::memmem::Finder;
use serde::Serialize;
use std::collections::HashMap;

/// Positions 1..=999 are kept one by one.
const EXACT: usize = 999;
/// From position 1,000 to 99,999 positions are kept in 10 bp slots, then in 1,000 bp slots.
/// FastQC's group boundaries for reads that long are multiples of 20 (and of 2,000 above
/// 100 kb), so slots never straddle a group and verdicts cover the whole read.
const FINE: usize = 10;
const FINE_END: usize = 99_999;
const COARSE: usize = 1000;
const FINE_SLOTS: usize = (FINE_END - EXACT) / FINE;
/// Phred+33 characters `!`..=`~`.
const QBINS: usize = 94;
/// Duplication and overrepresented sequences use the first 50 bases of longer reads (FastQC 0.12).
// Port of FastQC OverRepresentedSeqs.processSequence (truncation to 50 bp).
const DUP_KEY_LEN: usize = 50;
/// FastQC skips a position's quality percentiles unless more than this many reads cover it.
// Port of FastQC PerBaseQualityScores.getPercentages (minimum coverage).
const MIN_QUAL_COVER: u64 = 100;
/// Per-tile tracking covers the first 1,000 positions and switches off above 2,500 tiles.
const TILE_MAX_POS: usize = 1000;
const MAX_TILES: usize = 2500;

/// FastQC's default adapter list.
// Port of FastQC Configuration/adapter_list.txt.
pub const ADAPTERS: &[(&str, &[u8])] = &[
    ("Illumina Universal", b"AGATCGGAAGAG"),
    ("Illumina Small RNA 3'", b"TGGAATTCTCGG"),
    ("Illumina Small RNA 5'", b"GATCGTCGGACT"),
    ("Nextera", b"CTGTCTCTTATA"),
    ("PolyA", b"AAAAAAAAAAAA"),
    ("PolyG", b"GGGGGGGGGGGG"),
];
const ADAPTER_LEN: usize = 12;

/// Slot holding 0-based position `i`.
fn slot_of(i: usize) -> usize {
    if i < EXACT {
        i
    } else if i < FINE_END {
        EXACT + (i - EXACT) / FINE
    } else {
        EXACT + FINE_SLOTS + (i - FINE_END) / COARSE
    }
}

/// First and last 1-based position of slot `s`.
fn slot_span(s: usize) -> (usize, usize) {
    if s < EXACT {
        (s + 1, s + 1)
    } else if s < EXACT + FINE_SLOTS {
        let a = EXACT + (s - EXACT) * FINE;
        (a + 1, a + FINE)
    } else {
        let a = FINE_END + (s - EXACT - FINE_SLOTS) * COARSE;
        (a + 1, a + COARSE)
    }
}

/// FastQC's linear position groups for reads up to `max_len` (1-based, inclusive): single
/// positions up to 9, then equal-width groups chosen so there are fewer than 75 in all.
// Port of FastQC BaseGroup.makeLinearBaseGroups and BaseGroup.getLinearInterval.
pub fn base_groups(max_len: usize) -> Vec<(usize, usize)> {
    if max_len <= 75 {
        return (1..=max_len).map(|p| (p, p)).collect();
    }
    let interval = (0..)
        .flat_map(|e| [2, 5, 10].map(|b| b * 10usize.pow(e)))
        .find(|&i| 9 + (max_len - 9) / i + usize::from(!(max_len - 9).is_multiple_of(i)) < 75)
        .unwrap();
    let mut out = Vec::new();
    let mut start = 1;
    while start <= max_len {
        let mut end = start + interval - 1;
        if start < 10 {
            end = start;
        }
        if start == 10 && interval > 10 {
            end = interval - 1;
        }
        out.push((start, end.min(max_len)));
        start = if start < 10 {
            start + 1
        } else if start == 10 && interval > 10 {
            interval
        } else {
            start + interval
        };
    }
    out
}

/// Slots overlapping a group, with how many of the group's positions each covers.
fn slots_in(lo: usize, hi: usize, nslots: usize) -> impl Iterator<Item = (usize, usize)> {
    let first = slot_of(lo - 1);
    let last = slot_of(hi - 1).min(nslots.saturating_sub(1));
    (first..=last).filter(move |_| first < nslots).map(move |s| {
        let (a, b) = slot_span(s);
        (s, hi.min(b) + 1 - lo.max(a))
    })
}

#[inline]
fn base_index(b: u8) -> usize {
    match b {
        b'A' | b'a' => 0,
        b'C' | b'c' => 1,
        b'G' | b'g' => 2,
        b'T' | b't' => 3,
        b'N' | b'n' => 4,
        _ => 5, // other IUPAC codes: not A/C/G/T, not N
    }
}

/// How many GC counts claim each percentage bin for reads of `len` bases (FastQC's GC model):
/// a read with `g` GC bases covers the bins from `round((g-0.5)/len)` to `round((g+0.5)/len)`.
// Port of FastQC GCModel.GCModel (the claim range of each GC count).
fn gc_range(g: usize, len: usize) -> (usize, usize) {
    let lo = (g as f64 - 0.5).clamp(0.0, len as f64);
    let hi = (g as f64 + 0.5).clamp(0.0, len as f64);
    let pct = |c: f64| (c * 100.0 / len as f64 + 0.5).floor() as usize;
    (pct(lo), pct(hi))
}

// Port of FastQC GCModel.GCModel (claims per bin).
fn gc_claims(len: usize) -> [u32; 101] {
    let mut claims = [0u32; 101];
    for g in 0..=len {
        let (a, b) = gc_range(g, len);
        for c in &mut claims[a..=b] {
            *c += 1;
        }
    }
    claims
}

/// FastQC's duplication tracking: the first `limit` distinct sequences are counted through
/// the whole file; sequences first seen after that are not tracked.
// Port of FastQC OverRepresentedSeqs.processSequence (the 100,000-sequence limit).
struct Dup {
    counts: HashMap<Box<[u8]>, u32>,
    reads: u64,
    reads_at_limit: u64,
    limit: usize,
    frozen: bool,
}

impl Dup {
    fn add(&mut self, seq: &[u8]) {
        self.reads += 1;
        let key = if seq.len() > DUP_KEY_LEN { &seq[..DUP_KEY_LEN] } else { seq };
        if let Some(c) = self.counts.get_mut(key) {
            *c += 1;
            if !self.frozen {
                self.reads_at_limit = self.reads;
            }
        } else if !self.frozen {
            self.counts.insert(key.into(), 1);
            self.reads_at_limit = self.reads;
            if self.counts.len() >= self.limit {
                self.frozen = true;
            }
        }
    }

    /// Percent of reads left after deduplication, extrapolated as FastQC does: for each
    /// duplication level, the number of distinct sequences tracked is corrected for the chance
    /// that a sequence at that level was first seen only after tracking stopped.
    // Port of FastQC DuplicationLevel.calculateLevels.
    fn remaining_percent(&self) -> f64 {
        let mut by_level: HashMap<u32, u64> = HashMap::new();
        for &c in self.counts.values() {
            *by_level.entry(c).or_default() += 1;
        }
        let (mut dedup, mut raw) = (0.0, 0.0);
        for (&level, &seen) in &by_level {
            let n = corrected_count(self.reads_at_limit, self.reads, level as u64, seen);
            dedup += n;
            raw += n * level as f64;
        }
        if raw == 0.0 { 100.0 } else { dedup / raw * 100.0 }
    }
}

// Port of FastQC DuplicationLevel.getCorrectedCount.
fn corrected_count(at_limit: u64, total: u64, level: u64, seen: u64) -> f64 {
    if at_limit == total || total - seen < at_limit {
        return seen as f64;
    }
    let mut p_not_seen = 1.0;
    let give_up = 1.0 - seen as f64 / (seen as f64 + 0.01);
    for i in 0..at_limit {
        p_not_seen *= ((total - i) as f64 - level as f64) / (total - i) as f64;
        if p_not_seen < give_up {
            p_not_seen = 0.0;
            break;
        }
    }
    seen as f64 / (1.0 - p_not_seen)
}

pub struct Acc {
    reads: u64,
    bases: u64,
    min_len: usize,
    max_len: usize,
    zero_len: u64,
    len_hist: HashMap<usize, u64>,
    gc_hist: [f64; 101],
    gc_claims: HashMap<usize, [u32; 101]>,
    base_tot: [u64; 6],
    qual_tot: [u64; QBINS],
    read_meanq: [u64; QBINS],
    /// Per slot (see `slot_of`): quality histogram and A, C, G, T, N, other counts.
    pos_qual: Vec<[u64; QBINS]>,
    pos_base: Vec<[u64; 6]>,
    /// Per adapter, reads whose first hit starts in each slot.
    adapter_start: Vec<Vec<u64>>,
    finders: Vec<Finder<'static>>,
    min_qc: u8,
    max_qc: u8,
    dup: Dup,
    tiles: Tiles,
}

impl Acc {
    pub fn new(dup_distinct: usize) -> Self {
        Acc {
            reads: 0,
            bases: 0,
            min_len: usize::MAX,
            max_len: 0,
            zero_len: 0,
            len_hist: HashMap::new(),
            gc_hist: [0.0; 101],
            gc_claims: HashMap::new(),
            base_tot: [0; 6],
            qual_tot: [0; QBINS],
            read_meanq: [0; QBINS],
            pos_qual: Vec::new(),
            pos_base: Vec::new(),
            adapter_start: vec![Vec::new(); ADAPTERS.len()],
            finders: ADAPTERS.iter().map(|(_, s)| Finder::new(*s)).collect(),
            min_qc: u8::MAX,
            max_qc: 0,
            dup: Dup { counts: HashMap::new(), reads: 0, reads_at_limit: 0, limit: dup_distinct.max(1), frozen: false },
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
        self.dup.add(seq);
        if len == 0 {
            self.zero_len += 1;
            return;
        }
        let nslots = slot_of(len - 1) + 1;
        if self.pos_base.len() < nslots {
            self.pos_base.resize(nslots, [0; 6]);
            if qual.is_some() {
                self.pos_qual.resize(nslots, [0; QBINS]);
            }
        }

        for (i, &b) in seq.iter().enumerate() {
            let k = base_index(b);
            self.base_tot[k] += 1;
            let s = if i < EXACT { i } else { slot_of(i) };
            self.pos_base[s][k] += 1;
        }
        // GC per read over the read truncated to a multiple of 100 (or 1,000) bases, Ns
        // included in the length, as FastQC does.
        // Port of FastQC PerSequenceGCContent.processSequence / truncateSequence.
        let trunc = if len > 1000 { len / 1000 * 1000 } else if len > 100 { len / 100 * 100 } else { len };
        let gc = seq[..trunc].iter().filter(|&&b| matches!(b, b'G' | b'C' | b'g' | b'c')).count();
        let claims = self.gc_claims.entry(trunc).or_insert_with(|| gc_claims(trunc));
        let (a, b) = gc_range(gc, trunc);
        for (p, &c) in claims.iter().enumerate().take(b + 1).skip(a) {
            self.gc_hist[p] += 1.0 / c as f64;
        }

        if let Some(q) = qual {
            let mut qsum = 0u64;
            for (i, &c) in q.iter().enumerate().take(len) {
                self.min_qc = self.min_qc.min(c);
                self.max_qc = self.max_qc.max(c);
                let bin = (c.saturating_sub(33) as usize).min(QBINS - 1);
                self.qual_tot[bin] += 1;
                qsum += bin as u64;
                let s = if i < EXACT { i } else { slot_of(i) };
                self.pos_qual[s][bin] += 1;
            }
            self.read_meanq[(qsum / len as u64) as usize] += 1;
            self.add_tile(id, q, len.min(TILE_MAX_POS));
        } else if matches!(self.tiles, Tiles::Unknown) {
            self.tiles = Tiles::Off("no quality scores");
        }
        for (k, f) in self.finders.iter().enumerate() {
            if let Some(p) = f.find(seq) {
                let s = slot_of(p);
                let v = &mut self.adapter_start[k];
                if v.len() <= s {
                    v.resize(s + 1, 0);
                }
                v[s] += 1;
            }
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
    /// Things the reader should know about how the numbers were made (e.g. encoding guesses).
    pub warnings: Vec<String>,
    /// One entry per FastQC position group (single bases up to 9, then wider groups).
    pub per_position: Vec<PosStat>,
    /// (mean Phred of a read, number of reads)
    pub read_mean_quality: Vec<(u32, u64)>,
    /// Reads per GC percent, index 0..=100 (fractional, FastQC's GC model).
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
        /// Start of each position group.
        positions: Vec<usize>,
        /// `deviation[t][g]`: tile t's mean quality in group g minus the mean over tiles.
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
    /// First and last position of the group (1-based, inclusive).
    pub pos: usize,
    pub end: usize,
    pub mean_q: Option<f64>,
    /// Quality percentiles: FastQC's per-position percentile averaged over the group, using
    /// only positions covered by more than 100 reads.
    pub p10: Option<f64>,
    pub p25: Option<f64>,
    pub median: Option<f64>,
    pub p75: Option<f64>,
    pub p90: Option<f64>,
    /// A, C, G, T as percent of A+C+G+T at these positions; N as percent of all bases there.
    pub base_percent: [f64; 5],
}

#[derive(Serialize)]
pub struct AdapterStat {
    pub name: &'static str,
    /// Start of each position group.
    pub positions: Vec<usize>,
    /// Percent of reads with the adapter starting at or before each position, averaged over the group.
    pub percent: Vec<f64>,
    pub max_percent: f64,
}

#[derive(Serialize)]
pub struct Duplication {
    pub reads: u64,
    /// Distinct sequences tracked (first 50 bp of longer reads); counting stops adding new ones here.
    pub distinct_tracked: u64,
    /// Reads read when the last tracked sequence was first seen.
    pub reads_at_limit: u64,
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

/// FastQC's per-position percentile: the first score whose cumulative count reaches
/// `total * p / 100` (integer), counting from the encoding's zero.
// Port of FastQC QualityCount.getPercentile.
fn fq_percentile(hist: &[u64; QBINS], shift: usize, p: u64) -> f64 {
    let total: u64 = hist.iter().sum();
    let target = total * p / 100;
    let mut cum = 0;
    for (i, &c) in hist.iter().enumerate().skip(shift) {
        cum += c;
        if cum >= target {
            return (i - shift) as f64;
        }
    }
    -1.0
}

fn hist_mean(hist: &[u64; QBINS], shift: usize) -> Option<f64> {
    let n: u64 = hist.iter().skip(shift).sum();
    let s: u64 = hist.iter().enumerate().skip(shift).map(|(i, &c)| (i - shift) as u64 * c).sum();
    (n > 0).then(|| s as f64 / n as f64)
}

/// Encoding from the lowest and highest quality characters. FastQC calls anything whose lowest
/// character is `@` (64) or above Phred+64, which misreads high-quality Phred+33 data (Q31–Q40
/// only, `@`..`I`). Here Phred+64 also needs a character above `J` (74), the Phred+33 maximum.
// Port of FastQC PhredEncoding.getFastQEncodingOffset, with the Phred+64 rule changed as above.
fn phred_offset(min: u8, max: u8, forced: Option<u8>, warnings: &mut Vec<String>) -> u8 {
    if let Some(f) = forced {
        if f == 64 && min < 64 {
            warnings.push(format!("--phred 64 given, but quality characters go down to {:?}, which is below Phred+64's zero", min as char));
        }
        return f;
    }
    if min >= 64 && max > b'J' {
        return 64;
    }
    if min >= 64 {
        warnings.push(format!(
            "every quality character is between {:?} and {:?}, which fits both Phred+33 (Q{}–Q{}) and Phred+64 (Q{}–Q{}); \
             read as Phred+33 (FastQC would say Phred+64). Use --phred 64 if this is old Illumina 1.3–1.7 data",
            min as char,
            max as char,
            min - 33,
            max - 33,
            min - 64,
            max - 64
        ));
    }
    33
}

impl Acc {
    pub fn finish(self, file: String, forced_phred: Option<u8>) -> Report {
        let has_qual = self.max_qc > 0;
        let mut warnings = Vec::new();
        let offset = has_qual.then(|| phred_offset(self.min_qc, self.max_qc, forced_phred, &mut warnings));
        let shift = if offset == Some(64) { 31 } else { 0 };
        let q_at_least = |t: usize| -> u64 { self.qual_tot.iter().skip(t + shift).sum() };
        let qual_bases: u64 = self.qual_tot.iter().sum();
        let nslots = self.pos_base.len();

        let per_position: Vec<PosStat> = base_groups(self.max_len)
            .into_iter()
            .map(|(lo, hi)| {
                let mut bases = [0u64; 6];
                let mut qs = [0.0; 5];
                let mut qw = 0.0;
                let mut means = (0.0, 0.0);
                for (s, w) in slots_in(lo, hi, nslots) {
                    for (t, c) in bases.iter_mut().zip(&self.pos_base[s]) {
                        *t += c;
                    }
                    let Some(h) = self.pos_qual.get(s) else { continue };
                    let (a, b) = slot_span(s);
                    let width = (b + 1 - a) as u64;
                    if let Some(m) = hist_mean(h, shift) {
                        means.0 += m * w as f64;
                        means.1 += w as f64;
                    }
                    if h.iter().sum::<u64>() > MIN_QUAL_COVER * width {
                        for (v, p) in qs.iter_mut().zip([10, 25, 50, 75, 90]) {
                            *v += fq_percentile(h, shift, p) * w as f64;
                        }
                        qw += w as f64;
                    }
                }
                let q = |i: usize| (qw > 0.0).then(|| qs[i] / qw);
                let acgt: u64 = bases[..4].iter().sum();
                let all: u64 = bases.iter().sum();
                PosStat {
                    pos: lo,
                    end: hi,
                    mean_q: (means.1 > 0.0).then(|| means.0 / means.1),
                    p10: q(0),
                    p25: q(1),
                    median: q(2),
                    p75: q(3),
                    p90: q(4),
                    base_percent: [pct(bases[0], acgt), pct(bases[1], acgt), pct(bases[2], acgt), pct(bases[3], acgt), pct(bases[4], all)],
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

        // FastQC tracks adapter starts up to (longest read - adapter length + 1) positions.
        // Port of FastQC AdapterContent.processSequence / calculateEnrichment.
        let apos = (self.max_len + 1).saturating_sub(ADAPTER_LEN);
        let agroups = base_groups(apos);
        let adapters = ADAPTERS
            .iter()
            .zip(&self.adapter_start)
            .map(|((name, _), starts)| {
                // Reads with a hit starting at or before the end of each slot.
                let mut cum = Vec::with_capacity(starts.len());
                let mut run = 0;
                for &c in starts {
                    run += c;
                    cum.push(run);
                }
                let at = |s: usize| cum.get(s).copied().unwrap_or(run);
                let percent: Vec<f64> = agroups
                    .iter()
                    .map(|&(lo, hi)| {
                        let sum: f64 = slots_in(lo, hi, usize::MAX).map(|(s, w)| pct(at(s), self.reads) * w as f64).sum();
                        sum / (hi + 1 - lo) as f64
                    })
                    .collect();
                let max_percent = percent.iter().copied().fold(0.0, f64::max);
                AdapterStat { name, positions: agroups.iter().map(|g| g.0).collect(), percent, max_percent }
            })
            .collect();

        let duplication = Duplication {
            reads: self.dup.reads,
            distinct_tracked: self.dup.counts.len() as u64,
            reads_at_limit: self.dup.reads_at_limit,
            remaining_if_deduplicated_percent: self.dup.remaining_percent(),
        };
        let mut over: Vec<_> = self
            .dup
            .counts
            .iter()
            .filter(|(_, c)| pct(**c as u64, self.dup.reads) > 0.1)
            .map(|(s, &c)| Overrep {
                sequence: String::from_utf8_lossy(s).into_owned(),
                count: c,
                percent: pct(c as u64, self.dup.reads),
                possible_source: ADAPTERS
                    .iter()
                    .find(|(_, a)| memchr::memmem::find(s, a).is_some())
                    .map(|(n, _)| *n),
            })
            .collect();
        over.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.sequence.cmp(&b.sequence)));
        let over_total = over.len();
        over.truncate(20);

        let per_tile = match self.tiles {
            Tiles::On { map, .. } if !map.is_empty() => tile_summary(map),
            Tiles::On { .. } | Tiles::Unknown => PerTile::Skipped { reason: "no reads" },
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
            warnings,
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
        r.checks = checks(&r, self.zero_len, over_total);
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

fn span(p: &PosStat) -> String {
    if p.pos == p.end { format!("position {}", p.pos) } else { format!("positions {}–{}", p.pos, p.end) }
}

/// Thresholds are FastQC 0.12.1's defaults (`Configuration/limits.txt`).
// Port of FastQC Configuration/limits.txt and each module's raisesError/raisesWarning.
fn checks(r: &Report, zero_len: u64, over_total: usize) -> Vec<Check> {
    let mut out = Vec::new();
    if r.phred_offset.is_some() {
        let judged: Vec<&PosStat> = r.per_position.iter().filter(|p| p.median.is_some()).collect();
        let worst_median = judged.iter().min_by(|a, b| a.median.partial_cmp(&b.median).unwrap());
        let worst_lq = judged.iter().min_by(|a, b| a.p25.partial_cmp(&b.p25).unwrap());
        let (status, detail) = match (worst_median, worst_lq) {
            (Some(m), Some(l)) => {
                let (mv, lv) = (m.median.unwrap(), l.p25.unwrap());
                let status = grade(mv, 25.0, 20.0, false).max(grade(lv, 10.0, 5.0, false));
                let skipped = r.per_position.len() - judged.len();
                let note = if skipped > 0 {
                    format!("; {skipped} position groups covered by ≤ 100 reads not judged, as in FastQC")
                } else {
                    String::new()
                };
                (status, format!("lowest median Q{mv:.1} ({}), lowest lower quartile Q{lv:.1} ({}){note}", span(m), span(l)))
            }
            _ => (Status::Pass, "no position covered by more than 100 reads".into()),
        };
        out.push(Check { name: "Per-position quality", status, detail });
        // Most common mean quality; ties go to the lower score, as in FastQC.
        // Port of FastQC PerSequenceQualityScores.raisesError/raisesWarning.
        let mode = r.read_mean_quality.iter().fold((0u32, 0u64), |best, &(q, c)| if c > best.1 { (q, c) } else { best }).0 as f64;
        let status = if mode <= 20.0 {
            Status::Fail
        } else if mode <= 27.0 {
            Status::Warn
        } else {
            Status::Pass
        };
        out.push(Check { name: "Per-read quality", status, detail: format!("most common mean read quality Q{mode:.0}") });
    }
    let worst = r
        .per_position
        .iter()
        .map(|p| {
            let b = p.base_percent;
            ((b[0] - b[3]).abs().max((b[1] - b[2]).abs()), p)
        })
        .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let (worst_diff, where_) = worst.map(|(d, p)| (d, format!(" at {}", span(p)))).unwrap_or((0.0, String::new()));
    out.push(Check {
        name: "Per-position base content",
        status: grade(worst_diff, 10.0, 20.0, true),
        detail: format!("largest A/T or G/C imbalance {worst_diff:.1}%{where_}"),
    });
    out.push(gc_check(&r.gc_distribution));
    let worst_n = r.per_position.iter().map(|p| p.base_percent[4]).fold(0.0, f64::max);
    out.push(Check {
        name: "N content",
        status: grade(worst_n, 5.0, 20.0, true),
        detail: format!("highest N rate in a position group {worst_n:.2}%"),
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
            status: grade(worst_deviation.abs(), 5.0, 10.0, true),
            detail: format!(
                "{} tiles; worst: tile {worst_tile} from position {worst_position}, {:.1} {} the average",
                tiles.len(),
                worst_deviation.abs(),
                if *worst_deviation < 0.0 { "below" } else { "above" }
            ),
        });
    }
    let d = &r.duplication;
    let rem = d.remaining_if_deduplicated_percent;
    out.push(Check {
        name: "Duplication",
        status: grade(rem, 70.0, 50.0, false),
        detail: format!(
            "{rem:.1}% of reads would remain after deduplication (estimated from {} distinct sequences tracked over all {} reads)",
            d.distinct_tracked, d.reads
        ),
    });
    let top = r.overrepresented.first().map(|o| o.percent).unwrap_or(0.0);
    out.push(Check {
        name: "Overrepresented sequences",
        status: grade(top, 0.1, 1.0, true),
        detail: match over_total {
            1 => "1 sequence above 0.1% of reads".into(),
            n => format!("{n} sequences above 0.1% of reads"),
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
        detail: format!("{aname}: up to {amax:.2}% of reads"),
    });
    out
}

/// FastQC's per-sequence GC check: fit a normal curve centred on the histogram's mode (the
/// average of the bins within 10% of the peak, unless that run reaches either end), with the
/// spread measured around that mode, and count how far the histogram is from it.
// Port of FastQC PerSequenceGCContent.calculateDistribution.
fn gc_check(hist: &[f64]) -> Check {
    let n: f64 = hist.iter().sum();
    if n == 0.0 {
        return Check { name: "Per-read GC", status: Status::Pass, detail: "no reads".into() };
    }
    let mut first_mode = 0;
    for (i, &c) in hist.iter().enumerate() {
        if c > hist[first_mode] {
            first_mode = i;
        }
    }
    let near = |c: f64| c > hist[first_mode] - hist[first_mode] / 10.0;
    let (mut sum, mut k) = (0.0, 0usize);
    let mut off_top = true;
    for (i, &c) in hist.iter().enumerate().skip(first_mode) {
        if near(c) {
            sum += i as f64;
            k += 1;
        } else {
            off_top = false;
            break;
        }
    }
    let mut off_bottom = true;
    for i in (0..first_mode).rev() {
        if near(hist[i]) {
            sum += i as f64;
            k += 1;
        } else {
            off_bottom = false;
            break;
        }
    }
    let mode = if off_top || off_bottom { first_mode as f64 } else { sum / k as f64 };
    let var = hist.iter().enumerate().map(|(i, &c)| (i as f64 - mode).powi(2) * c).sum::<f64>() / (n - 1.0);
    let sd = var.sqrt();
    if !(sd.is_finite() && sd > 0.0) {
        // FastQC's curve is undefined here (NaN), so it raises nothing.
        return Check { name: "Per-read GC", status: Status::Pass, detail: format!("mode {mode:.1}% GC; too few distinct GC values to fit a curve") };
    }
    let deviation: f64 = hist
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let z = (i as f64 - mode) / sd;
            let density = (-(z * z) / 2.0).exp() / (sd * (2.0 * std::f64::consts::PI).sqrt());
            let expected = if density.is_finite() { density * n } else { 0.0 };
            (c - expected).abs()
        })
        .sum::<f64>()
        * 100.0
        / n;
    Check {
        name: "Per-read GC",
        status: grade(deviation, 15.0, 30.0, true),
        detail: format!("mode {mode:.1}% GC, {deviation:.1}% of reads off a normal curve"),
    }
}

/// Mean quality per flowcell tile and position, for spotting bad patches of the flowcell.
/// Follows FastQC: the first read's header decides which `:`-separated field holds the tile, a
/// later header that doesn't fit switches the module off, and after the first 10,000 reads only
/// every 10th read is used.
// Port of FastQC PerTileQualityScores.processSequence.
enum Tiles {
    Unknown,
    On { field: usize, reads: u64, map: HashMap<u32, TileAcc> },
    Off(&'static str),
}

#[derive(Default)]
struct TileAcc {
    sum: Vec<u64>,
    n: Vec<u64>,
}

/// Which `:`-separated field of a header holds the tile, using FastQC's rule: with 7+ fields
/// (CASAVA 1.8+, `@M00:RUN:FC:LANE:TILE:X:Y 1:N:0:...`) the 5th, with 5–6 fields (older
/// `@HWI:LANE:TILE:X:Y#IDX/1`) the 3rd.
// Port of FastQC PerTileQualityScores.processSequence (choosing the tile field).
fn tile_field(id: &[u8]) -> Option<usize> {
    match memchr::memchr_iter(b':', id).count() + 1 {
        n if n >= 7 => Some(4),
        n if n >= 5 => Some(2),
        _ => None,
    }
}

fn tile_at(id: &[u8], field: usize) -> Option<u32> {
    let raw = id.split(|&c| c == b':').nth(field)?;
    std::str::from_utf8(raw).ok()?.parse().ok()
}

/// Tile number from a header, choosing the field from that header alone.
#[cfg(test)]
pub fn parse_tile(id: &[u8]) -> Option<u32> {
    tile_at(id, tile_field(id)?)
}

impl Acc {
    fn add_tile(&mut self, id: &[u8], qual: &[u8], npos: usize) {
        if qual.is_empty() {
            return;
        }
        if matches!(self.tiles, Tiles::Unknown) {
            self.tiles = match tile_field(id) {
                Some(field) => Tiles::On { field, reads: 0, map: HashMap::new() },
                None => Tiles::Off("read names carry no Illumina tile number"),
            };
        }
        let Tiles::On { field, reads, map } = &mut self.tiles else { return };
        *reads += 1;
        if *reads > 10_000 && *reads % 10 != 0 {
            return;
        }
        let Some(tile) = tile_at(id, *field) else {
            self.tiles = Tiles::Off("a read name has no tile number where the first one had it (FastQC drops the module too)");
            return;
        };
        if !map.contains_key(&tile) && map.len() > MAX_TILES {
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
}

/// Each tile's mean quality per position group (the mean of its per-position means; 0 where
/// the tile has no data, as in FastQC), minus the average over tiles. The check uses the
/// largest deviation either way.
// Port of FastQC PerTileQualityScores.getPercentages.
fn tile_summary(map: HashMap<u32, TileAcc>) -> PerTile {
    let mut tiles: Vec<_> = map.into_iter().collect();
    tiles.sort_unstable_by_key(|(t, _)| *t);
    let npos = tiles.iter().map(|(_, a)| a.sum.len()).max().unwrap_or(0);
    let groups = base_groups(npos);
    let means: Vec<Vec<f64>> = tiles
        .iter()
        .map(|(_, a)| {
            groups
                .iter()
                .map(|&(lo, hi)| {
                    let m: Vec<f64> =
                        (lo - 1..hi).filter(|&p| a.n.get(p).copied().unwrap_or(0) > 0).map(|p| a.sum[p] as f64 / a.n[p] as f64).collect();
                    if m.is_empty() { 0.0 } else { m.iter().sum::<f64>() / m.len() as f64 }
                })
                .collect()
        })
        .collect();
    let avg: Vec<f64> = (0..groups.len()).map(|g| means.iter().map(|m| m[g]).sum::<f64>() / tiles.len() as f64).collect();
    let (mut worst, mut worst_tile, mut worst_position) = (0.0f64, tiles[0].0, 1);
    for (g, &(lo, _)) in groups.iter().enumerate() {
        for (m, (tile, _)) in means.iter().zip(&tiles) {
            let d = m[g] - avg[g];
            if d.abs() > worst.abs() {
                (worst, worst_tile, worst_position) = (d, *tile, lo);
            }
        }
    }
    let deviation = means.iter().map(|m| m.iter().zip(&avg).map(|(v, a)| ((v - a) * 100.0).round() as f32 / 100.0).collect()).collect();
    PerTile::Measured {
        tiles: tiles.iter().map(|(t, _)| *t).collect(),
        positions: groups.iter().map(|g| g.0).collect(),
        deviation,
        worst_tile,
        worst_position,
        worst_deviation: worst,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check(r: &Report, name: &str) -> Status {
        r.checks.iter().find(|c| c.name == name).unwrap().status
    }

    #[test]
    fn basic_counts() {
        let mut a = Acc::new(10);
        a.add(b"r", b"ACGTN", Some(b"IIII#")); // Q40 x4, Q2
        a.add(b"r", b"GGCC", Some(b"5555")); // Q20
        let r = a.finish("t".into(), None);
        assert_eq!(r.reads, 2);
        assert_eq!(r.bases, 9);
        assert_eq!(r.phred_offset, Some(33));
        assert!((r.gc_percent - 75.0).abs() < 1e-9); // 6 of 8 called bases
        assert!((r.q20_percent.unwrap() - 800.0 / 9.0).abs() < 1e-9);
        assert_eq!(r.per_position[0].base_percent[0], 50.0);
        assert_eq!(r.duplication.distinct_tracked, 2);
    }

    #[test]
    fn base_content_ignores_n_like_fastqc() {
        // Position 1: A, A, N, N. FastQC: A = 100% of A+C+G+T; N = 50% of all bases.
        let mut a = Acc::new(10);
        for s in [b"AC", b"AC", b"NC", b"NC"] {
            a.add(b"r", s, None);
        }
        let r = a.finish("t".into(), None);
        assert_eq!(r.per_position[0].base_percent, [100.0, 0.0, 0.0, 0.0, 50.0]);
    }

    #[test]
    fn high_quality_phred33_is_not_mistaken_for_phred64() {
        // Every quality between '@' (Q31) and 'I' (Q40): valid Phred+33, and FastQC's rule
        // (lowest character >= 64) would call it Phred+64.
        let mut a = Acc::new(10);
        for _ in 0..200 {
            a.add(b"r", b"ACGTACGTAC", Some(b"@@IIIIIIII"));
        }
        let r = a.finish("t".into(), None);
        assert_eq!(r.phred_offset, Some(33));
        assert_eq!(r.q30_percent, Some(100.0));
        assert_eq!(check(&r, "Per-position quality"), Status::Pass);
        assert!(r.warnings[0].contains("Phred+33"), "{:?}", r.warnings);

        let mut a = Acc::new(10);
        a.add(b"r", b"ACGT", Some(b"@@IJ"));
        assert_eq!(a.finish("t".into(), Some(64)).phred_offset, Some(64));

        // Real Phred+64 data uses characters above 'J'.
        let mut a = Acc::new(10);
        a.add(b"r", b"ACGT", Some(b"BBhh"));
        let r = a.finish("t".into(), None);
        assert_eq!((r.phred_offset, r.warnings.len()), (Some(64), 0));
    }

    #[test]
    fn quality_beyond_1kb_is_judged() {
        // 5 kb reads, Q40 for the first 1,000 bases and Q2 after.
        let seq = b"ACGT".repeat(1250);
        let mut qual = vec![b'I'; 1000];
        qual.resize(5000, b'#');
        let mut a = Acc::new(1000);
        for _ in 0..150 {
            a.add(b"r", &seq, Some(&qual));
        }
        let r = a.finish("t".into(), None);
        assert_eq!(r.per_position.last().unwrap().end, 5000);
        assert_eq!(check(&r, "Per-position quality"), Status::Fail);
    }

    #[test]
    fn groups_match_fastqc() {
        assert_eq!(base_groups(5), vec![(1, 1), (2, 2), (3, 3), (4, 4), (5, 5)]);
        let g = base_groups(101);
        assert_eq!(&g[..10], &[(1, 1), (2, 2), (3, 3), (4, 4), (5, 5), (6, 6), (7, 7), (8, 8), (9, 9), (10, 11)]);
        assert_eq!(*g.last().unwrap(), (100, 101));
        // Above 1 kb every boundary is a multiple of 20, so 10 bp slots never straddle a group.
        for len in [1001, 1500, 5000, 99_999, 250_000] {
            for (lo, hi) in base_groups(len) {
                for (s, _) in slots_in(lo, hi, usize::MAX) {
                    let (a, b) = slot_span(s);
                    assert!(a >= lo && (b <= hi || hi == len), "len {len}: slot {a}-{b} straddles {lo}-{hi}");
                }
            }
        }
    }

    #[test]
    fn slots() {
        assert_eq!((slot_of(0), slot_span(0)), (0, (1, 1)));
        assert_eq!((slot_of(998), slot_span(998)), (998, (999, 999)));
        assert_eq!(slot_span(slot_of(999)), (1000, 1009));
        assert_eq!(slot_span(slot_of(1009)), (1010, 1019));
        assert_eq!(slot_span(slot_of(99_998)), (99_990, 99_999));
        assert_eq!(slot_span(slot_of(99_999)), (100_000, 100_999));
    }

    #[test]
    fn finds_adapters() {
        let mut a = Acc::new(10);
        a.add(b"r", b"TTTTAGATCGGAAGAGCACAC", None);
        a.add(b"r", b"TTTTTTTTTTTTTTTTTTTTT", None);
        let r = a.finish("t".into(), None);
        let u = &r.adapters[0];
        assert_eq!(u.positions.len(), 21 + 1 - 12);
        assert_eq!(u.percent[3], 0.0);
        assert_eq!(u.percent[4], 50.0);
        assert_eq!(r.q30_percent, None);
    }

    #[test]
    fn gc_model_matches_fastqc() {
        // FastQC GCModel for 2-base reads: 0 GC → bins 0..=25, 1 → 25..=75, 2 → 75..=100;
        // bins 25 and 75 are claimed twice.
        let c = gc_claims(2);
        assert_eq!((c[0], c[25], c[50], c[75], c[100]), (1, 2, 1, 2, 1));
        assert_eq!(gc_range(1, 2), (25, 75));
        // Reads over 100 bp are truncated to a multiple of 100: GC of the first 100 bases only.
        let mut a = Acc::new(10);
        let mut seq = b"GC".repeat(50);
        seq.extend_from_slice(b"AAAAAAAAAAAAAAAAAAAA");
        a.add(b"r", &seq, None);
        let r = a.finish("t".into(), None);
        assert!(r.gc_distribution[100] > 0.0 && r.gc_distribution[90] == 0.0);
    }

    #[test]
    fn gc_curve_is_centred_on_the_mode() {
        // Bimodal: a big peak at 30% and a smaller one at 70%; the mean (~43%) would be wrong.
        let mut h = vec![0.0; 101];
        for (i, v) in h.iter_mut().enumerate() {
            let x = i as f64;
            *v = 1000.0 * (-((x - 30.0) / 4.0).powi(2) / 2.0).exp() + 300.0 * (-((x - 70.0) / 4.0).powi(2) / 2.0).exp();
        }
        let c = gc_check(&h);
        assert!(c.detail.starts_with("mode 30.0% GC"), "{}", c.detail);
        assert_eq!(c.status, Status::Fail);
    }

    #[test]
    fn duplication_tracks_distinct_sequences_through_the_file() {
        // 3 distinct tracked, then a flood of copies of the first: counted, though past the limit.
        let mut a = Acc::new(3);
        for s in [b"AAAA", b"CCCC", b"GGGG", b"TTTT"] {
            a.add(b"r", s, None);
        }
        for _ in 0..96 {
            a.add(b"r", b"AAAA", None);
        }
        let r = a.finish("t".into(), None);
        assert_eq!(r.duplication.distinct_tracked, 3);
        assert_eq!(r.overrepresented[0].count, 97);
        assert!((r.overrepresented[0].percent - 97.0).abs() < 1e-9);
        assert_eq!(check(&r, "Duplication"), Status::Fail);
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
        let r = a.finish("t".into(), None);
        let PerTile::Measured { worst_tile, worst_position, worst_deviation, .. } = r.per_tile else { panic!() };
        assert_eq!((worst_tile, worst_position), (1103, 3));
        assert!((worst_deviation + 20.0).abs() < 1e-9); // 10 - mean(40,40,10)
        assert_eq!(check(&r, "Per-tile quality"), Status::Fail);
    }

    #[test]
    fn a_header_without_a_tile_switches_per_tile_off() {
        let mut a = Acc::new(10);
        a.add(b"I:1:FC:1:1101:1:1", b"ACGT", Some(b"IIII"));
        a.add(b"read 2", b"ACGT", Some(b"IIII"));
        let r = a.finish("t".into(), None);
        assert!(matches!(r.per_tile, PerTile::Skipped { .. }));
        assert!(r.checks.iter().all(|c| c.name != "Per-tile quality"));
    }

    #[test]
    fn a_tile_far_above_average_also_counts() {
        let mut a = Acc::new(10);
        for _ in 0..50 {
            a.add(b"I:1:FC:1:1101:1:1", b"ACGT", Some(b"++++")); // Q10
            a.add(b"I:1:FC:1:1102:1:1", b"ACGT", Some(b"++++"));
            a.add(b"I:1:FC:1:1103:1:1", b"ACGT", Some(b"IIII")); // Q40: 20 above the mean of 20
        }
        let r = a.finish("t".into(), None);
        assert_eq!(check(&r, "Per-tile quality"), Status::Fail);
    }
}
