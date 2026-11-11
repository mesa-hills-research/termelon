//! The bytes each terminal sends for keys, mouse events, pastes and focus, from xterm's control
//! sequences document and the kitty keyboard protocol's specification.

use tuiframe_vt::KeyKind::{Press, Release, Repeat};
use tuiframe_vt::{Emulator, Key, KeyKind, Mods, Mouse, MouseButton, MouseKind, Profile};

const NONE: Mods = Mods::NONE;
const SHIFT: Mods = Mods::SHIFT;
const ALT: Mods = Mods::ALT;
const CTRL: Mods = Mods::CTRL;
const SUPER: Mods = Mods::SUPER;

#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
fn term(profile: Profile, setup: &[u8]) -> Emulator {
    let mut term = Emulator::with_profile(profile, 300, 300);
    term.feed(setup).unwrap();
    term
}

fn check(term: &Emulator, table: &[(Key, Mods, KeyKind, &str)]) {
    for &(key, mods, kind, expected) in table {
        let bytes = term.encode_key_event(key, mods, kind);
        assert_eq!(
            String::from_utf8_lossy(&bytes),
            expected,
            "{} {key:?} {mods:?} {kind:?}",
            term.profile().name
        );
    }
}

#[test]
fn xterm_legacy_keys() {
    check(
        &term(Profile::xterm(), b""),
        &[
            (Key::Char('a'), NONE, Press, "a"),
            (Key::Char('a'), SHIFT, Press, "A"),
            (Key::Char('a'), CTRL, Press, "\x01"),
            (Key::Char('z'), CTRL, Press, "\x1a"),
            (Key::Char('a'), ALT, Press, "\x1ba"),
            (Key::Char('a'), CTRL | ALT, Press, "\x1b\x01"),
            (Key::Char('a'), SHIFT | ALT, Press, "\x1bA"),
            (Key::Char(' '), CTRL, Press, "\x00"),
            (Key::Char('['), CTRL, Press, "\x1b"),
            (Key::Char('\\'), CTRL, Press, "\x1c"),
            (Key::Char(']'), CTRL, Press, "\x1d"),
            (Key::Char('/'), CTRL, Press, "\x1f"),
            (Key::Char('1'), CTRL, Press, "1"),
            (Key::Char('é'), NONE, Press, "é"),
            (Key::Char('é'), SHIFT, Press, "É"),
            (Key::Char('a'), SUPER, Press, "a"),
            (Key::Enter, NONE, Press, "\r"),
            (Key::Enter, SHIFT, Press, "\r"),
            (Key::Enter, ALT, Press, "\x1b\r"),
            (Key::Tab, NONE, Press, "\t"),
            (Key::Tab, SHIFT, Press, "\x1b[Z"),
            (Key::Backspace, NONE, Press, "\x7f"),
            (Key::Backspace, CTRL, Press, "\x08"),
            (Key::Backspace, ALT, Press, "\x1b\x7f"),
            (Key::Escape, NONE, Press, "\x1b"),
            (Key::Escape, ALT, Press, "\x1b\x1b"),
            (Key::Up, NONE, Press, "\x1b[A"),
            (Key::Down, NONE, Press, "\x1b[B"),
            (Key::Right, NONE, Press, "\x1b[C"),
            (Key::Left, NONE, Press, "\x1b[D"),
            (Key::Up, SHIFT, Press, "\x1b[1;2A"),
            (Key::Left, ALT, Press, "\x1b[1;3D"),
            (Key::Right, CTRL, Press, "\x1b[1;5C"),
            (Key::Up, CTRL | SHIFT, Press, "\x1b[1;6A"),
            (Key::Up, SUPER, Press, "\x1b[1;9A"),
            (Key::Home, NONE, Press, "\x1b[H"),
            (Key::End, NONE, Press, "\x1b[F"),
            (Key::Home, CTRL, Press, "\x1b[1;5H"),
            (Key::Insert, NONE, Press, "\x1b[2~"),
            (Key::Delete, NONE, Press, "\x1b[3~"),
            (Key::PageUp, NONE, Press, "\x1b[5~"),
            (Key::PageDown, NONE, Press, "\x1b[6~"),
            (Key::Delete, SHIFT, Press, "\x1b[3;2~"),
            (Key::PageUp, CTRL, Press, "\x1b[5;5~"),
            (Key::F(1), NONE, Press, "\x1bOP"),
            (Key::F(2), NONE, Press, "\x1bOQ"),
            (Key::F(3), NONE, Press, "\x1bOR"),
            (Key::F(4), NONE, Press, "\x1bOS"),
            (Key::F(1), SHIFT, Press, "\x1b[1;2P"),
            (Key::F(4), CTRL, Press, "\x1b[1;5S"),
            (Key::F(5), NONE, Press, "\x1b[15~"),
            (Key::F(6), NONE, Press, "\x1b[17~"),
            (Key::F(7), NONE, Press, "\x1b[18~"),
            (Key::F(8), NONE, Press, "\x1b[19~"),
            (Key::F(9), NONE, Press, "\x1b[20~"),
            (Key::F(10), NONE, Press, "\x1b[21~"),
            (Key::F(11), NONE, Press, "\x1b[23~"),
            (Key::F(12), NONE, Press, "\x1b[24~"),
            (Key::F(5), CTRL, Press, "\x1b[15;5~"),
            (Key::F(12), SHIFT, Press, "\x1b[24;2~"),
            (Key::F(13), NONE, Press, ""),
            (Key::Char('a'), NONE, Repeat, "a"),
            (Key::Char('a'), NONE, Release, ""),
        ],
    );
}

