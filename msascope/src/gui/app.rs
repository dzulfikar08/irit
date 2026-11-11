//! The main view: state, loading, layout rebuilds and the window's element tree. Painting the
//! alignment is in `canvas.rs` (the tree in `tree_panel.rs`), mouse, keys, selection, copy and
//! export in `input.rs`, the title bar, toolbar and status bar in `chrome.rs`.

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
use msascope::jobs::{Done, Jobs};
use msascope::motif::Hits;
use msascope::rows::{Filter, RowStats, Sort};
use msascope::pyramid::Pyramid;
use msascope::reference::Reference;
use msascope::scheme::{self, ColorMode, Scheme};
use msascope::select::{ExportOptions, RangeSet, Selection};
use msascope::tree::{self, TreeIndex, TreeRows};
use msascope::view::{self, ColumnMap, RowView, Viewport};

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

/// What is shown when columns are hidden or rows follow a tree: the column map, the row view,
/// the tree's node positions for that row view, and the pyramid built for them.
pub struct Layout {
    pub map: Option<ColumnMap>,
    pub rows: Option<RowView>,
    pub tree: Option<TreeRows>,
    pub pyramid: Pyramid,
    /// Changes with every rebuild; part of the tile key.
    pub generation: u64,
    /// Sequences shown when a filter hides some, else `None`.
    pub shown: Option<usize>,
    /// The reference row Differences, the identity bars and the pyramid compare with; `None`:
    /// the consensus.
    pub base: Option<Arc<Reference>>,
}

/// What the reference row does besides sorting and filtering.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RefOpts {
    /// Pinned under the tracks.
    pub pin: bool,
    /// The ruler counts its residues.
    pub ruler: bool,
    /// Differences, the identity bars and the density view compare with it.
    pub compare: bool,
}

impl Default for RefOpts {
    fn default() -> Self {
        RefOpts { pin: true, ruler: false, compare: true }
    }
}

/// Header tiles: the track bands rasterised, by tile index along x.
pub type HeaderTiles = msascope::header::TileStore<msascope::header::HeaderKey, i64, Arc<RenderImage>>;

/// What the Find box searches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindMode {
    Names,
    Motif,
}

/// A motif search's result and where the user is in it.
pub struct MotifState {
    pub hits: Arc<Hits>,
    /// Indices into `hits.hits` of the matches in shown rows, in view order; built for
    /// `nav_gen`'s layout.
    pub nav: Vec<u32>,
    pub nav_gen: u64,
    /// Position in `nav` of the current match.
    pub idx: Option<usize>,
    pub secs: f64,
}

/// The inputs row statistics depend on: inserts hidden, gappy threshold, reference row.
type StatsKey = (bool, Option<u8>, Option<usize>);

/// Where a tree comes from.
pub enum TreeSource {
    File(PathBuf),
    /// Stockholm `#=GF NH` lines.
    Embedded(String),
}

/// A clickable part of the tree panel (window coordinates) and its node.
#[derive(Clone, Copy, Debug)]
pub struct TreeHit {
    pub bounds: Bounds<Pixels>,
    pub node: u32,
}

/// The overview panel's image, cached until what it shows changes.
#[derive(Default)]
pub struct OverviewCache {
    pub key: Option<(ColorMode, scheme::Theme, u64, u32, u32)>,
    pub image: Option<Arc<RenderImage>>,
}

/// What a mouse drag does.
#[derive(Clone, Debug)]
pub(super) enum Drag {
    /// Pan the alignment: the last pointer position, the cell pressed, and whether the pointer
    /// has moved (a press and release without moving is a click: it places the cursor).
    Pan { last: Point<Pixels>, cell: Option<(usize, usize)>, moved: bool },
    /// Shift+drag: select the block from `anchor` (view row, view column) to the pointer.
    Block { anchor: (usize, usize) },
    /// Drag over names: select the rows from `anchor` (a view row), added to `base`.
    Rows { anchor: usize, base: RangeSet },
    /// Drag over the track header: select the columns from `anchor` (a view column).
    Cols { anchor: usize, base: RangeSet },
    /// Resize the name panel: pointer x and panel width when the drag started.
    Splitter { x0: Pixels, w0: f32 },
    /// Resize the tree panel.
    TreeSplitter { x0: Pixels, w0: f32 },
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
    tree: bool,
    tree_order: bool,
    tree_panel: bool,
    has_selection: bool,
    export: ExportOptions,
    fig: super::figure::FigOptions,
    sort: Option<Sort>,
    filter: bool,
    find_mode: FindMode,
    reference: Option<usize>,
    reference_candidate: Option<usize>,
    ref_opts: RefOpts,
    hidden_tracks: Vec<Box<str>>,
}

