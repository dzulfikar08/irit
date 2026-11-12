//! Self-contained HTML report: inline SVG charts, no scripts, no external files.

use crate::long::{LenBin, LongReads};
use crate::stats::{PerTile, Report, Status};
use std::fmt::Write;

const W: f64 = 760.0;
const H: f64 = 260.0;
const PAD_L: f64 = 48.0;
const PAD_R: f64 = 16.0;
const PAD_T: f64 = 14.0;
const PAD_B: f64 = 36.0;

const COLORS: [&str; 5] = ["#2f6fb3", "#c2552d", "#2f8f5b", "#8a5cc2", "#7a7a7a"];

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

struct Axes {
    x0: f64,
    x1: f64,
    y0: f64,
    y1: f64,
}

impl Axes {
    fn x(&self, v: f64) -> f64 {
        let span = (self.x1 - self.x0).max(1e-9);
        PAD_L + (v - self.x0) / span * (W - PAD_L - PAD_R)
    }
    fn y(&self, v: f64) -> f64 {
        let span = (self.y1 - self.y0).max(1e-9);
        H - PAD_B - (v - self.y0) / span * (H - PAD_T - PAD_B)
    }
}

/// "Nice" tick values covering [lo, hi].
fn ticks(lo: f64, hi: f64, want: usize) -> Vec<f64> {
    let span = (hi - lo).max(1e-9);
    let raw = span / want as f64;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 5.0, 10.0].iter().map(|m| m * mag).find(|s| span / s <= want as f64).unwrap_or(mag * 10.0);
    let mut v = (lo / step).ceil() * step;
    let mut out = Vec::new();
    while v <= hi + step * 1e-6 {
        out.push(v);
        v += step;
    }
    out
}

fn fmt_tick(v: f64) -> String {
    if v.abs() >= 1e6 {
        format!("{:.1}M", v / 1e6)
    } else if v.abs() >= 1e3 {
        format!("{:.0}k", v / 1e3)
    } else if v.fract().abs() < 1e-9 {
        format!("{v:.0}")
    } else {
        format!("{v:.1}")
    }
}

