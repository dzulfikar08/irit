//! What the Sort menu, the filter bar, Go to (`name:position`), motif search and the tree's
//! context menu do. The work runs in the core (`rows`, `goto`, `motif`, `tree`); this is the
//! glue to the view's state.

use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use gpui_kit::component::input::InputState;
use gpui_kit::*;
use msascope::goto::{self, Goto};
use msascope::motif::{self, Motif};
use msascope::reference::Reference;
use msascope::rows::{NameQuery, Sort, SortKey};
use msascope::select::{RangeSet, Selection};
use msascope::tree::TreeIndex;
use msascope::view;

use super::app::{FindMode, MotifState, MsaScope, RefOpts};

/// Motif hits kept for highlighting and Next/Previous (all are counted).
const MOTIF_LIMIT: usize = 1_000_000;

/// A percentage typed in the filter bar: empty is none; otherwise 0–100.
fn percent(text: &str) -> Result<Option<f32>, String> {
    let t = text.trim().trim_end_matches('%').trim();
    if t.is_empty() {
        return Ok(None);
    }
    match t.parse::<f32>() {
        Ok(p) if (0.0..=100.0).contains(&p) => Ok(Some(p / 100.0)),
        _ => Err(format!("'{t}' is not a percentage from 0 to 100")),
    }
}

impl MsaScope {
    /// Sort rows by `sort`; `None` returns to the tree's order (`tree_order`) or the file's.
    pub(super) fn set_sort(&mut self, sort: Option<Sort>, tree_order: bool, cx: &mut Context<Self>) {
        if sort.is_some_and(|s| s.key == SortKey::Reference) && self.reference.is_none() {
            self.status = "Set a reference row first: put the cursor on it (or select its name), then Sort ▸ Set reference".into();
            cx.notify();
            return;
        }
        self.sort = sort;
        self.scroll_top = true;
        if sort.is_none() {
            self.tree_order = tree_order && self.tree.is_some();
        }
        self.rebuild_layout(cx);
    }

    /// The Sort button's label.
    pub(super) fn sort_label(&self) -> String {
        match self.sort {
            Some(s) => {
                let k = match s.key {
                    SortKey::Name => "name",
                    SortKey::Identity => "identity",
                    SortKey::Reference => "identity to reference",
                    SortKey::Gaps => "gaps",
                    SortKey::Length => "length",
                };
                format!("Sort: {k}{}", if s.reverse { " (reversed)" } else { "" })
            }
            None if self.tree.is_some() && self.tree_order => "Sort: tree".into(),
            None => "Sort: file".into(),
        }
    }

    /// The row Sort ▸ Set reference would take: the cursor's row, else the one selected row.
    pub(super) fn reference_candidate(&self) -> Option<usize> {
        if let Some((r, _)) = self.cursor
            && self.row_view().is_none_or(|m| m.clade_of(r).is_none())
            && r < self.dims().0
        {
            return Some(self.aln_row(r));
        }
        (self.sel.cols.is_empty() && self.sel.rows.len() == 1).then(|| self.sel.rows.first().unwrap() as usize)
    }

    pub(super) fn set_reference(&mut self, r: Option<usize>, cx: &mut Context<Self>) {
        let was = self.reference;
        self.reference = r;
        self.ref_row = match (r, &self.data) {
            (Some(r), Some(d)) => Some(Arc::new(Reference::new(&d.aln, r))),
            _ => None,
        };
        if r.is_none() && self.sort.is_some_and(|s| s.key == SortKey::Reference) {
            self.sort = None;
        }
        self.status = match (r, &self.data) {
            (Some(r), Some(d)) => format!("Reference: {}", d.aln.names[r]).into(),
            (None, _) if was.is_some() => "Reference cleared".into(),
            _ => SharedString::default(),
        };
        // Rows sorted or filtered by identity to it, and the pyramid compared with it.
        if self.sort.is_some_and(|s| s.key == SortKey::Reference) || self.filter.min_identity.is_some() || (self.ref_opts.compare && was != r) {
            self.rebuild_layout(cx);
        }
        cx.notify();
    }

