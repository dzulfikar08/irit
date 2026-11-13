//! The track header below the ruler, rasterised as image tiles: consensus, identity and gaps,
//! `#=GC` / `#=GR` annotation tracks and the pinned reference row. Tiles are 256 device pixels
//! wide and the header's height, drawn once per [`HeaderKey`] and reused while the view pans or
//! repaints; only the ruler's text and the letters are painted every frame.

use std::collections::HashMap;
use std::hash::Hash;

use crate::annot::{Ss, TrackKind, pp_value, ss_class};
use crate::reference::Reference;
use crate::scheme::{ColorMode, Group, Theme, mix};
use crate::view::{Scene, TILE, aln_col};

/// Colours of the annotation tracks (0xRRGGBB).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AnnotColors {
    pub helix: u32,
    pub strand: u32,
    pub pair: u32,
    pub pair_close: u32,
    pub coil: u32,
    /// Behind the characters of a text track.
    pub shade: u32,
    pub pp_hi: u32,
    pub pp_lo: u32,
}

impl AnnotColors {
    pub fn new(theme: &Theme) -> AnnotColors {
        let strong = |g: Group| g.colors()[if theme.dark { 3 } else { 1 }];
        let pair = strong(Group::Hydrophobic);
        AnnotColors {
            helix: strong(Group::Positive),
            strand: strong(Group::Glycine),
            pair,
            pair_close: mix(pair, theme.bg, 0.4),
            coil: mix(theme.bg, theme.fg, 0.45),
            shade: mix(theme.bg, theme.fg, 0.09),
            pp_hi: mix(theme.bg, theme.accent, 0.85),
            pp_lo: mix(theme.bg, theme.fg, 0.35),
        }
    }
}

/// What one band of the header shows.
#[derive(Clone, Copy)]
pub enum Band<'a> {
    /// The consensus residue in its scheme colour.
    Consensus,
    /// Bars of the share of rows carrying the consensus residue (or the reference's, see
    /// [`Strip::identity_ref`]); `hi` at 90% or more.
    Identity { hi: u32, lo: u32 },
    /// Bars of the gap share.
    Gaps { ink: u32 },
    /// An annotation line, one byte per alignment column.
    Annot { data: &'a [u8], kind: TrackKind },
    /// A row's cells (the pinned reference), one code per alignment column.
    Row { codes: &'a [u8] },
}

/// A band at `y` (logical px from the top of the strip), `h` tall.
#[derive(Clone, Copy)]
pub struct BandSpec<'a> {
    pub y: f32,
    pub h: f32,
    pub band: Band<'a>,
}

/// The track header strip: its bands over the scene's columns.
pub struct Strip<'a> {
    pub scene: &'a Scene<'a>,
    pub bands: Vec<BandSpec<'a>>,
    /// The identity bars measure the reference's residue instead of the consensus.
    pub identity_ref: Option<&'a Reference>,
    pub bg: u32,
    pub annot: AnnotColors,
    /// Device pixels per logical pixel.
    pub scale: f64,
}

/// Fills pixel column `i` of a strip from logical y0 to y1 with a colour.
type Rect<'r> = dyn Fn(&mut [u8], usize, f64, f64, u32) + 'r;

/// Up to this many columns are sampled per pixel.
const SAMPLES: usize = 16;

