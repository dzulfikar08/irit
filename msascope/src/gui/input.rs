//! Mouse and keyboard: panning, zooming, the cell cursor, selecting rows, columns and blocks,
//! collapsing clades in the tree panel, and copying or exporting the selection.

use std::ops::RangeInclusive;
use std::path::PathBuf;
use std::rc::Rc;

use gpui_kit::*;
use msascope::select::{self, ExportOptions, Extract, RangeSet, Selection};
use msascope::view;

use super::app::{Drag, Fit, MsaScope};
use super::metrics::*;

/// What is under a point of the window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Region {
    /// (view row, view column) of the alignment.
    Cell(usize, usize),
    /// A view row's name.
    Name(usize),
    Tree,
    /// A view column of the track header.
    Header(usize),
    Outside,
}

impl MsaScope {
    pub(super) fn region(&self, pos: Point<Pixels>) -> Region {
        let area = self.area.get();
        let (rows, cols) = self.dims();
        let (x, y) = (
            f64::from(pos.x - area.origin.x),
            f64::from(pos.y - area.origin.y),
        );
        let name_x = area.origin.x - px(self.settings.name_width);
        if pos.y >= area.origin.y {
            if pos.x >= area.origin.x {
                return self
                    .vp
                    .hit(x, y, rows, cols)
                    .map_or(Region::Outside, |(r, c)| Region::Cell(r, c));
            }
            if pos.x >= name_x {
                return self
                    .vp
                    .hit(0.0, y, rows, cols.max(1))
                    .map_or(Region::Outside, |(r, _)| Region::Name(r));
            }
            if pos.x >= area.origin.x - px(self.left_w()) {
                return Region::Tree;
            }
        } else if pos.y >= area.origin.y - px(self.header_h()) && pos.x >= area.origin.x {
            return self
                .vp
                .hit(x, 0.0, rows.max(1), cols)
                .map_or(Region::Outside, |(_, c)| Region::Header(c));
        }
        Region::Outside
    }

    /// Alignment rows behind view rows `range`, as a set.
    pub(super) fn rows_set(&self, range: RangeInclusive<usize>) -> RangeSet {
        match self.row_view() {
            None => {
                let mut s = RangeSet::new();
                s.add(*range.start() as u32, *range.end() as u32 + 1);
                s
            }
            Some(rv) => {
                let mut v: Vec<u32> = range.flat_map(|r| rv.rows_of(r).iter().copied()).collect();
                v.sort_unstable();
                RangeSet::from_sorted(v)
            }
        }
    }

    /// Alignment columns from view column `a` to `b` (either order), hidden ones between them
    /// included, so "include hidden columns" can export them.
    pub(super) fn cols_set(&self, a: usize, b: usize) -> RangeSet {
        let (a, b) = (a.min(b), a.max(b));
        let mut s = RangeSet::new();
        s.add(
            view::aln_col(self.col_map(), a) as u32,
            view::aln_col(self.col_map(), b) as u32 + 1,
        );
        s
    }

    fn set_block(&mut self, a: (usize, usize), b: (usize, usize)) {
        let rows = self.rows_set(a.0.min(b.0)..=a.0.max(b.0));
        let cols = self.cols_set(a.1, b.1);
        self.sel = Rc::new(Selection { rows, cols });
    }

    /// Whether view row `r` is in the selected rows (a clade by its first member).
    pub(super) fn view_row_in_rows(&self, sel: &Selection, r: usize) -> bool {
        sel.rows.contains(self.aln_row(r) as u32)
    }

    pub(super) fn select_all(&mut self) {
        let Some(d) = &self.data else { return };
        let mut sel = Selection::default();
        sel.rows.add(0, d.aln.rows() as u32);
        sel.cols.add(0, d.aln.cols() as u32);
        self.sel = Rc::new(sel);
    }

    /// Escape: clear the selection, or the cursor when nothing is selected.
    pub(super) fn clear_selection(&mut self) {
        if self.sel.is_empty() {
            self.cursor = None;
            self.card_cursor = false;
        }
        self.sel = Rc::default();
        self.anchor = None;
    }

