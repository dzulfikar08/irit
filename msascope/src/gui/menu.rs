//! The application menu bar (macOS), app-wide shortcuts, and what happens when the window closes
//! or the Dock icon is clicked.
//!
//! Window-level actions (open, close, export) are handled by `MsaScope` on its root element; the
//! app-level handlers here only run when no window took the action, which is when no window is open.

use gpui_kit::*;

use super::keys::primary;

actions!(
    msascope,
    [
        About,
        Acknowledgements,
        Documentation,
        Hide,
        HideOthers,
        ShowAll,
        Quit,
        Open,
        OpenSample,
        CloseFile,
        CloseWindow,
        ExportSelection,
        CopySelection,
        SelectAllCells,
    ]
);

pub const DOCS_URL: &str = "https://dzulfikar08.github.io/irit/tools/msascope/";

/// Key bindings, the menu bar, and the app-level handlers. `open_window` opens the main window
/// (File > Open with no window open).
pub fn install(cx: &mut App, open_window: impl Fn(&mut App) + 'static) {
    cx.bind_keys([
        KeyBinding::new(primary("ctrl-q"), Quit, None),
        KeyBinding::new(primary("ctrl-o"), Open, None),
        KeyBinding::new(primary("ctrl-w"), CloseWindow, None),
        KeyBinding::new(primary("ctrl-e"), ExportSelection, None),
        KeyBinding::new(primary("ctrl-c"), CopySelection, Some("Alignment")),
        KeyBinding::new(primary("ctrl-a"), SelectAllCells, Some("Alignment")),
    ]);
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("alt-cmd-h", HideOthers, None),
        ]);
    }

    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Documentation, cx| cx.open_url(DOCS_URL));
    cx.on_action(|_: &Acknowledgements, cx| open_acknowledgements(cx));
    // With no window open, File > Open (or Cmd+O) opens one first.
    cx.on_action(move |_: &Open, cx| {
        if cx.windows().is_empty() {
            open_window(cx);
            cx.activate(true);
        }
    });

    // macOS apps stay running without a window (a click on the Dock icon brings one back, see
    // main.rs). Elsewhere, closing the last window quits.
    if !cfg!(target_os = "macos") {
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
    }

    cx.set_menus(menus());
}

fn menus() -> Vec<Menu> {
    let mut app_items = vec![
        MenuItem::action("About msascope", About),
        MenuItem::action("Acknowledgements", Acknowledgements),
        MenuItem::separator(),
    ];
    if cfg!(target_os = "macos") {
        app_items.extend([
            MenuItem::os_submenu("Services", SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action("Hide msascope", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
        ]);
    }
    app_items.push(MenuItem::action("Quit msascope", Quit));
    vec![
        Menu::new("msascope").items(app_items),
        Menu::new("File").items([
            MenuItem::action("Open…", Open),
            MenuItem::action("Open Sample Alignment", OpenSample),
            MenuItem::separator(),
            MenuItem::action("Export…", ExportSelection),
            MenuItem::separator(),
            MenuItem::action("Close File", CloseFile),
            MenuItem::action("Close Window", CloseWindow),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Copy Selection", CopySelection),
            MenuItem::action("Select All", SelectAllCells),
        ]),
        Menu::new("Help").items([
            MenuItem::action("msascope Help", Documentation),
            MenuItem::action("Acknowledgements", Acknowledgements),
        ]),
    ]
}

/// About msascope: the version, the licence and the help link.
pub fn show_about(window: &mut Window, cx: &mut App) {
    let detail = format!(
        "Version {}\n\nA viewer for very large multiple sequence alignments.\n\n© 2026 Dzulfikar Ats Tsauri. Part of irit, \
         GPL-3.0-or-later. Acknowledgements lists the open-source code it is built from.",
        env!("CARGO_PKG_VERSION")
    );
    let answer = window.prompt(
        PromptLevel::Info,
        "msascope",
        Some(&detail),
        &["OK", "Acknowledgements"],
        cx,
    );
    cx.spawn(async move |cx| {
        if answer.await == Ok(1) {
            cx.update(open_acknowledgements);
        }
    })
    .detach();
}

/// The licences of the code msascope is built from: `Acknowledgements.html` in the app bundle's
/// Resources (macOS) or next to the binary; the online copy if it isn't there (a source build).
fn open_acknowledgements(cx: &mut App) {
    let local = std::env::current_exe().ok().and_then(|exe| {
        let dir = exe.parent()?.to_path_buf();
        [
            dir.join("../Resources/Acknowledgements.html"),
            dir.join("Acknowledgements.html"),
        ]
        .into_iter()
        .find(|p| p.is_file())
        .and_then(|p| p.canonicalize().ok())
    });
    match local {
        Some(p) => cx.open_url(&format!("file://{}", p.display())),
        None => {
            cx.open_url("https://github.com/dzulfikar08/irit/blob/main/msascope/THIRD_PARTY.md")
        }
    }
}

/// Paths from `application:openURLs:` (Finder's Open, Open With, a drop on the Dock icon).
pub fn file_urls_to_paths(urls: &[String]) -> Vec<std::path::PathBuf> {
    urls.iter()
        .filter_map(|u| u.strip_prefix("file://"))
        .map(|p| std::path::PathBuf::from(percent_decode(p)))
        .collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hex = |c: u8| (c as char).to_digit(16);
            if let (Some(h), Some(l)) = (hex(b[i + 1]), hex(b[i + 2])) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[std::prelude::v1::test]
    fn file_urls_become_paths() {
        let urls = vec![
            "file:///Users/a/My%20Data/PF00042.sto".to_string(),
            "https://x".to_string(),
            "file:///t/x%2".to_string(),
        ];
        let p = file_urls_to_paths(&urls);
        assert_eq!(
            p,
            vec![
                std::path::PathBuf::from("/Users/a/My Data/PF00042.sto"),
                std::path::PathBuf::from("/t/x%2")
            ]
        );
    }
}
