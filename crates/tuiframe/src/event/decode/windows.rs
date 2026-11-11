//! Windows console records to events. Key records keep what a VT encoding loses (Shift+Enter,
//! releases, modifier keys alone), so they map more directly than bytes do.

use super::{Decoded, keys};
use crate::event::records::{
    InputRecord, KeyRecord, MouseRecord,
    control::{
        CAPSLOCK_ON, ENHANCED_KEY, LEFT_ALT_PRESSED, LEFT_CTRL_PRESSED, RIGHT_ALT_PRESSED,
        RIGHT_CTRL_PRESSED, SHIFT_PRESSED,
    },
    mouse::{
        DOUBLE_CLICK, FROM_LEFT_1ST_BUTTON_PRESSED, FROM_LEFT_2ND_BUTTON_PRESSED, MOUSE_HWHEELED,
        MOUSE_MOVED, MOUSE_WHEELED, RIGHTMOST_BUTTON_PRESSED,
    },
};
use crate::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MediaKey, ModifierKey, MouseButton,
    MouseEvent, MouseEventKind,
};

/// What decoding needs remembered between records.
#[derive(Debug, Default)]
pub(super) struct WindowsState {
    /// The modifier and lock keys alone and the keypad's middle key are reported.
    all_keys: bool,
    /// The mouse buttons held at the last press or release, to tell which changed.
    buttons: u32,
    /// The first half of a surrogate pair, from a key-down record and from a key-up record.
    high_down: Option<u16>,
    high_up: Option<u16>,
}

/// The mouse buttons the decoder reports, in the order it reports changes.
const BUTTONS: [(u32, MouseButton); 3] = [
    (FROM_LEFT_1ST_BUTTON_PRESSED, MouseButton::Left),
    (RIGHTMOST_BUTTON_PRESSED, MouseButton::Right),
    (FROM_LEFT_2ND_BUTTON_PRESSED, MouseButton::Middle),
];

/// Virtual-key codes, from `winuser.h`.
mod vk {
    pub(super) const BACK: u16 = 0x08;
    pub(super) const TAB: u16 = 0x09;
    pub(super) const CLEAR: u16 = 0x0c;
    pub(super) const RETURN: u16 = 0x0d;
    pub(super) const SHIFT: u16 = 0x10;
    pub(super) const CONTROL: u16 = 0x11;
    pub(super) const MENU: u16 = 0x12;
    pub(super) const PAUSE: u16 = 0x13;
    pub(super) const CAPITAL: u16 = 0x14;
    pub(super) const ESCAPE: u16 = 0x1b;
    pub(super) const SPACE: u16 = 0x20;
    pub(super) const PRIOR: u16 = 0x21;
    pub(super) const NEXT: u16 = 0x22;
    pub(super) const END: u16 = 0x23;
    pub(super) const HOME: u16 = 0x24;
    pub(super) const LEFT: u16 = 0x25;
    pub(super) const UP: u16 = 0x26;
    pub(super) const RIGHT: u16 = 0x27;
    pub(super) const DOWN: u16 = 0x28;
    pub(super) const SNAPSHOT: u16 = 0x2c;
    pub(super) const INSERT: u16 = 0x2d;
    pub(super) const DELETE: u16 = 0x2e;
    pub(super) const LWIN: u16 = 0x5b;
    pub(super) const RWIN: u16 = 0x5c;
    pub(super) const APPS: u16 = 0x5d;
    pub(super) const NUMPAD0: u16 = 0x60;
    pub(super) const NUMPAD9: u16 = 0x69;
    pub(super) const MULTIPLY: u16 = 0x6a;
    pub(super) const ADD: u16 = 0x6b;
    pub(super) const SEPARATOR: u16 = 0x6c;
    pub(super) const SUBTRACT: u16 = 0x6d;
    pub(super) const DECIMAL: u16 = 0x6e;
    pub(super) const DIVIDE: u16 = 0x6f;
    pub(super) const F1: u16 = 0x70;
    pub(super) const F24: u16 = 0x87;
    pub(super) const NUMLOCK: u16 = 0x90;
    pub(super) const SCROLL: u16 = 0x91;
    pub(super) const LSHIFT: u16 = 0xa0;
    pub(super) const RSHIFT: u16 = 0xa1;
    pub(super) const LCONTROL: u16 = 0xa2;
    pub(super) const RCONTROL: u16 = 0xa3;
    pub(super) const LMENU: u16 = 0xa4;
    pub(super) const RMENU: u16 = 0xa5;
    pub(super) const VOLUME_MUTE: u16 = 0xad;
    pub(super) const VOLUME_DOWN: u16 = 0xae;
    pub(super) const VOLUME_UP: u16 = 0xaf;
    pub(super) const MEDIA_NEXT_TRACK: u16 = 0xb0;
    pub(super) const MEDIA_PREV_TRACK: u16 = 0xb1;
    pub(super) const MEDIA_STOP: u16 = 0xb2;
    pub(super) const MEDIA_PLAY_PAUSE: u16 = 0xb3;
    pub(super) const OEM_1: u16 = 0xba;
    pub(super) const OEM_PLUS: u16 = 0xbb;
    pub(super) const OEM_COMMA: u16 = 0xbc;
    pub(super) const OEM_MINUS: u16 = 0xbd;
    pub(super) const OEM_PERIOD: u16 = 0xbe;
    pub(super) const OEM_2: u16 = 0xbf;
    pub(super) const OEM_3: u16 = 0xc0;
    pub(super) const OEM_4: u16 = 0xdb;
    pub(super) const OEM_5: u16 = 0xdc;
    pub(super) const OEM_6: u16 = 0xdd;
    pub(super) const OEM_7: u16 = 0xde;
    pub(super) const OEM_102: u16 = 0xe2;
    pub(super) const PLAY: u16 = 0xfa;
}

