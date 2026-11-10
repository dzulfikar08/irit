//! Title bar (app mark, file tab, theme menu), toolbar, status bar, and the empty / loading /
//! error card. The title bar and status bar follow Kantor's layout; the toolbar its format bar.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use msascope::view::{self, ColorMode};

use super::app::{Fit, MsaScope};
use super::components::{IconButtonExt as _, dot, icon_button, kbd};
use super::fonts::tabular;
use super::metrics::*;
use super::theme::ThemeChoice;
use super::tokens::tokens;

/// A thin vertical rule between toolbar groups.
fn separator(cx: &App) -> Div {
    div().flex_none().w(px(1.)).h(ICON_LG).mx(SPACE_2).bg(tokens(cx).border)
}

impl MsaScope {
    fn file_name(&self) -> Option<String> {
        self.path.as_ref().and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned())
    }

    /// msascope's mark: a 3 × 3 patch of residue colours.
    fn app_mark(cx: &App) -> Div {
        let t = tokens(cx);
        let cells: [Hsla; 9] = [t.info, t.success, t.app, t.success, t.warning, t.info, t.danger, t.info, t.success];
        div()
            .flex_none()
            .size(TAB_HEIGHT)
            .flex()
            .items_center()
            .justify_center()
            .rounded(RADIUS_MD)
            .border_1()
            .border_color(t.border_strong)
            .bg(t.bg)
            .child(
                v_flex()
                    .gap(px(1.5))
                    .children((0..3).map(|r| h_flex().gap(px(1.5)).children((0..3).map(move |c| div().size(px(4.)).rounded(px(1.)).bg(cells[r * 3 + c]))))),
            )
    }

    pub(super) fn render_title_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = tokens(cx);
        let compact = window.viewport_size().width < COMPACT_WINDOW;
        let entity = cx.entity().downgrade();

        let tab = self.file_name().map(|name| {
            let color = if self.error.is_some() {
                t.danger
            } else if self.loading.is_some() {
                t.warning
            } else {
                t.app
            };
            let close = entity.clone();
            div()
                .id("file-tab")
                .flex_none()
                .h(TAB_HEIGHT)
                .pl(SPACE_5)
                .pr(SPACE_2)
                .flex()
                .items_center()
                .gap(SPACE_3)
                .rounded(RADIUS_MD)
                .text_size(FONT_SMALL)
                .font_weight(FontWeight::MEDIUM)
                .bg(t.bg)
                .border_1()
                .border_color(t.border_strong)
                .text_color(t.fg)
                .child(dot(color))
                .child(div().w(if compact { px(110.) } else { TAB_TITLE_WIDTH }).truncate().child(name))
                .child(
                    div()
                        .id("tab-close")
                        .size(px(18.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(RADIUS_SM)
                        .text_color(t.fg_dim)
                        .hover(|s| s.bg(t.hover).text_color(t.fg))
                        .child("×")
                        .on_click(move |_, _, cx| {
                            cx.stop_propagation();
                            let _ = close.update(cx, |v, cx| v.close(cx));
                        }),
                )
        });

        let open = {
            let entity = entity.clone();
            icon_button("title-open", IconName::Plus, false, cx).tip("Open…", Some("Ctrl+O")).on_click(move |_, _, cx| {
                let _ = entity.update(cx, |v, cx| v.prompt_open(cx));
            })
        };

        let current = self.settings.theme;
        let theme_menu = {
            let entity = entity.clone();
            Button::new("theme")
                .ghost()
                .xsmall()
                .icon(if t.dark { IconName::Moon } else { IconName::Sun })
                .when(!compact, |b| b.label(current.label()))
                .tooltip("Theme")
                .dropdown_menu(move |mut menu, _, _| {
                    for choice in ThemeChoice::ALL {
                        let entity = entity.clone();
                        menu = menu.item(PopupMenuItem::new(choice.label()).checked(choice == current).on_click(move |_, window, cx| {
                            let _ = entity.update(cx, |v, cx| v.set_theme(choice, window, cx));
                        }));
                    }
                    menu
                })
        };

        h_flex()
            .size_full()
            .px(SPACE_4)
            .gap(SPACE_2)
            .bg(t.chrome)
            .border_b_1()
            .border_color(t.border)
            .child(Self::app_mark(cx))
            .child(h_flex().flex_1().min_w_0().gap(SPACE_2).children(tab).child(open))
            .child(theme_menu)
    }

    pub(super) fn render_toolbar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = tokens(cx);
        let compact = window.viewport_size().width < COMPACT_WINDOW;
        let entity = cx.entity().downgrade();
        let has_data = self.data.is_some();
        let input_w = if compact { INPUT_WIDTH_COMPACT } else { INPUT_WIDTH };

        let open = Button::new("open").ghost().xsmall().icon(IconName::FolderOpen).label("Open").tooltip("Open… · Ctrl+O").on_click(cx.listener(|v, _, _, cx| v.prompt_open(cx)));
        let input = |state: &Entity<InputState>, icon: IconName, keys: &'static str| {
            div().w(input_w).child(Input::new(state).xsmall().prefix(gpui_kit::component::Icon::new(icon).xsmall().text_color(t.fg_dim)).when(!compact, |i| i.suffix(kbd(keys, cx))))
        };
        let zoom = {
            let (out, inn) = (entity.clone(), entity.clone());
            h_flex()
                .gap(SPACE_1)
                .child(icon_button("zoom-out", IconName::Minus, false, cx).tip("Zoom out", Some("-")).on_click(move |_, _, cx| {
                    let _ = out.update(cx, |v, cx| {
                        v.zoom_by(-1, -1, None);
                        cx.notify();
                    });
                }))
                .child(
                    div()
                        .min_w(px(78.))
                        .text_center()
                        .text_size(FONT_SMALL)
                        .text_color(t.fg_muted)
                        .font_features(tabular())
                        .when(has_data, |d| d.child(view::zoom_label(self.vp.cw, self.vp.ch))),
                )
                .child(icon_button("zoom-in", IconName::Plus, false, cx).tip("Zoom in", Some("+")).on_click(move |_, _, cx| {
                    let _ = inn.update(cx, |v, cx| {
                        v.zoom_by(1, 1, None);
                        cx.notify();
                    });
                }))
        };
        let fit_items = |mut menu: gpui_kit::component::menu::PopupMenu, current: Option<Fit>, entity: WeakEntity<MsaScope>| {
            for f in [Fit::All, Fit::Width, Fit::Height, Fit::Readable] {
                let entity = entity.clone();
                menu = menu.item(PopupMenuItem::new(format!("{}   {}", f.label(), f.key())).checked(current == Some(f)).on_click(move |_, _, cx| {
                    let _ = entity.update(cx, |v, cx| {
                        v.request_fit(f);
                        cx.notify();
                    });
                }));
            }
            menu
        };
        let mode_items = |mut menu: gpui_kit::component::menu::PopupMenu, current: ColorMode, entity: WeakEntity<MsaScope>| {
            for (m, label) in [(ColorMode::Residues, "Residues"), (ColorMode::Differences, "Differences from consensus")] {
                let entity = entity.clone();
                menu = menu.item(PopupMenuItem::new(format!("{label}   D")).checked(current == m).on_click(move |_, _, cx| {
                    let _ = entity.update(cx, |v, cx| {
                        if v.mode != m {
                            v.toggle_mode();
                        }
                        cx.notify();
                    });
                }));
            }
            menu
        };
        let (fit, mode) = (self.fit, self.mode);
        let fit_label = fit.filter(|f| *f != Fit::Readable).map_or("Fit", |f| f.label());
        let mode_label = match mode {
            ColorMode::Residues => "Residues",
            ColorMode::Differences => "Differences",
        };

        let mut row = h_flex()
            .size_full()
            .gap(SPACE_1)
            .px(SPACE_4)
            .bg(t.chrome)
            .border_b_1()
            .border_color(t.border)
            .child(open)
            .child(separator(cx))
            .child(input(&self.jump, IconName::ArrowRight, "Ctrl G"))
            .child(div().w(SPACE_2))
            .child(input(&self.search, IconName::Search, "Ctrl F"))
            .child(separator(cx))
            .child(zoom);
        if compact {
            // Fit and colour fold into one menu.
            let e = entity.clone();
            row = row.child(separator(cx)).child(Button::new("more").ghost().xsmall().icon(IconName::Ellipsis).tooltip("Fit and colour").dropdown_menu(move |menu, _, _| {
                let menu = fit_items(menu, fit, e.clone()).separator();
                mode_items(menu, mode, e.clone())
            }));
        } else {
            let (e1, e2) = (entity.clone(), entity.clone());
            row = row
                .child(Button::new("fit").ghost().xsmall().icon(IconName::Maximize).label(fit_label).dropdown_caret(true).tooltip("Fit · 0, W, H").dropdown_menu(move |menu, _, _| fit_items(menu, fit, e1.clone())))
                .child(separator(cx))
                .child(Button::new("colour").ghost().xsmall().icon(IconName::Palette).label(mode_label).dropdown_caret(true).tooltip("Colour · D").dropdown_menu(move |menu, _, _| mode_items(menu, mode, e2.clone())));
        }
        row
    }

    pub(super) fn render_status_bar(&mut self, cx: &mut Context<Self>) -> Div {
        let t = tokens(cx);
        let hover = self.hover_text();
        let message: SharedString = if self.error.is_some() {
            "Couldn't open the file".into()
        } else if let Some(p) = &self.loading {
            format!("Loading {}…", p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())).into()
        } else if !hover.is_empty() {
            hover.into()
        } else {
            self.status.clone()
        };
        let theme = match self.settings.theme {
            ThemeChoice::System => format!("System ({})", if t.dark { "dark" } else { "light" }),
            c => c.label().to_owned(),
        };
        let d = self.data.clone();
        let selection = self.selected_row.zip(d.as_ref()).map(|(r, d)| format!("Selected: row {} ({})", view::group_digits(r + 1), d.aln.names[r]));
        h_flex()
            .flex_none()
            .w_full()
            .h(STATUS_BAR_HEIGHT)
            .px(SPACE_6)
            .gap(SPACE_6)
            .bg(t.chrome)
            .border_t_1()
            .border_color(t.border)
            .text_size(FONT_CAPTION)
            .text_color(t.fg_muted)
            .font_features(tabular())
            .child(h_flex().flex_none().gap(SPACE_3).child(dot(t.app)).child(div().text_color(t.fg).child("msascope")))
            .when_some(d.clone(), |row, d| {
                let format = format!("{}{} · {}", d.format.name(), if d.gzip { " (gzip)" } else { "" }, d.aln.alpha.name);
                row.child(div().flex_none().child(format))
                    .child(div().flex_none().text_color(t.fg).child(format!("{} sequences × {} columns", view::group_digits(d.aln.rows()), view::group_digits(d.aln.cols()))))
            })
            .child(div().flex_1().min_w_0().truncate().when(self.error.is_some(), |d| d.text_color(t.danger)).child(message))
            .when_some(selection, |row, s| row.child(div().flex_none().max_w(px(260.)).truncate().text_color(t.fg).child(s)))
            .when_some(d, |row, d| row.child(div().flex_none().child(format!("loaded in {:.2} s · {} MiB", d.load_secs(), view::group_digits(d.bytes() >> 20)))))
            .child(div().flex_none().child(theme))
            .child(div().flex_none().text_color(t.fg_dim).child(concat!("v", env!("CARGO_PKG_VERSION"))))
    }

    /// The window without an alignment: open a file or a sample, or see why loading failed.
    pub(super) fn render_empty(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = tokens(cx);
        let card = v_flex().w(px(420.)).p(SPACE_9).gap(SPACE_4).rounded(RADIUS_XL).border_1().border_color(t.border).bg(t.chrome);
        let card = if let Some(p) = &self.loading {
            let name = p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
            card.child(div().text_size(FONT_TITLE).font_weight(FontWeight::SEMIBOLD).child(format!("Loading {name}…")))
                .child(div().text_size(FONT_BODY).text_color(t.fg_muted).child("Reading the file, then the column summaries and the overview."))
        } else {
            let (title, detail, detail_color) = match &self.error {
                Some(e) => ("Couldn't open the file", e.clone(), t.danger),
                None => ("Open an alignment", SharedString::from("Aligned FASTA (gzip too), Clustal or Stockholm"), t.fg_muted),
            };
            let retry = if self.error.is_some() { "Open another…" } else { "Open…" };
            card.child(div().text_size(FONT_TITLE).font_weight(FontWeight::SEMIBOLD).child(title))
                .child(div().text_size(FONT_BODY).text_color(detail_color).child(detail))
                .when(!self.status.is_empty(), |c| c.child(div().text_size(FONT_BODY).text_color(t.fg_muted).child(self.status.clone())))
                .child(
                    h_flex()
                        .pt(SPACE_4)
                        .gap(SPACE_4)
                        .child(Button::new("empty-open").primary().small().label(retry).on_click(cx.listener(|v, _, _, cx| v.prompt_open(cx))))
                        .child(kbd("Ctrl O", cx))
                        .child(div().flex_1())
                        .child(Button::new("sample").ghost().small().label("Try a sample").on_click(cx.listener(|v, _, _, cx| v.open_sample(cx)))),
                )
        };
        div().flex_1().min_h_0().flex().items_center().justify_center().bg(t.bg).child(card)
    }
}

use gpui_kit::component::input::InputState;
