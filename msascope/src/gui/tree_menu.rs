//! The tree panel's context menu: right-click a node (its vertical line, or a collapsed clade's
//! triangle) for collapse and expand, collapse to a depth, select or show only the clade's rows,
//! ladderize and reroot.

use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::*;

use super::actions::clade_name;
use super::app::MsaScope;
use super::metrics::*;

impl MsaScope {
    /// A transparent layer over the tree panel that opens the menu for the node under the pointer.
    pub(super) fn tree_menu_overlay(&self, tree_w: Pixels, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let entity = cx.entity().downgrade();
        let hits = self.tree_hits.clone();
        let pointer = self.last_pointer.clone();
        div().id("tree-menu").absolute().left_0().top(px(HEADER_H)).bottom_0().w(tree_w).context_menu(move |menu: PopupMenu, window, cx| {
            let pos = pointer.get();
            let node = hits
                .borrow()
                .iter()
                .filter(|h| h.bounds.contains(&pos))
                .max_by(|a, b| a.bounds.origin.x.partial_cmp(&b.bounds.origin.x).unwrap())
                .map(|h| h.node);
            let Some(n) = node else { return menu };
            let Some(view) = entity.upgrade() else { return menu };
            let v = view.read(cx);
            let (Some(ix), Some(d)) = (v.tree.clone(), v.data.clone()) else { return menu };
            let collapsed = v.collapsed.get(n as usize).copied().unwrap_or(false);
            let size = ix.size(n);
            let act = |f: fn(&mut MsaScope, u32, &mut Context<MsaScope>)| {
                let e = entity.clone();
                move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                    let _ = e.update(cx, |v, cx| f(v, n, cx));
                }
            };
            let depths = {
                let e = entity.clone();
                move |mut m: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                    for depth in 1..=6u32 {
                        let e = e.clone();
                        m = m.item(PopupMenuItem::new(format!("{depth} {}", if depth == 1 { "level" } else { "levels" })).on_click(move |_, _, cx| {
                            let _ = e.update(cx, |v, cx| v.tree_collapse_depth(n, depth, cx));
                        }));
                    }
                    m
                }
            };
            menu.item(PopupMenuItem::label(clade_name(&ix, n, &d.aln.names)))
                .item(PopupMenuItem::new(if collapsed { "Expand" } else { "Collapse" }).on_click(act(|v, n, cx| v.toggle_clade(n, cx))))
                .submenu("Collapse to depth", window, cx, depths)
                .item(PopupMenuItem::new("Expand all below").on_click(act(|v, n, cx| v.tree_expand_all(n, cx))))
                .item(PopupMenuItem::new("Expand the whole tree").on_click(act(|v, _, cx| v.tree_expand_all(0, cx))))
                .separator()
                .item(PopupMenuItem::new(format!("Select its {} rows", msascope::view::group_digits(size))).on_click(act(|v, n, cx| v.select_clade(n, cx))))
                .item(PopupMenuItem::new("Show only this clade").on_click(act(|v, n, cx| v.show_clade(n, cx))))
                .separator()
                .item(PopupMenuItem::new("Ladderize this clade").on_click(act(|v, n, cx| v.ladderize(n, cx))))
                .item(PopupMenuItem::new("Reroot here").disabled(n == 0).on_click(act(|v, n, cx| v.reroot(n, cx))))
        })
    }
}
