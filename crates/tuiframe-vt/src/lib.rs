//! A terminal emulator in a library, for tests. It keeps the screen a terminal would show for the bytes
//! a program writes, answers queries as a chosen terminal would, and encodes keys the way that
//! terminal sends them.
//!
//! ```
//! use tuiframe_vt::{Emulator, Event, Profile};
//!
//! let mut term = Emulator::with_profile(Profile::kitty(), 40, 10);
//! term.feed(b"\x1b[?25l\x1b[3;1H\x1b[38;2;216;222;233mready\x1b[0m\x1b[?25h").unwrap();
//! assert_eq!(term.row_text(2).as_deref(), Some("ready"));
//!
//! // No character was drawn while the cursor showed.
//! assert!(!term.events().iter().any(|e| matches!(e, Event::Drawn { cursor_visible: true, .. })));
//!
//! // Queries are answered as kitty answers them.
//! term.feed(b"\x1b[?u\x1b[c").unwrap();
//! assert_eq!(term.take_output(), b"\x1b[?0u\x1b[?62;c");
//! ```
//!
//! # What it models
//!
//! The parser has the shape of Paul Williams' VT500 state machine and never panics or grows past
//! fixed bounds, whatever the input. The screen is a grid of cells, each a grapheme cluster with its
//! width and the full style (colours by name, index or RGB, every SGR attribute, underline styles
//! and colour, and OSC 8 hyperlinks). It has the cursor with its visibility and shape, the main and
//! alternate screens, autowrap with xterm's pending-wrap state, origin mode, scroll regions, the
//! title and its stack, OSC 52 writes, and the modes a terminal UI uses: mouse reports (9, 1000,
//! 1002, 1003, 1006), focus (1004), bracketed paste (2004), synchronized update (2026), grapheme
//! clusters (2027), and the kitty keyboard flags.
//!
//! # Strict mode
//!
//! Strict mode is on from the start. A sequence the emulator doesn't know, invalid UTF-8 or a
//! sequence past the parser's bounds is skipped, recorded with its bytes, and makes
//! [`Emulator::feed`] fail, so a test fails on anything nobody tested. A sequence the emulator knows
//! but the profile's terminal ignores is ignored without an error, as that terminal would.
//!
//! # Events
//!
//! [`Emulator::events`] lists what the terminal did, in byte order: each character drawn (with
//! where the cursor was and whether it showed), cursor moves, the cursor shown and hidden,
//! synchronized updates begun and ended, modes set and reset, titles and the bell.
//!
//! # Widths
//!
//! Text is split into grapheme clusters as it comes, and each cluster is measured by the
//! profile's [`WidthRules`]. A character that continues the cluster before it joins that cell,
//! which is measured again, so a cluster written one scalar at a time ends up as if written whole.
//! While a program has mode 2027 set, clusters are measured as if `joins_emoji` were on.

mod emulator;
mod event;
mod grid;
mod parser;
mod profile;
mod style;
mod width;

pub use emulator::{Cursor, CursorShape, Emulator};
pub use event::{ClipboardWrite, Event, FeedError, Pos, UnknownSequence};
pub use grid::Cell;
pub use profile::{KeyEncoding, Profile};
pub use style::{Color, Hyperlink, Style, Underline};
pub use width::WidthRules;