#[test]
fn application_cursor_keys_use_ss3() {
    check(
        &term(Profile::xterm(), b"\x1b[?1h"),
        &[
            (Key::Up, NONE, Press, "\x1bOA"),
            (Key::Left, NONE, Press, "\x1bOD"),
            (Key::Home, NONE, Press, "\x1bOH"),
            (Key::End, NONE, Press, "\x1bOF"),
            (Key::Up, CTRL, Press, "\x1b[1;5A"),
            (Key::Delete, NONE, Press, "\x1b[3~"),
        ],
    );
}

#[test]
fn modify_other_keys_level_2() {
    check(
        &term(Profile::xterm(), b"\x1b[>4;2m"),
        &[
            (Key::Char('a'), NONE, Press, "a"),
            (Key::Char('a'), SHIFT, Press, "A"),
            (Key::Char('a'), CTRL, Press, "\x1b[27;5;97~"),
            (Key::Char('a'), ALT, Press, "\x1b[27;3;97~"),
            (Key::Char('a'), CTRL | SHIFT, Press, "\x1b[27;6;65~"),
            (Key::Char('1'), CTRL, Press, "\x1b[27;5;49~"),
            (Key::Enter, SHIFT, Press, "\x1b[27;2;13~"),
            (Key::Enter, CTRL, Press, "\x1b[27;5;13~"),
            (Key::Tab, CTRL, Press, "\x1b[27;5;9~"),
            (Key::Backspace, CTRL, Press, "\x1b[27;5;127~"),
            (Key::Escape, ALT, Press, "\x1b[27;3;27~"),
            (Key::Up, CTRL, Press, "\x1b[1;5A"),
            (Key::F(5), SHIFT, Press, "\x1b[15;2~"),
        ],
    );
}

#[test]
fn modify_other_keys_level_1_only_where_a_modifier_would_be_lost() {
    check(
        &term(Profile::xterm(), b"\x1b[>4;1m"),
        &[
            (Key::Char('a'), CTRL, Press, "\x01"),
            (Key::Char('a'), ALT, Press, "\x1ba"),
            (Key::Char('1'), CTRL, Press, "\x1b[27;5;49~"),
            (Key::Char('a'), CTRL | SHIFT, Press, "\x1b[27;6;65~"),
            (Key::Enter, SHIFT, Press, "\x1b[27;2;13~"),
            (Key::Enter, ALT, Press, "\x1b\r"),
            (Key::Tab, SHIFT, Press, "\x1b[Z"),
        ],
    );
}

#[test]
fn modify_other_keys_only_where_the_terminal_has_it() {
    check(
        &term(Profile::konsole(), b"\x1b[>4;2m"),
        &[(Key::Char('a'), CTRL, Press, "\x01")],
    );
}

#[test]
fn kitty_without_flags_is_legacy() {
    check(
        &term(Profile::kitty(), b""),
        &[
            (Key::Escape, NONE, Press, "\x1b"),
            (Key::Char('a'), CTRL, Press, "\x01"),
            (Key::F(3), NONE, Press, "\x1bOR"),
        ],
    );
}

