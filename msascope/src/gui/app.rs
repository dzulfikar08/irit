//! The main view: state, input handling and layout. Painting the alignment is in `canvas.rs`, the
//! title bar, toolbar and status bar in `chrome.rs`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::v_flex;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use msascope::Loaded;
use msascope::alphabet::SeqType;
use msascope::pyramid::Pyramid;
use msascope::scheme::{self, ColorMode, Scheme};
use msascope::view::{self, ColumnMap, Viewport};

use super::cached::CachedView;
use super::metrics::*;
use super::theme::{Settings, ThemeChoice, is_dark};
use super::tokens::{self, tokens};

/// The fit zooms. `Readable` is the view a file opens at (see [`view::initial_view`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Fit {
    Readable,
    All,
    Width,
    Height,
}

impl Fit {
    pub fn label(self) -> &'static str {
        match self {
            Fit::Readable => "Readable zoom",
            Fit::All => "Fit all",
            Fit::Width => "Fit width",
            Fit::Height => "Fit height",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Fit::Readable => "R",
            Fit::All => "0",
            Fit::Width => "W",
            Fit::Height => "H",
        }
    }
}

/// What a set of cached tiles was drawn with: scale (cw, ch bits), colour mode, theme, whether
/// letters are drawn over them, display scale × 100 and the hidden-columns generation. The
/// scales are keyed exactly: they only change on zoom, and rounding would merge distinct
/// sub-pixel scales.
pub type TileKey = (u64, u64, ColorMode, scheme::Theme, bool, u32, u64);

/// Cached rasterised tiles for the current [`TileKey`], keyed by tile position.
#[derive(Default)]
pub struct TileCache {
    pub key: Option<TileKey>,
    pub tiles: HashMap<(i64, i64), (Arc<RenderImage>, u64)>,
    pub frame: u64,
    pub rendered: usize,
}

impl TileCache {
    pub fn reset(&mut self, window: &mut Window) {
        for (_, (img, _)) in self.tiles.drain() {
            let _ = window.drop_image(img);
        }
    }

    pub fn evict(&mut self, keep: usize, window: &mut Window) {
        if self.tiles.len() <= keep {
            return;
        }
        let mut by_age: Vec<_> = self.tiles.iter().map(|(k, (_, t))| (*t, *k)).collect();
        by_age.sort_unstable();
        for (_, k) in by_age.into_iter().take(self.tiles.len() - keep) {
            if let Some((img, _)) = self.tiles.remove(&k) {
                let _ = window.drop_image(img);
            }
        }
    }
}

pub struct Bench {
    phase: usize,
    frame: usize,
    last: Option<Instant>,
    intervals: Vec<f64>,
    pub paint_ms: Rc<RefCell<Vec<f64>>>,
}

/// (name, zoom x, zoom y) phases of `--bench`; zoom `None` means "fit the whole alignment".
const BENCH_PHASES: [(&str, Option<(i32, i32)>); 4] =
    [("overview (fit)", None), ("1 px per cell", Some((0, 0))), ("4 px per cell", Some((4, 4))), ("letters, 12x16 px", Some((7, 8)))];
const BENCH_FRAMES: usize = 240;

/// The columns shown while some are hidden: the map and the pyramid built for it.
pub struct HiddenCols {
    pub map: ColumnMap,
    pub pyramid: Pyramid,
    /// Changes with every rebuild; part of the tile key.
    pub generation: u64,
}

/// The overview panel's image, cached until what it shows changes.
#[derive(Default)]
pub struct OverviewCache {
    pub key: Option<(ColorMode, scheme::Theme, u64, u32, u32)>,
    pub image: Option<Arc<RenderImage>>,
}

/// What a mouse drag does.
#[derive(Clone, Copy, Debug)]
pub(super) enum Drag {
    /// Pan the alignment; the last pointer position.
    Pan(Point<Pixels>),
    /// Shift+drag: will select a block (not built yet); does nothing for now.
    Block,
    /// Resize the name panel: pointer x and panel width when the drag started.
    Splitter { x0: Pixels, w0: f32 },
    /// Pan by dragging in the overview panel.
    Overview,
}

/// The parts of the state the cached chrome (title bar, toolbar) shows. When it changes, the
/// chrome is drawn again; hover and pan frames redraw only the canvas.
#[derive(Clone, PartialEq)]
struct ChromeKey {
    file: Option<SharedString>,
    loading: bool,
    error: bool,
    zoom: String,
    fit: Option<Fit>,
    mode: ColorMode,
    theme: ThemeChoice,
    dark: bool,
    compact: bool,
    kind: Option<msascope::alphabet::Kind>,
    inserts: usize,
    hide_inserts: bool,
    hide_gappy: bool,
    gappy_pct: u8,
    overview: bool,
}

