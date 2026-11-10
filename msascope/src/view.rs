//! Viewport geometry, hit-testing and tile rasterisation. All sizes here are device pixels.

use crate::Alignment;
use crate::alphabet::{GAP, GAP_COLOR};
use crate::pyramid::{Pyramid, missing_code};
use crate::stats::ColumnStats;

pub const TILE: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ColorMode {
    /// Every residue in its own colour.
    Residues,
    /// Residues that match the column consensus are faded; differences stand out.
    Differences,
}

/// Zoom step to pixels per cell: each step is a factor of √2.
pub fn zoom_px(z: i32) -> f64 {
    2f64.powf(z as f64 / 2.0)
}

/// Largest zoom step at which `cells` fit in `px` pixels, clamped to `[min, max]`.
pub fn fit_zoom(cells: usize, px: f64, min: i32, max: i32) -> i32 {
    let mut z = max;
    while z > min && cells as f64 * zoom_px(z) > px {
        z -= 1;
    }
    z
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

pub struct Scene<'a> {
    pub aln: &'a Alignment,
    pub stats: &'a ColumnStats,
    pub pyramid: &'a Pyramid,
    pub mode: ColorMode,
}

fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
        ((x + (y - x) * t).round() as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

const DIFF_HIGHLIGHT: u32 = 0x202020;

impl Scene<'_> {
    /// Colour of one cell (0xRRGGBB).
    pub fn cell_color(&self, r: usize, c: usize) -> u32 {
        let code = self.aln.matrix.get(r, c);
        let color = self.aln.alphabet().color(code);
        match self.mode {
            ColorMode::Residues => color,
            ColorMode::Differences => {
                if code == GAP {
                    GAP_COLOR
                } else if code == self.stats.consensus[c] || code == missing_code(self.aln.kind()) {
                    mix(color, 0xFFFFFF, 0.8)
                } else {
                    color
                }
            }
        }
    }

    fn block_color(&self, maj: u8, diff: u8) -> u32 {
        let color = self.aln.alphabet().color(maj);
        match self.mode {
            ColorMode::Residues => color,
            ColorMode::Differences => {
                let base = if maj == GAP { GAP_COLOR } else { mix(color, 0xFFFFFF, 0.8) };
                // sqrt so that a few differing cells in a large block are still visible.
                let t = if diff == 0 { 0.0 } else { (diff as f32 / 255.0).sqrt().clamp(0.25, 1.0) };
                mix(base, DIFF_HIGHLIGHT, t)
            }
        }
    }

    /// Rasterise tile (tx, ty) at `cw` × `ch` pixels per cell into `out` (BGRA, TILE × TILE).
    /// Pixels outside the alignment are left transparent.
    pub fn render_tile(&self, cw: f64, ch: f64, tx: i64, ty: i64, out: &mut [u8]) {
        let (rows, cols) = (self.aln.rows(), self.aln.cols());
        out.fill(0);
        let level = self.pyramid.level_for((1.0 / cw).min(1.0 / ch));
        // Sample at pixel centres.
        let col_of = |i: usize| ((tx * TILE as i64 + i as i64) as f64 + 0.5) / cw;
        let row_of = |j: usize| ((ty * TILE as i64 + j as i64) as f64 + 0.5) / ch;
        let px_cols: Vec<Option<usize>> =
            (0..TILE).map(|i| Some(col_of(i)).filter(|&c| c >= 0.0 && (c as usize) < cols).map(|c| c as usize)).collect();
        for j in 0..TILE {
            let r = row_of(j);
            if r < 0.0 || r as usize >= rows {
                continue;
            }
            let r = r as usize;
            let line = &mut out[j * TILE * 4..(j + 1) * TILE * 4];
            let mut last: Option<(usize, u32)> = None;
            for (i, c) in px_cols.iter().enumerate() {
                let Some(c) = *c else { continue };
                let rgb = match level {
                    Some(l) => {
                        let (br, bc) = (r >> l.shift, c >> l.shift);
                        match last {
                            Some((k, rgb)) if k == bc => rgb,
                            _ => {
                                let (m, d) = l.at(br, bc);
                                let rgb = self.block_color(m, d);
                                last = Some((bc, rgb));
                                rgb
                            }
                        }
                    }
                    None => match last {
                        Some((k, rgb)) if k == c => rgb,
                        _ => {
                            let rgb = self.cell_color(r, c);
                            last = Some((c, rgb));
                            rgb
                        }
                    },
                };
                line[i * 4..i * 4 + 4].copy_from_slice(&[rgb as u8, (rgb >> 8) as u8, (rgb >> 16) as u8, 255]);
            }
        }
    }
}

