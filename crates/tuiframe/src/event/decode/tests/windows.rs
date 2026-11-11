//! Windows console records, from the virtual-key list and the console's record documentation.

use super::*;
use crate::event::records::{
    FocusRecord, InputRecord, KeyRecord, MouseRecord, WindowSizeRecord, control, mouse as flags,
};
use crate::event::{MediaKey, ModifierKey, MouseButton, MouseEvent, MouseEventKind};
use crate::layout::Size;

const NONE: KeyModifiers = KeyModifiers::NONE;
const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
const CTRL: KeyModifiers = KeyModifiers::CONTROL;
const ALT: KeyModifiers = KeyModifiers::ALT;

fn down(virtual_key: u16, unicode: u16, control_state: u32) -> InputRecord {
    InputRecord::Key(KeyRecord {
        key_down: true,
        repeat: 1,
        virtual_key,
        scan_code: 0,
        unicode,
        control_state,
    })
}

fn up(virtual_key: u16, unicode: u16, control_state: u32) -> InputRecord {
    InputRecord::Key(KeyRecord {
        key_down: false,
        repeat: 1,
        virtual_key,
        scan_code: 0,
        unicode,
        control_state,
    })
}

fn records(records: &[InputRecord]) -> Vec<Decoded> {
    let mut decoder = Decoder::new(DecoderOptions::default());
    let mut out = Vec::new();
    decoder.decode_records(records, &mut out);
    out
}

fn release(code: KeyCode, modifiers: KeyModifiers) -> Decoded {
    key_kind(code, modifiers, KeyEventKind::Release)
}

#[test]
fn every_virtual_key_in_the_table() {
    let mut table: Vec<(u16, KeyCode)> = vec![
        (0x08, KeyCode::Backspace),
        (0x09, KeyCode::Tab),
        (0x0c, KeyCode::KeypadBegin),
        (0x0d, KeyCode::Enter),
        (0x13, KeyCode::Pause),
        (0x14, KeyCode::CapsLock),
        (0x1b, KeyCode::Esc),
        (0x21, KeyCode::PageUp),
        (0x22, KeyCode::PageDown),
        (0x23, KeyCode::End),
        (0x24, KeyCode::Home),
        (0x25, KeyCode::Left),
        (0x26, KeyCode::Up),
        (0x27, KeyCode::Right),
        (0x28, KeyCode::Down),
        (0x2c, KeyCode::PrintScreen),
        (0x2d, KeyCode::Insert),
        (0x2e, KeyCode::Delete),
        (0x5d, KeyCode::Menu),
        (0x90, KeyCode::NumLock),
        (0x91, KeyCode::ScrollLock),
        (0xad, KeyCode::Media(MediaKey::MuteVolume)),
        (0xae, KeyCode::Media(MediaKey::LowerVolume)),
        (0xaf, KeyCode::Media(MediaKey::RaiseVolume)),
        (0xb0, KeyCode::Media(MediaKey::TrackNext)),
        (0xb1, KeyCode::Media(MediaKey::TrackPrevious)),
        (0xb2, KeyCode::Media(MediaKey::Stop)),
        (0xb3, KeyCode::Media(MediaKey::PlayPause)),
        (0xfa, KeyCode::Media(MediaKey::Play)),
    ];
    for n in 1..=24u8 {
        table.push((0x6f + u16::from(n), KeyCode::F(n)));
    }
    for (vk, code) in table {
        assert_eq!(
            records(&[down(vk, 0, 0), up(vk, 0, 0)]),
            [key(code, NONE), release(code, NONE)],
            "virtual key {vk:#x}"
        );
    }
}