#[test]
fn kitty_disambiguate() {
    check(
        &term(Profile::kitty(), b"\x1b[>1u"),
        &[
            (Key::Char('a'), NONE, Press, "a"),
            (Key::Char('a'), SHIFT, Press, "A"),
            (Key::Char('a'), CTRL, Press, "\x1b[97;5u"),
            (Key::Char('a'), ALT, Press, "\x1b[97;3u"),
            (Key::Char('a'), CTRL | SHIFT, Press, "\x1b[97;6u"),
            (Key::Char('a'), SUPER, Press, "\x1b[97;9u"),
            (Key::Escape, NONE, Press, "\x1b[27u"),
            (Key::Escape, SHIFT, Press, "\x1b[27;2u"),
            (Key::Enter, NONE, Press, "\r"),
            (Key::Enter, SHIFT, Press, "\x1b[13;2u"),
            (Key::Tab, NONE, Press, "\t"),
            (Key::Tab, SHIFT, Press, "\x1b[9;2u"),
            (Key::Backspace, NONE, Press, "\x7f"),
            (Key::Backspace, CTRL, Press, "\x1b[127;5u"),
            (Key::Up, NONE, Press, "\x1b[A"),
            (Key::Up, CTRL, Press, "\x1b[1;5A"),
            (Key::Home, NONE, Press, "\x1b[H"),
            (Key::End, SHIFT, Press, "\x1b[1;2F"),
            (Key::Insert, NONE, Press, "\x1b[2~"),
            (Key::Delete, ALT, Press, "\x1b[3;3~"),
            (Key::F(1), NONE, Press, "\x1b[P"),
            (Key::F(2), NONE, Press, "\x1b[Q"),
            (Key::F(3), NONE, Press, "\x1b[13~"),
            (Key::F(3), CTRL, Press, "\x1b[13;5~"),
            (Key::F(4), NONE, Press, "\x1b[S"),
            (Key::F(5), NONE, Press, "\x1b[15~"),
            (Key::F(12), NONE, Press, "\x1b[24~"),
            (Key::F(13), NONE, Press, "\x1b[57376u"),
            (Key::F(35), NONE, Press, "\x1b[57398u"),
            (Key::Char('a'), NONE, Release, ""),
            (Key::Char('a'), CTRL, Repeat, "\x1b[97;5u"),
        ],
    );
}

#[test]
fn kitty_disambiguate_turns_off_application_cursor_keys() {
    check(
        &term(Profile::kitty(), b"\x1b[?1h\x1b[>1u"),
        &[(Key::Up, NONE, Press, "\x1b[A")],
    );
    check(
        &term(Profile::kitty(), b"\x1b[?1h\x1b[>8u"),
        &[(Key::Up, NONE, Press, "\x1bOA")],
    );
}

#[test]
fn kitty_report_event_types() {
    check(
        &term(Profile::kitty(), b"\x1b[>3u"),
        &[
            (Key::Char('a'), NONE, Press, "a"),
            (Key::Char('a'), NONE, Repeat, "a"),
            (Key::Char('a'), NONE, Release, "\x1b[97;1:3u"),
            (Key::Char('a'), CTRL, Repeat, "\x1b[97;5:2u"),
            (Key::Char('a'), SHIFT, Release, "\x1b[97;2:3u"),
            (Key::Up, NONE, Repeat, "\x1b[1;1:2A"),
            (Key::Up, NONE, Release, "\x1b[1;1:3A"),
            (Key::Enter, NONE, Release, ""),
            (Key::Escape, NONE, Release, "\x1b[27;1:3u"),
            (Key::F(5), NONE, Release, "\x1b[15;1:3~"),
        ],
    );
}

#[test]
fn kitty_report_event_types_alone() {
    check(
        &term(Profile::kitty(), b"\x1b[>2u"),
        &[
            (Key::Escape, NONE, Press, "\x1b"),
            (Key::Escape, NONE, Release, "\x1b[27;1:3u"),
            (Key::Char('a'), CTRL, Press, "\x01"),
            (Key::Char('a'), CTRL, Release, "\x1b[97;5:3u"),
        ],
    );
}

