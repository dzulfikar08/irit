// Adapted from Kantor (github.com/dzulfikar08/kantor) by the same author, relicensed
// GPL-3.0-or-later for irit.
//! UI and monospace font selection.
//!
//! No fonts are bundled. SF Pro is used when installed, else the best free alternative. The
//! monospace font is only for residue letters; everything else uses the UI font.
//!
//! On Linux the automatic choice skips *variable* fonts (Inter's `Inter-var.ttf`, Adwaita Sans,
//! ...) when a static family is installed: GPUI's Linux text system (cosmic-text) loads a variable
//! font as one face and draws its default instance, so bold and medium text in such a family look
//! regular. A family is taken as variable when asking for its bold resolves to the same face as
//! its regular. macOS (Core Text draws variable weights) is unaffected.

use gpui_kit::component::Theme;
use gpui_kit::*;
use std::sync::OnceLock;

/// Preferred UI fonts, best first.
pub const UI_FONTS: &[&str] = &["SF Pro Text", "SF Pro", "Inter", "Inter Variable", "Adwaita Sans", "Noto Sans", "Cantarell"];

/// Preferred monospace fonts, best first.
pub const MONO_FONTS: &[&str] = &["SF Mono", "JetBrainsMono Nerd Font", "JetBrains Mono", "Adwaita Mono", "Noto Sans Mono"];

/// Installed font families. Read once per run: listing the system's fonts takes about 150 ms on
/// macOS, too slow for a render path.
pub fn installed(cx: &App) -> &'static [String] {
    static FONTS: OnceLock<Vec<String>> = OnceLock::new();
    FONTS.get_or_init(|| cx.text_system().all_font_names())
}

fn first_installed(installed: &[String], prefs: &[&str]) -> Option<String> {
    pick(installed, prefs, |_| true)
}

/// The first of `prefs` that is installed, preferring ones for which `real_bold` holds; if none
/// does, the first installed.
fn pick(installed: &[String], prefs: &[&str], real_bold: impl Fn(&str) -> bool) -> Option<String> {
    let mut present = prefs.iter().copied().filter(|p| installed.iter().any(|f| f == p));
    let first = present.clone().next();
    present.find(|p| real_bold(p)).or(first).map(str::to_owned)
}

/// Whether GPUI draws `family`'s bold with a bold face of its own (see the module docs). Always
/// true off Linux.
pub fn has_real_bold(family: &str, cx: &App) -> bool {
    !cfg!(target_os = "linux") || bold_is_own_face(cx.text_system(), family)
}

/// Whether `family`'s bold resolves to another face than its regular.
fn bold_is_own_face(ts: &TextSystem, family: &str) -> bool {
    let regular = font(SharedString::from(family.to_owned()));
    let bold = Font { weight: FontWeight::BOLD, ..regular.clone() };
    ts.resolve_font(&regular) != ts.resolve_font(&bold)
}

/// The UI font used when the user hasn't chosen one.
pub fn automatic_ui_font(cx: &App) -> Option<String> {
    pick(installed(cx), UI_FONTS, |f| has_real_bold(f, cx))
}

/// Apply `ui_font` (or the automatic choice) and the best monospace font to the theme.
pub fn apply(ui_font: Option<&str>, cx: &mut App) {
    let fonts = installed(cx);
    let ui = ui_font.filter(|f| fonts.iter().any(|i| i == f)).map(str::to_owned).or_else(|| automatic_ui_font(cx));
    let mono = first_installed(fonts, MONO_FONTS);
    Theme::update(cx, |theme| {
        if let Some(ui) = ui {
            theme.font_family = ui.into();
        }
        if let Some(mono) = mono {
            theme.mono_font_family = mono.into();
        }
    });
}

/// Tabular (fixed-width) figures, for numbers that change in place: ruler, zoom, counts.
pub fn tabular() -> FontFeatures {
    FontFeatures(std::sync::Arc::new(vec![("tnum".into(), 1)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[std::prelude::v1::test]
    fn automatic_choice_prefers_a_real_bold() {
        let installed: Vec<String> = ["Adwaita Sans", "Inter", "Noto Sans"].into_iter().map(String::from).collect();
        let variable = |f: &str| f == "Inter" || f == "Adwaita Sans";
        assert_eq!(pick(&installed, UI_FONTS, |f| !variable(f)).as_deref(), Some("Noto Sans"));
        // Every candidate variable: keep the first.
        assert_eq!(pick(&installed, UI_FONTS, |_| false).as_deref(), Some("Inter"));
        assert_eq!(pick(&installed, UI_FONTS, |_| true).as_deref(), Some("Inter"));
        assert_eq!(pick(&[], UI_FONTS, |_| true), None);
        assert_eq!(first_installed(&["Noto Sans Mono".to_owned()], MONO_FONTS).as_deref(), Some("Noto Sans Mono"));
    }
}
