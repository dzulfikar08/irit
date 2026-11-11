//! Figures of the alignment: SVG with the residue letters, names and ruler as real text (vector,
//! selectable), and the size rules for SVG and PNG. The GUI rasterises PNGs (cells with
//! [`crate::view::Scene::render`], text from [`svg`] with `cells: false`).

use std::io::{self, Write};

use crate::Alignment;
use crate::alphabet::GAP;
use crate::scheme::{Colors, Column};
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
}

/// The size an export would have: (width, height) in px at scale 1.
pub fn size(names_w: f64, ruler: bool, rows: usize, cols: usize, cw: f64, ch: f64) -> (f64, f64) {
    (names_w + cols as f64 * cw, if ruler { RULER_PX } else { 0.0 } + rows as f64 * ch)
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
        size(self.names_w, self.ruler, self.rows.len(), self.cols.len(), self.cw, self.ch)
    }

    /// Write the SVG. With `cells` off, only the text (names, ruler, letters) on a transparent
    /// background: the layer a PNG puts over its rasterised cells.
    pub fn svg(&self, cells: bool, out: &mut impl Write) -> io::Result<()> {
        let (w, h) = self.size();
        let top = if self.ruler { RULER_PX } else { 0.0 };
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
            for (v, &c) in self.cols.iter().enumerate() {
                let num = c + 1;
                if !(num.is_multiple_of(step) || v == 0) {
                    continue;
                }
                let x = x0 + (v as f64 + 0.5) * self.cw;
                let label = crate::view::group_digits(num);
                let half = label.len() as f64 * RULER_FONT * 0.32;
                if x - half < last + 4.0 {
                    continue;
                }
                last = x + half;
                writeln!(out, r#"<text x="{}" y="{}">{label}</text>"#, n(x), n(top - 8.0))?;
                if cells {
                    writeln!(out, r#"<rect x="{}" y="{}" width="1" height="5" fill="{}"/>"#, n(x - 0.5), n(top - 5.0), hex(self.muted))?;
                }
            }
            writeln!(out, "</g>")?;
        }
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
    fn limits() {
        assert!(check_svg(500, 400).is_ok());
        let e = check_svg(32_000, 700).unwrap_err();
        assert!(e.contains("200,000 cells (this is 22,400,000") && e.contains("export a PNG"), "{e}");
        assert!(check_svg(0, 5).is_err());
        assert!(check_png(16_384, 6_000).is_ok());
        assert!(check_png(16_385, 10).is_err());
        assert!(check_png(12_000, 12_000).unwrap_err().contains("100 megapixels"));
        assert_eq!(size(196.0, true, 10, 20, 12.0, 16.0), (436.0, 182.0));
    }
}
