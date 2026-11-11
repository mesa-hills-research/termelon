//! A terminal UI framework in pure Rust. It owns the whole path from the terminal's modes to the
//! bytes of each frame, which means raw mode, input decoding, a cell buffer, text and widgets, diffed
//! output in one synchronized write per frame, and capability detection with fallbacks.
//!
//! | Module | Holds |
//! |---|---|
//! | [`style`] | [`Color`], [`Modifier`], [`UnderlineStyle`] and [`Style`] |
//! | [`layout`] | [`Position`], [`Size`] and [`Rect`] |
//! | [`event`] | [`Event`] and the key and mouse types in it |
//! | [`caps`] | [`Capabilities`], what the terminal supports |
//!
//! The types used most are also at the crate's root.

pub mod caps;
pub mod event;
mod flags;
pub mod layout;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "used by `Terminal`, which comes with the terminal API"
    )
)]
mod output;
pub mod style;

pub use caps::{Capabilities, WidthProfile};
pub use event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};
pub use layout::{Position, Rect, Size};
pub use style::{Color, Modifier, Style, UnderlineStyle};