/// The scan code of the right Shift, which shares `VK_SHIFT` with the left one.
const RIGHT_SHIFT_SCAN: u16 = 0x36;

impl WindowsState {
    pub(super) fn new(all_keys: bool) -> Self {
        Self {
            all_keys,
            ..Self::default()
        }
    }

    pub(super) fn decode(&mut self, record: &InputRecord, erase: u8, out: &mut Vec<Decoded>) {
        match record {
            InputRecord::Key(key) => self.key(key, erase, out),
            InputRecord::Mouse(mouse) => self.mouse(mouse, out),
            InputRecord::WindowSize(size) => out.push(Decoded::Event(Event::Resize(size.size))),
            InputRecord::Focus(focus) => out.push(Decoded::Event(if focus.focused {
                Event::FocusGained
            } else {
                Event::FocusLost
            })),
        }
    }

    fn key(&mut self, record: &KeyRecord, erase: u8, out: &mut Vec<Decoded>) {
        let mut modifiers = modifiers(record.control_state);
        let kind = if record.key_down {
            KeyEventKind::Press
        } else {
            KeyEventKind::Release
        };
        let vk = record.virtual_key;
        // An Alt code (Alt held, digits typed on the keypad) arrives with the Alt key's release.
        if vk == vk::MENU && !record.key_down && record.unicode != 0 {
            if let Some(ch) = self.unit(record.unicode, true) {
                let event = KeyEvent::new(KeyCode::Char(ch), modifiers - KeyModifiers::ALT);
                out.push(Decoded::Event(Event::Key(event)));
            }
            return;
        }
        // The keypad's digits typed with Alt alone make that Alt code, so they aren't keys.
        if (vk::NUMPAD0..=vk::NUMPAD9).contains(&vk)
            && modifiers.contains(KeyModifiers::ALT)
            && !modifiers.intersects(KeyModifiers::SHIFT | KeyModifiers::CONTROL)
        {
            return;
        }
        let code = match named(record) {
            Some(KeyCode::Modifier(key)) => {
                modifiers |= modifier_bit(key);
                KeyCode::Modifier(key)
            }
            Some(KeyCode::Tab) if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
            Some(code) => code,
            None => {
                let Some(event) = self.character(record, modifiers, erase) else {
                    return;
                };
                let (code, char_modifiers) = event;
                modifiers = char_modifiers;
                code
            }
        };
        let alone = matches!(
            code,
            KeyCode::Modifier(_)
                | KeyCode::CapsLock
                | KeyCode::NumLock
                | KeyCode::ScrollLock
                | KeyCode::KeypadBegin
        );
        if alone && !self.all_keys {
            return;
        }
        let count = if record.key_down {
            record.repeat.max(1)
        } else {
            1
        };
        for _ in 0..count {
            let event = KeyEvent::new_with_kind(code, modifiers, kind);
            out.push(Decoded::Event(Event::Key(event)));
        }
    }