pub struct MsaScope {
    pub(super) data: Option<Arc<Loaded>>,
    /// Messages for the status bar ("3 matches", "Out of range …").
    pub(super) status: SharedString,
    pub(super) error: Option<SharedString>,
    /// The file being read, while it loads.
    pub(super) loading: Option<PathBuf>,
    pub(super) path: Option<PathBuf>,
    /// `--type`: used for the file given on the command line and every file opened later.
    ty: Option<SeqType>,
    pub(super) vp: Viewport,
    /// A fit to apply once the alignment area's size is known.
    pending: Option<Fit>,
    /// The fit in effect; re-applied when the window is resized, until the user zooms.
    pub(super) fit: Option<Fit>,
    pub(super) mode: ColorMode,
    /// The scheme `d` returns to from Differences.
    prev_scheme: Scheme,
    pub(super) tiles: Rc<RefCell<TileCache>>,
    /// Hidden columns in effect (`None`: every column shown), and the generation of the last
    /// rebuild asked for; a rebuild runs in the background and the old map stays until it ends.
    pub(super) hidden: Option<Arc<HiddenCols>>,
    cols_gen: u64,
    pub(super) cols_pending: bool,
    pub(super) hide_inserts: bool,
    pub(super) hide_gappy: bool,
    pub(super) overview: Rc<RefCell<OverviewCache>>,
    /// The overview image's bounds (window coordinates), written by its canvas each layout.
    pub(super) overview_area: Rc<Cell<Bounds<Pixels>>>,
    /// The pointer, while it is over the alignment or the track header.
    pub(super) pointer: Option<Point<Pixels>>,
    /// The view column under the pointer in the track header.
    pub(super) hover_track: Option<usize>,
    /// The hover card is hidden after a scroll until the pointer moves again.
    card_hidden: bool,
    /// The alignment area (window coordinates), written by the canvas each layout.
    pub(super) area: Rc<Cell<Bounds<Pixels>>>,
    pub(super) hover: Option<(usize, usize)>,
    /// The row under the pointer in the name panel.
    pub(super) hover_name: Option<usize>,
    pub(super) drag: Option<Drag>,
    pub(super) matches: Vec<usize>,
    match_idx: usize,
    pub(super) selected_row: Option<usize>,
    pub(super) jump: Entity<InputState>,
    pub(super) search: Entity<InputState>,
    focus: FocusHandle,
    pub(super) bench: Option<Bench>,
    pub(super) settings: Settings,
    title_bar: Entity<CachedView<MsaScope>>,
    toolbar: Entity<CachedView<MsaScope>>,
    chrome_key: Option<ChromeKey>,
    _subs: Vec<Subscription>,
}

