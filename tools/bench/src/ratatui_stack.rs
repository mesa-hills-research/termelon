//! ratatui 0.30 over crossterm 0.29, as Arko uses them. A frame is built into the terminal's current
//! buffer, then `Terminal::apply_buffer_with_cursor` does what `Terminal::draw` does after its
//! callback. That is the diff, the cursor, the buffer swap and reset, and the backend's flush, all
//! through `CrosstermBackend<Vec<u8>>`. A synchronized update wraps it, as in Arko's `draw`.
//!
//! Left out are `Terminal::draw`'s size query before each frame, a system call that a real terminal
//! pays, and Arko's own `CursorBackend` around the crossterm one.

use std::io;

use crossterm::queue;
use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};
use ratatui::backend::{Backend, ClearType, CrosstermBackend, WindowSize};
use ratatui::buffer::Cell as RatCell;
use ratatui::layout::{Position, Rect, Size};
use ratatui::style::{Color as RatColor, Modifier};
use ratatui::{Terminal, TerminalOptions, Viewport};

use crate::model::{Color, Cursor, Frame, Mods, Named, Style, Symbol, Underline};
use crate::stack::Stack;

/// Room for the output of one frame, so the buffer doesn't grow while a run is timed.
const OUTPUT_CAPACITY: usize = 4 << 20;

/// `CrosstermBackend<Vec<u8>>` with a size it reports itself, since there is no terminal to ask.
#[derive(Debug)]
struct MemoryBackend {
    inner: CrosstermBackend<Vec<u8>>,
    size: Size,
}

impl Backend for MemoryBackend {
    type Error = io::Error;

    fn draw<'a, I>(&mut self, content: I) -> io::Result<()>
    where
        I: Iterator<Item = (u16, u16, &'a RatCell)>,
    {
        self.inner.draw(content)
    }

    fn append_lines(&mut self, n: u16) -> io::Result<()> {
        self.inner.append_lines(n)
    }

    fn hide_cursor(&mut self) -> io::Result<()> {
        self.inner.hide_cursor()
    }

    fn show_cursor(&mut self) -> io::Result<()> {
        self.inner.show_cursor()
    }

    fn get_cursor_position(&mut self) -> io::Result<Position> {
        Ok(Position::ORIGIN)
    }

    fn set_cursor_position<P: Into<Position>>(&mut self, position: P) -> io::Result<()> {
        self.inner.set_cursor_position(position)
    }

    fn clear(&mut self) -> io::Result<()> {
        self.inner.clear()
    }

    fn clear_region(&mut self, clear_type: ClearType) -> io::Result<()> {
        self.inner.clear_region(clear_type)
    }

    fn size(&self) -> io::Result<Size> {
        Ok(self.size)
    }

    fn window_size(&mut self) -> io::Result<WindowSize> {
        Ok(WindowSize {
            columns_rows: self.size,
            pixels: Size::new(0, 0),
        })
    }

    fn flush(&mut self) -> io::Result<()> {
        Backend::flush(&mut self.inner)
    }
}

/// ratatui and crossterm drawing into memory.
#[derive(Debug)]
pub(crate) struct RatatuiStack {
    terminal: Terminal<MemoryBackend>,
    /// Whether the last frame's bytes are still in the output.
    presented: bool,
}

impl RatatuiStack {
    /// A stack for a screen of `width` by `height`.
    ///
    /// # Errors
    ///
    /// Fails if ratatui can't set up its terminal, which the in-memory backend doesn't cause.
    pub(crate) fn new(width: u16, height: u16) -> io::Result<RatatuiStack> {
        let backend = MemoryBackend {
            inner: CrosstermBackend::new(Vec::with_capacity(OUTPUT_CAPACITY)),
            size: Size::new(width, height),
        };
        let terminal = Terminal::with_options(
            backend,
            TerminalOptions {
                viewport: Viewport::Fullscreen,
            },
        )?;
        Ok(RatatuiStack {
            terminal,
            presented: false,
        })
    }

    /// Empties the output once its frame has been read.
    fn start(&mut self) {
        if self.presented {
            self.terminal.backend_mut().inner.writer_mut().clear();
            self.presented = false;
        }
    }
}

impl Stack for RatatuiStack {
    fn resize(&mut self, width: u16, height: u16) {
        self.start();
        self.terminal.backend_mut().size = Size::new(width, height);
        // What Arko does to repaint everything. This clears the screen and forgets the last frame.
        let _ = self.terminal.resize(Rect::new(0, 0, width, height));
    }