    /// The character a key types and the modifiers to report with it, or `None` for a dead key, the
    /// first half of a surrogate pair, or a key that types nothing.
    fn character(
        &mut self,
        record: &KeyRecord,
        modifiers: KeyModifiers,
        erase: u8,
    ) -> Option<(KeyCode, KeyModifiers)> {
        let unit = record.unicode;
        let ctrl_alt = KeyModifiers::CONTROL | KeyModifiers::ALT;
        if unit >= 0x20 && unit != 0x7f {
            let ch = self.unit(unit, record.key_down)?;
            // Alt Gr arrives as Ctrl+Alt, and the character it typed is the key.
            let modifiers = if modifiers.contains(ctrl_alt) {
                modifiers - ctrl_alt
            } else {
                modifiers
            };
            return Some((KeyCode::Char(ch), modifiers));
        }
        // Ctrl turns the character into a control code or nothing, so the key says which it was.
        if modifiers.contains(KeyModifiers::CONTROL)
            && let Some(ch) = key_character(record)
        {
            return Some((KeyCode::Char(ch), modifiers));
        }
        match u8::try_from(unit) {
            // No character and no Ctrl is a dead key, which the next key's character completes.
            Ok(0) | Err(_) => None,
            Ok(byte) => {
                let key = keys::control(byte, erase);
                Some((key.code, key.modifiers | modifiers))
            }
        }
    }

    /// A UTF-16 unit as a character, joining a surrogate pair across two records.
    fn unit(&mut self, unit: u16, down: bool) -> Option<char> {
        let high = if down {
            &mut self.high_down
        } else {
            &mut self.high_up
        };
        match unit {
            0xd800..=0xdbff => {
                *high = Some(unit);
                None
            }
            0xdc00..=0xdfff => {
                let first = high.take()?;
                char::decode_utf16([first, unit]).next()?.ok()
            }
            _ => char::from_u32(u32::from(unit)),
        }
    }

    fn mouse(&mut self, record: &MouseRecord, out: &mut Vec<Decoded>) {
        let modifiers = modifiers(record.control_state);
        let at = |kind| {
            Decoded::Event(Event::Mouse(MouseEvent {
                kind,
                column: record.column,
                row: record.row,
                modifiers,
            }))
        };
        let held = record.button_state;
        // The high word is the wheel's turn, signed.
        let turn = (held >> 16) & 0xffff;
        let forward = turn != 0 && turn < 0x8000;
        match record.event_flags {
            0 | DOUBLE_CLICK => {
                for (bit, button) in BUTTONS {
                    let before = self.buttons & bit != 0;
                    let now = held & bit != 0;
                    if now && !before {
                        out.push(at(MouseEventKind::Down(button)));
                    } else if before && !now {
                        out.push(at(MouseEventKind::Up(button)));
                    }
                }
                self.buttons = held;
            }
            MOUSE_MOVED => {
                let kind = BUTTONS
                    .iter()
                    .find(|(bit, _)| held & bit != 0)
                    .map_or(MouseEventKind::Moved, |&(_, button)| {
                        MouseEventKind::Drag(button)
                    });
                out.push(at(kind));
            }
            MOUSE_WHEELED if turn != 0 => out.push(at(if forward {
                MouseEventKind::ScrollUp
            } else {
                MouseEventKind::ScrollDown
            })),
            MOUSE_HWHEELED if turn != 0 => out.push(at(if forward {
                MouseEventKind::ScrollRight
            } else {
                MouseEventKind::ScrollLeft
            })),
            _ => {}
        }
    }
}

/// The modifiers in a record's control key state.
fn modifiers(state: u32) -> KeyModifiers {
    let mut modifiers = KeyModifiers::NONE;
    if state & SHIFT_PRESSED != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    if state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    modifiers
}

/// The modifier a modifier key's own record sets, which on release the state no longer has.
fn modifier_bit(key: ModifierKey) -> KeyModifiers {
    match key {
        ModifierKey::LeftShift | ModifierKey::RightShift => KeyModifiers::SHIFT,
        ModifierKey::LeftControl | ModifierKey::RightControl => KeyModifiers::CONTROL,
        ModifierKey::LeftAlt | ModifierKey::RightAlt => KeyModifiers::ALT,
        ModifierKey::LeftSuper | ModifierKey::RightSuper => KeyModifiers::SUPER,
        _ => KeyModifiers::NONE,
    }
}

