//! A terminal in memory for tests, [`TestTerminal`], and [`golden`] files to compare its screen
//! with.
//!
//! [`TestTerminal`] draws frames as `Terminal` does, through the same buffers, diff and encoder,
//! and feeds the bytes to [`vt`], a terminal emulator. So its [`screen`](TestTerminal::screen) is
//! what a terminal would show, the cursor included, and a test can check the bytes too. The module
//! needs the `testing` feature, which brings in the emulator.
//!
//! ```
//! use termelon::layout::{Position, Rect};
//! use termelon::testing::TestTerminal;
//! use termelon::text::Line;
//!
//! let mut term = TestTerminal::new(20, 3);
//! term.draw(|frame| {
//!     frame.render_widget(Line::from("hello"), Rect::new(0, 1, 20, 1));
//!     frame.set_cursor_position((5, 1));
//! })
//! .unwrap();
//! assert_eq!(term.screen().rows(), ["", "hello", ""]);
//! assert_eq!(term.cursor().map(|c| c.position), Some(Position::new(5, 1)));
//! ```

pub mod golden;
#[cfg(test)]
mod tests;

/// The terminal emulator behind [`TestTerminal`], for its profiles, cells and colours.
pub use termelon_vt as vt;

use crate::caps::env::Hints;
use crate::caps::{self, Capabilities, probe};
use crate::event::decode::{Decoded, Decoder, DecoderOptions};
use crate::frame::{Frame, Frames};
use crate::layout::{Position, Rect, Size};
use crate::output::{CompletedFrame, Cursor, CursorShape};
use crate::terminal::{Drawn, Options, enabled};
use crate::text;

/// A terminal in memory, which draws frames as `Terminal` does and keeps what a terminal would show
/// for the bytes.
///
/// It plays xterm unless [`with_profile`](Self::with_profile) names another terminal, and its
/// capabilities are what the startup probe finds in that terminal unless
/// [`with_caps`](Self::with_caps) gives them. Its first frame sends the setup a terminal gets at
/// open, for the [`Options`] given, so [`take_output`](Self::take_output) starts with it. Frames
/// are drawn with the capabilities' width profile, which text measures with inside the closure.
///
/// ```
/// use termelon::caps::{Capabilities, ColorDepth};
/// use termelon::layout::Rect;
/// use termelon::style::{Color, Style};
/// use termelon::testing::{TestTerminal, vt};
/// use termelon::text::Span;
///
/// let mut caps = Capabilities::default();
/// caps.colors = ColorDepth::Ansi256;
/// let mut term = TestTerminal::new(10, 1).with_profile(vt::Profile::kitty()).with_caps(caps);
/// term.draw(|frame| {
///     let ok = Span::styled("ok", Style::new().fg(Color::Rgb(0, 175, 0)));
///     frame.render_widget(ok, Rect::new(0, 0, 10, 1));
/// })
/// .unwrap();
/// let screen = term.screen();
/// assert_eq!(screen.cell(0, 0).map(|c| c.style().fg), Some(vt::Color::Indexed(34)));
/// assert!(screen.styled().contains("style 0 0 2 fg=idx:34"));
/// ```
#[derive(Debug)]
pub struct TestTerminal {
    emulator: vt::Emulator,
    size: Size,
    caps: Capabilities,
    /// Whether the capabilities came from [`with_caps`](Self::with_caps) rather than the probe.
    caps_given: bool,
    options: Options,
    frames: Frames,
    output: Vec<u8>,
    /// The next frame redraws everything, as the first one does, and one after a resize or a
    /// clear.
    redraw: bool,
    /// Whether the setup went out.
    opened: bool,
}

impl TestTerminal {
    /// A terminal of `width` by `height` cells playing xterm.
    pub fn new(width: u16, height: u16) -> Self {
        let size = Size::new(width, height);
        let profile = vt::Profile::xterm();
        let caps = probed(&profile);
        Self {
            emulator: vt::Emulator::with_profile(profile, width, height),
            frames: Frames::new(Rect::from(size), caps.width_profile),
            size,
            caps,
            caps_given: false,
            options: Options::default(),
            output: Vec::new(),
            redraw: true,
            opened: false,
        }
    }

