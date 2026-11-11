//! The hover card: what is under the pointer, next to it. Over a cell it shows the sequence (name,
//! accession, range), the residue with its position in the ungapped sequence, the column in the
//! alignment's own numbering (and among the shown columns when some are hidden), and the
//! column's consensus, identity and gap share. Over the track header it shows the column only.

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use msascope::alphabet::{GAP, Kind};
use msascope::annot::{TrackKind, pp_words, ss_words};
use msascope::scheme::Colors;
use msascope::view::{self, aln_col};

use super::app::MsaScope;
use super::fonts::tabular;
use super::metrics::*;
use super::tokens::{Elevation, tokens};

/// Three-letter amino acid names.
fn aa3(sym: u8) -> Option<&'static str> {
    Some(match sym {
        b'A' => "Ala",
        b'R' => "Arg",
        b'N' => "Asn",
        b'D' => "Asp",
        b'C' => "Cys",
        b'E' => "Glu",
        b'Q' => "Gln",
        b'G' => "Gly",
        b'H' => "His",
        b'I' => "Ile",
        b'L' => "Leu",
        b'K' => "Lys",
        b'M' => "Met",
        b'F' => "Phe",
        b'P' => "Pro",
        b'S' => "Ser",
        b'T' => "Thr",
        b'W' => "Trp",
        b'Y' => "Tyr",
        b'V' => "Val",
        b'B' => "Asx",
        b'Z' => "Glx",
        b'X' => "Xaa",
        b'U' => "Sec",
        b'O' => "Pyl",
        _ => return None,
    })
}

/// An annotation character and what it means: "H · α-helix", "8 · 75–85%", or the character.
pub fn track_value(kind: TrackKind, ch: u8) -> String {
    let c = ch as char;
    match kind {
        TrackKind::Structure => format!("{c} · {}", ss_words(ch)),
        TrackKind::Probability => format!("{c} · {}", pp_words(ch)),
        TrackKind::Text if msascope::header::is_blank(ch) => format!("{c} · none"),
        TrackKind::Text => c.to_string(),
    }
}

/// "72 of 698", with "· 60 of 115 shown" when columns are hidden, and "· insert" for an insert
/// column. `v` is the view column.
pub fn column_text(v: usize, c: usize, total: usize, shown: Option<usize>, insert: bool) -> String {
    let mut s = format!("{} of {}", view::group_digits(c + 1), view::group_digits(total));
    if let Some(n) = shown {
        s += &format!(" · {} of {} shown", view::group_digits(v + 1), view::group_digits(n));
    }
    if insert {
        s += " · insert";
    }
    s
}

/// The residue's position: "91" and "of 32–143" (the name's range), or "of 112" (the ungapped
/// length) when the name has none. For a gap, where it falls between residues.
pub fn position_text(aln: &msascope::Alignment, r: usize, c: usize) -> (String, String) {
    match aln.seq_position(r, c) {
        Some((pos, start, end)) => {
            let of = if msascope::name_range(&aln.names[r]).is_some() {
                format!("of {}–{}", view::group_digits(start), view::group_digits(end))
            } else {
                format!("of {}", view::group_digits(end))
            };
            (view::group_digits(pos), of)
        }
        None => {
            let before = aln.matrix.residues_before(r, c);
            let len = aln.matrix.residues_before(r, aln.cols());
            let start = msascope::name_range(&aln.names[r]).map_or(1, |(a, _)| a);
            let text = if before == 0 {
                format!("before {}", view::group_digits(start))
            } else if before == len {
                format!("after {}", view::group_digits(start + len - 1))
            } else {
                format!("between {} and {}", view::group_digits(start + before - 1), view::group_digits(start + before))
            };
            ("Gap".into(), text)
        }
    }
}