fn frame(a: &Axes, xlabel: &str, ylabel: &str, bands: &str) -> String {
    let mut s = format!(r#"<svg viewBox="0 0 {W} {H}" role="img" aria-label="{}">"#, esc(ylabel));
    s.push_str(bands);
    for t in ticks(a.y0, a.y1, 5) {
        let y = a.y(t);
        let _ = write!(
            s,
            r##"<line x1="{PAD_L}" x2="{}" y1="{y:.1}" y2="{y:.1}" stroke="#e6e6e6"/><text x="{}" y="{:.1}" class="tk" text-anchor="end">{}</text>"##,
            W - PAD_R,
            PAD_L - 6.0,
            y + 4.0,
            fmt_tick(t)
        );
    }
    for t in ticks(a.x0, a.x1, 8) {
        let x = a.x(t);
        let _ = write!(
            s,
            r#"<text x="{x:.1}" y="{}" class="tk" text-anchor="middle">{}</text>"#,
            H - PAD_B + 16.0,
            fmt_tick(t)
        );
    }
    let _ = write!(
        s,
        r##"<line x1="{PAD_L}" x2="{}" y1="{}" y2="{}" stroke="#999"/><text x="{}" y="{}" class="lb" text-anchor="middle">{}</text><text x="12" y="{}" class="lb" text-anchor="middle" transform="rotate(-90 12 {})">{}</text>"##,
        W - PAD_R,
        H - PAD_B,
        H - PAD_B,
        (PAD_L + W - PAD_R) / 2.0,
        H - 4.0,
        esc(xlabel),
        H / 2.0,
        H / 2.0,
        esc(ylabel)
    );
    s
}

fn polyline(a: &Axes, pts: &[(f64, f64)], color: &str, width: f64) -> String {
    let d: Vec<String> = pts.iter().map(|(x, y)| format!("{:.1},{:.1}", a.x(*x), a.y(*y))).collect();
    format!(r#"<polyline fill="none" stroke="{color}" stroke-width="{width}" points="{}"/>"#, d.join(" "))
}

fn legend(names: &[(&str, &str)]) -> String {
    let mut s = String::from(r#"<div class="lg">"#);
    for (n, c) in names {
        let _ = write!(s, r#"<span><i style="background:{c}"></i>{}</span>"#, esc(n));
    }
    s.push_str("</div>");
    s
}

/// (legend name, colour, points)
type Series<'a> = (&'a str, &'a str, Vec<(f64, f64)>);

fn line_chart(series: &[Series], xlabel: &str, ylabel: &str, y_range: Option<(f64, f64)>, bands: &dyn Fn(&Axes) -> String) -> String {
    let all = series.iter().flat_map(|s| s.2.iter());
    let (mut x0, mut x1, mut y1) = (f64::MAX, f64::MIN, f64::MIN);
    for (x, y) in all {
        x0 = x0.min(*x);
        x1 = x1.max(*x);
        y1 = y1.max(*y);
    }
    if x0 > x1 {
        return "<p class=\"muted\">No data.</p>".into();
    }
    let (y0, y1) = y_range.unwrap_or((0.0, if y1 <= 0.0 { 1.0 } else { y1 * 1.05 }));
    let a = Axes { x0, x1: x1.max(x0 + 1.0), y0, y1 };
    let mut s = frame(&a, xlabel, ylabel, &bands(&a));
    for (_, color, pts) in series {
        s.push_str(&polyline(&a, pts, color, 1.8));
    }
    s.push_str("</svg>");
    if series.len() > 1 {
        s.push_str(&legend(&series.iter().map(|(n, c, _)| (*n, *c)).collect::<Vec<_>>()));
    }
    s
}

fn quality_chart(r: &Report) -> String {
    let pts = |f: &dyn Fn(&crate::stats::PosStat) -> Option<f64>| -> Vec<(f64, f64)> {
        r.per_position.iter().filter_map(|p| f(p).map(|v| (p.pos as f64, v))).collect()
    };
    let median = pts(&|p| p.median);
    let mean = pts(&|p| p.mean_q);
    let ymax = r.per_position.iter().filter_map(|p| p.p90).fold(40.0, f64::max) + 2.0;
    let iqr: Vec<(f64, f64, f64)> = r
        .per_position
        .iter()
        .filter_map(|p| Some((p.pos as f64, p.p25?, p.p75?)))
        .collect();
    let decile: Vec<(f64, f64, f64)> = r
        .per_position
        .iter()
        .filter_map(|p| Some((p.pos as f64, p.p10?, p.p90?)))
        .collect();
    let bands = move |a: &Axes| {
        let zone = |lo: f64, hi: f64, c: &str| {
            format!(
                r#"<rect x="{PAD_L}" width="{}" y="{:.1}" height="{:.1}" fill="{c}"/>"#,
                W - PAD_L - PAD_R,
                a.y(hi),
                a.y(lo) - a.y(hi)
            )
        };
        let area = |pts: &[(f64, f64, f64)], fill: &str| {
            if pts.is_empty() {
                return String::new();
            }
            let top: Vec<String> = pts.iter().map(|(x, _, h)| format!("{:.1},{:.1}", a.x(*x), a.y(*h))).collect();
            let bot: Vec<String> = pts.iter().rev().map(|(x, l, _)| format!("{:.1},{:.1}", a.x(*x), a.y(*l))).collect();
            format!(r#"<polygon fill="{fill}" points="{} {}"/>"#, top.join(" "), bot.join(" "))
        };
        zone(0.0, 20.0, "#fbeaea") + &zone(20.0, 28.0, "#fbf3e2") + &zone(28.0, ymax, "#eaf5ee") + &area(&decile, "#2f6fb31f") + &area(&iqr, "#2f6fb347")
    };
    let mut s = line_chart(
        &[("median", COLORS[0], median), ("mean", COLORS[1], mean)],
        "Position in read (bp)",
        "Phred quality",
        Some((0.0, ymax)),
        &bands,
    );
    s.push_str(r#"<p class="muted">Shaded bands: 25–75% (dark) and 10–90% (light) of reads at each position.</p>"#);
    s
}

/// Heatmap of tile × position quality deviation. Only cells below the tile average are drawn
/// (white = at or above average, deepening red = further below), so the SVG stays small.
fn tile_heatmap(tiles: &[u32], positions: &[usize], deviation: &[Vec<f32>]) -> String {
    let npos = deviation.iter().map(Vec::len).max().unwrap_or(0);
    if npos == 0 || tiles.is_empty() {
        return String::new();
    }
    // At most ~150 columns: average neighbouring positions on long reads.
    let step = npos.div_ceil(150);
    let cols = npos.div_ceil(step);
    let (x0, y0) = (PAD_L, PAD_T);
    let pw = W - PAD_L - PAD_R;
    let rows = tiles.len();
    let row_h = (360.0 / rows as f64).clamp(2.0, 14.0);
    let ph = row_h * rows as f64;
    let h = ph + PAD_T + PAD_B;
    let cw = pw / cols as f64;
    let mut s = format!(
        r##"<svg viewBox="0 0 {W} {h:.0}" role="img" aria-label="Per-tile quality"><rect x="{x0}" y="{y0}" width="{pw}" height="{ph:.1}" fill="#fff" stroke="#ddd"/>"##
    );
    for (r, row) in deviation.iter().enumerate() {
        for c in 0..cols {
            let cells = &row[(c * step).min(row.len())..((c + 1) * step).min(row.len())];
            if cells.is_empty() {
                continue;
            }
            let d = cells.iter().map(|&v| v as f64).sum::<f64>() / cells.len() as f64;
            if d.abs() < 1.0 {
                continue;
            }
            // 1 → faint, 10 or more → full colour; red below the average, blue above.
            let a = ((d.abs() - 1.0) / 9.0).clamp(0.08, 1.0);
            let colour = if d < 0.0 { "#c0392b" } else { "#2c6fbb" };
            let _ = write!(
                s,
                r##"<rect x="{:.2}" y="{:.2}" width="{:.2}" height="{:.2}" fill="{colour}" fill-opacity="{a:.2}"/>"##,
                x0 + c as f64 * cw,
                y0 + r as f64 * row_h,
                cw + 0.3,
                row_h + 0.3
            );
        }
    }
    // Columns are position groups; label a few with the position they start at.
    for c in (0..cols).step_by(cols.div_ceil(8).max(1)) {
        let Some(p) = positions.get(c * step) else { continue };
        let _ = write!(s, r#"<text x="{:.1}" y="{:.1}" class="tk" text-anchor="middle">{}</text>"#, x0 + (c as f64 + 0.5) * cw, y0 + ph + 16.0, fmt_tick(*p as f64));
    }
    let label_every = (rows / 12).max(1);
    for (r, t) in tiles.iter().enumerate().step_by(label_every) {
        let _ = write!(s, r#"<text x="{:.1}" y="{:.1}" class="tk" text-anchor="end">{t}</text>"#, x0 - 6.0, y0 + (r as f64 + 0.5) * row_h + 3.5);
    }
    let _ = write!(
        s,
        r#"<text x="{:.1}" y="{:.1}" class="lb" text-anchor="middle">Position in read (bp)</text></svg>"#,
        x0 + pw / 2.0,
        h - 4.0
    );
    s
}

/// Bars over the log-scale length bins, one per bin (`None`: no bar), with the decades labelled.
fn length_bars(bins: &[LenBin], vals: &[Option<f64>], ylabel: &str, color: &str) -> String {
    let ymax = vals.iter().flatten().copied().fold(0.0, f64::max);
    if bins.is_empty() || ymax <= 0.0 {
        return "<p class=\"muted\">No data.</p>".into();
    }
    let a = Axes { x0: 0.0, x1: bins.len() as f64, y0: 0.0, y1: ymax * 1.05 };
    let mut s = format!(r#"<svg viewBox="0 0 {W} {H}" role="img" aria-label="{}">"#, esc(ylabel));
    for t in ticks(a.y0, a.y1, 5) {
        let y = a.y(t);
        let _ = write!(
            s,
            r##"<line x1="{PAD_L}" x2="{}" y1="{y:.1}" y2="{y:.1}" stroke="#e6e6e6"/><text x="{}" y="{:.1}" class="tk" text-anchor="end">{}</text>"##,
            W - PAD_R,
            PAD_L - 6.0,
            y + 4.0,
            fmt_tick(t)
        );
    }
    let bw = a.x(1.0) - a.x(0.0);
    for (i, (b, v)) in bins.iter().zip(vals).enumerate() {
        if let Some(v) = v {
            let _ = write!(
                s,
                r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{color}"><title>{}–{} bp: {}</title></rect>"#,
                a.x(i as f64) + 0.5,
                a.y(*v),
                (bw - 1.0).max(0.5),
                a.y(0.0) - a.y(*v),
                b.min,
                b.max,
                fmt_tick(*v)
            );
        }
        // Decades: a label where a bin starts at a power of ten.
        if b.min == 10usize.pow((b.min as f64).log10().round() as u32) {
            let _ = write!(s, r#"<text x="{:.1}" y="{}" class="tk" text-anchor="middle">{}</text>"#, a.x(i as f64), H - PAD_B + 16.0, fmt_tick(b.min as f64));
        }
    }
    let _ = write!(
        s,
        r##"<line x1="{PAD_L}" x2="{}" y1="{}" y2="{}" stroke="#999"/><text x="{}" y="{}" class="lb" text-anchor="middle">Read length (bp, log scale)</text><text x="12" y="{}" class="lb" text-anchor="middle" transform="rotate(-90 12 {})">{}</text></svg>"##,
        W - PAD_R,
        H - PAD_B,
        H - PAD_B,
        (PAD_L + W - PAD_R) / 2.0,
        H - 4.0,
        H / 2.0,
        H / 2.0,
        esc(ylabel)
    );
    s
}

fn long_section(b: &mut String, l: &LongReads) {
    b.push_str("<h2>Long reads</h2><table>");
    let row = |b: &mut String, k: &str, v: String| {
        let _ = write!(b, r#"<tr><th>{k}</th><td class="n">{v}</td></tr>"#);
    };
    let bp = |v: Option<usize>| v.map_or("n/a".into(), |v| format!("{} bp", thousands(v as u64)));
    row(b, "Total bases", thousands(l.bases));
    row(b, "N50 / N90", format!("{} / {}", bp(l.n50), bp(l.n90)));
    row(b, "Mean / median length", format!("{} / {} bp", thousands(l.mean_length.round() as u64), thousands(l.median_length.round() as u64)));
    row(b, "Longest read", bp(Some(l.longest)));
    if let Some(q) = &l.read_quality {
        row(b, "Read quality, mean / median", format!("Q{:.1} / Q{:.1}", q.mean, q.median));
        row(b, "Reads ≥ Q10", format!("{} ({:.2}%; {:.2}% of bases)", thousands(q.q10_reads), q.q10_percent, q.q10_bases_percent));
        row(b, "Reads ≥ Q20", format!("{} ({:.2}%; {:.2}% of bases)", thousands(q.q20_reads), q.q20_percent, q.q20_bases_percent));
    }
    b.push_str("</table>");
    let why = if l.detected == "auto" { "the median read is longer than 1,000 bp" } else { "--long was given" };
    let _ = write!(b, r#"<p class="muted">Shown because {why}. N50: half of all bases are in reads this long or longer. A read's quality is the Phred score of its mean error probability, as basecallers report it (lower than the mean of its scores).</p>"#);
    b.push_str("<h3>Read length</h3>");
    b.push_str(&length_bars(&l.length_bins, &l.length_bins.iter().map(|x| Some(x.reads as f64)).collect::<Vec<_>>(), "Reads", COLORS[3]));
    b.push_str(&length_bars(&l.length_bins, &l.length_bins.iter().map(|x| Some(x.bases as f64)).collect::<Vec<_>>(), "Bases", COLORS[0]));
    if let Some(q) = &l.read_quality {
        b.push_str("<h3>Read quality</h3>");
        let pts: Vec<_> = q.distribution.iter().map(|(q, c)| (*q as f64, *c as f64)).collect();
        b.push_str(&line_chart(&[("reads", COLORS[2], pts)], "Read quality (Phred of the mean error)", "Reads", None, &|_| String::new()));
        b.push_str("<h3>Read quality by length</h3>");
        b.push_str(&length_bars(&l.length_bins, &l.length_bins.iter().map(|x| x.mean_read_quality).collect::<Vec<_>>(), "Mean read quality", COLORS[1]));
    }
}

fn status_badge(s: Status) -> &'static str {
    match s {
        Status::Pass => r#"<b class="st pass">pass</b>"#,
        Status::Warn => r#"<b class="st warn">warn</b>"#,
        Status::Fail => r#"<b class="st fail">fail</b>"#,
    }
}

fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn html(r: &Report, version: &str) -> String {
    let mut b = String::new();
    let _ = write!(
        b,
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>leanqc: {name}</title><style>
body{{font:15px/1.5 system-ui,sans-serif;margin:0;color:#1d1d1f;background:#fafafa}}
main{{max-width:820px;margin:0 auto;padding:24px 20px 60px}}
h1{{font-size:20px;margin:0 0 2px;word-break:break-all}}h2{{font-size:16px;margin:32px 0 8px}}h3{{font-size:14px;margin:18px 0 6px}}
.muted{{color:#666;font-size:13px}}table{{border-collapse:collapse;width:100%;font-size:14px}}
td,th{{text-align:left;padding:5px 8px;border-bottom:1px solid #e3e3e3}}th{{font-weight:600}}
td.n{{text-align:right;font-variant-numeric:tabular-nums}}
.st{{display:inline-block;min-width:38px;text-align:center;border-radius:4px;padding:0 6px;font-size:12px;font-weight:600}}
.pass{{background:#dcefe2;color:#1d5e36}}.warn{{background:#f7e6c4;color:#7a4b00}}.fail{{background:#f6d4d1;color:#8c1d14}}
svg{{width:100%;height:auto;background:#fff;border:1px solid #e3e3e3;border-radius:6px}}
.tk{{font-size:11px;fill:#666}}.lb{{font-size:12px;fill:#333}}
.lg{{display:flex;gap:14px;flex-wrap:wrap;font-size:13px;margin:6px 0}}.lg i{{display:inline-block;width:12px;height:3px;margin:0 6px 3px 0;vertical-align:middle}}
code{{font-size:12px;word-break:break-all}}
</style></head><body><main>
<h1>{name}</h1><p class="muted">leanqc {version}</p>"#,
        name = esc(&r.file)
    );

    b.push_str("<h2>Summary</h2><table>");
    let row = |b: &mut String, k: &str, v: String| {
        let _ = write!(b, r#"<tr><th>{k}</th><td class="n">{v}</td></tr>"#);
    };
    row(&mut b, "Reads", thousands(r.reads));
    row(&mut b, "Bases", thousands(r.bases));
    row(&mut b, "Length", format!("{}–{} bp (mean {:.1})", r.min_len, r.max_len, r.mean_len));
    row(&mut b, "GC", format!("{:.2}%", r.gc_percent));
    row(&mut b, "N", format!("{:.3}%", r.n_percent));
    if let (Some(q20), Some(q30), Some(off)) = (r.q20_percent, r.q30_percent, r.phred_offset) {
        row(&mut b, "Bases ≥ Q20 / ≥ Q30", format!("{q20:.2}% / {q30:.2}%"));
        row(&mut b, "Quality encoding", format!("Phred+{off}"));
    }
    b.push_str("</table>");
    for w in &r.warnings {
        let _ = write!(b, r#"<p class="muted">Note: {}</p>"#, esc(w));
    }
    if r.per_position.iter().any(|p| p.end > p.pos) {
        b.push_str(r#"<p class="muted">Positions are grouped as FastQC groups them (single bases up to 9, then equal-width groups); each point is plotted at the start of its group.</p>"#);
    }

    b.push_str("<h2>Checks</h2><table>");
    for c in &r.checks {
        let _ = write!(b, "<tr><td>{}</td><td>{}</td><td class=\"muted\">{}</td></tr>", status_badge(c.status), c.name, esc(&c.detail));
    }
    b.push_str(r#"</table><p class="muted">Thresholds follow FastQC's defaults. A warn or fail is a prompt to look, not a verdict: amplicon, RNA-seq and trimmed data trip some checks by design.</p>"#);
    if let Some(l) = &r.long_reads {
        b.push_str(r#"<p class="muted">Long reads: the checks above are FastQC's, made for short reads. Read length warns whenever lengths differ, and per-read quality uses the mean of the scores with FastQC's Q20/Q27 limits, which many nanopore reads fall under. See the long-read views below.</p>"#);
        long_section(&mut b, l);
    }

    if r.phred_offset.is_some() {
        b.push_str("<h2>Quality by position</h2>");
        b.push_str(&quality_chart(r));

        match &r.per_tile {
            PerTile::Measured { tiles, positions, deviation, .. } => {
                b.push_str("<h2>Quality by flowcell tile</h2>");
                b.push_str(&tile_heatmap(tiles, positions, deviation));
                b.push_str(r#"<p class="muted">Rows are tiles, columns position groups. Red marks where a tile's mean quality falls below the average of all tiles, blue where it is above (full colour: 10 or more away). The check, like FastQC's, counts the largest difference either way.</p>"#);
            }
            PerTile::Skipped { reason } => {
                let _ = write!(b, r#"<h2>Quality by flowcell tile</h2><p class="muted">Skipped: {}.</p>"#, esc(reason));
            }
        }

        b.push_str("<h2>Mean quality per read</h2>");
        let pts: Vec<_> = r.read_mean_quality.iter().map(|(q, c)| (*q as f64, *c as f64)).collect();
        b.push_str(&line_chart(&[("reads", COLORS[0], pts)], "Mean Phred quality of the read", "Reads", None, &|_| String::new()));
    }

    b.push_str("<h2>Base content by position</h2>");
    let names = ["A", "C", "G", "T", "N"];
    let series: Vec<_> = (0..5)
        .map(|k| {
            (names[k], COLORS[k], r.per_position.iter().map(|p| (p.pos as f64, p.base_percent[k])).collect::<Vec<_>>())
        })
        .collect();
    b.push_str(&line_chart(&series, "Position in read (bp)", "% of reads", Some((0.0, 100.0)), &|_| String::new()));

    b.push_str("<h2>GC content per read</h2>");
    let pts: Vec<_> = r.gc_distribution.iter().enumerate().map(|(i, c)| (i as f64, *c)).collect();
    b.push_str(&line_chart(&[("reads", COLORS[2], pts)], "GC % of the read", "Reads", None, &|_| String::new()));

    b.push_str("<h2>Read length</h2>");
    if r.length_distribution.len() == 1 {
        let (l, c) = r.length_distribution[0];
        let _ = write!(b, "<p>All {} reads are {l} bp.</p>", thousands(c));
    } else {
        let pts: Vec<_> = bucket_lengths(&r.length_distribution);
        b.push_str(&line_chart(&[("reads", COLORS[3], pts)], "Read length (bp)", "Reads", None, &|_| String::new()));
    }

    b.push_str("<h2>Adapter content</h2>");
    let series: Vec<_> = r
        .adapters
        .iter()
        .enumerate()
        .map(|(i, a)| (a.name, COLORS[i % COLORS.len()], a.positions.iter().zip(&a.percent).map(|(p, v)| (*p as f64, *v)).collect::<Vec<_>>()))
        .collect();
    let amax = r.adapters.iter().map(|a| a.max_percent).fold(0.0, f64::max);
    b.push_str(&line_chart(&series, "Position in read (bp)", "% of reads with adapter by here", Some((0.0, (amax * 1.2).max(10.0))), &|_| String::new()));

    b.push_str("<h2>Overrepresented sequences</h2>");
    if r.overrepresented.is_empty() {
        let _ = write!(b, "<p>None above 0.1% of all {} reads.</p>", thousands(r.duplication.reads));
    } else {
        b.push_str("<table><tr><th>Sequence</th><th>Count</th><th>%</th><th>Possible source</th></tr>");
        for o in &r.overrepresented {
            let _ = write!(
                b,
                r#"<tr><td><code>{}</code></td><td class="n">{}</td><td class="n">{:.2}</td><td>{}</td></tr>"#,
                esc(&o.sequence),
                o.count,
                o.percent,
                o.possible_source.unwrap_or("no hit")
            );
        }
        b.push_str("</table>");
    }
    b.push_str("</main></body></html>\n");
    b
}

/// Long-read length histograms have thousands of distinct values; bin them for plotting.
fn bucket_lengths(dist: &[(usize, u64)]) -> Vec<(f64, f64)> {
    let max = dist.last().map(|d| d.0).unwrap_or(0);
    let width = (max / 200).max(1);
    let mut bins: std::collections::BTreeMap<usize, u64> = Default::default();
    for (l, c) in dist {
        *bins.entry(l / width * width).or_default() += c;
    }
    bins.into_iter().map(|(l, c)| (l as f64, c as f64)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_values() {
        assert_eq!(ticks(0.0, 40.0, 5), vec![0.0, 10.0, 20.0, 30.0, 40.0]);
        assert_eq!(fmt_tick(1500.0), "2k");
        assert_eq!(thousands(1234567), "1,234,567");
    }
}
