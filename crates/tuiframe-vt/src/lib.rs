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
//!
//! # Where terminals differ
//!
//! The emulator follows xterm where terminals disagree. Its tests replay random streams through
//! `alacritty_terminal` as well and compare the screens, and these are the places the two differ,
//! each pinned by a test.
//!
//! - SGR 21 is a double underline, as in xterm and kitty. Alacritty takes it as bold off.
//! - LF, IND, NEL and RI end a pending wrap. Alacritty keeps it, so its next character wraps again.
//! - EL 0 and ED 0 at a pending wrap erase the last column and end the pending wrap. There
//!   Alacritty's EL 0 erases nothing, and its ED 0 keeps the pending wrap.
//! - HT at a pending wrap stays in the last column. Alacritty wraps.
//! - CUU, CUD, CNL and CPL stop at the scroll region's margins. Alacritty's go to the screen's edge.
//! - Resetting origin mode moves the cursor home. In origin mode, Alacritty adds the region's top
//!   again to CHA and relative moves.
//! - ED 1 from the second row erases the first row too. Alacritty's leaves it.
//! - Erasing part of a wide character blanks all of it. Alacritty leaves the other half.
//! - An emoji ZWJ sequence or a flag is one cell as wide as the cluster. Alacritty gives each emoji
//!   a cell of its own, with the same text and cursor.
//! - A combining mark at the left edge, with nothing before it, is dropped. Alacritty joins it to
//!   the cell under the cursor.
//! - A wide character that doesn't fit with autowrap off moves left until it fits, as in kitty.
//!   Alacritty drops it.
//! - A scroll region that starts past the screen is ignored. Alacritty sets an empty region.
//! - Resetting any of modes 9, 1000, 1002 and 1003 turns mouse reports off, as in xterm. Alacritty
//!   resets only the mode named.
//!
//! # The styled-text format, version 1
//!
//! [`Emulator::styled`] writes the screen in a line format that reads well in a diff, and other
//! tools write the same format, so the two can be compared as text.
//!
//! ```text
//! size 80 24
//! cursor 2 23 visible bar
//! text 0 "  ⣷ Working…"
//! style 0 2 3 fg=#7f8ea3
//! style 0 4 13 fg=#d8dee9 bold
//! text 1 ""
//! ```
//!
//! - `size` gives the columns and rows.
//! - `cursor` gives the cursor's column and row, then `visible` and its shape (`block`,
//!   `underline`, `bar` or `default`), or `hidden` with no shape.
//! - Each row has one `text` line, in row order, with its text as a JSON string. Continuation cells
//!   are left out and trailing spaces trimmed. The escapes are those of `serde_json`, which escapes
//!   `"`, `\` and the C0 controls and leaves everything else as UTF-8.
//! - After each row's `text` line come its `style` lines, one per run of cells that write the same
//!   attributes: the row, the start column and the end column (exclusive), counted in cells, then the
//!   attributes. A run whose cells are all in the default style writes no line. A run can go past the
//!   trimmed text, as a coloured background does.
//! - The attributes come in this order: `fg=`, `bg=`, `ul=` (the underline colour), `underline` or
//!   `underline=double|curly|dotted|dashed`, `bold`, `dim`, `italic`, `blink`, `rapid-blink`,
//!   `reverse`, `hidden`, `strike`, `overline`, `link=<url>`. A colour is `#rrggbb`, `idx:N`, or one of
//!   `black red green yellow blue magenta cyan gray dark-gray light-red light-green light-yellow
//!   light-blue light-magenta light-cyan white`, ratatui's names for palette 0 to 15.
//! - Cells whose attributes write the same are one run, so two links to one address with different
//!   ids are one run.
//! - Every line ends with `\n`, the last one too.

mod emulator;
mod event;
mod grid;
mod input;
mod parser;
mod profile;
mod style;
mod width;

pub use emulator::{Cursor, CursorShape, Emulator};
pub use event::{ClipboardWrite, Event, FeedError, Pos, UnknownSequence};
pub use grid::Cell;
pub use input::{Key, KeyKind, Mods, Mouse, MouseButton, MouseKind};
pub use profile::{KeyEncoding, Profile};
pub use style::{Color, Hyperlink, Style, Underline};
pub use width::WidthRules;
