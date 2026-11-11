//! [`Frame`], what a program draws one frame into, and the buffers frames are drawn with.

use crate::buffer::Buffer;
use crate::caps::{Capabilities, WidthProfile};
use crate::layout::{Position, Rect};
use crate::output::encode::Encoder;
use crate::output::{CompletedFrame, Cursor, FrameStats};
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

/// The two buffers and the encoder that frames are drawn and written with, for the terminal and
/// for the test terminal alike.
///
/// The front buffer holds what the terminal shows once every packet so far is written. A full
/// frame draws into the back buffer over blank cells, and its packet takes the terminal from the
/// front buffer to it. A partial frame draws over the front buffer itself, inside its regions.
#[derive(Debug)]
pub(crate) struct Frames {
    front: Buffer,
    back: Buffer,
    encoder: Encoder,
    /// Where the last frame put the cursor.
    cursor: Option<Cursor>,
}

impl Frames {
    /// The buffers of a terminal of `area` whose screen is blank, measuring text with `profile`.
    pub(crate) fn new(area: Rect, profile: WidthProfile) -> Self {
        Self {
            front: Buffer::with_width_profile(area, profile),
            back: Buffer::with_width_profile(area, profile),
            encoder: Encoder::new(),
            cursor: None,
        }
    }

    /// The area frames cover.
    pub(crate) fn area(&self) -> Rect {
        self.front.area()
    }

    /// Makes both buffers cover `area`, and says whether that changed them, in which case the next
    /// frame has to redraw everything.
    pub(crate) fn resize(&mut self, area: Rect) -> bool {
        if area == self.front.area() {
            return false;
        }
        self.front.resize(area);
        self.back.resize(area);
        true
    }

    /// Runs `render` with a frame over the back buffer, blanked, and gives the cursor it asked for.
    pub(crate) fn render<F: FnOnce(&mut Frame<'_>)>(&mut self, render: F) -> Option<Cursor> {
        self.back.reset();
        let mut frame = Frame::new(&mut self.back, None);
        render(&mut frame);
        frame.into_cursor()
    }

    /// Appends to `packet` what takes the terminal from the front buffer to the frame just
    /// rendered, with the cursor where `cursor` says, and everything when `full`.
    pub(crate) fn encode(
        &mut self,
        full: bool,
        cursor: Option<Cursor>,
        caps: &Capabilities,
        packet: &mut Vec<u8>,
    ) -> FrameStats {
        if full {
            self.encoder.redraw();
        }
        let (front, back) = (&self.front, &self.back);
        self.encoder
            .encode(front, back, cursor.as_ref(), caps, packet)
    }

    /// Makes the frame just encoded the one the terminal shows.
    pub(crate) fn commit(
        &mut self,
        cursor: Option<Cursor>,
        stats: FrameStats,
    ) -> CompletedFrame<'_> {
        std::mem::swap(&mut self.front, &mut self.back);
        self.cursor = cursor;
        CompletedFrame::new(&self.front, cursor, stats)
    }

    /// Runs `render` with a frame over the front buffer that drops every write outside `regions`
    /// and starts with the last frame's cursor, and gives the cursor it left. The back buffer keeps
    /// the regions' rows as they were, which the next full frame resets.
    pub(crate) fn render_regions<F: FnOnce(&mut Frame<'_>)>(
        &mut self,
        regions: &[Rect],
        render: F,
    ) -> Option<Cursor> {
        self.back.copy_rows_from(&self.front, regions);
        self.front.set_clip(regions);
        let mut frame = Frame::new(&mut self.front, self.cursor);
        render(&mut frame);
        let cursor = frame.into_cursor();
        self.front.clear_clip();
        cursor
    }

    /// Appends to `packet` what changed inside `regions` in the partial frame just rendered, with
    /// the cursor where `cursor` says.
    pub(crate) fn encode_regions(
        &mut self,
        regions: &[Rect],
        cursor: Option<Cursor>,
        caps: &Capabilities,
        packet: &mut Vec<u8>,
    ) -> FrameStats {
        let (before, after) = (&self.back, &self.front);
        self.encoder
            .encode_regions(before, after, regions, cursor.as_ref(), caps, packet)
    }

    /// Makes the partial frame just encoded the one the terminal shows.
    pub(crate) fn commit_regions(
        &mut self,
        cursor: Option<Cursor>,
        stats: FrameStats,
    ) -> CompletedFrame<'_> {
        self.cursor = cursor;
        CompletedFrame::new(&self.front, cursor, stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::CursorShape;
    use crate::style::Style;

    #[test]
    fn a_partial_frame_changes_its_regions_alone() {
        let caps = Capabilities::default();
        let area = Rect::new(0, 0, 6, 2);
        let mut frames = Frames::new(area, WidthProfile::default());
        assert!(!frames.resize(area));
        let cursor = frames.render(|frame| {
            frame.render_widget("abcdef", Rect::new(0, 0, 6, 1));
            frame.set_cursor_position((1, 1));
        });
        let mut packet = Vec::new();
        let stats = frames.encode(true, cursor, &caps, &mut packet);
        let done = frames.commit(cursor, stats);
        assert_eq!(done.cursor(), Some(Cursor::at(Position::new(1, 1))));
        assert_eq!(done.buffer().symbol((5, 0)), "f");

        let region = Rect::new(2, 0, 2, 1);
        let cursor = frames.render_regions(&[region], |frame| {
            assert_eq!(frame.cursor(), Some(Cursor::at(Position::new(1, 1))));
            frame.buffer_mut().set_string(0, 0, "xxxxxx", Style::new());
        });
        let mut packet = Vec::new();
        let stats = frames.encode_regions(&[region], cursor, &caps, &mut packet);
        assert_eq!(stats.cells, 2);
        let done = frames.commit_regions(cursor, stats);
        let row: String = (0..6).map(|x| done.buffer().symbol((x, 0))).collect();
        assert_eq!(row, "abxxef");

        assert!(frames.resize(Rect::new(0, 0, 3, 3)));
        assert_eq!(frames.area(), Rect::new(0, 0, 3, 3));
    }

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
        frame.buffer_mut().set_string(0, 0, "hi", Style::new());
        frame.render_widget("yo", Rect::new(5, 2, 3, 1));
        assert_eq!(frame.into_cursor(), Some(bar));
        assert_eq!(buffer.symbol((1, 0)), "i");
        assert_eq!(buffer.symbol((6, 2)), "o");
    }
}