    /// Scroll as little as needed so view cell (r, c) is in view.
    pub(super) fn ensure_visible(&mut self, r: usize, c: usize) {
        let (vw, vh) = (self.vp.width / self.vp.cw, self.vp.height / self.vp.ch);
        let (c, r) = (c as f64, r as f64);
        if c < self.vp.col0 {
            self.vp.col0 = c;
        } else if c + 1.0 > self.vp.col0 + vw {
            self.vp.col0 = c + 1.0 - vw;
        }
        if r < self.vp.row0 {
            self.vp.row0 = r;
        } else if r + 1.0 > self.vp.row0 + vh {
            self.vp.row0 = r + 1.0 - vh;
        }
        let (rows, cols) = self.dims();
        self.vp.clamp(rows, cols);
    }

    /// Move the cursor to `to`; with `extend`, select the block from the anchor to it.
    fn move_cursor(&mut self, to: (usize, usize), extend: bool) {
        let from = self.cursor.unwrap_or(to);
        if extend {
            let anchor = *self.anchor.get_or_insert(from);
            self.set_block(anchor, to);
        } else {
            self.anchor = Some(to);
        }
        self.cursor = Some(to);
        self.card_cursor = true;
        self.ensure_visible(to.0, to.1);
    }

    /// The cell the cursor starts from when there is none: the top-left one in view.
    fn cursor_or_view(&self) -> (usize, usize) {
        let (rows, cols) = self.dims();
        self.cursor.unwrap_or_else(|| {
            let (r, c) = self.vp.visible(rows, cols);
            (
                r.start.min(rows.saturating_sub(1)),
                c.start.min(cols.saturating_sub(1)),
            )
        })
    }

