//! Keys in the kitty keyboard protocol, from its specification, under each of its five flags.

use super::*;
use crate::event::{MediaKey, ModifierKey};

const NONE: KeyModifiers = KeyModifiers::NONE;
const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
const CTRL: KeyModifiers = KeyModifiers::CONTROL;
const ALT: KeyModifiers = KeyModifiers::ALT;

#[test]
fn disambiguate_escape_codes() {
    check_table(&[
        (b"\x1b[27u", vec![key(KeyCode::Esc, NONE)]),
        (b"\x1b[27;3u", vec![key(KeyCode::Esc, ALT)]),
        (b"\x1b[97;5u", vec![key(KeyCode::Char('a'), CTRL)]),
        (b"\x1b[97;3u", vec![key(KeyCode::Char('a'), ALT)]),
        (b"\x1b[97;7u", vec![key(KeyCode::Char('a'), CTRL | ALT)]),
        // With Ctrl or Alt, a letter stays lowercase and Shift stays held, as crossterm has it.
        (b"\x1b[97;4u", vec![key(KeyCode::Char('a'), SHIFT | ALT)]),
        (b"\x1b[97;6u", vec![key(KeyCode::Char('a'), SHIFT | CTRL)]),
        (
            b"\x1b[97:65;6u",
            vec![key(KeyCode::Char('a'), SHIFT | CTRL)],
        ),
        (
            b"\x1b[233:201;4u",
            vec![key(KeyCode::Char('é'), SHIFT | ALT)],
        ),
        (
            b"\x1b[97;10u",
            vec![key(KeyCode::Char('A'), SHIFT | KeyModifiers::SUPER)],
        ),
        (b"\x1b[49;5u", vec![key(KeyCode::Char('1'), CTRL)]),
        (b"\x1b[32;5u", vec![key(KeyCode::Char(' '), CTRL)]),
        (b"\x1b[13;2u", vec![key(KeyCode::Enter, SHIFT)]),
        (b"\x1b[13;5u", vec![key(KeyCode::Enter, CTRL)]),
        (b"\x1b[9;2u", vec![key(KeyCode::BackTab, SHIFT)]),
        (b"\x1b[9;5u", vec![key(KeyCode::Tab, CTRL)]),
        (b"\x1b[127;5u", vec![key(KeyCode::Backspace, CTRL)]),
        (b"\x1b[127;3u", vec![key(KeyCode::Backspace, ALT)]),
        (
            b"\x1b[97;9u",
            vec![key(KeyCode::Char('a'), KeyModifiers::SUPER)],
        ),
        (
            b"\x1b[97;17u",
            vec![key(KeyCode::Char('a'), KeyModifiers::HYPER)],
        ),
        (
            b"\x1b[97;33u",
            vec![key(KeyCode::Char('a'), KeyModifiers::META)],
        ),
        // Caps Lock and Num Lock (64 and 128) aren't modifiers.
        (b"\x1b[97;69u", vec![key(KeyCode::Char('a'), CTRL)]),
        (b"\x1b[97;129u", vec![ch('a')]),
        // A non-Latin layout's key.
        (
            "\x1b[1092;5u".as_bytes(),
            vec![key(KeyCode::Char('ф'), CTRL)],
        ),
        // The legacy forms the protocol keeps for these keys.
        (b"\x1b[1;5A", vec![key(KeyCode::Up, CTRL)]),
        (b"\x1b[1;2H", vec![key(KeyCode::Home, SHIFT)]),
        (b"\x1b[2;3~", vec![key(KeyCode::Insert, ALT)]),
        (b"\x1b[13~", vec![key(KeyCode::F(3), NONE)]),
        (b"\x1b[1;5P", vec![key(KeyCode::F(1), CTRL)]),
        (b"\x1b[E", vec![key(KeyCode::KeypadBegin, NONE)]),
        (b"\x1b[1;5E", vec![key(KeyCode::KeypadBegin, CTRL)]),
        (b"\x1b[57427~", vec![key(KeyCode::KeypadBegin, NONE)]),
        (b"\x1b[57427;2~", vec![key(KeyCode::KeypadBegin, SHIFT)]),
    ]);
}

