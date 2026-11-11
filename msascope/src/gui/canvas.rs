//! Painting the alignment area, the name panel and the track header in one canvas, so rows and
//! columns line up within a frame. Every colour comes from the tokens, every size from metrics.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::ActiveTheme;
use gpui_kit::*;
use msascope::alphabet::GAP;
use msascope::annot::TrackKind;
use msascope::header::{AnnotColors, Band, BandSpec, HeaderKey, Strip};
use msascope::scheme::Colors;
use msascope::view::{self, Scene, TILE, Viewport, aln_col, row_src};
use rayon::prelude::*;

use super::app::MsaScope;
use super::bands::HeaderRow;
use super::fonts::tabular;
use super::metrics::*;
use super::tokens::tokens;
use super::tree_panel::{TreeGeom, paint_tree};

/// A residue letter shaped for the current cell size: its glyph, the x offset that centres it
/// in a cell, and the font metrics to place its baseline.
#[derive(Clone, Copy)]
struct CellGlyph {
    font: FontId,
    id: GlyphId,
    dx: Pixels,
    ascent: Pixels,
    descent: Pixels,
}

/// An opaque colour as 0xRRGGBB.
fn hex(c: Hsla) -> u32 {
    let c = Rgba::from(c);
    let ch = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    ch(c.r) << 16 | ch(c.g) << 8 | ch(c.b)
}

