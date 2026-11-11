//! Round trips through tuiframe-vt's encoders. Each profile's emulator, in each mode a program can
//! set, encodes every key with every modifier, and the decoder gives back the key. Where an
//! encoding can't carry a modifier or an event type, the expectation drops it, as `legacy` spells
//! out.

use tuiframe_vt::{Emulator, Key, KeyEncoding, KeyKind, Mods, Mouse, MouseKind, Profile};

use super::*;
use crate::event::{MouseButton, MouseEvent, MouseEventKind};

const NONE: KeyModifiers = KeyModifiers::NONE;
const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
const CTRL: KeyModifiers = KeyModifiers::CONTROL;
const ALT: KeyModifiers = KeyModifiers::ALT;

/// The kitty protocol's flags.
const DISAMBIGUATE: u8 = 1;
const REPORT_EVENTS: u8 = 2;
const ALL_KEYS: u8 = 8;

fn profiles() -> [Profile; 6] {
    [
        Profile::xterm(),
        Profile::kitty(),
        Profile::konsole(),
        Profile::windows_terminal(),
        Profile::tmux_3_4(),
        Profile::linux_console(),
    ]
}

/// An emulator playing `profile`, after a program sent it `setup`.
fn emulator(profile: &Profile, setup: &[u8]) -> Emulator {
    let mut emulator = Emulator::with_profile(profile.clone(), 80, 24);
    emulator.feed(setup).unwrap();
    emulator
}

/// What a program can set that changes how the profile encodes keys. Every profile has the legacy
/// encodings with and without application cursor keys, xterm modifyOtherKeys at both levels, and
/// the kitty protocol's terminals every combination of its flags.
fn key_modes(profile: &Profile) -> Vec<String> {
    let mut modes = vec![String::new(), "\x1b[?1h".to_owned()];
    if profile.modify_other_keys {
        modes.extend(["\x1b[>4;1m".to_owned(), "\x1b[>4;2m".to_owned()]);
    }
    if profile.kitty_keyboard {
        for flags in 1..=31 {
            modes.push(format!("\x1b[>{flags}u"));
            modes.push(format!("\x1b[?1h\x1b[>{flags}u"));
        }
    }
    modes
}

fn keys() -> Vec<Key> {
    let mut keys: Vec<Key> = ('a'..='z')
        .chain('0'..='9')
        .chain(" `-=[]\\;',./@^_~?éф".chars())
        .map(Key::Char)
        .collect();
    keys.extend([
        Key::Enter,
        Key::Tab,
        Key::Backspace,
        Key::Escape,
        Key::Up,
        Key::Down,
        Key::Left,
        Key::Right,
        Key::Home,
        Key::End,
        Key::PageUp,
        Key::PageDown,
        Key::Insert,
        Key::Delete,
    ]);
    keys.extend((1..=35).map(Key::F));
    keys
}

/// Every combination of Shift, Alt, Ctrl and Super.
fn all_mods() -> Vec<Mods> {
    (0..16)
        .map(|bits| {
            [Mods::SHIFT, Mods::ALT, Mods::CTRL, Mods::SUPER]
                .into_iter()
                .enumerate()
                .filter(|(i, _)| bits & (1 << i) != 0)
                .fold(Mods::NONE, |all, (_, m)| all | m)
        })
        .collect()
}

fn modifiers(mods: Mods) -> KeyModifiers {
    [
        (Mods::SHIFT, SHIFT),
        (Mods::ALT, ALT),
        (Mods::CTRL, CTRL),
        (Mods::SUPER, KeyModifiers::SUPER),
    ]
    .into_iter()
    .filter(|(m, _)| mods.contains(*m))
    .fold(NONE, |all, (_, m)| all | m)
}

/// The character a key types, a letter's capital with Shift.
fn typed(c: char, mods: Mods) -> char {
    let mut upper = c.to_uppercase();
    match (mods.contains(Mods::SHIFT), upper.next(), upper.next()) {
        (true, Some(up), None) => up,
        _ => c,
    }
}

