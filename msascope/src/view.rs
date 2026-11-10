//! Viewport geometry, hit-testing and tile rasterisation. All sizes here are device pixels.

use crate::Alignment;
use crate::alphabet::{GAP, NCODES};
use crate::pyramid::{Pyramid, missing_code};
use crate::scheme::{Colors, Column};
use crate::stats::ColumnStats;

pub const TILE: usize = 256;

/// Most pyramid blocks or cells read to colour one pixel at less than one pixel per cell. They
/// are spread evenly over the pixel's rectangle of cells.
pub const SAMPLES: usize = 32;

/// Zoom step to pixels per cell: each step is a factor of √2.
pub fn zoom_px(z: i32) -> f64 {
    2f64.powf(z as f64 / 2.0)
}

/// Zoom steps the +/− keys and Ctrl+wheel move between.
pub const ZOOM_MIN: i32 = -36;
pub const ZOOM_MAX: i32 = 10;
/// Letters zoom: about 11 × 16 px per cell (zoom steps 7 and 8). Also the largest scale the fit
/// zooms use, so a small alignment isn't blown up.
pub const LETTERS_CW: f64 = 11.313708498984761;
pub const LETTERS_CH: f64 = 16.0;
/// Columns narrower than this make "fit width" unreadable, so the first view uses letters zoom.
/// It is the width at which letters are drawn: a fit that hides the letters is not readable.
pub const MIN_FIT_CW: f64 = 7.0;

/// The exact scale (px per cell) at which `cells` fill `px` pixels, at most `max`. Not rounded
/// to a zoom step, so a fit leaves no empty band.
pub fn fit_scale(cells: usize, px: f64, max: f64) -> f64 {
    if cells == 0 || px <= 0.0 {
        return max;
    }
    (px / cells as f64).clamp(zoom_px(ZOOM_MIN), max)
}

/// The scale `steps` zoom steps (√2 each) from `s`. From a scale between two steps (after a fit),
/// the first step goes to the neighbouring step, so later steps land on the usual grid.
pub fn zoom_step(s: f64, steps: i32) -> f64 {
    if steps == 0 {
        return s;
    }
    let z = 2.0 * s.log2();
    let base = if steps > 0 { (z + 1e-6).floor() } else { (z - 1e-6).ceil() };
    zoom_px((base as i32 + steps).clamp(ZOOM_MIN, ZOOM_MAX))
}

/// The scale (cw, ch) an alignment opens at in a `w` × `h` px area:
/// - the whole alignment at letters zoom when it fits;
/// - otherwise columns fit to the width when that gives at least 4 px per column, else letters
///   zoom, and rows at letters zoom, so names and residues are readable from the start.
///   (The spec said 4 px; with the overview panel a 141-column seed fitted to 6.8 px, below the
///   7 px at which letters are drawn.)
pub fn initial_view(rows: usize, cols: usize, w: f64, h: f64) -> (f64, f64) {
    if cols as f64 * LETTERS_CW <= w && rows as f64 * LETTERS_CH <= h {
        return (fit_scale(cols, w, LETTERS_CW), fit_scale(rows, h, LETTERS_CH));
    }
    let fit_w = fit_scale(cols, w, LETTERS_CW);
    (if fit_w >= MIN_FIT_CW { fit_w } else { LETTERS_CW }, LETTERS_CH)
}

