//! The name panel's context menu: right-click a sequence name to make it the reference row (or
//! clear it), and to choose what the reference does.

use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::*;

use super::app::{MsaScope, RefOpts};
use super::input::Region;

impl MsaScope {
    /// A transparent layer over the name panel that opens the menu for the row under the pointer.
    pub(super) fn name_menu_overlay(
        &self,
        left: Pixels,
        width: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let entity = cx.entity().downgrade();
        let pointer = self.last_pointer.clone();
        div()
            .id("name-menu")
            .absolute()
            .left(left)
            .top(px(self.header_h()))
            .bottom_0()
            .w(width)
            .context_menu(move |menu: PopupMenu, _, cx| {
                let Some(view) = entity.upgrade() else {
                    return menu;
                };
                let v = view.read(cx);
                let Some(d) = v.data.clone() else { return menu };
                let Region::Name(vr) = v.region(pointer.get()) else {
                    return menu;
                };
                if v.row_view().is_some_and(|m| m.clade_of(vr).is_some()) {
                    return menu.item(
                        PopupMenuItem::new("A collapsed clade can't be the reference")
                            .disabled(true),
                    );
                }
                let r = v.aln_row(vr);
                let is_ref = v.reference == Some(r);
                let opts = v.ref_opts;
                let has_ref = v.reference.is_some();
                let e = entity.clone();
                let opt = |label: &str, on: bool, f: fn(&mut RefOpts)| {
                    let e = entity.clone();
                    PopupMenuItem::new(label.to_string())
                        .checked(on)
                        .disabled(!has_ref)
                        .on_click(move |_, _, cx| {
                            let _ = e.update(cx, |v, cx| v.set_ref_opts(f, cx));
                        })
                };
                let set = PopupMenuItem::new(if is_ref {
                    "Clear the reference   M"
                } else {
                    "Set as reference   M"
                })
                .on_click(move |_, _, cx| {
                    let _ = e.update(cx, |v, cx| {
                        v.set_reference(if is_ref { None } else { Some(r) }, cx)
                    });
                });
                menu.item(PopupMenuItem::label(d.aln.names[r].clone()))
                    .item(set)
                    .separator()
                    .item(opt("Pin the reference under the tracks", opts.pin, |o| {
                        o.pin = !o.pin
                    }))
                    .item(opt(
                        "Ruler counts the reference's residues",
                        opts.ruler,
                        |o| o.ruler = !o.ruler,
                    ))
                    .item(opt(
                        "Compare with the reference (Differences, identity)",
                        opts.compare,
                        |o| o.compare = !o.compare,
                    ))
            })
    }
}