#[test]
fn report_event_types() {
    use KeyEventKind::{Press, Release, Repeat};
    check_table(&[
        (
            b"\x1b[97;1u",
            vec![key_kind(KeyCode::Char('a'), NONE, Press)],
        ),
        (
            b"\x1b[97;1:1u",
            vec![key_kind(KeyCode::Char('a'), NONE, Press)],
        ),
        (
            b"\x1b[97;5:1u",
            vec![key_kind(KeyCode::Char('a'), CTRL, Press)],
        ),
        (
            b"\x1b[97;1:2u",
            vec![key_kind(KeyCode::Char('a'), NONE, Repeat)],
        ),
        (
            b"\x1b[97;1:3u",
            vec![key_kind(KeyCode::Char('a'), NONE, Release)],
        ),
        (b"\x1b[27;1:3u", vec![key_kind(KeyCode::Esc, NONE, Release)]),
        (b"\x1b[1;1:3A", vec![key_kind(KeyCode::Up, NONE, Release)]),
        (b"\x1b[1;5:2D", vec![key_kind(KeyCode::Left, CTRL, Repeat)]),
        (
            b"\x1b[3;1:3~",
            vec![key_kind(KeyCode::Delete, NONE, Release)],
        ),
        (
            b"\x1b[15;2:2~",
            vec![key_kind(KeyCode::F(5), SHIFT, Repeat)],
        ),
        (b"\x1b[1;1:3P", vec![key_kind(KeyCode::F(1), NONE, Release)]),
        (
            b"\x1b[57399;1:3u",
            vec![key_kind(KeyCode::Char('0'), NONE, Release)],
        ),
        // An event type the protocol doesn't define is a press.
        (
            b"\x1b[97;1:9u",
            vec![key_kind(KeyCode::Char('a'), NONE, Press)],
        ),
    ]);
}

#[test]
fn report_alternate_keys() {
    check_table(&[
        (b"\x1b[97:65;2u", vec![key(KeyCode::Char('A'), SHIFT)]),
        (b"\x1b[49:33;2u", vec![key(KeyCode::Char('!'), SHIFT)]),
        (
            b"\x1b[59:58;6u",
            vec![key(KeyCode::Char(':'), SHIFT | CTRL)],
        ),
        // Typed with Shift alone, a key of another layout stays its own character.
        (
            "\x1b[1092:1060:97;2u".as_bytes(),
            vec![key(KeyCode::Char('Ф'), SHIFT)],
        ),
        // The shifted key counts only with Shift held.
        (b"\x1b[97:65;5u", vec![key(KeyCode::Char('a'), CTRL)]),
    ]);
}

/// A shortcut on a layout outside ASCII is named by the key in the same place on a US layout,
/// which the protocol reports as the base layout key, so Ctrl+C is Ctrl+C on a Cyrillic layout.
/// The Cyrillic `ф` and `с` sit where `a` and `c` do, and the French `é` on the `2` key. A layout
/// of ASCII letters keeps its own, so the German Ctrl+Z is Ctrl+Z.
#[test]
fn shortcuts_on_other_layouts_are_their_us_keys() {
    check_table(&[
        (
            "\x1b[1092::97;5u".as_bytes(),
            vec![key(KeyCode::Char('a'), CTRL)],
        ),
        (
            "\x1b[1089::99;5u".as_bytes(),
            vec![key(KeyCode::Char('c'), CTRL)],
        ),
        (
            "\x1b[1089::99;3u".as_bytes(),
            vec![key(KeyCode::Char('c'), ALT)],
        ),
        (
            "\x1b[1092::97;9u".as_bytes(),
            vec![key(KeyCode::Char('a'), KeyModifiers::SUPER)],
        ),
        (
            "\x1b[1092:1060:97;6u".as_bytes(),
            vec![key(KeyCode::Char('a'), CTRL | SHIFT)],
        ),
        (
            "\x1b[233::50;5u".as_bytes(),
            vec![key(KeyCode::Char('2'), CTRL)],
        ),
        (
            "\x1b[1092::97;5:3u".as_bytes(),
            vec![key_kind(KeyCode::Char('a'), CTRL, KeyEventKind::Release)],
        ),
        // Without the base key, or with an ASCII key, the key is its own.
        (
            "\x1b[1092;5u".as_bytes(),
            vec![key(KeyCode::Char('ф'), CTRL)],
        ),
        (b"\x1b[97::97;5u", vec![key(KeyCode::Char('a'), CTRL)]),
        (b"\x1b[122::121;5u", vec![key(KeyCode::Char('z'), CTRL)]),
        // A base key that is no printable ASCII is ignored.
        (
            "\x1b[1092::9;5u".as_bytes(),
            vec![key(KeyCode::Char('ф'), CTRL)],
        ),
        (
            "\x1b[1092::1093;5u".as_bytes(),
            vec![key(KeyCode::Char('ф'), CTRL)],
        ),
    ]);
}

