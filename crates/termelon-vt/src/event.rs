//! What the emulator records as it goes: events, unknown sequences, clipboard writes.

use std::fmt;

/// A cell position, column `x` and row `y`, from 0 at the top left.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pos {
    /// The column.
    pub x: u16,
    /// The row.
    pub y: u16,
}

impl Pos {
    /// A position.
    pub const fn new(x: u16, y: u16) -> Pos {
        Pos { x, y }
    }
}

/// Something the terminal did, recorded at the byte that did it.
///
/// Events say what the terminal did, so a mode the profile's terminal doesn't have gives no event.
/// Each fact is one event. Mode 25 gives [`CursorShown`](Event::CursorShown) and
/// [`CursorHidden`](Event::CursorHidden), and mode 2026 [`SyncBegin`](Event::SyncBegin) and
/// [`SyncEnd`](Event::SyncEnd), rather than [`ModeSet`](Event::ModeSet) and
/// [`ModeReset`](Event::ModeReset).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Event {
    /// A character was drawn into the cell at `at`, as a cluster `width` cells wide. `cursor` is
    /// where the cursor was when the character came, and `cursor_visible` whether it showed.
    Drawn {
        /// The cell drawn into, the first of a wide cluster.
        at: Pos,
        /// The cluster's width after this character.
        width: u8,
        /// Where the cursor was.
        cursor: Pos,
        /// Whether the cursor showed.
        cursor_visible: bool,
    },
    /// A control or a sequence moved the cursor. Drawing moves it too, which gives no event.
    CursorMoved {
        /// Where it was.
        from: Pos,
        /// Where it is.
        to: Pos,
    },
    /// The cursor was shown (`CSI ? 25 h`).
    CursorShown {
        /// Where it showed.
        at: Pos,
    },
    /// The cursor was hidden (`CSI ? 25 l`).
    CursorHidden {
        /// Where it was.
        at: Pos,
    },
    /// A synchronized update began (`CSI ? 2026 h`).
    SyncBegin,
    /// A synchronized update ended (`CSI ? 2026 l`).
    SyncEnd,
    /// A DEC private mode was set (`CSI ? n h`).
    ModeSet(u16),
    /// A DEC private mode was reset (`CSI ? n l`).
    ModeReset(u16),
    /// The window title was set (OSC 0, OSC 2, or a pop from the title stack).
    TitleSet(String),
    /// The bell rang.
    Bell,
}

/// A sequence the emulator doesn't know, with its bytes.
///
/// Invalid UTF-8, a C0 or C1 control nobody uses, and sequences past the parser's bounds count
/// too, as do the strings the Linux console can't parse.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct UnknownSequence {
    pub(crate) bytes: Vec<u8>,
}

impl UnknownSequence {
    /// The sequence's bytes, cut at 256 with `…` after them.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

impl fmt::Display for UnknownSequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("`")?;
        for chunk in self.bytes.utf8_chunks() {
            for c in chunk.valid().chars() {
                match c {
                    '\x1b' => f.write_str("\\e")?,
                    '\\' => f.write_str("\\\\")?,
                    c if c.is_control() => write!(f, "\\x{:02x}", u32::from(c))?,
                    c => write!(f, "{c}")?,
                }
            }
            for byte in chunk.invalid() {
                write!(f, "\\x{byte:02x}")?;
            }
        }
        f.write_str("`")
    }
}

impl fmt::Debug for UnknownSequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UnknownSequence({self})")
    }
}

/// The unknown sequences one [`feed`](crate::Emulator::feed) met in strict mode.
#[derive(Clone, PartialEq, Eq)]
pub struct FeedError {
    pub(crate) sequences: Vec<UnknownSequence>,
}

impl FeedError {
    /// The sequences, in the order they came.
    pub fn sequences(&self) -> &[UnknownSequence] {
        &self.sequences
    }
}

impl fmt::Display for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.sequences.len() == 1 {
            f.write_str("unknown sequence ")?;
        } else {
            f.write_str("unknown sequences ")?;
        }
        for (i, sequence) in self.sequences.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            write!(f, "{sequence}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for FeedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "FeedError({self})")
    }
}

impl std::error::Error for FeedError {}

/// A write to the clipboard with OSC 52.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardWrite {
    /// The selection parameter, such as `c` for the clipboard. Empty means the terminal's default.
    pub selection: String,
    /// The data, decoded from base64. Empty clears the selection.
    pub data: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_sequences_print_readably() {
        let sequence = UnknownSequence {
            bytes: b"\x1b[?9999h\x07\xff\\".to_vec(),
        };
        assert_eq!(sequence.to_string(), "`\\e[?9999h\\x07\\xff\\\\`");
        let error = FeedError {
            sequences: vec![sequence.clone(), sequence],
        };
        assert!(error.to_string().starts_with("unknown sequences `"));
    }
}