/// The key code a key decodes to when nothing is lost.
fn code(key: Key, mods: Mods) -> KeyCode {
    match key {
        Key::Char(c) => KeyCode::Char(typed(c, mods)),
        Key::Enter => KeyCode::Enter,
        Key::Tab if mods.contains(Mods::SHIFT) => KeyCode::BackTab,
        Key::Tab => KeyCode::Tab,
        Key::Backspace => KeyCode::Backspace,
        Key::Escape => KeyCode::Esc,
        Key::Up => KeyCode::Up,
        Key::Down => KeyCode::Down,
        Key::Left => KeyCode::Left,
        Key::Right => KeyCode::Right,
        Key::Home => KeyCode::Home,
        Key::End => KeyCode::End,
        Key::PageUp => KeyCode::PageUp,
        Key::PageDown => KeyCode::PageDown,
        Key::Insert => KeyCode::Insert,
        Key::Delete => KeyCode::Delete,
        Key::F(n) => KeyCode::F(n),
    }
}

/// The C0 byte xterm sends for Ctrl with a character.
fn control_byte(c: char) -> Option<u8> {
    match c {
        'a'..='z' => u8::try_from(u32::from(c) - u32::from('a') + 1).ok(),
        '@' | ' ' | '2' => Some(0x00),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' | '~' => Some(0x1e),
        '_' | '7' | '/' => Some(0x1f),
        '8' | '?' => Some(0x7f),
        _ => None,
    }
}

/// The key a C0 byte reads as.
fn control_key(byte: u8) -> (KeyCode, KeyModifiers) {
    match byte {
        0x00 => (KeyCode::Char(' '), CTRL),
        0x09 => (KeyCode::Tab, NONE),
        0x0d => (KeyCode::Enter, NONE),
        0x1b => (KeyCode::Esc, NONE),
        0x7f => (KeyCode::Backspace, NONE),
        0x1c..=0x1f => (KeyCode::Char(char::from(b'4' + byte - 0x1c)), CTRL),
        _ => (KeyCode::Char(char::from(b'a' + byte - 1)), CTRL),
    }
}

/// What a key sent in its legacy bytes, with no modifier parameter, decodes to. Alt survives as
/// the ESC prefix, and Shift as a capital. The special keys keep no modifier (they have none here,
/// but on the Linux console, which drops them). Ctrl survives on the characters that have a control
/// code, and Ctrl+Backspace is 0x08, Ctrl+H.
fn legacy(key: Key, mods: Mods) -> Decoded {
    let alt = if mods.contains(Mods::ALT) { ALT } else { NONE };
    let (code, held) = match key {
        Key::Enter => (KeyCode::Enter, alt),
        Key::Tab if mods.contains(Mods::SHIFT) => (KeyCode::BackTab, SHIFT | alt),
        Key::Tab => (KeyCode::Tab, alt),
        Key::Backspace if mods.contains(Mods::CTRL) => (KeyCode::Char('h'), CTRL | alt),
        Key::Backspace => (KeyCode::Backspace, alt),
        Key::Escape => (KeyCode::Esc, alt),
        Key::Char(c) => match control_byte(c) {
            Some(byte) if mods.contains(Mods::CTRL) => {
                let (code, held) = control_key(byte);
                (code, held | alt)
            }
            _ => {
                let typed = typed(c, mods);
                let shift = if typed.is_uppercase() { SHIFT } else { NONE };
                (KeyCode::Char(typed), shift | alt)
            }
        },
        other => (code(other, Mods::NONE), NONE),
    };
    super::key(code, held)
}

