// Adapted from Kantor (github.com/dzulfikar08/kantor) by the same author, relicensed
// GPL-3.0-or-later for irit.
//! Small stateless pieces styled from the tokens.
#![allow(dead_code)]

use gpui_kit::component::Sizable as _;
use gpui_kit::component::button::{
    Button, ButtonCustomVariant, ButtonRounded, ButtonVariants as _,
};
use gpui_kit::*;

use super::metrics::*;
use super::tokens::tokens;

/// A keyboard hint like `Ctrl K`.
pub fn kbd(keys: impl Into<SharedString>, cx: &App) -> Div {
    let t = tokens(cx);
    div()
        .flex_none()
        .px(SPACE_2)
        .py(px(1.))
        .rounded(RADIUS_SM)
        .border_1()
        .border_color(t.border)
        .bg(t.surface)
        .text_size(FONT_CAPTION)
        .text_color(t.fg_muted)
        .child(super::keys::label(&keys.into()))
}

/// An uppercase caption used to label sections.
pub fn section_label(text: impl Into<SharedString>, cx: &App) -> Div {
    let t = tokens(cx);
    div()
        .text_size(FONT_CAPTION)
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(t.fg_dim)
        .child(text.into().to_uppercase())
}

/// A small round status dot.
pub fn dot(color: Hsla) -> Div {
    div().size(px(7.)).flex_none().rounded_full().bg(color)
}

/// A square icon button (22 px); `active` shows the selected state.
///
/// Built on GPUI Kit's `Button` so its tooltip goes through the window's tooltip overlay; give it
/// one with [`IconButtonExt::tip`].
pub fn icon_button(
    id: impl Into<ElementId>,
    icon: impl Into<gpui_kit::component::Icon>,
    active: bool,
    cx: &App,
) -> Button {
    let t = tokens(cx);
    let colors = ButtonCustomVariant::new(cx)
        .color(if active {
            t.selected
        } else {
            Hsla::transparent_black()
        })
        .foreground(if active { t.fg } else { t.fg_muted })
        .hover(if active { t.selected } else { t.hover })
        .active(t.selected);
    Button::new(id)
        .custom(colors)
        .tab_stop(false)
        .rounded(ButtonRounded::Size(RADIUS_MD))
        .flex_none()
        .size(CONTROL_SM)
        .p_0()
        .cursor_pointer()
        .child(gpui_kit::component::Icon::new(icon).with_size(ICON_MD))
}

/// Tooltip text for a control: the action, then its shortcut if it has one (`Fit · 0`).
pub fn tip_text(label: &str, shortcut: Option<&str>) -> String {
    match shortcut {
        Some(keys) if !keys.is_empty() => format!("{label} · {}", super::keys::label(keys)),
        _ => label.to_owned(),
    }
}

/// Tooltips for [`icon_button`]s and other GPUI Kit buttons.
pub trait IconButtonExt {
    /// Show `label` and, if given, its keyboard `shortcut` on hover.
    fn tip(self, label: &str, shortcut: Option<&str>) -> Self;
}

impl IconButtonExt for Button {
    fn tip(self, label: &str, shortcut: Option<&str>) -> Self {
        self.tooltip(tip_text(label, shortcut))
    }
}

#[cfg(test)]
mod tests {
    use super::tip_text;

    #[std::prelude::v1::test]
    fn tip_text_adds_the_shortcut_when_there_is_one() {
        assert_eq!(tip_text("Bold", Some("Ctrl+B")), "Bold · Ctrl+B");
        assert_eq!(tip_text("Insert table", None), "Insert table");
        assert_eq!(tip_text("Strikethrough", Some("")), "Strikethrough");
    }
}