#[test]
fn report_all_keys_as_escape_codes() {
    use KeyEventKind::Release;
    check_table(&[
        (b"\x1b[97u", vec![ch('a')]),
        (b"\x1b[97;2u", vec![key(KeyCode::Char('A'), SHIFT)]),
        (b"\x1b[13u", vec![key(KeyCode::Enter, NONE)]),
        (b"\x1b[9u", vec![key(KeyCode::Tab, NONE)]),
        (b"\x1b[127u", vec![key(KeyCode::Backspace, NONE)]),
        (b"\x1b[32u", vec![ch(' ')]),
        // A modifier key sets its own modifier, pressed and released.
        (
            b"\x1b[57441;2u",
            vec![key(KeyCode::Modifier(ModifierKey::LeftShift), SHIFT)],
        ),
        (
            b"\x1b[57441;1:3u",
            vec![key_kind(
                KeyCode::Modifier(ModifierKey::LeftShift),
                SHIFT,
                Release,
            )],
        ),
        (
            b"\x1b[57449u",
            vec![key(KeyCode::Modifier(ModifierKey::RightAlt), ALT)],
        ),
        (
            b"\x1b[57453u",
            vec![key(KeyCode::Modifier(ModifierKey::IsoLevel3Shift), NONE)],
        ),
    ]);
}

/// The associated text isn't kept, so a key is the same whether the terminal sent it or not.
#[test]
fn report_associated_text() {
    check_table(&[
        (b"\x1b[97;;97u", vec![ch('a')]),
        (b"\x1b[97;2;65u", vec![key(KeyCode::Char('A'), SHIFT)]),
        (b"\x1b[97:65;2;65u", vec![key(KeyCode::Char('A'), SHIFT)]),
        (b"\x1b[101;;101:769u", vec![ch('e')]),
        (b"\x1b[13;1;u", vec![key(KeyCode::Enter, NONE)]),
    ]);
}

