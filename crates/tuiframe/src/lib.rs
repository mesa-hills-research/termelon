//! A terminal UI framework in pure Rust. It owns the whole path from the terminal's modes to the
//! bytes of each frame, which means raw mode, input decoding, a cell buffer, text and widgets, diffed
//! output in one synchronized write per frame, and capability detection with fallbacks.
//!
//! | Module | Holds |
//! |---|---|
//! | [`style`] | [`Color`], [`Modifier`], [`UnderlineStyle`] and [`Style`] |
//! | [`layout`] | [`Position`], [`Size`] and [`Rect`] |
//!
//! The types used most are also at the crate's root.

mod flags;
pub mod layout;
pub mod style;

pub use layout::{Position, Rect, Size};
pub use style::{Color, Modifier, Style, UnderlineStyle};
