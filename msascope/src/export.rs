//! Figures of the alignment: SVG with the residue letters, names and ruler as real text (vector,
//! selectable), the shown `#=GC` / `#=GR` tracks and the pinned reference row under the ruler, and
//! the size rules for SVG and PNG. The GUI rasterises PNGs (cells with
//! [`crate::view::Scene::render`], track bands with [`crate::header::Strip::render`], text from
//! [`Figure::svg`] with `cells: false`).

use std::io::{self, Write};

use crate::Alignment;
use crate::alphabet::GAP;
use crate::annot::{Ss, TrackKind, pp_value, ss_class};
use crate::header::{AnnotColors, is_blank};
use crate::reference::Reference;
use crate::scheme::{Colors, Column, mix};
use crate::stats::ColumnStats;
use crate::view::{RowSrc, tick_step};

/// SVGs are refused above this many cells: one `<rect>` per run of a colour and a letter per
/// residue make larger files slow to open in browsers and editors.
pub const SVG_MAX_CELLS: usize = 200_000;
/// PNGs are refused above this width or height in pixels…
pub const PNG_MAX_SIDE: usize = 16_384;
/// …or above this many pixels (400 MB of RGBA while it is drawn).
pub const PNG_MAX_PIXELS: usize = 100_000_000;

/// Space above the cells for the ruler, and its font size, in px at scale 1.
pub const RULER_PX: f64 = 22.0;
const RULER_FONT: f64 = 10.5;
/// Letters are written when cells are at least this big (px), as in the window.
pub const LETTER_MIN: (f64, f64) = (7.0, 10.0);
/// Height of an annotation track and of the pinned reference row (px at scale 1), as in the
/// window.
pub const TRACK_PX: f64 = 14.0;
pub const PIN_PX: f64 = 16.0;
/// Strands end in an arrowhead this long (px) when longer than it, as in the window.
const ARROW_PX: f64 = 5.0;

/// A band between the ruler and the rows.
#[derive(Clone, Copy)]
pub enum TrackBand<'a> {
    /// An annotation line (`#=GC`, or a `#=GR` line of the reference), one byte per alignment
    /// column.
    Annot { data: &'a [u8], kind: TrackKind },
    /// The pinned reference row, one code per alignment column.
    Pinned { codes: &'a [u8] },
}

/// A track of a figure and its label in the name column.
pub struct FigTrack<'a> {
    pub band: TrackBand<'a>,
    pub label: String,
}

impl FigTrack<'_> {
    pub fn height(&self) -> f64 {
        match self.band {
            TrackBand::Annot { .. } => TRACK_PX,
            TrackBand::Pinned { .. } => PIN_PX,
        }
    }
}

/// One row of a figure: where its residues come from and its label.
pub struct FigRow<'a> {
    pub src: RowSrc<'a>,
    pub label: String,
}

/// What to draw: rows × alignment columns at `cw` × `ch` px per cell.
pub struct Figure<'a> {
    pub aln: &'a Alignment,
    pub stats: &'a ColumnStats,
    pub colors: &'a Colors,
    pub rows: Vec<FigRow<'a>>,
    /// Alignment columns, in order (hidden ones left out).
    pub cols: Vec<usize>,
    pub cw: f64,
    pub ch: f64,
    /// Width of the name column in px; 0 for none.
    pub names_w: f64,
    pub ruler: bool,
    /// Font families for names and ruler, and for letters (CSS lists).
    pub ui_font: String,
    pub mono_font: String,
    /// Text colours (0xRRGGBB): names, ruler numbers and letters on no fill.
    pub fg: u32,
    pub muted: u32,
    /// What Differences compares with, per alignment column (a reference row's codes); `None`
    /// for the consensus.
    pub base: Option<&'a [u8]>,
    /// Tracks under the ruler, top to bottom.
    pub tracks: Vec<FigTrack<'a>>,
    /// The ruler counts this row's residues instead of alignment columns.
    pub ruler_ref: Option<&'a Reference>,
}

/// The size an export would have: (width, height) in px at scale 1, with `tracks_h` px of
/// tracks under the ruler.
pub fn size(names_w: f64, ruler: bool, tracks_h: f64, rows: usize, cols: usize, cw: f64, ch: f64) -> (f64, f64) {
    (names_w + cols as f64 * cw, if ruler { RULER_PX } else { 0.0 } + tracks_h + rows as f64 * ch)
}

