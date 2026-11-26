//! A single self-contained HTML file: no CDN, no JavaScript, no network.
//!
//! ProLIF's own diagram pulls vis-network from unpkg at view time. It renders an empty box
//! offline, and a report that goes into a thesis has to still work in five years, so the
//! figure here is plain SVG written into the file.

use crate::stages::{Box3, Contact, Pose};
use std::collections::BTreeMap;
use std::fmt::Write;

/// Interaction classes, strongest first. Rank decides the colour and line weight.
fn rank_of(kind: &str) -> usize {
    match kind {
        "HBDonor" | "HBAcceptor" => 0,
        "PiStacking" | "FaceToFace" | "EdgeToFace" | "CationPi" | "PiCation" => 1,
        "Anionic" | "Cationic" => 1,
        "Hydrophobic" => 2,
        _ => 3,
    }
}

fn style_of(rank: usize) -> (&'static str, &'static str) {
    match rank {
        0 => ("var(--chart-1)", "H-bond"),
        1 => ("var(--chart-4)", "ionic / pi"),
        2 => ("var(--chart-2)", "hydrophobic"),
        _ => ("var(--muted-foreground)", "van der Waals"),
    }
}

pub struct ReportInput<'a> {
    pub title: &'a str,
    pub subtitle: &'a str,
    pub ligand_label: &'a str,
    pub poses: &'a [Pose],
    pub contacts: &'a [Contact],
    pub bx: Box3,
    /// RMSD of each pose to a reference, when one was given.
    pub rmsd: Option<&'a [f64]>,
}

