// Adapted from Kantor (github.com/dzulfikar08/kantor) by the same author, relicensed
// GPL-3.0-or-later for irit.
//! Colour tokens derived from a [`Palette`].
//!
//! Chrome, surfaces and borders are the foreground mixed over the background at small fixed
//! amounts, so a palette yields a consistent, layered UI. Views read colours from here, never as
//! raw values.

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use super::metrics;
use super::palette::{LIGHT, Palette, Rgb, mix};

/// Semantic colours for msascope's own views. Read with [`tokens`].
#[derive(Debug, Clone, Copy)]
pub struct Tokens {
    pub dark: bool,
    /// Alignment area and name rows.
    pub bg: Hsla,
    /// Title bar, toolbar, status bar, side panels.
    pub chrome: Hsla,
    /// Cards, inputs, keyboard hints.
    pub surface: Hsla,
    /// Menus, tooltips, dialogs.
    pub elevated: Hsla,
    pub fg: Hsla,
    pub fg_muted: Hsla,
    pub fg_dim: Hsla,
    pub border: Hsla,
    pub border_strong: Hsla,
    pub hover: Hsla,
    pub selected: Hsla,
    pub accent: Hsla,
    /// Column bands and selections (Phase 5).
    #[allow(dead_code)]
    pub accent_soft: Hsla,
    pub accent_fg: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
    pub info: Hsla,
    /// msascope's own colour: the app mark, the file tab and status bar dots.
    pub app: Hsla,
    /// Track header and corner, and the selected row's name (`grid_header_active`).
    #[allow(dead_code)]
    pub grid_line: Hsla,
    pub grid_header: Hsla,
    pub grid_header_active: Hsla,
    /// Letters on residue fills.
    pub res_ink: Hsla,
    /// The dot drawn for a gap at letter zoom.
    pub gap_ink: Hsla,
    /// Identity bars: high (≥ 90%) and lower identity.
    pub track_hi: Hsla,
    pub track_lo: Hsla,
    /// The same colours as 0xRRGGBB, for tiles rasterised on the CPU.
    pub bg_rgb: Rgb,
    pub fg_rgb: Rgb,
    pub accent_rgb: Rgb,
}

impl Global for Tokens {}

/// The active tokens (light ones until [`apply_palette`] runs).
pub fn tokens(cx: &App) -> Tokens {
    cx.try_global::<Tokens>().copied().unwrap_or_else(|| Tokens::from_palette(&LIGHT))
}

fn c(rgb: Rgb) -> Hsla {
    gpui_kit::rgb(rgb).into()
}

fn alpha(rgb: Rgb, a: f32) -> Hsla {
    let mut h = c(rgb);
    h.a = a;
    h
}

