//! A terminal UI framework in pure Rust. It owns the whole path from the terminal's modes to the
//! bytes of each frame, which means raw mode, input decoding, a cell buffer, text and widgets, diffed
//! output in one synchronized write per frame, and capability detection with fallbacks.
//!
//! | Module | Holds |
//! |---|---|
//! | [`buffer`] | [`Buffer`], the cells a frame draws into |
//! | [`text`] | [`Span`], [`Line`] and [`Text`], and the measure of text in cells |
//! | [`style`] | [`Color`], [`Modifier`], [`UnderlineStyle`] and [`Style`] |
//! | [`layout`] | [`Position`], [`Size`], [`Rect`], and [`Layout`] by [`Constraint`]s |
//! | [`widgets`] | [`Widget`], and the [`Clear`], [`Block`] and [`Paragraph`] widgets |
//! | [`event`] | [`Event`] and the key and mouse types in it |
//! | [`caps`] | [`Capabilities`], what the terminal supports |
//! | [`terminal`] | `Terminal`, opened with [`Options`], and its [`Events`] |
//! | `testing` | `TestTerminal`, a terminal in memory for tests, and golden files, with the `testing` feature |
//!
//! A frame's cursor is a [`Cursor`], and a frame the terminal took is a [`CompletedFrame`] with
//! its [`FrameStats`].
//!
//! The types used most are also at the crate's root.

pub mod buffer;
pub mod caps;
pub mod event;
mod flags;
pub mod frame;
pub mod layout;
mod output;
mod restore;
pub mod style;
mod sys;
pub mod terminal;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
pub mod text;
#[cfg(feature = "unstable-encoder")]
#[doc(hidden)]
pub mod unstable;
pub mod widgets;

pub use buffer::Buffer;
pub use caps::{Capabilities, WidthProfile};
pub use event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};
pub use frame::Frame;
pub use layout::{Alignment, Constraint, Layout, Position, Rect, Size};
pub use output::{CompletedFrame, Cursor, CursorShape, FrameStats};
pub use restore::{RestoreHandle, install_panic_hook};
pub use style::{Color, Modifier, Style, UnderlineStyle};
pub use terminal::{Drawn, Error, Events, KeyboardRequest, MouseMode, Options, Result};
#[cfg(any(unix, windows))]
pub use terminal::{Terminal, terminal_size};
pub use text::{Line, Span, Text};
pub use widgets::{Block, BorderSet, Borders, Clear, Padding, Paragraph, Widget, Wrap};
