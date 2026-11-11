//! The encoder on its own, behind the `unstable-encoder` feature, so the bench can measure it
//! against other stacks. It may change in any release.

use crate::buffer::Buffer;
use crate::caps::Capabilities;
use crate::output::encode;
use crate::output::{Cursor, FrameStats};

/// Turns frames into the bytes that take a terminal from one to the next, as `Terminal::draw` does.
#[derive(Debug)]
pub struct Encoder(encode::Encoder);

impl Encoder {
    /// An encoder for a terminal whose screen is blank.
    pub fn new() -> Self {
        Self(encode::Encoder::new())
    }

    /// Appends to `out` the packet that takes the terminal from `prev` to `next`, with the cursor
    /// where `cursor` says, or hidden.
    pub fn encode(
        &mut self,
        prev: &Buffer,
        next: &Buffer,
        cursor: Option<&Cursor>,
        caps: &Capabilities,
        out: &mut Vec<u8>,
    ) -> FrameStats {
        self.0.encode(prev, next, cursor, caps, out)
    }

    /// Makes the next frame a full redraw.
    pub fn redraw(&mut self) {
        self.0.redraw();
    }

    /// Turns scroll detection on or off.
    pub fn set_scroll_detection(&mut self, on: bool) {
        self.0.set_scroll_detection(on);
    }
}

impl Default for Encoder {
    fn default() -> Self {
        Self::new()
    }
}