fn luminance(rgb: Rgb) -> f32 {
    let ch = |s: u32| ((rgb >> s) & 0xff) as f32 / 255.0;
    0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum Elevation {
    Raised,
    Popover,
    Modal,
}

impl Tokens {
    pub fn from_palette(p: &Palette) -> Self {
        let on_bg = |a: f32| c(mix(p.background, p.foreground, a));
        // Headers sit between chrome and surface, a step above the cells.
        let header = mix(p.background, p.foreground, if p.dark { 0.06 } else { 0.045 });
        let (chrome, surface, elevated, border, border_strong) = if p.dark {
            (on_bg(0.03), on_bg(0.05), on_bg(0.07), on_bg(0.10), on_bg(0.18))
        } else {
            (on_bg(0.025), on_bg(0.035), c(0xffffff), on_bg(0.11), on_bg(0.20))
        };
        Self {
            dark: p.dark,
            bg: c(p.background),
            chrome,
            surface,
            elevated,
            fg: c(p.foreground),
            // 68% clears 4.5:1 on dark palettes; Latte's softer foreground needs 85% on light.
            fg_muted: alpha(p.foreground, if p.dark { 0.68 } else { 0.85 }),
            fg_dim: alpha(p.foreground, 0.42),
            border,
            border_strong,
            hover: alpha(p.foreground, 0.06),
            selected: alpha(p.accent, if p.dark { 0.16 } else { 0.12 }),
            accent: c(p.accent),
            accent_soft: alpha(p.accent, 0.12),
            accent_fg: if luminance(p.accent) > 0.6 { c(0x000000) } else { c(0xffffff) },
            success: c(p.green),
            warning: c(p.yellow),
            danger: c(p.red),
            info: c(p.blue),
            app: c(p.magenta),
            grid_line: if p.dark { on_bg(0.12) } else { on_bg(0.14) },
            grid_header: c(header),
            grid_header_active: c(mix(header, p.accent, if p.dark { 0.22 } else { 0.18 })),
            res_ink: if p.dark { c(0xeef1fb) } else { c(0x2a2c3c) },
            gap_ink: on_bg(0.35),
            track_hi: c(p.accent),
            track_lo: c(mix(p.background, p.accent, if p.dark { 0.45 } else { 0.35 })),
            bg_rgb: p.background,
            fg_rgb: p.foreground,
            accent_rgb: p.accent,
        }
    }

    /// The colours tiles are drawn with.
    pub fn tile_theme(&self) -> msascope::scheme::Theme {
        msascope::scheme::Theme { dark: self.dark, bg: self.bg_rgb, fg: self.fg_rgb, accent: self.accent_rgb }
    }

    /// Shadow stack for an elevation (lighter in light mode).
    pub fn shadow(&self, elevation: Elevation) -> Vec<BoxShadow> {
        let k = if self.dark { 1.0 } else { 0.55 };
        let s = |y: f32, blur: f32, a: f32| BoxShadow {
            color: Hsla { h: 0., s: 0., l: 0., a: a * k },
            offset: point(px(0.), px(y)),
            blur_radius: px(blur),
            spread_radius: px(0.),
            inset: false,
        };
        match elevation {
            Elevation::Raised => vec![s(1., 2., 0.18)],
            Elevation::Popover => vec![s(2., 6., 0.22), s(8., 24., 0.18)],
            Elevation::Modal => vec![s(4., 12., 0.25), s(16., 48., 0.30)],
        }
    }
}

/// Install `p` as the active palette: msascope's tokens plus GPUI Kit's theme.
pub fn apply_palette(p: &Palette, cx: &mut App) {
    let t = Tokens::from_palette(p);
    cx.set_global(t);

    let mode = if p.dark { ThemeMode::Dark } else { ThemeMode::Light };
    // Load the matching base theme first, then override colours in a second update
    // (a mode switch replaces colours; see `Theme::update`).
    Theme::change(mode, None, cx);

    let solid_hover = c(mix(p.background, p.foreground, 0.06));
    Theme::update(cx, |theme| {
        theme.font_size = metrics::REM;
        theme.radius = metrics::RADIUS_MD;
        theme.radius_lg = metrics::RADIUS_XL;
        theme.shadow = true;

        let k = &mut theme.colors;
        k.background = t.bg;
        k.foreground = t.fg;
        k.border = t.border;
        k.input = t.border;
        k.ring = t.accent;
        k.caret = t.fg;
        k.selection = alpha(p.accent, 0.35);

        k.primary = t.accent;
        k.primary_hover = c(mix(p.accent, p.foreground, 0.12));
        k.primary_active = c(mix(p.accent, p.background, 0.12));
        k.primary_foreground = t.accent_fg;
        // gpui-kit 0.9.1 colours buttons from their own fields, not from primary/secondary.
        k.button_primary = k.primary;
        k.button_primary_hover = k.primary_hover;
        k.button_primary_active = k.primary_active;
        k.button_primary_foreground = t.accent_fg;
        k.button = t.surface;
        k.button_hover = solid_hover;
        k.button_active = c(mix(p.background, p.foreground, 0.10));
        k.button_foreground = t.fg;
        k.button_secondary = t.surface;
        k.button_secondary_hover = solid_hover;
        k.button_secondary_active = c(mix(p.background, p.foreground, 0.10));
        k.button_secondary_foreground = t.fg;
        k.secondary = t.surface;
        k.secondary_hover = solid_hover;
        k.secondary_active = c(mix(p.background, p.foreground, 0.10));
        k.secondary_foreground = t.fg;
        k.muted = t.surface;
        k.muted_foreground = t.fg_muted;
        k.accent = solid_hover;
        k.accent_foreground = t.fg;
        k.popover = t.elevated;
        k.popover_foreground = t.fg;
        k.danger = t.danger;
        k.success = t.success;
        k.warning = t.warning;
        k.info = t.info;
        k.link = t.accent;

        k.title_bar = t.chrome;
        k.title_bar_border = t.border;
        k.status_bar = t.chrome;
        k.status_bar_border = t.border;
        k.tab_bar = t.chrome;
        k.tab = t.chrome;
        k.tab_active = t.bg;
        k.tab_foreground = t.fg_muted;
        k.tab_active_foreground = t.fg;

        k.list = t.bg;
        k.list_head = t.surface;
        k.list_even = t.bg;
        k.list_hover = solid_hover;
        k.list_active = t.selected;
        k.list_active_border = t.accent;

        k.scrollbar = Hsla::transparent_black();
        k.scrollbar_thumb = alpha(p.foreground, 0.22);
        k.scrollbar_thumb_hover = alpha(p.foreground, 0.38);
        k.window_border = t.border;
    });
}

#[cfg(test)]
mod tests {
    use super::super::palette::{DARK, contrast};
    use super::*;

    /// Gridlines and headers must stand apart from the cells in both modes.
    #[std::prelude::v1::test]
    fn grid_tokens_differ_from_cells() {
        for p in [DARK, LIGHT] {
            let t = Tokens::from_palette(&p);
            let l = |h: Hsla| h.l;
            assert!((l(t.grid_line) - l(t.bg)).abs() > 0.05, "grid line, dark={}", p.dark);
            assert!((l(t.grid_header) - l(t.bg)).abs() > 0.02, "header, dark={}", p.dark);
            assert_ne!(t.grid_header_active, t.grid_header);
            assert_eq!(t.grid_header_active.a, 1.0);
        }
    }

    /// The values in the redesign spec (sampled from the mockup) come out of the recipe.
    #[std::prelude::v1::test]
    fn tokens_match_the_spec() {
        let hex = |h: Hsla| {
            let r = Rgba::from(h);
            ((r.r * 255.0).round() as u32) << 16 | ((r.g * 255.0).round() as u32) << 8 | (r.b * 255.0).round() as u32
        };
        // Within 1 per channel: Hsla stores the mixed colour as floats.
        let near = |h: Hsla, want: Rgb| {
            let got = hex(h);
            assert!([16, 8, 0].iter().all(|s| ((got >> s) & 255).abs_diff((want >> s) & 255) <= 1), "{got:06x} vs {want:06x}");
        };
        let l = Tokens::from_palette(&LIGHT);
        near(l.chrome, 0xebedf2);
        near(l.grid_header, 0xe8eaef);
        near(l.border, 0xdddfe6);
        let d = Tokens::from_palette(&DARK);
        near(d.chrome, 0x232434);
        near(d.grid_header, 0x28293a);
        near(d.border, 0x303042);
    }

    /// The core's theme presets (used by its colour tests) are the GUI's palettes.
    #[std::prelude::v1::test]
    fn tile_themes_match_core_presets() {
        assert_eq!(Tokens::from_palette(&LIGHT).tile_theme(), msascope::scheme::Theme::LIGHT);
        assert_eq!(Tokens::from_palette(&DARK).tile_theme(), msascope::scheme::Theme::DARK);
        // and the letter ink is the same
        let l = Rgba::from(Tokens::from_palette(&LIGHT).res_ink);
        assert_eq!(((l.r * 255.0).round() as u32) << 16 | ((l.g * 255.0).round() as u32) << 8 | (l.b * 255.0).round() as u32, msascope::scheme::Theme::LIGHT.ink());
    }

    /// Text tokens on the surfaces they are drawn on are at least 4.5:1 (WCAG AA) in both palettes:
    /// `fg` and `fg_muted` on the background, chrome and track header, and `fg` on the selected
    /// row's name. `fg_dim` is exempt: it is only used for secondary text.
    #[std::prelude::v1::test]
    fn text_contrast_is_aa() {
        for p in [DARK, LIGHT] {
            let t = Tokens::from_palette(&p);
            // Translucent tokens composited over their surface, as the GPU draws them.
            let over = |fg: Hsla, bg: Hsla| -> Rgb {
                let (f, b) = (Rgba::from(fg), Rgba::from(bg));
                let ch = |x: f32, y: f32| (((x * f.a + y * (1.0 - f.a)) * 255.0).round() as u32).min(255);
                ch(f.r, b.r) << 16 | ch(f.g, b.g) << 8 | ch(f.b, b.b)
            };
            let pairs = [("bg", t.bg, true), ("chrome", t.chrome, true), ("header", t.grid_header, true), ("selected row", t.grid_header_active, false)];
            for (name, surface, muted_too) in pairs {
                let s = over(surface, surface);
                for (tn, text) in [("fg", t.fg), ("fg_muted", t.fg_muted)].into_iter().take(if muted_too { 2 } else { 1 }) {
                    let ratio = contrast(over(text, surface), s);
                    assert!(ratio >= 4.5, "{tn} on {name}, dark={}: {ratio:.2}", p.dark);
                }
            }
        }
    }
}
