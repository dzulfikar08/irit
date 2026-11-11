//! The tree panel, painted in the alignment's canvas so its leaves line up with the rows.
//!
//! Only what is in view is visited: a walk from the root that skips subtrees outside the visible
//! rows, and draws a subtree covering less than [`MIN_SPAN`] pixels as one wedge instead of
//! going into it. Below 2 px per row, a branch line already covered in its pixel row is skipped. Branch lines are 1 px in `fg_muted`; a collapsed clade is a triangle in
//! `accent_soft` with an `accent` outline.

use gpui_kit::*;
use msascope::tree::{NONE, TreeIndex, TreeRows};
use msascope::view::RowView;

use super::app::TreeHit;
use super::metrics::*;
use super::tokens::Tokens;

/// Subtrees shorter than this (px) are drawn as a wedge.
const MIN_SPAN: f32 = 6.0;

pub(super) struct TreeGeom<'a> {
    pub ix: &'a TreeIndex,
    pub tr: &'a TreeRows,
    pub rows: &'a RowView,
    /// Panel bounds (rows area only), window coordinates.
    pub bounds: Bounds<Pixels>,
    /// View rows: first row shown (fractional) and logical px per row.
    pub row0: f64,
    pub ch: f64,
}

impl TreeGeom<'_> {
    fn x(&self, n: u32) -> Pixels {
        let w = f32::from(self.bounds.size.width) - 2.0 * TREE_PAD;
        let d = self.ix.depth.max(f32::MIN_POSITIVE);
        self.bounds.origin.x + px(TREE_PAD + w.max(1.0) * (self.ix.x[n as usize] / d))
    }

    fn x_max(&self, n: u32) -> Pixels {
        let w = f32::from(self.bounds.size.width) - 2.0 * TREE_PAD;
        let d = self.ix.depth.max(f32::MIN_POSITIVE);
        self.bounds.origin.x + px(TREE_PAD + w.max(1.0) * (self.ix.x_max[n as usize].max(0.0) / d))
    }

    /// Window y of a position in view rows.
    fn y(&self, rows: f64) -> Pixels {
        self.bounds.origin.y + px(((rows - self.row0) * self.ch) as f32)
    }
}

/// Paint the tree and record where its internal nodes can be clicked.
pub(super) fn paint_tree(g: &TreeGeom, t: &Tokens, hits: &mut Vec<TreeHit>, window: &mut Window) {
    hits.clear();
    let tree = &g.ix.tree;
    let (top, bottom) = (g.row0, g.row0 + f64::from(g.bounds.size.height) / g.ch);
    let line = t.fg_muted;
    // Per pixel row, the span of branch lines drawn in it, when rows are dense enough to share
    // pixels; a line inside it is skipped.
    let h = f32::from(g.bounds.size.height).ceil().max(0.0) as usize + 1;
    let dense = g.ch < 2.0;
    let mut cover = vec![(f32::INFINITY, f32::NEG_INFINITY); if dense { h } else { 0 }];
    let mut hline = |x0: Pixels, x1: Pixels, y: Pixels, window: &mut Window| {
        if x1 <= x0 {
            return;
        }
        if dense {
            let row = (f32::from(y - g.bounds.origin.y).max(0.0) as usize).min(h - 1);
            let c = &mut cover[row];
            let (a, b) = (f32::from(x0), f32::from(x1));
            if a >= c.0 && b <= c.1 {
                return;
            }
            *c = (c.0.min(a), c.1.max(b));
        }
        window.paint_quad(fill(Bounds::new(point(x0, y.floor()), size(x1 - x0, px(1.))), line));
    };
    let mut stack = vec![0u32];
    while let Some(n) = stack.pop() {
        let y = g.tr.y[n as usize];
        if y.is_nan() {
            continue;
        }
        let (a, b) = g.tr.rows[n as usize];
        if (b as f64) <= top || (a as f64) >= bottom {
            continue;
        }
        let (xn, yn) = (g.x(n), g.y(y as f64));
        let parent = tree.nodes[n as usize].parent;
        let xp = if parent == NONE { g.bounds.origin.x + px(TREE_PAD / 2.0) } else { g.x(parent) };
        // A collapsed clade: a triangle from the node to its deepest leaf.
        let collapsed = b == a + 1 && g.rows.clade_of(a as usize).is_some_and(|k| g.tr.collapsed[k] == n);
        if collapsed {
            hline(xp, xn, yn, window);
            let half = px(((g.ch * 0.42) as f32).clamp(1.5, 7.0));
            let x1 = g.x_max(n).max(xn + px(10.));
            let mut fill_path = PathBuilder::fill();
            fill_path.move_to(point(xn, yn));
            fill_path.line_to(point(x1, yn - half));
            fill_path.line_to(point(x1, yn + half));
            fill_path.close();
            if let Ok(p) = fill_path.build() {
                window.paint_path(p, t.accent_soft);
            }
            let mut stroke = PathBuilder::stroke(px(1.));
            stroke.move_to(point(xn, yn));
            stroke.line_to(point(x1, yn - half));
            stroke.line_to(point(x1, yn + half));
            stroke.close();
            if let Ok(p) = stroke.build() {
                window.paint_path(p, t.accent);
            }
            window.paint_quad(fill(Bounds::new(point(xn - px(2.5), yn - px(2.5)), size(px(5.), px(5.))), t.accent).corner_radii(px(2.5)));
            hits.push(TreeHit { bounds: Bounds::new(point(xn - px(TREE_HIT), yn - half - px(2.)), size(x1 - xn + px(TREE_HIT), half * 2.0 + px(4.))), node: n });
            continue;
        }
        if tree.is_leaf(n) {
            hline(xp, xn, yn, window);
            continue;
        }
        let span = (b - a) as f64 * g.ch;
        if span < MIN_SPAN as f64 {
            // Too small to show inside: a wedge as tall as the rows it covers.
            hline(xp, xn, yn, window);
            let y0 = g.y(a as f64);
            let wedge = Bounds::new(point(xn, y0), size((g.x_max(n) - xn).max(px(1.)), px((span as f32).max(1.))));
            window.paint_quad(fill(wedge, Hsla { a: line.a * 0.5, ..line }));
            continue;
        }
        hline(xp, xn, yn, window);
        // The vertical line joins the first and last shown child.
        let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
        for c in tree.children(n) {
            let yc = g.tr.y[c as usize];
            if !yc.is_nan() {
                lo = lo.min(yc);
                hi = hi.max(yc);
            }
            stack.push(c);
        }
        if lo.is_finite() {
            let (y0, y1) = (g.y(lo as f64).floor(), g.y(hi as f64).floor());
            window.paint_quad(fill(Bounds::new(point(xn, y0), size(px(1.), (y1 - y0).max(px(1.)) + px(1.))), line));
            hits.push(TreeHit { bounds: Bounds::new(point(xn - px(TREE_HIT), y0 - px(3.)), size(px(2. * TREE_HIT), y1 - y0 + px(6.))), node: n });
        }
    }
}
