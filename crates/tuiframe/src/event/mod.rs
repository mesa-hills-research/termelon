//! What the terminal reports, as [`Event`]s.
//!
//! The types and their names follow crossterm's, so code that matches on crossterm's events ports
//! by changing its imports.
//!
//! ```
//! use tuiframe::event::{Event, KeyCode, KeyEvent, KeyModifiers};
//!
//! let event = Event::Key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
//! let quit = matches!(
//!     event,
//!     Event::Key(key) if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
//! );
//! assert!(quit);
//! ```

use crate::flags::flags;
use crate::layout::Size;

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the reader thread, which owns the decoder, isn't built yet"
    )
)]
pub(crate) mod decode;
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the Windows reader, which fills these, isn't built yet"
    )
)]
pub(crate) mod records;

/// Something the terminal reported.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Event {
    /// A key was pressed, repeated or released.
    Key(KeyEvent),
    /// The mouse was clicked, dragged or scrolled.
    Mouse(MouseEvent),
    /// Text was pasted, with its line breaks as `\n`.
    Paste(String),
    /// The terminal gained the keyboard focus.
    FocusGained,
    /// The terminal lost the keyboard focus.
    FocusLost,
    /// The terminal changed size, and the next frame redraws everything.
    Resize(Size),
    /// The program runs again after a suspend, and the next frame redraws everything.
    Resumed,
    /// The terminal switched between its dark and light themes. Only terminals asked to report
    /// it do.
    ColorScheme(Scheme),
}

/// A terminal's theme, from [`Event::ColorScheme`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scheme {
    /// Light text on a dark background.
    Dark,
    /// Dark text on a light background.
    Light,
}

/// A key and the modifiers held with it.
///
/// Where the terminal can tell them apart, a typed capital is `Char('A')` with `SHIFT`, Shift+Tab
/// is [`KeyCode::BackTab`], and Ctrl+J (a line feed) differs from Enter. Two key events are equal
/// when their key, modifiers and kind are, whatever the terminal sent them as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct KeyEvent {
    /// The key.
    pub code: KeyCode,
    /// The modifiers held.
    pub modifiers: KeyModifiers,
    /// Whether it was pressed, repeated or released. Only some terminals report repeats and
    /// releases.
    pub kind: KeyEventKind,
}

impl KeyEvent {
    /// A press of `code` with `modifiers`.
    #[must_use]
    pub const fn new(code: KeyCode, modifiers: KeyModifiers) -> Self {
        Self::new_with_kind(code, modifiers, KeyEventKind::Press)
    }

    /// A press, repeat or release of `code` with `modifiers`.
    #[must_use]
    pub const fn new_with_kind(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> Self {
        Self {
            code,
            modifiers,
            kind,
        }
    }
}

/// A key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyCode {
    /// A key that types a character, as it types it with Shift held (`'A'` with Shift). With Ctrl
    /// or Alt held as well, the kitty keyboard protocol and modifyOtherKeys give a letter
    /// lowercase, as crossterm does, so a `Char('c')` arm with Ctrl takes Ctrl+Shift+C there as it
    /// does where the terminal sends 0x03 for both.
    Char(char),
    /// Enter, or Return.
    Enter,
    /// Tab.
    Tab,
    /// Shift+Tab.
    BackTab,
    /// Backspace.
    Backspace,
    /// Escape.
    Esc,
    /// The left arrow.
    Left,
    /// The right arrow.
    Right,
    /// The up arrow.
    Up,
    /// The down arrow.
    Down,
    /// Home.
    Home,
    /// End.
    End,
    /// Page Up.
    PageUp,
    /// Page Down.
    PageDown,
    /// Insert.
    Insert,
    /// Delete, the key that deletes forward.
    Delete,
    /// A function key, from `F(1)`.
    F(u8),
    /// The menu key.
    Menu,
    /// The keypad's middle key (5) with Num Lock off.
    KeypadBegin,
    /// Caps Lock.
    CapsLock,
    /// Scroll Lock.
    ScrollLock,
    /// Num Lock.
    NumLock,
    /// Print Screen.
    PrintScreen,
    /// Pause.
    Pause,
    /// A media key.
    Media(MediaKey),
    /// A modifier key pressed on its own, which only the kitty keyboard protocol and Windows
    /// report.
    Modifier(ModifierKey),
}

/// A media key, as the kitty keyboard protocol lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MediaKey {
    /// Play.
    Play,
    /// Pause.
    Pause,
    /// Play or pause.
    PlayPause,
    /// Reverse.
    Reverse,
    /// Stop.
    Stop,
    /// Fast forward.
    FastForward,
    /// Rewind.
    Rewind,
    /// The next track.
    TrackNext,
    /// The previous track.
    TrackPrevious,
    /// Record.
    Record,
    /// Volume down.
    LowerVolume,
    /// Volume up.
    RaiseVolume,
    /// Mute.
    MuteVolume,
}

/// A modifier key, as the kitty keyboard protocol lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ModifierKey {
    /// The left Shift.
    LeftShift,
    /// The left Control.
    LeftControl,
    /// The left Alt, or Option on a Mac.
    LeftAlt,
    /// The left Super, the Windows key or Command on a Mac.
    LeftSuper,
    /// The left Hyper.
    LeftHyper,
    /// The left Meta.
    LeftMeta,
    /// The right Shift.
    RightShift,
    /// The right Control.
    RightControl,
    /// The right Alt, or Option on a Mac.
    RightAlt,
    /// The right Super, the Windows key or Command on a Mac.
    RightSuper,
    /// The right Hyper.
    RightHyper,
    /// The right Meta.
    RightMeta,
    /// ISO Level 3 Shift, which is Alt Gr on many layouts.
    IsoLevel3Shift,
    /// ISO Level 5 Shift.
    IsoLevel5Shift,
}

