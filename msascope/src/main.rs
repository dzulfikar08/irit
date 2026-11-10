use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use msascope::Loaded;
use msascope::alphabet::SeqType;
use msascope::view::{self, ColorMode, Scene, TILE, Viewport};
use rayon::prelude::*;

// Phase 0 ports the design system; Phase 1 uses the rest of it.
#[allow(dead_code)]
mod gui;
use gui::theme::{Settings, ThemeChoice, is_dark};

const NAMES_W: f32 = 200.0;
const RULER_H: f32 = 20.0;
const CONS_H: f32 = 14.0;
const CONS_BAR_H: f32 = 26.0;
const HEADER_H: f32 = RULER_H + CONS_H + CONS_BAR_H;
const ZOOM_MIN: i32 = -36;
const ZOOM_MAX: i32 = 10;
/// Letters are drawn once cells are at least this wide/tall (logical px).
const LETTER_W: f64 = 8.0;
const LETTER_H: f64 = 10.0;

fn color(c: u32) -> Hsla {
    rgb(c).into()
}

/// Cached rasterised tiles for the current zoom and colour mode, keyed by tile position.
#[derive(Default)]
struct TileCache {
    key: Option<(i32, i32, ColorMode, u32)>,
    tiles: HashMap<(i64, i64), (Arc<RenderImage>, u64)>,
    frame: u64,
    rendered: usize,
}

impl TileCache {
    fn reset(&mut self, window: &mut Window) {
        for (_, (img, _)) in self.tiles.drain() {
            let _ = window.drop_image(img);
        }
    }