pub fn render(input: &ReportInput<'_>) -> String {
    let mut by_res: BTreeMap<&str, Vec<&Contact>> = BTreeMap::new();
    for c in input.contacts {
        by_res.entry(c.residue.as_str()).or_default().push(c);
    }
    let mut residues: Vec<(&str, usize, Option<f64>, usize)> = by_res
        .iter()
        .map(|(res, cs)| {
            let best = cs.iter().map(|c| rank_of(&c.kind)).min().unwrap_or(3);
            let hb = cs
                .iter()
                .filter(|c| c.kind.starts_with("HB"))
                .map(|c| c.distance)
                .fold(f64::INFINITY, f64::min);
            (*res, best, if hb.is_finite() { Some(hb) } else { None }, cs.len())
        })
        .collect();
    residues.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(b.0)));

    let (w, h) = (728.0_f64, 470.0_f64);
    let (cx, cy, r) = (364.0_f64, 228.0_f64, 168.0_f64);
    let n = residues.len().max(1);
    let mut svg = String::new();

    for (k, (res, best, hb, count)) in residues.iter().enumerate() {
        let a = -std::f64::consts::FRAC_PI_2 + 2.0 * std::f64::consts::PI * (k as f64) / (n as f64);
        let (x, y) = (cx + r * a.cos(), cy + r * a.sin());
        let (col, _) = style_of(*best);
        let dash = if *best == 3 { " stroke-dasharray=\"4 3\"" } else { "" };
        let width = match best {
            0 => 2.2,
            2 => 1.6,
            _ => 1.0,
        };
        let opacity = if *best == 3 { 0.55 } else { 0.9 };
        let _ = write!(
            svg,
            "<line x1=\"{cx}\" y1=\"{cy}\" x2=\"{x:.1}\" y2=\"{y:.1}\" stroke=\"{col}\" \
             stroke-width=\"{width}\"{dash} opacity=\"{opacity}\"/>"
        );
        let (tw, th) = (66.0, 30.0);
        let _ = write!(
            svg,
            "<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{tw}\" height=\"{th}\" rx=\"7\" \
             fill=\"var(--card)\" stroke=\"{col}\" stroke-width=\"1.4\"/>",
            x - tw / 2.0,
            y - th / 2.0
        );
        let (name, num) = split_residue(res);
        let _ = write!(
            svg,
            "<text x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"11\" \
             font-weight=\"600\" fill=\"var(--foreground)\" font-family=\"var(--font-mono)\">\
             {}</text>",
            y - 2.0,
            esc(&format!("{name} {num}"))
        );
        let label = match hb {
            Some(d) => format!("{d:.2} Å"),
            None => format!("{count} contact{}", if *count == 1 { "" } else { "s" }),
        };
        let _ = write!(
            svg,
            "<text x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-size=\"8.5\" \
             fill=\"var(--muted-foreground)\" font-family=\"var(--font-mono)\">{}</text>",
            y + 9.0,
            esc(&label)
        );
    }

    let top = input.poses.first();
    let _ = write!(
        svg,
        "<circle cx=\"{cx}\" cy=\"{cy}\" r=\"54\" fill=\"var(--secondary)\" \
         stroke=\"var(--primary)\" stroke-width=\"2\"/>\
         <text x=\"{cx}\" y=\"{:.0}\" text-anchor=\"middle\" font-size=\"13\" font-weight=\"700\" \
         fill=\"var(--foreground)\">{}</text>\
         <text x=\"{cx}\" y=\"{:.0}\" text-anchor=\"middle\" font-size=\"11\" \
         fill=\"var(--primary)\" font-family=\"var(--font-mono)\">{}</text>\
         <text x=\"{cx}\" y=\"{:.0}\" text-anchor=\"middle\" font-size=\"8\" \
         fill=\"var(--muted-foreground)\">kcal/mol</text>",
        cy - 6.0,
        esc(input.ligand_label),
        cy + 11.0,
        top.map(|p| format!("{:.2}", p.affinity)).unwrap_or_else(|| "—".into()),
        cy + 24.0
    );

    let mut legend = String::new();
    for rank in 0..4 {
        let (c, l) = style_of(rank);
        let _ = write!(
            legend,
            "<span style=\"display:inline-flex;align-items:center;gap:6px;margin-right:16px\">\
             <span style=\"width:14px;height:3px;background:{c};display:inline-block;\
             border-radius:2px\"></span><span style=\"font-size:12px;\
             color:var(--muted-foreground)\">{l}</span></span>"
        );
    }

    let mut rows = String::new();
    for (i, p) in input.poses.iter().enumerate() {
        let rm = input
            .rmsd
            .and_then(|v| v.get(i))
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "—".into());
        let flag = match input.rmsd.and_then(|v| v.get(i)) {
            Some(v) if *v > 5.0 => " style=\"color:var(--muted-foreground)\"",
            _ => "",
        };
        let _ = write!(
            rows,
            "<tr{flag}><td>{}</td><td class=\"n\">{:.2}</td><td class=\"n\">{:.2}</td>\
             <td class=\"n\">{rm}</td></tr>",
            p.rank, p.affinity, p.rmsd_ub
        );
    }

    let badge = match input.rmsd.and_then(|v| v.first()) {
        Some(v) => format!(
            "<span class=\"badge {}\">RMSD {v:.2} Å vs reference</span>",
            if *v <= 2.0 { "ok" } else { "warn" }
        ),
        None => String::new(),
    };

    format!(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>{title}</title><style>
*{{box-sizing:border-box}} body{{margin:0;font-family:var(--font-sans,system-ui);
background:var(--background,#fff);color:var(--foreground,#111)}}
.hd{{display:flex;justify-content:space-between;align-items:baseline;flex-wrap:wrap;gap:8px}}
.t{{font-size:15px;font-weight:650}} .s{{font-size:12px;color:var(--muted-foreground,#666);
font-family:var(--font-mono,ui-monospace)}}
.badge{{display:inline-block;padding:3px 9px;border-radius:var(--radius,6px);font-size:11px;
font-weight:600;font-family:var(--font-mono,ui-monospace);background:var(--secondary,#eee)}}
.badge.ok{{color:var(--success,#15803d)}} .badge.warn{{color:var(--warning,#b45309)}}
table{{border-collapse:collapse;width:100%;margin-top:14px;font-size:12px;
font-family:var(--font-mono,ui-monospace)}}
th,td{{text-align:left;padding:4px 8px;border-bottom:1px solid var(--border,#e5e5e5)}}
th{{color:var(--muted-foreground,#666);font-weight:600}} td.n{{text-align:right}}
.note{{margin-top:14px;font-size:11.5px;color:var(--muted-foreground,#666);line-height:1.5}}
</style></head><body>
<div class="hd"><div><div class="t">{title}</div><div class="s">{subtitle}</div></div>
<div>{badge}</div></div>
<svg width="100%" viewBox="0 0 {w} {h}" style="display:block">{svg}</svg>
<div>{legend}</div>
<table><thead><tr><th>pose</th><th class="n">affinity</th><th class="n">RMSD to pose 1</th>
<th class="n">RMSD to ref</th></tr></thead><tbody>{rows}</tbody></table>
<div class="note">Box centre {bcx:.2}, {bcy:.2}, {bcz:.2} Å; size {bsx:.0} × {bsy:.0} × {bsz:.0} Å.
Affinity is the AutoDock Vina score, not a measured binding free energy, and it ranks poses
far better than it ranks different molecules. Poses with a large RMSD to pose 1 generally sit
in a different pocket rather than being a variation on the same binding mode.</div>
</body></html>"#,
        title = esc(input.title),
        subtitle = esc(input.subtitle),
        bcx = input.bx.center[0], bcy = input.bx.center[1], bcz = input.bx.center[2],
        bsx = input.bx.size[0], bsy = input.bx.size[1], bsz = input.bx.size[2],
    )
}

fn split_residue(res: &str) -> (String, String) {
    let head = res.split('.').next().unwrap_or(res);
    let name: String = head.chars().take_while(|c| c.is_alphabetic()).collect();
    let num: String = head.chars().skip_while(|c| c.is_alphabetic()).collect();
    (name, num)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contact(residue: &str, kind: &str, distance: f64) -> Contact {
        Contact { residue: residue.into(), kind: kind.into(), distance }
    }

    #[test]
    fn splits_residue_labels() {
        assert_eq!(split_residue("CYS919.A"), ("CYS".to_string(), "919".to_string()));
        assert_eq!(split_residue("GLY22"), ("GLY".to_string(), "22".to_string()));
    }

    #[test]
    fn hydrogen_bonds_outrank_van_der_waals() {
        assert!(rank_of("HBAcceptor") < rank_of("Hydrophobic"));
        assert!(rank_of("Hydrophobic") < rank_of("VdWContact"));
    }

    #[test]
    fn report_is_self_contained() {
        let poses = vec![Pose { rank: 1, affinity: -11.09, rmsd_lb: 0.0, rmsd_ub: 0.0 }];
        let contacts = vec![
            contact("CYS919.A", "HBAcceptor", 3.38),
            contact("VAL848.A", "Hydrophobic", 4.38),
        ];
        let html = render(&ReportInput {
            title: "t",
            subtitle: "s",
            ligand_label: "sorafenib",
            poses: &poses,
            contacts: &contacts,
            bx: Box3 { center: [0.0; 3], size: [24.0; 3] },
            rmsd: Some(&[0.75]),
        });
        assert!(!html.contains("http://"), "report must not fetch anything");
        assert!(!html.contains("https://"), "report must not fetch anything");
        assert!(!html.contains("<script"), "report must not need JavaScript");
        assert!(html.contains("CYS 919"));
        assert!(html.contains("-11.09"));
    }

    #[test]
    fn escapes_user_supplied_text() {
        let html = render(&ReportInput {
            title: "<img src=x onerror=alert(1)>",
            subtitle: "s",
            ligand_label: "l",
            poses: &[],
            contacts: &[],
            bx: Box3 { center: [0.0; 3], size: [24.0; 3] },
            rmsd: None,
        });
        assert!(!html.contains("<img src=x"));
        assert!(html.contains("&lt;img"));
    }
}