#[test]
fn modifier_keys_alone() {
    let enhanced = control::ENHANCED_KEY;
    let shift_down = |scan_code| {
        InputRecord::Key(KeyRecord {
            key_down: true,
            repeat: 1,
            virtual_key: 0x10,
            scan_code,
            unicode: 0,
            control_state: control::SHIFT_PRESSED,
        })
    };
    let modifier = |key| KeyCode::Modifier(key);
    assert_eq!(
        records(&[
            shift_down(0x2a),
            shift_down(0x36),
            down(0x11, 0, control::LEFT_CTRL_PRESSED),
            down(0x11, 0, control::RIGHT_CTRL_PRESSED | enhanced),
            down(0x12, 0, control::LEFT_ALT_PRESSED),
            down(0x12, 0, control::RIGHT_ALT_PRESSED | enhanced),
            down(0x5b, 0, 0),
            down(0x5c, 0, 0),
            down(0xa0, 0, 0),
            down(0xa1, 0, 0),
            down(0xa2, 0, 0),
            down(0xa3, 0, 0),
            down(0xa4, 0, 0),
            down(0xa5, 0, 0),
            // A release no longer has the modifier in the state, and still says it.
            up(0x10, 0, 0),
        ]),
        [
            key(modifier(ModifierKey::LeftShift), SHIFT),
            key(modifier(ModifierKey::RightShift), SHIFT),
            key(modifier(ModifierKey::LeftControl), CTRL),
            key(modifier(ModifierKey::RightControl), CTRL),
            key(modifier(ModifierKey::LeftAlt), ALT),
            key(modifier(ModifierKey::RightAlt), ALT),
            key(modifier(ModifierKey::LeftSuper), KeyModifiers::SUPER),
            key(modifier(ModifierKey::RightSuper), KeyModifiers::SUPER),
            key(modifier(ModifierKey::LeftShift), SHIFT),
            key(modifier(ModifierKey::RightShift), SHIFT),
            key(modifier(ModifierKey::LeftControl), CTRL),
            key(modifier(ModifierKey::RightControl), CTRL),
            key(modifier(ModifierKey::LeftAlt), ALT),
            key(modifier(ModifierKey::RightAlt), ALT),
            release(modifier(ModifierKey::LeftShift), SHIFT),
        ]
    );
}

#[test]
fn characters_and_modifiers() {
    let shift = control::SHIFT_PRESSED;
    let ctrl = control::LEFT_CTRL_PRESSED;
    let alt = control::LEFT_ALT_PRESSED;
    let rows = [
        (
            down(0x41, u16::from(b'a'), 0),
            key(KeyCode::Char('a'), NONE),
        ),
        (
            down(0x41, u16::from(b'A'), shift),
            key(KeyCode::Char('A'), SHIFT),
        ),
        // Caps Lock alone types the capital, with no Shift held.
        (
            down(0x41, u16::from(b'A'), control::CAPSLOCK_ON),
            key(KeyCode::Char('A'), NONE),
        ),
        (
            down(0x41, u16::from(b'a'), alt),
            key(KeyCode::Char('a'), ALT),
        ),
        (down(0x41, 0x01, ctrl), key(KeyCode::Char('a'), CTRL)),
        (
            down(0x41, 0x01, ctrl | shift),
            key(KeyCode::Char('A'), CTRL | SHIFT),
        ),
        (
            down(0x41, 0x01, ctrl | shift | control::CAPSLOCK_ON),
            key(KeyCode::Char('a'), CTRL | SHIFT),
        ),
        (down(0x5a, 0x1a, ctrl), key(KeyCode::Char('z'), CTRL)),
        (down(0x32, 0x00, ctrl), key(KeyCode::Char('2'), CTRL)),
        (down(0x36, 0x1e, ctrl), key(KeyCode::Char('6'), CTRL)),
        (down(0x20, 0x00, ctrl), key(KeyCode::Char(' '), CTRL)),
        (down(0x20, 0x20, 0), ch(' ')),
        (down(0xdb, 0x1b, ctrl), key(KeyCode::Char('['), CTRL)),
        (down(0xdc, 0x1c, ctrl), key(KeyCode::Char('\\'), CTRL)),
        (down(0xdd, 0x1d, ctrl), key(KeyCode::Char(']'), CTRL)),
        (
            down(0xbd, 0x1f, ctrl | shift),
            key(KeyCode::Char('-'), CTRL | SHIFT),
        ),
        (
            down(0x31, u16::from(b'!'), shift),
            key(KeyCode::Char('!'), SHIFT),
        ),
        (down(0x6b, u16::from(b'+'), 0), ch('+')),
        (down(0x61, u16::from(b'1'), 0), ch('1')),
        (down(0xbf, u16::from(b'/'), 0), ch('/')),
        (down(0xde, 0xe9, 0), ch('é')),
        (down(0xe7, 0x4e2d, 0), ch('中')),
        (down(0x0d, 0x0d, shift), key(KeyCode::Enter, SHIFT)),
        (down(0x0d, 0x0a, ctrl), key(KeyCode::Enter, CTRL)),
        (down(0x0d, 0x0d, alt), key(KeyCode::Enter, ALT)),
        (down(0x09, 0x09, shift), key(KeyCode::BackTab, SHIFT)),
        (down(0x08, 0x7f, ctrl), key(KeyCode::Backspace, CTRL)),
        (down(0x1b, 0x1b, 0), key(KeyCode::Esc, NONE)),
        (down(0x26, 0, ctrl | shift), key(KeyCode::Up, CTRL | SHIFT)),
        // A control code from a key with no character of its own.
        (down(0xe7, 0x0a, 0), key(KeyCode::Char('j'), CTRL)),
    ];
    for (record, expected) in rows {
        assert_eq!(records(&[record]), [expected], "{record:?}");
    }
}

