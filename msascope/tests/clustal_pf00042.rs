//! Clustal X column rules on real data: 40 sequences × 20 columns (61–80) of the Pfam PF00042
//! (Globin) seed, in `tests/data/` (Pfam, CC0). The expected colouring below was worked out by
//! counting each column by hand; percentages are of all 40 rows, gaps included.

use msascope::alphabet::Kind;
use msascope::scheme::{ColorMode, Colors, Scheme, Theme};
use msascope::stats::ColumnStats;

fn load() -> (msascope::Alignment, ColumnStats) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/pf00042_seed_40x20.sto");
    let a = msascope::parse::load(&path).unwrap();
    let s = ColumnStats::compute(&a);
    (a, s)
}

/// Whether residue `sym` is coloured in fixture column `k` (original column 61 + k).
fn colored(a: &msascope::Alignment, s: &ColumnStats, k: usize, sym: u8) -> bool {
    s.clustal[k] & (1 << a.alphabet().encode(sym).unwrap()) != 0
}

#[test]
fn clustal_x_on_pf00042_seed() {
    let (a, s) = load();
    assert_eq!((a.rows(), a.cols(), a.kind()), (40, 20, Kind::Protein));
    // Column 71: L 27, F 6, V 2, I 1, Y 3, one gap. WLVIMAFCHP is 36 of 40 (90%) > 60%: the
    // hydrophobic residues are blue, and Y (aromatic) is cyan by the same rule.
    for sym in *b"LFVIY" {
        assert!(colored(&a, &s, 10, sym), "{}", sym as char);
    }
    // Column 72: A 14, S 8, K 6, … WLVIMAFCHP 17 (42%), TS 9 (22%), KR 6 (15%): nothing passes
    // except G, which is always coloured.
    for sym in *b"ASKD" {
        assert!(!colored(&a, &s, 11, sym), "{}", sym as char);
    }
    assert!(colored(&a, &s, 11, b'G'));
    // Column 75: S 19 of 40 is 47.5%, not more than 50%: S is not coloured, nor A (14, 35%).
    assert!(!colored(&a, &s, 14, b'S') && !colored(&a, &s, 14, b'A') && colored(&a, &s, 14, b'G'));
    // Column 76: D 9 + E 7 = 40% (not > 50%), KR 9 (22%): neither D nor E is coloured.
    assert!(!colored(&a, &s, 15, b'D') && !colored(&a, &s, 15, b'E'));
    // Column 78: H in all 40 rows: H is in WLVIMAFCHP, so it is coloured (cyan).
    assert!(colored(&a, &s, 17, b'H'));
    // Column 62: P is always coloured; K 11 + R 1 = 30% is not enough for K.
    assert!(colored(&a, &s, 1, b'P') && !colored(&a, &s, 1, b'K'));
    // Column 69: G always; TS 11 (27.5%) is not enough for S or T.
    assert!(colored(&a, &s, 8, b'G') && !colored(&a, &s, 8, b'S') && !colored(&a, &s, 8, b'T'));

    // The fills: a coloured L at letter zoom is the hydrophobic fill; an uncoloured A shows the
    // background (no fill), in both themes.
    for theme in [Theme::LIGHT, Theme::DARK] {
        let c = Colors::new(a.alphabet(), ColorMode::new(Scheme::ClustalX), theme, true);
        let col = |k: usize| msascope::scheme::Column { consensus: s.consensus[k], identity: s.identity[k], shares: &s.shares[k], clustal: s.clustal[k] };
        let code = |b| a.alphabet().encode(b).unwrap();
        assert_eq!(c.fill(code(b'L'), &col(10)), Some(if theme.dark { 0x3A5590 } else { 0xA9C0F3 }));
        assert_eq!(c.fill(code(b'A'), &col(11)), None);
    }
}

/// Column 80 is `.` in every row: an insert column. Columns 66–68 are `.` in 39 rows and an
/// upper-case residue in the last: match columns.
#[test]
fn seed_insert_columns() {
    let (a, _) = load();
    assert_eq!((0..a.cols()).filter(|&c| a.is_insert(c)).collect::<Vec<_>>(), [19]);
}