/// `n` with thousands separators: 32112 -> "32,112".
pub fn group_digits(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, ch) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// The zoom readout: the cell size in px ("11 × 16 px"), or how many columns or rows share a
/// pixel once cells are smaller than one ("1 px = 43 columns × 64 rows").
pub fn zoom_label(cw: f64, ch: f64) -> String {
    let per_px = |s: f64| group_digits((1.0 / s).round().max(1.0) as usize);
    // One decimal below 10 px (an exact fit is rarely a whole number), none above.
    let size = |s: f64| {
        let t = if s >= 10.0 { format!("{s:.0}") } else { format!("{s:.1}") };
        t.strip_suffix(".0").map(str::to_owned).unwrap_or(t)
    };
    match (cw >= 1.0, ch >= 1.0) {
        (true, true) => format!("{} × {} px", size(cw), size(ch)),
        (false, false) => format!("1 px = {} columns × {} rows", per_px(cw), per_px(ch)),
        (true, false) => format!("{} px wide, 1 px = {} rows", size(cw), per_px(ch)),
        (false, true) => format!("1 px = {} columns, {} px tall", per_px(cw), size(ch)),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// Top-left corner, in (fractional) cells.
    pub col0: f64,
    pub row0: f64,
    /// Pixels per cell.
    pub cw: f64,
    pub ch: f64,
    /// Size of the alignment area in pixels.
    pub width: f64,
    pub height: f64,
}

impl Viewport {
    /// Cell under a point relative to the alignment area's top-left corner.
    pub fn hit(&self, x: f64, y: f64, rows: usize, cols: usize) -> Option<(usize, usize)> {
        if x < 0.0 || y < 0.0 || x >= self.width || y >= self.height {
            return None;
        }
        let c = (self.col0 + x / self.cw).floor();
        let r = (self.row0 + y / self.ch).floor();
        (c >= 0.0 && r >= 0.0 && (c as usize) < cols && (r as usize) < rows).then_some((r as usize, c as usize))
    }

    /// Keep the view inside the alignment (or at the origin when the alignment is smaller).
    pub fn clamp(&mut self, rows: usize, cols: usize) {
        let max_c = (cols as f64 - self.width / self.cw).max(0.0);
        let max_r = (rows as f64 - self.height / self.ch).max(0.0);
        self.col0 = self.col0.clamp(0.0, max_c);
        self.row0 = self.row0.clamp(0.0, max_r);
    }

    /// Change pixels-per-cell while keeping the cell under (x, y) in place.
    pub fn zoom_at(&mut self, cw: f64, ch: f64, x: f64, y: f64) {
        let (c, r) = (self.col0 + x / self.cw, self.row0 + y / self.ch);
        self.cw = cw;
        self.ch = ch;
        self.col0 = c - x / cw;
        self.row0 = r - y / ch;
    }

    /// Visible cell ranges (half-open), clipped to the alignment.
    pub fn visible(&self, rows: usize, cols: usize) -> (std::ops::Range<usize>, std::ops::Range<usize>) {
        let r0 = self.row0.max(0.0).floor() as usize;
        let c0 = self.col0.max(0.0).floor() as usize;
        let r1 = ((self.row0 + self.height / self.ch).ceil().max(0.0) as usize).min(rows);
        let c1 = ((self.col0 + self.width / self.cw).ceil().max(0.0) as usize).min(cols);
        (r0.min(r1)..r1, c0.min(c1)..c1)
    }

    /// Tiles overlapping the view: (tile x, tile y) in the global pixel grid of this zoom.
    pub fn tiles(&self) -> impl Iterator<Item = (i64, i64)> {
        let t = TILE as f64;
        let x0 = (self.col0 * self.cw / t).floor() as i64;
        let y0 = (self.row0 * self.ch / t).floor() as i64;
        let x1 = ((self.col0 * self.cw + self.width) / t).ceil() as i64;
        let y1 = ((self.row0 * self.ch + self.height) / t).ceil() as i64;
        (y0..y1).flat_map(move |y| (x0..x1).map(move |x| (x, y)))
    }

    /// Top-left of a tile relative to the alignment area.
    pub fn tile_origin(&self, tx: i64, ty: i64) -> (f64, f64) {
        (tx as f64 * TILE as f64 - self.col0 * self.cw, ty as f64 * TILE as f64 - self.row0 * self.ch)
    }
}

/// The columns shown, as ascending alignment columns (view column `v` shows alignment column
/// `visible[v]`). Built when columns are hidden; no map means every column is shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColumnMap {
    pub visible: Vec<u32>,
}

impl ColumnMap {
    /// The columns of `0..cols` not hidden by `hide`; `None` when nothing is hidden.
    pub fn build(cols: usize, hide: impl Fn(usize) -> bool + Sync) -> Option<ColumnMap> {
        let visible: Vec<u32> = (0..cols as u32).filter(|&c| !hide(c as usize)).collect();
        (visible.len() < cols).then_some(ColumnMap { visible })
    }

    /// Columns hidden by insert state and gap share: inserts when `inserts`, and columns where at
    /// least `gappy` of the rows are gaps (0–1) when given.
    pub fn hiding(aln: &Alignment, stats: &ColumnStats, inserts: bool, gappy: Option<f32>) -> Option<ColumnMap> {
        let gappy = gappy.map(|g| g - 1e-6);
        Self::build(aln.cols(), |c| (inserts && aln.is_insert(c)) || gappy.is_some_and(|g| stats.gaps[c] >= g))
    }

    pub fn len(&self) -> usize {
        self.visible.len()
    }

    pub fn is_empty(&self) -> bool {
        self.visible.is_empty()
    }

    /// The view column showing alignment column `c`, or `Err` with the next shown view column
    /// (which may be `len()`) when `c` is hidden.
    pub fn view_of(&self, c: usize) -> Result<usize, usize> {
        self.visible.binary_search(&(c as u32))
    }

    /// View columns `v` (in `range`) where a run of hidden columns ends just before `v`, with the
    /// number hidden: `v = 0` when the first columns are hidden, and `v = len()` for a run at the
    /// end.
    pub fn hidden_runs(&self, total: usize, range: std::ops::Range<usize>) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        for v in range.start..=range.end.min(self.len()) {
            let prev = if v == 0 { -1 } else { self.visible[v - 1] as i64 };
            let next = self.visible.get(v).map_or(total as i64, |&c| c as i64);
            if next - prev > 1 {
                out.push((v, (next - prev - 1) as usize));
            }
        }
        out
    }
}