    fn evict(&mut self, keep: usize, window: &mut Window) {
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

struct Bench {
    phase: usize,
    frame: usize,
    last: Option<Instant>,
    intervals: Vec<f64>,
    paint_ms: Rc<RefCell<Vec<f64>>>,
}

/// (name, zoom x, zoom y) phases of `--bench`; zoom `None` means "fit the whole alignment".
const BENCH_PHASES: [(&str, Option<(i32, i32)>); 4] = [
    ("overview (fit)", None),
    ("1 px per cell", Some((0, 0))),
    ("4 px per cell", Some((4, 4))),
    ("letters, 12x16 px", Some((7, 8))),
];
const BENCH_FRAMES: usize = 240;

struct MsaScope {
    data: Option<Arc<Loaded>>,
    status: SharedString,
    path: Option<PathBuf>,
    /// `--type`: used for the file given on the command line and every file opened later.
    ty: Option<SeqType>,
    vp: Viewport,
    zx: i32,
    zy: i32,
    fit_pending: bool,
    /// Re-fit on window resize until the user scrolls or zooms.
    auto_fit: bool,
    mode: ColorMode,
    tiles: Rc<RefCell<TileCache>>,
    area: Rc<Cell<Bounds<Pixels>>>,
    hover: Option<(usize, usize)>,
    drag: Option<Point<Pixels>>,
    matches: Vec<usize>,
    match_idx: usize,
    selected_row: Option<usize>,
    jump: Entity<InputState>,
    search: Entity<InputState>,
    focus: FocusHandle,
    bench: Option<Bench>,
    settings: Settings,
    _subs: Vec<Subscription>,
}

impl MsaScope {
    fn new(path: Option<PathBuf>, ty: Option<SeqType>, bench: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let jump = cx.new(|cx| InputState::new(window, cx).placeholder("Go to column, or row:column"));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Find sequence name"));
        let mut subs = vec![
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
        ];
        // Follow the OS appearance while the theme is "System".
        subs.push(cx.observe_window_appearance(window, |v, window, cx| {
            if v.settings.theme == ThemeChoice::System {
                gui::tokens::apply_palette(ThemeChoice::System.palette(is_dark(window.appearance())), cx);
                cx.notify();
            }
        }));
        let mut v = MsaScope {
            data: None,
            status: "Open an alignment (FASTA, Clustal, Stockholm; .gz ok), or try a sample".into(),
            path: None,
            ty,
            vp: Viewport { col0: 0.0, row0: 0.0, cw: 1.0, ch: 1.0, width: 0.0, height: 0.0 },
            zx: 0,
            zy: 0,
            fit_pending: false,
            auto_fit: false,
            mode: ColorMode::Residues,
            tiles: Rc::default(),
            area: Rc::new(Cell::new(Bounds::default())),
            hover: None,
            drag: None,
            matches: Vec::new(),
            match_idx: 0,
            selected_row: None,
            jump,
            search,
            focus: cx.focus_handle(),
            bench: bench.then(|| Bench { phase: 0, frame: 0, last: None, intervals: Vec::new(), paint_ms: Rc::default() }),
            settings: Settings::load(),
            _subs: subs,
        };
        v.focus.focus(window, cx);
        if let Some(p) = path {
            v.open(p, cx);
        }
        v
    }

    fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.status = format!("Loading {}…", path.display()).into();
        self.path = Some(path.clone());
        cx.notify();
        let ty = self.ty;
        let task = cx.background_executor().spawn(async move { Loaded::open_as(&path, ty) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |v, cx| {
                match result {
                    Ok(l) => {
                        eprintln!("msascope: {}", l.summary());
                        if let Some(rss) = peak_rss_mb() {
                            eprintln!("msascope: peak RSS after load {rss} MiB");
                        }
                        v.status = l.summary().into();
                        v.data = Some(Arc::new(l));
                        v.fit_pending = true;
                        v.auto_fit = true;
                        v.matches.clear();
                        v.selected_row = None;
                    }
                    Err(e) => {
                        eprintln!("msascope: {e:#}");
                        v.status = format!("Error: {e:#}").into();
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Write a synthetic 2,000 × 3,000 nucleotide alignment (a viral-surveillance-like test set)
    /// to the temp folder and open it, so a first run has something to look at.
    fn open_sample(&mut self, cx: &mut Context<Self>) {
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
                    v.status = format!("Error: couldn't write the sample alignment: {e}").into();
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn prompt_open(&mut self, cx: &mut Context<Self>) {
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

    fn dims(&self) -> (usize, usize) {
        self.data.as_ref().map_or((0, 0), |d| (d.aln.rows(), d.aln.cols()))
    }

    fn set_zoom(&mut self, zx: i32, zy: i32, anchor: Option<(f64, f64)>) {
        self.auto_fit = false;
        self.zx = zx.clamp(ZOOM_MIN, ZOOM_MAX);
        self.zy = zy.clamp(ZOOM_MIN, ZOOM_MAX);
        let (x, y) = anchor.unwrap_or((self.vp.width / 2.0, self.vp.height / 2.0));
        self.vp.zoom_at(view::zoom_px(self.zx), view::zoom_px(self.zy), x, y);
        let (r, c) = self.dims();
        self.vp.clamp(r, c);
    }

    fn fit(&mut self) {
        let (rows, cols) = self.dims();
        let zx = view::fit_zoom(cols, self.vp.width, ZOOM_MIN, 7);
        let zy = view::fit_zoom(rows, self.vp.height, ZOOM_MIN, 8);
        self.vp.col0 = 0.0;
        self.vp.row0 = 0.0;
        self.set_zoom(zx, zy, Some((0.0, 0.0)));
        self.auto_fit = true;
    }

    fn pan(&mut self, dx_px: f64, dy_px: f64) {
        self.auto_fit = false;
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

    fn jump_to(&mut self, text: &str, cx: &mut Context<Self>) {
        let parse = |s: &str| s.trim().replace([',', '_'], "").parse::<usize>().ok().filter(|&n| n >= 1).map(|n| n - 1);
        let (row, col) = match text.split_once(':') {
            Some((r, c)) => (parse(r), parse(c)),
            None => (None, parse(text)),
        };
        let (rows, cols) = self.dims();
        if row.is_none_or(|r| r < rows) && col.is_none_or(|c| c < cols) && (row.is_some() || col.is_some()) {
            if self.zx < 7 && col.is_some() {
                self.set_zoom(7, self.zy.max(8), None);
            }
            self.center_on(row, col);
            self.selected_row = row.or(self.selected_row);
        } else {
            self.status = format!("Out of range: {rows} rows × {cols} columns").into();
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
        self.status = if q.is_empty() { d.summary().into() } else { format!("{} matches", self.matches.len()).into() };
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
        if self.zy < 6 {
            let zx = self.zx;
            self.set_zoom(zx, 8, None);
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
            "+" | "=" => self.set_zoom(self.zx + 1, self.zy + 1, None),
            "-" => self.set_zoom(self.zx - 1, self.zy - 1, None),
            "0" => self.fit(),
            "d" => self.toggle_mode(),
            "o" if ctrl => self.prompt_open(cx),
            "f" if ctrl => self.search.update(cx, |s, cx| s.focus(window, cx)),
            "g" if ctrl => self.jump.update(cx, |s, cx| s.focus(window, cx)),
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            ColorMode::Residues => ColorMode::Differences,
            ColorMode::Differences => ColorMode::Residues,
        };
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
            self.set_zoom(self.zx + sx, self.zy + sy, Some((x.max(0.0), y.max(0.0))));
        } else if ev.modifiers.shift && dx.abs() < 0.5 {
            self.pan(-dy, 0.0);
        } else {
            self.pan(-dx, -dy);
        }
        cx.notify();
    }

    fn update_hover(&mut self, pos: Point<Pixels>) {
        let area = self.area.get();
        let (rows, cols) = self.dims();
        self.hover = self.vp.hit(f64::from(pos.x - area.origin.x), f64::from(pos.y - area.origin.y), rows, cols);
    }

    fn hover_text(&self) -> String {
        let (Some(d), Some((r, c))) = (&self.data, self.hover) else { return String::new() };
        let code = d.aln.matrix.get(r, c);
        format!(
            "row {} · column {} · {} · consensus {} ({:.0}% identity) · {}",
            r + 1,
            c + 1,
            d.aln.symbol(code) as char,
            d.aln.symbol(d.stats.consensus[c]) as char,
            d.stats.identity[c] * 100.0,
            d.aln.names[r]
        )
    }

    fn step_bench(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(b) = &mut self.bench else { return };
        if self.data.is_none() || self.vp.width <= 0.0 || self.fit_pending {
            window.request_animation_frame();
            return;
        }
        let now = Instant::now();
        if b.frame == 0 {
            let (name, zoom) = BENCH_PHASES[b.phase];
            eprintln!("bench: phase {name}");
            match zoom {
                None => self.fit(),
                Some((zx, zy)) => {
                    self.vp.col0 = 0.0;
                    self.vp.row0 = 0.0;
                    self.set_zoom(zx, zy, Some((0.0, 0.0)));
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

    fn alignment_canvas(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let data = self.data.clone();
        let tiles = self.tiles.clone();
        let area_cell = self.area.clone();
        let (vp, zx, zy, mode) = (self.vp, self.zx, self.zy, self.mode);
        let selected = self.selected_row;
        let hover = self.hover;
        let theme = cx.theme();
        let mono: SharedString = theme.mono_font_family.clone();
        let (fg, muted, bg, panel, border) = (theme.foreground, theme.muted_foreground, theme.background, theme.secondary, theme.border);
        let paint_ms = self.bench.as_ref().map(|b| b.paint_ms.clone());
        let scale = window.scale_factor() as f64;
        canvas(
            move |bounds, window, _| {
                let area = Bounds::new(
                    point(bounds.origin.x + px(NAMES_W), bounds.origin.y + px(HEADER_H)),
                    size((bounds.size.width - px(NAMES_W)).max(px(0.)), (bounds.size.height - px(HEADER_H)).max(px(0.))),
                );
                // The view reads the area size in `render`, which ran before this layout; redraw on change.
                if area_cell.replace(area).size != area.size {
                    window.refresh();
                }
                area
            },
            move |bounds, area, window, cx| {
                let t0 = Instant::now();
                window.paint_quad(fill(bounds, bg));
                let Some(d) = data else { return };
                let (rows, cols) = (d.aln.rows(), d.aln.cols());
                let font = Font { family: mono, ..window.text_style().font() };
                let run = |len: usize, color: Hsla| TextRun { len, font: font.clone(), color, background_color: None, underline: None, strikethrough: None };

                // Device-pixel geometry, with the scroll offset snapped to whole pixels so tiles land on the pixel grid.
                let (cwd, chd) = (vp.cw * scale, vp.ch * scale);
                let off_x = (vp.col0 * cwd).round();
                let off_y = (vp.row0 * chd).round();
                let dev_vp = Viewport {
                    col0: off_x / cwd,
                    row0: off_y / chd,
                    cw: cwd,
                    ch: chd,
                    width: f64::from(area.size.width) * scale,
                    height: f64::from(area.size.height) * scale,
                };
                let to_x = |dev: f64| area.origin.x + px((dev / scale) as f32);
                let to_y = |dev: f64| area.origin.y + px((dev / scale) as f32);

                // 1. Alignment tiles.
                {
                    let mut cache = tiles.borrow_mut();
                    let key = (zx, zy, mode, (scale * 100.0) as u32);
                    if cache.key != Some(key) {
                        cache.reset(window);
                        cache.key = Some(key);
                    }
                    cache.frame += 1;
                    let frame = cache.frame;
                    let wanted: Vec<(i64, i64)> = dev_vp.tiles().collect();
                    let missing: Vec<(i64, i64)> = wanted.iter().filter(|k| !cache.tiles.contains_key(k)).copied().collect();
                    if !missing.is_empty() {
                        let scene = Scene { aln: &d.aln, stats: &d.stats, pyramid: &d.pyramid, mode };
                        let rendered: Vec<_> = missing
                            .par_iter()
                            .map(|&(tx, ty)| {
                                let mut buf = vec![0u8; TILE * TILE * 4];
                                scene.render_tile(cwd, chd, tx, ty, &mut buf);
                                let img = image::RgbaImage::from_raw(TILE as u32, TILE as u32, buf).unwrap();
                                ((tx, ty), Arc::new(RenderImage::new(vec![image::Frame::new(img)])))
                            })
                            .collect();
                        cache.rendered += rendered.len();
                        for (k, img) in rendered {
                            cache.tiles.insert(k, (img, frame));
                        }
                    }
                    window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
                        let tile_size = size(px((TILE as f64 / scale) as f32), px((TILE as f64 / scale) as f32));
                        for k in &wanted {
                            let entry = cache.tiles.get_mut(k).unwrap();
                            entry.1 = frame;
                            let (x, y) = (k.0 as f64 * TILE as f64 - off_x, k.1 as f64 * TILE as f64 - off_y);
                            let b = Bounds::new(point(to_x(x), to_y(y)), tile_size);
                            let _ = window.paint_image(b, b, Corners::default(), entry.0.clone(), 0, false);
                        }
                    });
                    let keep = (wanted.len() * 4).max(64);
                    cache.evict(keep, window);
                }

                let (vis_r, vis_c) = dev_vp.visible(rows, cols);
                let cell_x = |c: usize| to_x(c as f64 * cwd - off_x);
                let cell_y = |r: usize| to_y(r as f64 * chd - off_y);
                let letters = vp.cw >= LETTER_W && vp.ch >= LETTER_H;
                let font_px = px((vp.ch.min(vp.cw * 1.4) * 0.75).clamp(7.0, 16.0) as f32);
                let mut glyphs: HashMap<u8, ShapedLine> = HashMap::new();
                let mut glyph = |ch: u8, color: Hsla, window: &mut Window| {
                    glyphs
                        .entry(ch)
                        .or_insert_with(|| {
                            let s: SharedString = (ch as char).to_string().into();
                            window.text_system().shape_line(s, font_px, &[run(1, color)], None)
                        })
                        .clone()
                };

                // 2. Letters on cells.
                window.with_content_mask(Some(ContentMask { bounds: area }), |window| {
                    if letters {
                        let text = color(0x1E1E1E);
                        for r in vis_r.clone() {
                            for c in vis_c.clone() {
                                let g = glyph(d.aln.symbol(d.aln.matrix.get(r, c)), text, window);
                                let x = cell_x(c) + (px(vp.cw as f32) - g.width) / 2.0;
                                let _ = g.paint(point(x, cell_y(r)), px(vp.ch as f32), TextAlign::Left, None, window, cx);
                            }
                        }
                    }
                    if let Some(r) = selected.filter(|r| vis_r.contains(r)) {
                        let h = px(vp.ch as f32).max(px(2.));
                        window.paint_quad(outline(Bounds::new(point(area.origin.x, cell_y(r)), size(area.size.width, h)), color(0xD81B60), BorderStyle::Solid));
                    }
                    if let Some((r, c)) = hover
                        && vp.cw >= 3.0
                        && vp.ch >= 3.0
                    {
                        let b = Bounds::new(point(cell_x(c), cell_y(r)), size(px(vp.cw as f32), px(vp.ch as f32)));
                        window.paint_quad(outline(b, color(0x000000), BorderStyle::Solid));
                    }
                });

                // 3. Sticky name column.
                let names = Bounds::new(point(bounds.origin.x, area.origin.y), size(px(NAMES_W), area.size.height));
                window.paint_quad(fill(names, panel));
                window.with_content_mask(Some(ContentMask { bounds: names }), |window| {
                    if vp.ch >= 8.0 {
                        let fpx = px((vp.ch * 0.8).min(13.0) as f32);
                        for r in vis_r.clone() {
                            let name: String = d.aln.names[r].chars().take(48).collect();
                            let c = if Some(r) == selected { color(0xD81B60) } else { fg };
                            let line = window.text_system().shape_line(name.clone().into(), fpx, &[run(name.len(), c)], None);
                            let y = cell_y(r) + (px(vp.ch as f32) - fpx * 1.25) / 2.0;
                            let _ = line.paint(point(names.origin.x + px(6.), y), fpx * 1.25, TextAlign::Left, None, window, cx);
                        }
                    } else if !vis_r.is_empty() {
                        let label = format!("rows {}–{}", vis_r.start + 1, vis_r.end);
                        let line = window.text_system().shape_line(label.clone().into(), px(12.), &[run(label.len(), muted)], None);
                        let _ = line.paint(point(names.origin.x + px(6.), names.origin.y + px(4.)), px(16.), TextAlign::Left, None, window, cx);
                        // Position marker: where the view sits in the whole alignment.
                        let h = f64::from(names.size.height);
                        let y0 = vis_r.start as f64 / rows as f64 * h;
                        let y1 = (vis_r.end as f64 / rows as f64 * h).max(y0 + 2.0);
                        let bar = Bounds::new(point(names.origin.x + px(NAMES_W - 6.), names.origin.y + px(y0 as f32)), size(px(4.), px((y1 - y0) as f32)));
                        window.paint_quad(fill(bar, muted));
                    }
                });
                window.paint_quad(fill(Bounds::new(point(names.origin.x + px(NAMES_W - 1.), bounds.origin.y), size(px(1.), bounds.size.height)), border));

                // 4. Header: ruler, consensus and identity tracks.
                let header = Bounds::new(point(area.origin.x, bounds.origin.y), size(area.size.width, px(HEADER_H)));
                window.paint_quad(fill(header, panel));
                window.with_content_mask(Some(ContentMask { bounds: header }), |window| {
                    let step = view::tick_step(vp.cw, 70.0);
                    let first = vis_c.start.div_ceil(step) * step;
                    let mut c = first.max(step) - 1; // 1-based labels: ticks at columns step, 2*step, ...
                    if vis_c.start == 0 {
                        let line = window.text_system().shape_line("1".into(), px(11.), &[run(1, muted)], None);
                        let _ = line.paint(point(cell_x(0) + px(2.), header.origin.y + px(2.)), px(14.), TextAlign::Left, None, window, cx);
                    }
                    while c < vis_c.end {
                        let x = cell_x(c) + px(vp.cw as f32) / 2.0;
                        window.paint_quad(fill(Bounds::new(point(x, header.origin.y + px(RULER_H - 6.)), size(px(1.), px(6.))), muted));
                        let label = (c + 1).to_string();
                        let line = window.text_system().shape_line(label.clone().into(), px(11.), &[run(label.len(), muted)], None);
                        let _ = line.paint(point(x - line.width / 2.0, header.origin.y + px(2.)), px(14.), TextAlign::Left, None, window, cx);
                        c += step;
                    }

                    // Tracks, one entry per device pixel column, merged into runs of equal colour/height.
                    let npx = dev_vp.width.ceil() as usize;
                    let track = view::column_track(&d.stats, dev_vp.col0, cwd, npx);
                    let cons_y = header.origin.y + px(RULER_H);
                    let bar_y = cons_y + px(CONS_H);
                    let mut i = 0;
                    while i < npx {
                        let Some((code, ident)) = track[i] else { break };
                        let rgb = d.aln.alphabet().color(code);
                        let hq = (ident * (CONS_BAR_H - 4.0)).round();
                        let mut j = i + 1;
                        while j < npx && track[j].is_some_and(|(c2, id2)| d.aln.alphabet().color(c2) == rgb && (id2 * (CONS_BAR_H - 4.0)).round() == hq) {
                            j += 1;
                        }
                        let (x0, x1) = (to_x(i as f64), to_x(j as f64));
                        window.paint_quad(fill(Bounds::new(point(x0, cons_y), size(x1 - x0, px(CONS_H - 2.))), color(rgb)));
                        let bar_c = if ident >= 0.9 { 0x3F7F5F } else if ident >= 0.5 { 0x8FB89F } else { 0xC9A15A };
                        window.paint_quad(fill(Bounds::new(point(x0, bar_y + px(CONS_BAR_H - 2.0 - hq)), size(x1 - x0, px(hq))), color(bar_c)));
                        i = j;
                    }
                    if letters {
                        let text = color(0x1E1E1E);
                        for c in vis_c.clone() {
                            let g = glyph(d.aln.symbol(d.stats.consensus[c]), text, window);
                            let x = cell_x(c) + (px(vp.cw as f32) - g.width) / 2.0;
                            let _ = g.paint(point(x, cons_y - px(1.)), px(CONS_H), TextAlign::Left, None, window, cx);
                        }
                    }
                });
                // Corner labels.
                let corner = Bounds::new(bounds.origin, size(px(NAMES_W), px(HEADER_H)));
                window.paint_quad(fill(corner, panel));
                for (label, y) in [("position", 2.0), ("consensus", RULER_H), ("identity", RULER_H + CONS_H + 6.0)] {
                    let line = window.text_system().shape_line(label.into(), px(11.), &[run(label.len(), muted)], None);
                    let _ = line.paint(point(bounds.origin.x + px(6.), bounds.origin.y + px(y)), px(14.), TextAlign::Left, None, window, cx);
                }
                window.paint_quad(fill(Bounds::new(point(bounds.origin.x, area.origin.y - px(1.)), size(bounds.size.width, px(1.))), border));
                if let Some(p) = paint_ms {
                    p.borrow_mut().push(t0.elapsed().as_secs_f64() * 1000.0);
                }
            },
        )
        .size_full()
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
            self.fit_pending |= self.auto_fit;
        }
        if self.fit_pending && self.data.is_some() && w > 0.0 {
            self.fit_pending = false;
            self.fit();
        } else if self.data.is_some() && w <= 0.0 {
            window.request_animation_frame(); // lay out once to learn the area size
        }
        self.step_bench(window, cx);

        let theme = cx.theme();
        let status_bg = theme.secondary;
        let muted = theme.muted_foreground;
        let mode_label = match self.mode {
            ColorMode::Residues => "Colour: residues",
            ColorMode::Differences => "Colour: differences",
        };
        let zoom_label = format!("{:.3} × {:.3} px/cell", self.vp.cw, self.vp.ch);
        let hover = self.hover_text();
        v_flex()
            .size_full()
            .on_key_down(cx.listener(|v, ev: &KeyDownEvent, window, cx| {
                if ev.keystroke.key == "escape" {
                    v.focus.focus(window, cx);
                    cx.notify();
                }
            }))
            .child(
                h_flex()
                    .gap_2()
                    .p_2()
                    .child(Button::new("open").small().label("Open…").on_click(cx.listener(|v, _, _, cx| v.prompt_open(cx))))
                    .when(self.data.is_none(), |row| {
                        row.child(Button::new("sample").small().label("Try a sample").on_click(cx.listener(|v, _, _, cx| v.open_sample(cx))))
                    })
                    .child(div().w(px(220.)).child(Input::new(&self.jump).small()))
                    .child(div().w(px(220.)).child(Input::new(&self.search).small()))
                    .child(Button::new("zoom-out").small().label("−").on_click(cx.listener(|v, _, _, cx| {
                        v.set_zoom(v.zx - 1, v.zy - 1, None);
                        cx.notify();
                    })))
                    .child(Button::new("zoom-in").small().label("+").on_click(cx.listener(|v, _, _, cx| {
                        v.set_zoom(v.zx + 1, v.zy + 1, None);
                        cx.notify();
                    })))
                    .child(Button::new("fit").small().label("Fit").on_click(cx.listener(|v, _, _, cx| {
                        v.fit();
                        cx.notify();
                    })))
                    .child(Button::new("mode").small().ghost().label(mode_label).on_click(cx.listener(|v, _, _, cx| {
                        v.toggle_mode();
                        cx.notify();
                    })))
                    .child(div().text_xs().text_color(muted).child(zoom_label)),
            )
            .child(
                div()
                    .id("alignment")
                    .flex_1()
                    .min_h_0()
                    .track_focus(&self.focus)
                    .on_key_down(cx.listener(Self::on_key))
                    .on_scroll(cx.listener(Self::on_scroll))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|v, ev: &MouseDownEvent, window, cx| {
                            v.focus.focus(window, cx);
                            v.drag = Some(ev.position);
                            v.update_hover(ev.position);
                            if let Some((r, _)) = v.hover {
                                v.selected_row = Some(r);
                            }
                            cx.notify();
                        }),
                    )
                    .on_mouse_up(MouseButton::Left, cx.listener(|v, _: &MouseUpEvent, _, _| v.drag = None))
                    .on_mouse_move(cx.listener(|v, ev: &MouseMoveEvent, _, cx| {
                        if let Some(start) = v.drag.filter(|_| ev.dragging()) {
                            let d = ev.position - start;
                            v.pan(-f64::from(d.x), -f64::from(d.y));
                            v.drag = Some(ev.position);
                        }
                        v.update_hover(ev.position);
                        cx.notify();
                    }))
                    .child(self.alignment_canvas(window, cx)),
            )
            .child(
                h_flex()
                    .gap_4()
                    .px_2()
                    .py_1()
                    .bg(status_bg)
                    .text_xs()
                    .text_color(muted)
                    .child(self.status.clone())
                    .child(hover),
            )
    }
}

fn peak_rss_mb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("VmHWM:"))?;
    Some(line.split_whitespace().nth(1)?.parse::<u64>().ok()? / 1024)
}

const USAGE: &str = "usage: msascope [FILE] [--type dna|rna|protein] [--stats] [--bench]
  FILE     aligned FASTA (.gz ok), Clustal or Stockholm
  --type   residue type, instead of guessing it from the residues (also for files opened later)
  --stats  load, print sizes and timings, and exit without opening a window
  --bench  open the window, pan through four zoom levels and print frame timings";

fn main() -> anyhow::Result<()> {
    let mut path = None;
    let (mut stats, mut bench) = (false, false);
    let mut ty: Option<SeqType> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let parse_ty = |v: Option<String>| -> anyhow::Result<SeqType> {
            let v = v.ok_or_else(|| anyhow::anyhow!("--type needs a value\n{USAGE}"))?;
            v.parse().map_err(|e| anyhow::anyhow!("{e}\n{USAGE}"))
        };
        match a.as_str() {
            "--type" => ty = Some(parse_ty(args.next())?),
            _ if a.starts_with("--type=") => ty = Some(parse_ty(Some(a["--type=".len()..].to_string()))?),
            "--stats" => stats = true,
            "--bench" => bench = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            _ => path = Some(PathBuf::from(a)),
        }
    }
    if stats {
        let Some(p) = path else { anyhow::bail!("--stats needs a file\n{USAGE}") };
        let l = Loaded::open_as(&p, ty)?;
        println!("{}", l.summary());
        if let Some(rss) = peak_rss_mb() {
            println!("peak RSS {rss} MiB");
        }
        return Ok(());
    }
    gpui_kit::application().with_assets(gpui_kit::assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        let settings = Settings::load();
        gui::tokens::apply_palette(settings.theme.palette(is_dark(cx.window_appearance())), cx);
        gui::fonts::apply(None, cx);
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
            titlebar: Some(TitlebarOptions { title: Some("msascope".into()), ..Default::default() }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| MsaScope::new(path, ty, bench, window, cx)))
            .expect("failed to open window");
    });
    Ok(())
}