impl MsaScope {
    pub fn new(path: Option<PathBuf>, ty: Option<SeqType>, bench: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let jump = cx.new(|cx| InputState::new(window, cx).placeholder("Go to column, or row:column"));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find sequence"));
        let subs = vec![
            cx.subscribe_in(&jump, window, |this, state, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { .. } => {
                    let text = state.read(cx).value().to_string();
                    this.jump_to(&text, cx);
                }
                InputEvent::Blur => this.refocus(window, cx),
                _ => {}
            }),
            cx.subscribe_in(&search, window, |this, state, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    let q = state.read(cx).value().to_string();
                    this.run_search(&q, cx);
                }
                InputEvent::PressEnter { shift, .. } => this.next_match(if *shift { -1 } else { 1 }, cx),
                InputEvent::Blur => this.refocus(window, cx),
                _ => {}
            }),
            // Follow the OS appearance while the theme is "System".
            cx.observe_window_appearance(window, |v, window, cx| {
                if v.settings.theme == ThemeChoice::System {
                    tokens::apply_palette(ThemeChoice::System.palette(is_dark(window.appearance())), cx);
                    window.refresh();
                    cx.notify();
                }
            }),
        ];
        let title_bar = CachedView::manual(cx, |v: &mut MsaScope, window, cx| v.render_title_bar(window, cx).into_any_element());
        let toolbar = CachedView::manual(cx, |v: &mut MsaScope, window, cx| v.render_toolbar(window, cx).into_any_element());
        let mut v = MsaScope {
            data: None,
            status: SharedString::default(),
            error: None,
            loading: None,
            path: None,
            ty,
            vp: Viewport { col0: 0.0, row0: 0.0, cw: view::LETTERS_CW, ch: view::LETTERS_CH, width: 0.0, height: 0.0 },
            pending: None,
            fit: None,
            mode: ColorMode::new(Scheme::ClustalX),
            prev_scheme: Scheme::ClustalX,
            tiles: Rc::default(),
            hidden: None,
            cols_gen: 0,
            cols_pending: false,
            hide_inserts: true,
            hide_gappy: false,
            overview: Rc::default(),
            overview_area: Rc::new(Cell::new(Bounds::default())),
            pointer: None,
            hover_track: None,
            card_hidden: false,
            area: Rc::new(Cell::new(Bounds::default())),
            hover: None,
            hover_name: None,
            drag: None,
            matches: Vec::new(),
            match_idx: 0,
            selected_row: None,
            jump,
            search,
            focus: cx.focus_handle(),
            bench: bench.then(|| Bench { phase: 0, frame: 0, last: None, intervals: Vec::new(), paint_ms: Rc::default() }),
            settings: Settings::load(),
            title_bar,
            toolbar,
            chrome_key: None,
            _subs: subs,
        };
        v.focus.focus(window, cx);
        if let Some(p) = path {
            v.open(p, cx);
        }
        v
    }

    pub(super) fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.status = SharedString::default();
        self.error = None;
        self.loading = Some(path.clone());
        self.path = Some(path.clone());
        cx.notify();
        let ty = self.ty;
        let task = cx.background_executor().spawn(async move { Loaded::open_as(&path, ty) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |v, cx| {
                v.loading = None;
                match result {
                    Ok(l) => {
                        eprintln!("msascope: {}", l.summary());
                        if let Some(rss) = peak_rss_mb() {
                            eprintln!("msascope: peak RSS after load {rss} MiB");
                        }
                        let kind = l.aln.kind();
                        if !v.mode.scheme.applies(kind) {
                            v.mode.scheme = Scheme::default_for(kind);
                        }
                        if !v.prev_scheme.applies(kind) {
                            v.prev_scheme = Scheme::default_for(kind);
                        }
                        v.data = Some(Arc::new(l));
                        v.hidden = None;
                        v.rebuild_columns(cx);
                        v.pending = Some(Fit::Readable);
                        v.vp.col0 = 0.0;
                        v.vp.row0 = 0.0;
                        v.matches.clear();
                        v.selected_row = None;
                        v.hover = None;
                    }
                    Err(e) => {
                        eprintln!("msascope: {e:#}");
                        v.data = None;
                        v.error = Some(format!("{e:#}").into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Close the file: back to the empty window.
    pub(super) fn close(&mut self, cx: &mut Context<Self>) {
        self.data = None;
        self.hidden = None;
        self.cols_gen += 1;
        self.cols_pending = false;
        self.path = None;
        self.error = None;
        self.status = SharedString::default();
        self.matches.clear();
        self.selected_row = None;
        self.hover = None;
        cx.notify();
    }

    /// Write a synthetic 2,000 × 3,000 nucleotide alignment (a viral-surveillance-like test set)
    /// to the temp folder and open it, so a first run has something to look at.
    pub(super) fn open_sample(&mut self, cx: &mut Context<Self>) {
        self.status = "Making a sample alignment (2,000 sequences × 3,000 columns)…".into();
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            let path = std::env::temp_dir().join("msascope-sample-2000x3000.fa");
            let spec = msascope::synth::Spec { rows: 2000, cols: 3000, kind: msascope::alphabet::Kind::Nucleotide, clades: 12, seed: 1 };
            let mut out = std::io::BufWriter::new(std::fs::File::create(&path)?);
            msascope::synth::write_fasta(&spec, &mut out)?;
            std::io::Write::flush(&mut out)?;
            std::io::Result::Ok(path)
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |v, cx| match result {
                Ok(p) => v.open(p, cx),
                Err(e) => {
                    v.error = Some(format!("Couldn't write the sample alignment: {e}").into());
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn prompt_open(&mut self, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(p) = paths.into_iter().next()
            {
                this.update(cx, |v, cx| v.open(p, cx)).ok();
            }
        })
        .detach();
    }

    /// Give keyboard focus back to the alignment when a text box lets go of it (Escape).
    fn refocus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.focused(cx).is_none() {
            self.focus.focus(window, cx);
        }
    }

    /// Rows and shown (view) columns.
    pub(super) fn dims(&self) -> (usize, usize) {
        self.data.as_ref().map_or((0, 0), |d| (d.aln.rows(), view::view_cols(self.col_map(), d.aln.cols())))
    }

    pub(super) fn col_map(&self) -> Option<&ColumnMap> {
        self.hidden.as_deref().map(|h| &h.map)
    }

    /// Columns hidden now.
    pub(super) fn hidden_count(&self) -> usize {
        self.data.as_ref().map_or(0, |d| d.aln.cols()) - self.dims().1
    }

    /// Rebuild the hidden-columns map and its pyramid in the background after a toggle; the
    /// view keeps the old one until the new one is ready, then keeps the column at its left edge.
    pub(super) fn rebuild_columns(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.data.clone() else { return };
        self.cols_gen += 1;
        let generation = self.cols_gen;
        let inserts = self.hide_inserts && !d.aln.inserts.is_empty();
        let gappy = self.hide_gappy.then(|| self.settings.gappy_pct as f32 / 100.0);
        if !inserts && gappy.is_none() && self.hidden.is_none() {
            self.cols_pending = false;
            return;
        }
        self.cols_pending = true;
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            ColumnMap::hiding(&d.aln, &d.stats, inserts, gappy).map(|map| {
                let pyramid = Pyramid::build_mapped(&d.aln, &d.stats.consensus, Some(&map.visible));
                Arc::new(HiddenCols { map, pyramid, generation })
            })
        });
        cx.spawn(async move |this, cx| {
            let hidden = task.await;
            this.update(cx, |v, cx| {
                if v.cols_gen != generation {
                    return;
                }
                let cols = v.dims().1;
                let left = (cols > 0).then(|| view::aln_col(v.col_map(), (v.vp.col0.max(0.0) as usize).min(cols - 1)));
                v.hidden = hidden;
                v.cols_pending = false;
                if let Some(left) = left {
                    let nv = v.col_map().map_or(left, |m| m.view_of(left).unwrap_or_else(|e| e));
                    v.vp.col0 = nv as f64 + v.vp.col0.fract();
                }
                let (r, c) = v.dims();
                v.vp.clamp(r, c);
                if v.fit.is_some() && v.pending.is_none() {
                    v.pending = v.fit;
                }
                v.hover = None;
                v.hover_track = None;
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn set_hide_inserts(&mut self, on: bool, cx: &mut Context<Self>) {
        self.hide_inserts = on;
        self.rebuild_columns(cx);
    }

    /// Hide columns with at least `pct`% gaps (`None`: show them).
    pub(super) fn set_gappy(&mut self, pct: Option<u8>, cx: &mut Context<Self>) {
        self.hide_gappy = pct.is_some();
        if let Some(p) = pct {
            self.settings.gappy_pct = p;
            self.settings.save();
        }
        self.rebuild_columns(cx);
    }

    pub(super) fn set_scheme(&mut self, s: Scheme) {
        if s != Scheme::Differences {
            self.prev_scheme = s;
        }
        self.mode.scheme = s;
    }

    /// The next scheme that applies to the open alignment (`s`).
    fn cycle_scheme(&mut self) {
        let kind = self.data.as_ref().map_or(msascope::alphabet::Kind::Protein, |d| d.aln.kind());
        let all: Vec<Scheme> = Scheme::ALL.into_iter().filter(|s| s.applies(kind)).collect();
        let i = all.iter().position(|&s| s == self.mode.scheme).map_or(0, |i| (i + 1) % all.len());
        self.set_scheme(all[i]);
    }

    pub(super) fn show_overview(&self) -> bool {
        self.settings.overview && self.bench.is_none() && self.data.is_some()
    }

    pub(super) fn set_theme(&mut self, choice: ThemeChoice, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.theme = choice;
        self.settings.save();
        tokens::apply_palette(choice.palette(is_dark(window.appearance())), cx);
        window.refresh();
        cx.notify();
    }

    /// Set the scale (px per cell), keeping the cell under (x, y) in place.
    fn set_scale(&mut self, cw: f64, ch: f64, anchor: Option<(f64, f64)>) {
        let (x, y) = anchor.unwrap_or((self.vp.width / 2.0, self.vp.height / 2.0));
        self.vp.zoom_at(cw, ch, x, y);
        let (r, c) = self.dims();
        self.vp.clamp(r, c);
    }

    /// Zoom by whole √2 steps; ends any fit.
    pub(super) fn zoom_by(&mut self, sx: i32, sy: i32, anchor: Option<(f64, f64)>) {
        self.fit = None;
        let (cw, ch) = (view::zoom_step(self.vp.cw, sx), view::zoom_step(self.vp.ch, sy));
        self.set_scale(cw, ch, anchor);
    }

    pub(super) fn apply_fit(&mut self, f: Fit) {
        let (rows, cols) = self.dims();
        let (w, h) = (self.vp.width, self.vp.height);
        match f {
            Fit::Readable => {
                let (cw, ch) = view::initial_view(rows, cols, w, h);
                self.vp.cw = cw;
                self.vp.ch = ch;
            }
            Fit::All => {
                self.vp.cw = view::fit_scale(cols, w, view::LETTERS_CW);
                self.vp.ch = view::fit_scale(rows, h, view::LETTERS_CH);
                self.vp.col0 = 0.0;
                self.vp.row0 = 0.0;
            }
            Fit::Width => {
                self.vp.cw = view::fit_scale(cols, w, view::LETTERS_CW);
                self.vp.col0 = 0.0;
            }
            Fit::Height => {
                self.vp.ch = view::fit_scale(rows, h, view::LETTERS_CH);
                self.vp.row0 = 0.0;
            }
        }
        self.vp.clamp(rows, cols);
        self.fit = Some(f);
    }

    /// A fit chosen from the toolbar or a key: now if the area size is known, else at first layout.
    pub(super) fn request_fit(&mut self, f: Fit) {
        if self.vp.width > 0.0 { self.apply_fit(f) } else { self.pending = Some(f) }
    }

    fn pan(&mut self, dx_px: f64, dy_px: f64) {
        self.vp.col0 += dx_px / self.vp.cw;
        self.vp.row0 += dy_px / self.vp.ch;
        let (r, c) = self.dims();
        self.vp.clamp(r, c);
    }

    /// Scroll so that cell (row, col) is in view, roughly centred.
    fn center_on(&mut self, row: Option<usize>, col: Option<usize>) {
        if let Some(c) = col {
            self.vp.col0 = c as f64 - self.vp.width / self.vp.cw / 2.0;
        }
        if let Some(r) = row {
            self.vp.row0 = r as f64 - self.vp.height / self.vp.ch / 2.0;
        }
        let (rows, cols) = self.dims();
        self.vp.clamp(rows, cols);
    }

    /// Go to a column (alignment numbering, from 1) or `row:column`. A hidden column goes to the
    /// next shown one.
    fn jump_to(&mut self, text: &str, cx: &mut Context<Self>) {
        let parse = |s: &str| s.trim().replace([',', '_'], "").parse::<usize>().ok().filter(|&n| n >= 1).map(|n| n - 1);
        let (row, col) = match text.split_once(':') {
            Some((r, c)) => (parse(r), parse(c)),
            None => (None, parse(text)),
        };
        let rows = self.dims().0;
        let cols = self.data.as_ref().map_or(0, |d| d.aln.cols());
        let shown = self.dims().1;
        let mut note = None;
        let vcol = col.filter(|&c| c < cols).map(|c| match self.col_map().map(|m| m.view_of(c)) {
            None | Some(Ok(_)) => self.col_map().map_or(c, |m| m.view_of(c).unwrap()),
            Some(Err(v)) => {
                note = Some(format!("Column {} is hidden; showing the next shown column", view::group_digits(c + 1)));
                v.min(shown.saturating_sub(1))
            }
        });
        if row.is_none_or(|r| r < rows) && col.is_none_or(|c| c < cols) && (row.is_some() || col.is_some()) {
            let col = vcol;
            self.status = note.map(SharedString::from).unwrap_or_default();
            if self.vp.cw < view::LETTERS_CW && col.is_some() {
                self.fit = None;
                self.set_scale(view::LETTERS_CW, self.vp.ch.max(view::LETTERS_CH), None);
            }
            self.center_on(row, col);
            self.selected_row = row.or(self.selected_row);
        } else {
            self.status = format!("Out of range: {} rows × {} columns", view::group_digits(rows), view::group_digits(cols)).into();
        }
        cx.notify();
    }

    fn run_search(&mut self, q: &str, cx: &mut Context<Self>) {
        let Some(d) = self.data.clone() else { return };
        self.matches = view::search_names(&d.aln.names, q);
        self.match_idx = 0;
        if let Some(&r) = self.matches.first() {
            self.select_row(r);
        }
        self.status = if q.is_empty() { SharedString::default() } else { format!("{} matches", view::group_digits(self.matches.len())).into() };
        cx.notify();
    }

    fn next_match(&mut self, step: isize, cx: &mut Context<Self>) {
        if self.matches.is_empty() {
            return;
        }
        let n = self.matches.len() as isize;
        self.match_idx = ((self.match_idx as isize + step).rem_euclid(n)) as usize;
        self.select_row(self.matches[self.match_idx]);
        self.status = format!("match {} of {}", self.match_idx + 1, n).into();
        cx.notify();
    }

    fn select_row(&mut self, r: usize) {
        self.selected_row = Some(r);
        if self.vp.ch < NAME_ROW_MIN {
            self.fit = None;
            self.set_scale(self.vp.cw, view::LETTERS_CH, None);
        }
        let visible = self.vp.row0 <= r as f64 && (r as f64) < self.vp.row0 + self.vp.height / self.vp.ch - 1.0;
        if !visible {
            self.center_on(Some(r), None);
        }
    }

    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let k = &ev.keystroke;
        let (w, h) = (self.vp.width, self.vp.height);
        let ctrl = k.modifiers.control || k.modifiers.platform;
        match k.key.as_str() {
            "left" => self.pan(-w / 8.0, 0.0),
            "right" => self.pan(w / 8.0, 0.0),
            "up" => self.pan(0.0, -h / 8.0),
            "down" => self.pan(0.0, h / 8.0),
            "pageup" => self.pan(0.0, -h * 0.9),
            "pagedown" => self.pan(0.0, h * 0.9),
            "home" if ctrl => self.center_on(Some(0), Some(0)),
            "end" if ctrl => self.center_on(Some(usize::MAX / 2), Some(usize::MAX / 2)),
            "home" => self.vp.col0 = 0.0,
            "end" => self.pan(1e12, 0.0),
            "+" | "=" => self.zoom_by(1, 1, None),
            "-" => self.zoom_by(-1, -1, None),
            "0" => self.request_fit(Fit::All),
            "w" if !ctrl => self.request_fit(Fit::Width),
            "h" if !ctrl => self.request_fit(Fit::Height),
            "r" if !ctrl => self.request_fit(Fit::Readable),
            "d" if !ctrl => self.toggle_differences(),
            "c" if !ctrl => self.mode.by_conservation = !self.mode.by_conservation,
            "s" if !ctrl => self.cycle_scheme(),
            "i" if !ctrl => self.set_hide_inserts(!self.hide_inserts, cx),
            "g" if !ctrl => {
                let p = (!self.hide_gappy).then_some(self.settings.gappy_pct);
                self.set_gappy(p, cx)
            }
            "o" if ctrl => self.prompt_open(cx),
            "f" if ctrl => self.search.update(cx, |s, cx| s.focus(window, cx)),
            "g" if ctrl => self.jump.update(cx, |s, cx| s.focus(window, cx)),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Differences from the consensus, or back to the scheme before it.
    pub(super) fn toggle_differences(&mut self) {
        if self.mode.scheme == Scheme::Differences {
            self.mode.scheme = self.prev_scheme;
        } else {
            self.mode.scheme = Scheme::Differences;
        }
    }

    fn on_scroll(&mut self, ev: &ScrollEvent, _: &mut Window, cx: &mut Context<Self>) {
        let d = ev.delta.pixel_delta(px(40.));
        let (dx, dy) = (f64::from(d.x), f64::from(d.y));
        let area = self.area.get();
        let (x, y) = (f64::from(ev.position.x - area.origin.x), f64::from(ev.position.y - area.origin.y));
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
    fn update_hover(&mut self, pos: Point<Pixels>) {
        let area = self.area.get();
        let (rows, cols) = self.dims();
        let (x, y) = (f64::from(pos.x - area.origin.x), f64::from(pos.y - area.origin.y));
        self.hover = self.vp.hit(x, y, rows, cols);
        // Over the name panel: the row only.
        let in_names = pos.x < area.origin.x && pos.x >= area.origin.x - px(self.settings.name_width);
        self.hover_name = if in_names { self.vp.hit(0.0, y, rows, cols).map(|(r, _)| r) } else { None };
        let in_header = pos.y < area.origin.y && pos.y >= area.origin.y - px(HEADER_H) && pos.x >= area.origin.x;
        self.hover_track = if in_header { self.vp.hit(x, 0.0, rows.max(1), cols).map(|(_, c)| c) } else { None };
        self.pointer = Some(pos);
    }

    /// Whether the hover card is shown: over a cell or the track header, not while dragging or
    /// just after a scroll.
    pub(super) fn card_visible(&self) -> bool {
        self.data.is_some() && self.drag.is_none() && !self.card_hidden && self.pointer.is_some() && (self.hover.is_some() || self.hover_track.is_some())
    }

    fn step_bench(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(b) = &mut self.bench else { return };
        if self.data.is_none() || self.vp.width <= 0.0 || self.pending.is_some() {
            window.request_animation_frame();
            return;
        }
        let now = Instant::now();
        if b.frame == 0 {
            let (name, zoom) = BENCH_PHASES[b.phase];
            eprintln!("bench: phase {name}");
            match zoom {
                None => self.apply_fit(Fit::All),
                Some((zx, zy)) => {
                    self.fit = None;
                    self.vp.col0 = 0.0;
                    self.vp.row0 = 0.0;
                    self.set_scale(view::zoom_px(zx), view::zoom_px(zy), Some((0.0, 0.0)));
                }
            }
            let b = self.bench.as_mut().unwrap();
            b.intervals.clear();
            b.paint_ms.borrow_mut().clear();
            b.last = None;
            self.tiles.borrow_mut().rendered = 0;
        } else {
            // Pan diagonally by 1/12 of the view per frame (new tiles every few frames); bounce at the edges.
            let (dx, dy) = (self.vp.width / 12.0, self.vp.height / 24.0);
            let before = (self.vp.col0, self.vp.row0);
            self.pan(dx, dy);
            if (self.vp.col0, self.vp.row0) == before {
                self.vp.col0 = 0.0;
                self.vp.row0 = 0.0;
            }
        }
        let b = self.bench.as_mut().unwrap();
        if let Some(last) = b.last {
            b.intervals.push(now.duration_since(last).as_secs_f64() * 1000.0);
        }
        b.last = Some(now);
        b.frame += 1;
        if b.frame > BENCH_FRAMES {
            let mut iv = b.intervals.clone();
            let mut pm = b.paint_ms.borrow().clone();
            let stats = |v: &mut Vec<f64>| {
                v.sort_by(f64::total_cmp);
                let q = |p: f64| v[((v.len() - 1) as f64 * p) as usize];
                format!("median {:.1} ms, p95 {:.1} ms, max {:.1} ms", q(0.5), q(0.95), q(1.0))
            };
            let slow = iv.iter().filter(|&&x| x > 25.0).count();
            eprintln!(
                "bench: {} frames; interval {}; frames over 25 ms: {slow}; paint {}; tiles rasterised {}",
                iv.len(),
                stats(&mut iv),
                stats(&mut pm),
                self.tiles.borrow().rendered
            );
            b.phase += 1;
            b.frame = 0;
            if b.phase == BENCH_PHASES.len() {
                if let Some(rss) = peak_rss_mb() {
                    eprintln!("bench: peak RSS {rss} MiB");
                }
                cx.quit();
                return;
            }
        }
        window.request_animation_frame();
    }

    fn chrome_key(&self, window: &Window, cx: &App) -> ChromeKey {
        let file = self.path.as_ref().and_then(|p| p.file_name()).map(|n| SharedString::from(n.to_string_lossy().into_owned()));
        ChromeKey {
            file,
            loading: self.loading.is_some(),
            error: self.error.is_some(),
            zoom: view::zoom_label(self.vp.cw, self.vp.ch),
            fit: self.fit,
            mode: self.mode,
            theme: self.settings.theme,
            dark: tokens(cx).dark,
            compact: window.viewport_size().width < COMPACT_WINDOW,
            kind: self.data.as_ref().map(|d| d.aln.kind()),
            inserts: self.data.as_ref().map_or(0, |d| d.aln.insert_count()),
            hide_inserts: self.hide_inserts,
            hide_gappy: self.hide_gappy,
            gappy_pct: self.settings.gappy_pct,
            overview: self.settings.overview,
        }
    }

    fn on_mouse_down(&mut self, ev: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        self.update_hover(ev.position);
        if ev.modifiers.shift {
            self.drag = Some(Drag::Block);
        } else {
            self.drag = Some(Drag::Pan(ev.position));
            if let Some(r) = self.hover.map(|(r, _)| r).or(self.hover_name) {
                self.selected_row = Some(r);
            }
        }
        cx.notify();
    }

    fn on_mouse_move(&mut self, ev: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        match self.drag.filter(|_| ev.dragging()) {
            Some(Drag::Pan(last)) => {
                let d = ev.position - last;
                self.pan(-f64::from(d.x), -f64::from(d.y));
                self.drag = Some(Drag::Pan(ev.position));
            }
            Some(Drag::Splitter { x0, w0 }) => {
                let w = (w0 + f32::from(ev.position.x - x0)).clamp(NAME_W_MIN, NAME_W_MAX);
                self.settings.name_width = w.round();
            }
            Some(Drag::Overview) => self.overview_pan(ev.position),
            Some(Drag::Block) | None => {}
        }
        if !ev.dragging() {
            self.card_hidden = false;
        }
        self.update_hover(ev.position);
        cx.notify();
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

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(Drag::Splitter { .. }) = self.drag {
            self.settings.save();
        }
        self.drag = None;
        cx.notify();
    }
}

impl Render for MsaScope {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let area = self.area.get();
        let (w, h) = (f64::from(area.size.width), f64::from(area.size.height));
        if (w, h) != (self.vp.width, self.vp.height) && w > 0.0 {
            self.vp.width = w;
            self.vp.height = h;
            let (r, c) = self.dims();
            self.vp.clamp(r, c);
            if self.pending.is_none() {
                self.pending = self.fit;
            }
        }
        if self.data.is_some() && w > 0.0 {
            if let Some(f) = self.pending.take() {
                self.apply_fit(f);
            }
        } else if self.data.is_some() {
            window.request_animation_frame(); // lay out once to learn the area size
        }
        self.step_bench(window, cx);

        // Cached chrome: drawn again only when what it shows changed.
        let key = self.chrome_key(window, cx);
        let chrome_changed = self.chrome_key.as_ref() != Some(&key);
        self.chrome_key = Some(key);
        let chrome = |view: &Entity<CachedView<MsaScope>>, h: Pixels| -> AnyElement {
            let v = AnyView::from(view.clone());
            if chrome_changed {
                div().flex_none().w_full().h(h).child(v).into_any_element()
            } else {
                v.cached(StyleRefinement::default().flex_none().w_full().h(h)).into_any_element()
            }
        };
        let title = chrome(&self.title_bar, TITLE_BAR_HEIGHT);
        let tools = chrome(&self.toolbar, CONTROL_LG + px(1.));

        let t = tokens(cx);
        let main = if self.data.is_some() {
            let name_w = px(self.settings.name_width);
            let alignment = div()
                .id("alignment")
                .relative()
                .flex_1()
                .min_w_0()
                .h_full()
                .on_hover(cx.listener(|v, hovered: &bool, _, cx| {
                    if !*hovered && v.drag.is_none() {
                        v.hover = None;
                        v.hover_name = None;
                        v.hover_track = None;
                        v.pointer = None;
                        cx.notify();
                    }
                }))
                .track_focus(&self.focus)
                .on_key_down(cx.listener(Self::on_key))
                .on_scroll(cx.listener(Self::on_scroll))
                .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                .child(self.alignment_canvas(window, cx))
                // The name panel's splitter, over its right edge.
                .child(
                    div()
                        .id("name-splitter")
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(name_w - px(SPLITTER_W / 2.0))
                        .w(px(SPLITTER_W))
                        .cursor_col_resize()
                        .hover(|s| s.bg(t.border_strong))
                        .when(matches!(self.drag, Some(Drag::Splitter { .. })), |d| d.bg(t.accent))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|v, ev: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                v.drag = Some(Drag::Splitter { x0: ev.position.x, w0: v.settings.name_width });
                                cx.notify();
                            }),
                        ),
                );
            // Move and release go to the row, so drags that leave the alignment (or start in the
            // overview) keep working.
            div()
                .flex()
                .flex_row()
                .flex_1()
                .min_h_0()
                .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
                .on_mouse_move(cx.listener(Self::on_mouse_move))
                .child(alignment)
                .when(self.show_overview(), |d| d.child(self.render_overview(window, cx)))
                .into_any_element()
        } else {
            self.render_empty(window, cx).into_any_element()
        };

        v_flex()
            .size_full()
            .bg(t.bg)
            .text_color(t.fg)
            .on_key_down(cx.listener(|v, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape" {
                    v.focus.focus(window, cx);
                    cx.notify();
                }
            }))
            .child(title)
            .child(tools)
            .child(main)
            .child(self.render_status_bar(cx))
            .when_some(self.render_hover_card(window, cx), |d, card| d.child(card))
    }
}

pub fn peak_rss_mb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("VmHWM:"))?;
    Some(line.split_whitespace().nth(1)?.parse::<u64>().ok()? / 1024)
}
