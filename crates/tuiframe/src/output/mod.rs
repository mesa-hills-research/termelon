//! What reaches the terminal. The diff finds the cells a frame changes, and the encoder turns them
//! into one packet of bytes.

pub(crate) mod color;
pub(crate) mod diff;
pub(crate) mod encode;
#[cfg(test)]
mod tests;

use crate::buffer::Buffer;
use crate::layout::Position;

/// The cursor's shape, which terminals draw as a block, an underline or a bar.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorShape {
    /// The shape the user set in their terminal.
    #[default]
    Default,
    /// A block over the cell.
    Block,
    /// A line under the cell.
    Underline,
    /// A bar before the cell.
    Bar,
}

/// Where a frame puts the terminal's cursor, and how it looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Cursor {
    /// The cell it's on.
    pub position: Position,
    /// Its shape.
    pub shape: CursorShape,
    /// Whether it blinks. The terminal's default shape keeps the user's setting.
    pub blinking: bool,
}

impl Cursor {
    /// The cursor at `position`, in the terminal's default shape.
    pub const fn at(position: Position) -> Self {
        Self {
            position,
            shape: CursorShape::Default,
            blinking: false,
        }
    }
}

/// What writing a frame took.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct FrameStats {
    /// The cells written, erased cells included.
    pub cells: usize,
    /// The bytes of the frame's packet.
    pub bytes: usize,
}

/// A frame the terminal took, which lends the buffer it drew and gives the cursor and the stats.
#[derive(Debug, Clone, Copy)]
pub struct CompletedFrame<'a> {
    buffer: &'a Buffer,
    cursor: Option<Cursor>,
    stats: FrameStats,
}

impl<'a> CompletedFrame<'a> {
    pub(crate) const fn new(buffer: &'a Buffer, cursor: Option<Cursor>, stats: FrameStats) -> Self {
        Self {
            buffer,
            cursor,
            stats,
        }
    }

    /// The cells as the frame drew them.
    pub const fn buffer(&self) -> &'a Buffer {
        self.buffer
    }

    /// Where the frame put the cursor, or `None` when it hid it.
    pub const fn cursor(&self) -> Option<Cursor> {
        self.cursor
    }

    /// What writing it took.
    pub const fn stats(&self) -> FrameStats {
        self.stats
    }
}
