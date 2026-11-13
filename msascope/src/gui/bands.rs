//! The bands of the track header, top to bottom: the ruler, consensus, identity and gaps, the
//! file's `#=GC` tracks, the reference row's `#=GR` tracks, and the pinned reference row. The
//! header's height follows from which are shown.

use gpui_kit::SharedString;
use msascope::annot::TrackKind;

use super::app::MsaScope;
use super::metrics::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderRow {
    Ruler,
    Consensus,
    Identity,
    Gaps,
    /// A `#=GC` track, by index into `Alignment::gc`.
    Gc(usize),
    /// A `#=GR` track of the reference row, by index into `Alignment::gr`.
    Gr(usize),
    /// The reference row, pinned under the tracks.
    Pinned,
}

impl HeaderRow {
    /// A number for the tile key.
    pub fn code(self) -> u64 {
        match self {
            HeaderRow::Ruler => 0,
            HeaderRow::Consensus => 1,
            HeaderRow::Identity => 2,
            HeaderRow::Gaps => 3,
            HeaderRow::Pinned => 4,
            HeaderRow::Gc(i) => 1_000 + i as u64,
            HeaderRow::Gr(i) => 1 << 32 | i as u64,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HeaderBand {
    pub row: HeaderRow,
    /// Logical px from the top of the header.
    pub y: f32,
    pub h: f32,
}

/// Name of a `#=GR` track in the hidden-tracks list and the menu.
pub fn gr_key(tag: &str) -> String {
    format!("GR {tag}")
}

impl MsaScope {
    /// The header's bands as shown now.
    pub(super) fn header_bands(&self) -> Vec<HeaderBand> {
        let mut out = Vec::with_capacity(8);
        let mut y = 0.0;
        let mut push = |row: HeaderRow, h: f32| {
            out.push(HeaderBand { row, y, h });
            y += h;
        };
        push(HeaderRow::Ruler, RULER_H);
        push(HeaderRow::Consensus, CONS_H);
        push(HeaderRow::Identity, IDENT_H);
        push(HeaderRow::Gaps, GAPS_H);
        if let Some(d) = &self.data {
            for (i, t) in d.aln.gc.iter().enumerate() {
                if !self.hidden_tracks.iter().any(|h| **h == *t.tag) {
                    push(HeaderRow::Gc(i), TRACK_H);
                }
            }
            if let Some(r) = &self.ref_row {
                let all = &d.aln.gr;
                let first = all.partition_point(|g| (g.row as usize) < r.row);
                for (i, g) in msascope::annot::row_annots(all, r.row).iter().enumerate() {
                    if !self.hidden_tracks.iter().any(|h| **h == gr_key(&g.tag)) {
                        push(HeaderRow::Gr(first + i), TRACK_H);
                    }
                }
            }
            if self.ref_row.is_some() && self.ref_opts.pin {
                push(HeaderRow::Pinned, PIN_H);
            }
        }
        out
    }

    pub(super) fn header_h(&self) -> f32 {
        self.header_bands().last().map_or(HEADER_H, |b| b.y + b.h)
    }

    /// The band at `y` logical px below the header's top.
    pub(super) fn header_row_at(&self, y: f32) -> Option<HeaderRow> {
        self.header_bands()
            .into_iter()
            .find(|b| y >= b.y && y < b.y + b.h)
            .map(|b| b.row)
    }

    /// The label at the left of a band.
    pub(super) fn band_label(&self, row: HeaderRow) -> SharedString {
        let d = self.data.as_ref();
        let compare = self.compare_base().is_some();
        match row {
            HeaderRow::Ruler if self.ref_opts.ruler && self.ref_row.is_some() => {
                "Reference residue".into()
            }
            HeaderRow::Ruler => "Column".into(),
            HeaderRow::Consensus => "Consensus".into(),
            HeaderRow::Identity if compare => "Identity to ref. %".into(),
            HeaderRow::Identity => "Identity %".into(),
            HeaderRow::Gaps => "Gaps".into(),
            HeaderRow::Gc(i) => d.map_or(SharedString::default(), |d| {
                d.aln.gc[i].tag.to_string().into()
            }),
            HeaderRow::Gr(i) => d.map_or(SharedString::default(), |d| {
                format!("{} (ref.)", d.aln.gr[i].tag).into()
            }),
            HeaderRow::Pinned => match (d, &self.ref_row) {
                (Some(d), Some(r)) => d.aln.names[r.row].clone().into(),
                _ => SharedString::default(),
            },
        }
    }

    /// How a track band is drawn.
    pub(super) fn band_kind(&self, row: HeaderRow) -> Option<TrackKind> {
        let d = self.data.as_ref()?;
        match row {
            HeaderRow::Gc(i) => Some(d.aln.gc[i].kind()),
            HeaderRow::Gr(i) => Some(TrackKind::of(&d.aln.gr[i].tag)),
            _ => None,
        }
    }

    /// The annotation line of a track band.
    pub(super) fn band_data(&self, row: HeaderRow) -> Option<&[u8]> {
        let d = self.data.as_ref()?;
        match row {
            HeaderRow::Gc(i) => Some(&d.aln.gc[i].data),
            HeaderRow::Gr(i) => Some(&d.aln.gr[i].data),
            _ => None,
        }
    }
}
