//! Title bar (app mark, file tab, theme menu), toolbar, status bar, and the empty / loading /
//! error card. The title bar and status bar follow Kantor's layout; the toolbar its format bar.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{IconName, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use gpui_kit::assets::IconName as Lucide;
use msascope::scheme::{self, Scheme};
use msascope::view;

use super::app::{Fit, MsaScope};
use super::components::{IconButtonExt as _, dot, icon_button, kbd};
use super::fonts::tabular;
use super::metrics::*;
use super::theme::{GAPPY_CHOICES, ThemeChoice};
use msascope::select::ExportOptions;
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

        let open = Button::new("open").ghost().xsmall().icon(IconName::FolderOpen).label("Open").tooltip("Open an alignment or a tree… · Ctrl+O").on_click(cx.listener(|v, _, _, cx| v.prompt_open(cx)));
        let has_sel = !self.sel.is_empty();
        let export_opts = self.export;
        let export = {
            let e = entity.clone();
            Button::new("export").ghost().xsmall().icon(Lucide::ArrowDown).label("Export").dropdown_caret(true).tooltip("Export or copy as FASTA").dropdown_menu(move |menu, _, _| {
                let item = |label: &str, enabled: bool, f: fn(&mut MsaScope, &mut Window, &mut Context<MsaScope>)| {
                    let e = e.clone();
                    PopupMenuItem::new(label.to_string()).disabled(!enabled).on_click(move |_, window, cx| {
                        let _ = e.update(cx, |v, cx| {
                            f(v, window, cx);
                            cx.notify();
                        });
                    })
                };
                let check = |label: &str, on: bool, f: fn(&mut ExportOptions)| {
                    let e = e.clone();
                    PopupMenuItem::new(label.to_string()).checked(on).on_click(move |_, _, cx| {
                        let _ = e.update(cx, |v, cx| {
                            f(&mut v.export);
                            cx.notify();
                        });
                    })
                };
                menu.item(item("Selection as FASTA…   Ctrl+E", has_sel, |v, _, cx| v.prompt_export(true, cx)))
                    .item(item("All rows as FASTA…", has_data, |v, _, cx| v.prompt_export(false, cx)))
                    .separator()
                    .item(item("Copy selection as FASTA   Ctrl+C", has_sel, |v, w, cx| v.copy(true, w, cx)))
                    .item(item("Copy selection as text   Ctrl+Shift+C", has_sel, |v, w, cx| v.copy(false, w, cx)))
                    .separator()
                    .item(check("Include hidden columns", export_opts.include_hidden, |o| o.include_hidden = !o.include_hidden))
                    .item(check("Leave out gaps", export_opts.strip_gaps, |o| o.strip_gaps = !o.strip_gaps))
            })
        };
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
        let kind = self.data.as_ref().map(|d| d.aln.kind());
        let mode = self.mode;
        let scheme_items = move |mut menu: gpui_kit::component::menu::PopupMenu, entity: WeakEntity<MsaScope>| {
            let schemes = Scheme::ALL.into_iter().filter(|s| kind.is_none_or(|k| s.applies(k)));
            for s in schemes {
                let entity = entity.clone();
                let key = if s == Scheme::Differences { "   D" } else { "" };
                menu = menu.item(PopupMenuItem::new(format!("{}{key}", s.label())).checked(mode.scheme == s).on_click(move |_, _, cx| {
                    let _ = entity.update(cx, |v, cx| {
                        v.set_scheme(s);
                        cx.notify();
                    });
                }));
            }
            let e = entity.clone();
            menu.separator().item(PopupMenuItem::new("By conservation   C").checked(mode.by_conservation).on_click(move |_, _, cx| {
                let _ = e.update(cx, |v, cx| {
                    v.mode.by_conservation = !v.mode.by_conservation;
                    cx.notify();
                });
            }))
        };
        let inserts = self.data.as_ref().map_or(0, |d| d.aln.insert_count());
        let (hide_inserts, hide_gappy, gappy_pct) = (self.hide_inserts, self.hide_gappy, self.settings.gappy_pct);
        let column_items = move |mut menu: gpui_kit::component::menu::PopupMenu, entity: WeakEntity<MsaScope>| {
            if inserts > 0 {
                let e = entity.clone();
                menu = menu
                    .item(PopupMenuItem::new(format!("Hide {} insert columns   I", view::group_digits(inserts))).checked(hide_inserts).on_click(move |_, _, cx| {
                        let _ = e.update(cx, |v, cx| v.set_hide_inserts(!v.hide_inserts, cx));
                    }))
                    .separator();
            }
            let e = entity.clone();
            menu = menu.item(PopupMenuItem::new("Show gappy columns").checked(!hide_gappy).on_click(move |_, _, cx| {
                let _ = e.update(cx, |v, cx| v.set_gappy(None, cx));
            }));
            for p in GAPPY_CHOICES {
                let e = entity.clone();
                let label = if p == 100 { "Hide all-gap columns".to_string() } else { format!("Hide columns ≥ {p}% gaps") };
                let key = if p == gappy_pct { "   G" } else { "" };
                menu = menu.item(PopupMenuItem::new(format!("{label}{key}")).checked(hide_gappy && gappy_pct == p).on_click(move |_, _, cx| {
                    let _ = e.update(cx, |v, cx| v.set_gappy(Some(p), cx));
                }));
            }
            menu
        };
        let fit = self.fit;
        let fit_label = fit.filter(|f| *f != Fit::Readable).map_or("Fit", |f| f.label());
        // A strip of the scheme's legend colours, as on the button in the mockup.
        let theme = t.tile_theme();
        let swatches = kind.map(|k| scheme::legend(mode.scheme, k, &theme)).unwrap_or_default();
        let strip = h_flex().gap(px(1.)).children(swatches.into_iter().take(8).map(|(c, _)| div().w(px(4.)).h(px(10.)).rounded(px(1.)).bg(rgb(c))));
        let scheme_label = if mode.by_conservation { format!("{} · conservation", mode.scheme.label()) } else { mode.scheme.label().to_string() };
        let columns_label = match (hide_inserts && inserts > 0, hide_gappy) {
            (true, true) => format!("Inserts, gappy ≥ {gappy_pct}% hidden"),
            (true, false) => "Inserts hidden".to_string(),
            (false, true) if gappy_pct == 100 => "All-gap hidden".to_string(),
            (false, true) => format!("Gappy ≥ {gappy_pct}% hidden"),
            (false, false) => "All columns".to_string(),
        };
        let any_hidden = (hide_inserts && inserts > 0) || hide_gappy;

        let mut row = h_flex()
            .size_full()
            .gap(SPACE_1)
            .px(SPACE_4)
            .bg(t.chrome)
            .border_b_1()
            .border_color(t.border)
            .child(open)
            .child(export)
            .child(separator(cx))
            .child(input(&self.jump, IconName::ArrowRight, "Ctrl G"))
            .child(div().w(SPACE_2))
            .child(input(&self.search, IconName::Search, "Ctrl F"))
            .child(separator(cx))
            .child(zoom);
        if compact {
            // Fit, colour and columns fold into one menu.
            let e = entity.clone();
            row = row.child(separator(cx)).child(Button::new("more").ghost().xsmall().icon(IconName::Ellipsis).tooltip("Fit, colour and columns").dropdown_menu(move |menu, _, _| {
                let menu = fit_items(menu, fit, e.clone()).separator();
                let menu = scheme_items(menu, e.clone()).separator();
                column_items(menu, e.clone())
            }));
        } else {
            let (e1, e2, e3) = (entity.clone(), entity.clone(), entity.clone());
            row = row
                .child(Button::new("fit").ghost().xsmall().icon(IconName::Maximize).label(fit_label).dropdown_caret(true).tooltip("Fit · 0, W, H").dropdown_menu(move |menu, _, _| fit_items(menu, fit, e1.clone())))
                .child(separator(cx))
                .child(
                    Button::new("colour")
                        .outline()
                        .xsmall()
                        .child(h_flex().gap(SPACE_3).child(strip).child(scheme_label))
                        .dropdown_caret(true)
                        .tooltip("Colour scheme · S cycles, D differences, C conservation")
                        .dropdown_menu(move |menu, _, _| scheme_items(menu, e2.clone())),
                )
                .child(div().w(SPACE_2))
                .child(
                    Button::new("columns")
                        .ghost()
                        .xsmall()
                        .selected(any_hidden)
                        .icon(Lucide::EyeOff)
                        .label(columns_label)
                        .dropdown_caret(true)
                        .tooltip("Hide insert and gappy columns · I, G")
                        .dropdown_menu(move |menu, _, _| column_items(menu, e3.clone())),
                );
        }
        row = row.child(div().flex_1());
        if self.tree.is_some() {
            let (e1, e2) = (entity.clone(), entity.clone());
            let tree_order = self.tree_order;
            let panel = self.settings.tree_panel;
            row = row
                .child(
                    Button::new("sort")
                        .ghost()
                        .xsmall()
                        .icon(Lucide::SortAscending)
                        .label(if tree_order { "Sort: tree" } else { "Sort: file" })
                        .dropdown_caret(true)
                        .tooltip("Row order · T")
                        .dropdown_menu(move |menu, _, _| {
                            let (a, b, c, d) = (e1.clone(), e1.clone(), e1.clone(), e1.clone());
                            menu.item(PopupMenuItem::new("Tree order   T").checked(tree_order).on_click(move |_, _, cx| {
                                let _ = a.update(cx, |v, cx| v.set_tree_order(true, cx));
                            }))
                            .item(PopupMenuItem::new("File order").checked(!tree_order).on_click(move |_, _, cx| {
                                let _ = b.update(cx, |v, cx| v.set_tree_order(false, cx));
                            }))
                            .separator()
                            .item(PopupMenuItem::new("Open another tree…").on_click(move |_, _, cx| {
                                let _ = c.update(cx, |v, cx| v.prompt_open(cx));
                            }))
                            .item(PopupMenuItem::new("Remove the tree").on_click(move |_, _, cx| {
                                let _ = d.update(cx, |v, cx| v.remove_tree(cx));
                            }))
                        }),
                )
                .child(icon_button("tree-panel", Lucide::PanelLeft, panel && tree_order, cx).tip("Tree panel", None).on_click(move |_, _, cx| {
                    let _ = e2.update(cx, |v, cx| {
                        v.settings.tree_panel = !v.settings.tree_panel;
                        v.settings.save();
                        cx.notify();
                    });
                }));
        }
        let e = entity.clone();
        let overview_on = self.settings.overview;
        row = row.child(icon_button("overview", Lucide::PanelRight, overview_on, cx).tip("Overview panel", None).on_click(move |_, _, cx| {
            let _ = e.update(cx, |v, cx| {
                v.settings.overview = !v.settings.overview;
                v.settings.save();
                cx.notify();
            });
        }));
        row
    }

    pub(super) fn render_status_bar(&mut self, cx: &mut Context<Self>) -> Div {
        let t = tokens(cx);
        let message: SharedString = if self.error.is_some() {
            "Couldn't open the file".into()
        } else if let Some(p) = &self.loading {
            format!("Loading {}…", p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned())).into()
        } else if self.tree_loading {
            "Reading the tree…".into()
        } else if self.layout_pending {
            "Arranging rows and columns…".into()
        } else {
            self.status.clone()
        };
        let hidden = self.hidden_count();
        let theme = match self.settings.theme {
            ThemeChoice::System => format!("System ({})", if t.dark { "dark" } else { "light" }),
            c => c.label().to_owned(),
        };
        let d = self.data.clone();
        let selection = d.as_ref().filter(|_| !self.sel.is_empty()).map(|d| {
            let s = &self.sel;
            if s.cols.is_empty() && s.rows.len() == 1 {
                let r = s.rows.first().unwrap() as usize;
                format!("Selected: {}", d.aln.names[r])
            } else {
                let mut text = format!("Selected: {}", s.describe(d.aln.rows(), d.aln.cols()));
                // Columns hidden inside the selection are counted in it; say how many are shown.
                if let Some(m) = self.col_map()
                    && !s.cols.is_empty()
                {
                    let shown: usize = s.cols.ranges().iter().map(|&(a, b)| m.visible.partition_point(|&c| c < b) - m.visible.partition_point(|&c| c < a)).sum();
                    if shown < s.cols.len() {
                        text += &format!(" ({} shown)", view::group_digits(shown));
                    }
                }
                text
            }
        });
        let collapsed = self.collapsed_count();
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
            .when(hidden > 0, |row| row.child(div().flex_none().child(format!("{} columns hidden", view::group_digits(hidden)))))
            .when(collapsed > 0, |row| row.child(div().flex_none().child(format!("{} {} collapsed", view::group_digits(collapsed), if collapsed == 1 { "clade" } else { "clades" }))))
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