    pub(super) fn on_key(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let k = &ev.keystroke;
        let (w, h) = (self.vp.width, self.vp.height);
        let ctrl = k.modifiers.control || k.modifiers.platform;
        let (shift, alt) = (k.modifiers.shift, k.modifiers.alt);
        let (rows, cols) = self.dims();
        let has_data = self.data.is_some() && rows > 0 && cols > 0;
        let page = ((h / self.vp.ch).floor() as isize - 1).max(1);
        let cur = self.cursor_or_view();
        // Cursor moves: (rows, columns) by a step, or to a target.
        let target: Option<(usize, usize)> = match k.key.as_str() {
            _ if !has_data || alt => None,
            "left" if ctrl => Some((
                cur.0,
                select::next_residue(
                    &self.data.as_ref().unwrap().aln,
                    self.row_view(),
                    self.col_map(),
                    cur.0,
                    cur.1,
                    -1,
                    cols,
                ),
            )),
            "right" if ctrl => Some((
                cur.0,
                select::next_residue(
                    &self.data.as_ref().unwrap().aln,
                    self.row_view(),
                    self.col_map(),
                    cur.0,
                    cur.1,
                    1,
                    cols,
                ),
            )),
            "up" if ctrl => Some((0, cur.1)),
            "down" if ctrl => Some((rows - 1, cur.1)),
            "left" => Some(select::step(cur, (0, -1), rows, cols)),
            "right" => Some(select::step(cur, (0, 1), rows, cols)),
            "up" => Some(select::step(cur, (-1, 0), rows, cols)),
            "down" => Some(select::step(cur, (1, 0), rows, cols)),
            "pageup" => Some(select::step(cur, (-page, 0), rows, cols)),
            "pagedown" => Some(select::step(cur, (page, 0), rows, cols)),
            "home" if ctrl => Some((0, 0)),
            "end" if ctrl => Some((rows - 1, cols - 1)),
            "home" => Some((cur.0, 0)),
            "end" => Some((cur.0, cols - 1)),
            _ => None,
        };
        if let Some(to) = target {
            // The first arrow press without a cursor places it on the top-left cell in view.
            let to = if self.cursor.is_none()
                && !shift
                && matches!(k.key.as_str(), "left" | "right" | "up" | "down")
                && !ctrl
            {
                cur
            } else {
                to
            };
            self.move_cursor(to, shift);
            cx.stop_propagation();
            cx.notify();
            return;
        }
        match k.key.as_str() {
            "left" if alt => self.pan(-w / 8.0, 0.0),
            "right" if alt => self.pan(w / 8.0, 0.0),
            "up" if alt => self.pan(0.0, -h / 8.0),
            "down" if alt => self.pan(0.0, h / 8.0),
            "escape" => self.clear_selection(),
            // Ctrl+A, Ctrl+C, Ctrl+E and Ctrl+O are key bindings (gui::menu), shown in the menu bar.
            "c" if ctrl && shift => self.copy(false, window, cx),
            "+" | "=" => self.zoom_by(1, 1, None),
            "-" => self.zoom_by(-1, -1, None),
            "0" => self.request_fit(Fit::All),
            "w" if !ctrl => self.request_fit(Fit::Width),
            "h" if !ctrl => self.request_fit(Fit::Height),
            "r" if !ctrl => self.request_fit(Fit::Readable),
            "d" if !ctrl => self.toggle_differences(),
            "m" if !ctrl => self.toggle_reference(cx),
            "c" if !ctrl => self.mode.by_conservation = !self.mode.by_conservation,
            "s" if !ctrl => self.cycle_scheme(),
            "i" if !ctrl => self.set_hide_inserts(!self.hide_inserts, cx),
            "g" if !ctrl => {
                let p = (!self.hide_gappy).then_some(self.settings.gappy_pct);
                self.set_gappy(p, cx)
            }
            "t" if !ctrl && self.tree.is_some() => self.set_tree_order(!self.tree_order, cx),
            "f" if ctrl && shift => self.set_find_mode(super::app::FindMode::Motif, window, cx),
            "f" if ctrl => self.search.update(cx, |s, cx| s.focus(window, cx)),
            "l" if ctrl => self.toggle_filter_bar(window, cx),
            "f3" if self.motif.is_some() => self.next_hit(if shift { -1 } else { 1 }, cx),
            "g" if ctrl => self.jump.update(cx, |s, cx| s.focus(window, cx)),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Differences from the consensus, or back to the scheme before it.
    pub(super) fn toggle_differences(&mut self) {
        if self.mode.scheme == msascope::scheme::Scheme::Differences {
            self.mode.scheme = self.prev_scheme;
        } else {
            self.mode.scheme = msascope::scheme::Scheme::Differences;
        }
    }

    pub(super) fn on_scroll(&mut self, ev: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(40.));
        let (dx, dy) = (f64::from(d.x), f64::from(d.y));
        let area = self.area.get();
        let (x, y) = (
            f64::from(ev.position.x - area.origin.x),
            f64::from(ev.position.y - area.origin.y),
        );
        if ev.modifiers.control || ev.modifiers.platform {
            if dy.abs() < 0.5 {
                return;
            }
            let step = if dy > 0.0 { 1 } else { -1 };
            // Ctrl+Shift zooms only horizontally, Ctrl+Alt only vertically.
            let (sx, sy) = match (ev.modifiers.shift, ev.modifiers.alt) {
                (true, _) => (step, 0),
                (_, true) => (0, step),
                _ => (step, step),
            };
            self.zoom_by(sx, sy, Some((x.max(0.0), y.max(0.0))));
        } else if ev.modifiers.shift && dx.abs() < 0.5 {
            self.pan(-dy, 0.0);
        } else {
            self.pan(-dx, -dy);
        }
        self.card_hidden = true;
        cx.notify();
    }

    /// The cell, name row or track column under the pointer.
    pub(super) fn update_hover(&mut self, pos: Point<Pixels>) {
        let r = self.region(pos);
        self.hover = match r {
            Region::Cell(r, c) => Some((r, c)),
            _ => None,
        };
        self.hover_name = match r {
            Region::Name(r) => Some(r),
            _ => None,
        };
        self.hover_track = match r {
            Region::Header(c) => Some(c),
            _ => None,
        };
        self.hover_band = match r {
            Region::Header(_) => {
                let top = self.area.get().origin.y - px(self.header_h());
                self.header_row_at(f32::from(pos.y - top))
            }
            _ => None,
        };
        self.pointer = Some(pos);
    }

    /// Whether the hover card is shown: over a cell or the track header, not while dragging or
    /// just after a scroll; or at the cursor after a key moved it.
    pub(super) fn card_visible(&self) -> bool {
        if self.data.is_none() || self.drag.is_some() {
            return false;
        }
        if self.card_cursor && self.cursor.is_some() {
            return true;
        }
        !self.card_hidden
            && self.pointer.is_some()
            && (self.hover.is_some() || self.hover_track.is_some())
    }

    pub(super) fn on_mouse_down(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        self.update_hover(ev.position);
        self.card_cursor = false;
        let ctrl = ev.modifiers.control || ev.modifiers.platform;
        let shift = ev.modifiers.shift;
        match self.region(ev.position) {
            Region::Tree => {
                let hit = self
                    .tree_hits
                    .borrow()
                    .iter()
                    .filter(|h| h.bounds.contains(&ev.position))
                    .max_by(|a, b| a.bounds.origin.x.partial_cmp(&b.bounds.origin.x).unwrap())
                    .map(|h| h.node);
                if let Some(n) = hit {
                    self.toggle_clade(n, cx);
                }
            }
            Region::Name(r) => {
                let anchor = match self.anchor {
                    Some((a, _)) if shift => a,
                    _ => r,
                };
                let mut base = if ctrl {
                    self.sel.rows.clone()
                } else {
                    RangeSet::new()
                };
                if ctrl && self.view_row_in_rows(&self.sel, r) {
                    // Ctrl+click on a selected row takes it out.
                    let take = self.rows_set(r..=r);
                    for &(a, b) in take.ranges() {
                        base.remove(a, b);
                    }
                    self.sel = Rc::new(Selection {
                        rows: base.clone(),
                        cols: if ctrl {
                            self.sel.cols.clone()
                        } else {
                            RangeSet::new()
                        },
                    });
                    self.drag = Some(Drag::Rows { anchor: r, base });
                    self.anchor = Some((r, self.cursor.map_or(0, |c| c.1)));
                    cx.notify();
                    return;
                }
                let mut rows = base.clone();
                rows.union(&self.rows_set(anchor.min(r)..=anchor.max(r)));
                let cols = if ctrl {
                    self.sel.cols.clone()
                } else {
                    RangeSet::new()
                };
                self.sel = Rc::new(Selection { rows, cols });
                self.anchor = Some((anchor, self.cursor.map_or(0, |c| c.1)));
                self.drag = Some(Drag::Rows { anchor, base });
            }
            Region::Header(c) => {
                let anchor = match self.anchor {
                    Some((_, a)) if shift => a,
                    _ => c,
                };
                let base = if ctrl {
                    self.sel.cols.clone()
                } else {
                    RangeSet::new()
                };
                let mut cols = base.clone();
                if ctrl
                    && self
                        .sel
                        .cols
                        .contains(view::aln_col(self.col_map(), c) as u32)
                {
                    let a = view::aln_col(self.col_map(), c) as u32;
                    cols.remove(a, a + 1);
                } else {
                    cols.union(&self.cols_set(anchor, c));
                }
                let rows = if ctrl {
                    self.sel.rows.clone()
                } else {
                    RangeSet::new()
                };
                self.sel = Rc::new(Selection { rows, cols });
                self.anchor = Some((self.cursor.map_or(0, |c| c.0), anchor));
                self.drag = Some(Drag::Cols { anchor, base });
            }
            Region::Cell(r, c) if shift => {
                let anchor = self.anchor.or(self.cursor).unwrap_or((r, c));
                self.anchor = Some(anchor);
                self.set_block(anchor, (r, c));
                self.cursor = Some((r, c));
                self.drag = Some(Drag::Block { anchor });
            }
            Region::Cell(r, c) => {
                self.drag = Some(Drag::Pan {
                    last: ev.position,
                    cell: Some((r, c)),
                    moved: false,
                })
            }
            Region::Outside => {
                self.drag = Some(Drag::Pan {
                    last: ev.position,
                    cell: None,
                    moved: false,
                })
            }
        }
        cx.notify();
    }

    pub(super) fn on_mouse_move(
        &mut self,
        ev: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(mut drag) = self.drag.take().filter(|_| ev.dragging()) {
            match &mut drag {
                Drag::Pan { last, moved, .. } => {
                    let d = ev.position - *last;
                    if d.x.abs() + d.y.abs() > px(0.) {
                        *moved = true;
                    }
                    *last = ev.position;
                    self.pan(-f64::from(d.x), -f64::from(d.y));
                }
                Drag::Block { anchor } => {
                    let (rows, cols) = self.dims();
                    if let Some(to) = self.clamped_cell(ev.position, rows, cols) {
                        self.set_block(*anchor, to);
                        self.cursor = Some(to);
                    }
                }
                Drag::Rows { anchor, base } => {
                    let (rows, cols) = self.dims();
                    if let Some((r, _)) = self.clamped_cell(ev.position, rows, cols.max(1)) {
                        let mut s = base.clone();
                        s.union(&self.rows_set((*anchor).min(r)..=(*anchor).max(r)));
                        let cols = self.sel.cols.clone();
                        self.sel = Rc::new(Selection { rows: s, cols });
                    }
                }
                Drag::Cols { anchor, base } => {
                    let (rows, cols) = self.dims();
                    if let Some((_, c)) = self.clamped_cell(ev.position, rows.max(1), cols) {
                        let mut s = base.clone();
                        s.union(&self.cols_set(*anchor, c));
                        let rows = self.sel.rows.clone();
                        self.sel = Rc::new(Selection { rows, cols: s });
                    }
                }
                Drag::Splitter { x0, w0 } => {
                    let w = (*w0 + f32::from(ev.position.x - *x0)).clamp(NAME_W_MIN, NAME_W_MAX);
                    self.settings.name_width = w.round();
                }
                Drag::TreeSplitter { x0, w0 } => {
                    let w = (*w0 + f32::from(ev.position.x - *x0)).clamp(TREE_W_MIN, TREE_W_MAX);
                    self.settings.tree_width = w.round();
                }
                Drag::Overview => self.overview_pan(ev.position),
            }
            self.drag = Some(drag);
        }
        if !ev.dragging() {
            self.card_hidden = false;
            self.card_cursor = false;
        }
        self.update_hover(ev.position);
        cx.notify();
    }

    /// The cell under `pos`, clamped to the alignment when the pointer is past its edges.
    fn clamped_cell(&self, pos: Point<Pixels>, rows: usize, cols: usize) -> Option<(usize, usize)> {
        if rows == 0 || cols == 0 {
            return None;
        }
        let area = self.area.get();
        let (x, y) = (
            f64::from(pos.x - area.origin.x),
            f64::from(pos.y - area.origin.y),
        );
        let c = (self.vp.col0 + x.clamp(0.0, self.vp.width - 1.0) / self.vp.cw)
            .floor()
            .clamp(0.0, cols as f64 - 1.0) as usize;
        let r = (self.vp.row0 + y.clamp(0.0, self.vp.height - 1.0) / self.vp.ch)
            .floor()
            .clamp(0.0, rows as f64 - 1.0) as usize;
        Some((r, c))
    }

    /// Centre the view on the point of the overview under `pos`.
    pub(super) fn overview_pan(&mut self, pos: Point<Pixels>) {
        let b = self.overview_area.get();
        let (rows, cols) = self.dims();
        if b.size.width <= px(0.) || b.size.height <= px(0.) {
            return;
        }
        let fx = (f64::from(pos.x - b.origin.x) / f64::from(b.size.width)).clamp(0.0, 1.0);
        let fy = (f64::from(pos.y - b.origin.y) / f64::from(b.size.height)).clamp(0.0, 1.0);
        self.vp.col0 = fx * cols as f64 - self.vp.width / self.vp.cw / 2.0;
        self.vp.row0 = fy * rows as f64 - self.vp.height / self.vp.ch / 2.0;
        self.vp.clamp(rows, cols);
    }

    pub(super) fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        match self.drag.take() {
            Some(Drag::Splitter { .. } | Drag::TreeSplitter { .. }) => self.settings.save(),
            // A click on a cell: the cursor goes there, and the selection is cleared.
            Some(Drag::Pan {
                cell: Some(cell),
                moved: false,
                ..
            }) => {
                self.cursor = Some(cell);
                self.anchor = Some(cell);
                self.sel = Rc::default();
            }
            _ => {}
        }
        cx.notify();
    }

