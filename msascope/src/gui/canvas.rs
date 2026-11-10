//! Painting the alignment area, the name panel and the track header in one canvas, so rows and
//! columns line up within a frame. Every colour comes from the tokens, every size from metrics.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::ActiveTheme;
use gpui_kit::*;
use msascope::alphabet::GAP;
use msascope::scheme::Colors;
use msascope::view::{self, Scene, TILE, Viewport, aln_col};
use rayon::prelude::*;

use super::app::MsaScope;
use super::fonts::tabular;
use super::metrics::*;
use super::tokens::tokens;

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

fn rgb_h(c: u32) -> Hsla {
    rgb(c).into()
}

/// A small triangle pointing up with its base centred on `x` at `y`.
fn notch(x: Pixels, y: Pixels, color: Hsla, window: &mut Window) {
    let mut p = Path::new(point(x - px(3.), y));
    p.line_to(point(x, y - px(5.)));
    p.line_to(point(x + px(3.), y));
    p.line_to(point(x - px(3.), y));
    window.paint_path(p, color);
}

impl MsaScope {
    pub(super) fn alignment_canvas(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let data = self.data.clone();
        let tiles = self.tiles.clone();
        let area_cell = self.area.clone();
        let (vp, mode) = (self.vp, self.mode);
        let hidden = self.hidden.clone();
        let selected = self.selected_row;
        let (hover, hover_name) = (self.hover, self.hover_name);
        let name_w = self.settings.name_width;
        let t = tokens(cx);
        let theme = cx.theme();
        let mono: SharedString = theme.mono_font_family.clone();
        let ui: SharedString = theme.font_family.clone();
        let paint_ms = self.bench.as_ref().map(|b| b.paint_ms.clone());
        let scale = window.scale_factor() as f64;
        let letters = vp.cw >= LETTER_W && vp.ch >= LETTER_H;
        let tile_theme = t.tile_theme();
        canvas(
            move |bounds, window, _| {
                let area = Bounds::new(
                    point(bounds.origin.x + px(name_w), bounds.origin.y + px(HEADER_H)),
                    size((bounds.size.width - px(name_w)).max(px(0.)), (bounds.size.height - px(HEADER_H)).max(px(0.))),
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
                let map = hidden.as_deref().map(|h| &h.map);
                let pyramid = hidden.as_deref().map_or(&d.pyramid, |h| &h.pyramid);
                let generation = hidden.as_deref().map_or(0, |h| h.generation);
                let (rows, cols) = (d.aln.rows(), view::view_cols(map, d.aln.cols()));
                let colors = Colors::new(d.aln.alphabet(), mode, tile_theme, letters);
                let scene = Scene { aln: &d.aln, stats: &d.stats, pyramid, map, colors: &colors };
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
                let mut glyphs: HashMap<u8, Option<CellGlyph>> = HashMap::new();
                let mut glyph = |ch: u8, window: &mut Window| -> Option<CellGlyph> {
                    *glyphs.entry(ch).or_insert_with(|| {
                        let s: SharedString = (ch as char).to_string().into();
                        let line = window.text_system().shape_line(s, font_px, &[run(&mono_font, 1, t.res_ink)], None);
                        let run = line.runs.first()?;
                        let g = run.glyphs.first()?;
                        Some(CellGlyph { font: run.font_id, id: g.id, dx: (cw_px - line.width) / 2.0 + g.position.x, ascent: line.ascent, descent: line.descent })
                    })
                };
                // Letters on a fill take the residue ink; on the background (no fill), muted text.
                let paint_letter = |g: CellGlyph, x: Pixels, y: Pixels, line_h: Pixels, filled: bool, window: &mut Window| {
                    let baseline = (line_h - g.ascent - g.descent) / 2.0 + g.ascent;
                    let ink = if filled { t.res_ink } else { t.fg_muted };
                    let _ = window.paint_glyph(point(x + g.dx, y + baseline), g.font, g.id, font_px, ink);
                };
                // A gap at letter zoom: a small centred dot instead of a dash.
                let dot = px((vp.cw.min(vp.ch) * 0.16).clamp(1.5, 3.0) as f32);

                // 2. Letters on cells.
                window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
                    if letters {
                        let columns: Vec<_> = vis_c.clone().map(|v| (v, aln_col(map, v))).map(|(v, c)| (v, c, scene.column(c))).collect();
                        for r in vis_r.clone() {
                            for (v, c, col) in &columns {
                                let code = d.aln.matrix.get(r, *c);
                                if code == GAP {
                                    let o = point(cell_x(*v) + (cw_px - dot) / 2.0, cell_y(r) + (ch_px - dot) / 2.0);
                                    window.paint_quad(fill(Bounds::new(o, size(dot, dot)), t.gap_ink).corner_radii(dot / 2.0));
                                    continue;
                                }
                                if let Some(g) = glyph(d.aln.symbol(code), window) {
                                    paint_letter(g, cell_x(*v), cell_y(r), ch_px, colors.fill(code, col).is_some(), window);
                                }
                            }
                        }
                    }
                    if let Some(r) = selected.filter(|r| vis_r.contains(r)) {
                        let h = ch_px.max(px(2.));
                        window.paint_quad(outline(Bounds::new(point(area.origin.x, cell_y(r)), size(area.size.width, h)), t.accent, BorderStyle::Solid));
                    }
                    if let Some((r, c)) = hover
                        && vp.cw >= HOVER_MIN
                        && vp.ch >= HOVER_MIN
                    {
                        let b = Bounds::new(point(cell_x(c), cell_y(r)), size(cw_px, ch_px));
                        window.paint_quad(outline(b, t.fg, BorderStyle::Solid));
                    }
                });

                // 3. Sticky name panel.
                let names = Bounds::new(point(bounds.origin.x, area.origin.y), size(px(name_w), area.size.height));
                window.paint_quad(fill(names, t.bg));
                window.with_content_mask(Some(ContentMask { bounds: names }), |window| {
                    let x0 = names.origin.x + SPACE_4;
                    let avail = px(name_w) - SPACE_4 * 2.0;
                    if vp.ch >= NAME_ROW_MIN {
                        let fpx = FONT_SMALL.min(px((vp.ch * 0.8) as f32));
                        let line_h = fpx * 1.3;
                        let mut wrapper = window.text_system().line_wrapper(ui_font.clone(), fpx);
                        for r in vis_r.clone() {
                            let y = cell_y(r);
                            let row = Bounds::new(point(names.origin.x, y), size(px(name_w), ch_px));
                            if Some(r) == selected {
                                window.paint_quad(fill(row, t.grid_header_active));
                            } else if Some(r) == hover_name || hover.is_some_and(|(hr, _)| hr == r) {
                                window.paint_quad(fill(row, t.hover));
                            }
                            // "ID/start-end": the range after the last '/' in the dim colour.
                            let name = &d.aln.names[r];
                            let split = name.rfind('/').filter(|&i| i > 0 && name[i + 1..].contains('-')).unwrap_or(name.len());
                            let runs = [run(&ui_font, split, t.fg), run(&ui_font, name.len() - split, t.fg_dim)];
                            let runs = if split == name.len() { &runs[..1] } else { &runs[..] };
                            let (text, runs) = wrapper.truncate_line(SharedString::from(name.clone()), avail, "…", runs, TruncateFrom::End);
                            let line = window.text_system().shape_line(text, fpx, &runs, None);
                            let ty = y + (ch_px - line_h) / 2.0;
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
                });
                window.paint_quad(fill(Bounds::new(point(names.origin.x + px(name_w - 1.), bounds.origin.y), size(px(1.), bounds.size.height)), t.border));

                // 4. Track header: ruler, consensus, identity and gaps.
                let header = Bounds::new(point(area.origin.x, bounds.origin.y), size(area.size.width, px(HEADER_H)));
                window.paint_quad(fill(header, t.grid_header));
                let caption = |s: &str, color: Hsla, window: &mut Window| window.text_system().shape_line(s.to_owned().into(), FONT_CAPTION, &[run(&ui_font, s.len(), color)], None);
                window.with_content_mask(Some(ContentMask { bounds: header }), |window| {
                    // Ruler, in the alignment's own column numbers: a label on every column whose
                    // number is a multiple of the step (labels at least TICK_LABEL_GAP apart once
                    // hidden columns make the numbers uneven), minor ticks halfway.
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
                    window.paint_quad(fill(Bounds::new(point(header.origin.x, ruler_base), size(header.size.width, px(1.))), t.border));

                    // Tracks, one entry per device pixel column, merged into runs of equal colour/height.
                    let npx = dev_vp.width.ceil() as usize;
                    let track = view::column_track(&d.stats, map, dev_vp.col0, cwd, npx);
                    let cons_y = header.origin.y + px(RULER_H);
                    let ident_y = cons_y + px(CONS_H);
                    let gaps_y = ident_y + px(IDENT_H);
                    let ident_h = |id: f32| (id * (IDENT_H - 6.0)).round();
                    let gaps_h = |g: f32| (g * (GAPS_H - 2.0)).round();
                    // The consensus residue in the scheme's colour for its column; none where the
                    // scheme leaves it unfilled.
                    let fill_of = |code: u8, col: usize| colors.fill(code, &scene.column(col));
                    let mut i = 0;
                    while i < npx {
                        let Some(p) = track[i] else { break };
                        let (fc, hi, hq, gq) = (fill_of(p.code, p.col), p.identity >= 0.9, ident_h(p.identity), gaps_h(p.gaps));
                        let mut j = i + 1;
                        while j < npx
                            && track[j].is_some_and(|q| fill_of(q.code, q.col) == fc && (q.identity >= 0.9) == hi && ident_h(q.identity) == hq && gaps_h(q.gaps) == gq)
                        {
                            j += 1;
                        }
                        let (x0, x1) = (to_x(i as f64), to_x(j as f64));
                        if let Some(fc) = fc {
                            window.paint_quad(fill(Bounds::new(point(x0, cons_y + px(1.)), size(x1 - x0, px(CONS_H - 2.))), rgb_h(fc)));
                        }
                        let bar = if hi { t.track_hi } else { t.track_lo };
                        window.paint_quad(fill(Bounds::new(point(x0, ident_y + px(IDENT_H - 2.0 - hq)), size(x1 - x0, px(hq))), bar));
                        if gq > 0.0 {
                            window.paint_quad(fill(Bounds::new(point(x0, gaps_y + px(1.)), size(x1 - x0, px(gq))), t.fg_dim));
                        }
                        i = j;
                    }
                    if letters {
                        for v in vis_c.clone() {
                            let c = aln_col(map, v);
                            let code = d.stats.consensus[c];
                            if code == GAP {
                                continue;
                            }
                            if let Some(g) = glyph(d.aln.symbol(code), window) {
                                paint_letter(g, cell_x(v), cons_y, px(CONS_H), fill_of(code, c).is_some(), window);
                            }
                        }
                    }
                    // Hidden columns: a notch on the ruler where each hidden run was, and a faint
                    // line down through the tracks (runs closer than 4 px share one mark).
                    if let Some(m) = map {
                        let mut last = f32::NEG_INFINITY;
                        let line = Hsla { a: 0.3, ..t.warning };
                        for (v, _) in m.hidden_runs(d.aln.cols(), vis_c.clone()) {
                            let x = cell_x(v);
                            if f32::from(x) - last < 4.0 {
                                continue;
                            }
                            last = f32::from(x);
                            window.paint_quad(fill(Bounds::new(point(x, ruler_base + px(1.)), size(px(1.), px(HEADER_H - RULER_H))), line));
                            notch(x + px(0.5), ruler_base + px(1.), t.warning, window);
                        }
                    }
                });
                // Corner: track labels, right-aligned against the alignment.
                let corner = Bounds::new(bounds.origin, size(px(name_w), px(HEADER_H)));
                window.paint_quad(fill(corner, t.grid_header));
                for (label, y, h) in [("Column", 0.0, RULER_H), ("Consensus", RULER_H, CONS_H), ("Identity %", RULER_H + CONS_H, IDENT_H), ("Gaps", RULER_H + CONS_H + IDENT_H, GAPS_H)] {
                    let line = caption(label, t.fg_muted, window);
                    let lh = FONT_CAPTION * 1.25;
                    let ty = bounds.origin.y + px(y) + (px(h) - lh) / 2.0;
                    let _ = line.paint(point(corner.origin.x + px(name_w) - SPACE_4 - line.width, ty), lh, TextAlign::Left, None, window, cx);
                }
                window.paint_quad(fill(Bounds::new(point(names.origin.x + px(name_w - 1.), bounds.origin.y), size(px(1.), px(HEADER_H))), t.border));
                window.paint_quad(fill(Bounds::new(point(bounds.origin.x, area.origin.y - px(1.)), size(bounds.size.width, px(1.))), t.border));
                if let Some(p) = paint_ms {
                    p.borrow_mut().push(t0.elapsed().as_secs_f64() * 1000.0);
                }
            },
        )
        .size_full()
    }
}