#[test]
fn kitty_report_all_keys() {
    check(
        &term(Profile::kitty(), b"\x1b[>9u"),
        &[
            (Key::Char('a'), NONE, Press, "\x1b[97u"),
            (Key::Char('a'), SHIFT, Press, "\x1b[97;2u"),
            (Key::Enter, NONE, Press, "\x1b[13u"),
            (Key::Tab, NONE, Press, "\x1b[9u"),
            (Key::Backspace, NONE, Press, "\x1b[127u"),
            (Key::Escape, NONE, Press, "\x1b[27u"),
            (Key::Up, NONE, Press, "\x1b[A"),
        ],
    );
}

#[test]
fn kitty_alternate_keys() {
    check(
        &term(Profile::kitty(), b"\x1b[>13u"),
        &[
            (Key::Char('a'), SHIFT, Press, "\x1b[97:65;2u"),
            (Key::Char('a'), NONE, Press, "\x1b[97u"),
            (Key::Char('1'), SHIFT, Press, "\x1b[49;2u"),
        ],
    );
}

#[test]
fn kitty_associated_text() {
    check(
        &term(Profile::kitty(), b"\x1b[>25u"),
        &[
            (Key::Char('a'), NONE, Press, "\x1b[97;;97u"),
            (Key::Char('a'), SHIFT, Press, "\x1b[97;2;65u"),
            (Key::Char('a'), CTRL, Press, "\x1b[97;5u"),
        ],
    );
}

#[test]
fn kitty_all_flags() {
    check(
        &term(Profile::kitty(), b"\x1b[>31u"),
        &[
            (Key::Char('a'), SHIFT, Press, "\x1b[97:65;2;65u"),
            (Key::Char('a'), NONE, Release, "\x1b[97;1:3u"),
            (Key::Enter, NONE, Release, "\x1b[13;1:3u"),
        ],
    );
}

#[test]
fn kitty_flags_follow_the_screen_in_use() {
    let term = term(Profile::kitty(), b"\x1b[>1u\x1b[?1049h");
    check(&term, &[(Key::Escape, NONE, Press, "\x1b")]);
}

#[test]
fn kitty_flags_only_where_the_terminal_has_the_protocol() {
    check(
        &term(Profile::xterm(), b"\x1b[>1u"),
        &[(Key::Escape, NONE, Press, "\x1b")],
    );
}

#[test]
fn windows_terminal_has_the_kitty_protocol() {
    check(
        &term(Profile::windows_terminal(), b"\x1b[>1u"),
        &[(Key::Enter, SHIFT, Press, "\x1b[13;2u")],
    );
}

#[test]
fn linux_console_keys() {
    check(
        &term(Profile::linux_console(), b""),
        &[
            (Key::F(1), NONE, Press, "\x1b[[A"),
            (Key::F(5), NONE, Press, "\x1b[[E"),
            (Key::F(6), NONE, Press, "\x1b[17~"),
            (Key::Home, NONE, Press, "\x1b[1~"),
            (Key::End, NONE, Press, "\x1b[4~"),
            (Key::Up, CTRL, Press, "\x1b[A"),
            (Key::Delete, SHIFT, Press, "\x1b[3~"),
            (Key::Char('a'), ALT, Press, "\x1ba"),
            (Key::Char('c'), CTRL, Press, "\x03"),
        ],
    );
}

#[test]
fn tmux_home_and_end() {
    check(
        &term(Profile::tmux_3_4(), b""),
        &[
            (Key::Home, NONE, Press, "\x1b[1~"),
            (Key::End, NONE, Press, "\x1b[4~"),
            (Key::Home, CTRL, Press, "\x1b[1;5H"),
            (Key::Up, NONE, Press, "\x1b[A"),
        ],
    );
}

fn mouse(term: &Emulator, kind: MouseKind, x: u16, y: u16, mods: Mods) -> Vec<u8> {
    term.encode_mouse(Mouse { kind, x, y, mods })
}

const LEFT: MouseButton = MouseButton::Left;

#[test]
fn no_mouse_reports_without_a_mode() {
    let term = term(Profile::xterm(), b"\x1b[?1006h");
    assert!(mouse(&term, MouseKind::Press(LEFT), 0, 0, NONE).is_empty());
}

