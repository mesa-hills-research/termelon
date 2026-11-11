//! termelon's buffer and encoder. A frame is written into the back buffer, a run of ASCII text in
//! one style with `set_string` and every other cell with `cell_mut`, as a program writes spans, and
//! the encoder then diffs it against the front buffer into one packet, as `Terminal::draw` does
//! after its closure. The buffers then swap.
//!
//! Left out are the hand-off to the writer thread and the size check before each frame.

use termelon::buffer::Buffer;
use termelon::caps::{Capabilities, ColorDepth, Support, WidthProfile};
use termelon::layout::{Position, Rect};
use termelon::style::{Color as TfColor, Modifier, Style as TfStyle, UnderlineStyle};
use termelon::unstable::Encoder;
use termelon::{Cursor as TfCursor, CursorShape as TfShape};

use crate::model::{Color, Cursor, CursorShape, Frame, Mods, Named, Style, Symbol, Underline};
use crate::stack::Stack;

/// Room for the output of one frame, so the buffer doesn't grow while a run is timed.
const OUTPUT_CAPACITY: usize = 4 << 20;

/// Clusters measured as ratatui measures them, by unicode-width's width of the whole cluster, so
/// both stacks lay out the same frames.
const PROFILE: WidthProfile = WidthProfile::new(true, true, false);

/// Termelon drawing into memory.
#[derive(Debug)]
pub(crate) struct TermelonStack {
    front: Buffer,
    back: Buffer,
    encoder: Encoder,
    caps: Capabilities,
    out: Vec<u8>,
    /// A run of ASCII text in one style, waiting to be written.
    text: String,
}

impl TermelonStack {
    /// A stack for a screen of `width` by `height`, with scroll detection as `scroll` says.
    pub(crate) fn new(width: u16, height: u16, scroll: bool) -> TermelonStack {
        let area = Rect::new(0, 0, width, height);
        let mut encoder = Encoder::new();
        encoder.set_scroll_detection(scroll);
        // What the ratatui stack writes. It has no hyperlinks or underline shapes.
        let mut caps = Capabilities::default();
        caps.colors = ColorDepth::TrueColor;
        caps.synchronized_output = Support::Yes;
        caps.underline_color = Support::Yes;
        caps.cursor_shape = Support::Yes;
        // Erasing with the background, as every terminal of the known-terminal table does but
        // tmux and GNU screen.
        caps.bce = Support::Yes;
        TermelonStack {
            front: Buffer::with_width_profile(area, PROFILE),
            back: Buffer::with_width_profile(area, PROFILE),
            encoder,
            caps,
            out: Vec::with_capacity(OUTPUT_CAPACITY),
            text: String::new(),
        }
    }

    /// Writes the waiting run of text from column `x` of row `y`.
    fn flush(&mut self, x: u16, y: u16, style: TfStyle) {
        if !self.text.is_empty() {
            self.back.set_string(x, y, &self.text, style);
            self.text.clear();
        }
    }
}

impl Stack for TermelonStack {
    fn resize(&mut self, width: u16, height: u16) {
        let area = Rect::new(0, 0, width, height);
        self.front.resize(area);
        self.back.resize(area);
        self.encoder.redraw();
    }

    fn render(&mut self, frame: &Frame) {
        self.back.reset();
        if self.back.area().width != frame.width() || self.back.area().height != frame.height() {
            return;
        }
        let mut scratch = [0; 4];
        for y in 0..frame.height() {
            let mut run = (0, Style::default(), convert(Style::default()));
            for (x, cell) in (0..).zip(frame.row(y)) {
                let ascii = matches!(cell.symbol, Symbol::Char(c) if c.is_ascii());
                if !ascii || cell.style != run.1 {
                    self.flush(run.0, y, run.2);
                    run = (x, cell.style, convert(cell.style));
                }
                match cell.symbol {
                    Symbol::Continuation => run.0 = x + 1,
                    Symbol::Char(c) if c.is_ascii() => self.text.push(c),
                    symbol => {
                        if let Some(mut target) = self.back.cell_mut((x, y)) {
                            target
                                .set_symbol(frame.symbol(symbol, &mut scratch))
                                .set_style(run.2);
                        }
                        run.0 = x + 1;
                    }
                }
                if let Some(url) = frame.link_of(cell.style)
                    && let Some(mut target) = self.back.cell_mut((x, y))
                {
                    target.set_link(Some(url));
                }
            }
            self.flush(run.0, y, run.2);
        }
    }