    fn render(&mut self, frame: &Frame) {
        let buffer = self.terminal.current_buffer_mut();
        if buffer.area.width != frame.width() || buffer.area.height != frame.height() {
            return;
        }
        let mut scratch = [0; 4];
        let mut last = (Style::default(), convert(Style::default()));
        for (target, cell) in buffer.content.iter_mut().zip(frame.cells()) {
            if cell.symbol == Symbol::Continuation {
                target.reset();
                continue;
            }
            target.set_symbol(frame.symbol(cell.symbol, &mut scratch));
            if cell.style != last.0 {
                last = (cell.style, convert(cell.style));
            }
            let (fg, bg, underline_color, modifier) = last.1;
            target.fg = fg;
            target.bg = bg;
            target.underline_color = underline_color;
            target.modifier = modifier;
        }
    }

    fn present(&mut self, cursor: Cursor) -> &[u8] {
        self.start();
        let _ = queue!(
            self.terminal.backend_mut().inner.writer_mut(),
            BeginSynchronizedUpdate
        );
        let position = cursor.visible.then_some(Position {
            x: cursor.x,
            y: cursor.y,
        });
        let _ = self.terminal.apply_buffer_with_cursor(position);
        let _ = queue!(
            self.terminal.backend_mut().inner.writer_mut(),
            EndSynchronizedUpdate
        );
        self.presented = true;
        self.terminal.backend().inner.writer()
    }
}

/// A cell's colours and modifiers in ratatui's types. ratatui has no overline, hyperlinks or underline
/// shapes, so those are dropped, and an underline of any shape is a plain one.
fn convert(style: Style) -> (RatColor, RatColor, RatColor, Modifier) {
    let mut modifier = Modifier::empty();
    for (mods, flag) in [
        (Mods::BOLD, Modifier::BOLD),
        (Mods::DIM, Modifier::DIM),
        (Mods::ITALIC, Modifier::ITALIC),
        (Mods::BLINK, Modifier::SLOW_BLINK),
        (Mods::RAPID_BLINK, Modifier::RAPID_BLINK),
        (Mods::REVERSE, Modifier::REVERSED),
        (Mods::HIDDEN, Modifier::HIDDEN),
        (Mods::STRIKE, Modifier::CROSSED_OUT),
    ] {
        if style.mods.contains(mods) {
            modifier |= flag;
        }
    }
    if style.underline != Underline::None {
        modifier |= Modifier::UNDERLINED;
    }
    (color(style.fg), color(style.bg), color(style.ul), modifier)
}

fn color(color: Color) -> RatColor {
    match color {
        Color::Default => RatColor::Reset,
        Color::Indexed(index) => RatColor::Indexed(index),
        Color::Rgb(r, g, b) => RatColor::Rgb(r, g, b),
        Color::Named(named) => match named {
            Named::Black => RatColor::Black,
            Named::Red => RatColor::Red,
            Named::Green => RatColor::Green,
            Named::Yellow => RatColor::Yellow,
            Named::Blue => RatColor::Blue,
            Named::Magenta => RatColor::Magenta,
            Named::Cyan => RatColor::Cyan,
            Named::Gray => RatColor::Gray,
            Named::DarkGray => RatColor::DarkGray,
            Named::LightRed => RatColor::LightRed,
            Named::LightGreen => RatColor::LightGreen,
            Named::LightYellow => RatColor::LightYellow,
            Named::LightBlue => RatColor::LightBlue,
            Named::LightMagenta => RatColor::LightMagenta,
            Named::LightCyan => RatColor::LightCyan,
            Named::White => RatColor::White,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::CursorShape;

    const BEGIN: &str = "\x1b[?2026h";
    const END: &str = "\x1b[?2026l";

    fn draw(stack: &mut RatatuiStack, frame: &Frame) -> String {
        stack.render(frame);
        String::from_utf8(stack.present(frame.cursor).to_vec()).unwrap()
    }

    fn frame_with(text: &str) -> Frame {
        let mut frame = Frame::new(10, 3);
        frame.put_str(0, 1, text, Style::default(), 10);
        frame
    }

    #[test]
    fn a_frame_is_one_synchronized_update_that_draws_the_text() {
        let mut stack = RatatuiStack::new(10, 3).unwrap();
        let out = draw(&mut stack, &frame_with("hello"));
        assert!(out.starts_with(BEGIN), "{out:?}");
        assert!(out.ends_with(END), "{out:?}");
        assert_eq!(out.matches(BEGIN).count(), 1);
        // The row, then the cells in the order of the buffer.
        assert!(out.contains("\x1b[2;1Hhello"), "{out:?}");
    }

    #[test]
    fn only_the_changes_are_drawn_the_second_time() {
        let mut stack = RatatuiStack::new(10, 3).unwrap();
        let first = frame_with("hello");
        draw(&mut stack, &first);
        let again = draw(&mut stack, &first);
        assert!(!again.contains("hello"), "{again:?}");
        let changed = draw(&mut stack, &frame_with("jello"));
        assert!(changed.contains("\x1b[2;1Hj"), "{changed:?}");
        assert!(!changed.contains("ello"), "{changed:?}");
        assert!(changed.len() < 80, "{changed:?}");
    }

    #[test]
    fn colours_and_modifiers_reach_the_output() {
        let mut stack = RatatuiStack::new(10, 3).unwrap();
        let mut frame = Frame::new(10, 3);
        let style = Style::default()
            .fg(Color::Rgb(1, 2, 3))
            .bg(Color::Indexed(9))
            .bold();
        frame.put_str(0, 0, "x", style, 10);
        let out = draw(&mut stack, &frame);
        assert!(out.contains("\x1b[1m"), "bold: {out:?}");
        assert!(out.contains("38;2;1;2;3"), "foreground: {out:?}");
        assert!(out.contains("48;5;9"), "background: {out:?}");
        assert!(out.contains('x'), "{out:?}");
    }

    #[test]
    fn a_visible_cursor_is_placed_and_shown_and_a_hidden_one_hidden() {
        let mut stack = RatatuiStack::new(10, 3).unwrap();
        let mut frame = Frame::new(10, 3);
        frame.cursor = Cursor {
            x: 4,
            y: 2,
            visible: true,
            shape: CursorShape::Bar,
        };
        let shown = draw(&mut stack, &frame);
        assert!(shown.contains("\x1b[?25h\x1b[3;5H"), "{shown:?}");
        frame.cursor.visible = false;
        let hidden = draw(&mut stack, &frame);
        assert!(hidden.contains("\x1b[?25l"), "{hidden:?}");
        assert!(!hidden.contains("\x1b[?25h"), "{hidden:?}");
    }

    #[test]
    fn wide_characters_and_clusters_are_drawn_once() {
        let mut stack = RatatuiStack::new(10, 3).unwrap();
        let mut frame = Frame::new(10, 3);
        frame.put_str(0, 0, "漢a👍🏽b", Style::default(), 10);
        let out = draw(&mut stack, &frame);
        // Each wide cell's continuation is skipped, and ratatui moves the cursor again after it.
        assert!(out.contains("漢\x1b[1;3Ha👍🏽\x1b[1;6Hb"), "{out:?}");
    }

    #[test]
    fn a_resize_clears_the_screen_and_repaints_everything() {
        let mut stack = RatatuiStack::new(10, 3).unwrap();
        let frame = frame_with("hello");
        draw(&mut stack, &frame);
        stack.resize(10, 3);
        let out = draw(&mut stack, &frame);
        assert!(out.contains("\x1b[2J"), "{out:?}");
        assert!(out.contains("hello"), "{out:?}");
        // A new size gets a buffer of that size.
        stack.resize(12, 4);
        let mut bigger = Frame::new(12, 4);
        bigger.put_str(0, 3, "tall", Style::default(), 12);
        let out = draw(&mut stack, &bigger);
        assert!(out.contains("\x1b[4;1Htall"), "{out:?}");
    }

    #[test]
    fn the_output_holds_one_frame_at_a_time() {
        let mut stack = RatatuiStack::new(10, 3).unwrap();
        let frame = frame_with("hello");
        let first = draw(&mut stack, &frame).len();
        let second = draw(&mut stack, &frame).len();
        assert!(second < first);
    }

    #[test]
    fn a_frame_of_the_wrong_size_is_ignored() {
        let mut stack = RatatuiStack::new(10, 3).unwrap();
        let out = draw(&mut stack, &Frame::new(5, 5));
        assert!(!out.contains("\x1b[1;1H"), "{out:?}");
    }

    #[test]
    fn named_colours_and_modifiers_convert() {
        let style = Style::default()
            .fg(Color::Named(Named::LightMagenta))
            .bg(Color::Named(Named::DarkGray))
            .mods(Mods::STRIKE)
            .italic();
        let (fg, bg, ul, modifier) = convert(Style {
            underline: Underline::Curly,
            ..style
        });
        assert_eq!(fg, RatColor::LightMagenta);
        assert_eq!(bg, RatColor::DarkGray);
        assert_eq!(ul, RatColor::Reset);
        assert_eq!(
            modifier,
            Modifier::CROSSED_OUT | Modifier::ITALIC | Modifier::UNDERLINED
        );
    }
}
