//! Keys from C0 controls, characters, xterm's CSI and SS3 forms, the `~` codes, modifyOtherKeys and
//! the kitty keyboard protocol.

use super::tokenizer::Csi;
use crate::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MediaKey, ModifierKey};

/// A typed character, with Shift when it's a capital, as crossterm gives it.
pub(super) fn char_key(ch: char) -> KeyEvent {
    let modifiers = if ch.is_uppercase() {
        KeyModifiers::SHIFT
    } else {
        KeyModifiers::NONE
    };
    KeyEvent::new(KeyCode::Char(ch), modifiers)
}

/// A C0 control or DEL, as crossterm decodes them. LF is Ctrl+J, as in raw mode it can only be
/// that. DEL is Backspace, and so is `0x08` where `erase` says Backspace sends it, and Ctrl+H
/// otherwise.
pub(super) fn control(byte: u8, erase: u8) -> KeyEvent {
    let ctrl = |code| KeyEvent::new(code, KeyModifiers::CONTROL);
    match byte {
        0x00 => ctrl(KeyCode::Char(' ')),
        0x09 => KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        0x0d => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        0x7f => KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        0x08 if erase == 0x08 => KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
        0x01..=0x1a => ctrl(KeyCode::Char(char::from(b'a' + byte - 1))),
        // What Ctrl+4 to Ctrl+7 send, as crossterm names them.
        0x1c..=0x1f => ctrl(KeyCode::Char(char::from(b'4' + byte - 0x1c))),
        _ => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
    }
}

/// The modifiers in a modifier parameter, `1 +` the bits for shift 1, alt 2, ctrl 4, super 8,
/// hyper 16 and meta 32. Caps Lock (64) and Num Lock (128) are left out. A value past 256 isn't a
/// modifier parameter, so it gives `None`.
pub(super) fn modifiers(param: Option<u32>) -> Option<KeyModifiers> {
    let bits = param.unwrap_or(1).saturating_sub(1);
    if bits > 0xff {
        return None;
    }
    let mut modifiers = KeyModifiers::NONE;
    for (bit, modifier) in [
        (1, KeyModifiers::SHIFT),
        (2, KeyModifiers::ALT),
        (4, KeyModifiers::CONTROL),
        (8, KeyModifiers::SUPER),
        (16, KeyModifiers::HYPER),
        (32, KeyModifiers::META),
    ] {
        if bits & bit != 0 {
            modifiers |= modifier;
        }
    }
    Some(modifiers)
}

/// The kitty protocol's event type, 1 press, 2 repeat and 3 release.
fn kind(event: Option<u32>) -> KeyEventKind {
    match event {
        Some(2) => KeyEventKind::Repeat,
        Some(3) => KeyEventKind::Release,
        _ => KeyEventKind::Press,
    }
}

/// A key from a control sequence, or `None` when it isn't one. `CSI ... R` is a cursor report
/// while one is expected, so it's a key (F3) only otherwise.
pub(super) fn csi(csi: &Csi, cursor_expected: bool) -> Option<KeyEvent> {
    if !csi.is_plain() {
        return None;
    }
    let params = &csi.params;
    match csi.final_byte {
        b'R' if cursor_expected => None,
        b'u' => kitty(csi),
        b'~' => tilde(csi),
        b'Z' => {
            let (modifiers, kind) = letter_modifiers(csi)?;
            Some(KeyEvent::new_with_kind(
                KeyCode::BackTab,
                modifiers | KeyModifiers::SHIFT,
                kind,
            ))
        }
        letter => {
            let code = letter_key(letter)?;
            // A cursor report nobody asked for has a row other than 1, so it's dropped.
            if letter == b'R' && params.count() > 1 && params.value(0) != Some(1) {
                return None;
            }
            let (modifiers, kind) = letter_modifiers(csi)?;
            Some(KeyEvent::new_with_kind(code, modifiers, kind))
        }
    }
}

/// The key of xterm's `CSI 1 ; m X` and `SS3 X` forms.
fn letter_key(letter: u8) -> Option<KeyCode> {
    Some(match letter {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'E' => KeyCode::KeypadBegin,
        b'F' => KeyCode::End,
        b'H' => KeyCode::Home,
        b'P' => KeyCode::F(1),
        b'Q' => KeyCode::F(2),
        b'R' => KeyCode::F(3),
        b'S' => KeyCode::F(4),
        _ => return None,
    })
}

/// The modifiers and event type of `CSI 1 ; m[:e] X`. With one parameter, as some terminals send
/// `CSI 5 D`, that parameter is the modifier.
fn letter_modifiers(csi: &Csi) -> Option<(KeyModifiers, KeyEventKind)> {
    let params = &csi.params;
    let group = usize::from(params.count() > 1);
    let modifiers = modifiers(params.value(group))?;
    Some((modifiers, kind(params.sub(group, 1))))
}