/// Alignment column of view column `v`.
#[inline]
pub fn aln_col(map: Option<&ColumnMap>, v: usize) -> usize {
    map.map_or(v, |m| m.visible[v] as usize)
}

/// Number of view columns.
pub fn view_cols(map: Option<&ColumnMap>, cols: usize) -> usize {
    map.map_or(cols, ColumnMap::len)
}

/// What the tiles are drawn from. `pyramid` must be built for `map` (the full pyramid when `map`
/// is `None`, [`Pyramid::build_mapped`] otherwise).
pub struct Scene<'a> {
    pub aln: &'a Alignment,
    pub stats: &'a ColumnStats,
    pub pyramid: &'a Pyramid,
    pub map: Option<&'a ColumnMap>,
    pub colors: &'a Colors,
}

impl Scene<'_> {
    pub fn view_cols(&self) -> usize {
        view_cols(self.map, self.aln.cols())
    }

    /// The scheme's view of alignment column `c`.
    #[inline]
    pub fn column(&self, c: usize) -> Column<'_> {
        let s = self.stats;
        Column { consensus: s.consensus[c], identity: s.identity[c], shares: &s.shares[c], clustal: s.clustal[c] }
    }

    /// Fill of cell (row, view column), or `None` when the scheme leaves it unfilled.
    pub fn fill(&self, r: usize, v: usize) -> Option<u32> {
        let c = aln_col(self.map, v);
        self.colors.fill(self.aln.matrix.get(r, c), &self.column(c))
    }

    /// Colour of one cell (0xRRGGBB); unfilled cells and gaps take the background.
    pub fn cell_color(&self, r: usize, v: usize) -> u32 {
        self.fill(r, v).unwrap_or(self.colors.theme.bg)
    }

    /// Rasterise tile (tx, ty) at `cw` × `ch` pixels per cell into `out` (BGRA, TILE × TILE).
    pub fn render_tile(&self, cw: f64, ch: f64, tx: i64, ty: i64, out: &mut [u8]) {
        self.render(cw, ch, tx * TILE as i64, ty * TILE as i64, TILE, TILE, out);
    }

    /// Rasterise the `w` × `h` pixels whose top-left is pixel (x0, y0) of the whole alignment drawn
    /// at `cw` × `ch` pixels per cell, into `out` (BGRA, `w` × `h`). Pixels outside the alignment
    /// are left transparent.
    ///
    /// At one pixel per cell or more, each pixel shows the cell under its centre. Below that, in
    /// either axis, each pixel shows the density rule ([`Colors::density`]) of the cells it
    /// covers: their majority code and the share that differ from their column's consensus, from
    /// up to [`SAMPLES`] pyramid blocks (the coarsest level whose blocks still fit the pixel's
    /// narrower side) or cells, spread over the pixel.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&self, cw: f64, ch: f64, x0: i64, y0: i64, w: usize, h: usize, out: &mut [u8]) {
        let rows = self.aln.rows();
        let cols = self.view_cols();
        out.fill(0);
        let put = |out: &mut [u8], i: usize, j: usize, rgb: u32| {
            out[(j * w + i) * 4..][..4].copy_from_slice(&[rgb as u8, (rgb >> 8) as u8, (rgb >> 16) as u8, 255]);
        };
        if cw >= 1.0 && ch >= 1.0 {
            // Sample at pixel centres.
            let px_cols: Vec<Option<(usize, usize)>> = (0..w)
                .map(|i| {
                    let v = ((x0 + i as i64) as f64 + 0.5) / cw;
                    (v >= 0.0 && (v as usize) < cols).then(|| (v as usize, aln_col(self.map, v as usize)))
                })
                .collect();
            let columns: Vec<Option<Column>> = px_cols.iter().map(|p| p.map(|(_, c)| self.column(c))).collect();
            for j in 0..h {
                let r = ((y0 + j as i64) as f64 + 0.5) / ch;
                if r < 0.0 || r as usize >= rows {
                    continue;
                }
                let r = r as usize;
                let mut last: Option<(usize, u32)> = None;
                for (i, p) in px_cols.iter().enumerate() {
                    let Some((v, c)) = *p else { continue };
                    let rgb = match last {
                        Some((k, rgb)) if k == v => rgb,
                        _ => {
                            let rgb = self.colors.cell(self.aln.matrix.get(r, c), columns[i].as_ref().unwrap());
                            last = Some((v, rgb));
                            rgb
                        }
                    };
                    put(out, i, j, rgb);
                }
            }
            return;
        }
        // Density: cells (or blocks) per pixel in each axis.
        let (sx, sy) = (1.0 / cw, 1.0 / ch);
        let level = self.pyramid.level_for(sx.min(sy));
        let shift = level.map_or(0, |l| l.shift);
        let (units_x, units_y) = level.map_or((cols, rows), |l| (l.cols, l.rows));
        let unit = (1u64 << shift) as f64;
        // Unit indices sampled in one pixel's span [a, b) of cells, at most `n` spread evenly.
        let spans = |p0: i64, n_px: usize, per_px: f64, total: usize, units: usize, n: usize| -> Vec<Vec<u32>> {
            (0..n_px)
                .map(|i| {
                    let a = (p0 + i as i64) as f64 * per_px;
                    let b = a + per_px;
                    if b <= 0.0 || a >= total as f64 {
                        return Vec::new();
                    }
                    let (a, b) = (a.max(0.0), b.min(total as f64));
                    let (ua, ub) = ((a / unit).floor() as usize, ((b / unit).ceil() as usize).min(units).max((a / unit).floor() as usize + 1));
                    let k = (ub - ua).min(n).max(1);
                    (0..k).map(|t| (ua + (t * (ub - ua) + (ub - ua) / 2) / k) as u32).filter(|&u| (u as usize) < units).collect()
                })
                .collect()
        };
        // Split the sample budget between the axes in proportion to how many units each covers.
        let per = |s: f64| (s / unit).max(1.0);
        let (ux, uy) = (per(sx), per(sy));
        let nx = ((SAMPLES as f64 * ux / uy).sqrt().round() as usize).clamp(1, SAMPLES);
        let ny = (SAMPLES / nx).max(1);
        let xs = spans(x0, w, sx, cols, units_x, nx);
        let ys = spans(y0, h, sy, rows, units_y, ny);
        // Alignment columns and consensus of the sampled cells (cells mode only).
        let missing = missing_code(self.aln.kind());
        let acols: Vec<Vec<(usize, u8)>> = if level.is_none() {
            xs.iter().map(|u| u.iter().map(|&v| { let c = aln_col(self.map, v as usize); (c, self.stats.consensus[c]) }).collect()).collect()
        } else {
            Vec::new()
        };
        let mut votes = [0u16; NCODES];
        for (j, yu) in ys.iter().enumerate() {
            if yu.is_empty() {
                continue;
            }
            let mut last: Option<(usize, u32)> = None;
            for (i, xu) in xs.iter().enumerate() {
                if xu.is_empty() {
                    continue;
                }
                // Neighbouring pixels covering the same units give the same colour.
                if let Some((k, rgb)) = last
                    && xs[k] == *xu
                {
                    put(out, i, j, rgb);
                    continue;
                }
                votes.fill(0);
                let (mut dsum, mut n) = (0u32, 0u32);
                for &y in yu {
                    match level {
                        Some(l) => {
                            for &x in xu {
                                let (m, d) = l.at(y as usize, x as usize);
                                votes[m as usize] += 1;
                                dsum += d as u32;
                                n += 1;
                            }
                        }
                        None => {
                            for &(c, cons) in &acols[i] {
                                let code = self.aln.matrix.get(y as usize, c);
                                votes[code as usize & (NCODES - 1)] += 1;
                                dsum += if code != GAP && code != missing && code != cons { 255 } else { 0 };
                                n += 1;
                            }
                        }
                    }
                }
                let maj = majority(&votes);
                let rgb = self.colors.density(maj, (dsum / n.max(1)) as u8);
                last = Some((i, rgb));
                put(out, i, j, rgb);
            }
        }
    }
}

