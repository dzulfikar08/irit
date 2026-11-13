//! Theme choice (System, Light, Dark) and the small settings file it is saved in.
//!
//! Settings live in `msascope/settings.toml` under the platform's config folder
//! (`$XDG_CONFIG_HOME` or `~/.config` on Linux, `~/Library/Application Support` on macOS,
//! `%APPDATA%` on Windows), as `key = value` lines. Unknown keys and bad values are ignored.

use std::path::{Path, PathBuf};

use gpui_kit::*;

use super::app::RefOpts;
use super::metrics::{
    NAME_W_DEFAULT, NAME_W_MAX, NAME_W_MIN, TREE_W_DEFAULT, TREE_W_MAX, TREE_W_MIN,
};
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
    /// The reference row chosen in each file, most recent first (at most [`REFS_MAX`]).
    pub refs: Vec<RefMemory>,
    /// App Sandbox only: security-scoped bookmarks (base64) of recent files and allowed folders,
    /// most recent first (at most [`BOOKMARKS_MAX`]). See `gui::sandbox`.
    pub bookmarks: Vec<(PathBuf, String)>,
}

/// Recent files kept in the settings.
pub const RECENT_MAX: usize = 10;
/// Files whose reference row is remembered.
pub const REFS_MAX: usize = 50;
/// Bookmarks kept (App Sandbox): the recent files plus allowed folders.
pub const BOOKMARKS_MAX: usize = 30;

/// Which version of a file a remembered reference belongs to: its size and modification time.
/// Taken from the file's metadata when it is opened (no extra reading); a file rewritten since
/// has another stamp, and its old choice is not applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub size: u64,
    /// Modification time in ns since the Unix epoch.
    pub mtime: u128,
}

impl FileStamp {
    pub fn of(path: &Path) -> Option<FileStamp> {
        let m = std::fs::metadata(path).ok()?;
        let mtime = m
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_nanos();
        Some(FileStamp {
            size: m.len(),
            mtime,
        })
    }
}

/// The reference row of one file and what it does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RefMemory {
    /// Absolute path of the alignment.
    pub path: PathBuf,
    pub stamp: FileStamp,
    /// Alignment row, and its name (the row is restored only where the name still matches; a
    /// row of the same name elsewhere is taken instead).
    pub row: usize,
    pub name: String,
    pub opts: RefOpts,
}

impl RefMemory {
    /// One settings line: `reference = "size<TAB>mtime<TAB>row<TAB>flags<TAB>name<TAB>path"`,
    /// flags `pin ruler compare` as three 0/1 digits. `None` for names or paths it can't hold.
    fn to_line(&self) -> Option<String> {
        let path = self.path.to_str()?;
        let bad = |s: &str| s.is_empty() || s.contains(['\t', '\n', '\r']) || s.ends_with('"');
        if bad(path) || bad(&self.name) {
            return None;
        }
        let o = self.opts;
        let b = |x: bool| if x { '1' } else { '0' };
        Some(format!(
            "reference = \"{}\t{}\t{}\t{}{}{}\t{}\t{path}\"\n",
            self.stamp.size,
            self.stamp.mtime,
            self.row,
            b(o.pin),
            b(o.ruler),
            b(o.compare),
            self.name
        ))
    }

    fn parse(v: &str) -> Option<RefMemory> {
        let mut f = v.splitn(6, '\t');
        let size = f.next()?.parse().ok()?;
        let mtime = f.next()?.parse().ok()?;
        let row = f.next()?.parse().ok()?;
        let flags = f.next()?.as_bytes();
        let name = f.next()?;
        let path = f.next()?;
        if flags.len() != 3
            || !flags.iter().all(|c| matches!(c, b'0' | b'1'))
            || name.is_empty()
            || path.is_empty()
        {
            return None;
        }
        let opts = RefOpts {
            pin: flags[0] == b'1',
            ruler: flags[1] == b'1',
            compare: flags[2] == b'1',
        };
        Some(RefMemory {
            path: PathBuf::from(path),
            stamp: FileStamp { size, mtime },
            row,
            name: name.into(),
            opts,
        })
    }