impl MsaScope {
    pub(super) fn render_hover_card(&self, _: &mut Window, cx: &App) -> Option<AnyElement> {
        if !self.card_visible() {
            return None;
        }
        let d = self.data.as_ref()?;
        let t = tokens(cx);
        let map = self.col_map();
        let (total, shown) = (d.aln.cols(), self.dims().1);
        // At the cursor after a key moved it (next to its cell), else under the pointer.
        let at_cursor = self.card_cursor.then_some(self.cursor).flatten();
        let (r, v) = match (at_cursor, self.hover, self.hover_track) {
            (Some((r, v)), _, _) => (Some(r), v),
            (None, Some((r, v)), _) => (Some(r), v),
            (None, None, Some(v)) => (None, v),
            _ => return None,
        };
        let pos = match at_cursor {
            Some((cr, cv)) => {
                let area = self.area.get();
                let x = area.origin.x + px(((cv as f64 + 1.0 - self.vp.col0) * self.vp.cw) as f32);
                let y = area.origin.y + px(((cr as f64 + 1.0 - self.vp.row0) * self.vp.ch) as f32);
                point(x.min(area.right()), y.min(area.bottom()))
            }
            None => self.pointer?,
        };
        if v >= shown || r.is_some_and(|r| r >= self.dims().0) {
            return None;
        }
        let c = aln_col(map, v);
        let colors = Colors::new(d.aln.alphabet(), self.mode, t.tile_theme(), true);
        let base = self.layout.as_deref().and_then(|l| l.base.as_deref());
        let column = msascope::scheme::Column { consensus: base.map_or(d.stats.consensus[c], |b| b.codes[c]), identity: d.stats.identity[c], shares: &d.stats.shares[c], clustal: d.stats.clustal[c] };
        let mono = cx.theme().mono_font_family.clone();
        // A residue letter on its fill, as in the alignment.
        let chip = |code: u8| {
            let fill = colors.fill(code, &column);
            div()
                .min_w(px(16.))
                .px(SPACE_1)
                .rounded(px(3.))
                .text_center()
                .font_family(mono.clone())
                .when_some(fill, |d, f| d.bg(rgb(f)))
                .when(fill.is_none(), |d| d.border_1().border_color(t.border))
                .text_color(if fill.is_some() { t.res_ink } else { t.fg_muted })
                .child(if code == GAP { "·".to_string() } else { (d.aln.symbol(code) as char).to_string() })
        };
        let row = |label: SharedString, value: AnyElement| h_flex().gap(SPACE_7).justify_between().child(div().text_color(t.fg_muted).child(label)).child(value);
        let value = || h_flex().gap(SPACE_2).text_color(t.fg).font_weight(FontWeight::MEDIUM);
        let dim = |s: String| div().text_color(t.fg_dim).font_weight(FontWeight::NORMAL).child(s);

        let mut card = v_flex()
            .gap(SPACE_2)
            .px(SPACE_5)
            .py(SPACE_4)
            .min_w(px(250.))
            .rounded(RADIUS_LG)
            .border_1()
            .border_color(t.border)
            .bg(t.elevated)
            .shadow(t.shadow(Elevation::Popover))
            .text_size(FONT_SMALL)
            .font_features(tabular())
            .text_color(t.fg);
        let clade = r.and_then(|r| self.row_view().and_then(|m| m.clade_of(r)));
        match r {
            Some(vr) if clade.is_some() => {
                let m = self.row_view().unwrap();
                let k = clade.unwrap();
                let members = m.members(k);
                let label = self
                    .layout
                    .as_deref()
                    .and_then(|l| l.tree.as_ref())
                    .zip(self.tree.as_deref())
                    .map(|(tr, ix)| ix.tree.label(tr.collapsed[k]).to_string())
                    .filter(|l| !l.is_empty() && l.parse::<f64>().is_err())
                    .unwrap_or_else(|| "Collapsed clade".into());
                card = card.child(
                    h_flex()
                        .gap(SPACE_6)
                        .justify_between()
                        .child(div().font_weight(FontWeight::SEMIBOLD).max_w(px(260.)).truncate().child(label))
                        .child(div().text_size(FONT_CAPTION).text_color(t.fg_dim).child(format!("{} sequences", view::group_digits(members.len())))),
                );
                let code = msascope::view::row_src(Some(m), vr).get(&d.aln, c);
                let name3 = (d.aln.kind() == Kind::Protein).then(|| aa3(d.aln.symbol(code))).flatten();
                let res = value().child(chip(code)).when_some(name3.filter(|_| code != GAP), |h, n| h.child(n)).child(dim("clade consensus".into()));
                card = card.child(row("Residue".into(), res.into_any_element()));
            }
            Some(vr) => {
                let r = self.aln_row(vr);
                let name = &d.aln.names[r];
                let (id, range) = match name.rsplit_once('/') {
                    Some((id, rg)) if msascope::name_range(name).is_some() => (id.to_string(), Some(format!("/{rg}"))),
                    _ => (name.clone(), None),
                };
                let meta: Vec<String> = d.aln.accession(r).map(str::to_owned).into_iter().chain(range).collect();
                card = card.child(
                    h_flex()
                        .gap(SPACE_6)
                        .justify_between()
                        .child(div().font_weight(FontWeight::SEMIBOLD).max_w(px(260.)).truncate().child(id))
                        .when(!meta.is_empty(), |h| h.child(div().text_size(FONT_CAPTION).text_color(t.fg_dim).child(meta.join(" · ")))),
                );
                let code = d.aln.matrix.get(r, c);
                let (p, of) = position_text(&d.aln, r, c);
                let name3 = (d.aln.kind() == Kind::Protein).then(|| aa3(d.aln.symbol(code))).flatten();
                let mut res = value().child(chip(code));
                if code != GAP {
                    res = res.when_some(name3, |h, n| h.child(n)).child(p);
                } else {
                    res = res.child(p);
                }
                card = card.child(row("Residue".into(), res.child(dim(of)).into_any_element()));
                // The row's own annotation (#=GR) at this column.
                for g in msascope::annot::row_annots(&d.aln.gr, r) {
                    card = card.child(row(g.tag.to_string().into(), value().child(track_value(TrackKind::of(&g.tag), g.data[c])).into_any_element()));
                }
            }
            None => {
                card = card.child(div().font_weight(FontWeight::SEMIBOLD).child(format!("Column {}", view::group_digits(c + 1))));
                // An annotation track: its character here, in words.
                if let Some(band) = self.hover_band
                    && let (Some(kind), Some(data)) = (self.band_kind(band), self.band_data(band))
                {
                    card = card.child(row(self.band_label(band), value().child(track_value(kind, data[c])).into_any_element()));
                }
            }
        }
        // The reference's residue in this column, and its number.
        if let Some(rf) = self.ref_row.as_deref()
            && r.is_none_or(|vr| self.aln_row(vr) != rf.row)
        {
            let code = rf.codes[c];
            let pos = rf.position(c).map_or_else(|| "gap".to_string(), |p| format!("residue {}", view::group_digits(p)));
            let name = d.aln.names[rf.row].split('/').next().unwrap_or("").to_string();
            card = card.child(row("Reference".into(), value().child(chip(code)).child(pos).child(dim(name)).into_any_element()));
        }
        let col_text = column_text(v, c, total, map.map(|_| shown), d.aln.is_insert(c));
        card = card.child(row("Column".into(), value().child(col_text).into_any_element()));
        // Over the ruler: the hidden columns on either side of this one.
        if r.is_none()
            && let Some(m) = map
        {
            for (at, n) in m.hidden_runs(total, v..v + 1) {
                let (a, label) = if at == v { (if v == 0 { 0 } else { aln_col(map, v - 1) + 1 }, "Hidden before") } else { (c + 1, "Hidden after") };
                let text = if n == 1 { format!("column {}", view::group_digits(a + 1)) } else { format!("{} columns, {}–{}", view::group_digits(n), view::group_digits(a + 1), view::group_digits(a + n)) };
                card = card.child(row(label.into(), value().child(text).into_any_element()));
            }
        }
        let cons = d.stats.consensus[c];
        card = card.child(row(
            "Consensus".into(),
            value()
                .child(chip(cons))
                .child(format!("identity {:.0}% · gaps {:.0}%", d.stats.identity[c] * 100.0, d.stats.gaps[c] * 100.0))
                .into_any_element(),
        ));

        // Next to the pointer, below and to the right; on the other side near the window's edges.
        let target = Bounds::new(pos - point(px(16.), px(16.)), size(px(32.), px(32.)));
        Some(deferred(anchored().attach(target, Anchor::BottomRight).anchor(Anchor::TopLeft).child(card)).with_priority(2).into_any_element())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Hover coordinates: the residue's ungapped position (from the name's range when it has
    /// one), gaps between residues, and the column in alignment and shown numbering.
    #[std::prelude::v1::test]
    fn hover_coordinates() {
        let a = msascope::parse::parse_bytes(b"# STOCKHOLM 1.0\nG/32-36 -AC.gTE-\nplain   MA-..K--\n//\n").unwrap();
        // row 0: residues at columns 1, 2, 4, 5, 6 are 32..=36
        assert_eq!(position_text(&a, 0, 1), ("32".into(), "of 32–36".into()));
        assert_eq!(position_text(&a, 0, 6), ("36".into(), "of 32–36".into()));
        assert_eq!(position_text(&a, 0, 3).1, "between 33 and 34");
        assert_eq!(position_text(&a, 0, 0).1, "before 32");
        assert_eq!(position_text(&a, 0, 7).1, "after 36");
        // no range: counted from 1, "of" the ungapped length
        assert_eq!(position_text(&a, 1, 5), ("3".into(), "of 3".into()));
        // columns 3 and 4 are inserts ('.' / lower case in both rows): hidden, column 6 is the
        // 5th of 6 shown
        let s = msascope::stats::ColumnStats::compute(&a);
        let m = msascope::view::ColumnMap::hiding(&a, &s, true, None).unwrap();
        assert_eq!(m.visible, [0, 1, 2, 5, 6, 7]);
        let v = m.view_of(6).unwrap();
        assert_eq!(column_text(v, aln_col(Some(&m), v), a.cols(), Some(m.len()), false), "7 of 8 · 5 of 6 shown");
        assert_eq!(column_text(3, 3, 8, None, true), "4 of 8 · insert");
        assert_eq!(aa3(b'E'), Some("Glu"));
    }
}
