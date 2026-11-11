//! Keys and mouse events, for the encoders that send them as a terminal would.

use std::ops::BitOr;

/// A key on the keyboard.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Key {
    /// A key that types a character, given unshifted (`'a'`, not `'A'`). Shift makes letters
    /// uppercase.
    Char(char),
    /// Enter.
    Enter,
    /// Tab.
    Tab,
    /// Backspace.
    Backspace,
    /// Escape.
    Escape,
    /// Up arrow.
    Up,
    /// Down arrow.
    Down,
    /// Left arrow.
    Left,
    /// Right arrow.
    Right,
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
    /// Delete.
    Delete,
    /// A function key, F1 to F35. Without the kitty keyboard protocol, only F1 to F12 send anything.
    F(u8),
}

/// The modifier keys held, combined with `|`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Mods(u8);

impl Mods {
    /// No modifier.
    pub const NONE: Mods = Mods(0);
    /// Shift.
    pub const SHIFT: Mods = Mods(1);
    /// Alt, which terminals also call Meta.
    pub const ALT: Mods = Mods(2);
    /// Control.
    pub const CTRL: Mods = Mods(4);
    /// Super, the Windows or Command key.
    pub const SUPER: Mods = Mods(8);

    /// The bits as terminals number them: shift 1, alt 2, ctrl 4, super 8.
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Whether every modifier of `other` is held.
    pub const fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }

    /// Whether any modifier of `other` is held.
    pub const fn intersects(self, other: Mods) -> bool {
        self.0 & other.0 != 0
    }

    /// Whether none is held.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// These modifiers but those of `other`.
    #[must_use]
    pub const fn without(self, other: Mods) -> Mods {
        Mods(self.0 & !other.0)
    }
}

impl BitOr for Mods {
    type Output = Mods;

    fn bitor(self, other: Mods) -> Mods {
        Mods(self.0 | other.0)
    }
}

/// Whether a key went down, repeats or came up.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum KeyKind {
    /// It went down.
    #[default]
    Press,
    /// It's held and repeats.
    Repeat,
    /// It came up. Only the kitty protocol's flag 2 reports this.
    Release,
}

/// A mouse button.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// The left button.
    Left,
    /// The middle button.
    Middle,
    /// The right button.
    Right,
}

/// What the mouse did.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MouseKind {
    /// A button went down.
    Press(MouseButton),
    /// A button came up.
    Release(MouseButton),
    /// The mouse moved with a button down.
    Drag(MouseButton),
    /// The mouse moved with no button down.
    Move,
    /// The wheel turned up.
    ScrollUp,
    /// The wheel turned down.
    ScrollDown,
    /// The wheel tilted left.
    ScrollLeft,
    /// The wheel tilted right.
    ScrollRight,
}

/// A mouse event over a cell.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Mouse {
    /// What happened.
    pub kind: MouseKind,
    /// The column, from 0.
    pub x: u16,
    /// The row, from 0.
    pub y: u16,
    /// The modifiers held. Only shift, alt and ctrl are reported.
    pub mods: Mods,
}

impl Mouse {
    /// A mouse event with no modifiers.
    pub const fn new(kind: MouseKind, x: u16, y: u16) -> Mouse {
        Mouse {
            kind,
            x,
            y,
            mods: Mods::NONE,
        }
    }
}