    /// The terminal playing the one `profile` describes, with the capabilities the probe finds in
    /// it unless [`with_caps`](Self::with_caps) gave them.
    #[must_use]
    pub fn with_profile(mut self, profile: vt::Profile) -> Self {
        if !self.caps_given {
            self.caps = probed(&profile);
        }
        self.emulator = vt::Emulator::with_profile(profile, self.size.width, self.size.height);
        self.reopen()
    }

    /// The terminal drawing for `caps`, whatever its profile answers.
    #[must_use]
    pub fn with_caps(mut self, caps: Capabilities) -> Self {
        self.caps = caps;
        self.caps_given = true;
        self.reopen()
    }

    /// The terminal set up with `options`, as `Terminal::open` would set it up. Only the modes it
    /// turns on matter here.
    #[must_use]
    pub fn with_options(mut self, options: Options) -> Self {
        self.options = options;
        self.reopen()
    }

    /// The terminal as it is before its first frame, with what changed taken into account.
    fn reopen(mut self) -> Self {
        self.frames = Frames::new(Rect::from(self.size), self.caps.width_profile);
        self.output.clear();
        self.redraw = true;
        self.opened = false;
        self
    }

    /// Draws a frame as `Terminal::draw` does, never holding it. The first frame, and the first
    /// after a [`resize`](Self::resize) or a [`clear`](Self::clear), redraws everything.
    ///
    /// # Errors
    ///
    /// The sequences the emulator didn't know, which the frame wrote all the same.
    pub fn draw<F: FnOnce(&mut Frame<'_>)>(
        &mut self,
        render: F,
    ) -> Result<CompletedFrame<'_>, vt::FeedError> {
        let opened = self.open();
        if self.frames.resize(Rect::from(self.size)) {
            self.redraw = true;
        }
        let frames = &mut self.frames;
        let cursor = text::with_width_profile(self.caps.width_profile, || frames.render(render));
        let full = std::mem::take(&mut self.redraw);
        let mut packet = Vec::new();
        let stats = self.frames.encode(full, cursor, &self.caps, &mut packet);
        let fed = self.send(&packet);
        let done = self.frames.commit(cursor, stats);
        opened.and(fed).map(|()| done)
    }

    /// Draws a partial frame as `Terminal::draw_regions` does, changing `regions` alone. It returns
    /// [`Drawn::FullFrameDue`] without calling `render` before the first frame and when a full
    /// frame is due.
    ///
    /// # Errors
    ///
    /// The sequences the emulator didn't know, which the frame wrote all the same.
    pub fn draw_regions<F: FnOnce(&mut Frame<'_>)>(
        &mut self,
        regions: &[Rect],
        render: F,
    ) -> Result<Drawn<'_>, vt::FeedError> {
        if self.redraw || Rect::from(self.size) != self.frames.area() {
            return Ok(Drawn::FullFrameDue);
        }
        let frames = &mut self.frames;
        let cursor = text::with_width_profile(self.caps.width_profile, || {
            frames.render_regions(regions, render)
        });
        let mut packet = Vec::new();
        let stats = self
            .frames
            .encode_regions(regions, cursor, &self.caps, &mut packet);
        let fed = self.send(&packet);
        let done = self.frames.commit_regions(cursor, stats);
        fed.map(|()| Drawn::Written(done))
    }

    /// Changes the size, as a terminal window does. The emulator's screen keeps what fits, and the
    /// next frame redraws everything.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.size = Size::new(width, height);
        self.emulator.resize(width, height);
    }

    /// Makes the next frame redraw everything, as `Terminal::clear` does.
    pub fn clear(&mut self) {
        self.redraw = true;
    }

    /// The bytes written since the last call, as a terminal would have been sent them.
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }

    /// What the terminal shows.
    pub fn screen(&self) -> Screen<'_> {
        Screen {
            emulator: &self.emulator,
        }
    }

    /// The cursor as the terminal shows it, or `None` when it's hidden.
    pub fn cursor(&self) -> Option<Cursor> {
        self.screen().cursor()
    }

    /// The capabilities frames are drawn for.
    pub fn capabilities(&self) -> &Capabilities {
        &self.caps
    }

    /// The size, in cells.
    pub fn size(&self) -> Size {
        self.size
    }

    /// The emulator, for what [`Screen`] doesn't show, such as the modes set or the events.
    pub fn emulator(&self) -> &vt::Emulator {
        &self.emulator
    }

    /// Sends the setup before the first frame.
    fn open(&mut self) -> Result<(), vt::FeedError> {
        if self.opened {
            return Ok(());
        }
        self.opened = true;
        self.send(&enabled(&self.options, &self.caps).setup())
    }

    fn send(&mut self, bytes: &[u8]) -> Result<(), vt::FeedError> {
        self.output.extend_from_slice(bytes);
        self.emulator.feed(bytes)
    }
}