pub struct MsaScope {
    pub(super) data: Option<Arc<Loaded>>,
    /// Messages for the status bar ("3 matches", "Out of range …").
    pub(super) status: SharedString,
    pub(super) error: Option<SharedString>,
    /// The load running, if any (one at a time); the open alignment stays until it ends.
    pub(super) jobs: Jobs,
    /// The file loading is Clustal or Stockholm, read in two passes (for the progress bar).
    pub(super) load_two_pass: bool,
    /// The open alignment's file.
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
    pub(super) prev_scheme: Scheme,
    pub(super) tiles: Rc<RefCell<TileCache>>,
    /// Hidden columns and tree order in effect (`None`: every column, file order), and the
    /// generation of the last rebuild asked for; a rebuild runs in the background and the old
    /// layout stays until it ends.
    pub(super) layout: Option<Arc<Layout>>,
    layout_gen: u64,
    pub(super) layout_pending: bool,
    /// The tree, matched to the alignment's rows.
    pub(super) tree: Option<Arc<TreeIndex>>,
    pub(super) tree_name: Option<SharedString>,
    pub(super) tree_loading: bool,
    /// Collapsed nodes, one flag per node.
    pub(super) collapsed: Vec<bool>,
    /// Rows in tree order (otherwise the file's order); only with a tree.
    pub(super) tree_order: bool,
    /// Rows sorted by something other than the tree or file order (tree order is off meanwhile).
    pub(super) sort: Option<Sort>,
    pub(super) filter: Filter,
    /// The filter bar is open (it also shows while a filter is set).
    pub(super) filter_bar: bool,
    pub(super) filter_name: Entity<InputState>,
    pub(super) filter_ident: Entity<InputState>,
    pub(super) filter_gaps: Entity<InputState>,
    /// The clade the clade filter shows, for the filter bar.
    pub(super) clade_label: Option<SharedString>,
    /// Empty the filter boxes at the next frame (it needs the window).
    pub(super) clear_inputs: bool,
    /// Scroll to the first row when the layout being built arrives (after a sort or filter).
    pub(super) scroll_top: bool,
    /// The reference row (alignment row) for identity sorting and filtering.
    pub(super) reference: Option<usize>,
    /// The reference row's codes and residue numbers.
    pub(super) ref_row: Option<Arc<Reference>>,
    pub(super) ref_opts: RefOpts,
    /// Annotation tracks switched off: `#=GC` tags, and `GR <tag>` for the reference's lines.
    pub(super) hidden_tracks: Vec<Box<str>>,
    pub(super) header_tiles: Rc<RefCell<HeaderTiles>>,
    /// The header band under the pointer.
    pub(super) hover_band: Option<super::bands::HeaderRow>,
    row_stats: Option<(StatsKey, Arc<RowStats>)>,
    pub(super) find_mode: FindMode,
    pub(super) motif: Option<MotifState>,
    pub(super) motif_cancel: Arc<std::sync::atomic::AtomicBool>,
    /// Where the pointer was last seen anywhere in the window (for the tree's context menu).
    pub(super) last_pointer: Rc<Cell<Point<Pixels>>>,
    /// A tree file to match once the alignment being opened has loaded (`--tree`, or a tree
    /// dropped or opened before or with it).
    pub(super) next_tree: Option<PathBuf>,
    /// The tree panel's clickable nodes, written by the canvas each frame.
    pub(super) tree_hits: Rc<RefCell<Vec<TreeHit>>>,
    pub(super) hide_inserts: bool,
    pub(super) hide_gappy: bool,
    pub(super) overview: Rc<RefCell<OverviewCache>>,
    /// The overview image's bounds (window coordinates), written by its canvas each layout.
    pub(super) overview_area: Rc<Cell<Bounds<Pixels>>>,
    /// The pointer, while it is over the alignment or the track header.
    pub(super) pointer: Option<Point<Pixels>>,
    /// The view column under the pointer in the track header.
    pub(super) hover_track: Option<usize>,
    /// The alignment area (window coordinates), written by the canvas each layout.
    pub(super) area: Rc<Cell<Bounds<Pixels>>>,
    pub(super) hover: Option<(usize, usize)>,
    /// The row under the pointer in the name panel.
    pub(super) hover_name: Option<usize>,
    pub(super) drag: Option<Drag>,
    pub(super) matches: Vec<usize>,
    pub(super) match_idx: usize,
    /// Selected rows and columns, in alignment coordinates.
    pub(super) sel: Rc<Selection>,
    /// The cell cursor (view row, view column), moved by the arrow keys or placed by a click.
    pub(super) cursor: Option<(usize, usize)>,
    /// Where Shift+arrows and Shift+click extend a selection from (view row, view column).
    pub(super) anchor: Option<(usize, usize)>,
    /// The hover card shows the cursor's cell (after a key moved it) instead of the pointer's.
    pub(super) card_cursor: bool,
    pub(super) export: ExportOptions,
    /// Names, ruler and PNG scale for figures.
    pub(super) fig: super::figure::FigOptions,
    pub(super) jump: Entity<InputState>,
    pub(super) search: Entity<InputState>,
    pub(super) focus: FocusHandle,
    pub(super) bench: Option<Bench>,
    pub(super) settings: Settings,
    title_bar: Entity<CachedView<MsaScope>>,
    toolbar: Entity<CachedView<MsaScope>>,
    chrome_key: Option<ChromeKey>,
    pub(super) card_hidden: bool,
    _subs: Vec<Subscription>,
}

