//! The results crossterm gives and Arko's key map depends on, and crossterm's own decoding tests.
//! Where the two differ, the row says why.

use super::*;
use crate::event::records::{InputRecord, KeyRecord, control};
use crate::event::{MouseButton, MouseEvent, MouseEventKind};

const NONE: KeyModifiers = KeyModifiers::NONE;
const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
const CTRL: KeyModifiers = KeyModifiers::CONTROL;
const ALT: KeyModifiers = KeyModifiers::ALT;

#[test]
fn what_arko_depends_on() {
    check_table(&[
        // LF is Ctrl+J in raw mode, the new-line chord that works in every terminal.
        (b"\n", vec![key(KeyCode::Char('j'), CTRL)]),
        (b"\r", vec![key(KeyCode::Enter, NONE)]),
        // An ESC prefix is Alt (Alt+Enter, Alt+Backspace, Alt+B, F, D, Q and A).
        (b"\x1b\r", vec![key(KeyCode::Enter, ALT)]),
        (b"\x1b\x7f", vec![key(KeyCode::Backspace, ALT)]),
        (b"\x1bb", vec![key(KeyCode::Char('b'), ALT)]),
        (b"\x1bf", vec![key(KeyCode::Char('f'), ALT)]),
        (b"\x1bd", vec![key(KeyCode::Char('d'), ALT)]),
        (b"\x1bq", vec![key(KeyCode::Char('q'), ALT)]),
        (b"\x1ba", vec![key(KeyCode::Char('a'), ALT)]),
        // Shift+Tab is BackTab, legacy and kitty.
        (b"\x1b[Z", vec![key(KeyCode::BackTab, SHIFT)]),
        (b"\x1b[9;2u", vec![key(KeyCode::BackTab, SHIFT)]),
        // A typed capital is the capital with Shift.
        (b"A", vec![key(KeyCode::Char('A'), SHIFT)]),
        // Shift+Enter, which only the kitty protocol tells from Enter.
        (b"\x1b[13;2u", vec![key(KeyCode::Enter, SHIFT)]),
        // Ctrl+letters, and the cursor and word moves. 0x08 is Ctrl+H, which deletes a character,
        // unless VERASE says Backspace sends it.
        (b"\x03", vec![key(KeyCode::Char('c'), CTRL)]),
        (b"\x08", vec![key(KeyCode::Char('h'), CTRL)]),
        (b"\x04", vec![key(KeyCode::Char('d'), CTRL)]),
        (b"\x17", vec![key(KeyCode::Char('w'), CTRL)]),
        (b"\x1b[1;5D", vec![key(KeyCode::Left, CTRL)]),
        (b"\x1b[1;3C", vec![key(KeyCode::Right, ALT)]),
        (b"\x1b[1;2A", vec![key(KeyCode::Up, SHIFT)]),
        (b"\x1b[1;5H", vec![key(KeyCode::Home, CTRL)]),
        (b"\x1b[1;5F", vec![key(KeyCode::End, CTRL)]),
        (b"\x1b[5~", vec![key(KeyCode::PageUp, NONE)]),
        (b"\x1b[6~", vec![key(KeyCode::PageDown, NONE)]),
        (b"\x1b[3~", vec![key(KeyCode::Delete, NONE)]),
        (b"\x7f", vec![key(KeyCode::Backspace, NONE)]),
        // Esc, under the kitty protocol too.
        (b"\x1b", vec![key(KeyCode::Esc, NONE)]),
        (b"\x1b[27u", vec![key(KeyCode::Esc, NONE)]),
        // Focus.
        (b"\x1b[I", vec![event(Event::FocusGained)]),
        (b"\x1b[O", vec![event(Event::FocusLost)]),
    ]);
}

#[test]
fn a_lone_esc_is_esc_once_nothing_more_is_pending() {
    let mut decoder = Decoder::new(DecoderOptions::default());
    let mut out = Vec::new();
    decoder.feed(b"\x1b", &mut out);
    assert!(out.is_empty());
    assert!(decoder.has_pending());
    decoder.flush_pending(&mut out);
    assert_eq!(out, [key(KeyCode::Esc, NONE)]);
    assert!(!decoder.has_pending());
    // An ESC that a byte follows in the same read starts a sequence.
    out.clear();
    decoder.feed(b"\x1b[A", &mut out);
    assert_eq!(out, [key(KeyCode::Up, NONE)]);
}

#[test]
fn releases_arrive_for_the_program_to_skip() {
    let release = |code| key_kind(code, NONE, KeyEventKind::Release);
    assert_eq!(decode(b"\x1b[97;1:3u"), [release(KeyCode::Char('a'))]);
    let mut decoder = Decoder::new(DecoderOptions::default());
    let mut out = Vec::new();
    let record = |key_down| {
        InputRecord::Key(KeyRecord {
            key_down,
            repeat: 1,
            virtual_key: 0x0d,
            scan_code: 0x1c,
            unicode: 0x0d,
            control_state: 0,
        })
    };
    decoder.decode_records(&[record(true), record(false)], &mut out);
    assert_eq!(out, [key(KeyCode::Enter, NONE), release(KeyCode::Enter)]);
}