/// Whether the bytes carry the modifiers as a parameter, kitty's `CSI ... u`, a `CSI 1 ; m X`, a
/// `CSI n ; m ~` or modifyOtherKeys, so nothing is lost.
fn parameterized(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x1b[") && (bytes.contains(&b';') || bytes.ends_with(b"u"))
}

/// Whether the terminal sends nothing for the key. Without the kitty protocol's flag 2 it sends no
/// release, and where Enter, Tab and Backspace keep their legacy bytes they send none either.
/// Without the kitty protocol, F13 and up send nothing.
fn sends_nothing(flags: u8, key: Key, mods: Mods, kind: KeyKind) -> bool {
    let legacy_keys = matches!(key, Key::Enter | Key::Tab | Key::Backspace)
        && flags & ALL_KEYS == 0
        && (flags & DISAMBIGUATE == 0 || mods.is_empty());
    match kind {
        KeyKind::Release => flags & REPORT_EVENTS == 0 || legacy_keys,
        _ => flags == 0 && matches!(key, Key::F(n) if n > 12),
    }
}

#[test]
fn every_key_and_modifier_under_each_profile() {
    let mut checked = 0;
    for profile in profiles() {
        for setup in key_modes(&profile) {
            let emulator = emulator(&profile, setup.as_bytes());
            let flags = if profile.kitty_keyboard {
                emulator.keyboard_flags()
            } else {
                0
            };
            for key in keys() {
                for mods in all_mods() {
                    for kind in [KeyKind::Press, KeyKind::Repeat, KeyKind::Release] {
                        let bytes = emulator.encode_key_event(key, mods, kind);
                        let context = || {
                            format!(
                                "{} after {setup:?}, {key:?} with {mods:?}, {kind:?}, sent {:?}",
                                profile.name,
                                String::from_utf8_lossy(&bytes)
                            )
                        };
                        let decoded = decode(&bytes);
                        assert_eq!(
                            bytes.is_empty(),
                            sends_nothing(flags, key, mods, kind),
                            "{}",
                            context()
                        );
                        if bytes.is_empty() {
                            continue;
                        }
                        let expected = if parameterized(&bytes) {
                            let kind = match (flags & REPORT_EVENTS != 0, kind) {
                                (true, KeyKind::Repeat) => KeyEventKind::Repeat,
                                (true, KeyKind::Release) => KeyEventKind::Release,
                                _ => KeyEventKind::Press,
                            };
                            let event =
                                KeyEvent::new_with_kind(code(key, mods), modifiers(mods), kind);
                            Decoded::Event(Event::Key(event))
                        } else {
                            legacy(key, mods)
                        };
                        assert_eq!(decoded, [expected], "{}", context());
                        checked += 1;
                    }
                }
            }
        }
    }
    assert!(checked > 100_000, "{checked}");
}

#[test]
fn the_linux_console_drops_modifiers_on_special_keys() {
    let emulator = emulator(&Profile::linux_console(), b"");
    assert_eq!(emulator.profile().keys, KeyEncoding::LinuxConsole);
    assert_eq!(
        decode(&emulator.encode_key(Key::F(1), Mods::CTRL)),
        [key(KeyCode::F(1), NONE)]
    );
    assert_eq!(
        decode(&emulator.encode_key(Key::Up, Mods::SHIFT)),
        [key(KeyCode::Up, NONE)]
    );
}

#[test]
fn every_mouse_report_under_each_profile() {
    use MouseKind::{Drag, Move, Press, Release, ScrollDown, ScrollLeft, ScrollRight, ScrollUp};
    use tuiframe_vt::MouseButton as Button;
    let button = |b| match b {
        Button::Left => MouseButton::Left,
        Button::Middle => MouseButton::Middle,
        Button::Right => MouseButton::Right,
    };
    let mut kinds = vec![Move, ScrollUp, ScrollDown, ScrollLeft, ScrollRight];
    for b in [Button::Left, Button::Middle, Button::Right] {
        kinds.extend([Press(b), Release(b), Drag(b)]);
    }
    let places = [(0, 0), (7, 3), (221, 222), (222, 222), (223, 5), (500, 300)];
    let mods = [
        Mods::NONE,
        Mods::SHIFT,
        Mods::ALT,
        Mods::CTRL,
        Mods::SHIFT | Mods::ALT | Mods::CTRL,
        Mods::SUPER,
    ];
    let mut checked = 0;
    for profile in profiles() {
        for tracking in [9, 1000, 1002, 1003] {
            for sgr in [false, true] {
                if !profile.has_mode(tracking) || (sgr && !profile.has_mode(1006)) {
                    continue;
                }
                let setup = format!("\x1b[?{tracking}h{}", if sgr { "\x1b[?1006h" } else { "" });
                let emulator = emulator(&profile, setup.as_bytes());
                for kind in &kinds {
                    for &(x, y) in &places {
                        for held in mods {
                            let mouse = Mouse {
                                kind: *kind,
                                x,
                                y,
                                mods: held,
                            };
                            let bytes = emulator.encode_mouse(mouse);
                            let reported = match tracking {
                                9 => matches!(kind, Press(_)),
                                1000 => !matches!(kind, Drag(_) | Move),
                                1002 => *kind != Move,
                                _ => true,
                            };
                            let fits = sgr || (x <= 222 && y <= 222);
                            let context = format!(
                                "{} after {setup:?}, {mouse:?}, sent {:?}",
                                profile.name,
                                String::from_utf8_lossy(&bytes)
                            );
                            assert_eq!(bytes.is_empty(), !(reported && fits), "{context}");
                            if bytes.is_empty() {
                                continue;
                            }
                            let expected_kind = match *kind {
                                Press(b) => MouseEventKind::Down(button(b)),
                                // X10 reports don't say which button came up.
                                Release(b) if sgr => MouseEventKind::Up(button(b)),
                                Release(_) => MouseEventKind::Up(MouseButton::Left),
                                Drag(b) => MouseEventKind::Drag(button(b)),
                                Move => MouseEventKind::Moved,
                                ScrollUp => MouseEventKind::ScrollUp,
                                ScrollDown => MouseEventKind::ScrollDown,
                                ScrollLeft => MouseEventKind::ScrollLeft,
                                ScrollRight => MouseEventKind::ScrollRight,
                            };
                            // Only Shift, Alt and Ctrl are reported, and none in mode 9.
                            let modifiers = if tracking == 9 {
                                NONE
                            } else {
                                modifiers(held) - KeyModifiers::SUPER
                            };
                            let expected = event(Event::Mouse(MouseEvent {
                                kind: expected_kind,
                                column: x,
                                row: y,
                                modifiers,
                            }));
                            assert_eq!(decode(&bytes), [expected], "{context}");
                            checked += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 1_000, "{checked}");
}

#[test]
fn pastes_and_focus_under_each_profile() {
    let texts = [
        "hello",
        "",
        "two\nlines",
        "dos\r\nline ends",
        "old\rmac",
        "日本語 and emoji 😀",
        "keys inside \x1b[A\x03 stay text",
        "a\n\nb\n",
    ];
    for profile in profiles() {
        let emulator = emulator(&profile, b"\x1b[?2004h\x1b[?1004h");
        for text in texts {
            let bytes = emulator.encode_paste(text);
            let decoded = decode(&bytes);
            let lf = text.replace("\r\n", "\n").replace('\r', "\n");
            if profile.has_mode(2004) {
                assert_eq!(decoded, [event(Event::Paste(lf))], "{}", profile.name);
            } else if !text.contains('\x1b') {
                // Without bracketed paste, the text arrives as the keys that type it.
                let keys: Vec<Decoded> = lf
                    .chars()
                    .map(|c| match c {
                        '\n' => key(KeyCode::Enter, NONE),
                        c => decode(c.to_string().as_bytes()).remove(0),
                    })
                    .collect();
                assert_eq!(decoded, keys, "{}", profile.name);
            }
        }
        let gained = decode(&emulator.encode_focus(true));
        let lost = decode(&emulator.encode_focus(false));
        if profile.has_mode(1004) {
            assert_eq!(gained, [event(Event::FocusGained)], "{}", profile.name);
            assert_eq!(lost, [event(Event::FocusLost)], "{}", profile.name);
        } else {
            assert!(gained.is_empty() && lost.is_empty(), "{}", profile.name);
        }
    }
}