/// The capabilities the startup probe finds in the terminal `profile` describes, with no
/// environment variables.
fn probed(profile: &vt::Profile) -> Capabilities {
    let mut terminal = vt::Emulator::with_profile(profile.clone(), 80, 24);
    terminal.set_strict(false);
    let _ = terminal.feed(&probe::queries(false));
    let mut decoded = Vec::new();
    Decoder::new(DecoderOptions::default()).feed(&terminal.take_output(), &mut decoded);
    let replies: Vec<_> = decoded
        .into_iter()
        .filter_map(|decoded| match decoded {
            Decoded::Reply(reply) => Some(reply),
            Decoded::Event(_) => None,
        })
        .collect();
    caps::detect(&probe::answers(&replies), &Hints::default(), None)
}

/// What a [`TestTerminal`] shows, from [`TestTerminal::screen`].
#[derive(Debug, Clone, Copy)]
pub struct Screen<'a> {
    emulator: &'a vt::Emulator,
}

impl<'a> Screen<'a> {
    /// The text, a line for each row, with continuation cells left out and trailing spaces
    /// trimmed.
    pub fn text(&self) -> String {
        self.emulator.text()
    }

    /// Each row's text, as [`text`](Self::text) gives it.
    pub fn rows(&self) -> Vec<String> {
        self.emulator.rows()
    }

    /// The screen in the styled-text format, its text with a line for each run of styled cells,
    /// which [`vt`] documents.
    pub fn styled(&self) -> String {
        self.emulator.styled()
    }

    /// The screen as an HTML fragment, with its colours and attributes.
    pub fn to_html(&self) -> String {
        self.emulator.to_html()
    }

    /// The screen as an SVG image, with its colours, attributes and cursor. [`vt::SvgAnimation`]
    /// makes an animation of screens one after another.
    pub fn to_svg(&self) -> String {
        self.emulator.to_svg()
    }

    /// The cursor, or `None` when it's hidden.
    pub fn cursor(&self) -> Option<Cursor> {
        let cursor = self.emulator.cursor();
        cursor.visible.then(|| Cursor {
            position: Position::new(cursor.x, cursor.y),
            shape: match cursor.shape {
                vt::CursorShape::Default => CursorShape::Default,
                vt::CursorShape::Block => CursorShape::Block,
                vt::CursorShape::Underline => CursorShape::Underline,
                vt::CursorShape::Bar => CursorShape::Bar,
            },
            blinking: cursor.blinking,
        })
    }

    /// The cell at column `x` and row `y`, or `None` outside the screen.
    pub fn cell(&self, x: u16, y: u16) -> Option<&'a vt::Cell> {
        self.emulator.cell(x, y)
    }
}
