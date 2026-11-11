//! Title bar (app mark, file tab, theme menu), toolbar, status bar, and the empty / loading /
//! error card. The title bar and status bar follow Kantor's layout; the toolbar its format bar.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{Disableable as _, IconName, Selectable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use gpui_kit::assets::IconName as Lucide;
use msascope::scheme::{self, Scheme};
use msascope::view;

use super::app::{Fit, FindMode, MsaScope};
use super::figure::{FigOptions, Kind, Region};
use msascope::rows::{Sort, SortKey};
use msascope::parse::Stage;
use super::components::{IconButtonExt as _, dot, icon_button, kbd};
use super::fonts::tabular;
use super::metrics::*;
use super::theme::{GAPPY_CHOICES, ThemeChoice};
use msascope::select::ExportOptions;
use super::tokens::tokens;

/// What a Sort menu item does.
type SortAction = dyn Fn(&mut MsaScope, &mut Context<MsaScope>);

/// A thin vertical rule between toolbar groups.
fn separator(cx: &App) -> Div {
    div().flex_none().w(px(1.)).h(ICON_LG).mx(SPACE_2).bg(tokens(cx).border)
}

impl MsaScope {
    /// The file in the tab: the one loading, else the one open.
    fn file_name(&self) -> Option<String> {
        self.loading().or(self.path.as_deref()).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned())
    }

    /// The loading line ("12.5 of 40.2 MB read · 31,240 sequences", or the step after reading)
    /// and the share of the progress bar to fill.
    pub(super) fn progress_text(&self) -> Option<(String, f32)> {
        let job = self.jobs.current()?;
        let s = job.progress.snapshot();
        let mb = |b: u64| format!("{:.1}", b as f64 / 1e6);
        let gz = job.path.extension().is_some_and(|e| e == "gz");
        let rows = format!("{} sequences", view::group_digits(s.rows as usize));
        let text = match s.stage {
            Stage::Read => {
                let pass = match (self.load_two_pass, s.pass) {
                    (false, _) => "",
                    (true, 1) => "names: ",
                    (true, _) => "residues: ",
                };
                let of = if s.total > 0 { format!(" of {} MB", mb(s.total)) } else { String::new() };
                format!("{pass}{}{of} read{} · {rows}", mb(s.bytes), if gz { " (compressed)" } else { "" })
            }
            Stage::Columns => format!("{rows} read · column summaries…"),
            Stage::Overview => format!("{rows} read · overview…"),
        };
        // The bar: reading takes most of the time (two passes share it for Clustal and
        // Stockholm); the two steps after it the last 10%.
        let f = s.fraction() as f32;
        let frac = match s.stage {
            Stage::Read if !self.load_two_pass => f * 0.9,
            Stage::Read if s.pass >= 2 => 0.3 + f * 0.6,
            Stage::Read => f * 0.3,
            Stage::Columns => 0.92,
            Stage::Overview => 0.96,
        };
        Some((text, frac))
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
            } else if self.loading().is_some() {
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
        let fig = self.fig;
        let export = {
            let e = entity.clone();
            Button::new("export").ghost().xsmall().icon(Lucide::ArrowDown).label("Export").dropdown_caret(true).tooltip("Export FASTA, PNG or SVG, or copy").dropdown_menu(move |menu, _, _| {
                let item = |label: &str, enabled: bool, f: fn(&mut MsaScope, &mut Window, &mut Context<MsaScope>)| {
                    let e = e.clone();
                    PopupMenuItem::new(label.to_string()).disabled(!enabled).on_click(move |_, window, cx| {
                        let _ = e.update(cx, |v, cx| {
                            f(v, window, cx);
                            cx.notify();
                        });
                    })
                };
                let fig_check = |label: &str, on: bool, f: fn(&mut FigOptions)| {
                    let e = e.clone();
                    PopupMenuItem::new(label.to_string()).checked(on).on_click(move |_, _, cx| {
                        let _ = e.update(cx, |v, cx| {
                            f(&mut v.fig);
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
                    .separator()
                    .item(item("View as PNG…", has_data, |v, w, cx| v.export_figure(Region::View, Kind::Png, w, cx)))
                    .item(item("View as SVG…", has_data, |v, w, cx| v.export_figure(Region::View, Kind::Svg, w, cx)))
                    .item(item("Selection as PNG…", has_sel, |v, w, cx| v.export_figure(Region::Selection, Kind::Png, w, cx)))
                    .item(item("Selection as SVG…", has_sel, |v, w, cx| v.export_figure(Region::Selection, Kind::Svg, w, cx)))
                    .item(fig_check("Names in figures", fig.names, |o| o.names = !o.names))
                    .item(fig_check("Ruler in figures", fig.ruler, |o| o.ruler = !o.ruler))
                    .item(fig_check("PNG at 1×", fig.scale == 1, |o| o.scale = 1))
                    .item(fig_check("PNG at 2×", fig.scale == 2, |o| o.scale = 2))
                    .item(fig_check("PNG at 4×", fig.scale == 4, |o| o.scale = 4))
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
            .child({
                let e = entity.clone();
                let motif = self.find_mode == FindMode::Motif;
                Button::new("find-mode")
                    .ghost()
                    .xsmall()
                    .selected(motif)
                    .label(if motif { "Residues" } else { "Names" })
                    .tooltip(if motif { "Searching residues (motifs) · Ctrl Shift F" } else { "Searching names · Ctrl Shift F searches residues" })
                    .on_click(move |_, window, cx| {
                        let _ = e.update(cx, |v, cx| {
                            let m = if v.find_mode == FindMode::Names { FindMode::Motif } else { FindMode::Names };
                            v.set_find_mode(m, window, cx);
                        });
                    })
            })
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
        if has_data {
            let e1 = entity.clone();
            let (sort, tree, tree_order) = (self.sort, self.tree.is_some(), self.tree_order);
            let d = self.data.clone();
            let name_of = move |r: usize| d.as_ref().map_or(String::new(), |d| d.aln.names[r].clone());
            let reference = self.reference.map(&name_of);
            let candidate = self.reference_candidate().filter(|&r| Some(r) != self.reference).map(|r| (r, name_of(r)));
            let filter_on = self.filter.is_active() || self.filter_bar;
            row = row.child(
                Button::new("sort")
                    .ghost()
                    .xsmall()
                    .selected(sort.is_some())
                    .icon(Lucide::SortAscending)
                    .label(self.sort_label())
                    .dropdown_caret(true)
                    .tooltip("Row order · T tree order")
                    .dropdown_menu(move |mut menu, _, _| {
                        let e = e1.clone();
                        let act = move |f: Box<SortAction>| {
                            let e = e.clone();
                            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                                let _ = e.update(cx, |v, cx| f(v, cx));
                            }
                        };
                        if tree {
                            menu = menu.item(PopupMenuItem::new("Tree order   T").checked(sort.is_none() && tree_order).on_click(act(Box::new(|v, cx| v.set_sort(None, true, cx)))));
                        }
                        menu = menu.item(PopupMenuItem::new("File order").checked(sort.is_none() && !(tree && tree_order)).on_click(act(Box::new(|v, cx| v.set_sort(None, false, cx)))));
                        menu = menu.separator();
                        for key in SortKey::ALL {
                            let label = match (key, &reference) {
                                (SortKey::Reference, Some(n)) => format!("Identity to {}", truncate(n, 28)),
                                _ => key.label().to_string(),
                            };
                            let rev = sort.is_some_and(|s| s.key == key && s.reverse);
                            menu = menu.item(
                                PopupMenuItem::new(label)
                                    .checked(sort.is_some_and(|s| s.key == key))
                                    .disabled(key == SortKey::Reference && reference.is_none())
                                    .on_click(act(Box::new(move |v, cx| v.set_sort(Some(Sort { key, reverse: rev }), false, cx)))),
                            );
                        }
                        if let Some(s) = sort {
                            menu = menu.item(PopupMenuItem::new("Reverse order").checked(s.reverse).on_click(act(Box::new(move |v, cx| v.set_sort(Some(Sort { reverse: !s.reverse, ..s }), false, cx)))));
                        }
                        menu = menu.separator();
                        match &candidate {
                            Some((r, n)) => {
                                let r = *r;
                                menu = menu.item(PopupMenuItem::new(format!("Set reference: {}", truncate(n, 28))).on_click(act(Box::new(move |v, cx| v.set_reference(Some(r), cx)))));
                            }
                            None => menu = menu.item(PopupMenuItem::new("Set reference: put the cursor on a row").disabled(true)),
                        }
                        if reference.is_some() {
                            menu = menu.item(PopupMenuItem::new("Clear the reference").on_click(act(Box::new(|v, cx| v.set_reference(None, cx)))));
                        }
                        menu = menu.separator().item(PopupMenuItem::new("Filter rows…   Ctrl L").checked(filter_on).on_click({
                            let e = e1.clone();
                            move |_, window, cx| {
                                let _ = e.update(cx, |v, cx| v.toggle_filter_bar(window, cx));
                            }
                        }));
                        if tree {
                            menu = menu
                                .separator()
                                .item(PopupMenuItem::new("Open another tree…").on_click(act(Box::new(|v, cx| v.prompt_open(cx)))))
                                .item(PopupMenuItem::new("Remove the tree").on_click(act(Box::new(|v, cx| v.remove_tree(cx)))));
                        }
                        menu
                    }),
            );
        }
        if self.tree.is_some() {
            let e2 = entity.clone();
            let panel = self.settings.tree_panel;
            let shown = panel && self.tree_order && self.sort.is_none();
            row = row.child(icon_button("tree-panel", Lucide::PanelLeft, shown, cx).tip("Tree panel", None).on_click(move |_, _, cx| {
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
        } else if let Some(p) = self.loading() {
            let name = p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
            let detail = self.progress_text().map(|(t, f)| format!(": {:.0}% · {t}", f * 100.0)).unwrap_or_default();
            format!("Loading {name}{detail} · Esc cancels").into()
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
        let shown_of = self.shown_of();
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
            .when_some(shown_of, |row, (n, m)| row.child(div().flex_none().text_color(t.warning).child(format!("{} of {} rows shown", view::group_digits(n), view::group_digits(m)))))
            .when(collapsed > 0, |row| row.child(div().flex_none().child(format!("{} {} collapsed", view::group_digits(collapsed), if collapsed == 1 { "clade" } else { "clades" }))))
            .child(div().flex_1().min_w_0().truncate().when(self.error.is_some(), |d| d.text_color(t.danger)).child(message))
            .when_some(selection, |row, s| row.child(div().flex_none().max_w(px(260.)).truncate().text_color(t.fg).child(s)))
            .when_some(d, |row, d| row.child(div().flex_none().child(format!("loaded in {:.2} s · {} MiB", d.load_secs(), view::group_digits(d.bytes() >> 20)))))
            .child(div().flex_none().child(theme))
            .child(div().flex_none().text_color(t.fg_dim).child(concat!("v", env!("CARGO_PKG_VERSION"))))
    }

    /// The filter bar under the toolbar: name / accession (or `/regex/`), minimum identity and
    /// maximum gap share, the clade filter, "N of M rows shown" and Clear.
    pub(super) fn render_filter_bar(&mut self, cx: &mut Context<Self>) -> Div {
        let t = tokens(cx);
        let label = |s: &'static str| div().flex_none().text_size(FONT_SMALL).text_color(t.fg_muted).child(s);
        let field = |state: &Entity<InputState>, w: Pixels| div().w(w).child(Input::new(state).xsmall());
        let total = self.data.as_ref().map_or(0, |d| d.aln.rows());
        let count = match self.shown_of() {
            Some((n, m)) => format!("{} of {} rows shown", view::group_digits(n), view::group_digits(m)),
            None => format!("All {} rows shown", view::group_digits(total)),
        };
        let active = self.filter.is_active();
        let ident_label = if self.reference.is_some() { "Min identity to reference %" } else { "Min identity %" };
        let clade = self.clade_label.clone().map(|c| {
            h_flex()
                .id("clade-chip")
                .flex_none()
                .gap(SPACE_2)
                .px(SPACE_3)
                .h(px(22.))
                .rounded(RADIUS_SM)
                .bg(t.accent_soft)
                .text_size(FONT_SMALL)
                .text_color(t.fg)
                .child(format!("Clade: {c}"))
                .child(div().text_color(t.fg_dim).child("×"))
                .cursor_pointer()
                .on_click(cx.listener(|v, _, _, cx| v.clear_clade(cx)))
        });
        h_flex()
            .flex_none()
            .w_full()
            .h(CONTROL_LG)
            .px(SPACE_4)
            .gap(SPACE_3)
            .bg(t.chrome)
            .border_b_1()
            .border_color(t.border)
            .child(div().flex_none().text_size(FONT_SMALL).font_weight(FontWeight::MEDIUM).text_color(t.fg).child("Filter rows"))
            .child(field(&self.filter_name, px(240.)))
            .child(label(ident_label))
            .child(field(&self.filter_ident, px(58.)))
            .child(label("Max gaps %"))
            .child(field(&self.filter_gaps, px(58.)))
            .children(clade)
            .child(div().flex_1())
            .child(div().flex_none().text_size(FONT_SMALL).font_features(tabular()).text_color(if active { t.warning } else { t.fg_muted }).child(count))
            .child(Button::new("filter-clear").ghost().xsmall().label("Clear").disabled(!active).on_click(cx.listener(|v, _, _, cx| v.clear_filters(cx))))
            .child(icon_button("filter-close", IconName::Close, false, cx).tip("Close and clear the filter", Some("Ctrl+L")).on_click(cx.listener(|v, _, _, cx| {
                v.clear_filters(cx);
                v.filter_bar = false;
            })))
    }

    /// Up to five recent files that still exist, with their size and age; a click opens one.
    fn render_recent(&self, cx: &mut Context<Self>) -> Option<Div> {
        let t = tokens(cx);
        let now = std::time::SystemTime::now();
        let rows: Vec<_> = self
            .settings
            .recent
            .iter()
            .filter_map(|p| std::fs::metadata(p).ok().filter(|m| m.is_file()).map(|m| (p.clone(), m)))
            .take(5)
            .enumerate()
            .map(|(i, (p, m))| {
                let name = p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
                let age = m.modified().ok().and_then(|t| now.duration_since(t).ok()).map_or(String::new(), |d| {
                    let s = d.as_secs();
                    match s {
                        0..60 => "just now".into(),
                        60..3600 => format!("{} min ago", s / 60),
                        3600..86400 => format!("{} h ago", s / 3600),
                        _ => format!("{} d ago", s / 86400),
                    }
                });
                let size = m.len() as f64;
                let size = if size >= 1e9 { format!("{:.1} GB", size / 1e9) } else if size >= 1e6 { format!("{:.1} MB", size / 1e6) } else { format!("{:.0} kB", (size / 1e3).max(1.0)) };
                h_flex()
                    .id(("recent", i))
                    .w_full()
                    .gap(SPACE_4)
                    .px(SPACE_3)
                    .py(SPACE_2)
                    .rounded(RADIUS_SM)
                    .text_size(FONT_BODY)
                    .cursor_pointer()
                    .hover(|s| s.bg(t.hover))
                    .child(div().flex_1().min_w_0().truncate().text_color(t.fg).child(name))
                    .child(div().flex_none().text_color(t.fg_dim).font_features(tabular()).child(format!("{size} · modified {age}")))
                    .on_click(cx.listener(move |v, _, _, cx| v.open(p.clone(), cx)))
            })
            .collect();
        (!rows.is_empty()).then(|| v_flex().pt(SPACE_4).gap(SPACE_1).child(div().text_size(FONT_CAPTION).text_color(t.fg_muted).child("Recent")).children(rows))
    }

    /// The window without an alignment: open a file or a sample, or see why loading failed.
    pub(super) fn render_empty(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Div {
        let t = tokens(cx);
        let card = v_flex().w(px(420.)).p(SPACE_9).gap(SPACE_4).rounded(RADIUS_XL).border_1().border_color(t.border).bg(t.chrome);
        let card = if let Some(p) = self.loading() {
            let name = p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
            let (text, frac) = self.progress_text().unwrap_or_default();
            let elapsed = self.jobs.current().map_or(0.0, |j| j.started.elapsed().as_secs_f64());
            let keep = self.path.as_ref().filter(|_| self.data.is_some()).and_then(|p| p.file_name()).map(|n| n.to_string_lossy().into_owned());
            card.child(div().text_size(FONT_TITLE).font_weight(FontWeight::SEMIBOLD).truncate().child(format!("Loading {name}")))
                .child(
                    div()
                        .w_full()
                        .h(px(6.))
                        .rounded(px(3.))
                        .bg(t.border)
                        .child(div().h_full().rounded(px(3.)).bg(t.accent).w(relative(frac.clamp(0.0, 1.0)))),
                )
                .child(h_flex().gap(SPACE_4).text_size(FONT_BODY).text_color(t.fg_muted).font_features(tabular()).child(div().flex_1().min_w_0().truncate().child(text)).child(format!("{elapsed:.1} s")))
                .child(div().text_size(FONT_BODY).text_color(t.fg_dim).child("Reading the file, then the column summaries and the overview."))
                .child(
                    h_flex()
                        .pt(SPACE_4)
                        .gap(SPACE_4)
                        .child(Button::new("cancel-load").small().label("Cancel").on_click(cx.listener(|v, _, _, cx| v.cancel_load(cx))))
                        .child(kbd("Esc", cx))
                        .child(div().flex_1())
                        .when_some(keep, |d, k| d.child(div().max_w(px(200.)).text_size(FONT_CAPTION).text_color(t.fg_dim).truncate().child(format!("{k} stays open if you cancel")))),
                )
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
                .children(self.render_recent(cx))
        };
        div().flex_1().min_h_0().flex().items_center().justify_center().bg(t.bg).child(card)
    }
}

use gpui_kit::component::input::InputState;

/// `s` cut to `n` characters with an ellipsis.
fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n { s.to_string() } else { s.chars().take(n - 1).collect::<String>() + "…" }
}