/// Runs of `true` in `flags`, as half-open index ranges.
fn runs(flags: &[bool]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < flags.len() {
        if flags[i] {
            let j = flags[i..].iter().position(|&f| !f).map_or(flags.len(), |k| i + k);
            out.push((i, j));
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

impl MsaScope {
    pub(super) fn alignment_canvas(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let data = self.data.clone();
        let tiles = self.tiles.clone();
        let area_cell = self.area.clone();
        let (vp, mode) = (self.vp, self.mode);
        let layout = self.layout.clone();
        let sel = self.sel.clone();
        let cursor = self.cursor;
        // Motif hits, and the current one (index into the hits).
        let motif = self.motif.as_ref().map(|m| (m.hits.clone(), m.idx.and_then(|i| m.nav.get(i)).copied()));
        let tree_ix = self.tree.clone();
        let tree = self.tree.clone().filter(|_| self.show_tree());
        let tree_hits = self.tree_hits.clone();
        let tree_w = self.tree_w();
        let (hover, hover_name) = (self.hover, self.hover_name);
        let name_w = self.settings.name_width;
        let left_w = tree_w + name_w;
        let t = tokens(cx);
        let theme = cx.theme();
        let mono: SharedString = theme.mono_font_family.clone();
        let ui: SharedString = theme.font_family.clone();
        let paint_ms = self.bench.as_ref().map(|b| b.paint_ms.clone());
        let scale = window.scale_factor() as f64;
        let letters = vp.cw >= LETTER_W && vp.ch >= LETTER_H;
        let tile_theme = t.tile_theme();
        let bands = self.header_bands();
        let header_h = bands.last().map_or(HEADER_H, |b| b.y + b.h);
        let band_labels: Vec<SharedString> = bands.iter().map(|b| self.band_label(b.row)).collect();
        let header_tiles = self.header_tiles.clone();
        let ref_row = self.ref_row.clone();
        let ref_ruler = self.ref_opts.ruler;
        canvas(
            move |bounds, window, _| {
                let area = Bounds::new(
                    point(bounds.origin.x + px(left_w), bounds.origin.y + px(header_h)),
                    size((bounds.size.width - px(left_w)).max(px(0.)), (bounds.size.height - px(header_h)).max(px(0.))),
                );
                // The view reads the area size in `render`, which ran before this layout: render again
                // on change. (`refresh` does nothing while the window draws; this schedules a frame.)
                if area_cell.replace(area).size != area.size {
                    window.request_animation_frame();
                }
                area
            },
            move |bounds, area, window, cx| {
                let t0 = Instant::now();
                window.paint_quad(fill(bounds, t.bg));
                let Some(d) = data else { return };
                let map = layout.as_deref().and_then(|l| l.map.as_ref());
                let rview = layout.as_deref().and_then(|l| l.rows.as_ref());
                let pyramid = layout.as_deref().map_or(&d.pyramid, |l| &l.pyramid);
                let generation = layout.as_deref().map_or(0, |l| l.generation);
                let (rows, cols) = (view::view_rows(rview, d.aln.rows()), view::view_cols(map, d.aln.cols()));
                let colors = Colors::new(d.aln.alphabet(), mode, tile_theme, letters);
                let base = layout.as_deref().and_then(|l| l.base.as_deref());
                let scene = Scene { aln: &d.aln, stats: &d.stats, pyramid, map, rows: rview, colors: &colors, base: base.map(|b| &b.codes[..]) };
                // The alignment row of a view row (a collapsed clade's first member).
                let first_row = |r: usize| rview.map_or(r, |m| m.rows_of(r)[0] as usize);
                let mono_font = Font { family: mono, ..window.text_style().font() };
                let ui_font = Font { family: ui, features: tabular(), ..window.text_style().font() };
                let run = |font: &Font, len: usize, color: Hsla| TextRun { len, font: font.clone(), color, background_color: None, underline: None, strikethrough: None };

                // Device-pixel geometry, with the scroll offset snapped to whole pixels so tiles land on the pixel grid.
                let (cwd, chd) = (vp.cw * scale, vp.ch * scale);
                let off_x = (vp.col0 * cwd).round();
                let off_y = (vp.row0 * chd).round();
                let dev_vp = Viewport {
                    col0: off_x / cwd,
                    row0: off_y / chd,
                    cw: cwd,
                    ch: chd,
                    width: f64::from(area.size.width) * scale,
                    height: f64::from(area.size.height) * scale,
                };
                let to_x = |dev: f64| area.origin.x + px((dev / scale) as f32);
                let to_y = |dev: f64| area.origin.y + px((dev / scale) as f32);

                // 1. Alignment tiles.
                {
                    let mut cache = tiles.borrow_mut();
                    let key = (vp.cw.to_bits(), vp.ch.to_bits(), mode, tile_theme, letters, (scale * 100.0) as u32, generation);
                    if cache.key != Some(key) {
                        cache.reset(window);
                        cache.key = Some(key);
                    }
                    cache.frame += 1;
                    let frame = cache.frame;
                    let wanted: Vec<(i64, i64)> = dev_vp.tiles().collect();
                    let missing: Vec<(i64, i64)> = wanted.iter().filter(|k| !cache.tiles.contains_key(k)).copied().collect();
                    if !missing.is_empty() {
                        let rendered: Vec<_> = missing
                            .par_iter()
                            .map(|&(tx, ty)| {
                                let mut buf = vec![0u8; TILE * TILE * 4];
                                scene.render_tile(cwd, chd, tx, ty, &mut buf);
                                let img = image::RgbaImage::from_raw(TILE as u32, TILE as u32, buf).unwrap();
                                ((tx, ty), Arc::new(RenderImage::new(vec![image::Frame::new(img)])))
                            })
                            .collect();
                        cache.rendered += rendered.len();
                        for (k, img) in rendered {
                            cache.tiles.insert(k, (img, frame));
                        }
                    }
                    window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
                        let tile_size = size(px((TILE as f64 / scale) as f32), px((TILE as f64 / scale) as f32));
                        for k in &wanted {
                            let entry = cache.tiles.get_mut(k).unwrap();
                            entry.1 = frame;
                            let (x, y) = (k.0 as f64 * TILE as f64 - off_x, k.1 as f64 * TILE as f64 - off_y);
                            let b = Bounds::new(point(to_x(x), to_y(y)), tile_size);
                            let _ = window.paint_image(b, b, Corners::default(), entry.0.clone(), 0, false);
                        }
                    });
                    let keep = (wanted.len() * 4).max(64);
                    cache.evict(keep, window);
                }

                let (vis_r, vis_c) = dev_vp.visible(rows, cols);
                let cell_x = |c: usize| to_x(c as f64 * cwd - off_x);
                let cell_y = |r: usize| to_y(r as f64 * chd - off_y);
                let (cw_px, ch_px) = (px(vp.cw as f32), px(vp.ch as f32));
                let font_px = px((vp.ch.min(vp.cw * 1.4) * 0.75).clamp(7.0, 16.0) as f32);
                // Each residue letter is shaped once per frame; cells then paint its glyph directly,
                // which skips the per-line work of `ShapedLine::paint` (thousands of cells a frame).
                // Letters in the annotation tracks and the pinned row, which have their own heights.
                let small_px = px((vp.cw * 1.3).clamp(7.0, 10.5) as f32);
                let pin_px = px((f64::from(PIN_H).min(vp.cw * 1.4) * 0.75).clamp(7.0, 12.0) as f32);
                let mut glyphs: HashMap<(u8, u32), Option<CellGlyph>> = HashMap::new();
                let mut glyph_at = |ch: u8, size: Pixels, window: &mut Window| -> Option<CellGlyph> {
                    *glyphs.entry((ch, f32::from(size).to_bits())).or_insert_with(|| {
                        let s: SharedString = (ch as char).to_string().into();
                        let line = window.text_system().shape_line(s, size, &[run(&mono_font, 1, t.res_ink)], None);
                        let run = line.runs.first()?;
                        let g = run.glyphs.first()?;
                        Some(CellGlyph { font: run.font_id, id: g.id, dx: (cw_px - line.width) / 2.0 + g.position.x, ascent: line.ascent, descent: line.descent })
                    })
                };
                // Letters on a fill take the residue ink; on the background (no fill), muted text.
                let paint_letter_at = |g: CellGlyph, size: Pixels, x: Pixels, y: Pixels, line_h: Pixels, filled: bool, window: &mut Window| {
                    let baseline = (line_h - g.ascent - g.descent) / 2.0 + g.ascent;
                    let ink = if filled { t.res_ink } else { t.fg_muted };
                    let _ = window.paint_glyph(point(x + g.dx, y + baseline), g.font, g.id, size, ink);
                };
                let paint_letter = |g: CellGlyph, x: Pixels, y: Pixels, line_h: Pixels, filled: bool, window: &mut Window| paint_letter_at(g, font_px, x, y, line_h, filled, window);
                // A gap at letter zoom: a small centred dot instead of a dash.
                let dot = px((vp.cw.min(vp.ch) * 0.16).clamp(1.5, 3.0) as f32);

                // 2. Letters on cells.
                window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
                    if letters {
                        let columns: Vec<_> = vis_c.clone().map(|v| (v, aln_col(map, v))).map(|(v, c)| (v, c, scene.column(c))).collect();
                        for r in vis_r.clone() {
                            let src = row_src(rview, r);
                            for (v, c, col) in &columns {
                                let code = src.get(&d.aln, *c);
                                if code == GAP {
                                    let o = point(cell_x(*v) + (cw_px - dot) / 2.0, cell_y(r) + (ch_px - dot) / 2.0);
                                    window.paint_quad(fill(Bounds::new(o, size(dot, dot)), t.gap_ink).corner_radii(dot / 2.0));
                                    continue;
                                }
                                if let Some(g) = glyph_at(d.aln.symbol(code), font_px, window) {
                                    paint_letter(g, cell_x(*v), cell_y(r), ch_px, colors.fill(code, col).is_some(), window);
                                }
                            }
                        }
                    }
                    // Selection: cells in selected rows and columns get the accent wash and each
                    // rectangle of them an accent outline. Rows and columns are sampled once per
                    // pixel when cells are smaller than that.
                    if !sel.is_empty() && !vis_r.is_empty() && !vis_c.is_empty() {
                        let sample = |range: std::ops::Range<usize>, per_px: f64| -> Vec<usize> {
                            let step = (1.0 / per_px).ceil().max(1.0) as usize;
                            range.step_by(step).collect()
                        };
                        let rs = sample(vis_r.clone(), vp.ch);
                        let cs = sample(vis_c.clone(), vp.cw);
                        let rflags: Vec<bool> = rs.iter().map(|&r| sel.row_selected(first_row(r) as u32)).collect();
                        let cflags: Vec<bool> = cs.iter().map(|&v| sel.col_selected(aln_col(map, v) as u32)).collect();
                        let (rr, cr) = (runs(&rflags), runs(&cflags));
                        let block = !sel.rows.is_empty() || !sel.cols.is_empty();
                        let outline_ok = rr.len() * cr.len() <= 400;
                        for &(ra, rb) in &rr {
                            let (y0, y1) = (cell_y(rs[ra]), cell_y(rs.get(rb).copied().unwrap_or(vis_r.end)));
                            for &(ca, cb) in &cr {
                                let (x0, x1) = (cell_x(cs[ca]), cell_x(cs.get(cb).copied().unwrap_or(vis_c.end)));
                                let b = Bounds::new(point(x0, y0), size((x1 - x0).max(px(1.)), (y1 - y0).max(px(1.))));
                                window.paint_quad(fill(b, t.accent_soft));
                                if block && outline_ok && !sel.rows.is_empty() {
                                    window.paint_quad(quad(b, Corners::default(), Hsla::transparent_black(), Edges::all(px(1.5)), t.accent, BorderStyle::Solid));
                                }
                            }
                        }
                    }
                    // Collapsed clade rows: a thin outline so they read as one row for many.
                    if let Some(m) = rview
                        && m.clades() > 0
                        && vp.ch >= 4.0
                    {
                        for r in vis_r.clone().filter(|&r| m.clade_of(r).is_some()) {
                            let b = Bounds::new(point(area.origin.x, cell_y(r)), size(area.size.width, ch_px));
                            window.paint_quad(quad(b, Corners::default(), Hsla::transparent_black(), Edges::all(px(1.)), t.border_strong, BorderStyle::Dashed));
                        }
                    }
                    // Motif hits: a wash over the cells from each match's first to last residue (the
                    // shown columns only), the current match outlined.
                    if let Some((hits, current)) = &motif
                        && !hits.hits.is_empty()
                        && !vis_r.is_empty()
                        && !vis_c.is_empty()
                    {
                        let wash = Hsla { a: if t.dark { 0.45 } else { 0.38 }, ..t.warning };
                        let span = |h: &msascope::motif::Hit| -> Option<(usize, usize)> {
                            let a = map.map_or(h.start as usize, |m| m.view_of(h.start as usize).unwrap_or_else(|e| e));
                            let b = map.map_or(h.end as usize + 1, |m| m.view_of(h.end as usize).map_or_else(|e| e, |v| v + 1));
                            let (a, b) = (a.max(vis_c.start), b.min(vis_c.end));
                            (a < b).then_some((a, b))
                        };
                        let mut budget = MOTIF_RECTS_MAX;
                        let draw = |r: usize, i: usize, window: &mut Window| {
                            let Some((a, b)) = span(&hits.hits[i]) else { return };
                            let bx = Bounds::new(point(cell_x(a), cell_y(r)), size((cell_x(b) - cell_x(a)).max(px(1.)), ch_px.max(px(1.))));
                            window.paint_quad(fill(bx, wash));
                            if Some(i as u32) == *current {
                                window.paint_quad(quad(bx, Corners::default(), Hsla::transparent_black(), Edges::all(px(2.)), t.accent, BorderStyle::Solid));
                            }
                        };
                        if hits.hits.len() < vis_r.len() {
                            for (i, h) in hits.hits.iter().enumerate() {
                                let vr = match rview {
                                    None => Some(h.row as usize),
                                    Some(m) => m.shown(h.row as usize).filter(|&v| m.clade_of(v).is_none()),
                                };
                                if let Some(vr) = vr.filter(|v| vis_r.contains(v)) {
                                    draw(vr, i, window);
                                    budget -= 1;
                                    if budget == 0 {
                                        break;
                                    }
                                }
                            }
                        } else {
                            'rows: for r in vis_r.clone() {
                                if rview.is_some_and(|m| m.clade_of(r).is_some()) {
                                    continue;
                                }
                                for i in hits.in_row(first_row(r) as u32) {
                                    draw(r, i, window);
                                    budget -= 1;
                                    if budget == 0 {
                                        break 'rows;
                                    }
                                }
                            }
                        }
                    }
                    if let Some((r, c)) = cursor.filter(|&(r, c)| vis_r.contains(&r) && vis_c.contains(&c)) {
                        let b = Bounds::new(point(cell_x(c), cell_y(r)), size(cw_px.max(px(2.)), ch_px.max(px(2.))));
                        window.paint_quad(quad(b, Corners::default(), Hsla::transparent_black(), Edges::all(px(2.)), t.accent, BorderStyle::Solid));
                    }
                    if let Some((r, c)) = hover
                        && vp.cw >= HOVER_MIN
                        && vp.ch >= HOVER_MIN
                    {
                        let b = Bounds::new(point(cell_x(c), cell_y(r)), size(cw_px, ch_px));
                        window.paint_quad(outline(b, t.fg, BorderStyle::Solid));
                    }
                });

                // 3. Sticky name panel, right of the tree panel.
                let names = Bounds::new(point(bounds.origin.x + px(tree_w), area.origin.y), size(px(name_w), area.size.height));
                window.paint_quad(fill(names, t.bg));
                window.with_content_mask(Some(ContentMask { bounds: names }), |window| {
                    let x0 = names.origin.x + SPACE_4;
                    let avail = px(name_w) - SPACE_4 * 2.0;
                    if vp.ch >= NAME_ROW_MIN {
                        let fpx = FONT_SMALL.min(px((vp.ch * 0.8) as f32));
                        let line_h = fpx * 1.3;
                        let bold = Font { weight: FontWeight::SEMIBOLD, ..ui_font.clone() };
                        let mut wrapper = window.text_system().line_wrapper(ui_font.clone(), fpx);
                        for r in vis_r.clone() {
                            let y = cell_y(r);
                            let row = Bounds::new(point(names.origin.x, y), size(px(name_w), ch_px));
                            if sel.rows.contains(first_row(r) as u32) {
                                window.paint_quad(fill(row, t.grid_header_active));
                            } else if Some(r) == hover_name || hover.is_some_and(|(hr, _)| hr == r) {
                                window.paint_quad(fill(row, t.hover));
                            }
                            if cursor.is_some_and(|(cr, _)| cr == r) {
                                window.paint_quad(fill(Bounds::new(row.origin, size(px(2.), ch_px)), t.accent));
                            }
                            let ty = y + (ch_px - line_h) / 2.0;
                            if let Some(k) = rview.and_then(|m| m.clade_of(r)) {
                                // A collapsed clade: its label (or first member) in semibold, and
                                // the number of sequences on the right.
                                let m = rview.unwrap();
                                let n = m.members(k).len();
                                let count = format!("{} seqs", view::group_digits(n));
                                let cl = window.text_system().shape_line(count.clone().into(), fpx, &[run(&ui_font, count.len(), t.fg_dim)], None);
                                let node = layout.as_deref().and_then(|l| l.tree.as_ref()).map(|tr| tr.collapsed[k]);
                                let label = node
                                    .and_then(|n| tree_ix.as_deref().map(|ix| ix.tree.label(n)))
                                    // A name, not a support value.
                                    .filter(|l| !l.is_empty() && l.parse::<f64>().is_err())
                                    .map(str::to_owned)
                                    .unwrap_or_else(|| format!("{} …", d.aln.names[m.members(k)[0] as usize].split('/').next().unwrap_or("")));
                                let label_runs = [run(&bold, label.len(), t.fg)];
                                let (text, runs) = wrapper.truncate_line(SharedString::from(label.clone()), avail - cl.width - SPACE_4, "…", &label_runs, TruncateFrom::End);
                                let line = window.text_system().shape_line(text, fpx, &runs, None);
                                let _ = line.paint(point(x0, ty), line_h, TextAlign::Left, None, window, cx);
                                let _ = cl.paint(point(names.origin.x + px(name_w) - SPACE_4 - cl.width, ty), line_h, TextAlign::Left, None, window, cx);
                                continue;
                            }
                            // The reference row: a "ref" tag at the right.
                            let mut avail = avail;
                            if ref_row.as_ref().is_some_and(|x| x.row == first_row(r)) {
                                let tag = window.text_system().shape_line("ref".into(), FONT_CAPTION, &[run(&ui_font, 3, t.accent)], None);
                                let tw = tag.width + SPACE_3;
                                let tx = names.origin.x + px(name_w) - SPACE_4 - tw;
                                let chip = Bounds::new(point(tx, y + (ch_px - line_h) / 2.0), size(tw, line_h));
                                window.paint_quad(fill(chip, t.accent_soft).corner_radii(px(3.)));
                                let _ = tag.paint(point(tx + SPACE_3 / 2.0, ty), line_h, TextAlign::Left, None, window, cx);
                                avail -= tw + SPACE_2;
                            }
                            // "ID/start-end": the range after the last '/' in the dim colour.
                            let name = &d.aln.names[first_row(r)];
                            let split = name.rfind('/').filter(|&i| i > 0 && name[i + 1..].contains('-')).unwrap_or(name.len());
                            let runs = [run(&ui_font, split, t.fg), run(&ui_font, name.len() - split, t.fg_dim)];
                            let runs = if split == name.len() { &runs[..1] } else { &runs[..] };
                            let (text, runs) = wrapper.truncate_line(SharedString::from(name.clone()), avail, "…", runs, TruncateFrom::End);
                            let line = window.text_system().shape_line(text, fpx, &runs, None);
                            let _ = line.paint(point(x0, ty), line_h, TextAlign::Left, None, window, cx);
                        }
                    } else if !vis_r.is_empty() {
                        let label = format!("rows {}–{} of {}", view::group_digits(vis_r.start + 1), view::group_digits(vis_r.end), view::group_digits(rows));
                        let line = window.text_system().shape_line(label.clone().into(), FONT_SMALL, &[run(&ui_font, label.len(), t.fg_muted)], None);
                        let _ = line.paint(point(x0, names.origin.y + SPACE_3), FONT_SMALL * 1.4, TextAlign::Left, None, window, cx);
                        // Position marker: where the view sits in the whole alignment.
                        let h = f64::from(names.size.height);
                        let y0 = vis_r.start as f64 / rows as f64 * h;
                        let y1 = (vis_r.end as f64 / rows as f64 * h).max(y0 + 2.0);
                        let bar = Bounds::new(point(names.origin.x + px(name_w) - SPACE_3, names.origin.y + px(y0 as f32)), size(SPACE_2 - px(1.), px((y1 - y0) as f32)));
                        window.paint_quad(fill(bar, t.fg_dim).corner_radii(px(1.5)));
                    }
                    // Where the rows the tree does not have begin.
                    if let Some(m) = rview
                        && (m.tree_end as usize) < rows
                        && tree.is_some()
                        && vis_r.contains(&(m.tree_end as usize))
                    {
                        let y = cell_y(m.tree_end as usize);
                        window.paint_quad(fill(Bounds::new(point(names.origin.x, y - px(1.)), size(px(name_w), px(1.))), t.border_strong));
                    }
                });

                // 3b. Tree panel, left of the names.
                if let (Some(ix), Some(tr), Some(m)) = (tree.as_deref(), layout.as_deref().and_then(|l| l.tree.as_ref()), rview) {
                    let tb = Bounds::new(point(bounds.origin.x, area.origin.y), size(px(tree_w), area.size.height));
                    window.paint_quad(fill(tb, t.bg));
                    window.with_content_mask(Some(ContentMask { bounds: tb }), |window| {
                        let g = TreeGeom { ix, tr, rows: m, bounds: Bounds::new(tb.origin, size(tb.size.width - px(1.), tb.size.height)), row0: off_y / chd, ch: vp.ch };
                        paint_tree(&g, &t, &mut tree_hits.borrow_mut(), window);
                        if (m.tree_end as usize) < rows && vis_r.contains(&(m.tree_end as usize)) {
                            let y = cell_y(m.tree_end as usize);
                            window.paint_quad(fill(Bounds::new(point(tb.origin.x, y - px(1.)), size(px(tree_w), px(1.))), t.border_strong));
                            if vp.ch >= NAME_ROW_MIN {
                                let s = "not in tree";
                                let l = window.text_system().shape_line(s.into(), FONT_CAPTION, &[run(&ui_font, s.len(), t.fg_dim)], None);
                                let _ = l.paint(point(tb.origin.x + SPACE_4, y + px(1.)), FONT_CAPTION * 1.3, TextAlign::Left, None, window, cx);
                            }
                        }
                    });
                    window.paint_quad(fill(Bounds::new(point(bounds.origin.x + px(tree_w - 1.), bounds.origin.y), size(px(1.), bounds.size.height)), t.border));
                } else {
                    tree_hits.borrow_mut().clear();
                }
                window.paint_quad(fill(Bounds::new(point(names.origin.x + px(name_w - 1.), bounds.origin.y), size(px(1.), bounds.size.height)), t.border));

                // 4. Track header: the ruler, then the bands (consensus, identity, gaps, annotation
                // tracks, the pinned reference) drawn from cached tiles, with letters over them.
                let header = Bounds::new(point(area.origin.x, bounds.origin.y), size(area.size.width, px(header_h)));
                window.paint_quad(fill(header, t.grid_header));
                let caption = |s: &str, color: Hsla, window: &mut Window| window.text_system().shape_line(s.to_owned().into(), FONT_CAPTION, &[run(&ui_font, s.len(), color)], None);
                window.with_content_mask(Some(ContentMask { bounds: header }), |window| {
                    // Selected columns: the active header colour behind the ruler numbers.
                    if !sel.cols.is_empty() && !vis_c.is_empty() {
                        let step = (1.0 / vp.cw).ceil().max(1.0) as usize;
                        let cs: Vec<usize> = vis_c.clone().step_by(step).collect();
                        let flags: Vec<bool> = cs.iter().map(|&v| sel.cols.contains(aln_col(map, v) as u32)).collect();
                        for (a, b) in runs(&flags) {
                            let (x0, x1) = (cell_x(cs[a]), cell_x(cs.get(b).copied().unwrap_or(vis_c.end)));
                            window.paint_quad(fill(Bounds::new(point(x0, header.origin.y), size((x1 - x0).max(px(1.)), px(RULER_H))), t.grid_header_active));
                        }
                    }
                    if let Some((_, c)) = cursor.filter(|&(_, c)| vis_c.contains(&c)) {
                        let b = Bounds::new(point(cell_x(c), header.origin.y + px(RULER_H - 3.)), size(cw_px.max(px(2.)), px(2.)));
                        window.paint_quad(fill(b, t.accent));
                    }
                    // Ruler, in the alignment's own column numbers (or the reference's residue
                    // numbers): a label on every column whose number is a multiple of the step
                    // (labels at least TICK_LABEL_GAP apart once hidden columns or gaps make the
                    // numbers uneven), minor ticks halfway.
                    let step = view::tick_step(vp.cw, TICK_LABEL_GAP);
                    let ruler_base = header.origin.y + px(RULER_H) - px(1.);
                    let tick = |v: usize, h: Pixels, window: &mut Window| {
                        let x = cell_x(v) + cw_px / 2.0;
                        window.paint_quad(fill(Bounds::new(point(x, ruler_base - h), size(px(1.), h)), t.border_strong));
                    };
                    let minor = step / 2;
                    let minors = minor >= 1 && step.is_multiple_of(2) && minor as f64 * vp.cw >= 6.0;
                    let label_y = header.origin.y + SPACE_2;
                    let mut last_label = f32::NEG_INFINITY;
                    let mut label = |v: usize, n: usize, centred: bool, window: &mut Window| {
                        let text = view::group_digits(n);
                        let line = caption(&text, t.fg_muted, window);
                        let x = if centred { cell_x(v) + cw_px / 2.0 - line.width / 2.0 } else { cell_x(v) + SPACE_1 };
                        if f32::from(x) < last_label + 6.0 {
                            return false;
                        }
                        last_label = f32::from(x + line.width);
                        let _ = line.paint(point(x, label_y), FONT_CAPTION * 1.3, TextAlign::Left, None, window, cx);
                        true
                    };
                    match ref_row.as_deref().filter(|_| ref_ruler) {
                        Some(r) => {
                            let marks = msascope::reference::ruler_marks(r, map, vis_c.clone(), step, if minors { minor } else { 0 }, vis_c.start == 0);
                            for (v, n, major) in marks {
                                if major {
                                    if label(v, n, true, window) {
                                        tick(v, px(6.), window);
                                    }
                                } else {
                                    tick(v, px(3.), window);
                                }
                            }
                        }
                        None => {
                            if vis_c.start == 0 && cols > 0 {
                                label(0, aln_col(map, 0) + 1, false, window);
                            }
                            if map.is_none() {
                                let mut c = (vis_c.start.div_ceil(step) * step).max(step) - 1;
                                if minors {
                                    let mut m = (vis_c.start.div_ceil(minor) * minor).max(minor) - 1;
                                    while m < vis_c.end {
                                        if !(m + 1).is_multiple_of(step) {
                                            tick(m, px(3.), window);
                                        }
                                        m += minor;
                                    }
                                }
                                while c < vis_c.end {
                                    tick(c, px(6.), window);
                                    label(c, c + 1, true, window);
                                    c += step;
                                }
                            } else {
                                for v in vis_c.clone() {
                                    let n = aln_col(map, v) + 1;
                                    if n.is_multiple_of(step) {
                                        if label(v, n, true, window) {
                                            tick(v, px(6.), window);
                                        }
                                    } else if minors && n.is_multiple_of(minor) {
                                        tick(v, px(3.), window);
                                    }
                                }
                            }
                        }
                    }
                    window.paint_quad(fill(Bounds::new(point(header.origin.x, ruler_base), size(header.size.width, px(1.))), t.border));

                    // The bands below the ruler, as image tiles 256 device pixels wide, drawn once
                    // per scale, scheme, theme, layout and set of bands.
                    let strip_y = header.origin.y + px(RULER_H);
                    let strip_bands: Vec<BandSpec> = bands
                        .iter()
                        .filter_map(|b| {
                            let band = match b.row {
                                HeaderRow::Ruler => return None,
                                HeaderRow::Consensus => Band::Consensus,
                                HeaderRow::Identity => Band::Identity { hi: hex(t.track_hi), lo: hex(t.track_lo) },
                                HeaderRow::Gaps => Band::Gaps { ink: hex(t.fg_dim) },
                                HeaderRow::Gc(i) => Band::Annot { data: &d.aln.gc[i].data, kind: d.aln.gc[i].kind() },
                                HeaderRow::Gr(i) => Band::Annot { data: &d.aln.gr[i].data, kind: TrackKind::of(&d.aln.gr[i].tag) },
                                HeaderRow::Pinned => Band::Row { codes: &ref_row.as_deref()?.codes },
                            };
                            Some(BandSpec { y: b.y - RULER_H, h: b.h, band })
                        })
                        .collect();
                    let strip = Strip { scene: &scene, bands: strip_bands, identity_ref: base, bg: hex(t.grid_header), annot: AnnotColors::new(&tile_theme), scale };
                    let strip_h = strip.height_dev();
                    if strip_h > 0 {
                        let mut cache = header_tiles.borrow_mut();
                        let key = HeaderKey {
                            cw: cwd.to_bits(),
                            scale: (scale * 100.0) as u32,
                            mode,
                            theme: tile_theme,
                            inks: [hex(t.track_hi), hex(t.track_lo), hex(t.fg_dim), hex(t.grid_header)],
                            generation,
                            reference: ref_row.as_ref().map(|r| r.row as u32),
                            bands: bands.iter().map(|b| (b.row.code(), (b.y * 4.0) as u32, (b.h * 4.0) as u32)).collect(),
                        };
                        for img in cache.begin(key) {
                            let _ = window.drop_image(img);
                        }
                        let wanted: Vec<i64> = msascope::header::tiles_x(off_x, dev_vp.width).collect();
                        let missing = cache.missing(&wanted);
                        let rendered: Vec<_> = missing
                            .par_iter()
                            .map(|&tx| {
                                let mut buf = vec![0u8; TILE * strip_h * 4];
                                strip.render(cwd, tx * TILE as i64, TILE, &mut buf);
                                let img = image::RgbaImage::from_raw(TILE as u32, strip_h as u32, buf).unwrap();
                                (tx, Arc::new(RenderImage::new(vec![image::Frame::new(img)])))
                            })
                            .collect();
                        for (tx, img) in rendered {
                            cache.insert(tx, img);
                        }
                        let tile_size = size(px((TILE as f64 / scale) as f32), px((strip_h as f64 / scale) as f32));
                        for tx in &wanted {
                            let img = cache.get(tx).unwrap().clone();
                            let b = Bounds::new(point(to_x(*tx as f64 * TILE as f64 - off_x), strip_y), tile_size);
                            let _ = window.paint_image(b, b, Corners::default(), img, 0, false);
                        }
                        for img in cache.evict((wanted.len() * 4).max(16)) {
                            let _ = window.drop_image(img);
                        }
                    }
                    // Letters over the bands: the consensus at letter zoom, the characters of text
                    // tracks once columns are wide enough, and the pinned row's residues.
                    for b in &bands {
                        let y = header.origin.y + px(b.y);
                        match b.row {
                            HeaderRow::Consensus if letters => {
                                for v in vis_c.clone() {
                                    let c = aln_col(map, v);
                                    let code = d.stats.consensus[c];
                                    if code == GAP {
                                        continue;
                                    }
                                    if let Some(g) = glyph_at(d.aln.symbol(code), font_px, window) {
                                        paint_letter(g, cell_x(v), y, px(b.h), colors.fill(code, &scene.column(c)).is_some(), window);
                                    }
                                }
                            }
                            HeaderRow::Gc(_) | HeaderRow::Gr(_) if vp.cw >= LETTER_W => {
                                let (data, kind) = match b.row {
                                    HeaderRow::Gc(i) => (&d.aln.gc[i].data, d.aln.gc[i].kind()),
                                    HeaderRow::Gr(i) => (&d.aln.gr[i].data, TrackKind::of(&d.aln.gr[i].tag)),
                                    _ => unreachable!(),
                                };
                                if kind != TrackKind::Text {
                                    continue;
                                }
                                for v in vis_c.clone() {
                                    let ch = data[aln_col(map, v)];
                                    if msascope::header::is_blank(ch) {
                                        continue;
                                    }
                                    if let Some(g) = glyph_at(ch, small_px, window) {
                                        paint_letter_at(g, small_px, cell_x(v), y, px(b.h), false, window);
                                    }
                                }
                            }
                            HeaderRow::Pinned if letters => {
                                let Some(r) = ref_row.as_deref() else { continue };
                                for v in vis_c.clone() {
                                    let c = aln_col(map, v);
                                    let code = r.codes[c];
                                    if code == GAP {
                                        continue;
                                    }
                                    if let Some(g) = glyph_at(d.aln.symbol(code), pin_px, window) {
                                        paint_letter_at(g, pin_px, cell_x(v), y, px(b.h), colors.fill(code, &scene.column(c)).is_some(), window);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    // The pinned row: an accent line along its top, and its cursor column.
                    if let Some(b) = bands.iter().find(|b| b.row == HeaderRow::Pinned) {
                        let y = header.origin.y + px(b.y);
                        window.paint_quad(fill(Bounds::new(point(header.origin.x, y), size(header.size.width, px(1.))), t.accent));
                    }
                    // Hidden columns: a thin tick at the ruler's foot where each hidden run was and
                    // a faint line down through the tracks. Runs closer than HIDDEN_MARK_MERGE px
                    // share one mark, drawn 2 px wide; the hover card over the ruler gives the
                    // hidden columns next to the column under the pointer.
                    if let Some(m) = map {
                        let mut marks: Vec<(f32, usize)> = Vec::new();
                        for (v, _) in m.hidden_runs(d.aln.cols(), vis_c.clone()) {
                            let x = f32::from(cell_x(v)).round();
                            match marks.last_mut() {
                                Some((lx, n)) if x - *lx < HIDDEN_MARK_MERGE => *n += 1,
                                _ => marks.push((x, 1)),
                            }
                        }
                        let tick = Hsla { a: 0.75, ..t.warning };
                        let line = Hsla { a: 0.16, ..t.warning };
                        for (x, n) in marks {
                            let w = if n > 1 { px(2.) } else { px(1.) };
                            window.paint_quad(fill(Bounds::new(point(px(x), ruler_base - px(5.)), size(w, px(5.))), tick));
                            window.paint_quad(fill(Bounds::new(point(px(x), ruler_base + px(1.)), size(px(1.), px(header_h - RULER_H))), line));
                        }
                    }
                });
                // Corner: band labels, right-aligned against the alignment; the pinned row's name
                // left of it with an accent mark; over the tree panel, its title.
                let corner = Bounds::new(bounds.origin, size(px(left_w), px(header_h)));
                window.paint_quad(fill(corner, t.grid_header));
                window.with_content_mask(Some(ContentMask { bounds: corner }), |window| {
                    for (b, label) in bands.iter().zip(&band_labels) {
                        let lh = FONT_CAPTION * 1.25;
                        let ty = bounds.origin.y + px(b.y) + (px(b.h) - lh) / 2.0;
                        let right = corner.origin.x + px(left_w) - SPACE_4;
                        if b.row == HeaderRow::Pinned {
                            let x0 = corner.origin.x + px(tree_w);
                            let row = Bounds::new(point(x0, bounds.origin.y + px(b.y)), size(px(name_w), px(b.h)));
                            window.paint_quad(fill(row, t.accent_soft));
                            window.paint_quad(fill(Bounds::new(row.origin, size(px(2.), px(b.h))), t.accent));
                            let tag = caption("ref", t.accent, window);
                            let _ = tag.paint(point(right - tag.width, ty), lh, TextAlign::Left, None, window, cx);
                            let avail = right - tag.width - SPACE_4 - (x0 + SPACE_4);
                            let runs = [run(&ui_font, label.len(), t.fg)];
                            let mut wrapper = window.text_system().line_wrapper(ui_font.clone(), FONT_CAPTION);
                            let (text, runs) = wrapper.truncate_line(label.clone(), avail, "…", &runs, TruncateFrom::End);
                            let line = window.text_system().shape_line(text, FONT_CAPTION, &runs, None);
                            let _ = line.paint(point(x0 + SPACE_4, ty), lh, TextAlign::Left, None, window, cx);
                            continue;
                        }
                        let line = caption(label, t.fg_muted, window);
                        let _ = line.paint(point(right - line.width, ty), lh, TextAlign::Left, None, window, cx);
                    }
                });
                if tree.is_some() {
                    let line = caption("Tree", t.fg_muted, window);
                    let _ = line.paint(point(bounds.origin.x + SPACE_4, bounds.origin.y + px(HEADER_H) - FONT_CAPTION * 1.25 - SPACE_2), FONT_CAPTION * 1.25, TextAlign::Left, None, window, cx);
                    window.paint_quad(fill(Bounds::new(point(bounds.origin.x + px(tree_w - 1.), bounds.origin.y), size(px(1.), px(header_h))), t.border));
                }
                window.paint_quad(fill(Bounds::new(point(names.origin.x + px(name_w - 1.), bounds.origin.y), size(px(1.), px(header_h))), t.border));
                window.paint_quad(fill(Bounds::new(point(bounds.origin.x, area.origin.y - px(1.)), size(bounds.size.width, px(1.))), t.border));
                if let Some(p) = paint_ms {
                    p.borrow_mut().push(t0.elapsed().as_secs_f64() * 1000.0);
                }
            },
        )
        .size_full()
    }
}

