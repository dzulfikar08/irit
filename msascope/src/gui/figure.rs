//! Export ▸ PNG / SVG of the view or of the selection (rows × shown columns), with the `#=GC` /
//! `#=GR` tracks and the pinned reference row shown in the window. SVGs come from the core's
//! writer (letters, names, ruler and track characters as text). PNGs rasterise the cells and the
//! track bands with the same code as the tiles, then draw the text layer of that SVG over them
//! with resvg.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

use gpui_kit::component::ActiveTheme;
use gpui_kit::*;
use msascope::annot::TrackKind;
use msascope::export::{self, FigRow, FigTrack, Figure, TrackBand};
use msascope::header::{AnnotColors, Band, BandSpec, Strip};
use msascope::pyramid::Pyramid;
use msascope::scheme::Colors;
use msascope::view::{self, ColumnMap, RowSrc, RowView, Scene, aln_col, row_src};
use resvg::{tiny_skia, usvg};

use super::actions::clade_name;
use super::app::MsaScope;
use super::bands::HeaderRow;
use super::metrics::*;
use super::tokens::tokens;

/// What goes in a figure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FigOptions {
    pub names: bool,
    pub ruler: bool,
    /// PNG pixels per logical pixel.
    pub scale: u32,
}

impl Default for FigOptions {
    fn default() -> Self {
        FigOptions { names: true, ruler: true, scale: 2 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    /// The cells in view, at the current zoom.
    View,
    /// The selected rows (in the order shown) × the selected shown columns, at letter size or the
    /// current zoom if larger.
    Selection,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Png,
    Svg,
}

/// A figure row: a view row of the current layout, or an alignment row.
#[derive(Clone, Copy)]
enum RowRef {
    View(usize),
    Seq(u32),
}

/// System fonts, loaded once (the first PNG takes ~0.1–0.3 s for this).
fn fonts(ui: &str, mono: &str) -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        db.set_sans_serif_family(ui);
        db.set_monospace_family(mono);
        Arc::new(db)
    })
    .clone()
}