/// `SS3` and a letter, or the keypad's keys in application mode.
pub(super) fn ss3(csi: &Csi) -> Option<KeyEvent> {
    let final_byte = csi.final_byte;
    let code = match final_byte {
        b'M' => KeyCode::Enter,
        b'X' => KeyCode::Char('='),
        b'j' => KeyCode::Char('*'),
        b'k' => KeyCode::Char('+'),
        b'l' => KeyCode::Char(','),
        b'm' => KeyCode::Char('-'),
        b'n' => KeyCode::Char('.'),
        b'o' => KeyCode::Char('/'),
        b'p'..=b'y' => KeyCode::Char(char::from(b'0' + final_byte - b'p')),
        letter => letter_key(letter)?,
    };
    // `SS3 5 P`, a form some terminals send for modified F1 to F4, has the modifier last.
    let count = csi.params.count();
    let modifiers = match count.checked_sub(1) {
        Some(last) => modifiers(csi.params.value(last))?,
        None => KeyModifiers::NONE,
    };
    Some(KeyEvent::new(code, modifiers))
}

/// `CSI n ; m ~`, and modifyOtherKeys' `CSI 27 ; m ; c ~`.
fn tilde(csi: &Csi) -> Option<KeyEvent> {
    let params = &csi.params;
    let code = match params.value(0)? {
        27 => return modify_other_keys(csi),
        1 | 7 => KeyCode::Home,
        2 => KeyCode::Insert,
        3 => KeyCode::Delete,
        4 | 8 => KeyCode::End,
        5 => KeyCode::PageUp,
        6 => KeyCode::PageDown,
        n @ 11..=15 => KeyCode::F(function(n - 10)),
        n @ 17..=21 => KeyCode::F(function(n - 11)),
        n @ 23..=26 => KeyCode::F(function(n - 12)),
        n @ 28..=29 => KeyCode::F(function(n - 13)),
        n @ 31..=34 => KeyCode::F(function(n - 14)),
        57427 => KeyCode::KeypadBegin,
        _ => return None,
    };
    let modifiers = modifiers(params.value(1))?;
    Some(KeyEvent::new_with_kind(
        code,
        modifiers,
        kind(params.sub(1, 1)),
    ))
}

/// A function key's number, which the tables keep under 36.
fn function(n: u32) -> u8 {
    u8::try_from(n).unwrap_or(u8::MAX)
}

/// xterm's modifyOtherKeys, `CSI 27 ; m ; c ~`, where `c` is the character as typed.
fn modify_other_keys(csi: &Csi) -> Option<KeyEvent> {
    let params = &csi.params;
    let modifiers = modifiers(params.value(1))?;
    let code = character_key(params.value(2)?, modifiers)?;
    Some(KeyEvent::new(shifted(code, modifiers, None), modifiers))
}

/// The key a code point names in modifyOtherKeys and the kitty protocol, where Enter, Tab,
/// Backspace and Esc are their C0 codes.
fn character_key(code: u32, modifiers: KeyModifiers) -> Option<KeyCode> {
    Some(match code {
        9 if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
        9 => KeyCode::Tab,
        13 => KeyCode::Enter,
        27 => KeyCode::Esc,
        8 | 127 => KeyCode::Backspace,
        0x00..=0x1f => return None,
        _ => KeyCode::Char(char::from_u32(code)?),
    })
}

/// A character key held with Shift is the character it types, the `alternate` the kitty protocol's
/// flag 4 reports or else the capital of a letter, so Shift+a is `A` with Shift in every encoding.
fn shifted(code: KeyCode, modifiers: KeyModifiers, alternate: Option<u32>) -> KeyCode {
    match code {
        KeyCode::Char(ch) if modifiers.contains(KeyModifiers::SHIFT) => alternate
            .and_then(char::from_u32)
            .filter(|ch| !ch.is_control())
            .or_else(|| single_upper(ch))
            .map_or(code, KeyCode::Char),
        _ => code,
    }
}

/// The kitty protocol's `CSI code[:shifted[:base]] ; mods[:event] ; text u`.
fn kitty(csi: &Csi) -> Option<KeyEvent> {
    let params = &csi.params;
    let code = params.value(0)?;
    let mut modifiers = modifiers(params.value(1))?;
    let kind = kind(params.sub(1, 1));
    let key = match kitty_functional(code) {
        Some(KeyCode::Modifier(key)) => {
            modifiers |= modifier_bit(key);
            KeyCode::Modifier(key)
        }
        Some(key) => key,
        None => shifted(character_key(code, modifiers)?, modifiers, params.sub(0, 1)),
    };
    // (Flag 16's associated text, the third parameter, isn't kept, so a key compares equal
    // whatever flags the terminal had.)
    Some(KeyEvent::new_with_kind(key, modifiers, kind))
}

/// The capital of `ch` when it has exactly one.
fn single_upper(ch: char) -> Option<char> {
    let mut upper = ch.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(up), None) if up != ch => Some(up),
        _ => None,
    }
}