#[test]
fn x10_encoded_reports() {
    let term = term(Profile::xterm(), b"\x1b[?1000h");
    assert_eq!(
        mouse(&term, MouseKind::Press(LEFT), 0, 0, NONE),
        b"\x1b[M !!"
    );
    assert_eq!(
        mouse(&term, MouseKind::Release(LEFT), 0, 0, NONE),
        b"\x1b[M#!!"
    );
    assert_eq!(
        mouse(&term, MouseKind::Press(MouseButton::Right), 1, 2, CTRL),
        b"\x1b[M2\"#"
    );
    assert_eq!(mouse(&term, MouseKind::ScrollUp, 2, 3, NONE), b"\x1b[M`#$");
    assert!(mouse(&term, MouseKind::Drag(LEFT), 0, 0, NONE).is_empty());
    assert!(mouse(&term, MouseKind::Move, 0, 0, NONE).is_empty());
    assert_eq!(
        mouse(&term, MouseKind::Press(LEFT), 222, 0, NONE),
        b"\x1b[M \xff!"
    );
    assert!(mouse(&term, MouseKind::Press(LEFT), 223, 0, NONE).is_empty());
}

#[test]
fn sgr_reports() {
    let term = term(Profile::xterm(), b"\x1b[?1000;1006h");
    assert_eq!(
        mouse(&term, MouseKind::Press(LEFT), 4, 9, NONE),
        b"\x1b[<0;5;10M"
    );
    assert_eq!(
        mouse(&term, MouseKind::Release(LEFT), 4, 9, NONE),
        b"\x1b[<0;5;10m"
    );
    assert_eq!(
        mouse(&term, MouseKind::Press(MouseButton::Middle), 0, 0, ALT),
        b"\x1b[<9;1;1M"
    );
    assert_eq!(
        mouse(&term, MouseKind::Press(MouseButton::Right), 0, 0, CTRL),
        b"\x1b[<18;1;1M"
    );
    assert_eq!(
        mouse(&term, MouseKind::ScrollDown, 0, 0, SHIFT),
        b"\x1b[<69;1;1M"
    );
    assert_eq!(
        mouse(&term, MouseKind::ScrollLeft, 0, 0, NONE),
        b"\x1b[<66;1;1M"
    );
    assert_eq!(
        mouse(&term, MouseKind::ScrollRight, 0, 0, NONE),
        b"\x1b[<67;1;1M"
    );
    assert_eq!(
        mouse(&term, MouseKind::Press(LEFT), 299, 299, NONE),
        b"\x1b[<0;300;300M"
    );
}

#[test]
fn drags_with_mode_1002_and_motion_with_1003() {
    let button = term(Profile::xterm(), b"\x1b[?1002;1006h");
    assert_eq!(
        mouse(&button, MouseKind::Drag(LEFT), 1, 1, NONE),
        b"\x1b[<32;2;2M"
    );
    assert!(mouse(&button, MouseKind::Move, 1, 1, NONE).is_empty());
    let any = term(Profile::xterm(), b"\x1b[?1003;1006h");
    assert_eq!(mouse(&any, MouseKind::Move, 0, 0, NONE), b"\x1b[<35;1;1M");
    assert_eq!(
        mouse(&any, MouseKind::Drag(MouseButton::Right), 0, 0, NONE),
        b"\x1b[<34;1;1M"
    );
}

#[test]
fn x10_mode_reports_presses_without_modifiers() {
    let term = term(Profile::xterm(), b"\x1b[?9h");
    assert_eq!(
        mouse(&term, MouseKind::Press(LEFT), 0, 0, SHIFT),
        b"\x1b[M !!"
    );
    assert!(mouse(&term, MouseKind::Release(LEFT), 0, 0, NONE).is_empty());
    assert!(mouse(&term, MouseKind::ScrollUp, 0, 0, NONE).is_empty());
}

#[test]
fn paste() {
    let plain = term(Profile::xterm(), b"");
    assert_eq!(plain.encode_paste("a\nb\r\nc"), b"a\rb\rc");
    let bracketed = term(Profile::xterm(), b"\x1b[?2004h");
    assert_eq!(bracketed.encode_paste("a\nb"), b"\x1b[200~a\rb\x1b[201~");
}

#[test]
fn focus() {
    assert!(term(Profile::xterm(), b"").encode_focus(true).is_empty());
    let term = term(Profile::xterm(), b"\x1b[?1004h");
    assert_eq!(term.encode_focus(true), b"\x1b[I");
    assert_eq!(term.encode_focus(false), b"\x1b[O");
}

#[test]
fn encode_key_is_a_press() {
    let term = term(Profile::kitty(), b"\x1b[>3u");
    assert_eq!(term.encode_key(Key::Char('a'), CTRL), b"\x1b[97;5u");
}