impl MsaScope {
    pub(super) fn export_figure(&mut self, region: Region, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        let Some(d) = self.data.clone() else { return };
        let layout = self.layout.clone();
        let map = layout.as_deref().and_then(|l| l.map.as_ref());
        let rview = layout.as_deref().and_then(|l| l.rows.as_ref());
        let (nrows, ncols) = self.dims();
        let opts = self.fig;
        let names_w = if opts.names { self.settings.name_width as f64 } else { 0.0 };
        // The annotation tracks and the pinned row as shown in the window, with their labels.
        let track_rows: Vec<(HeaderRow, String)> =
            self.header_bands().into_iter().filter(|b| matches!(b.row, HeaderRow::Gc(_) | HeaderRow::Gr(_) | HeaderRow::Pinned)).map(|b| (b.row, self.band_label(b.row).to_string())).collect();
        let tracks_h: f64 = track_rows.iter().map(|(r, _)| if *r == HeaderRow::Pinned { export::PIN_PX } else { export::TRACK_PX }).sum();
        let ref_row = self.ref_row.clone();
        let ruler_ref = self.ref_opts.ruler;
        // Rows, alignment columns, view columns (View only) and the cell size.
        let (rows, cols, mut cw, mut ch): (Vec<RowRef>, Vec<usize>, f64, f64) = match region {
            Region::View => {
                let (r, c) = self.vp.visible(nrows, ncols);
                (r.map(RowRef::View).collect(), c.map(|v| aln_col(map, v)).collect(), self.vp.cw, self.vp.ch)
            }
            Region::Selection => {
                if self.sel.is_empty() {
                    self.status = "Nothing selected to export".into();
                    cx.notify();
                    return;
                }
                let sel = self.sel.clone();
                let order: Vec<u32> = rview.map_or_else(|| (0..d.aln.rows() as u32).collect(), RowView::expanded);
                let rows = order.into_iter().filter(|&r| sel.row_selected(r)).map(RowRef::Seq).collect();
                let shown = |c: usize| map.is_none_or(|m| m.view_of(c).is_ok());
                let cols = (0..d.aln.cols()).filter(|&c| sel.col_selected(c as u32) && shown(c)).collect();
                (rows, cols, self.vp.cw.max(view::LETTERS_CW), self.vp.ch.max(view::LETTERS_CH))
            }
        };
        let s = opts.scale.max(1) as f64;
        let check = |cw: f64, ch: f64| {
            let (w, h) = export::size(names_w, opts.ruler, tracks_h, rows.len(), cols.len(), cw, ch);
            match kind {
                Kind::Svg => export::check_svg(rows.len(), cols.len()),
                Kind::Png => export::check_png((w * s).ceil() as usize, (h * s).ceil() as usize),
            }
        };
        let mut ok = check(cw, ch);
        if ok.is_err() && kind == Kind::Png && region == Region::Selection {
            // Too big at letter size: the current zoom, at least a pixel per cell.
            (cw, ch) = (self.vp.cw.max(1.0), self.vp.ch.max(1.0));
            ok = check(cw, ch);
        }
        if let Err(e) = ok {
            self.status = e.into();
            cx.notify();
            return;
        }
        // Labels: names, or a collapsed clade's name.
        let tree = self.tree.clone();
        let label = |r: RowRef| -> String {
            match r {
                RowRef::Seq(a) => d.aln.names[a as usize].clone(),
                RowRef::View(v) => match rview.and_then(|m| m.clade_of(v).map(|k| (m, k))) {
                    Some((m, k)) => {
                        let node = layout.as_deref().and_then(|l| l.tree.as_ref()).map(|t| t.collapsed[k]);
                        match (node, &tree) {
                            (Some(n), Some(ix)) => clade_name(ix, n, &d.aln.names),
                            _ => format!("{} … ({} sequences)", d.aln.names[m.members(k)[0] as usize], m.members(k).len()),
                        }
                    }
                    None => d.aln.names[rview.map_or(v, |m| m.rows[v] as usize)].clone(),
                },
            }
        };
        let labels: Vec<String> = rows.iter().map(|&r| label(r)).collect();
        let t = tokens(cx);
        let theme = cx.theme();
        let (ui, mono) = (theme.font_family.to_string(), theme.mono_font_family.to_string());
        let (fg, muted) = (hsla_rgb(t.fg), hsla_rgb(t.fg_muted));
        let tile_theme = t.tile_theme();
        let mode = self.mode;
        let first_view_col = self.vp.visible(nrows, ncols).1.start;
        let dir = self.path.as_deref().and_then(|p| p.parent()).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let stem = self.path.as_deref().and_then(|p| p.file_name()).and_then(|n| n.to_str()).map_or("alignment", |n| n.split('.').next().unwrap_or(n)).to_string();
        let ext = if kind == Kind::Png { "png" } else { "svg" };
        let name = format!("{stem}.{}.{ext}", if region == Region::View { "view" } else { "selection" });
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        let _ = window;
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let p2 = path.clone();
            let (nr, nc) = (rows.len(), cols.len());
            let write = cx.background_executor().spawn(async move {
                let t0 = Instant::now();
                let letters = cw >= LETTER_W && ch >= LETTER_H;
                let colors = Colors::new(d.aln.alphabet(), mode, tile_theme, letters);
                let map = layout.as_deref().and_then(|l| l.map.as_ref());
                let rview = layout.as_deref().and_then(|l| l.rows.as_ref());
                let base = layout.as_deref().and_then(|l| l.base.as_deref()).map(|b| &b.codes[..]);
                let fig_rows: Vec<FigRow> = rows
                    .iter()
                    .zip(labels)
                    .map(|(&r, label)| FigRow { src: match r { RowRef::View(v) => row_src(rview, v), RowRef::Seq(a) => RowSrc::Seq(a as usize) }, label })
                    .collect();
                let tracks: Vec<FigTrack> = track_rows
                    .into_iter()
                    .filter_map(|(row, label)| {
                        let band = match row {
                            HeaderRow::Gc(i) => TrackBand::Annot { data: &d.aln.gc[i].data, kind: d.aln.gc[i].kind() },
                            HeaderRow::Gr(i) => TrackBand::Annot { data: &d.aln.gr[i].data, kind: TrackKind::of(&d.aln.gr[i].tag) },
                            HeaderRow::Pinned => TrackBand::Pinned { codes: &ref_row.as_deref()?.codes },
                            _ => return None,
                        };
                        Some(FigTrack { band, label })
                    })
                    .collect();
                let fig = Figure {
                    aln: &d.aln,
                    stats: &d.stats,
                    colors: &colors,
                    rows: fig_rows,
                    cols: cols.clone(),
                    cw,
                    ch,
                    names_w,
                    ruler: opts.ruler,
                    ui_font: format!("{ui}, sans-serif"),
                    mono_font: format!("{mono}, monospace"),
                    fg,
                    muted,
                    base,
                    tracks,
                    ruler_ref: ref_row.as_deref().filter(|_| ruler_ref),
                };
                let bytes = match kind {
                    Kind::Svg => {
                        let mut out = Vec::new();
                        fig.svg(true, &mut out)?;
                        out
                    }
                    Kind::Png => {
                        let (w, h) = fig.size();
                        let (pw, ph) = ((w * s).ceil() as u32, (h * s).ceil() as u32);
                        let mut pix = tiny_skia::Pixmap::new(pw, ph).ok_or_else(|| std::io::Error::other("image too large"))?;
                        let bg = tile_theme.bg;
                        pix.fill(tiny_skia::Color::from_rgba8((bg >> 16) as u8, (bg >> 8) as u8, bg as u8, 255));
                        // Cells, drawn as the tiles are, under the ruler and the tracks.
                        let (ox, oy) = ((names_w * s).round() as usize, ((fig.tracks_top() + fig.tracks_h()) * s).round() as usize);
                        let (cwp, chp) = (cw * s, ch * s);
                        let (cw_px, ch_px) = (((nc as f64) * cwp).round() as usize, ((nr as f64) * chp).round() as usize);
                        // The view's own layout, or one of just the selected rows and columns.
                        let (sub_rows, sub_map, own);
                        let (scene, x0, y0) = match rows.first() {
                            Some(RowRef::View(v0)) => {
                                let scene = Scene { aln: &d.aln, stats: &d.stats, pyramid: layout.as_deref().map_or(&d.pyramid, |l| &l.pyramid), map, rows: rview, colors: &colors, base };
                                (scene, (first_view_col as f64 * cwp).round() as i64, (*v0 as f64 * chp).round() as i64)
                            }
                            _ => {
                                let seqs: Vec<u32> = rows.iter().map(|r| if let RowRef::Seq(a) = r { *a } else { 0 }).collect();
                                let n = seqs.len() as u32;
                                sub_rows = RowView::new(seqs, d.aln.rows(), n, Vec::new(), Vec::new(), Vec::new());
                                sub_map = ColumnMap { visible: cols.iter().map(|&c| c as u32).collect() };
                                let pyramid = if cwp >= 1.0 && chp >= 1.0 {
                                    &d.pyramid
                                } else {
                                    own = Pyramid::build_mapped(&d.aln, base.unwrap_or(&d.stats.consensus), Some(&sub_map.visible), Some(&sub_rows));
                                    &own
                                };
                                (Scene { aln: &d.aln, stats: &d.stats, pyramid, map: Some(&sub_map), rows: Some(&sub_rows), colors: &colors, base }, 0, 0)
                            }
                        };
                        let data = pix.data_mut();
                        // BGRA `src` (`w` px wide) onto the image at (x, y), skipping transparent pixels.
                        let mut blit = |src: &[u8], w: usize, h: usize, x: usize, y: usize| {
                            for j in 0..h.min((ph as usize).saturating_sub(y)) {
                                for i in 0..w.min((pw as usize).saturating_sub(x)) {
                                    let k = (j * w + i) * 4;
                                    if src[k + 3] == 0 {
                                        continue;
                                    }
                                    let o = ((y + j) * pw as usize + x + i) * 4;
                                    data[o..o + 4].copy_from_slice(&[src[k + 2], src[k + 1], src[k], 255]);
                                }
                            }
                        };
                        let mut buf = vec![0u8; cw_px * ch_px * 4];
                        scene.render(cwp, chp, x0, y0, cw_px, ch_px, &mut buf);
                        blit(&buf, cw_px, ch_px, ox, oy);
                        drop(buf);
                        // Track bands, drawn as the header's tiles are.
                        if !fig.tracks.is_empty() {
                            let mut y = 0.0f32;
                            let bands = fig
                                .tracks
                                .iter()
                                .map(|t| {
                                    let band = match t.band {
                                        TrackBand::Annot { data, kind } => Band::Annot { data, kind },
                                        TrackBand::Pinned { codes } => Band::Row { codes },
                                    };
                                    let b = BandSpec { y, h: t.height() as f32, band };
                                    y += t.height() as f32;
                                    b
                                })
                                .collect();
                            let strip = Strip { scene: &scene, bands, identity_ref: None, bg: tile_theme.bg, annot: AnnotColors::new(&tile_theme), scale: s };
                            let sh = strip.height_dev();
                            let mut sbuf = vec![0u8; cw_px * sh * 4];
                            strip.render(cwp, x0, cw_px, &mut sbuf);
                            blit(&sbuf, cw_px, sh, ox, (fig.tracks_top() * s).round() as usize);
                        }
                        // Names, ruler and letters on top.
                        let mut svg = Vec::new();
                        fig.svg(false, &mut svg)?;
                        let mut opt = usvg::Options { fontdb: fonts(&ui, &mono), ..Default::default() };
                        opt.font_family = ui.clone();
                        let tree = usvg::Tree::from_data(&svg, &opt).map_err(std::io::Error::other)?;
                        resvg::render(&tree, tiny_skia::Transform::from_scale(s as f32, s as f32), &mut pix.as_mut());
                        pix.encode_png().map_err(std::io::Error::other)?
                    }
                };
                std::fs::write(&p2, &bytes)?;
                std::io::Result::Ok((bytes.len(), t0.elapsed().as_secs_f64()))
            });
            this.update(cx, |v, cx| {
                v.status = format!("Writing {}…", path.display()).into();
                cx.notify();
            })
            .ok();
            let result = write.await;
            this.update(cx, |v, cx| {
                v.status = match result {
                    Ok((bytes, secs)) => {
                        eprintln!("msascope: {} {} rows × {} columns, {} bytes, {secs:.3} s", path.display(), nr, nc, bytes);
                        format!("Wrote {} ({} rows × {} columns, {:.1} MB, {secs:.2} s)", path.display(), view::group_digits(nr), view::group_digits(nc), bytes as f64 / 1e6).into()
                    }
                    Err(e) => format!("Couldn't write {}: {e}", path.display()).into(),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}

/// 0xRRGGBB of a colour.
fn hsla_rgb(c: Hsla) -> u32 {
    let r = Rgba::from(c);
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u32;
    b(r.r) << 16 | b(r.g) << 8 | b(r.b)
}