#[test]
fn alt_gr_types_its_character() {
    let alt_gr = control::LEFT_CTRL_PRESSED | control::RIGHT_ALT_PRESSED;
    assert_eq!(
        records(&[
            down(0x51, u16::from(b'@'), alt_gr),
            down(0x45, 0x20ac, alt_gr),
            down(0xdb, u16::from(b'{'), alt_gr | control::SHIFT_PRESSED),
            // Ctrl+Alt with no character of the layout's is the key with Ctrl and Alt.
            down(0x41, 0, alt_gr),
        ]),
        [
            ch('@'),
            ch('€'),
            key(KeyCode::Char('{'), SHIFT),
            key(KeyCode::Char('a'), CTRL | ALT),
        ]
    );
}

#[test]
fn a_dead_key_types_nothing() {
    assert_eq!(records(&[down(0xde, 0, 0), down(0x45, 0xe9, 0)]), [ch('é')]);
}

#[test]
fn surrogate_pairs_are_joined() {
    // U+1F600, as D83D DE00, in either order of downs and ups.
    let (high, low) = (0xd83d, 0xde00);
    let smile = KeyCode::Char('😀');
    assert_eq!(
        records(&[
            down(0xe7, high, 0),
            up(0xe7, high, 0),
            down(0xe7, low, 0),
            up(0xe7, low, 0),
        ]),
        [key(smile, NONE), release(smile, NONE)]
    );
    assert_eq!(
        records(&[
            down(0xe7, high, 0),
            down(0xe7, low, 0),
            up(0xe7, high, 0),
            up(0xe7, low, 0),
        ]),
        [key(smile, NONE), release(smile, NONE)]
    );
    // A low surrogate alone is dropped, and a new high one replaces the old.
    assert_eq!(
        records(&[
            down(0xe7, low, 0),
            down(0xe7, 0xd800, 0),
            down(0xe7, high, 0),
            down(0xe7, low, 0),
        ]),
        [key(smile, NONE)]
    );
}

#[test]
fn a_repeat_count_of_n_gives_n_presses() {
    let held = |repeat, virtual_key, unicode| {
        InputRecord::Key(KeyRecord {
            key_down: true,
            repeat,
            virtual_key,
            scan_code: 0,
            unicode,
            control_state: 0,
        })
    };
    assert_eq!(
        records(&[
            held(3, 0x41, u16::from(b'a')),
            held(0, 0x28, 0),
            up(0x41, u16::from(b'a'), 0)
        ]),
        [
            ch('a'),
            ch('a'),
            ch('a'),
            key(KeyCode::Down, NONE),
            release(KeyCode::Char('a'), NONE),
        ]
    );
}

#[test]
fn alt_codes_arrive_with_the_alt_release() {
    let alt = control::LEFT_ALT_PRESSED;
    assert_eq!(
        records(&[
            down(0x12, 0, alt),
            down(0x62, 0, alt),
            up(0x62, 0, alt),
            down(0x63, 0, alt),
            up(0x63, 0, alt),
            up(0x12, 0xe9, 0),
        ]),
        [key(KeyCode::Modifier(ModifierKey::LeftAlt), ALT), ch('é'),]
    );
    // With Ctrl too, the keypad's digits are keys.
    assert_eq!(
        records(&[down(0x62, 0, alt | control::LEFT_CTRL_PRESSED)]),
        [key(KeyCode::Char('2'), CTRL | ALT)]
    );
}

