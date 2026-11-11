//! Colour schemes. Each scheme gives a cell's fill from its residue code and its column, for a
//! light or a dark theme, in two strengths: pale (light theme) or dark (dark theme) fills behind
//! letters, which keep the letter ink at 4.5:1 or more, and strong fills for zooms where no
//! letters are drawn. Below one pixel per cell, tiles use the density rule ([`Colors::density`]).
//!
//! Colours are 0xRRGGBB. Nothing here depends on the GUI.

use crate::alphabet::{Alphabet, GAP, Kind, NCODES};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scheme {
    /// Clustal X groups, coloured only where the column passes the group's rule.
    ClustalX,
    /// Four steps of the accent by the share of the column that has this residue.
    PercentIdentity,
    /// Kyte–Doolittle hydropathy, blue (hydrophilic) to orange (hydrophobic).
    Hydrophobicity,
    /// A, C, G, T/U each in their own colour.
    Nucleotide,
    /// Residues that match the column consensus are faded; differences stand out.
    Differences,
}

impl Scheme {
    pub const ALL: [Scheme; 5] = [Scheme::ClustalX, Scheme::PercentIdentity, Scheme::Hydrophobicity, Scheme::Nucleotide, Scheme::Differences];

    pub fn label(self) -> &'static str {
        match self {
            Scheme::ClustalX => "Clustal X",
            Scheme::PercentIdentity => "Percent identity",
            Scheme::Hydrophobicity => "Hydrophobicity",
            Scheme::Nucleotide => "Nucleotide",
            Scheme::Differences => "Differences",
        }
    }

    /// Clustal X and hydrophobicity are for proteins, nucleotide for DNA and RNA.
    pub fn applies(self, kind: Kind) -> bool {
        match self {
            Scheme::ClustalX | Scheme::Hydrophobicity => kind == Kind::Protein,
            Scheme::Nucleotide => kind == Kind::Nucleotide,
            Scheme::PercentIdentity | Scheme::Differences => true,
        }
    }

    pub fn default_for(kind: Kind) -> Scheme {
        if kind == Kind::Protein { Scheme::ClustalX } else { Scheme::Nucleotide }
    }

    /// The settings-file name.
    pub fn key(self) -> &'static str {
        match self {
            Scheme::ClustalX => "clustalx",
            Scheme::PercentIdentity => "identity",
            Scheme::Hydrophobicity => "hydrophobicity",
            Scheme::Nucleotide => "nucleotide",
            Scheme::Differences => "differences",
        }
    }

    pub fn from_key(k: &str) -> Option<Scheme> {
        Scheme::ALL.into_iter().find(|s| s.key() == k)
    }
}

/// The scheme and the "by conservation" modifier, which fades every fill toward the background by
/// (1 − column identity).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ColorMode {
    pub scheme: Scheme,
    pub by_conservation: bool,
}

impl ColorMode {
    pub fn new(scheme: Scheme) -> Self {
        ColorMode { scheme, by_conservation: false }
    }
}

/// The theme colours a scheme needs (0xRRGGBB).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Theme {
    pub dark: bool,
    /// The view's background: gaps, and what faded fills mix toward.
    pub bg: u32,
    /// Foreground text colour (ambiguity codes, gap-majority blocks, differences at low zoom).
    pub fg: u32,
    /// The accent (percent identity).
    pub accent: u32,
}

impl Theme {
    /// Catppuccin Latte with the magenta accent, as the GUI's light palette.
    pub const LIGHT: Theme = Theme { dark: false, bg: 0xeff1f5, fg: 0x4c4f69, accent: 0x8839ef };
    /// Catppuccin Mocha with the magenta accent, as the GUI's dark palette.
    pub const DARK: Theme = Theme { dark: true, bg: 0x1e1e2e, fg: 0xcdd6f4, accent: 0xcba6f7 };

    /// Letter ink drawn on fills.
    pub fn ink(&self) -> u32 {
        if self.dark { 0xeef1fb } else { 0x2a2c3c }
    }
}

