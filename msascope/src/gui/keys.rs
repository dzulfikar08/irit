// Adapted from Kantor (github.com/dzulfikar08/kantor) by the same author, relicensed
// GPL-3.0-or-later for irit.
//! Platform shortcuts: bindings and labels are written once with `ctrl-` / "Ctrl", and become
//! Cmd (⌘) on macOS.
//!
//! GPUI's own `secondary-` alias can't be used: `KeyBinding::new` parses without an OS and
//! rejects it.
#![allow(dead_code)]

/// A binding written with `ctrl-` for this platform: `cmd-` on macOS.
/// Ctrl+Tab / Ctrl+Shift+Tab stay Ctrl (Cmd+Tab is the macOS app switcher).
pub fn primary(keys: &'static str) -> &'static str {
    if cfg!(target_os = "macos") {
        to_mac(keys).map_or(keys, |s| Box::leak(s.into_boxed_str()))
    } else {
        keys
    }
}

fn to_mac(keys: &str) -> Option<String> {
    if !keys.contains("ctrl-") || keys.contains("tab") {
        return None;
    }
    Some(keys.replace("ctrl-", "cmd-"))
}

/// A shortcut label written for Linux ("Ctrl+Shift+B", "Ctrl K") as shown on this platform:
/// "⌘⇧B", "⌘K" on macOS.
pub fn label(keys: &str) -> String {
    if cfg!(target_os = "macos") {
        mac_label(keys)
    } else {
        keys.to_owned()
    }
}

fn mac_label(keys: &str) -> String {
    if keys.contains("Tab") {
        return keys.replace("Ctrl", "⌃");
    }
    let mut out = String::new();
    // Labels list several shortcuts as "Ctrl+Z / Ctrl+Y"; keep separators.
    for (i, part) in keys.split(" / ").enumerate() {
        if i > 0 {
            out.push_str(" / ");
        }
        let words: Vec<&str> = part.split(['+', ' ']).filter(|w| !w.is_empty()).collect();
        let mut mods = String::new();
        let mut rest = Vec::new();
        for w in words {
            match w {
                "Ctrl" | "Super" | "Cmd" => mods.push('⌘'),
                "Shift" => mods.push('⇧'),
                "Alt" => mods.push('⌥'),
                other => rest.push(other),
            }
        }
        // Order modifiers as macOS does: ⌃⌥⇧⌘.
        let order = ['⌃', '⌥', '⇧', '⌘'];
        let mut sorted: Vec<char> = mods.chars().collect();
        sorted.sort_by_key(|c| order.iter().position(|o| o == c));
        sorted.dedup();
        out.extend(sorted);
        out.push_str(&rest.join(" "));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[std::prelude::v1::test]
    fn bindings_become_cmd_on_mac_except_tab() {
        assert_eq!(to_mac("ctrl-shift-b").as_deref(), Some("cmd-shift-b"));
        assert_eq!(to_mac("ctrl-tab"), None);
        assert_eq!(to_mac("super-q"), None);
    }

    #[std::prelude::v1::test]
    fn labels_use_mac_symbols() {
        assert_eq!(mac_label("Ctrl K"), "⌘K");
        assert_eq!(mac_label("Ctrl+Shift+B"), "⇧⌘B");
        assert_eq!(mac_label("Ctrl+Alt+N"), "⌥⌘N");
        assert_eq!(mac_label("Ctrl+Z / Ctrl+Y"), "⌘Z / ⌘Y");
        assert_eq!(mac_label("F4"), "F4");
        assert_eq!(mac_label("Ctrl+Tab"), "⌃+Tab");
    }
}