/// The modifier a modifier key sets, which the kitty protocol leaves to the next event.
fn modifier_bit(key: ModifierKey) -> KeyModifiers {
    match key {
        ModifierKey::LeftShift | ModifierKey::RightShift => KeyModifiers::SHIFT,
        ModifierKey::LeftControl | ModifierKey::RightControl => KeyModifiers::CONTROL,
        ModifierKey::LeftAlt | ModifierKey::RightAlt => KeyModifiers::ALT,
        ModifierKey::LeftSuper | ModifierKey::RightSuper => KeyModifiers::SUPER,
        ModifierKey::LeftHyper | ModifierKey::RightHyper => KeyModifiers::HYPER,
        ModifierKey::LeftMeta | ModifierKey::RightMeta => KeyModifiers::META,
        ModifierKey::IsoLevel3Shift | ModifierKey::IsoLevel5Shift => KeyModifiers::NONE,
    }
}

/// The kitty protocol's functional keys, numbered from 57344 in Unicode's private use area.
fn kitty_functional(code: u32) -> Option<KeyCode> {
    Some(match code {
        57344 => KeyCode::Esc,
        57345 => KeyCode::Enter,
        57346 => KeyCode::Tab,
        57347 => KeyCode::Backspace,
        57348 => KeyCode::Insert,
        57349 => KeyCode::Delete,
        57350 => KeyCode::Left,
        57351 => KeyCode::Right,
        57352 => KeyCode::Up,
        57353 => KeyCode::Down,
        57354 => KeyCode::PageUp,
        57355 => KeyCode::PageDown,
        57356 => KeyCode::Home,
        57357 => KeyCode::End,
        57358 => KeyCode::CapsLock,
        57359 => KeyCode::ScrollLock,
        57360 => KeyCode::NumLock,
        57361 => KeyCode::PrintScreen,
        57362 => KeyCode::Pause,
        57363 => KeyCode::Menu,
        n @ 57364..=57398 => KeyCode::F(function(n - 57363)),
        n @ 57399..=57408 => KeyCode::Char(char::from_digit(n - 57399, 10)?),
        57409 => KeyCode::Char('.'),
        57410 => KeyCode::Char('/'),
        57411 => KeyCode::Char('*'),
        57412 => KeyCode::Char('-'),
        57413 => KeyCode::Char('+'),
        57414 => KeyCode::Enter,
        57415 => KeyCode::Char('='),
        57416 => KeyCode::Char(','),
        57417 => KeyCode::Left,
        57418 => KeyCode::Right,
        57419 => KeyCode::Up,
        57420 => KeyCode::Down,
        57421 => KeyCode::PageUp,
        57422 => KeyCode::PageDown,
        57423 => KeyCode::Home,
        57424 => KeyCode::End,
        57425 => KeyCode::Insert,
        57426 => KeyCode::Delete,
        57427 => KeyCode::KeypadBegin,
        57428 => KeyCode::Media(MediaKey::Play),
        57429 => KeyCode::Media(MediaKey::Pause),
        57430 => KeyCode::Media(MediaKey::PlayPause),
        57431 => KeyCode::Media(MediaKey::Reverse),
        57432 => KeyCode::Media(MediaKey::Stop),
        57433 => KeyCode::Media(MediaKey::FastForward),
        57434 => KeyCode::Media(MediaKey::Rewind),
        57435 => KeyCode::Media(MediaKey::TrackNext),
        57436 => KeyCode::Media(MediaKey::TrackPrevious),
        57437 => KeyCode::Media(MediaKey::Record),
        57438 => KeyCode::Media(MediaKey::LowerVolume),
        57439 => KeyCode::Media(MediaKey::RaiseVolume),
        57440 => KeyCode::Media(MediaKey::MuteVolume),
        57441 => KeyCode::Modifier(ModifierKey::LeftShift),
        57442 => KeyCode::Modifier(ModifierKey::LeftControl),
        57443 => KeyCode::Modifier(ModifierKey::LeftAlt),
        57444 => KeyCode::Modifier(ModifierKey::LeftSuper),
        57445 => KeyCode::Modifier(ModifierKey::LeftHyper),
        57446 => KeyCode::Modifier(ModifierKey::LeftMeta),
        57447 => KeyCode::Modifier(ModifierKey::RightShift),
        57448 => KeyCode::Modifier(ModifierKey::RightControl),
        57449 => KeyCode::Modifier(ModifierKey::RightAlt),
        57450 => KeyCode::Modifier(ModifierKey::RightSuper),
        57451 => KeyCode::Modifier(ModifierKey::RightHyper),
        57452 => KeyCode::Modifier(ModifierKey::RightMeta),
        57453 => KeyCode::Modifier(ModifierKey::IsoLevel3Shift),
        57454 => KeyCode::Modifier(ModifierKey::IsoLevel5Shift),
        _ => return None,
    })
}