    /// `m`: the cursor's row (or the one selected row) becomes the reference; on the reference
    /// itself, the reference is cleared.
    pub(super) fn toggle_reference(&mut self, cx: &mut Context<Self>) {
        match self.reference_candidate() {
            Some(r) if Some(r) == self.reference => self.set_reference(None, cx),
            Some(r) => self.set_reference(Some(r), cx),
            None if self.reference.is_some() => self.set_reference(None, cx),
            None => {
                self.status = "Put the cursor on a row (or select one name) to make it the reference".into();
                cx.notify();
            }
        }
    }

    /// Change what the reference does; comparing with it rebuilds the pyramid.
    pub(super) fn set_ref_opts(&mut self, f: impl FnOnce(&mut RefOpts), cx: &mut Context<Self>) {
        let before = self.ref_opts;
        f(&mut self.ref_opts);
        if before.compare != self.ref_opts.compare && self.reference.is_some() {
            self.rebuild_layout(cx);
        }
        cx.notify();
    }

    /// Show or hide an annotation track (a `#=GC` tag, or `GR <tag>`).
    pub(super) fn toggle_track(&mut self, key: &str, cx: &mut Context<Self>) {
        match self.hidden_tracks.iter().position(|h| &**h == key) {
            Some(i) => {
                self.hidden_tracks.remove(i);
            }
            None => self.hidden_tracks.push(key.into()),
        }
        cx.notify();
    }

    /// Read the filter bar's three boxes; rebuild the rows when the filter changed.
    pub(super) fn read_filter(&mut self, cx: &mut Context<Self>) {
        let text = |s: &Entity<InputState>, cx: &App| s.read(cx).value().to_string();
        let (name, ident, gaps) = (text(&self.filter_name, cx), text(&self.filter_ident, cx), text(&self.filter_gaps, cx));
        let mut errors = Vec::new();
        let name = NameQuery::parse(&name).unwrap_or_else(|e| {
            errors.push(e);
            None
        });
        let ident = percent(&ident).unwrap_or_else(|e| {
            errors.push(format!("Min identity: {e}"));
            None
        });
        let gaps = percent(&gaps).unwrap_or_else(|e| {
            errors.push(format!("Max gaps: {e}"));
            None
        });
        let name_key = |q: &Option<NameQuery>| match q {
            None => String::new(),
            Some(NameQuery::Substring(s)) => format!("s{s}"),
            Some(NameQuery::Regex(r)) => format!("r{}", r.as_str()),
        };
        let changed = name_key(&name) != name_key(&self.filter.name) || ident != self.filter.min_identity || gaps != self.filter.max_gaps;
        self.status = errors.join("; ").into();
        if changed {
            self.filter.name = name;
            self.filter.min_identity = ident;
            self.filter.max_gaps = gaps;
            self.scroll_top = true;
            self.rebuild_layout(cx);
        }
        cx.notify();
    }

    /// Clear: every filter off (the boxes are emptied at the next frame).
    pub(super) fn clear_filters(&mut self, cx: &mut Context<Self>) {
        self.filter = Default::default();
        self.clade_label = None;
        self.clear_inputs = true;
        self.status = SharedString::default();
        self.rebuild_layout(cx);
        cx.notify();
    }

