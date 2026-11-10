// Adapted from Kantor (github.com/dzulfikar08/kantor) by the same author, relicensed
// GPL-3.0-or-later for irit.
//! Palettes: the handful of base colours the UI tokens are derived from. No GPUI types here.
//!
//! The two built-in palettes are Catppuccin Latte (light) and Mocha (dark), MIT-licensed (see
//! THIRD_PARTY.md), with the mauve/magenta hue as the accent.

/// A 24-bit RGB colour, `0xRRGGBB`.
pub type Rgb = u32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Palette {
    pub dark: bool,
    pub background: Rgb,
    pub foreground: Rgb,
    pub accent: Rgb,
    pub red: Rgb,
    pub green: Rgb,
    pub yellow: Rgb,
    pub blue: Rgb,
    pub magenta: Rgb,
    pub cyan: Rgb,
}

/// Catppuccin Latte.
pub const LIGHT: Palette = Palette {
    dark: false,
    background: 0xeff1f5,
    foreground: 0x4c4f69,
    accent: 0x8839ef,
    red: 0xd20f39,
    green: 0x40a02b,
    yellow: 0xdf8e1d,
    blue: 0x1e66f5,
    magenta: 0x8839ef,
    cyan: 0x179299,
};

/// Catppuccin Mocha.
pub const DARK: Palette = Palette {
    dark: true,
    background: 0x1e1e2e,
    foreground: 0xcdd6f4,
    accent: 0xcba6f7,
    red: 0xf38ba8,
    green: 0xa6e3a1,
    yellow: 0xf9e2af,
    blue: 0x89b4fa,
    magenta: 0xcba6f7,
    cyan: 0x94e2d5,
};

/// Parse `#rrggbb` / `rrggbb` (an optional alpha byte is ignored).
#[cfg_attr(not(test), allow(dead_code))]
pub fn parse_hex(s: &str) -> Option<Rgb> {
    let h = s.trim().trim_start_matches('#');
    let h = match h.len() {
        6 => h,
        8 => &h[..6],
        _ => return None,
    };
    u32::from_str_radix(h, 16).ok()
}

#[cfg_attr(not(test), allow(dead_code))]
/// Relative luminance in 0..=1 (sRGB, not linearised; good enough to pick light or dark).
pub fn luminance(c: Rgb) -> f32 {
    let r = ((c >> 16) & 0xff) as f32;
    let g = ((c >> 8) & 0xff) as f32;
    let b = (c & 0xff) as f32;
    (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255.0
}

/// Linear mix of two colours; `t = 0` gives `a`, `t = 1` gives `b`.
pub fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    let ch = |c: Rgb, s: u32| ((c >> s) & 0xff) as f32;
    let m = |s| (ch(a, s) + (ch(b, s) - ch(a, s)) * t).round().clamp(0.0, 255.0) as u32;
    (m(16) << 16) | (m(8) << 8) | m(0)
}

/// WCAG 2 contrast ratio between two opaque colours (1 to 21).
#[cfg_attr(not(test), allow(dead_code))]
pub fn contrast(a: Rgb, b: Rgb) -> f32 {
    let lin = |c: Rgb, s: u32| {
        let v = ((c >> s) & 0xff) as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let l = |c| 0.2126 * lin(c, 16) + 0.7152 * lin(c, 8) + 0.0722 * lin(c, 0);
    let (x, y) = (l(a), l(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[std::prelude::v1::test]
    fn hex_parsing() {
        assert_eq!(parse_hex("#a1B2c3"), Some(0xa1b2c3));
        assert_eq!(parse_hex("a1b2c3ff"), Some(0xa1b2c3));
        assert_eq!(parse_hex("#abc"), None);
        assert_eq!(parse_hex("#zzzzzz"), None);
    }

    #[std::prelude::v1::test]
    fn mixing() {
        assert_eq!(mix(0x000000, 0xffffff, 0.0), 0x000000);
        assert_eq!(mix(0x000000, 0xffffff, 1.0), 0xffffff);
        assert_eq!(mix(0x000000, 0xfefefe, 0.5), 0x7f7f7f);
    }

    #[std::prelude::v1::test]
    fn contrast_ratio() {
        assert!((contrast(0x000000, 0xffffff) - 21.0).abs() < 0.01);
        assert!((contrast(0x777777, 0x777777) - 1.0).abs() < 1e-6);
        assert!(luminance(LIGHT.background) > 0.5 && luminance(DARK.background) < 0.5);
    }
}