    /// What to copy or export: the selection, or everything when `all`.
    fn extract(&self, all: bool, include_hidden: bool) -> Option<Extract> {
        let d = self.data.as_ref()?;
        let sel = (!all).then_some(&*self.sel);
        Some(Extract::new(
            &d.aln,
            sel,
            self.row_view(),
            self.col_map(),
            include_hidden,
        ))
    }

    /// Ctrl+C: the selection as FASTA (or as plain text with Shift) to the clipboard; above
    /// [`select::CLIPBOARD_MAX`], to a file instead.
    pub(super) fn copy(&mut self, fasta: bool, _: &mut Window, cx: &mut Context<Self>) {
        if self.sel.is_empty() {
            self.status =
                "Nothing selected: click names, drag on the ruler or Shift+drag over cells".into();
            return;
        }
        let Some(d) = self.data.clone() else { return };
        let opts = ExportOptions {
            fasta,
            ..self.export
        };
        let Some(e) = self.extract(false, opts.include_hidden) else {
            return;
        };
        if e.cols.is_empty() || e.rows.is_empty() {
            self.status =
                "The selection has no shown columns (include hidden columns from the Export menu)"
                    .into();
            return;
        }
        let size = e.size(&d.aln, fasta);
        if size > select::CLIPBOARD_MAX {
            self.status = format!(
                "The selection is {} MiB, too much for the clipboard; choose a file",
                size >> 20
            )
            .into();
            self.prompt_export(true, cx);
            return;
        }
        let (n, m) = (e.rows.len(), e.cols.len());
        let task = cx.background_executor().spawn(async move {
            let mut out = Vec::with_capacity(size);
            e.write(&d.aln, opts, &mut out)
                .map(|_| String::from_utf8_lossy(&out).into_owned())
        });
        cx.spawn(async move |this, cx| {
            let text = task.await;
            this.update(cx, |v, cx| {
                match text {
                    Ok(t) => {
                        cx.write_to_clipboard(ClipboardItem::new_string(t));
                        let what = if fasta { "FASTA" } else { "text" };
                        v.status = format!(
                            "Copied {} sequences × {} columns as {what}",
                            view::group_digits(n),
                            view::group_digits(m)
                        )
                        .into();
                    }
                    Err(e) => v.status = format!("Couldn't copy: {e}").into(),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Export ▸ FASTA: the selection (`selection`) or every row, × the shown columns (or all with
    /// "include hidden columns"), to a file chosen in the save dialog, written in the background.
    pub(super) fn prompt_export(&mut self, selection: bool, cx: &mut Context<Self>) {
        let Some(d) = self.data.clone() else { return };
        if selection && self.sel.is_empty() {
            self.status = "Nothing selected to export".into();
            cx.notify();
            return;
        }
        let Some(e) = self.extract(!selection, self.export.include_hidden) else {
            return;
        };
        let opts = ExportOptions {
            fasta: true,
            ..self.export
        };
        let dir = self
            .path
            .as_deref()
            .and_then(|p| p.parent())
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let stem = self
            .path
            .as_deref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map_or("alignment", |n| n.split('.').next().unwrap_or(n))
            .to_string();
        let name = format!("{stem}{}.fa", if selection { ".selection" } else { "" });
        let rx = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = rx.await else { return };
            let (n, m) = (e.rows.len(), e.cols.len());
            let p2 = path.clone();
            let write = cx.background_executor().spawn(async move {
                let f = std::fs::File::create(&p2)?;
                let mut w = std::io::BufWriter::with_capacity(1 << 20, f);
                e.write(&d.aln, opts, &mut w)?;
                std::io::Write::flush(&mut w)
            });
            this.update(cx, |v, cx| {
                v.status = format!("Writing {}…", path.display()).into();
                cx.notify();
            })
            .ok();
            let result = write.await;
            this.update(cx, |v, cx| {
                v.status = match result {
                    Ok(()) => format!(
                        "Wrote {} sequences × {} columns to {}",
                        view::group_digits(n),
                        view::group_digits(m),
                        path.display()
                    )
                    .into(),
                    Err(e) => format!("Couldn't write {}: {e}", path.display()).into(),
                };
                cx.notify();
            })
            .ok();
        })
        .detach();
    }
}