/// Whether an SVG of `rows` × `cols` cells may be written.
pub fn check_svg(rows: usize, cols: usize) -> Result<(), String> {
    let cells = rows.saturating_mul(cols);
    if rows == 0 || cols == 0 {
        return Err("Nothing to export: no rows or no shown columns".into());
    }
    if cells > SVG_MAX_CELLS {
        return Err(format!(
            "SVG is limited to {} cells (this is {}: {} rows × {} columns): export a PNG, or select less",
            crate::view::group_digits(SVG_MAX_CELLS),
            crate::view::group_digits(cells),
            crate::view::group_digits(rows),
            crate::view::group_digits(cols)
        ));
    }
    Ok(())
}

/// Whether a PNG of `w` × `h` px may be drawn.
pub fn check_png(w: usize, h: usize) -> Result<(), String> {
    if w == 0 || h == 0 {
        return Err("Nothing to export".into());
    }
    if w > PNG_MAX_SIDE || h > PNG_MAX_SIDE || w.saturating_mul(h) > PNG_MAX_PIXELS {
        return Err(format!(
            "PNG too large: {} × {} px (limit {} px a side, {} megapixels): lower the scale, zoom out or select less",
            crate::view::group_digits(w),
            crate::view::group_digits(h),
            crate::view::group_digits(PNG_MAX_SIDE),
            PNG_MAX_PIXELS / 1_000_000
        ));
    }
    Ok(())
}

fn hex(c: u32) -> String {
    format!("#{:06x}", c & 0xff_ffff)
}

/// `s` with `&`, `<`, `>` and `"` escaped for XML.
pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            c if (c as u32) < 0x20 && c != '\t' => {}
            c => o.push(c),
        }
    }
    o
}

/// Short decimal for coordinates.
fn n(x: f64) -> String {
    let s = format!("{:.2}", x);
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() || s == "-0" { "0".into() } else { s.into() }
}