#[test]
fn windows_shift_enter_and_shift_tab() {
    let mut decoder = Decoder::new(DecoderOptions::default());
    let mut out = Vec::new();
    let record = |virtual_key, unicode| {
        InputRecord::Key(KeyRecord {
            key_down: true,
            repeat: 1,
            virtual_key,
            scan_code: 0,
            unicode,
            control_state: control::SHIFT_PRESSED,
        })
    };
    decoder.decode_records(&[record(0x0d, 0x0d), record(0x09, 0x09)], &mut out);
    assert_eq!(
        out,
        [key(KeyCode::Enter, SHIFT), key(KeyCode::BackTab, SHIFT)]
    );
}

/// crossterm's tests of its Unix parser, with the results it gives.
#[test]
fn crossterms_own_cases() {
    let mouse = |kind, column, row, modifiers| {
        event(Event::Mouse(MouseEvent {
            kind,
            column,
            row,
            modifiers,
        }))
    };
    let left_down = MouseEventKind::Down(MouseButton::Left);
    let left_up = MouseEventKind::Up(MouseButton::Left);
    check_table(&[
        (b"\x1b", vec![key(KeyCode::Esc, NONE)]),
        (b"\x1bc", vec![key(KeyCode::Char('c'), ALT)]),
        (b"\x1bH", vec![key(KeyCode::Char('H'), ALT | SHIFT)]),
        (b"\x1b\x14", vec![key(KeyCode::Char('t'), ALT | CTRL)]),
        (b"\t", vec![key(KeyCode::Tab, NONE)]),
        (b"\x1b[D", vec![key(KeyCode::Left, NONE)]),
        (b"\x1b[2D", vec![key(KeyCode::Left, SHIFT)]),
        (b"\x1b[3~", vec![key(KeyCode::Delete, NONE)]),
        (b"\x1b[3;2~", vec![key(KeyCode::Delete, SHIFT)]),
        (
            b"\x1b[200~on and on and on\x1b[201~",
            vec![event(Event::Paste("on and on and on".into()))],
        ),
        (b"\x1b[O", vec![event(Event::FocusLost)]),
        (b"\x1b[32;30;40;M", vec![mouse(left_down, 29, 39, NONE)]),
        (b"\x1b[M0\x60\x70", vec![mouse(left_down, 63, 79, CTRL)]),
        (b"\x1b[<0;20;10;M", vec![mouse(left_down, 19, 9, NONE)]),
        (b"\x1b[<0;20;10M", vec![mouse(left_down, 19, 9, NONE)]),
        (b"\x1b[<0;20;10;m", vec![mouse(left_up, 19, 9, NONE)]),
        (b"\x1b[<0;20;10m", vec![mouse(left_up, 19, 9, NONE)]),
        ("Ž".as_bytes(), vec![key(KeyCode::Char('Ž'), SHIFT)]),
        ("ñ".as_bytes(), vec![ch('ñ')]),
        (b"c", vec![ch('c')]),
        (b"C", vec![key(KeyCode::Char('C'), SHIFT)]),
        (b"\x1b[97u", vec![ch('a')]),
        // crossterm gives 'a' with Shift here and compares keys with their case folded, so its
        // test reads 'A'.
        (b"\x1b[97;2u", vec![key(KeyCode::Char('A'), SHIFT)]),
        (b"\x1b[97;7u", vec![key(KeyCode::Char('a'), ALT | CTRL)]),
        (b"\x1b[13u", vec![key(KeyCode::Enter, NONE)]),
        (b"\x1b[27u", vec![key(KeyCode::Esc, NONE)]),
        (b"\x1b[57358u", vec![key(KeyCode::CapsLock, NONE)]),
        (b"\x1b[57376u", vec![key(KeyCode::F(13), NONE)]),
        (b"\x1b[57399u", vec![ch('0')]),
        (b"\x1b[57419u", vec![key(KeyCode::Up, NONE)]),
        (
            b"\x1b[57449;3:3u",
            vec![key_kind(
                KeyCode::Modifier(crate::event::ModifierKey::RightAlt),
                ALT,
                KeyEventKind::Release,
            )],
        ),
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
    ]);
}

/// Where this decoder differs from crossterm on purpose.
#[test]
fn departures_from_crossterm() {
    check_table(&[
        // crossterm gives Esc for ESC ESC and loses an Alt-prefixed sequence.
        (b"\x1b\x1b", vec![key(KeyCode::Esc, ALT)]),
        (b"\x1b\x1b[A", vec![key(KeyCode::Up, ALT)]),
        // crossterm takes every `CSI r ; c R` for a cursor report.
        (b"\x1b[1;5R", vec![key(KeyCode::F(3), CTRL)]),
        // crossterm waits after `ESC [` and `ESC O` for more, and then drops both keys.
        (b"\x1b[", vec![key(KeyCode::Char('['), ALT)]),
        (b"\x1bO", vec![key(KeyCode::Char('O'), ALT | SHIFT)]),
        // crossterm drops these.
        (b"\x1b[E", vec![key(KeyCode::KeypadBegin, NONE)]),
        (b"\x1b[57427~", vec![key(KeyCode::KeypadBegin, NONE)]),
        // With the shifted key reported, crossterm drops Shift. A capital keeps it here, as when
        // it's typed.
        (b"\x1b[97:65;2u", vec![key(KeyCode::Char('A'), SHIFT)]),
    ]);
}
