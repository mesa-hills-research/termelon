//! What a benchmark needs from a terminal stack: build its buffer from a neutral frame, then diff it
//! against the previous frame and encode the changes.

use crate::model::{Cursor, Frame};
use crate::ratatui_stack::RatatuiStack;
use crate::termelon_stack::TermelonStack;

/// A terminal stack under test. The output goes to memory.
pub(crate) trait Stack {
    /// Makes the screen `width` by `height` and clears it, so the next frame is a full repaint.
    /// Whatever the stack writes for that goes in the next frame's output.
    fn resize(&mut self, width: u16, height: u16);

    /// Builds the stack's own buffer from `frame`.
    fn render(&mut self, frame: &Frame);

    /// Diffs the rendered frame against the one before and encodes the changes, inside a
    /// synchronized update, with the cursor at `cursor`. Returns the bytes written since the last
    /// frame, which stay valid until the next call.
    fn present(&mut self, cursor: Cursor) -> &[u8];
}

/// The names of the stacks.
pub(crate) const NAMES: [&str; 3] = ["ratatui", "termelon", "termelon-scroll"];

/// A stack by name for a screen of `width` by `height`.
pub(crate) fn make(name: &str, width: u16, height: u16) -> Result<Box<dyn Stack>, String> {
    match name {
        "ratatui" => RatatuiStack::new(width, height)
            .map(|stack| Box::new(stack) as Box<dyn Stack>)
            .map_err(|error| format!("couldn't set up {name}: {error}")),
        "termelon" => Ok(Box::new(TermelonStack::new(width, height, false))),
        "termelon-scroll" => Ok(Box::new(TermelonStack::new(width, height, true))),
        _ => Err(format!("unknown stack `{name}`")),
    }
}

/// What a stack is built from, for the report's heading.
pub(crate) fn describe(name: &str) -> &'static str {
    match name {
        "ratatui" => "ratatui 0.30 + crossterm 0.29",
        "termelon" => "termelon's buffer and encoder",
        "termelon-scroll" => "termelon with scroll detection",
        _ => "unknown stack",
    }
}