impl Strip<'_> {
    /// Height of the strip in device pixels.
    pub fn height_dev(&self) -> usize {
        self.bands
            .iter()
            .map(|b| ((b.y + b.h) as f64 * self.scale).round() as usize)
            .max()
            .unwrap_or(0)
    }

    /// Rasterise the `w` pixels from device x `x0` of the strip at `cw` device pixels per column
    /// into `out` (BGRA, `w` × [`Strip::height_dev`]).
    pub fn render(&self, cw: f64, x0: i64, w: usize, out: &mut [u8]) {
        let h = self.height_dev();
        let s = self.scene;
        let cols = s.view_cols();
        out.fill(0);
        let rect = |out: &mut [u8], i: usize, y0: f64, y1: f64, rgb: u32| {
            let (a, b) = (
                (y0 * self.scale).round().max(0.0) as usize,
                ((y1 * self.scale).round() as usize).min(h),
            );
            for j in a..b {
                out[(j * w + i) * 4..][..4].copy_from_slice(&[
                    rgb as u8,
                    (rgb >> 8) as u8,
                    (rgb >> 16) as u8,
                    255,
                ]);
            }
        };
        // View columns sampled for each pixel; empty past the alignment.
        let samples: Vec<Vec<usize>> = (0..w)
            .map(|i| {
                let a = (x0 + i as i64) as f64 / cw;
                let b = (x0 + i as i64 + 1) as f64 / cw;
                if a < 0.0 || a as usize >= cols {
                    return Vec::new();
                }
                let (a, b) = (a as usize, (b.ceil() as usize).clamp(a as usize + 1, cols));
                let step = (b - a).div_ceil(SAMPLES).max(1);
                (a..b).step_by(step).map(|v| aln_col(s.map, v)).collect()
            })
            .collect();
        for (i, cs) in samples.iter().enumerate() {
            if !cs.is_empty() {
                rect(out, i, 0.0, h as f64 / self.scale, self.bg);
            }
        }
        for b in &self.bands {
            let (y, bh) = (b.y as f64, b.h as f64);
            match b.band {
                Band::Consensus => {
                    for (i, cs) in samples.iter().enumerate() {
                        let Some(&c) = cs.first() else { continue };
                        if let Some(f) = s.colors.fill(s.stats.consensus[c], &s.column(c)) {
                            rect(out, i, y + 1.0, y + bh - 1.0, f);
                        }
                    }
                }
                Band::Identity { hi, lo } => {
                    for (i, cs) in samples.iter().enumerate() {
                        if cs.is_empty() {
                            continue;
                        }
                        let id = cs
                            .iter()
                            .map(|&c| {
                                self.identity_ref
                                    .map_or(s.stats.identity[c], |r| r.identity(s.stats, c))
                            })
                            .sum::<f32>()
                            / cs.len() as f32;
                        let hq = (id as f64 * (bh - 6.0)).round();
                        rect(
                            out,
                            i,
                            y + bh - 2.0 - hq,
                            y + bh - 2.0,
                            if id >= 0.9 { hi } else { lo },
                        );
                    }
                }
                Band::Gaps { ink } => {
                    for (i, cs) in samples.iter().enumerate() {
                        if cs.is_empty() {
                            continue;
                        }
                        let g = cs.iter().map(|&c| s.stats.gaps[c]).sum::<f32>() / cs.len() as f32;
                        let gq = (g as f64 * (bh - 2.0)).round();
                        rect(out, i, y + 1.0, y + 1.0 + gq, ink);
                    }
                }
                Band::Row { codes } => {
                    for (i, cs) in samples.iter().enumerate() {
                        let Some(&c) = cs.first() else { continue };
                        rect(out, i, y, y + bh, s.colors.cell(codes[c], &s.column(c)));
                    }
                }
                Band::Annot { data, kind } => self.annot(data, kind, &samples, y, bh, out, &rect),
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn annot(
        &self,
        data: &[u8],
        kind: TrackKind,
        samples: &[Vec<usize>],
        y: f64,
        bh: f64,
        out: &mut [u8],
        rect: &Rect,
    ) {
        let a = &self.annot;
        let mid = y + bh / 2.0;
        match kind {
            TrackKind::Structure => {
                // The most common class among a pixel's columns (unannotated ones only when
                // nothing else is there).
                let class: Vec<Ss> = samples
                    .iter()
                    .map(|cs| majority_ss(cs.iter().map(|&c| ss_class(data[c]))))
                    .collect();
                // Strands end in an arrowhead this many device pixels long, when long enough.
                let head = (5.0 * self.scale).round() as usize;
                let mut i = 0;
                while i < class.len() {
                    let k = class[i];
                    let j = class[i..]
                        .iter()
                        .position(|&x| x != k)
                        .map_or(class.len(), |n| i + n);
                    for p in i..j {
                        match k {
                            Ss::Helix => rect(out, p, mid - 4.0, mid + 4.0, a.helix),
                            Ss::Strand => {
                                let d = j - 1 - p;
                                if j - i > head && d < head {
                                    let half = (bh / 2.0 - 2.0) * (d as f64 + 0.5) / head as f64;
                                    rect(
                                        out,
                                        p,
                                        mid - half.max(0.5),
                                        mid + half.max(0.5),
                                        a.strand,
                                    );
                                } else {
                                    rect(out, p, mid - 3.0, mid + 3.0, a.strand);
                                }
                            }
                            Ss::PairOpen => rect(out, p, mid - 3.0, mid + 3.0, a.pair),
                            Ss::PairClose => rect(out, p, mid - 3.0, mid + 3.0, a.pair_close),
                            Ss::Coil => rect(out, p, mid - 0.5, mid + 0.5, a.coil),
                            Ss::None => {}
                        }
                    }
                    i = j;
                }
            }
            TrackKind::Probability => {
                for (i, cs) in samples.iter().enumerate() {
                    let (sum, n) = cs
                        .iter()
                        .filter_map(|&c| pp_value(data[c]))
                        .fold((0.0f32, 0), |(s, n), v| (s + v, n + 1));
                    if n == 0 {
                        continue;
                    }
                    let v = sum / n as f32;
                    let hq = (v as f64 * (bh - 4.0)).round().max(1.0);
                    rect(
                        out,
                        i,
                        y + bh - 2.0 - hq,
                        y + bh - 2.0,
                        if v >= 0.9 { a.pp_hi } else { a.pp_lo },
                    );
                }
            }
            TrackKind::Text => {
                for (i, cs) in samples.iter().enumerate() {
                    if cs.iter().any(|&c| !is_blank(data[c])) {
                        rect(out, i, y + 1.0, y + bh - 1.0, a.shade);
                    }
                }
            }
        }
    }
}

/// Gap-like characters of annotation lines.
pub fn is_blank(b: u8) -> bool {
    matches!(b, b'.' | b'-' | b'_' | b'~' | b' ')
}

fn majority_ss(it: impl Iterator<Item = Ss>) -> Ss {
    let order = [Ss::Helix, Ss::Strand, Ss::PairOpen, Ss::PairClose, Ss::Coil];
    let mut n = [0u32; 5];
    for k in it {
        if let Some(i) = order.iter().position(|&o| o == k) {
            n[i] += 1;
        }
    }
    let (i, &best) = n
        .iter()
        .enumerate()
        .max_by_key(|&(i, &v)| (v, std::cmp::Reverse(i)))
        .unwrap();
    if best == 0 { Ss::None } else { order[i] }
}

/// What a set of header tiles was drawn with. Any change starts a new set.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct HeaderKey {
    /// Device pixels per column (bits) and display scale × 100.
    pub cw: u64,
    pub scale: u32,
    pub mode: ColorMode,
    pub theme: Theme,
    /// Colours from the GUI's tokens (bars, background).
    pub inks: [u32; 4],
    /// The layout generation: hidden columns, the row order, what Differences compares with.
    pub generation: u64,
    /// The reference row (identity bars and the pinned row).
    pub reference: Option<u32>,
    /// The bands: kind and logical position of each, as (tag, y, h).
    pub bands: Vec<(u64, u32, u32)>,
}

/// Rasterised tiles for one key, by tile index, with the frame each was last used in.
pub struct TileStore<K, I, V> {
    pub key: Option<K>,
    pub tiles: HashMap<I, (V, u64)>,
    pub frame: u64,
    /// Tiles drawn since the counter was last reset.
    pub rendered: usize,
}

impl<K, I, V> Default for TileStore<K, I, V> {
    fn default() -> Self {
        TileStore {
            key: None,
            tiles: HashMap::new(),
            frame: 0,
            rendered: 0,
        }
    }
}

impl<K: PartialEq, I: Hash + Eq + Copy + Ord, V> TileStore<K, I, V> {
    /// Use `key` for this frame: when it differs from the last one, every tile is removed and
    /// returned (to be freed).
    pub fn begin(&mut self, key: K) -> Vec<V> {
        self.frame += 1;
        if self.key.as_ref() == Some(&key) {
            return Vec::new();
        }
        self.key = Some(key);
        self.clear()
    }

    pub fn clear(&mut self) -> Vec<V> {
        self.tiles.drain().map(|(_, (v, _))| v).collect()
    }

    pub fn insert(&mut self, i: I, v: V) {
        self.rendered += 1;
        self.tiles.insert(i, (v, self.frame));
    }

    /// The tile at `i`, marked as used in this frame.
    pub fn get(&mut self, i: &I) -> Option<&V> {
        let f = self.frame;
        self.tiles.get_mut(i).map(|e| {
            e.1 = f;
            &e.0
        })
    }

    pub fn missing<'a>(&self, wanted: impl IntoIterator<Item = &'a I>) -> Vec<I>
    where
        I: 'a,
    {
        wanted
            .into_iter()
            .filter(|i| !self.tiles.contains_key(i))
            .copied()
            .collect()
    }

    /// Keep the `keep` most recently used tiles; return the others (to be freed).
    pub fn evict(&mut self, keep: usize) -> Vec<V> {
        if self.tiles.len() <= keep {
            return Vec::new();
        }
        let mut by_age: Vec<(u64, I)> = self.tiles.iter().map(|(k, (_, t))| (*t, *k)).collect();
        by_age.sort_unstable();
        let n = self.tiles.len() - keep;
        by_age
            .into_iter()
            .take(n)
            .filter_map(|(_, k)| self.tiles.remove(&k).map(|e| e.0))
            .collect()
    }
}