    fn present(&mut self, cursor: Cursor) -> &[u8] {
        self.out.clear();
        let cursor = cursor.visible.then(|| TfCursor {
            position: Position::new(cursor.x, cursor.y),
            shape: match cursor.shape {
                CursorShape::Default => TfShape::Default,
                CursorShape::Block => TfShape::Block,
                CursorShape::Underline => TfShape::Underline,
                CursorShape::Bar => TfShape::Bar,
            },
            blinking: false,
        });
        self.encoder.encode(
            &self.front,
            &self.back,
            cursor.as_ref(),
            &self.caps,
            &mut self.out,
        );
        std::mem::swap(&mut self.front, &mut self.back);
        &self.out
    }
}

/// A cell's style in termelon's types, with every field set.
fn convert(style: Style) -> TfStyle {
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
        (Mods::OVERLINE, Modifier::OVERLINED),
    ] {
        if style.mods.contains(mods) {
            modifier |= flag;
        }
    }
    let underline = match style.underline {
        Underline::None => None,
        Underline::Single => Some(UnderlineStyle::Single),
        Underline::Double => Some(UnderlineStyle::Double),
        Underline::Curly => Some(UnderlineStyle::Curly),
        Underline::Dotted => Some(UnderlineStyle::Dotted),
        Underline::Dashed => Some(UnderlineStyle::Dashed),
    };
    if underline.is_some() {
        modifier |= Modifier::UNDERLINED;
    }
    TfStyle {
        fg: Some(color(style.fg)),
        bg: Some(color(style.bg)),
        underline_color: Some(color(style.ul)),
        underline_style: underline,
        add_modifier: modifier,
        sub_modifier: Modifier::empty(),
    }
}

fn color(color: Color) -> TfColor {
    match color {
        Color::Default => TfColor::Reset,
        Color::Indexed(index) => TfColor::Indexed(index),
        Color::Rgb(r, g, b) => TfColor::Rgb(r, g, b),
        Color::Named(named) => match named {
            Named::Black => TfColor::Black,
            Named::Red => TfColor::Red,
            Named::Green => TfColor::Green,
            Named::Yellow => TfColor::Yellow,
            Named::Blue => TfColor::Blue,
            Named::Magenta => TfColor::Magenta,
            Named::Cyan => TfColor::Cyan,
            Named::Gray => TfColor::Gray,
            Named::DarkGray => TfColor::DarkGray,
            Named::LightRed => TfColor::LightRed,
            Named::LightGreen => TfColor::LightGreen,
            Named::LightYellow => TfColor::LightYellow,
            Named::LightBlue => TfColor::LightBlue,
            Named::LightMagenta => TfColor::LightMagenta,
            Named::LightCyan => TfColor::LightCyan,
            Named::White => TfColor::White,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draw(stack: &mut TermelonStack, frame: &Frame) -> String {
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
        let mut stack = TermelonStack::new(10, 3, false);
        let out = draw(&mut stack, &frame_with("hello"));
        assert!(out.starts_with("\x1b[?2026h"), "{out:?}");
        assert!(out.ends_with("\x1b[?2026l"), "{out:?}");
        assert!(out.contains("hello"), "{out:?}");
    }

    #[test]
    fn only_the_changes_are_drawn_the_second_time() {
        let mut stack = TermelonStack::new(10, 3, false);
        let first = frame_with("hello");
        draw(&mut stack, &first);
        assert_eq!(draw(&mut stack, &first), "");
        let changed = draw(&mut stack, &frame_with("jello"));
        assert!(
            changed.contains('j') && !changed.contains("ello"),
            "{changed:?}"
        );
    }

    #[test]
    fn the_buffer_holds_what_the_frame_holds() {
        let mut frame = Frame::new(12, 2);
        let style = Style::default().fg(Color::Rgb(1, 2, 3)).bold();
        frame.put_str(0, 0, "a界b👍🏽", style, 12);
        let mut stack = TermelonStack::new(12, 2, false);
        stack.render(&frame);
        let row: String = (0..12).map(|x| stack.back.symbol((x, 0))).collect();
        assert_eq!(row.trim_end(), frame.row_text(0));
        let cell = stack.back.cell((0, 0)).unwrap();
        assert_eq!(cell.fg(), TfColor::Rgb(1, 2, 3));
        assert!(cell.modifier().contains(Modifier::BOLD));
    }

    #[test]
    fn a_resize_redraws_everything() {
        let mut stack = TermelonStack::new(10, 3, false);
        let first = frame_with("hello");
        draw(&mut stack, &first);
        stack.resize(10, 3);
        let again = draw(&mut stack, &first);
        assert!(
            again.contains("\x1b[2J") && again.contains("hello"),
            "{again:?}"
        );
    }
}