impl MsaScope {
    pub fn new(path: Option<PathBuf>, tree: Option<PathBuf>, ty: Option<SeqType>, bench: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let jump = cx.new(|cx| InputState::new(window, cx).placeholder("Column, row:col or name:pos"));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find sequence"));
        let filter_name = cx.new(|cx| InputState::new(window, cx).placeholder("Name or accession, or /regex/"));
        let filter_ident = cx.new(|cx| InputState::new(window, cx).placeholder("0–100"));
        let filter_gaps = cx.new(|cx| InputState::new(window, cx).placeholder("0–100"));
        let subs = vec![
            cx.subscribe_in(&jump, window, |this, state, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { .. } => {
                    let text = state.read(cx).value().to_string();
                    this.go_to(&text, cx);
                }
                InputEvent::Blur => this.refocus(window, cx),
                _ => {}
            }),
            cx.subscribe_in(&search, window, |this, state, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => {
                    let q = state.read(cx).value().to_string();
                    match this.find_mode {
                        FindMode::Names => this.run_search(&q, cx),
                        FindMode::Motif => this.run_motif(&q, cx),
                    }
                }
                InputEvent::PressEnter { shift, .. } => match this.find_mode {
                    FindMode::Names => this.next_match(if *shift { -1 } else { 1 }, cx),
                    FindMode::Motif => this.next_hit(if *shift { -1 } else { 1 }, cx),
                },
                InputEvent::Blur => this.refocus(window, cx),
                _ => {}
            }),
            cx.subscribe_in(&filter_name, window, |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => this.read_filter(cx),
                InputEvent::Blur => this.refocus(window, cx),
                _ => {}
            }),
            cx.subscribe_in(&filter_ident, window, |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => this.read_filter(cx),
                InputEvent::Blur => this.refocus(window, cx),
                _ => {}
            }),
            cx.subscribe_in(&filter_gaps, window, |this, _, ev: &InputEvent, window, cx| match ev {
                InputEvent::Change => this.read_filter(cx),
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
            jobs: Jobs::default(),
            load_two_pass: false,
            path: None,
            ty,
            vp: Viewport { col0: 0.0, row0: 0.0, cw: view::LETTERS_CW, ch: view::LETTERS_CH, width: 0.0, height: 0.0 },
            pending: None,
            fit: None,
            mode: ColorMode::new(Scheme::ClustalX),
            prev_scheme: Scheme::ClustalX,
            tiles: Rc::default(),
            layout: None,
            layout_gen: 0,
            layout_pending: false,
            tree: None,
            tree_name: None,
            tree_loading: false,
            collapsed: Vec::new(),
            tree_order: true,
            sort: None,
            filter: Filter::default(),
            filter_bar: false,
            filter_name,
            filter_ident,
            filter_gaps,
            clade_label: None,
            clear_inputs: false,
            scroll_top: false,
            reference: None,
            ref_row: None,
            ref_opts: RefOpts::default(),
            hidden_tracks: Vec::new(),
            header_tiles: Rc::default(),
            hover_band: None,
            row_stats: None,
            find_mode: FindMode::Names,
            motif: None,
            motif_cancel: Arc::default(),
            last_pointer: Rc::new(Cell::new(Point::default())),
            next_tree: tree,
            tree_hits: Rc::default(),
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
            sel: Rc::default(),
            cursor: None,
            anchor: None,
            card_cursor: false,
            export: ExportOptions::default(),
            fig: super::figure::FigOptions::default(),
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
        match path {
            Some(p) => v.open(p, cx),
            None => {
                if let Some(t) = v.next_tree.take() {
                    v.open_any(t, cx);
                }
            }
        }
        v
    }

    /// The file being read, while a load runs.
    pub(super) fn loading(&self) -> Option<&std::path::Path> {
        self.jobs.current().map(|j| j.path.as_path())
    }

    /// Open an alignment in the background, showing its progress (polled every 100 ms) until it
    /// ends or is cancelled. The alignment open now stays until the new one has loaded.
    pub(super) fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.status = SharedString::default();
        self.error = None;
        let total = std::fs::metadata(&path).map_or(0, |m| m.len());
        self.load_two_pass = msascope::parse::sniff(&path).is_ok_and(|(f, _)| f != msascope::parse::Format::Fasta);
        let job = self.jobs.start(path.clone(), total);
        let (id, progress) = (job.id, job.progress.clone());
        cx.notify();
        let ty = self.ty;
        let task = cx.background_executor().spawn(async move { Loaded::open_with(&path, ty, Some(&progress)) });
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(std::time::Duration::from_millis(100)).await;
                let running = this.update(cx, |v, cx| {
                    let on = v.jobs.is_current(id);
                    if on {
                        cx.notify();
                    }
                    on
                });
                if !matches!(running, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |v, cx| {
                let Done::Current(path, result) = v.jobs.finish(id, result) else { return };
                match result {
                    Ok(l) => {
                        v.settings.add_recent(&path);
                        v.settings.save();
                        v.path = Some(path);
                        eprintln!("msascope: {}", l.summary());
                        if let Some(a) = l.annotation_summary() {
                            eprintln!("msascope: {a}");
                        }
                        for w in &l.aln.warnings {
                            eprintln!("msascope: warning: {w}");
                        }
                        let warn = match l.aln.warnings.as_slice() {
                            [] => None,
                            [w] => Some(w.clone()),
                            [w, rest @ ..] => Some(format!("{w} (and {} more, see the terminal)", rest.len())),
                        };
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
                        let embedded = l.aln.newick.clone();
                        v.data = Some(Arc::new(l));
                        v.layout = None;
                        v.clear_tree();
                        v.pending = Some(Fit::Readable);
                        v.vp.col0 = 0.0;
                        v.vp.row0 = 0.0;
                        v.matches.clear();
                        v.reset_selection();
                        v.reset_rows(cx);
                        v.hover = None;
                        v.hidden_tracks.clear();
                        if let Some(w) = warn {
                            v.status = w.into();
                        }
                        // The tree: one given with the file, else the file's own (#=GF NH), else
                        // a sibling file with the same stem.
                        let source = v.next_tree.take().map(TreeSource::File).or(embedded.map(TreeSource::Embedded)).or_else(|| {
                            v.path.as_deref().and_then(|p| tree::sibling_candidates(p).into_iter().find(|c| c.is_file())).map(TreeSource::File)
                        });
                        match source {
                            Some(s) => v.load_tree(s, cx),
                            None => v.rebuild_layout(cx),
                        }
                    }
                    Err(e) => {
                        eprintln!("msascope: {e:#}");
                        v.data = None;
                        v.close(cx);
                        v.path = Some(path);
                        v.error = Some(format!("{e:#}").into());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Cancel / Escape while loading: stop the load (its memory is freed when the reader returns),
    /// keep the alignment that was open.
    pub(super) fn cancel_load(&mut self, cx: &mut Context<Self>) {
        if let Some(p) = self.jobs.cancel() {
            let name = p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned());
            eprintln!("msascope: cancelled loading {name}");
            self.status = format!("Cancelled loading {name}").into();
            cx.notify();
        }
    }

    /// Close the file: back to the empty window.
    pub(super) fn close(&mut self, cx: &mut Context<Self>) {
        self.jobs.cancel();
        self.data = None;
        self.layout = None;
        self.layout_gen += 1;
        self.layout_pending = false;
        self.clear_tree();
        self.path = None;
        self.error = None;
        self.status = SharedString::default();
        self.matches.clear();
        self.reset_selection();
        self.reset_rows(cx);
        self.hover = None;
        cx.notify();
    }

    /// Forget sorting, filters, the reference row and motif hits (a new file, or none).
    fn reset_rows(&mut self, cx: &mut Context<Self>) {
        self.sort = None;
        self.filter = Filter::default();
        self.filter_bar = false;
        self.reference = None;
        self.ref_row = None;
        self.row_stats = None;
        self.cancel_motif();
        self.motif = None;
        self.clade_label = None;
        self.clear_inputs = true;
        cx.notify();
    }

    pub(super) fn reset_selection(&mut self) {
        self.sel = Rc::default();
        self.cursor = None;
        self.anchor = None;
        self.card_cursor = false;
    }

    fn clear_tree(&mut self) {
        self.filter.clade = None;
        self.tree = None;
        self.tree_name = None;
        self.tree_loading = false;
        self.collapsed.clear();
        self.tree_order = true;
    }

    /// Open a file the user chose or dropped: a tree is matched to the open alignment (or kept
    /// for the next one), anything else is opened as an alignment.
    pub(super) fn open_any(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        let mut head = [0u8; 256];
        let n = std::fs::File::open(&path).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
        if tree::looks_like_tree(&path, &head[..n]) {
            if self.data.is_some() && self.loading().is_none() {
                self.load_tree(TreeSource::File(path), cx);
            } else {
                self.status = format!("{} will be matched to the next alignment opened", path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().into_owned())).into();
                self.next_tree = Some(path);
                cx.notify();
            }
        } else {
            self.open(path, cx);
        }
    }

    /// Several dropped files: alignments first, so a tree dropped with one is matched to it.
    pub(super) fn open_dropped(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let mut trees = Vec::new();
        let mut alns = Vec::new();
        for p in paths {
            let mut head = [0u8; 256];
            let n = std::fs::File::open(p).and_then(|mut f| std::io::Read::read(&mut f, &mut head)).unwrap_or(0);
            if tree::looks_like_tree(p, &head[..n]) { trees.push(p.clone()) } else { alns.push(p.clone()) }
        }
        match (alns.into_iter().next(), trees.into_iter().next()) {
            (Some(a), t) => {
                if t.is_some() {
                    self.next_tree = t;
                }
                self.open(a, cx);
            }
            (None, Some(t)) => self.open_any(t, cx),
            (None, None) => {}
        }
    }

    /// Read, parse and match a tree in the background, then show rows in its order.
    pub(super) fn load_tree(&mut self, source: TreeSource, cx: &mut Context<Self>) {
        let Some(d) = self.data.clone() else { return };
        let name: SharedString = match &source {
            TreeSource::File(p) => p.file_name().map_or_else(|| p.display().to_string(), |n| n.to_string_lossy().into_owned()).into(),
            TreeSource::Embedded(_) => "tree in the file (#=GF NH)".into(),
        };
        self.tree_loading = true;
        self.status = format!("Reading {name}…").into();
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            let t0 = Instant::now();
            let text = match source {
                TreeSource::File(p) => std::fs::read_to_string(&p).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))?,
                TreeSource::Embedded(s) => s,
            };
            let t = tree::Tree::parse(&text)?;
            let parse = t0.elapsed();
            let t1 = Instant::now();
            let ix = TreeIndex::new(t, &d.aln.names, &d.aln.accessions);
            anyhow::Ok((ix, parse, t1.elapsed()))
        });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |v, cx| {
                v.tree_loading = false;
                match result {
                    Ok((ix, parse, matching)) => {
                        let report = ix.matching.report();
                        eprintln!(
                            "msascope: tree {name}: {} nodes, {} leaves; parse {:.3} s, match + index {:.3} s, {} MiB; {report}",
                            ix.tree.len(),
                            ix.matching.leaves,
                            parse.as_secs_f64(),
                            matching.as_secs_f64(),
                            ix.bytes() >> 20
                        );
                        if !ix.matching.unmatched_leaves.is_empty() {
                            eprintln!("msascope: leaves not in the alignment: {}", ix.matching.unmatched_leaves.join(", "));
                        }
                        if ix.matching.matched == 0 {
                            v.status = format!("No leaf of {name} matches a sequence name; tree not used").into();
                            v.rebuild_layout(cx);
                            return;
                        }
                        v.status = format!("{name}: {report}").into();
                        v.collapsed = vec![false; ix.tree.len()];
                        v.tree = Some(Arc::new(ix));
                        v.tree_name = Some(name);
                        v.tree_order = true;
                    }
                    Err(e) => {
                        eprintln!("msascope: tree {name}: {e:#}");
                        v.status = format!("Couldn't read {name}: {e:#}").into();
                    }
                }
                v.rebuild_layout(cx);
            })
            .ok();
        })
        .detach();
    }

    pub(super) fn remove_tree(&mut self, cx: &mut Context<Self>) {
        self.clear_tree();
        self.status = SharedString::default();
        self.rebuild_layout(cx);
    }

    /// Collapse or expand a node of the tree.
    pub(super) fn toggle_clade(&mut self, node: u32, cx: &mut Context<Self>) {
        let Some(f) = self.collapsed.get_mut(node as usize) else { return };
        *f = !*f;
        self.rebuild_layout(cx);
    }

    pub(super) fn set_tree_order(&mut self, on: bool, cx: &mut Context<Self>) {
        self.tree_order = on;
        self.sort = None;
        self.rebuild_layout(cx);
    }

    /// Collapsed clades shown.
    pub(super) fn collapsed_count(&self) -> usize {
        self.row_view().map_or(0, RowView::clades)
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
        let rx = cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None });
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await {
                this.update(cx, |v, cx| v.open_dropped(&paths, cx)).ok();
            }
        })
        .detach();
    }

    /// Give keyboard focus back to the alignment when a text box lets go of it (Escape).
    pub(super) fn refocus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.focused(cx).is_none() {
            self.focus.focus(window, cx);
        }
    }

    /// Shown (view) rows and columns.
    pub(super) fn dims(&self) -> (usize, usize) {
        self.data.as_ref().map_or((0, 0), |d| (view::view_rows(self.row_view(), d.aln.rows()), view::view_cols(self.col_map(), d.aln.cols())))
    }

    pub(super) fn col_map(&self) -> Option<&ColumnMap> {
        self.layout.as_deref().and_then(|l| l.map.as_ref())
    }

    pub(super) fn row_view(&self) -> Option<&RowView> {
        self.layout.as_deref().and_then(|l| l.rows.as_ref())
    }

    /// The alignment row shown at view row `v` (a collapsed clade's first member).
    pub(super) fn aln_row(&self, v: usize) -> usize {
        self.row_view().map_or(v, |m| m.rows_of(v)[0] as usize)
    }

    /// The view row showing alignment row `r`; `None` when a filter hides it.
    pub(super) fn view_row(&self, r: usize) -> Option<usize> {
        match self.row_view() {
            None => Some(r),
            Some(m) => m.shown(r),
        }
    }

    /// Sequences shown and in the file, when a filter hides some.
    pub(super) fn shown_of(&self) -> Option<(usize, usize)> {
        let d = self.data.as_ref()?;
        self.layout.as_deref().and_then(|l| l.shown).map(|n| (n, d.aln.rows()))
    }

    /// Whether the tree panel is shown: a tree in tree order, with the panel switched on.
    pub(super) fn show_tree(&self) -> bool {
        self.settings.tree_panel && self.layout.as_deref().is_some_and(|l| l.tree.is_some())
    }

    /// Width of the tree panel (0 when hidden) and of the tree and name panels together.
    pub(super) fn tree_w(&self) -> f32 {
        if self.show_tree() { self.settings.tree_width } else { 0.0 }
    }

    /// The reference the views compare with: set, and "compare" on.
    pub(super) fn compare_base(&self) -> Option<Arc<Reference>> {
        self.ref_row.clone().filter(|_| self.ref_opts.compare)
    }

    pub(super) fn left_w(&self) -> f32 {
        self.tree_w() + self.settings.name_width
    }

    /// Columns hidden now.
    pub(super) fn hidden_count(&self) -> usize {
        self.data.as_ref().map_or(0, |d| d.aln.cols()) - self.dims().1
    }

    /// Rebuild the column map, the row view and the pyramid for them in the background after a
    /// change (hidden columns, tree, collapse, sort); the view keeps the old layout until the new
    /// one is ready, then keeps the column at its left edge, the row at its top and the cursor on
    /// the same cell.
    pub(super) fn rebuild_layout(&mut self, cx: &mut Context<Self>) {
        let Some(d) = self.data.clone() else { return };
        self.layout_gen += 1;
        let generation = self.layout_gen;
        let inserts = self.hide_inserts && !d.aln.inserts.is_empty();
        let gappy = self.hide_gappy.then(|| self.settings.gappy_pct as f32 / 100.0);
        let tree = self.tree.clone().filter(|_| self.tree_order);
        let (sort, filter, reference) = (self.sort, self.filter.clone(), self.reference);
        let base = self.compare_base();
        if !inserts && gappy.is_none() && tree.is_none() && sort.is_none() && !filter.is_active() && base.is_none() && self.layout.is_none() {
            self.layout_pending = false;
            return;
        }
        let need_stats = sort.is_some_and(|s| s.key.needs_stats()) || filter.needs_stats();
        let key: StatsKey = (inserts, self.hide_gappy.then_some(self.settings.gappy_pct), reference);
        let cached = self.row_stats.as_ref().filter(|(k, _)| *k == key).map(|(_, s)| s.clone());
        let collapsed = self.collapsed.clone();
        self.layout_pending = true;
        cx.notify();
        let task = cx.background_executor().spawn(async move {
            let t0 = Instant::now();
            let map = ColumnMap::hiding(&d.aln, &d.stats, inserts, gappy);
            let stats = need_stats.then(|| cached.unwrap_or_else(|| Arc::new(RowStats::compute(&d.aln, &d.stats.consensus, map.as_ref(), reference))));
            let keep = filter.keep(&d.aln, stats.as_deref());
            let t_rows = t0.elapsed();
            let (rows, tree_rows) = match (sort, &tree) {
                (Some(s), _) => (Some(msascope::rows::flat_view(&d.aln, keep.as_deref(), Some(s), stats.as_deref())), None),
                (None, Some(ix)) => {
                    let (rv, tr) = tree::row_view_kept(&d.aln, ix, &collapsed, keep.as_deref());
                    (Some(rv), tr)
                }
                (None, None) if keep.is_some() => (Some(msascope::rows::flat_view(&d.aln, keep.as_deref(), None, None)), None),
                (None, None) => (None, None),
            };
            let shown = keep.as_ref().map(|k| k.iter().filter(|&&x| x).count());
            let t_order = t0.elapsed() - t_rows;
            let layout = (map.is_some() || rows.is_some() || base.is_some()).then(|| {
                let against = base.as_deref().map_or(&d.stats.consensus[..], |b| &b.codes[..]);
                let pyramid = Pyramid::build_mapped(&d.aln, against, map.as_ref().map(|m| &m.visible[..]), rows.as_ref());
                Arc::new(Layout { map, rows, tree: tree_rows, pyramid, generation, shown, base })
            });
            (layout, stats, (t_rows, t_order, t0.elapsed()))
        });
        cx.spawn(async move |this, cx| {
            let (layout, stats, (t_rows, t_order, took)) = task.await;
            this.update(cx, |v, cx| {
                if let Some(s) = stats {
                    v.row_stats = Some((key, s));
                }
                if v.layout_gen != generation {
                    return;
                }
                if layout.as_ref().is_some_and(|l| l.rows.is_some()) {
                    eprintln!(
                        "msascope: layout {:.3} s (row figures and filter {:.3} s, row order {:.3} s, then pyramid)",
                        took.as_secs_f64(),
                        t_rows.as_secs_f64(),
                        t_order.as_secs_f64()
                    );
                }
                let (rows, cols) = v.dims();
                // What is at the top-left and under the cursor, in alignment coordinates.
                let left = (cols > 0).then(|| view::aln_col(v.col_map(), (v.vp.col0.max(0.0) as usize).min(cols - 1)));
                let top = (rows > 0).then(|| v.aln_row((v.vp.row0.max(0.0) as usize).min(rows - 1)));
                let cursor = v.cursor.filter(|&(r, c)| r < rows && c < cols).map(|(r, c)| (v.aln_row(r), view::aln_col(v.col_map(), c)));
                v.layout = layout;
                v.layout_pending = false;
                let to_view_col = |v: &MsaScope, c: usize| v.col_map().map_or(c, |m| m.view_of(c).unwrap_or_else(|e| e));
                if let Some(left) = left {
                    v.vp.col0 = to_view_col(v, left) as f64 + v.vp.col0.fract();
                }
                if std::mem::take(&mut v.scroll_top) {
                    v.vp.row0 = 0.0;
                } else if let Some(top) = top {
                    v.vp.row0 = v.view_row(top).map_or(0.0, |r| r as f64 + v.vp.row0.fract());
                }
                let (rows, cols) = v.dims();
                v.cursor = cursor.and_then(|(r, c)| Some((v.view_row(r)?.min(rows.saturating_sub(1)), to_view_col(v, c).min(cols.saturating_sub(1)))));
                v.anchor = None;
                v.vp.clamp(rows, cols);
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
        self.rebuild_layout(cx);
    }

    /// Hide columns with at least `pct`% gaps (`None`: show them).
    pub(super) fn set_gappy(&mut self, pct: Option<u8>, cx: &mut Context<Self>) {
        self.hide_gappy = pct.is_some();
        if let Some(p) = pct {
            self.settings.gappy_pct = p;
            self.settings.save();
        }
        self.rebuild_layout(cx);
    }

    pub(super) fn set_scheme(&mut self, s: Scheme) {
        if s != Scheme::Differences {
            self.prev_scheme = s;
        }
        self.mode.scheme = s;
    }

    /// The next scheme that applies to the open alignment (`s`).
    pub(super) fn cycle_scheme(&mut self) {
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
    pub(super) fn set_scale(&mut self, cw: f64, ch: f64, anchor: Option<(f64, f64)>) {
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

    pub(super) fn pan(&mut self, dx_px: f64, dy_px: f64) {
        self.vp.col0 += dx_px / self.vp.cw;
        self.vp.row0 += dy_px / self.vp.ch;
        let (r, c) = self.dims();
        self.vp.clamp(r, c);
    }

    /// Scroll so that cell (row, col) is in view, roughly centred.
    pub(super) fn center_on(&mut self, row: Option<usize>, col: Option<usize>) {
        if let Some(c) = col {
            self.vp.col0 = c as f64 - self.vp.width / self.vp.cw / 2.0;
        }
        if let Some(r) = row {
            self.vp.row0 = r as f64 - self.vp.height / self.vp.ch / 2.0;
        }
        let (rows, cols) = self.dims();
        self.vp.clamp(rows, cols);
    }

    pub(super) fn run_search(&mut self, q: &str, cx: &mut Context<Self>) {
        let Some(d) = self.data.clone() else { return };
        let mut m = view::search_names(&d.aln.names, q);
        m.retain(|&r| self.view_row(r).is_some());
        self.matches = m;
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

    /// Select alignment row `row` (as a click on its name does) and scroll it into view; a row a
    /// filter hides is selected without scrolling.
    pub(super) fn select_row(&mut self, row: usize) {
        let Some(r) = self.view_row(row) else {
            let mut rows = RangeSet::new();
            rows.add(row as u32, row as u32 + 1);
            self.sel = Rc::new(Selection { rows, ..Default::default() });
            self.status = "That sequence is hidden by the row filter".into();
            return;
        };
        self.sel = Rc::new(Selection { rows: self.rows_set(r..=r), ..Default::default() });
        self.anchor = Some((r, self.cursor.map_or(0, |c| c.1)));
        if self.vp.ch < NAME_ROW_MIN {
            self.fit = None;
            self.set_scale(self.vp.cw, view::LETTERS_CH, None);
        }
        let visible = self.vp.row0 <= r as f64 && (r as f64) < self.vp.row0 + self.vp.height / self.vp.ch - 1.0;
        if !visible {
            self.center_on(Some(r), None);
        }
    }

    fn step_bench(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(b) = &mut self.bench else { return };
        // Wait for the file, the window size, and a tree and layout still being built.
        if self.data.is_none() || self.vp.width <= 0.0 || self.pending.is_some() || self.tree_loading || self.layout_pending {
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
            loading: self.loading().is_some(),
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
            tree: self.tree.is_some(),
            tree_order: self.tree_order,
            tree_panel: self.settings.tree_panel,
            has_selection: !self.sel.is_empty(),
            export: self.export,
            fig: self.fig,
            sort: self.sort,
            filter: self.filter.is_active() || self.filter_bar,
            find_mode: self.find_mode,
            reference: self.reference,
            reference_candidate: self.reference_candidate(),
            ref_opts: self.ref_opts,
            hidden_tracks: self.hidden_tracks.clone(),
        }
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
        if std::mem::take(&mut self.clear_inputs) {
            for s in [&self.filter_name, &self.filter_ident, &self.filter_gaps] {
                s.update(cx, |s, cx| s.set_value("", window, cx));
            }
        }

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
        let main = if self.data.is_some() && self.loading().is_none() {
            let tree_w = px(self.tree_w());
            let name_w = px(self.settings.name_width);
            let splitter = |id: &'static str, left: Pixels, active: bool, tree: bool| {
                div()
                    .id(id)
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(left - px(SPLITTER_W / 2.0))
                    .w(px(SPLITTER_W))
                    .cursor_col_resize()
                    .hover(|s| s.bg(t.border_strong))
                    .when(active, |d| d.bg(t.accent))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |v, ev: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            v.drag = Some(if tree {
                                Drag::TreeSplitter { x0: ev.position.x, w0: v.settings.tree_width }
                            } else {
                                Drag::Splitter { x0: ev.position.x, w0: v.settings.name_width }
                            });
                            cx.notify();
                        }),
                    )
            };
            let tree_split = self.show_tree().then(|| splitter("tree-splitter", tree_w, matches!(self.drag, Some(Drag::TreeSplitter { .. })), true));
            let name_split = splitter("name-splitter", tree_w + name_w, matches!(self.drag, Some(Drag::Splitter { .. })), false);
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
                // The tree's context menu, over the tree panel.
                .when(self.show_tree(), |d| d.child(self.tree_menu_overlay(tree_w, cx)))
                // The name panel's context menu (the reference row).
                .child(self.name_menu_overlay(tree_w, name_w, cx))
                // The splitters, over the right edges of the tree and name panels.
                .children(tree_split)
                .child(name_split);
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
            .id("root")
            .size_full()
            .bg(t.bg)
            .text_color(t.fg)
            .on_mouse_move(cx.listener(|v, ev: &MouseMoveEvent, _, _| v.last_pointer.set(ev.position)))
            .on_key_down(cx.listener(|v, ev: &KeyDownEvent, window, cx| {
                let k = &ev.keystroke;
                let ctrl = k.modifiers.control || k.modifiers.platform;
                match k.key.as_str() {
                    "escape" => {
                        if v.loading().is_some() {
                            v.cancel_load(cx);
                        }
                        v.focus.focus(window, cx);
                    }
                    // The same shortcuts as on the alignment, while a text box has the focus.
                    _ if v.data.is_none() => return,
                    "g" if ctrl => v.jump.update(cx, |s, cx| s.focus(window, cx)),
                    "f" if ctrl && k.modifiers.shift => v.set_find_mode(FindMode::Motif, window, cx),
                    "f" if ctrl => v.search.update(cx, |s, cx| s.focus(window, cx)),
                    "l" if ctrl => v.toggle_filter_bar(window, cx),
                    "f3" if v.motif.is_some() => v.next_hit(if k.modifiers.shift { -1 } else { 1 }, cx),
                    _ => return,
                }
                cx.notify();
            }))
            // Files dropped anywhere on the window: alignments and trees.
            .drag_over::<ExternalPaths>(move |s, _, _, _| s.border_2().border_color(t.accent))
            .on_drop(cx.listener(|v, paths: &ExternalPaths, _, cx| {
                v.open_dropped(paths.paths(), cx);
            }))
            .child(title)
            .child(tools)
            .when(self.data.is_some() && self.loading().is_none() && (self.filter_bar || self.filter.is_active()), |d| d.child(self.render_filter_bar(cx)))
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