    /// The row to restore in an alignment with these row names: the stored row if its name
    /// matches, else the first row of that name.
    pub fn find_row(&self, names: &[String]) -> Option<usize> {
        if names.get(self.row).is_some_and(|n| *n == self.name) {
            return Some(self.row);
        }
        names.iter().position(|n| *n == self.name)
    }
}

/// The gappy thresholds the toolbar offers.
pub const GAPPY_CHOICES: [u8; 5] = [25, 50, 75, 90, 100];

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: ThemeChoice::System,
            name_width: NAME_W_DEFAULT,
            overview: true,
            gappy_pct: 50,
            tree_width: TREE_W_DEFAULT,
            tree_panel: true,
            recent: Vec::new(),
            refs: Vec::new(),
            bookmarks: Vec::new(),
        }
    }
}

impl Settings {
    pub fn parse(text: &str) -> Self {
        let mut s = Settings::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            if k.trim() == "reference" {
                // Taken as written (a name or path may start or end with spaces): one pair of
                // quotes removed.
                let raw = v.strip_prefix(' ').unwrap_or(v);
                let raw = raw
                    .strip_prefix('"')
                    .and_then(|r| r.strip_suffix('"'))
                    .unwrap_or(raw);
                if s.refs.len() < REFS_MAX
                    && let Some(m) = RefMemory::parse(raw)
                    && !s.refs.iter().any(|r| r.path == m.path)
                {
                    s.refs.push(m);
                }
                continue;
            }
            let v = v.trim().trim_matches('"');
            match k.trim() {
                "bookmark" if s.bookmarks.len() < BOOKMARKS_MAX => {
                    if let Some((b, p)) = v.split_once(' ')
                        && !b.is_empty()
                        && !p.is_empty()
                    {
                        s.bookmarks.push((PathBuf::from(p), b.to_string()));
                    }
                }
                "recent" if !v.is_empty() && s.recent.len() < RECENT_MAX => {
                    s.recent.push(PathBuf::from(v))
                }
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
        let recent: String = self
            .recent
            .iter()
            .filter_map(|p| p.to_str())
            .map(|p| format!("recent = \"{p}\"\n"))
            .collect();
        let refs: String = self.refs.iter().filter_map(RefMemory::to_line).collect();
        let bookmarks: String = self
            .bookmarks
            .iter()
            .filter_map(|(p, b)| Some(format!("bookmark = \"{b} {}\"\n", p.to_str()?)))
            .collect();
        format!(
            "# msascope settings\ntheme = \"{}\"\nname_width = {}\noverview = {}\ngappy_pct = {}\ntree_width = {}\ntree_panel = {}\n{recent}{refs}{bookmarks}",
            self.theme.key(),
            self.name_width.round(),
            self.overview,
            self.gappy_pct,
            self.tree_width.round(),
            self.tree_panel
        )
    }

    fn path() -> Option<PathBuf> {
        let env = |k: &str| {
            std::env::var_os(k)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
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
    /// Keep a sandbox bookmark for `path` (a file or folder), most recent first.
    pub fn add_bookmark(&mut self, path: &std::path::Path, bookmark: String) {
        self.bookmarks.retain(|(p, _)| p != path);
        self.bookmarks.insert(0, (path.to_path_buf(), bookmark));
        self.bookmarks.truncate(BOOKMARKS_MAX);
    }

    /// Resolve the stored bookmarks (App Sandbox), so recent files and allowed folders can be
    /// read again; files that moved are followed, and bookmarks that no longer resolve dropped.
    pub fn restore_bookmarks(&mut self) {
        if !super::sandbox::active() || self.bookmarks.is_empty() {
            return;
        }
        let resolved = super::sandbox::restore(&self.bookmarks);
        let mut kept = Vec::new();
        for (old, new, fresh) in resolved {
            let b = fresh.unwrap_or_else(|| {
                self.bookmarks
                    .iter()
                    .find(|(p, _)| *p == old)
                    .map(|(_, b)| b.clone())
                    .unwrap_or_default()
            });
            if new != old {
                for r in self.recent.iter_mut().filter(|r| **r == old) {
                    *r = new.clone();
                }
            }
            kept.push((new, b));
        }
        self.bookmarks = kept;
    }

    pub fn add_recent(&mut self, path: &std::path::Path) {
        let p = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        if p.to_str().is_none() {
            return;
        }
        self.recent.retain(|r| r != &p);
        self.recent.insert(0, p);
        self.recent.truncate(RECENT_MAX);
    }

    /// The reference remembered for `path` as it is now (`stamp`); `None` when there is none or
    /// the file has changed since.
    pub fn remembered_ref(&self, path: &Path, stamp: FileStamp) -> Option<&RefMemory> {
        let p = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        self.refs.iter().find(|r| r.path == p && r.stamp == stamp)
    }

    /// Remember `path`'s reference row (`Some((row, name, opts))`) first, or forget it (`None`).
    /// Any older entry for the path goes; at most [`REFS_MAX`] files are kept.
    pub fn remember_ref(
        &mut self,
        path: &Path,
        stamp: FileStamp,
        reference: Option<(usize, String, RefOpts)>,
    ) {
        let p = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
        self.refs.retain(|r| r.path != p);
        if let Some((row, name, opts)) = reference {
            let m = RefMemory {
                path: p,
                stamp,
                row,
                name,
                opts,
            };
            if m.to_line().is_some() {
                self.refs.insert(0, m);
                self.refs.truncate(REFS_MAX);
            }
        }
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .map_or_else(Settings::default, |t| Settings::parse(&t))
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
    fn sandbox_bookmarks_round_trip() {
        let mut s = Settings::default();
        s.add_bookmark(Path::new("/Users/a/data folder"), "AAAB/+==".into());
        s.add_bookmark(Path::new("/Users/a/x.sto"), "QUJD".into());
        s.add_bookmark(Path::new("/Users/a/data folder"), "REVG".into());
        let back = Settings::parse(&s.to_text());
        assert_eq!(
            back.bookmarks,
            vec![
                (PathBuf::from("/Users/a/data folder"), "REVG".to_string()),
                (PathBuf::from("/Users/a/x.sto"), "QUJD".to_string())
            ]
        );
        // Malformed lines are skipped.
        assert!(
            Settings::parse("bookmark = \"nospace\"\nbookmark = \" /x\"\n")
                .bookmarks
                .is_empty()
        );
    }

    #[std::prelude::v1::test]
    fn settings_round_trip_and_bad_values() {
        let mut s = Settings {
            theme: ThemeChoice::Dark,
            name_width: 250.0,
            overview: false,
            gappy_pct: 90,
            tree_width: 200.0,
            tree_panel: false,
            recent: Vec::new(),
            refs: Vec::new(),
            bookmarks: Vec::new(),
        };
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
        let s = Settings::parse(
            "theme = purple\nname_width = 9999\nother = 1\nno equals sign\ngappy_pct = 0\n",
        );
        assert_eq!(
            s,
            Settings {
                theme: ThemeChoice::System,
                name_width: NAME_W_MAX,
                ..Settings::default()
            }
        );
        assert_eq!(Settings::parse("gappy_pct = 101").gappy_pct, 50);
        assert_eq!(
            Settings::parse("name_width = nan").name_width,
            NAME_W_DEFAULT
        );
        assert_eq!(Settings::parse("tree_width = 5").tree_width, TREE_W_MIN);
    }

    #[std::prelude::v1::test]
    fn references_are_remembered_per_file_version() {
        let mut s = Settings::default();
        let st = FileStamp {
            size: 1234,
            mtime: 1_731_400_000_123_456_789,
        };
        let o = RefOpts {
            pin: false,
            ruler: true,
            compare: true,
        };
        let a = Path::new("/data/x y/PF00042.sto");
        s.remember_ref(a, st, Some((3, " odd name/1-90".into(), o)));
        let t = s.to_text();
        // Paths are stored absolute (on Windows "/data/…" gains a drive).
        let abs = |p: &str| std::path::absolute(p).unwrap();
        let line = format!(
            "reference = \"1234\t1731400000123456789\t3\t011\t odd name/1-90\t{}\"\n",
            abs("/data/x y/PF00042.sto").display()
        );
        assert!(t.contains(&line), "{t}");
        let back = Settings::parse(&t);
        assert_eq!(back, s);
        let m = back.remembered_ref(a, st).unwrap();
        assert_eq!((m.row, m.name.as_str(), m.opts), (3, " odd name/1-90", o));
        // Another version of the file: not applied.
        assert!(
            back.remembered_ref(
                a,
                FileStamp {
                    size: 1234,
                    mtime: 1
                }
            )
            .is_none()
        );
        assert!(
            back.remembered_ref(a, FileStamp { size: 99, ..st })
                .is_none()
        );
        assert!(
            back.remembered_ref(Path::new("/data/other.sto"), st)
                .is_none()
        );
        // The stored row if its name still matches, else a row of that name, else none.
        let names = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        assert_eq!(
            m.find_row(&names(&["a", "b", "c", " odd name/1-90"])),
            Some(3)
        );
        assert_eq!(
            m.find_row(&names(&[" odd name/1-90", "b", "c", "d"])),
            Some(0)
        );
        assert_eq!(m.find_row(&names(&["a", "b"])), None);
        // Setting again replaces the entry; forgetting removes it.
        s.remember_ref(
            a,
            FileStamp { size: 1, mtime: 2 },
            Some((0, "r".into(), RefOpts::default())),
        );
        assert_eq!(s.refs.len(), 1);
        assert!(s.remembered_ref(a, st).is_none());
        s.remember_ref(a, st, None);
        assert!(s.refs.is_empty());
        // Capped, newest first; names or paths a line can't hold are not stored.
        for i in 0..REFS_MAX + 7 {
            s.remember_ref(
                Path::new(&format!("/d/f{i}.sto")),
                st,
                Some((i, format!("r{i}"), o)),
            );
        }
        s.remember_ref(Path::new("/d/tab.sto"), st, Some((0, "a\tb".into(), o)));
        assert_eq!(s.refs.len(), REFS_MAX);
        assert_eq!(s.refs[0].path, abs(&format!("/d/f{}.sto", REFS_MAX + 6)));
        assert_eq!(Settings::parse(&s.to_text()).refs, s.refs);
        // Damaged lines are skipped.
        let bad = "reference = \"x\t1\t0\t111\tn\t/p\"\nreference = \"1\t1\t0\t12\tn\t/p\"\nreference = \"1\t1\t0\t111\tn\"\nreference = \"5\t6\t7\t100\tn\t/ok\"\n";
        let p = Settings::parse(bad);
        assert_eq!(p.refs.len(), 1);
        assert_eq!(
            (p.refs[0].row, p.refs[0].opts),
            (
                7,
                RefOpts {
                    pin: true,
                    ruler: false,
                    compare: false
                }
            )
        );
    }

    #[std::prelude::v1::test]
    fn system_follows_the_os() {
        assert!(ThemeChoice::System.palette(true).dark);
        assert!(!ThemeChoice::System.palette(false).dark);
        assert!(ThemeChoice::Dark.palette(false).dark);
        assert!(!ThemeChoice::Light.palette(true).dark);
    }
}
