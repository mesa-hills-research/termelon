//! [`Frame`], what a program draws one frame into.

use crate::buffer::Buffer;
use crate::layout::{Position, Rect};
use crate::output::Cursor;
use crate::widgets::Widget;

/// One frame being drawn, over a buffer the size of the terminal. A full frame that sets no cursor
/// hides it, and a partial one keeps the last frame's.
#[derive(Debug)]
pub struct Frame<'a> {
    buffer: &'a mut Buffer,
    cursor: Option<Cursor>,
}

impl<'a> Frame<'a> {
    pub(crate) fn new(buffer: &'a mut Buffer, cursor: Option<Cursor>) -> Self {
        Self { buffer, cursor }
    }

    /// The area to draw in, the whole terminal.
    pub fn area(&self) -> Rect {
        self.buffer.area()
    }

    /// The cells, to draw into.
    pub fn buffer_mut(&mut self) -> &mut Buffer {
        self.buffer
    }

    /// Draws `widget` into `area`.
    pub fn render_widget<W: Widget>(&mut self, widget: W, area: Rect) {
        widget.render(area, self.buffer);
    }

    /// Where the cursor goes and how it looks, or `None` to hide it.
    pub fn set_cursor(&mut self, cursor: Option<Cursor>) {
        self.cursor = cursor;
    }

    /// Shows the cursor at `position`, in the terminal's default shape.
    pub fn set_cursor_position<P: Into<Position>>(&mut self, position: P) {
        self.cursor = Some(Cursor::at(position.into()));
    }

    /// The cursor the frame asks for.
    pub fn cursor(&self) -> Option<Cursor> {
        self.cursor
    }

    pub(crate) fn into_cursor(self) -> Option<Cursor> {
        self.cursor
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::CursorShape;

    #[test]
    fn a_frame_hides_the_cursor_unless_it_sets_one() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 3));
        let mut frame = Frame::new(&mut buffer, None);
        assert_eq!(frame.area(), Rect::new(0, 0, 10, 3));
        assert_eq!(frame.cursor(), None);
        frame.set_cursor_position((2, 1));
        assert_eq!(frame.cursor(), Some(Cursor::at(Position::new(2, 1))));
        let bar = Cursor {
            position: Position::new(0, 0),
            shape: CursorShape::Bar,
            blinking: true,
        };
        frame.set_cursor(Some(bar));
        frame
            .buffer_mut()
            .set_string(0, 0, "hi", crate::style::Style::new());
        frame.render_widget("yo", Rect::new(5, 2, 3, 1));
        assert_eq!(frame.into_cursor(), Some(bar));
        assert_eq!(buffer.symbol((1, 0)), "i");
        assert_eq!(buffer.symbol((6, 2)), "o");
    }
}
