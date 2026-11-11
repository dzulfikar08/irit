// Adapted from Kantor (github.com/dzulfikar08/kantor) by the same author, relicensed
// GPL-3.0-or-later for irit.
//! Sizes. Every size a view uses comes from here.
#![allow(dead_code)]

use gpui_kit::{Pixels, px};

// Type scale
pub const FONT_CAPTION: Pixels = px(10.5);
pub const FONT_SMALL: Pixels = px(11.5);
pub const FONT_BODY: Pixels = px(12.5);
pub const FONT_EMPHASIS: Pixels = px(13.);
pub const FONT_TITLE: Pixels = px(15.);
pub const FONT_DISPLAY: Pixels = px(22.);

/// Root rem size; GPUI Kit components scale from it (rem 14 = compact, desktop-like).
pub const REM: Pixels = px(14.);

// Spacing scale
pub const SPACE_1: Pixels = px(2.);
pub const SPACE_2: Pixels = px(4.);
pub const SPACE_3: Pixels = px(6.);
pub const SPACE_4: Pixels = px(8.);
pub const SPACE_5: Pixels = px(10.);
pub const SPACE_6: Pixels = px(12.);
pub const SPACE_7: Pixels = px(16.);
pub const SPACE_8: Pixels = px(20.);
pub const SPACE_9: Pixels = px(24.);
pub const SPACE_10: Pixels = px(32.);

// Radii
pub const RADIUS_SM: Pixels = px(4.);
pub const RADIUS_MD: Pixels = px(6.);
pub const RADIUS_LG: Pixels = px(8.);
pub const RADIUS_XL: Pixels = px(12.);

// Controls and icons
pub const CONTROL_SM: Pixels = px(22.);
pub const CONTROL_MD: Pixels = px(26.);
pub const CONTROL_LG: Pixels = px(32.);
pub const ICON_SM: Pixels = px(12.);
pub const ICON_MD: Pixels = px(14.);
pub const ICON_LG: Pixels = px(16.);

// Shell
pub const TITLE_BAR_HEIGHT: Pixels = px(38.);
pub const STATUS_BAR_HEIGHT: Pixels = px(26.);
pub const TAB_HEIGHT: Pixels = px(28.);
/// Text only truncates inside a known width, so tab titles get a fixed one.
pub const TAB_TITLE_WIDTH: Pixels = px(150.);
pub const ROW_HEIGHT: Pixels = px(28.);
/// Below this window width the toolbar folds its less used controls into a menu.
pub const COMPACT_WINDOW: Pixels = px(900.);
pub const INPUT_WIDTH: Pixels = px(220.);
pub const INPUT_WIDTH_COMPACT: Pixels = px(150.);

// Alignment view (logical px). The track heights add up to the header height.
pub const NAME_W_DEFAULT: f32 = 196.0;
pub const NAME_W_MIN: f32 = 120.0;
pub const NAME_W_MAX: f32 = 480.0;
pub const TREE_W_DEFAULT: f32 = 118.0;
pub const TREE_W_MIN: f32 = 60.0;
pub const TREE_W_MAX: f32 = 480.0;
/// Space left of the root and right of the deepest leaf in the tree panel.
pub const TREE_PAD: f32 = 8.0;
/// How far (px) from a tree node's vertical line a click still hits it.
pub const TREE_HIT: f32 = 5.0;
/// Hidden-column marks on the ruler closer than this (px) are merged into one.
pub const HIDDEN_MARK_MERGE: f32 = 6.0;
pub const OVERVIEW_W: f32 = 132.0;
/// Width of the grab area of the name panel's splitter.
pub const SPLITTER_W: f32 = 6.0;
pub const RULER_H: f32 = 22.0;
pub const CONS_H: f32 = 17.0;
pub const IDENT_H: f32 = 28.0;
pub const GAPS_H: f32 = 10.0;
pub const HEADER_H: f32 = RULER_H + CONS_H + IDENT_H + GAPS_H;
/// Names are drawn once rows are at least this tall.
pub const NAME_ROW_MIN: f64 = 8.0;
/// Letters are drawn once cells are at least this wide and tall.
pub const LETTER_W: f64 = 7.0;
pub const LETTER_H: f64 = 10.0;
/// Hover outlines are drawn once cells are at least this big.
pub const HOVER_MIN: f64 = 3.0;
/// Ruler labels are at least this far apart.
pub const TICK_LABEL_GAP: f64 = 70.0;
/// At most this many motif matches are drawn per frame (zoomed far out on a file with many).
pub const MOTIF_RECTS_MAX: usize = 20_000;