/// Linear mix of two colours; `t = 0` gives `a`, `t = 1` gives `b`.
pub fn mix(a: u32, b: u32, t: f32) -> u32 {
    let ch = |s: u32| {
        let (x, y) = (((a >> s) & 255) as f32, ((b >> s) & 255) as f32);
        ((x + (y - x) * t).round().clamp(0.0, 255.0) as u32) << s
    };
    ch(16) | ch(8) | ch(0)
}

/// WCAG 2 contrast ratio between two opaque colours (1 to 21).
pub fn contrast(a: u32, b: u32) -> f32 {
    let lin = |c: u32, s: u32| {
        let v = ((c >> s) & 0xff) as f32 / 255.0;
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    let l = |c| 0.2126 * lin(c, 16) + 0.7152 * lin(c, 8) + 0.0722 * lin(c, 0);
    let (x, y) = (l(a), l(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

/// Clustal X residue groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Hydrophobic,
    Positive,
    Negative,
    Polar,
    Cysteine,
    Glycine,
    Proline,
    Aromatic,
    Other,
}

impl Group {
    pub const LEGEND: [(Group, &'static str); 8] = [
        (Group::Hydrophobic, "Hydrophobic"),
        (Group::Positive, "Positive"),
        (Group::Negative, "Negative"),
        (Group::Polar, "Polar"),
        (Group::Aromatic, "Aromatic H Y"),
        (Group::Glycine, "Glycine"),
        (Group::Proline, "Proline"),
        (Group::Cysteine, "Cysteine"),
    ];

    pub fn of(sym: u8) -> Group {
        match sym {
            b'A' | b'I' | b'L' | b'M' | b'F' | b'W' | b'V' => Group::Hydrophobic,
            b'K' | b'R' => Group::Positive,
            b'E' | b'D' => Group::Negative,
            b'N' | b'Q' | b'S' | b'T' => Group::Polar,
            b'C' => Group::Cysteine,
            b'G' => Group::Glycine,
            b'P' => Group::Proline,
            b'H' | b'Y' => Group::Aromatic,
            _ => Group::Other,
        }
    }

    /// (light letter fill, light strong, dark letter fill, dark strong). `Other` is derived from
    /// the theme instead (see [`Colors::new`]).
    pub(crate) fn colors(self) -> [u32; 4] {
        match self {
            Group::Hydrophobic => [0xA9C0F3, 0x7FA6E8, 0x3A5590, 0x6F93E0],
            Group::Positive => [0xF3A6A0, 0xE8706A, 0x83373E, 0xE06C75],
            Group::Negative => [0xDCAEE0, 0xC77BD6, 0x6E3F78, 0xB77FD0],
            Group::Polar => [0xA6DCA8, 0x6CCB6C, 0x2F6B48, 0x5FB878],
            Group::Cysteine => [0xF4BCBC, 0xF2A0A0, 0x7C4B58, 0xD9909A],
            Group::Glycine => [0xF5C79E, 0xF2B24C, 0x80592F, 0xE0A060],
            Group::Proline => [0xDEDC94, 0xD9D94A, 0x66662A, 0xC9C25A],
            Group::Aromatic => [0x9FD6D6, 0x4CC2C2, 0x2A6970, 0x4FB3B8],
            Group::Other => [0, 0, 0, 0],
        }
    }

    /// The legend swatch: the letter fill of the theme.
    pub fn swatch(self, theme: &Theme) -> u32 {
        let c = self.colors();
        if theme.dark { c[2] } else { c[0] }
    }
}

/// Nucleotide colours: (light letter fill, light strong, dark letter fill, dark strong).
fn nt_colors(sym: u8) -> Option<[u32; 4]> {
    Some(match sym {
        b'A' => [0x8FD18F, 0x5BBF5B, 0x2F6B3A, 0x5FB86A],
        b'C' => [0x9DBAF2, 0x4F86D9, 0x2F4F8A, 0x5B86D9],
        b'G' => [0xF5C38A, 0xF2A33A, 0x7A5326, 0xE0A050],
        b'T' | b'U' => [0xF2A0A6, 0xE0525B, 0x7D3038, 0xE06070],
        _ => return None,
    })
}

pub const NT_LEGEND: [(u8, &str); 4] = [(b'A', "A"), (b'C', "C"), (b'G', "G"), (b'T', "T / U")];

pub fn nt_swatch(sym: u8, theme: &Theme) -> u32 {
    nt_colors(sym).map_or(theme.bg, |c| if theme.dark { c[2] } else { c[0] })
}

/// Kyte–Doolittle hydropathy.
pub fn kyte_doolittle(sym: u8) -> Option<f32> {
    Some(match sym {
        b'I' => 4.5,
        b'V' => 4.2,
        b'L' => 3.8,
        b'F' => 2.8,
        b'C' => 2.5,
        b'M' => 1.9,
        b'A' => 1.8,
        b'G' => -0.4,
        b'T' => -0.7,
        b'S' => -0.8,
        b'W' => -0.9,
        b'Y' => -1.3,
        b'P' => -1.6,
        b'H' => -3.2,
        b'E' | b'Q' | b'D' | b'N' => -3.5,
        b'K' => -3.9,
        b'R' => -4.5,
        _ => return None,
    })
}

/// Hydrophobicity scale anchors: (hydrophilic, neutral, hydrophobic) for letter fills and strong
/// fills.
fn hydro_anchors(theme: &Theme, letters: bool) -> (u32, u32, u32) {
    match (theme.dark, letters) {
        (false, true) => (0x9DBAF2, 0xDCDFE7, 0xF5C38A),
        (false, false) => (0x4F86D9, mix(theme.bg, theme.fg, 0.12), 0xF2A33A),
        (true, true) => (0x2F4F8A, 0x3B3D50, 0x7A5326),
        (true, false) => (0x5B86D9, mix(theme.bg, theme.fg, 0.15), 0xE0A050),
    }
}

pub fn hydro_color(kd: f32, theme: &Theme, letters: bool) -> u32 {
    let (lo, mid, hi) = hydro_anchors(theme, letters);
    let t = (kd / 4.5).clamp(-1.0, 1.0);
    if t < 0.0 { mix(mid, lo, -t) } else { mix(mid, hi, t) }
}

/// Percent identity steps (more than 80, 60, 40% of the column), as mixes of the background
/// toward the accent.
fn pid_steps(theme: &Theme, letters: bool) -> [u32; 3] {
    let t = match (theme.dark, letters) {
        (false, true) => [0.55, 0.40, 0.25],
        (true, true) => [0.45, 0.32, 0.20],
        (false, false) => [0.85, 0.60, 0.35],
        (true, false) => [0.80, 0.55, 0.30],
    };
    t.map(|t| mix(theme.bg, theme.accent, t))
}

/// Percent identity step of a share (0–255 of the column's rows): 0 = more than 80%, 1 = more
/// than 60%, 2 = more than 40%, `None` below.
pub fn pid_step(share: u8) -> Option<usize> {
    let s = share as u32 * 100;
    if s > 80 * 255 {
        Some(0)
    } else if s > 60 * 255 {
        Some(1)
    } else if s > 40 * 255 {
        Some(2)
    } else {
        None
    }
}

/// Bit set in a Clustal X column mask when C passes the hydrophobic rule (blue, not pink).
pub const CYS_BLUE: u32 = 1 << 31;

/// Which codes a protein column colours under Clustal X, as a bit mask over codes (bit `code`),
/// from the column's code counts over `n` rows (gaps count toward `n`). The rules are the table in
/// Jalview's Clustal X documentation; "more than" is strict:
///
/// | Residue | Colour | Coloured when |
/// |---|---|---|
/// | A I L M F W V | blue | WLVIMAFCHP > 60% |
/// | C | pink | C > 85% (checked first) |
/// | C | blue | WLVIMAFCHP > 60% |
/// | K R | red | KR > 60%, or one of K, R, Q > 85% |
/// | E | magenta | KR > 60%, QE > 50%, or one of E, Q, D > 85% |
/// | D | magenta | KR > 60%, one of K, R, Q > 85%, or ED > 50% |
/// | N | green | N > 50%, or one of N, Y > 85% |
/// | Q | green | KR > 60%, QE > 50%, or one of Q, E, K, R > 85% |
/// | S T | green | WLVIMAFCHP > 60%, TS > 50%, or one of S, T > 85% |
/// | G | orange | always |
/// | P | yellow | always |
/// | H Y | cyan | WLVIMAFCHP > 60%, or one of W, Y, A, C, P, Q, F, H, I, L, M, V > 85% |
pub fn clustal_mask(alpha: &Alphabet, counts: &[u32; NCODES], n: u32) -> u32 {
    if alpha.kind != Kind::Protein || n == 0 {
        return 0;
    }
    let count = |s: &[u8]| -> u64 { s.iter().map(|&b| alpha.encode(b).map_or(0, |c| counts[c as usize] as u64)).sum() };
    // share > pct%, in integers: count × 100 > pct × n
    let over = |s: &[u8], pct: u64| count(s) * 100 > pct * n as u64;
    let any85 = |s: &[u8]| s.iter().any(|&b| over(&[b], 85));
    let hyd = over(b"WLVIMAFCHP", 60);
    let kr = over(b"KR", 60);
    let qe = over(b"QE", 50);
    let mut mask = 0u32;
    let mut set = |syms: &[u8], on: bool| {
        if on {
            for &b in syms {
                if let Some(c) = alpha.encode(b) {
                    mask |= 1 << c;
                }
            }
        }
    };
    set(b"AILMFWV", hyd);
    set(b"C", hyd || over(b"C", 85));
    set(b"KR", kr || any85(b"KRQ"));
    set(b"E", kr || qe || any85(b"EQD"));
    set(b"D", kr || any85(b"KRQ") || over(b"ED", 50));
    set(b"N", over(b"N", 50) || any85(b"NY"));
    set(b"Q", kr || qe || any85(b"QEKR"));
    set(b"ST", hyd || over(b"TS", 50) || any85(b"ST"));
    set(b"GP", true);
    set(b"HY", hyd || any85(b"WYACPQFHILMV"));
    // C is in the hydrophobic group, so > 85% C always passes the blue rule too: pink first.
    if hyd && !over(b"C", 85) {
        mask |= CYS_BLUE;
    }
    mask
}

/// The legend of a scheme: swatches (letter fills of the theme) and their labels.
pub fn legend(scheme: Scheme, kind: Kind, theme: &Theme) -> Vec<(u32, &'static str)> {
    match scheme {
        Scheme::ClustalX if kind == Kind::Protein => {
            Group::LEGEND.iter().map(|&(g, l)| (g.swatch(theme), l)).collect()
        }
        Scheme::PercentIdentity => {
            let s = pid_steps(theme, true);
            vec![(s[0], "More than 80%"), (s[1], "More than 60%"), (s[2], "More than 40%")]
        }
        Scheme::Hydrophobicity => vec![
            (hydro_color(4.5, theme, true), "Hydrophobic"),
            (hydro_color(0.0, theme, true), "Neutral"),
            (hydro_color(-4.5, theme, true), "Hydrophilic"),
        ],
        Scheme::Differences => {
            let c = if kind == Kind::Protein { Group::Hydrophobic.swatch(theme) } else { nt_swatch(b'A', theme) };
            vec![(c, "Differs from consensus"), (mix(c, theme.bg, 0.8), "Matches consensus")]
        }
        _ => {
            let mut v: Vec<(u32, &'static str)> = NT_LEGEND.iter().map(|&(s, l)| (nt_swatch(s, theme), l)).collect();
            v.push((mix(theme.bg, theme.fg, 0.25), "Other"));
            v
        }
    }
}

/// What a scheme needs to know about one column.
#[derive(Clone, Copy, Debug)]
pub struct Column<'a> {
    pub consensus: u8,
    /// Share of rows carrying the consensus residue.
    pub identity: f32,
    /// Share of each code, 0–255 of the column's rows.
    pub shares: &'a [u8; NCODES],
    /// Clustal X mask ([`clustal_mask`]).
    pub clustal: u32,
}

/// One scheme's colours for one alphabet and theme, at letter or strong strength.
pub struct Colors {
    pub mode: ColorMode,
    pub theme: Theme,
    pub letters: bool,
    kind: Kind,
    missing: u8,
    cys: Option<u8>,
    /// Group or nucleotide colour per code, at this strength.
    base: [u32; NCODES],
    /// The same at strong strength, for density hues.
    strong: [u32; NCODES],
    /// Pink for C under Clustal X (its group colour), and the hydrophobic blue.
    cys_pink: u32,
    hyd_blue: u32,
    hydro: [u32; NCODES],
    hydro_strong: [u32; NCODES],
    pid: [u32; 3],
}

impl Colors {
    pub fn new(alpha: &Alphabet, mode: ColorMode, theme: Theme, letters: bool) -> Self {
        let idx = |strong: bool| match (theme.dark, strong) {
            (false, false) => 0,
            (false, true) => 1,
            (true, false) => 2,
            (true, true) => 3,
        };
        // Ambiguity codes and other residues: the background with some foreground.
        let other = |strong: bool| mix(theme.bg, theme.fg, if strong { 0.35 } else { 0.25 });
        let table = |strong: bool| -> [u32; NCODES] {
            let mut t = [other(strong); NCODES];
            t[GAP as usize] = theme.bg;
            for (code, slot) in t.iter_mut().enumerate().take(alpha.len()).skip(1) {
                let s = alpha.symbol(code as u8);
                let c = match alpha.kind {
                    Kind::Nucleotide => nt_colors(s),
                    Kind::Protein => Some(Group::of(s)).filter(|g| *g != Group::Other).map(Group::colors),
                };
                if let Some(c) = c {
                    *slot = c[idx(strong)];
                }
            }
            t
        };
        let hydro_table = |strong: bool| -> [u32; NCODES] {
            let mut t = [hydro_anchors(&theme, !strong).1; NCODES];
            t[GAP as usize] = theme.bg;
            for (code, slot) in t.iter_mut().enumerate().take(alpha.len()).skip(1) {
                if let Some(kd) = kyte_doolittle(alpha.symbol(code as u8)) {
                    *slot = hydro_color(kd, &theme, !strong);
                }
            }
            t
        };
        let base = table(!letters);
        let strong = table(true);
        Colors {
            mode,
            theme,
            letters,
            kind: alpha.kind,
            missing: crate::pyramid::missing_code(alpha.kind),
            cys: alpha.encode(b'C').filter(|_| alpha.kind == Kind::Protein),
            cys_pink: Group::Cysteine.colors()[idx(!letters)],
            hyd_blue: Group::Hydrophobic.colors()[idx(!letters)],
            hydro: hydro_table(!letters),
            hydro_strong: hydro_table(true),
            pid: pid_steps(&theme, letters),
            base,
            strong,
        }
    }

    /// Fill of residue `code` in a column, or `None` where the scheme leaves the cell unfilled
    /// (the background shows, and letters are drawn in the muted text colour).
    #[inline]
    pub fn fill(&self, code: u8, col: &Column) -> Option<u32> {
        if code == GAP {
            return None;
        }
        let i = code as usize & (NCODES - 1);
        let c = match self.mode.scheme {
            Scheme::ClustalX if self.kind == Kind::Protein => {
                if col.clustal & (1 << i) == 0 {
                    return None;
                }
                if Some(code) == self.cys && col.clustal & CYS_BLUE != 0 { self.hyd_blue } else if Some(code) == self.cys { self.cys_pink } else { self.base[i] }
            }
            Scheme::PercentIdentity => self.pid[pid_step(col.shares[i])?],
            Scheme::Hydrophobicity if self.kind == Kind::Protein => self.hydro[i],
            Scheme::Differences => {
                if code == col.consensus || code == self.missing {
                    mix(self.base[i], self.theme.bg, 0.8)
                } else {
                    self.base[i]
                }
            }
            _ => self.base[i],
        };
        Some(if self.mode.by_conservation { mix(c, self.theme.bg, (1.0 - col.identity).clamp(0.0, 1.0)) } else { c })
    }

    /// [`Colors::fill`], with the background where there is none.
    #[inline]
    pub fn cell(&self, code: u8, col: &Column) -> u32 {
        self.fill(code, col).unwrap_or(self.theme.bg)
    }

    /// Colour of a block of cells at less than one pixel per cell, from its majority code and the
    /// share (0–255) of its cells that differ from their column's consensus:
    /// - a residue majority: the background mixed toward the majority's strong hue by
    ///   `0.08 + 0.72 × (1 − diff)`, so conserved blocks are strong and variable ones faint;
    /// - a gap majority: the background with 3% of the foreground.
    ///
    /// Differences keeps its own rule: the faded majority colour, mixed toward the foreground by
    /// how much of the block differs.
    pub fn density(&self, maj: u8, diff: u8) -> u32 {
        let bg = self.theme.bg;
        if maj == GAP {
            return mix(bg, self.theme.fg, 0.03);
        }
        let i = maj as usize & (NCODES - 1);
        let d = diff as f32 / 255.0;
        let hue = match self.mode.scheme {
            Scheme::PercentIdentity => self.theme.accent,
            Scheme::Hydrophobicity if self.kind == Kind::Protein => self.hydro_strong[i],
            Scheme::Differences => {
                let base = mix(self.strong[i], bg, 0.8);
                // sqrt so that a few differing cells in a large block are still visible.
                let t = if diff == 0 { 0.0 } else { d.sqrt().clamp(0.25, 1.0) };
                return mix(base, self.theme.fg, t);
            }
            _ => self.strong[i],
        };
        mix(bg, hue, 0.08 + 0.72 * (1.0 - d))
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alphabet::{DNA, NT_MIXED, PROTEIN, RNA};

    fn counts(alpha: &Alphabet, col: &[u8]) -> ([u32; NCODES], u32) {
        let mut c = [0u32; NCODES];
        for &b in col {
            c[alpha.encode(b).unwrap() as usize] += 1;
        }
        (c, col.len() as u32)
    }

    fn colored(col: &[u8], sym: u8) -> bool {
        let (c, n) = counts(&PROTEIN, col);
        clustal_mask(&PROTEIN, &c, n) & (1 << PROTEIN.encode(sym).unwrap()) != 0
    }

    #[test]
    fn clustal_rules() {
        // 7 of 10 hydrophobic (WLVIMAFCHP): A, L, V and the S/T and H/Y rules all fire.
        let col = b"AALLVVIKST";
        assert!(colored(col, b'A') && colored(col, b'S') && colored(col, b'T'));
        assert!(!colored(col, b'K')); // KR 10%
        // exactly 60% hydrophobic is not "more than"
        assert!(!colored(b"AAALLLKKST", b'A'));
        // KR 70%: K, R, E, D and Q are coloured
        let col = b"KKKKRRREQD";
        for s in *b"KREDQ" {
            assert!(colored(col, s), "{}", s as char);
        }
        assert!(!colored(col, b'N'));
        // G and P always
        assert!(colored(b"G---------", b'G') && colored(b"P---------", b'P'));
        // C: pink at > 85% C, blue with the hydrophobic group, else none
        let (c, n) = counts(&PROTEIN, b"CCCCCCCCCK");
        let m = clustal_mask(&PROTEIN, &c, n);
        assert!(m & (1 << PROTEIN.encode(b'C').unwrap()) != 0 && m & CYS_BLUE == 0);
        let (c, n) = counts(&PROTEIN, b"CLLLLLLLKK");
        let m = clustal_mask(&PROTEIN, &c, n);
        assert!(m & CYS_BLUE != 0);
        assert!(!colored(b"CCCKKKEEDD", b'C'));
        // gaps count toward the column: 5 L of 10 rows is not > 60%
        assert!(!colored(b"LLLLL-----", b'L'));
        // nucleotides have no Clustal X colouring
        let (c, n) = counts(&DNA, b"AAAA");
        assert_eq!(clustal_mask(&DNA, &c, n), 0);
    }

    #[test]
    fn pid_steps_by_share() {
        assert_eq!(pid_step(255), Some(0));
        assert_eq!(pid_step(205), Some(0)); // 80.4%
        assert_eq!(pid_step(204), Some(1)); // 80.0% is not more than 80
        assert_eq!(pid_step(103), Some(2));
        assert_eq!(pid_step(102), None);
    }

    /// Every fill that letters are drawn on keeps the theme's ink at 4.5:1 or more (WCAG AA), for
    /// every scheme, alphabet, theme and code, with and without the conservation fade; and the
    /// muted text drawn on unfilled cells is readable on the background (checked in the GUI's
    /// token test).
    #[test]
    fn letters_readable_on_every_fill() {
        let shares_all: Vec<[u8; NCODES]> = [255u8, 210, 160, 110, 0].iter().map(|&s| [s; NCODES]).collect();
        for theme in [Theme::LIGHT, Theme::DARK] {
            let ink = theme.ink();
            for alpha in [&DNA, &RNA, &NT_MIXED, &PROTEIN] {
                for scheme in Scheme::ALL {
                    for by_conservation in [false, true] {
                        let colors = Colors::new(alpha, ColorMode { scheme, by_conservation }, theme, true);
                        for shares in &shares_all {
                            for identity in [1.0, 0.6, 0.2, 0.0] {
                                for (consensus, clustal) in [(1u8, u32::MAX), (2, !CYS_BLUE)] {
                                    let col = Column { consensus, identity, shares, clustal };
                                    for code in 1..alpha.len() as u8 {
                                        if let Some(f) = colors.fill(code, &col) {
                                            let c = contrast(f, ink);
                                            assert!(c >= 4.5, "{scheme:?} cons={by_conservation} {alpha:?} {} dark={}: {f:06x} {c:.2}", alpha.symbol(code) as char, theme.dark);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // the ink on the background itself (a faded fill can come close to it)
            assert!(contrast(theme.bg, ink) >= 4.5);
        }
    }

    /// The spec's Clustal X fills (redesign spec §2.4) and the measured nucleotide ones.
    #[test]
    fn spec_fills() {
        let c = Colors::new(&PROTEIN, ColorMode::new(Scheme::Hydrophobicity), Theme::LIGHT, true);
        let col_shares = [0u8; NCODES];
        let col = Column { consensus: 0, identity: 1.0, shares: &col_shares, clustal: 0 };
        // the most hydrophobic residue is the orange end, the most hydrophilic the blue end
        assert_eq!(c.cell(PROTEIN.encode(b'I').unwrap(), &col), 0xF5C38A);
        assert_eq!(c.cell(PROTEIN.encode(b'R').unwrap(), &col), 0x9DBAF2);
        assert_eq!(Group::of(b'W'), Group::Hydrophobic);
        assert_eq!(Group::Hydrophobic.swatch(&Theme::DARK), 0x3A5590);
        let n = Colors::new(&RNA, ColorMode::new(Scheme::Nucleotide), Theme::DARK, true);
        assert_eq!(n.cell(RNA.encode(b'U').unwrap(), &col), 0x7D3038);
        assert_eq!(n.cell(GAP, &col), Theme::DARK.bg);
    }

    /// Density strength falls as the share of differing cells rises, in every scheme and theme.
    #[test]
    fn density_monotone_in_diff() {
        for theme in [Theme::LIGHT, Theme::DARK] {
            for scheme in Scheme::ALL {
                let c = Colors::new(&PROTEIN, ColorMode::new(scheme), theme, false);
                let code = PROTEIN.encode(b'L').unwrap();
                let dist = |d: u8| contrast(c.density(code, d), theme.bg);
                let mut last = if scheme == Scheme::Differences { 0.0 } else { f32::INFINITY };
                for d in (0..=255u8).step_by(15) {
                    let x = dist(d);
                    if scheme == Scheme::Differences {
                        // differences: more differing cells stand out more
                        assert!(x >= last - 1e-3, "{scheme:?} dark={} d={d}", theme.dark);
                    } else {
                        assert!(x <= last + 1e-3, "{scheme:?} dark={} d={d}", theme.dark);
                    }
                    last = x;
                }
                assert_eq!(c.density(GAP, 0), mix(theme.bg, theme.fg, 0.03));
            }
        }
    }

    #[test]
    fn schemes_by_kind() {
        assert_eq!(Scheme::default_for(Kind::Protein), Scheme::ClustalX);
        assert_eq!(Scheme::default_for(Kind::Nucleotide), Scheme::Nucleotide);
        assert!(!Scheme::ClustalX.applies(Kind::Nucleotide) && Scheme::Differences.applies(Kind::Nucleotide));
        for s in Scheme::ALL {
            assert_eq!(Scheme::from_key(s.key()), Some(s));
        }
    }
}