    pub(super) fn toggle_filter_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter_bar = !self.filter_bar;
        if self.filter_bar {
            self.filter_name.update(cx, |s, cx| s.focus(window, cx));
        } else {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    /// Show only the rows of node `n`'s clade (with the other filters).
    pub(super) fn show_clade(&mut self, n: u32, cx: &mut Context<Self>) {
        let Some(ix) = self.tree.clone() else { return };
        self.filter.clade = Some(Arc::new(ix.rows_sorted(n)));
        self.clade_label = Some(clade_name(&ix, n, self.data.as_ref().map(|d| &d.aln.names[..]).unwrap_or(&[])).into());
        self.filter_bar = true;
        self.scroll_top = true;
        self.rebuild_layout(cx);
    }

    pub(super) fn clear_clade(&mut self, cx: &mut Context<Self>) {
        self.filter.clade = None;
        self.clade_label = None;
        self.rebuild_layout(cx);
    }

    /// Go to (Ctrl G): `column`, `row:column`, `name:position` or a name.
    pub(super) fn go_to(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(d) = self.data.clone() else { return };
        let (rows, shown) = self.dims();
        let cols = d.aln.cols();
        let err = |v: &mut Self, m: String, cx: &mut Context<Self>| {
            v.status = m.into();
            cx.notify();
        };
        let g = match goto::parse(text) {
            Ok(g) => g,
            Err(e) => return err(self, e, cx),
        };
        // (view row, alignment column), and a note for the status bar.
        let (row, col, mut note): (Option<usize>, Option<usize>, Option<String>) = match g {
            Goto::Col(c) => (None, Some(c), None),
            Goto::RowCol(r, c) => {
                if r >= rows {
                    return err(self, format!("Out of range: {} rows shown", view::group_digits(rows)), cx);
                }
                (Some(r), Some(c), None)
            }
            Goto::Name(q) | Goto::NamePos(q, _) if d.aln.rows() == 0 => return err(self, format!("No sequence named '{q}'"), cx),
            Goto::Name(q) => match goto::find_row(&d.aln, &q, None) {
                Ok(r) => {
                    self.select_row(r);
                    if self.view_row(r).is_some() {
                        self.status = d.aln.names[r].clone().into();
                    }
                    cx.notify();
                    return;
                }
                Err(e) => return err(self, e, cx),
            },
            Goto::NamePos(q, pos) => {
                let r = match goto::find_row(&d.aln, &q, Some(pos)) {
                    Ok(r) => r,
                    Err(e) => return err(self, e, cx),
                };
                let c = match goto::column_of(&d.aln, r, pos) {
                    Ok(c) => c,
                    Err(e) => return err(self, e, cx),
                };
                let Some(vr) = self.view_row(r) else {
                    return err(self, format!("{} is hidden by the row filter", d.aln.names[r]), cx);
                };
                let note = self.row_view().and_then(|m| m.clade_of(vr)).map(|_| format!("{} is in a collapsed clade", d.aln.names[r]));
                let note = note.unwrap_or_else(|| format!("{} residue {pos}: column {}", d.aln.names[r], view::group_digits(c + 1)));
                (Some(vr), Some(c), Some(note))
            }
        };
        if col.is_some_and(|c| c >= cols) {
            return err(self, format!("Out of range: {} columns", view::group_digits(cols)), cx);
        }
        let vcol = col.map(|c| match self.col_map().map(|m| m.view_of(c)) {
            None => c,
            Some(Ok(v)) => v,
            Some(Err(v)) => {
                note = Some(format!("Column {} is hidden; showing the next shown column", view::group_digits(c + 1)));
                v.min(shown.saturating_sub(1))
            }
        });
        self.status = note.map(SharedString::from).unwrap_or_default();
        if self.vp.cw < view::LETTERS_CW && vcol.is_some() {
            self.fit = None;
            self.set_scale(view::LETTERS_CW, self.vp.ch.max(view::LETTERS_CH), None);
        }
        self.center_on(row, vcol);
        if let (Some(r), Some(c)) = (row, vcol) {
            self.cursor = Some((r, c));
            self.anchor = Some((r, c));
            self.card_cursor = true;
        }
        cx.notify();
    }

    /// Switch the Find box between names and residues, and search again with its text.
    pub(super) fn set_find_mode(&mut self, mode: FindMode, window: &mut Window, cx: &mut Context<Self>) {
        if self.find_mode != mode {
            self.find_mode = mode;
            let ph = match mode {
                FindMode::Names => "Find sequence",
                FindMode::Motif => "Motif: HGK, C-x(2)-C, /re/",
            };
            self.search.update(cx, |s, cx| s.set_placeholder(ph, window, cx));
            let q = self.search.read(cx).value().to_string();
            match mode {
                FindMode::Names => {
                    self.cancel_motif();
                    self.motif = None;
                    self.run_search(&q, cx);
                }
                FindMode::Motif => {
                    self.matches.clear();
                    self.run_motif(&q, cx);
                }
            }
        }
        self.search.update(cx, |s, cx| s.focus(window, cx));
        cx.notify();
    }

    pub(super) fn cancel_motif(&mut self) {
        self.motif_cancel.store(true, Ordering::Relaxed);
    }

    /// Search residues for the motif in the background; a newer search cancels this one.
    pub(super) fn run_motif(&mut self, q: &str, cx: &mut Context<Self>) {
        self.cancel_motif();
        let Some(d) = self.data.clone() else { return };
        if q.trim().is_empty() {
            self.motif = None;
            self.status = SharedString::default();
            cx.notify();
            return;
        }
        let m = match Motif::compile(q, d.aln.kind()) {
            Ok(m) => m,
            Err(e) => {
                self.motif = None;
                self.status = e.into();
                cx.notify();
                return;
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.motif_cancel = cancel.clone();
        self.status = "Searching residues…".into();
        cx.notify();
        let query = q.to_string();
        let task = cx.background_executor().spawn(async move {
            let t = Instant::now();
            motif::search(&d.aln, &m, MOTIF_LIMIT, &cancel).map(|h| (h, t.elapsed().as_secs_f64(), m.pattern))
        });
        cx.spawn(async move |this, cx| {
            let Some((hits, secs, pattern)) = task.await else { return };
            this.update(cx, |v, cx| {
                eprintln!("msascope: motif {query} ({pattern}): {} matches in {} sequences, {secs:.3} s", hits.total, hits.rows);
                v.motif = Some(MotifState { hits: Arc::new(hits), nav: Vec::new(), nav_gen: u64::MAX, idx: None, secs });
                v.next_hit(1, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Build the order of the hits in shown rows for the current layout, if it changed.
    fn motif_nav(&mut self) {
        let generation = self.layout.as_deref().map_or(0, |l| l.generation);
        let Some(m) = &self.motif else { return };
        if m.nav_gen == generation {
            return;
        }
        let map = self.col_map();
        let mut nav: Vec<(usize, usize, u32)> = m
            .hits
            .hits
            .iter()
            .enumerate()
            .filter_map(|(i, h)| {
                let r = match self.row_view() {
                    None => h.row as usize,
                    Some(rv) => rv.shown(h.row as usize)?,
                };
                let c = map.map_or(h.start as usize, |m| m.view_of(h.start as usize).unwrap_or_else(|e| e));
                Some((r, c, i as u32))
            })
            .collect();
        nav.sort_unstable();
        let m = self.motif.as_mut().unwrap();
        m.nav = nav.into_iter().map(|x| x.2).collect();
        m.nav_gen = generation;
        m.idx = None;
    }

    /// Go to the next (`step` 1) or previous (-1) motif hit in the rows shown.
    pub(super) fn next_hit(&mut self, step: isize, cx: &mut Context<Self>) {
        self.motif_nav();
        let Some(m) = &self.motif else { return };
        let (total, nrows, secs, kept) = (m.hits.total, m.hits.rows, m.secs, m.hits.hits.len());
        let n = m.nav.len();
        let count = format!(
            "{} {} in {} {}",
            view::group_digits(total),
            if total == 1 { "match" } else { "matches" },
            view::group_digits(nrows),
            if nrows == 1 { "sequence" } else { "sequences" }
        );
        if n == 0 {
            let why = if total > 0 { " (none in the rows shown)" } else { "" };
            self.status = format!("{count}{why} · {secs:.2} s").into();
            cx.notify();
            return;
        }
        let i = match m.idx {
            None => 0,
            Some(i) => (i as isize + step).rem_euclid(n as isize) as usize,
        };
        let h = m.hits.hits[m.nav[i] as usize];
        self.motif.as_mut().unwrap().idx = Some(i);
        let Some(r) = self.view_row(h.row as usize) else { return };
        let c = self.col_map().map_or(h.start as usize, |m| m.view_of(h.start as usize).unwrap_or_else(|e| e)).min(self.dims().1.saturating_sub(1));
        if self.vp.cw < view::LETTERS_CW {
            self.fit = None;
            self.set_scale(view::LETTERS_CW, self.vp.ch.max(view::LETTERS_CH), None);
        }
        self.center_on(Some(r), Some(c));
        self.cursor = Some((r, c));
        self.anchor = Some((r, c));
        let d = self.data.clone().unwrap();
        let first = goto::range(&d.aln, h.row as usize).0 + h.residue as usize;
        let capped = if total > kept { format!(" (first {} kept)", view::group_digits(MOTIF_LIMIT)) } else { String::new() };
        self.status = format!("match {} of {} shown · {count}{capped} · {} at {first} · {secs:.2} s", view::group_digits(i + 1), view::group_digits(n), d.aln.names[h.row as usize]).into();
        cx.notify();
    }

    /// Collapse the clades `depth` levels below node `n`.
    pub(super) fn tree_collapse_depth(&mut self, n: u32, depth: u32, cx: &mut Context<Self>) {
        let Some(ix) = self.tree.clone() else { return };
        ix.collapse_to_depth(&mut self.collapsed, n, depth);
        self.rebuild_layout(cx);
    }

    pub(super) fn tree_expand_all(&mut self, n: u32, cx: &mut Context<Self>) {
        let Some(ix) = self.tree.clone() else { return };
        ix.expand_all(&mut self.collapsed, n);
        self.rebuild_layout(cx);
    }

    /// Select the rows of node `n`'s clade.
    pub(super) fn select_clade(&mut self, n: u32, cx: &mut Context<Self>) {
        let Some(ix) = self.tree.clone() else { return };
        let rows = RangeSet::from_sorted(ix.rows_sorted(n));
        self.status = format!("Selected {} sequences of the clade", view::group_digits(rows.len())).into();
        self.sel = Rc::new(Selection { rows, ..Default::default() });
        cx.notify();
    }

    /// Replace the tree with one made from it in the background (ladderized, rerooted).
    fn replace_tree(&mut self, what: &'static str, f: impl FnOnce(&TreeIndex, &msascope::Alignment) -> (TreeIndex, bool) + Send + 'static, cx: &mut Context<Self>) {
        let (Some(ix), Some(d)) = (self.tree.clone(), self.data.clone()) else { return };
        self.tree_loading = true;
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            let t = Instant::now();
            let (new, keep_collapsed) = f(&ix, &d.aln);
            (new, keep_collapsed, t.elapsed().as_secs_f64())
        });
        cx.spawn(async move |this, cx| {
            let (new, keep_collapsed, secs) = task.await;
            this.update(cx, |v, cx| {
                v.tree_loading = false;
                eprintln!("msascope: {what} {secs:.3} s");
                if !keep_collapsed {
                    v.collapsed = vec![false; new.tree.len()];
                    v.filter.clade = None;
                    v.clade_label = None;
                }
                v.tree = Some(Arc::new(new));
                v.status = format!("Tree {what}").into();
                v.rebuild_layout(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn ladderize(&mut self, n: u32, cx: &mut Context<Self>) {
        self.replace_tree("ladderized", move |ix, _| (ix.ladderized(n), true), cx);
    }

    /// Reroot on the branch above node `n`; node numbers change, so collapsed clades and a clade
    /// filter are reset.
    pub(super) fn reroot(&mut self, n: u32, cx: &mut Context<Self>) {
        self.replace_tree("rerooted", move |ix, aln| (TreeIndex::new(ix.tree.reroot(n), &aln.names, &aln.accessions), false), cx);
    }
}

/// A clade's name for menus and the filter bar: its label when it is a name, else its first
/// sequence's name and the number of sequences.
pub(super) fn clade_name(ix: &TreeIndex, n: u32, names: &[String]) -> String {
    let label = ix.tree.label(n);
    let size = ix.size(n);
    if !label.is_empty() && label.parse::<f64>().is_err() {
        return format!("{label} ({})", view::group_digits(size));
    }
    let first = ix.rows(n).first().and_then(|&r| names.get(r as usize)).map_or("", |s| s.split('/').next().unwrap_or(s));
    format!("{first} … ({} sequences)", view::group_digits(size))
}