/// Header tiles along x covering device pixels `[x0, x0 + width)`.
pub fn tiles_x(x0: f64, width: f64) -> std::ops::Range<i64> {
    let t = TILE as f64;
    (x0 / t).floor() as i64..((x0 + width) / t).ceil() as i64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_bytes;
    use crate::pyramid::Pyramid;
    use crate::scheme::{Colors, Scheme};
    use crate::stats::ColumnStats;
    use crate::view::ColumnMap;

    fn px(out: &[u8], w: usize, i: usize, j: usize) -> u32 {
        let p = &out[(j * w + i) * 4..][..4];
        (p[2] as u32) << 16 | (p[1] as u32) << 8 | p[0] as u32
    }

    /// Annotation tracks follow the column map: with the insert columns hidden, each pixel shows
    /// the annotation of the alignment column it stands for.
    #[test]
    fn tracks_follow_hidden_columns() {
        let a = parse_bytes(b"# STOCKHOLM 1.0\nx AC..DEFG\ny AC.aDEFG\n#=GC SS_cons HHxxEE-.\n#=GC PP_cons 9*..0099\n//\n").unwrap();
        let s = ColumnStats::compute(&a);
        let map = ColumnMap::hiding(&a, &s, true, None).unwrap();
        assert_eq!(map.visible, [0, 1, 4, 5, 6, 7]);
        let pyr = Pyramid::build_mapped(&a, &s.consensus, Some(&map.visible), None);
        let colors = Colors::new(
            a.alphabet(),
            ColorMode::new(Scheme::ClustalX),
            Theme::LIGHT,
            true,
        );
        let scene = Scene {
            aln: &a,
            stats: &s,
            pyramid: &pyr,
            map: Some(&map),
            rows: None,
            colors: &colors,
            base: None,
        };
        let ac = AnnotColors::new(&Theme::LIGHT);
        let strip = Strip {
            scene: &scene,
            bands: vec![
                BandSpec {
                    y: 0.0,
                    h: 14.0,
                    band: Band::Annot {
                        data: &a.gc[0].data,
                        kind: TrackKind::Structure,
                    },
                },
                BandSpec {
                    y: 14.0,
                    h: 14.0,
                    band: Band::Annot {
                        data: &a.gc[1].data,
                        kind: TrackKind::Probability,
                    },
                },
            ],
            identity_ref: None,
            bg: 0x123456,
            annot: ac,
            scale: 1.0,
        };
        let (cw, w) = (10.0, 64);
        let h = strip.height_dev();
        assert_eq!(h, 28);
        let mut out = vec![0u8; w * h * 4];
        strip.render(cw, 0, w, &mut out);
        let mid = |v: usize| px(&out, w, v * 10 + 5, 7);
        // view columns 0, 1: helix; 2, 3 (alignment 4, 5): strand; 4 (alignment 6): coil line
        assert_eq!((mid(0), mid(1)), (ac.helix, ac.helix));
        assert_eq!((mid(2), mid(3)), (ac.strand, ac.strand));
        assert_eq!(mid(4), ac.coil);
        assert_eq!(mid(5), 0x123456); // '.': nothing
        // past the alignment: transparent
        assert_eq!(&out[(7 * w + 63) * 4..][..4], [0, 0, 0, 0]);
        // PP: '9' at view 0 is a high bar (0.9 counts as high), '0' at view 2 (alignment 4) a
        // 1 px bar in the low colour
        let bottom = |v: usize| px(&out, w, v * 10 + 5, 14 + 14 - 3);
        assert_eq!(bottom(0), ac.pp_hi);
        assert_eq!(bottom(2), ac.pp_lo);
        let top = |v: usize| px(&out, w, v * 10 + 5, 14 + 3);
        assert_eq!(top(2), 0x123456);
    }

    #[test]
    fn identity_bars_compare_with_the_reference() {
        let a = parse_bytes(b">r\nAAAC\n>x\nACAC\n>y\nCCAC\n").unwrap();
        let s = ColumnStats::compute(&a);
        let pyr = Pyramid::build(&a, &s.consensus);
        let colors = Colors::new(
            a.alphabet(),
            ColorMode::new(Scheme::Nucleotide),
            Theme::LIGHT,
            true,
        );
        let r = Reference::new(&a, 0);
        let scene = Scene {
            aln: &a,
            stats: &s,
            pyramid: &pyr,
            map: None,
            rows: None,
            colors: &colors,
            base: Some(&r.codes),
        };
        let (hi, lo) = (0x00ff00, 0x0000ff);
        let mut strip = Strip {
            scene: &scene,
            bands: vec![BandSpec {
                y: 0.0,
                h: 30.0,
                band: Band::Identity { hi, lo },
            }],
            identity_ref: None,
            bg: 0xffffff,
            annot: AnnotColors::new(&Theme::LIGHT),
            scale: 1.0,
        };
        let bar = |strip: &Strip, v: usize| {
            let mut out = vec![0u8; 4 * 30 * 4];
            strip.render(1.0, 0, 4, &mut out);
            (0..30).filter(|&j| px(&out, 4, v, j) != 0xffffff).count()
        };
        // column 1: consensus C (2 of 3) → bar 2/3 × 24 = 16 px; the reference's A is 1 of 3 → 8 px
        assert_eq!(bar(&strip, 1), 16);
        strip.identity_ref = Some(&r);
        assert_eq!(bar(&strip, 1), 8);
        assert_eq!(bar(&strip, 2), 24);
    }

    #[test]
    fn tile_store_invalidates_on_key_change() {
        let key = |f: &dyn Fn(&mut HeaderKey)| {
            let mut k = HeaderKey {
                cw: 10f64.to_bits(),
                scale: 125,
                mode: ColorMode::new(Scheme::ClustalX),
                theme: Theme::LIGHT,
                inks: [1, 2, 3, 4],
                generation: 1,
                reference: None,
                bands: vec![(0, 0, 17)],
            };
            f(&mut k);
            k
        };
        let mut store: TileStore<HeaderKey, i64, u32> = TileStore::default();
        assert!(store.begin(key(&|_| {})).is_empty());
        store.insert(0, 100);
        store.insert(1, 101);
        // same key: tiles stay
        assert!(store.begin(key(&|_| {})).is_empty());
        assert_eq!(store.get(&1), Some(&101));
        assert_eq!(store.missing(&[0, 1, 2]), [2]);
        let changes: [&dyn Fn(&mut HeaderKey); 7] = [
            &|k| k.mode.scheme = Scheme::Differences,
            &|k| k.mode.by_conservation = true,
            &|k| k.theme = Theme::DARK,
            &|k| k.generation = 2,
            &|k| k.cw = 11f64.to_bits(),
            &|k| k.reference = Some(3),
            &|k| k.bands.push((5, 17, 14)),
        ];
        for f in changes {
            store.clear();
            store.begin(key(&|_| {}));
            store.insert(0, 7);
            let dropped = store.begin(key(f));
            assert_eq!(dropped, [7], "a changed key frees the old tiles");
            assert!(store.tiles.is_empty());
        }
    }

    #[test]
    fn eviction_keeps_recent_tiles() {
        let mut store: TileStore<u8, i64, i64> = TileStore::default();
        store.begin(0);
        for i in 0..4 {
            store.insert(i, i);
        }
        store.begin(0);
        store.get(&3);
        store.get(&1);
        let mut gone = store.evict(2);
        gone.sort();
        assert_eq!(gone, [0, 2]);
        assert_eq!(tiles_x(0.0, 256.0), 0..1);
        assert_eq!(tiles_x(100.0, 300.0), 0..2);
    }
}