#[test]
fn every_functional_key() {
    let mut table: Vec<(u32, KeyCode)> = vec![
        (57344, KeyCode::Esc),
        (57345, KeyCode::Enter),
        (57346, KeyCode::Tab),
        (57347, KeyCode::Backspace),
        (57348, KeyCode::Insert),
        (57349, KeyCode::Delete),
        (57350, KeyCode::Left),
        (57351, KeyCode::Right),
        (57352, KeyCode::Up),
        (57353, KeyCode::Down),
        (57354, KeyCode::PageUp),
        (57355, KeyCode::PageDown),
        (57356, KeyCode::Home),
        (57357, KeyCode::End),
        (57358, KeyCode::CapsLock),
        (57359, KeyCode::ScrollLock),
        (57360, KeyCode::NumLock),
        (57361, KeyCode::PrintScreen),
        (57362, KeyCode::Pause),
        (57363, KeyCode::Menu),
        (57409, KeyCode::Char('.')),
        (57410, KeyCode::Char('/')),
        (57411, KeyCode::Char('*')),
        (57412, KeyCode::Char('-')),
        (57413, KeyCode::Char('+')),
        (57414, KeyCode::Enter),
        (57415, KeyCode::Char('=')),
        (57416, KeyCode::Char(',')),
        (57417, KeyCode::Left),
        (57418, KeyCode::Right),
        (57419, KeyCode::Up),
        (57420, KeyCode::Down),
        (57421, KeyCode::PageUp),
        (57422, KeyCode::PageDown),
        (57423, KeyCode::Home),
        (57424, KeyCode::End),
        (57425, KeyCode::Insert),
        (57426, KeyCode::Delete),
        (57427, KeyCode::KeypadBegin),
        (57428, KeyCode::Media(MediaKey::Play)),
        (57429, KeyCode::Media(MediaKey::Pause)),
        (57430, KeyCode::Media(MediaKey::PlayPause)),
        (57431, KeyCode::Media(MediaKey::Reverse)),
        (57432, KeyCode::Media(MediaKey::Stop)),
        (57433, KeyCode::Media(MediaKey::FastForward)),
        (57434, KeyCode::Media(MediaKey::Rewind)),
        (57435, KeyCode::Media(MediaKey::TrackNext)),
        (57436, KeyCode::Media(MediaKey::TrackPrevious)),
        (57437, KeyCode::Media(MediaKey::Record)),
        (57438, KeyCode::Media(MediaKey::LowerVolume)),
        (57439, KeyCode::Media(MediaKey::RaiseVolume)),
        (57440, KeyCode::Media(MediaKey::MuteVolume)),
        (57441, KeyCode::Modifier(ModifierKey::LeftShift)),
        (57442, KeyCode::Modifier(ModifierKey::LeftControl)),
        (57443, KeyCode::Modifier(ModifierKey::LeftAlt)),
        (57444, KeyCode::Modifier(ModifierKey::LeftSuper)),
        (57445, KeyCode::Modifier(ModifierKey::LeftHyper)),
        (57446, KeyCode::Modifier(ModifierKey::LeftMeta)),
        (57447, KeyCode::Modifier(ModifierKey::RightShift)),
        (57448, KeyCode::Modifier(ModifierKey::RightControl)),
        (57449, KeyCode::Modifier(ModifierKey::RightAlt)),
        (57450, KeyCode::Modifier(ModifierKey::RightSuper)),
        (57451, KeyCode::Modifier(ModifierKey::RightHyper)),
        (57452, KeyCode::Modifier(ModifierKey::RightMeta)),
        (57453, KeyCode::Modifier(ModifierKey::IsoLevel3Shift)),
        (57454, KeyCode::Modifier(ModifierKey::IsoLevel5Shift)),
    ];
    for n in 1..=35u8 {
        table.push((57363 + u32::from(n), KeyCode::F(n)));
    }
    for digit in 0..=9u8 {
        table.push((
            57399 + u32::from(digit),
            KeyCode::Char(char::from(b'0' + digit)),
        ));
    }
    for (code, expected) in table {
        // Ctrl, so a modifier key's own bit is the only other one.
        let decoded = decode(format!("\x1b[{code};5u").as_bytes());
        let Some(Decoded::Event(Event::Key(got))) = decoded.first() else {
            panic!("{code} decoded to {decoded:?}");
        };
        assert_eq!(decoded.len(), 1, "{code}");
        assert_eq!(got.code, expected, "{code}");
        assert!(got.modifiers.contains(CTRL), "{code}");
        assert_eq!(got.kind, KeyEventKind::Press, "{code}");
    }
}

#[test]
fn malformed_keys_are_dropped() {
    check_table(&[
        // Not a code point, and no code.
        (b"\x1b[1114112ux", vec![ch('x')]),
        (b"\x1b[55296ux", vec![ch('x')]),
        (b"\x1b[;5ux", vec![ch('x')]),
        (b"\x1b[0ux", vec![ch('x')]),
        (b"\x1b[5ux", vec![ch('x')]),
        // A modifier parameter past 256.
        (b"\x1b[97;300ux", vec![ch('x')]),
        // Unknown functional keys are characters of the private use area.
        ("\x1b[57500u".as_bytes(), vec![ch('\u{e09c}')]),
    ]);
}

#[test]
fn flags_reply_is_no_key() {
    assert_eq!(
        decode(b"\x1b[?1u"),
        [Decoded::Reply(Reply::KittyKeyboard(1))]
    );
}