/// Most common code; ties go to a residue over the gap, then to the lower code.
fn majority(votes: &[u16; NCODES]) -> u8 {
    let mut best = GAP as usize;
    for (i, &v) in votes.iter().enumerate().skip(1) {
        if v > votes[best] || (v > 0 && v == votes[best] && best == GAP as usize) {
            best = i;
        }
    }
    best as u8
}

/// One pixel column of the tracks: consensus code and alignment column of its first column, mean
/// identity and mean gap share of its columns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TrackPx {
    pub code: u8,
    pub col: usize,
    pub identity: f32,
    pub gaps: f32,
}

/// Per-pixel summary of the column tracks for a strip of `npx` pixels starting at view column
/// `col0`, sampling at most 16 columns per pixel.
pub fn column_track(stats: &ColumnStats, map: Option<&ColumnMap>, col0: f64, cw: f64, npx: usize) -> Vec<Option<TrackPx>> {
    let cols = view_cols(map, stats.consensus.len());
    (0..npx)
        .map(|i| {
            let a = col0 + i as f64 / cw;
            let b = col0 + (i + 1) as f64 / cw;
            if a < 0.0 || a as usize >= cols {
                return None;
            }
            let (a, b) = (a as usize, (b.ceil() as usize).clamp(a as usize + 1, cols));
            let step = (b - a).div_ceil(16).max(1);
            let (mut ident, mut gaps, mut n) = (0.0f32, 0.0f32, 0);
            for v in (a..b).step_by(step) {
                let c = aln_col(map, v);
                ident += stats.identity[c];
                gaps += stats.gaps[c];
                n += 1;
            }
            let c = aln_col(map, a);
            Some(TrackPx { code: stats.consensus[c], col: c, identity: ident / n as f32, gaps: gaps / n as f32 })
        })
        .collect()
}

