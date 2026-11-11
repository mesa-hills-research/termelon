//! Windows console input records as plain structs, so the decoder and its tests build on every
//! system. The Windows reader copies each `INPUT_RECORD`'s fields into these and drops menu records.

use crate::layout::Size;

/// One console input record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputRecord {
    Key(KeyRecord),
    Mouse(MouseRecord),
    WindowSize(WindowSizeRecord),
    Focus(FocusRecord),
}

/// A `KEY_EVENT_RECORD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct KeyRecord {
    pub(crate) key_down: bool,
    pub(crate) repeat: u16,
    pub(crate) virtual_key: u16,
    pub(crate) scan_code: u16,
    /// One UTF-16 unit, so a surrogate pair takes two records.
    pub(crate) unicode: u16,
    pub(crate) control_state: u32,
}

/// A `MOUSE_EVENT_RECORD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MouseRecord {
    /// Relative to the window, which the reader works out.
    pub(crate) column: u16,
    pub(crate) row: u16,
    pub(crate) button_state: u32,
    pub(crate) control_state: u32,
    pub(crate) event_flags: u32,
}

/// A `WINDOW_BUFFER_SIZE_RECORD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WindowSizeRecord {
    /// The window's size, which the reader reads, not the record's buffer size.
    pub(crate) size: Size,
}

/// A `FOCUS_EVENT_RECORD`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FocusRecord {
    pub(crate) focused: bool,
}

/// The bits of `control_state`, from `wincon.h`.
pub(crate) mod control {
    pub(crate) const RIGHT_ALT_PRESSED: u32 = 0x0001;
    pub(crate) const LEFT_ALT_PRESSED: u32 = 0x0002;
    pub(crate) const RIGHT_CTRL_PRESSED: u32 = 0x0004;
    pub(crate) const LEFT_CTRL_PRESSED: u32 = 0x0008;
    pub(crate) const SHIFT_PRESSED: u32 = 0x0010;
    pub(crate) const CAPSLOCK_ON: u32 = 0x0080;
    pub(crate) const ENHANCED_KEY: u32 = 0x0100;
}

/// The bits of `button_state` and the values of `event_flags`, from `wincon.h`.
pub(crate) mod mouse {
    pub(crate) const FROM_LEFT_1ST_BUTTON_PRESSED: u32 = 0x0001;
    pub(crate) const RIGHTMOST_BUTTON_PRESSED: u32 = 0x0002;
    pub(crate) const FROM_LEFT_2ND_BUTTON_PRESSED: u32 = 0x0004;
    pub(crate) const MOUSE_MOVED: u32 = 0x0001;
    pub(crate) const DOUBLE_CLICK: u32 = 0x0002;
    pub(crate) const MOUSE_WHEELED: u32 = 0x0004;
    pub(crate) const MOUSE_HWHEELED: u32 = 0x0008;
}