/// The keys that type no character.
fn named(record: &KeyRecord) -> Option<KeyCode> {
    let right = record.control_state & ENHANCED_KEY != 0;
    let side = |left, right_key| KeyCode::Modifier(if right { right_key } else { left });
    Some(match record.virtual_key {
        vk::BACK => KeyCode::Backspace,
        vk::TAB => KeyCode::Tab,
        vk::CLEAR => KeyCode::KeypadBegin,
        vk::RETURN => KeyCode::Enter,
        vk::SHIFT if record.scan_code == RIGHT_SHIFT_SCAN => {
            KeyCode::Modifier(ModifierKey::RightShift)
        }
        vk::SHIFT => KeyCode::Modifier(ModifierKey::LeftShift),
        vk::CONTROL => side(ModifierKey::LeftControl, ModifierKey::RightControl),
        vk::MENU => side(ModifierKey::LeftAlt, ModifierKey::RightAlt),
        vk::PAUSE => KeyCode::Pause,
        vk::CAPITAL => KeyCode::CapsLock,
        vk::ESCAPE => KeyCode::Esc,
        vk::PRIOR => KeyCode::PageUp,
        vk::NEXT => KeyCode::PageDown,
        vk::END => KeyCode::End,
        vk::HOME => KeyCode::Home,
        vk::LEFT => KeyCode::Left,
        vk::UP => KeyCode::Up,
        vk::RIGHT => KeyCode::Right,
        vk::DOWN => KeyCode::Down,
        vk::SNAPSHOT => KeyCode::PrintScreen,
        vk::INSERT => KeyCode::Insert,
        vk::DELETE => KeyCode::Delete,
        vk::LWIN => KeyCode::Modifier(ModifierKey::LeftSuper),
        vk::RWIN => KeyCode::Modifier(ModifierKey::RightSuper),
        vk::APPS => KeyCode::Menu,
        n @ vk::F1..=vk::F24 => KeyCode::F(u8::try_from(n - vk::F1 + 1).unwrap_or(u8::MAX)),
        vk::NUMLOCK => KeyCode::NumLock,
        vk::SCROLL => KeyCode::ScrollLock,
        vk::LSHIFT => KeyCode::Modifier(ModifierKey::LeftShift),
        vk::RSHIFT => KeyCode::Modifier(ModifierKey::RightShift),
        vk::LCONTROL => KeyCode::Modifier(ModifierKey::LeftControl),
        vk::RCONTROL => KeyCode::Modifier(ModifierKey::RightControl),
        vk::LMENU => KeyCode::Modifier(ModifierKey::LeftAlt),
        vk::RMENU => KeyCode::Modifier(ModifierKey::RightAlt),
        vk::VOLUME_MUTE => KeyCode::Media(MediaKey::MuteVolume),
        vk::VOLUME_DOWN => KeyCode::Media(MediaKey::LowerVolume),
        vk::VOLUME_UP => KeyCode::Media(MediaKey::RaiseVolume),
        vk::MEDIA_NEXT_TRACK => KeyCode::Media(MediaKey::TrackNext),
        vk::MEDIA_PREV_TRACK => KeyCode::Media(MediaKey::TrackPrevious),
        vk::MEDIA_STOP => KeyCode::Media(MediaKey::Stop),
        vk::MEDIA_PLAY_PAUSE => KeyCode::Media(MediaKey::PlayPause),
        vk::PLAY => KeyCode::Media(MediaKey::Play),
        _ => return None,
    })
}

/// The character a key types with no modifier on a US layout, for keys Ctrl was held with. Letters
/// are capitals when Shift or Caps Lock, but not both, is on.
fn key_character(record: &KeyRecord) -> Option<char> {
    let vk = record.virtual_key;
    let ch = match vk {
        0x30..=0x39 | 0x41..=0x5a => char::from(u8::try_from(vk).ok()?),
        vk::SPACE => ' ',
        vk::NUMPAD0..=vk::NUMPAD9 => char::from(u8::try_from(vk - vk::NUMPAD0).ok()? + b'0'),
        vk::MULTIPLY => '*',
        vk::ADD => '+',
        vk::SEPARATOR => ',',
        vk::SUBTRACT => '-',
        vk::DECIMAL => '.',
        vk::DIVIDE => '/',
        vk::OEM_1 => ';',
        vk::OEM_PLUS => '=',
        vk::OEM_COMMA => ',',
        vk::OEM_MINUS => '-',
        vk::OEM_PERIOD => '.',
        vk::OEM_2 => '/',
        vk::OEM_3 => '`',
        vk::OEM_4 => '[',
        vk::OEM_5 | vk::OEM_102 => '\\',
        vk::OEM_6 => ']',
        vk::OEM_7 => '\'',
        _ => return None,
    };
    if ch.is_ascii_uppercase() {
        let shift = record.control_state & SHIFT_PRESSED != 0;
        let caps = record.control_state & CAPSLOCK_ON != 0;
        if shift == caps {
            return Some(ch.to_ascii_lowercase());
        }
    }
    Some(ch)
}