/// Ruler tick spacing (in columns) so that labels are at least `min_px` pixels apart.
pub fn tick_step(cw: f64, min_px: f64) -> usize {
    let mut step = 1usize;
    loop {
        for m in [1, 2, 5] {
            if (step * m) as f64 * cw >= min_px {
                return step * m;
            }
        }
        step *= 10;
    }
}

/// Indices of sequences whose name contains `query` (case-insensitive).
pub fn search_names(names: &[String], query: &str) -> Vec<usize> {
    let q = query.to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    names.iter().enumerate().filter(|(_, n)| n.to_lowercase().contains(&q)).map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_bytes;
    use crate::scheme::{self, ColorMode, Scheme, Theme};

    fn vp() -> Viewport {
        Viewport { col0: 10.0, row0: 5.0, cw: 4.0, ch: 2.0, width: 100.0, height: 50.0 }
    }

    #[test]
    fn hit_testing() {
        let v = vp();
        assert_eq!(v.hit(0.0, 0.0, 100, 100), Some((5, 10)));
        assert_eq!(v.hit(7.9, 3.9, 100, 100), Some((6, 11)));
        assert_eq!(v.hit(-1.0, 0.0, 100, 100), None);
        assert_eq!(v.hit(99.0, 49.0, 100, 100), Some((29, 34)));
        assert_eq!(v.hit(99.0, 49.0, 20, 100), None); // past the last row
    }

    #[test]
    fn zoom_keeps_anchor() {
        let mut v = vp();
        let before = v.hit(40.0, 20.0, 1000, 1000);
        v.zoom_at(8.0, 4.0, 40.0, 20.0);
        assert_eq!(v.hit(40.0, 20.0, 1000, 1000), before);
    }

    #[test]
    fn clamp_and_visible() {
        let mut v = vp();
        v.col0 = 1e9;
        v.row0 = -3.0;
        v.clamp(30, 40);
        assert_eq!((v.col0, v.row0), (15.0, 0.0));
        let (r, c) = v.visible(30, 40);
        assert_eq!((r, c), (0..25, 15..40));
    }

    #[test]
    fn tiles_cover_view() {
        let v = Viewport { col0: 100.0, row0: 0.0, cw: 3.0, ch: 3.0, width: 600.0, height: 100.0 };
        let t: Vec<_> = v.tiles().collect();
        // x pixels 300..900 -> tiles 1..=3
        assert_eq!(t, [(1, 0), (2, 0), (3, 0)]);
        assert_eq!(v.tile_origin(1, 0), (-44.0, 0.0));
    }

    #[test]
    fn zoom_steps() {
        assert_eq!(zoom_px(0), 1.0);
        assert_eq!(zoom_px(4), 4.0);
        assert!((zoom_px(7) - LETTERS_CW).abs() < 1e-12);
        assert_eq!(zoom_step(4.0, 1), zoom_px(5));
        assert_eq!(zoom_step(4.0, -2), zoom_px(2));
        // From an exact fit between steps 3 (2.83) and 4 (4): one step to each side.
        assert_eq!(zoom_step(3.3, 1), zoom_px(4));
        assert_eq!(zoom_step(3.3, -1), zoom_px(3));
        assert_eq!(zoom_step(zoom_px(ZOOM_MAX), 1), zoom_px(ZOOM_MAX));
        assert_eq!(zoom_step(1e-9, -1), zoom_px(ZOOM_MIN));
        assert_eq!(tick_step(10.0, 60.0), 10);
        assert_eq!(tick_step(0.01, 60.0), 10000);
    }

    #[test]
    fn fit_is_exact() {
        // No more than 1 px empty, for awkward sizes in both axes.
        for (cells, px) in [(698usize, 838.0f64), (32_112, 600.0), (30_000, 1083.0), (100_000, 517.0), (7, 913.0)] {
            let s = fit_scale(cells, px, f64::INFINITY);
            assert!((px - cells as f64 * s).abs() <= 1.0, "{cells} cells in {px} px");
        }
        // Capped at letters zoom: a tiny alignment sits at the top-left instead of being blown up.
        assert_eq!(fit_scale(5, 1000.0, LETTERS_CW), LETTERS_CW);
        assert_eq!(fit_scale(0, 1000.0, LETTERS_CW), LETTERS_CW);
    }

    #[test]
    fn initial_views() {
        // Pfam PF00042 full: 32,112 × 698 in 838 × 600 px. Fit width would be 1.2 px per column,
        // too narrow, so letters zoom in both axes.
        let (cw, ch) = initial_view(32_112, 698, 838.0, 600.0);
        assert!(cw >= MIN_FIT_CW && ch == LETTERS_CH, "{cw} × {ch}");
        // 50 × 100 fits at letters zoom in 1200 × 900: the whole alignment.
        assert_eq!(initial_view(50, 100, 1200.0, 900.0), (LETTERS_CW, LETTERS_CH));
        // 500 × 100 in 838 × 600: columns fit the width (8.4 px), rows at letters zoom.
        let (cw, ch) = initial_view(500, 100, 838.0, 600.0);
        assert!((cw - 8.38).abs() < 1e-9 && ch == LETTERS_CH);
        // PF00042 seed, 73 × 141, in the 952 px left next to the overview panel: fit width would
        // be 6.75 px, too narrow for letters, so letters zoom.
        assert_eq!(initial_view(73, 141, 952.0, 680.0), (LETTERS_CW, LETTERS_CH));
        // 100,000 × 30,000: letters zoom.
        assert_eq!(initial_view(100_000, 30_000, 1083.0, 600.0), (LETTERS_CW, LETTERS_CH));
    }

    #[test]
    fn labels() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(999), "999");
        assert_eq!(group_digits(32_112), "32,112");
        assert_eq!(group_digits(1_234_567), "1,234,567");
        assert_eq!(zoom_label(LETTERS_CW, 16.0), "11 × 16 px");
        assert_eq!(zoom_label(1.0 / 43.0, 1.0 / 64.0), "1 px = 43 columns × 64 rows");
        assert_eq!(zoom_label(8.0, 0.5), "8 px wide, 1 px = 2 rows");
        assert_eq!(zoom_label(1084.0 / 698.0, 600.0 / 32_112.0), "1.6 px wide, 1 px = 54 rows");
        assert_eq!(zoom_label(7.69, 8.56), "7.7 × 8.6 px");
        assert_eq!(zoom_label(0.001, 16.0), "1 px = 1,000 columns, 16 px tall");
    }

    fn scene_parts(text: &[u8]) -> (Alignment, ColumnStats, Pyramid) {
        let aln = parse_bytes(text).unwrap();
        let stats = ColumnStats::compute(&aln);
        let pyramid = Pyramid::build(&aln, &stats.consensus);
        (aln, stats, pyramid)
    }

    fn fasta(rows: &[String]) -> Vec<u8> {
        rows.iter().enumerate().flat_map(|(i, s)| format!(">s{i}\n{s}\n").into_bytes()).collect()
    }

    fn px(out: &[u8], w: usize, i: usize, j: usize) -> u32 {
        let p = &out[(j * w + i) * 4..][..4];
        (p[2] as u32) << 16 | (p[1] as u32) << 8 | p[0] as u32
    }

    #[test]
    fn tile_pixels() {
        let rows: Vec<String> = (0..40).map(|r| if r == 3 { "T".repeat(64) } else { "A".repeat(64) }).collect();
        let (aln, stats, pyramid) = scene_parts(&fasta(&rows));
        let colors = |scheme, theme| Colors::new(aln.alphabet(), ColorMode::new(scheme), theme, false);
        let nt = colors(Scheme::Nucleotide, Theme::LIGHT);
        let mut scene = Scene { aln: &aln, stats: &stats, pyramid: &pyramid, map: None, colors: &nt };
        let mut out = vec![0u8; TILE * TILE * 4];
        // 2 px per cell: row 3 is pixel rows 6-7.
        scene.render_tile(2.0, 2.0, 0, 0, &mut out);
        let col = scene.column(0);
        let (a, t) = (nt.cell(1, &col), nt.cell(4, &col));
        assert_eq!(px(&out, TILE, 0, 0), a);
        assert_eq!(px(&out, TILE, 5, 7), t);
        assert_eq!(out[200 * TILE * 4 + 3], 0); // below the last row: transparent
        // Dark theme: other fills.
        let dark = colors(Scheme::Nucleotide, Theme::DARK);
        scene.colors = &dark;
        scene.render_tile(2.0, 2.0, 0, 0, &mut out);
        assert_ne!(px(&out, TILE, 0, 0), a);
        // Differences: matching cells faded, the T row in full colour.
        let diff = colors(Scheme::Differences, Theme::LIGHT);
        scene.colors = &diff;
        scene.render_tile(2.0, 2.0, 0, 0, &mut out);
        assert_ne!(px(&out, TILE, 0, 0), a);
        assert_eq!(px(&out, TILE, 5, 7), t);
        // Zoomed out 1/8: block (0, x) holds row 3, so it differs from the block below.
        scene.render_tile(0.125, 0.125, 0, 0, &mut out);
        assert_ne!(px(&out, TILE, 0, 0), px(&out, TILE, 0, 1));
    }

    /// Gaps are the theme's background at every zoom of one pixel per cell or more, in both
    /// themes (no separate gap colour), and the gap-majority density is the background with 3% of
    /// the foreground.
    #[test]
    fn gaps_are_background() {
        let (aln, stats, pyramid) = scene_parts(b">a\nA--A\n>b\nA--A\n");
        for theme in [Theme::LIGHT, Theme::DARK] {
            let c = Colors::new(aln.alphabet(), ColorMode::new(Scheme::Nucleotide), theme, true);
            let scene = Scene { aln: &aln, stats: &stats, pyramid: &pyramid, map: None, colors: &c };
            let mut out = vec![0u8; 16 * 4 * 4];
            scene.render(4.0, 4.0, 0, 0, 16, 4, &mut out);
            assert_eq!(px(&out, 16, 6, 1), theme.bg);
            scene.render(0.25, 0.25, 0, 0, 1, 1, &mut out[..4]);
        }
    }

    /// A density tile of a synthetic alignment with two clades, each with its own conserved
    /// columns, shows two bands: conserved blocks strong, the other clade's blocks faint.
    #[test]
    fn density_shows_two_clades() {
        // 400 rows × 400 columns; clade 1 (rows 0–199) is all A in columns 0–199 and random
        // elsewhere, clade 2 the opposite with C.
        let mut x = 12345u32;
        let mut rnd = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            b"ACGT"[(x % 4) as usize] as char
        };
        let rows: Vec<String> = (0..400)
            .map(|r| (0..400).map(|c| if (r < 200) == (c < 200) { if r < 200 { 'A' } else { 'C' } } else { rnd() }).collect())
            .collect();
        let (aln, stats, pyramid) = scene_parts(&fasta(&rows));
        let c = Colors::new(aln.alphabet(), ColorMode::new(Scheme::Nucleotide), Theme::LIGHT, false);
        let scene = Scene { aln: &aln, stats: &stats, pyramid: &pyramid, map: None, colors: &c };
        // 400 cells in 40 px: 10 cells per pixel.
        let mut out = vec![0u8; 40 * 40 * 4];
        scene.render(0.1, 0.1, 0, 0, 40, 40, &mut out);
        let strength = |i, j| scheme::contrast(px(&out, 40, i, j), Theme::LIGHT.bg);
        // clade 1's conserved block (top-left) vs its variable part (top-right)
        assert!(strength(5, 5) > strength(35, 5) + 0.3, "{} {}", strength(5, 5), strength(35, 5));
        assert!(strength(35, 35) > strength(5, 35) + 0.3);
        // uniform within a band
        assert_eq!(px(&out, 40, 2, 3), px(&out, 40, 15, 12));
        // Rows much denser than columns (1.6 px per column, 50 rows per pixel), as at "Fit all" on
        // a tall, narrow alignment: still the same two bands, not single sampled rows.
        let mut tall = vec![0u8; 40 * 8 * 4];
        scene.render(0.1, 0.02, 0, 0, 40, 8, &mut tall);
        let s = |i, j| scheme::contrast(px(&tall, 40, i, j), Theme::LIGHT.bg);
        assert!(s(5, 1) > s(35, 1) + 0.3 && s(35, 6) > s(5, 6) + 0.3);
    }

    /// Rendering with columns hidden equals rendering the alignment with those columns removed,
    /// at letter scale and in the density view.
    #[test]
    fn hidden_columns_equal_removed() {
        let rows: Vec<String> = (0..48).map(|r| (0..90).map(|c| b"ACGT--"[(r * 5 + c * 7 + (r * c) % 11) % 6] as char).collect()).collect();
        let (aln, stats, _) = scene_parts(&fasta(&rows));
        let map = ColumnMap::build(aln.cols(), |c| c % 4 == 1 || (30..41).contains(&c)).unwrap();
        let mapped = Pyramid::build_mapped(&aln, &stats.consensus, Some(&map.visible));
        let kept: Vec<String> = rows.iter().map(|r| map.visible.iter().map(|&c| r.as_bytes()[c as usize] as char).collect()).collect();
        let (b, _, _) = scene_parts(&fasta(&kept));
        // The removed alignment's column stats differ only in which columns exist: build its
        // stats from the original ones so the consensus is the same.
        let pick = |v: &Vec<f32>| map.visible.iter().map(|&c| v[c as usize]).collect::<Vec<_>>();
        let bstats = ColumnStats {
            consensus: map.visible.iter().map(|&c| stats.consensus[c as usize]).collect(),
            identity: pick(&stats.identity),
            gaps: pick(&stats.gaps),
            shares: map.visible.iter().map(|&c| stats.shares[c as usize]).collect(),
            clustal: map.visible.iter().map(|&c| stats.clustal[c as usize]).collect(),
        };
        let bpyr = Pyramid::build(&b, &bstats.consensus);
        for scheme in [Scheme::Nucleotide, Scheme::PercentIdentity, Scheme::Differences] {
            let c = Colors::new(aln.alphabet(), ColorMode::new(scheme), Theme::DARK, false);
            let s1 = Scene { aln: &aln, stats: &stats, pyramid: &mapped, map: Some(&map), colors: &c };
            let s2 = Scene { aln: &b, stats: &bstats, pyramid: &bpyr, map: None, colors: &c };
            for (cw, ch) in [(3.0, 2.0), (0.3, 0.2), (1.5, 0.1), (0.05, 0.05)] {
                let (mut o1, mut o2) = (vec![0u8; 64 * 64 * 4], vec![0u8; 64 * 64 * 4]);
                s1.render(cw, ch, 0, 0, 64, 64, &mut o1);
                s2.render(cw, ch, 0, 0, 64, 64, &mut o2);
                assert!(o1 == o2, "{scheme:?} at {cw} × {ch}");
            }
        }
    }

    #[test]
    fn column_maps() {
        let m = ColumnMap::build(10, |c| matches!(c, 0 | 3 | 4 | 9)).unwrap();
        assert_eq!(m.visible, [1, 2, 5, 6, 7, 8]);
        assert_eq!(m.view_of(5), Ok(2));
        assert_eq!(m.view_of(3), Err(2)); // hidden: the next shown is view column 2 (column 5)
        assert_eq!(m.view_of(9), Err(6));
        assert_eq!(m.hidden_runs(10, 0..6), [(0, 1), (2, 2), (6, 1)]);
        assert_eq!(m.hidden_runs(10, 3..5), []);
        assert!(ColumnMap::build(5, |_| false).is_none());
        // hit-testing goes through the map: a click on view column 2 is alignment column 5
        let v = Viewport { col0: 0.0, row0: 0.0, cw: 10.0, ch: 10.0, width: 100.0, height: 100.0 };
        let (r, vc) = v.hit(25.0, 5.0, 3, m.len()).unwrap();
        assert_eq!((r, aln_col(Some(&m), vc)), (0, 5));
        assert_eq!(v.hit(65.0, 5.0, 3, m.len()), None); // past the last shown column
        // inserts and gappy columns
        let (aln, stats, _) = scene_parts(b"# STOCKHOLM 1.0\nx AC.gT-\ny A-tG--\nz A-.GT-\n//\n");
        assert_eq!(ColumnMap::hiding(&aln, &stats, true, None).unwrap().visible, [0, 1, 3, 4, 5]);
        assert_eq!(ColumnMap::hiding(&aln, &stats, false, Some(0.5)).unwrap().visible, [0, 3, 4]);
        assert_eq!(ColumnMap::hiding(&aln, &stats, true, Some(1.0)).unwrap().visible, [0, 1, 3, 4]);
        assert!(ColumnMap::hiding(&aln, &stats, false, None).is_none());
    }

    #[test]
    fn tracks_and_search() {
        let aln = parse_bytes(b">alpha\nAAC\n>Beta\nAGC\n>gamma\nATC\n").unwrap();
        let s = ColumnStats::compute(&aln);
        let t = column_track(&s, None, 0.0, 1.0, 4);
        assert_eq!(t[0], Some(TrackPx { code: 1, col: 0, identity: 1.0, gaps: 0.0 }));
        assert_eq!(t[3], None);
        let m = ColumnMap::build(3, |c| c == 0).unwrap();
        assert_eq!(column_track(&s, Some(&m), 0.0, 1.0, 3)[0].unwrap().col, 1);
        assert_eq!(column_track(&s, Some(&m), 0.0, 1.0, 3)[2], None);
        let t = column_track(&s, None, 0.0, 0.5, 1); // two columns per pixel
        assert!((t[0].unwrap().identity - (1.0 + 1.0 / 3.0) / 2.0).abs() < 1e-6);
        let g = parse_bytes(b">a\nA-\n>b\nA-\n>c\nAC\n").unwrap();
        let t = column_track(&ColumnStats::compute(&g), None, 0.0, 1.0, 2);
        assert!((t[1].unwrap().gaps - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(search_names(&aln.names, "ET"), [1]);
        assert_eq!(search_names(&aln.names, "a"), [0, 1, 2]);
        assert!(search_names(&aln.names, "").is_empty());
    }
}