flags! {
    /// The modifiers held with a key or a mouse event, combined with `|`.
    ///
    /// ```
    /// use tuiframe::event::KeyModifiers;
    ///
    /// let held = KeyModifiers::CONTROL | KeyModifiers::SHIFT;
    /// assert!(held.contains(KeyModifiers::CONTROL));
    /// assert!(!held.intersects(KeyModifiers::ALT));
    /// ```
    pub struct KeyModifiers(u8);
    empty = "NONE";
    /// Shift.
    const SHIFT = 1;
    /// Control.
    const CONTROL = 1 << 1;
    /// Alt, or Option on a Mac.
    const ALT = 1 << 2;
    /// Super, the Windows key or Command on a Mac.
    const SUPER = 1 << 3;
    /// Hyper.
    const HYPER = 1 << 4;
    /// Meta.
    const META = 1 << 5;
}

impl KeyModifiers {
    /// No modifiers, the same as [`KeyModifiers::empty`].
    pub const NONE: Self = Self::empty();
}

/// Whether a key went down, repeated or came up.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyEventKind {
    /// It went down.
    #[default]
    Press,
    /// It's held and repeating.
    Repeat,
    /// It came up.
    Release,
}

/// A mouse button press, release, drag or scroll, at a cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct MouseEvent {
    /// What the mouse did.
    pub kind: MouseEventKind,
    /// The cell's column, from 0 at the left.
    pub column: u16,
    /// The cell's row, from 0 at the top.
    pub row: u16,
    /// The modifiers held.
    pub modifiers: KeyModifiers,
}

impl MouseEvent {
    /// The mouse doing `kind` at `column` and `row`, with `modifiers` held.
    #[must_use]
    pub const fn new(kind: MouseEventKind, column: u16, row: u16, modifiers: KeyModifiers) -> Self {
        Self {
            kind,
            column,
            row,
            modifiers,
        }
    }
}

/// What the mouse did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MouseEventKind {
    /// A button went down.
    Down(MouseButton),
    /// A button came up.
    Up(MouseButton),
    /// The mouse moved with a button held.
    Drag(MouseButton),
    /// The mouse moved with no button held, which terminals report only in any-motion mode.
    Moved,
    /// The wheel turned up, away from the user.
    ScrollUp,
    /// The wheel turned down, toward the user.
    ScrollDown,
    /// The wheel tilted left.
    ScrollLeft,
    /// The wheel tilted right.
    ScrollRight,
}

/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MouseButton {
    /// The left button.
    Left,
    /// The right button.
    Right,
    /// The middle button, often the wheel.
    Middle,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_gives_a_press() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(key.kind, KeyEventKind::Press);
        let copy = key;
        assert_eq!(copy, key);
        assert_eq!(
            KeyEvent::new_with_kind(KeyCode::Enter, KeyModifiers::SHIFT, KeyEventKind::Release),
            KeyEvent {
                kind: KeyEventKind::Release,
                ..key
            }
        );
        let click = MouseEvent::new(
            MouseEventKind::Down(MouseButton::Left),
            3,
            4,
            KeyModifiers::CONTROL,
        );
        assert_eq!((click.column, click.row), (3, 4));
        assert_eq!(click.modifiers, KeyModifiers::CONTROL);
    }

    #[test]
    fn key_modifiers_have_the_bits_crossterm_uses() {
        let bits = [
            KeyModifiers::NONE,
            KeyModifiers::SHIFT,
            KeyModifiers::CONTROL,
            KeyModifiers::ALT,
            KeyModifiers::SUPER,
            KeyModifiers::HYPER,
            KeyModifiers::META,
        ]
        .map(KeyModifiers::bits);
        assert_eq!(bits, [0, 1, 2, 4, 8, 16, 32]);
        assert_eq!(KeyModifiers::all().bits(), 0b11_1111);
        assert_eq!(
            !KeyModifiers::SHIFT,
            KeyModifiers::all() - KeyModifiers::SHIFT
        );
    }

    #[test]
    fn key_modifiers_set_operations() {
        let mut m = KeyModifiers::NONE;
        assert!(m.is_empty());
        assert!(m.contains(KeyModifiers::NONE));
        m.insert(KeyModifiers::ALT);
        m |= KeyModifiers::CONTROL;
        assert!(m.contains(KeyModifiers::ALT | KeyModifiers::CONTROL));
        assert!(!m.contains(KeyModifiers::ALT | KeyModifiers::SHIFT));
        assert!(m.intersects(KeyModifiers::ALT | KeyModifiers::SHIFT));
        m.remove(KeyModifiers::ALT);
        assert_eq!(m, KeyModifiers::CONTROL);
        assert_eq!(m & KeyModifiers::SHIFT, KeyModifiers::NONE);
    }

    #[test]
    fn key_modifiers_debug_names_its_flags() {
        assert_eq!(format!("{:?}", KeyModifiers::NONE), "KeyModifiers(NONE)");
        assert_eq!(
            format!("{:?}", KeyModifiers::META | KeyModifiers::SHIFT),
            "KeyModifiers(SHIFT | META)"
        );
    }
}