fn mouse_record(column: u16, row: u16, button_state: u32, event_flags: u32) -> InputRecord {
    InputRecord::Mouse(MouseRecord {
        column,
        row,
        button_state,
        control_state: 0,
        event_flags,
    })
}

fn mouse_event(kind: MouseEventKind, column: u16, row: u16) -> Decoded {
    event(Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: NONE,
    }))
}

#[test]
fn mouse_records() {
    use MouseButton::{Left, Middle, Right};
    use MouseEventKind::{Down, Drag, Moved, ScrollDown, ScrollLeft, ScrollRight, ScrollUp, Up};
    let left = flags::FROM_LEFT_1ST_BUTTON_PRESSED;
    let right = flags::RIGHTMOST_BUTTON_PRESSED;
    let middle = flags::FROM_LEFT_2ND_BUTTON_PRESSED;
    assert_eq!(
        records(&[
            mouse_record(1, 2, left, 0),
            mouse_record(2, 2, left, flags::MOUSE_MOVED),
            mouse_record(3, 2, 0, 0),
            mouse_record(3, 3, 0, flags::MOUSE_MOVED),
            mouse_record(3, 3, left, flags::DOUBLE_CLICK),
            mouse_record(3, 3, left | right, 0),
            mouse_record(4, 3, left | right, flags::MOUSE_MOVED),
            mouse_record(4, 3, middle, 0),
            mouse_record(4, 3, middle, flags::MOUSE_MOVED),
            mouse_record(4, 3, 0, 0),
            mouse_record(5, 5, 120 << 16, flags::MOUSE_WHEELED),
            mouse_record(5, 5, 0xff88 << 16, flags::MOUSE_WHEELED),
            mouse_record(5, 5, 120 << 16, flags::MOUSE_HWHEELED),
            mouse_record(5, 5, 0xff88 << 16, flags::MOUSE_HWHEELED),
            mouse_record(5, 5, 0, flags::MOUSE_WHEELED),
        ]),
        [
            mouse_event(Down(Left), 1, 2),
            mouse_event(Drag(Left), 2, 2),
            mouse_event(Up(Left), 3, 2),
            mouse_event(Moved, 3, 3),
            mouse_event(Down(Left), 3, 3),
            mouse_event(Down(Right), 3, 3),
            mouse_event(Drag(Left), 4, 3),
            mouse_event(Up(Left), 4, 3),
            mouse_event(Up(Right), 4, 3),
            mouse_event(Down(Middle), 4, 3),
            mouse_event(Drag(Middle), 4, 3),
            mouse_event(Up(Middle), 4, 3),
            mouse_event(ScrollUp, 5, 5),
            mouse_event(ScrollDown, 5, 5),
            mouse_event(ScrollRight, 5, 5),
            mouse_event(ScrollLeft, 5, 5),
        ]
    );
}

#[test]
fn mouse_modifiers() {
    let record = InputRecord::Mouse(MouseRecord {
        column: 0,
        row: 0,
        button_state: flags::FROM_LEFT_1ST_BUTTON_PRESSED,
        control_state: control::SHIFT_PRESSED | control::RIGHT_CTRL_PRESSED,
        event_flags: 0,
    });
    assert_eq!(
        records(&[record]),
        [event(Event::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: SHIFT | CTRL,
        }))]
    );
}

#[test]
fn window_size_and_focus_records() {
    assert_eq!(
        records(&[
            InputRecord::WindowSize(WindowSizeRecord {
                size: Size::new(120, 40),
            }),
            InputRecord::Focus(FocusRecord { focused: false }),
            InputRecord::Focus(FocusRecord { focused: true }),
        ]),
        [
            event(Event::Resize(Size::new(120, 40))),
            event(Event::FocusLost),
            event(Event::FocusGained),
        ]
    );
}

#[test]
fn keys_that_type_nothing_give_no_event() {
    assert_eq!(records(&[down(0xff, 0, 0), down(0x41, 0, 0)]), []);
}