/// Per-pixel summary of the column tracks for a strip of `npx` pixels starting at column `col0`:
/// (consensus code, mean identity) of the columns under each pixel, sampling at most 16 columns.
pub fn column_track(stats: &ColumnStats, col0: f64, cw: f64, npx: usize) -> Vec<Option<(u8, f32)>> {
    let cols = stats.consensus.len();
    (0..npx)
        .map(|i| {
            let a = col0 + i as f64 / cw;
            let b = col0 + (i + 1) as f64 / cw;
            if a < 0.0 || a as usize >= cols {
                return None;
            }
            let (a, b) = (a as usize, (b.ceil() as usize).clamp(a as usize + 1, cols));
            let step = (b - a).div_ceil(16).max(1);
            let (mut sum, mut n) = (0.0f32, 0);
            for c in (a..b).step_by(step) {
                sum += stats.identity[c];
                n += 1;
            }
            Some((stats.consensus[a], sum / n as f32))
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
        assert_eq!(fit_zoom(1000, 1000.0, -20, 8), 0);
        assert_eq!(fit_zoom(4000, 1000.0, -20, 8), -4);
        assert_eq!(tick_step(10.0, 60.0), 10);
        assert_eq!(tick_step(0.01, 60.0), 10000);
    }

    #[test]
    fn tile_pixels() {
        let rows: Vec<String> = (0..40).map(|r| if r == 3 { "T".repeat(64) } else { "A".repeat(64) }).collect();
        let text: Vec<u8> = rows.iter().enumerate().flat_map(|(i, s)| format!(">s{i}\n{s}\n").into_bytes()).collect();
        let aln = parse_bytes(&text).unwrap();
        let stats = ColumnStats::compute(&aln);
        let pyramid = Pyramid::build(&aln, &stats.consensus);
        let mut scene = Scene { aln: &aln, stats: &stats, pyramid: &pyramid, mode: ColorMode::Residues };
        let mut out = vec![0u8; TILE * TILE * 4];
        let px = |out: &[u8], i: usize, j: usize| {
            let p = &out[(j * TILE + i) * 4..][..4];
            (p[2] as u32) << 16 | (p[1] as u32) << 8 | p[0] as u32
        };
        // 2 px per cell: row 3 is pixel rows 6-7.
        scene.render_tile(2.0, 2.0, 0, 0, &mut out);
        let (a, t) = (aln.alphabet().color(1), aln.alphabet().color(4));
        assert_eq!(px(&out, 0, 0), a);
        assert_eq!(px(&out, 5, 7), t);
        assert_eq!(out[200 * TILE * 4 + 3], 0); // below the last row: transparent
        // Differences mode: matching cells faded, the T row in full colour.
        scene.mode = ColorMode::Differences;
        scene.render_tile(2.0, 2.0, 0, 0, &mut out);
        assert_ne!(px(&out, 0, 0), a);
        assert_eq!(px(&out, 5, 7), t);
        // Zoomed out 1/8: pyramid level 0, block (0, x) holds row 3, so it is highlighted.
        scene.render_tile(0.125, 0.125, 0, 0, &mut out);
        assert_ne!(px(&out, 0, 0), px(&out, 0, 1));
    }

    #[test]
    fn tracks_and_search() {
        let aln = parse_bytes(b">alpha\nAAC\n>Beta\nAGC\n>gamma\nATC\n").unwrap();
        let s = ColumnStats::compute(&aln);
        let t = column_track(&s, 0.0, 1.0, 4);
        assert_eq!(t[0], Some((1, 1.0)));
        assert_eq!(t[3], None);
        let t = column_track(&s, 0.0, 0.5, 1); // two columns per pixel
        assert!((t[0].unwrap().1 - (1.0 + 1.0 / 3.0) / 2.0).abs() < 1e-6);
        assert_eq!(search_names(&aln.names, "ET"), [1]);
        assert_eq!(search_names(&aln.names, "a"), [0, 1, 2]);
        assert!(search_names(&aln.names, "").is_empty());
    }
}
