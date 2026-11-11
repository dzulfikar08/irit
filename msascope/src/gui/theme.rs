//! Theme choice (System, Light, Dark) and the small settings file it is saved in.
//!
//! Settings live in `msascope/settings.toml` under the platform's config folder
//! (`$XDG_CONFIG_HOME` or `~/.config` on Linux, `~/Library/Application Support` on macOS,
//! `%APPDATA%` on Windows), as `key = value` lines. Unknown keys and bad values are ignored.

use std::path::PathBuf;

use gpui_kit::*;

use super::metrics::{NAME_W_DEFAULT, NAME_W_MAX, NAME_W_MIN, TREE_W_DEFAULT, TREE_W_MAX, TREE_W_MIN};
use super::palette::{DARK, LIGHT, Palette};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ThemeChoice {
    /// Follow the OS appearance.
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 3] = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::System => "System",
            ThemeChoice::Light => "Light",
            ThemeChoice::Dark => "Dark",
        }
    }

    fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }

    /// The palette to use when the OS appearance is `dark_os`.
    pub fn palette(self, dark_os: bool) -> &'static Palette {
        match self {
            ThemeChoice::Light => &LIGHT,
            ThemeChoice::Dark => &DARK,
            ThemeChoice::System if dark_os => &DARK,
            ThemeChoice::System => &LIGHT,
        }
    }
}

pub fn is_dark(a: WindowAppearance) -> bool {
    matches!(a, WindowAppearance::Dark | WindowAppearance::VibrantDark)
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub theme: ThemeChoice,
    /// Width of the name panel, logical px.
    pub name_width: f32,
    /// The overview panel on the right.
    pub overview: bool,
    /// The "gappy" threshold: columns with at least this share of gaps (percent) are hidden
    /// while gappy columns are hidden.
    pub gappy_pct: u8,
    /// Width of the tree panel, logical px.
    pub tree_width: f32,
    /// The tree panel, when a tree is loaded.
    pub tree_panel: bool,
    /// Alignments opened, most recent first (at most [`RECENT_MAX`]).
    pub recent: Vec<PathBuf>,
}

/// Recent files kept in the settings.
pub const RECENT_MAX: usize = 10;

/// The gappy thresholds the toolbar offers.
pub const GAPPY_CHOICES: [u8; 5] = [25, 50, 75, 90, 100];

impl Default for Settings {
    fn default() -> Self {
        Settings { theme: ThemeChoice::System, name_width: NAME_W_DEFAULT, overview: true, gappy_pct: 50, tree_width: TREE_W_DEFAULT, tree_panel: true, recent: Vec::new() }
    }
}

impl Settings {
    pub fn parse(text: &str) -> Self {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim().trim_matches('"');
            match k.trim() {
                "recent" if !v.is_empty() && s.recent.len() < RECENT_MAX => s.recent.push(PathBuf::from(v)),
                "theme" => {
                    if let Some(t) = ThemeChoice::ALL.into_iter().find(|t| t.key() == v) {
                        s.theme = t;
                    }
                }
                "name_width" => {
                    if let Ok(w) = v.parse::<f32>()
                        && w.is_finite()
                    {
                        s.name_width = w.clamp(NAME_W_MIN, NAME_W_MAX);
                    }
                }
                "overview" => s.overview = v != "false",
                "tree_panel" => s.tree_panel = v != "false",
                "tree_width" => {
                    if let Ok(w) = v.parse::<f32>()
                        && w.is_finite()
                    {
                        s.tree_width = w.clamp(TREE_W_MIN, TREE_W_MAX);
                    }
                }
                "gappy_pct" => {
                    if let Ok(p) = v.parse::<u8>()
                        && (1..=100).contains(&p)
                    {
                        s.gappy_pct = p;
                    }
                }
                _ => {}
            }
        }
        s
    }

    pub fn to_text(&self) -> String {
        let recent: String = self.recent.iter().filter_map(|p| p.to_str()).map(|p| format!("recent = \"{p}\"\n")).collect();
        format!(
            "# msascope settings\ntheme = \"{}\"\nname_width = {}\noverview = {}\ngappy_pct = {}\ntree_width = {}\ntree_panel = {}\n{recent}",
            self.theme.key(),
            self.name_width.round(),
            self.overview,
            self.gappy_pct,
            self.tree_width.round(),
            self.tree_panel
        )
    }

    fn path() -> Option<PathBuf> {
        let env = |k: &str| std::env::var_os(k).map(PathBuf::from).filter(|p| p.is_absolute());
        let base = if cfg!(target_os = "windows") {
            env("APPDATA")
        } else if cfg!(target_os = "macos") {
            env("HOME").map(|h| h.join("Library/Application Support"))
        } else {
            env("XDG_CONFIG_HOME").or_else(|| env("HOME").map(|h| h.join(".config")))
        };
        Some(base?.join("msascope").join("settings.toml"))
    }

    /// Put `path` first in the recent files (made absolute; once only; non-UTF-8 paths skipped).
    pub fn add_recent(&mut self, path: &std::path::Path) {
        let p = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        if p.to_str().is_none() {
            return;
        }
        self.recent.retain(|r| r != &p);
        self.recent.insert(0, p);
        self.recent.truncate(RECENT_MAX);
    }

    pub fn load() -> Self {
        Self::path().and_then(|p| std::fs::read_to_string(p).ok()).map_or_else(Settings::default, |t| Settings::parse(&t))
    }

    /// Save, ignoring errors (a read-only home folder just means settings don't persist).
    pub fn save(&self) {
        if let Some(p) = Self::path() {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(p, self.to_text());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[std::prelude::v1::test]
    fn settings_round_trip_and_bad_values() {
        let mut s = Settings { theme: ThemeChoice::Dark, name_width: 250.0, overview: false, gappy_pct: 90, tree_width: 200.0, tree_panel: false, recent: Vec::new() };
        assert_eq!(Settings::parse(&s.to_text()), s);
        // Recent files: newest first, once each, at most RECENT_MAX, a path with '=' kept whole.
        for i in 0..12 {
            s.add_recent(std::path::Path::new(&format!("/data/a=b/f{i}.fa")));
        }
        s.add_recent(std::path::Path::new("/data/a=b/f5.fa"));
        assert_eq!(s.recent.len(), RECENT_MAX);
        assert_eq!(s.recent[0], PathBuf::from("/data/a=b/f5.fa"));
        assert_eq!(s.recent[1], PathBuf::from("/data/a=b/f11.fa"));
        assert_eq!(Settings::parse(&s.to_text()), s);
        let s = Settings::parse("theme = purple\nname_width = 9999\nother = 1\nno equals sign\ngappy_pct = 0\n");
        assert_eq!(s, Settings { theme: ThemeChoice::System, name_width: NAME_W_MAX, ..Settings::default() });
        assert_eq!(Settings::parse("gappy_pct = 101").gappy_pct, 50);
        assert_eq!(Settings::parse("name_width = nan").name_width, NAME_W_DEFAULT);
        assert_eq!(Settings::parse("tree_width = 5").tree_width, TREE_W_MIN);
    }

    #[std::prelude::v1::test]
    fn system_follows_the_os() {
        assert!(ThemeChoice::System.palette(true).dark);
        assert!(!ThemeChoice::System.palette(false).dark);
        assert!(ThemeChoice::Dark.palette(false).dark);
        assert!(!ThemeChoice::Light.palette(true).dark);
    }
}
