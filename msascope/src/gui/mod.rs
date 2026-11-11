//! The GPUI side of msascope: design system (palette, tokens, metrics, fonts, keys, components)
//! and the views built from it. The rule: views never use raw colours or sizes. Sizes come from
//! [`metrics`], colours from [`tokens`].

pub mod actions;
pub mod app;
pub mod cached;
pub mod canvas;
pub mod chrome;
pub mod components;
pub mod figure;
pub mod fonts;
pub mod hover;
pub mod input;
pub mod keys;
pub mod metrics;
pub mod overview;
pub mod palette;
pub mod theme;
pub mod tokens;
pub mod tree_menu;
pub mod tree_panel;
