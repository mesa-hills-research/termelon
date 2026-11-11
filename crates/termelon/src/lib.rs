//! A terminal user interface library for Rust. It owns the whole path from the terminal's modes to
//! the bytes of each frame, which means raw mode, input decoding, a cell buffer, text and widgets,
//! diffed output in one synchronized write per frame, and capability detection with fallbacks.
//!
//! This program fills the screen with a box, counts presses of Space, and quits on `q`.
//!
//! ```no_run
//! use std::time::Duration;
//!
//! use termelon::widgets::{Block, Paragraph};
//! use termelon::{Drawn, Event, KeyCode, KeyEventKind, Options, Terminal};
//!
//! fn main() -> Result<(), termelon::Error> {
//!     let mut terminal = Terminal::open(Options::default())?;
//!     let mut events = terminal.events();
//!     let mut presses = 0;
//!     loop {
//!         let drawn = terminal.draw(|frame| {
//!             let text = format!("Space pressed {presses} times. Press q to quit.");
//!             let block = Block::bordered().title(" Termelon ");
//!             frame.render_widget(Paragraph::new(text).block(block), frame.area());
//!         })?;
//!         // A frame isn't drawn while the terminal is still taking the last one, so look again soon.
//!         let wait = if matches!(drawn, Drawn::Held) {
//!             Duration::from_millis(10)
//!         } else {
//!             Duration::from_secs(60)
//!         };
//!         match events.next_timeout(wait)? {
//!             Some(Event::Key(key)) if key.kind != KeyEventKind::Release => match key.code {
//!                 KeyCode::Char('q') => break,
//!                 KeyCode::Char(' ') => presses += 1,
//!                 _ => {}
//!             },
//!             _ => {}
//!         }
//!     }
//!     terminal.close()
//! }
//! ```
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
//! | [`frame`] | [`Frame`], what the closure of a draw gets to render into |
//! | [`terminal`] | `Terminal` (on Unix and Windows), opened with [`Options`], and its [`Events`] |
//! | `testing` | `TestTerminal`, a terminal in memory for tests, and golden files, with the `testing` feature |
//!
//! A frame's cursor is a [`Cursor`], and a frame the terminal took is a [`CompletedFrame`] with
//! its [`FrameStats`].
//!
//! The types used most are also at the crate's root. The
//! [repository](https://github.com/mesa-hills-research/termelon/tree/main/docs) has guides to how
//! the library works, capabilities and their fallbacks, testing, and porting from ratatui and
//! crossterm.
//!
//! # Features
//!
//! | Feature | Adds |
//! |---|---|
//! | `testing` | The `testing` module. It depends on `termelon-vt`, the emulator |
//! | `stream` | `futures_core::Stream` for [`Events`] |

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
pub use restore::{RestoreHandle, install_panic_hook, is_writer_thread};
pub use style::{Color, Modifier, Style, UnderlineStyle};
pub use terminal::{Drawn, Error, Events, KeyboardRequest, MouseMode, Options, Result};
#[cfg(any(unix, windows))]
pub use terminal::{Terminal, terminal_size};
pub use text::{Line, Span, Text};
pub use widgets::{Block, BorderSet, Borders, Clear, Padding, Paragraph, Widget, Wrap};