impl Figure<'_> {
    fn column(&self, c: usize) -> Column<'_> {
        let s = self.stats;
        Column { consensus: self.base.map_or(s.consensus[c], |b| b[c]), identity: s.identity[c], shares: &s.shares[c], clustal: s.clustal[c] }
    }

    pub fn size(&self) -> (f64, f64) {
        size(self.names_w, self.ruler, self.tracks_h(), self.rows.len(), self.cols.len(), self.cw, self.ch)
    }

    pub fn tracks_h(&self) -> f64 {
        self.tracks.iter().map(FigTrack::height).sum()
    }

    /// Where the tracks start (below the ruler), px.
    pub fn tracks_top(&self) -> f64 {
        if self.ruler { RULER_PX } else { 0.0 }
    }

    /// Ruler labels: (figure column, number) for the columns whose number is a multiple of the
    /// step, and the first column (or the reference's first residue in the figure).
    fn ruler_labels(&self, step: usize) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        match self.ruler_ref {
            Some(r) => {
                let mut first = true;
                for (v, &c) in self.cols.iter().enumerate() {
                    let Some(p) = r.position(c) else { continue };
                    if p.is_multiple_of(step) || first {
                        out.push((v, p));
                    }
                    first = false;
                }
            }
            None => {
                for (v, &c) in self.cols.iter().enumerate() {
                    if (c + 1).is_multiple_of(step) || v == 0 {
                        out.push((v, c + 1));
                    }
                }
            }
        }
        out
    }

    /// The tracks: labels in the name column, the pinned row's mark and top line (in both
    /// layers: a PNG's rasterised bands don't have them), with `cells` the bands themselves, and
    /// the characters of text tracks and the pinned row's letters as text.
    fn svg_tracks(&self, cells: bool, out: &mut impl Write) -> io::Result<()> {
        if self.tracks.is_empty() {
            return Ok(());
        }
        let theme = &self.colors.theme;
        let ac = AnnotColors::new(theme);
        let x0 = self.names_w;
        let width = self.cols.len() as f64 * self.cw;
        let mut y = self.tracks_top();
        let tops: Vec<f64> = self
            .tracks
            .iter()
            .map(|t| {
                let top = y;
                y += t.height();
                top
            })
            .collect();
        writeln!(out, r#"<g id="tracks">"#)?;
        // Labels: track names right-aligned against the alignment; the pinned row's name on a
        // tinted strip with an accent mark and a "ref" tag.
        if self.names_w > 0.0 {
            writeln!(out, r#"<g id="track-labels" font-family="{}" font-size="{}">"#, escape(&self.ui_font), RULER_FONT)?;
            for (t, &top) in self.tracks.iter().zip(&tops) {
                let base = top + t.height() / 2.0 + RULER_FONT * 0.35;
                match t.band {
                    TrackBand::Annot { .. } => {
                        writeln!(out, r#"<text x="{}" y="{}" fill="{}" text-anchor="end">{}</text>"#, n(self.names_w - 8.0), n(base), hex(self.muted), escape(&t.label))?;
                    }
                    TrackBand::Pinned { .. } => {
                        writeln!(out, r#"<rect x="0" y="{}" width="{}" height="{}" fill="{}"/>"#, n(top), n(self.names_w), n(t.height()), hex(mix(theme.bg, theme.accent, 0.14)))?;
                        writeln!(out, r#"<rect x="0" y="{}" width="2" height="{}" fill="{}"/>"#, n(top), n(t.height()), hex(theme.accent))?;
                        writeln!(out, r#"<clipPath id="ref-clip"><rect x="0" y="{}" width="{}" height="{}"/></clipPath>"#, n(top), n((self.names_w - 34.0).max(0.0)), n(t.height()))?;
                        writeln!(out, r#"<text x="8" y="{}" fill="{}" clip-path="url(#ref-clip)">{}</text>"#, n(base), hex(self.fg), escape(&t.label))?;
                        writeln!(out, r#"<text x="{}" y="{}" fill="{}" text-anchor="end">ref</text>"#, n(self.names_w - 8.0), n(base), hex(theme.accent))?;
                    }
                }
            }
            writeln!(out, "</g>")?;
        }
        if cells {
            writeln!(out, r#"<g id="track-bands" shape-rendering="crispEdges">"#)?;
            writeln!(out, r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#, n(x0), n(self.tracks_top()), n(width), n(self.tracks_h()), hex(theme.bg))?;
            for (t, &top) in self.tracks.iter().zip(&tops) {
                self.svg_band(t, top, &ac, out)?;
            }
            writeln!(out, "</g>")?;
        }
        // The pinned row's accent line along its top.
        if let Some((_, &top)) = self.tracks.iter().zip(&tops).find(|(t, _)| matches!(t.band, TrackBand::Pinned { .. })) {
            writeln!(out, r#"<rect x="0" y="{}" width="{}" height="1" fill="{}"/>"#, n(top), n(x0 + width), hex(theme.accent))?;
        }
        // Characters: text tracks once columns are wide enough, the pinned row's residues at
        // letter size.
        if self.cw >= LETTER_MIN.0 {
            let small = (self.cw * 1.3).clamp(7.0, 10.5);
            let pin = (PIN_PX.min(self.cw * 1.4) * 0.75).clamp(7.0, 12.0);
            writeln!(out, r#"<g id="track-text" font-family="{}" text-anchor="middle">"#, escape(&self.mono_font))?;
            for (t, &top) in self.tracks.iter().zip(&tops) {
                let (mut on, mut off) = (String::new(), String::new());
                let fs = match t.band {
                    TrackBand::Annot { data, kind: TrackKind::Text } => {
                        for (v, &c) in self.cols.iter().enumerate() {
                            if !is_blank(data[c]) {
                                off += &format!(r#"<tspan x="{}">{}</tspan>"#, n(x0 + (v as f64 + 0.5) * self.cw), escape(&(data[c] as char).to_string()));
                            }
                        }
                        small
                    }
                    TrackBand::Annot { .. } => continue,
                    TrackBand::Pinned { codes } => {
                        for (v, &c) in self.cols.iter().enumerate() {
                            let code = codes[c];
                            if code == GAP {
                                continue;
                            }
                            let t = if self.colors.fill(code, &self.column(c)).is_some() { &mut on } else { &mut off };
                            *t += &format!(r#"<tspan x="{}">{}</tspan>"#, n(x0 + (v as f64 + 0.5) * self.cw), escape(&(self.aln.symbol(code) as char).to_string()));
                        }
                        pin
                    }
                };
                let y = top + t.height() / 2.0 + fs * 0.36;
                for (spans, color) in [(on, theme.ink()), (off, self.muted)] {
                    if !spans.is_empty() {
                        writeln!(out, r#"<text y="{}" font-size="{}" fill="{}">{spans}</text>"#, n(y), n(fs), hex(color))?;
                    }
                }
            }
            writeln!(out, "</g>")?;
        }
        writeln!(out, "</g>")
    }

    /// One track band's shapes, `top` px from the top: runs of equal value share a shape.
    fn svg_band(&self, t: &FigTrack, top: f64, ac: &AnnotColors, out: &mut impl Write) -> io::Result<()> {
        let (x0, cw, h) = (self.names_w, self.cw, t.height());
        let mid = top + h / 2.0;
        let rect = |out: &mut dyn Write, v: usize, e: usize, y0: f64, y1: f64, rgb: u32| {
            writeln!(out, r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#, n(x0 + v as f64 * cw), n(y0), n((e - v) as f64 * cw), n(y1 - y0), hex(rgb))
        };
        // Runs of equal keys over the figure's columns.
        let runs = |key: &dyn Fn(usize) -> u32| {
            let mut r = Vec::new();
            let mut v = 0;
            while v < self.cols.len() {
                let k = key(self.cols[v]);
                let mut e = v + 1;
                while e < self.cols.len() && key(self.cols[e]) == k {
                    e += 1;
                }
                r.push((v, e, k));
                v = e;
            }
            r
        };
        match t.band {
            TrackBand::Pinned { codes } => {
                for (v, e, rgb) in runs(&|c| self.colors.cell(codes[c], &self.column(c))) {
                    rect(out, v, e, top, top + h, rgb)?;
                }
            }
            TrackBand::Annot { data, kind: TrackKind::Text } => {
                for (v, e, k) in runs(&|c| !is_blank(data[c]) as u32) {
                    if k == 1 {
                        rect(out, v, e, top + 1.0, top + h - 1.0, ac.shade)?;
                    }
                }
            }
            TrackBand::Annot { data, kind: TrackKind::Probability } => {
                for (v, e, k) in runs(&|c| data[c] as u32) {
                    let Some(p) = pp_value(k as u8) else { continue };
                    let hq = (p as f64 * (h - 4.0)).round().max(1.0);
                    rect(out, v, e, top + h - 2.0 - hq, top + h - 2.0, if p >= 0.9 { ac.pp_hi } else { ac.pp_lo })?;
                }
            }
            TrackBand::Annot { data, kind: TrackKind::Structure } => {
                let class = |c: usize| ss_class(data[c]);
                let order = [Ss::None, Ss::Helix, Ss::Strand, Ss::PairOpen, Ss::PairClose, Ss::Coil];
                for (v, e, k) in runs(&|c| order.iter().position(|&o| o == class(c)).unwrap() as u32) {
                    match order[k as usize] {
                        Ss::Helix => rect(out, v, e, mid - 4.0, mid + 4.0, ac.helix)?,
                        Ss::Strand => {
                            let (xa, xe) = (x0 + v as f64 * cw, x0 + e as f64 * cw);
                            if xe - xa > ARROW_PX {
                                let half = h / 2.0 - 2.0;
                                let xh = xe - ARROW_PX;
                                writeln!(out, r#"<rect x="{}" y="{}" width="{}" height="6" fill="{}"/>"#, n(xa), n(mid - 3.0), n(xh - xa), hex(ac.strand))?;
                                writeln!(out, r#"<polygon points="{},{} {},{} {},{}" fill="{}"/>"#, n(xh), n(mid - half), n(xe), n(mid), n(xh), n(mid + half), hex(ac.strand))?;
                            } else {
                                rect(out, v, e, mid - 3.0, mid + 3.0, ac.strand)?;
                            }
                        }
                        Ss::PairOpen => rect(out, v, e, mid - 3.0, mid + 3.0, ac.pair)?,
                        Ss::PairClose => rect(out, v, e, mid - 3.0, mid + 3.0, ac.pair_close)?,
                        Ss::Coil => rect(out, v, e, mid - 0.5, mid + 0.5, ac.coil)?,
                        Ss::None => {}
                    }
                }
            }
        }
        Ok(())
    }

    /// Write the SVG. With `cells` off, only the text (names, ruler, letters) on a transparent
    /// background: the layer a PNG puts over its rasterised cells.
    pub fn svg(&self, cells: bool, out: &mut impl Write) -> io::Result<()> {
        let (w, h) = self.size();
        let ruler_top = self.tracks_top();
        // Where the rows start.
        let top = ruler_top + self.tracks_h();
        let x0 = self.names_w;
        let theme = &self.colors.theme;
        writeln!(out, r#"<?xml version="1.0" encoding="UTF-8"?>"#)?;
        writeln!(
            out,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}" viewBox="0 0 {} {}">"#,
            n(w),
            n(h),
            n(w),
            n(h)
        )?;
        writeln!(out, "<!-- msascope: {} rows × {} columns -->", self.rows.len(), self.cols.len())?;
        if cells {
            writeln!(out, r#"<rect width="100%" height="100%" fill="{}"/>"#, hex(theme.bg))?;
        }
        // Ruler: alignment column numbers on every multiple of the step, as in the window.
        if self.ruler && !self.cols.is_empty() {
            let step = tick_step(self.cw, 70.0);
            writeln!(out, r#"<g id="ruler" font-family="{}" font-size="{}" fill="{}" text-anchor="middle">"#, escape(&self.ui_font), RULER_FONT, hex(self.muted))?;
            let mut last = f64::NEG_INFINITY;
            for (v, num) in self.ruler_labels(step) {
                let x = x0 + (v as f64 + 0.5) * self.cw;
                let label = crate::view::group_digits(num);
                let half = label.len() as f64 * RULER_FONT * 0.32;
                if x - half < last + 4.0 {
                    continue;
                }
                last = x + half;
                writeln!(out, r#"<text x="{}" y="{}">{label}</text>"#, n(x), n(ruler_top - 8.0))?;
                if cells {
                    writeln!(out, r#"<rect x="{}" y="{}" width="1" height="5" fill="{}"/>"#, n(x - 0.5), n(ruler_top - 5.0), hex(self.muted))?;
                }
            }
            writeln!(out, "</g>")?;
        }
        self.svg_tracks(cells, out)?;
        // Names, clipped to their column.
        if self.names_w > 0.0 && self.ch >= 4.0 {
            let fs = (self.ch * 0.75).min(12.0);
            writeln!(out, r#"<clipPath id="names-clip"><rect x="0" y="{}" width="{}" height="{}"/></clipPath>"#, n(top), n(self.names_w - 4.0), n(h - top))?;
            writeln!(out, r#"<g id="names" font-family="{}" font-size="{}" fill="{}" clip-path="url(#names-clip)">"#, escape(&self.ui_font), n(fs), hex(self.fg))?;
            for (i, r) in self.rows.iter().enumerate() {
                let y = top + (i as f64 + 0.5) * self.ch + fs * 0.35;
                writeln!(out, r#"<text x="4" y="{}">{}</text>"#, n(y), escape(&r.label))?;
            }
            writeln!(out, "</g>")?;
        }
        let mut buf = vec![0u8; self.aln.cols()];
        let decode = |src: RowSrc, buf: &mut [u8]| src.decode(self.aln, buf);
        if cells {
            // One rect per run of equal fill in a row.
            writeln!(out, r#"<g id="cells" shape-rendering="crispEdges">"#)?;
            for (i, r) in self.rows.iter().enumerate() {
                decode(r.src, &mut buf);
                let y = top + i as f64 * self.ch;
                let mut v = 0;
                while v < self.cols.len() {
                    let c = self.cols[v];
                    let fill = self.colors.fill(buf[c], &self.column(c));
                    let mut e = v + 1;
                    while e < self.cols.len() && self.colors.fill(buf[self.cols[e]], &self.column(self.cols[e])) == fill {
                        e += 1;
                    }
                    if let Some(f) = fill {
                        writeln!(
                            out,
                            r#"<rect x="{}" y="{}" width="{}" height="{}" fill="{}"/>"#,
                            n(x0 + v as f64 * self.cw),
                            n(y),
                            n((e - v) as f64 * self.cw),
                            n(self.ch),
                            hex(f)
                        )?;
                    }
                    v = e;
                }
            }
            writeln!(out, "</g>")?;
        }
        // Letters: per row, one <text> for letters on a fill and one for the rest, each letter a
        // <tspan> at its own x (renderers differ on lists of x on <text>), so a row selects and
        // copies as its residues.
        if self.cw >= LETTER_MIN.0 && self.ch >= LETTER_MIN.1 {
            let fs = (self.ch.min(self.cw * 1.4) * 0.75).clamp(7.0, 16.0);
            writeln!(out, r#"<g id="letters" font-family="{}" font-size="{}" text-anchor="middle">"#, escape(&self.mono_font), n(fs))?;
            for (i, r) in self.rows.iter().enumerate() {
                decode(r.src, &mut buf);
                let y = top + (i as f64 + 0.5) * self.ch + fs * 0.36;
                let (mut on, mut off) = (String::new(), String::new());
                for (v, &c) in self.cols.iter().enumerate() {
                    let code = buf[c];
                    if code == GAP {
                        continue;
                    }
                    let t = if self.colors.fill(code, &self.column(c)).is_some() { &mut on } else { &mut off };
                    let ch = escape(&(self.aln.symbol(code) as char).to_string());
                    *t += &format!(r#"<tspan x="{}">{ch}</tspan>"#, n(x0 + (v as f64 + 0.5) * self.cw));
                }
                for (spans, color) in [(on, theme.ink()), (off, self.muted)] {
                    if !spans.is_empty() {
                        writeln!(out, r#"<text y="{}" fill="{}">{spans}</text>"#, n(y), hex(color))?;
                    }
                }
            }
            writeln!(out, "</g>")?;
        }
        writeln!(out, "</svg>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_bytes;
    use crate::scheme::{ColorMode, Scheme, Theme};

    fn fig<'a>(aln: &'a Alignment, stats: &'a ColumnStats, colors: &'a Colors, cw: f64) -> Figure<'a> {
        Figure {
            aln,
            stats,
            colors,
            rows: (0..aln.rows()).map(|r| FigRow { src: RowSrc::Seq(r), label: aln.names[r].clone() }).collect(),
            cols: vec![0, 1, 3],
            cw,
            ch: 16.0,
            names_w: 100.0,
            ruler: true,
            ui_font: "Inter, sans-serif".into(),
            mono_font: "JetBrains Mono, monospace".into(),
            fg: 0x4c4f69,
            muted: 0x6c6f85,
            base: None,
            tracks: Vec::new(),
            ruler_ref: None,
        }
    }

    #[test]
    fn svg_structure() {
        let a = parse_bytes(b">a<&b\nMK-V\n>c\nMKAV\n").unwrap();
        let s = ColumnStats::compute(&a);
        let colors = Colors::new(a.alphabet(), ColorMode::new(Scheme::Hydrophobicity), Theme::LIGHT, true);
        let f = fig(&a, &s, &colors, 12.0);
        let mut out = Vec::new();
        f.svg(true, &mut out).unwrap();
        let svg = String::from_utf8(out).unwrap();
        assert!(svg.starts_with("<?xml") && svg.trim_end().ends_with("</svg>"));
        // 100 px of names + 3 columns × 12 px; ruler 22 px + 2 rows × 16 px.
        assert!(svg.contains(r#"width="136" height="54""#), "{svg}");
        assert!(svg.contains("a&lt;&amp;b"));
        // Letters as text, columns 0, 1, 3 (the hidden column 2 is left out): M K V in each row.
        let mkv = r#"<tspan x="106">M</tspan><tspan x="118">K</tspan><tspan x="130">V</tspan></text>"#;
        assert_eq!(svg.matches(mkv).count(), 2, "{svg}");
        // The ruler labels column 1.
        assert!(svg.contains(">1</text>"));
        // The text-only layer has no rects but the names' clip.
        let mut t = Vec::new();
        f.svg(false, &mut t).unwrap();
        let t = String::from_utf8(t).unwrap();
        assert_eq!(t.matches("<rect").count(), 1);
        assert!(t.contains(mkv));
        // Below letter size: no letters.
        let mut small = Vec::new();
        fig(&a, &s, &colors, 4.0).svg(true, &mut small).unwrap();
        assert!(!String::from_utf8(small).unwrap().contains(r#"id="letters""#));
    }

    #[test]
    fn svg_tracks_and_pinned_row() {
        let a = parse_bytes(
            b"# STOCKHOLM 1.0\ns1/8-13 ACDEFG\ns2      AC-EFH\n#=GC SS_cons  HHEEE.\n#=GC PP_cons  9*55..\n#=GC seq_cons ACxEF.\n//\n",
        )
        .unwrap();
        assert_eq!(a.gc.len(), 3);
        let s = ColumnStats::compute(&a);
        let colors = Colors::new(a.alphabet(), ColorMode::new(Scheme::Hydrophobicity), Theme::LIGHT, true);
        let r = Reference::new(&a, 0);
        let mut f = fig(&a, &s, &colors, 12.0);
        f.cols = (0..6).collect();
        f.tracks = a.gc.iter().map(|t| FigTrack { band: TrackBand::Annot { data: &t.data, kind: t.kind() }, label: t.tag.to_string() }).collect();
        f.tracks.push(FigTrack { band: TrackBand::Pinned { codes: &r.codes }, label: a.names[0].clone() });
        f.ruler_ref = Some(&r);
        let svg = |f: &Figure, cells: bool| {
            let mut out = Vec::new();
            f.svg(cells, &mut out).unwrap();
            String::from_utf8(out).unwrap()
        };
        let full = svg(&f, true);
        // 100 px names + 6 × 12 px; ruler 22 + 3 tracks × 14 + pinned 16 + 2 rows × 16.
        assert_eq!(f.tracks_h(), 58.0);
        assert!(full.contains(r#"width="172" height="112""#), "{full}");
        // Labels, as text.
        for l in [">SS_cons</text>", ">PP_cons</text>", ">seq_cons</text>", ">ref</text>", ">s1/8-13</text>"] {
            assert!(full.contains(l), "{l}\n{full}");
        }
        // SS_cons (top 22, middle 29): helix over columns 0–1, a strand over 2–4 ending in an
        // arrowhead 5 px long.
        assert!(full.contains(r#"<rect x="100" y="25" width="24" height="8""#), "{full}");
        assert!(full.contains(r#"<rect x="124" y="26" width="31" height="6""#), "{full}");
        assert!(full.contains(r#"<polygon points="155,24 160,29 155,34""#), "{full}");
        // PP_cons (top 36): bars for 9, * and the run of two 5s; none for the dots.
        // 5 → half of the 10 px bar height, from the bottom 2 px up.
        assert!(full.contains(r#"<rect x="124" y="43" width="24" height="5""#), "{full}");
        assert!(full.contains(r#"<rect x="100" y="39" width="12" height="9""#), "{full}");
        // seq_cons (top 50): its characters as text, dots left out.
        assert!(full.contains(r#"<tspan x="106">A</tspan><tspan x="118">C</tspan><tspan x="130">x</tspan><tspan x="142">E</tspan><tspan x="154">F</tspan></text>"#), "{full}");
        // The pinned row (top 64): an accent line along its top, cells, and its residues.
        assert!(full.contains(r#"<rect x="0" y="64" width="172" height="1""#), "{full}");
        assert!(full.contains(r#"<tspan x="166">G</tspan>"#));
        // The ruler counts the reference's residues: its first (8) and 10.
        assert!(full.contains(">8</text>") && full.contains(">10</text>") && !full.contains(">1</text>"), "{full}");
        // Rows start under the tracks: first name at 80 + 8 + 12 × 0.35.
        assert!(full.contains(r#"<text x="4" y="92.2">s1/8-13</text>"#), "{full}");
        // Text layer of a PNG: no band shapes, the same text and the pinned row's marks.
        let text = svg(&f, false);
        assert!(!text.contains("track-bands") && !text.contains("<polygon"));
        assert!(text.contains(">seq_cons</text>") && text.contains(r#"<tspan x="166">G</tspan>"#));
        assert!(text.contains(r#"<rect x="0" y="64" width="172" height="1""#));
        // No name column: no labels; narrow columns: no characters.
        f.names_w = 0.0;
        f.cw = 4.0;
        let bare = svg(&f, true);
        assert!(!bare.contains("track-labels") && !bare.contains("track-text") && bare.contains("track-bands"));
        // Without tracks nothing changes.
        f.tracks.clear();
        assert!(!svg(&f, true).contains(r#"id="tracks""#));
    }

    #[test]
    fn limits() {
        assert!(check_svg(500, 400).is_ok());
        let e = check_svg(32_000, 700).unwrap_err();
        assert!(e.contains("200,000 cells (this is 22,400,000") && e.contains("export a PNG"), "{e}");
        assert!(check_svg(0, 5).is_err());
        assert!(check_png(16_384, 6_000).is_ok());
        assert!(check_png(16_385, 10).is_err());
        assert!(check_png(12_000, 12_000).unwrap_err().contains("100 megapixels"));
        assert_eq!(size(196.0, true, 0.0, 10, 20, 12.0, 16.0), (436.0, 182.0));
        assert_eq!(size(196.0, true, 30.0, 10, 20, 12.0, 16.0), (436.0, 212.0));
    }
}
