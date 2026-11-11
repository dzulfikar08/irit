//! The overview panel on the right: the whole alignment (shown columns) as one image drawn with
//! the density rule, the visible area as an accent box, the visible range, and the legend of the
//! colour scheme. Clicking or dragging in the image pans the view.

use std::sync::Arc;

use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use msascope::scheme::{self, Colors};
use msascope::view::{self, Scene, aln_col};
use rayon::prelude::*;

use super::app::{Drag, MsaScope};
use super::components::section_label;
use super::fonts::tabular;
use super::metrics::*;
use super::tokens::tokens;

/// Rows of the image rendered per parallel task.
const STRIP: usize = 32;

impl MsaScope {
    pub(super) fn render_overview(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = tokens(cx);
        let d = self.data.clone().expect("overview needs data");
        let (rows, cols) = self.dims();
        let map = self.col_map();
        // Leave room for the range text and the legend under the image.
        let avail = f32::from(window.viewport_size().height) - f32::from(TITLE_BAR_HEIGHT + CONTROL_LG + STATUS_BAR_HEIGHT) - 330.0;
        let img_h = avail.clamp(80.0, 520.0);
        let img_w = OVERVIEW_W - 2.0 * f32::from(SPACE_4) - 2.0;

        let (vis_r, vis_c) = self.vp.visible(rows, cols);
        let meta = if cols == 0 || rows == 0 {
            vec!["No columns shown".to_string()]
        } else {
            let (c0, c1) = (aln_col(map, vis_c.start.min(cols - 1)) + 1, aln_col(map, vis_c.end.max(1) - 1) + 1);
            let mut m = vec![
                format!("Columns {}–{} of {}", view::group_digits(c0), view::group_digits(c1), view::group_digits(d.aln.cols())),
                format!("Rows {}–{} of {}", view::group_digits(vis_r.start + 1), view::group_digits(vis_r.end), view::group_digits(rows)),
            ];
            if self.hidden_count() > 0 {
                m.push(format!("{} columns hidden", view::group_digits(self.hidden_count())));
            }
            m
        };

        let theme = t.tile_theme();
        let mut legend = scheme::legend(self.mode.scheme, d.aln.kind(), &theme);
        // Differences against the reference row.
        if self.layout.as_deref().is_some_and(|l| l.base.is_some()) {
            for (_, label) in &mut legend {
                *label = match *label {
                    "Differs from consensus" => "Differs from reference",
                    "Matches consensus" => "Matches reference",
                    l => l,
                };
            }
        }
        let cache = self.overview.clone();
        let area = self.overview_area.clone();
        let layout = self.layout.clone();
        let (mode, vp) = (self.mode, self.vp);
        let scale = window.scale_factor();
        let image = canvas(
            move |bounds, _, _| {
                area.set(bounds);
                bounds
            },
            move |_, bounds, window, _| {
                let map = layout.as_deref().and_then(|l| l.map.as_ref());
                let rview = layout.as_deref().and_then(|l| l.rows.as_ref());
                let pyramid = layout.as_deref().map_or(&d.pyramid, |l| &l.pyramid);
                let generation = layout.as_deref().map_or(0, |l| l.generation);
                let (rows, cols) = (view::view_rows(rview, d.aln.rows()), view::view_cols(map, d.aln.cols()));
                if rows == 0 || cols == 0 {
                    return;
                }
                let (w, h) = ((f32::from(bounds.size.width) * scale).round() as usize, (f32::from(bounds.size.height) * scale).round() as usize);
                if w == 0 || h == 0 {
                    return;
                }
                // The alignment stretched to the image: the same density rule as the tiles.
                let key = (mode, theme, generation, w as u32, h as u32);
                let mut c = cache.borrow_mut();
                if c.key != Some(key) {
                    if let Some(old) = c.image.take() {
                        let _ = window.drop_image(old);
                    }
                    let colors = Colors::new(d.aln.alphabet(), mode, theme, false);
                    let scene = Scene { aln: &d.aln, stats: &d.stats, pyramid, map, rows: rview, colors: &colors, base: layout.as_deref().and_then(|l| l.base.as_deref()).map(|b| &b.codes[..]) };
                    let (cw, ch) = (w as f64 / cols as f64, h as f64 / rows as f64);
                    let mut buf = vec![0u8; w * h * 4];
                    buf.par_chunks_mut(w * 4 * STRIP).enumerate().for_each(|(i, strip)| {
                        let sh = strip.len() / (w * 4);
                        scene.render(cw, ch, 0, (i * STRIP) as i64, w, sh, strip);
                    });
                    let img = image::RgbaImage::from_raw(w as u32, h as u32, buf).unwrap();
                    c.image = Some(Arc::new(RenderImage::new(vec![image::Frame::new(img)])));
                    c.key = Some(key);
                }
                if let Some(img) = c.image.clone() {
                    let _ = window.paint_image(bounds, bounds, Corners::all(RADIUS_SM - px(1.)), img, 0, false);
                }
                // The visible area, at least 4 px on a side.
                let fx = |col: f64| bounds.origin.x + bounds.size.width * (col / cols as f64).clamp(0.0, 1.0) as f32;
                let fy = |row: f64| bounds.origin.y + bounds.size.height * (row / rows as f64).clamp(0.0, 1.0) as f32;
                let (mut x0, mut x1) = (fx(vp.col0), fx(vp.col0 + vp.width / vp.cw));
                let (mut y0, mut y1) = (fy(vp.row0), fy(vp.row0 + vp.height / vp.ch));
                let min = px(4.);
                if x1 - x0 < min {
                    let m = (x0 + x1) / 2.0;
                    (x0, x1) = ((m - min / 2.0).max(bounds.origin.x), (m + min / 2.0).min(bounds.right()));
                }
                if y1 - y0 < min {
                    let m = (y0 + y1) / 2.0;
                    (y0, y1) = ((m - min / 2.0).max(bounds.origin.y), (m + min / 2.0).min(bounds.bottom()));
                }
                let b = Bounds::new(point(x0, y0), size(x1 - x0, y1 - y0));
                window.paint_quad(quad(b, Corners::all(px(2.)), Hsla::transparent_black(), Edges::all(px(2.)), t.accent, BorderStyle::Solid));
            },
        )
        .size_full();

        v_flex()
            .flex_none()
            .w(px(OVERVIEW_W))
            .h_full()
            .p(SPACE_4)
            .gap(SPACE_3)
            .bg(t.chrome)
            .border_l_1()
            .border_color(t.border)
            .text_size(FONT_CAPTION)
            .font_features(tabular())
            .child(section_label("Overview", cx))
            .child(
                div()
                    .id("overview-image")
                    .flex_none()
                    .w(px(img_w + 2.0))
                    .h(px(img_h + 2.0))
                    .rounded(RADIUS_SM)
                    .border_1()
                    .border_color(t.border)
                    .bg(t.bg)
                    .cursor_pointer()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|v, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            v.drag = Some(Drag::Overview);
                            v.fit = None;
                            v.overview_pan(ev.position);
                            cx.notify();
                        }),
                    )
                    .child(image),
            )
            .child(v_flex().gap(px(1.)).text_color(t.fg_muted).children(meta).child(div().text_color(t.fg_dim).child("Click or drag to pan")))
            .child(div().pt(SPACE_4).child(section_label(self.mode.scheme.label(), cx)))
            .child(v_flex().gap(SPACE_1).text_color(t.fg_muted).children(legend.into_iter().map(|(c, label)| {
                h_flex().gap(SPACE_3).child(div().flex_none().w(px(12.)).h(px(10.)).rounded(px(2.)).bg(rgb(c))).child(label)
            })))
            .when(self.mode.by_conservation, |p| p.child(div().text_color(t.fg_dim).child("Faded where the column varies (by conservation)")))
    }
}
