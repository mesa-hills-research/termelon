//! Keys in the legacy encodings, from xterm's control sequences document, the Linux console and
//! xterm's modifyOtherKeys.

use super::*;

const NONE: KeyModifiers = KeyModifiers::NONE;
const SHIFT: KeyModifiers = KeyModifiers::SHIFT;
const CTRL: KeyModifiers = KeyModifiers::CONTROL;
const ALT: KeyModifiers = KeyModifiers::ALT;

#[test]
fn c0_controls() {
    let mut rows: Vec<(&[u8], Vec<Decoded>)> = vec![
        (b"\x00", vec![key(KeyCode::Char(' '), CTRL)]),
        (b"\x09", vec![key(KeyCode::Tab, NONE)]),
        (b"\x0a", vec![key(KeyCode::Char('j'), CTRL)]),
        (b"\x0d", vec![key(KeyCode::Enter, NONE)]),
        (b"\x7f", vec![key(KeyCode::Backspace, NONE)]),
        (b"\x08", vec![key(KeyCode::Char('h'), CTRL)]),
        (b"\x1c", vec![key(KeyCode::Char('4'), CTRL)]),
        (b"\x1d", vec![key(KeyCode::Char('5'), CTRL)]),
        (b"\x1e", vec![key(KeyCode::Char('6'), CTRL)]),
        (b"\x1f", vec![key(KeyCode::Char('7'), CTRL)]),
    ];
    let letters: Vec<[u8; 1]> = (1..=26u8)
        .filter(|&byte| ![0x09, 0x0a, 0x0d].contains(&byte))
        .map(|byte| [byte])
        .collect();
    for byte in &letters {
        let letter = char::from(b'a' + byte[0] - 1);
        rows.push((byte, vec![key(KeyCode::Char(letter), CTRL)]));
    }
    check_table(&rows);
}

#[test]
fn bs_is_backspace_only_where_verase_says_so() {
    let bs = DecoderOptions {
        erase: 0x08,
        ..DecoderOptions::default()
    };
    assert_eq!(
        decode_with(bs, b"\x08\x7f"),
        [key(KeyCode::Backspace, NONE), key(KeyCode::Backspace, NONE)]
    );
    let del = DecoderOptions {
        erase: 0x7f,
        ..DecoderOptions::default()
    };
    assert_eq!(
        decode_with(del, b"\x7f\x08"),
        [key(KeyCode::Backspace, NONE), key(KeyCode::Char('h'), CTRL)]
    );
    // An erase byte that is neither counts as DEL.
    let other = DecoderOptions {
        erase: 0xff,
        ..DecoderOptions::default()
    };
    assert_eq!(
        decode_with(other, b"\x7f\x08"),
        [key(KeyCode::Backspace, NONE), key(KeyCode::Char('h'), CTRL)]
    );
}

#[test]
fn characters() {
    check_table(&[
        (b"a", vec![ch('a')]),
        (b"A", vec![key(KeyCode::Char('A'), SHIFT)]),
        (b" ", vec![ch(' ')]),
        (b"~", vec![ch('~')]),
        (b"!", vec![ch('!')]),
        ("é".as_bytes(), vec![ch('é')]),
        ("Ž".as_bytes(), vec![key(KeyCode::Char('Ž'), SHIFT)]),
        ("€".as_bytes(), vec![ch('€')]),
        ("𐌼".as_bytes(), vec![ch('𐌼')]),
        ("日本".as_bytes(), vec![ch('日'), ch('本')]),
    ]);
}

#[test]
fn invalid_utf8_is_dropped_and_what_follows_decodes() {
    check_table(&[
        (&[0xc3, 0x28], vec![ch('(')]),
        (&[0xa0, 0xa1, b'x'], vec![ch('x')]),
        (&[0xe2, 0x28, 0xa1, b'y'], vec![ch('('), ch('y')]),
        (&[0xe2, 0x82, 0x28], vec![ch('(')]),
        (&[0xf0, 0x28, 0x8c, 0xbc], vec![ch('(')]),
        (&[0xf0, 0x90, 0x28, 0xbc], vec![ch('(')]),
        // Overlong, a surrogate, and past U+10FFFF.
        (&[0xe0, 0x80, 0xaf, b'z'], vec![ch('z')]),
        (&[0xed, 0xa0, 0x80, b'z'], vec![ch('z')]),
        (&[0xf4, 0x90, 0x80, 0x80, b'z'], vec![ch('z')]),
        (&[0xc0, 0xaf, b'z'], vec![ch('z')]),
        (&[0xff, b'z'], vec![ch('z')]),
    ]);
}

#[test]
fn a_character_split_across_reads_decodes_whole() {
    let bytes = "€".as_bytes();
    for split in 1..bytes.len() {
        let mut decoder = Decoder::new(DecoderOptions::default());
        let mut out = Vec::new();
        decoder.feed(&bytes[..split], &mut out);
        decoder.flush_pending(&mut out);
        assert!(out.is_empty(), "part of a character is kept, at {split}");
        decoder.feed(&bytes[split..], &mut out);
        assert_eq!(out, [ch('€')]);
    }
}

#[test]
fn escape_prefix_is_alt() {
    check_table(&[
        (b"\x1ba", vec![key(KeyCode::Char('a'), ALT)]),
        (b"\x1bA", vec![key(KeyCode::Char('A'), ALT | SHIFT)]),
        (b"\x1b\x14", vec![key(KeyCode::Char('t'), ALT | CTRL)]),
        (b"\x1b\x7f", vec![key(KeyCode::Backspace, ALT)]),
        (b"\x1b\x08", vec![key(KeyCode::Char('h'), ALT | CTRL)]),
        (b"\x1b\r", vec![key(KeyCode::Enter, ALT)]),
        (b"\x1b\t", vec![key(KeyCode::Tab, ALT)]),
        (b"\x1b\x00", vec![key(KeyCode::Char(' '), ALT | CTRL)]),
        (b"\x1b ", vec![key(KeyCode::Char(' '), ALT)]),
        (b"\x1b!", vec![key(KeyCode::Char('!'), ALT)]),
        (b"\x1b^", vec![key(KeyCode::Char('^'), ALT)]),
        (b"\x1bX", vec![key(KeyCode::Char('X'), ALT | SHIFT)]),
        ("\x1bé".as_bytes(), vec![key(KeyCode::Char('é'), ALT)]),
        (b"\x1b", vec![key(KeyCode::Esc, NONE)]),
        (b"\x1b\x1b", vec![key(KeyCode::Esc, ALT)]),
        (
            b"\x1b\x1b\x1b",
            vec![key(KeyCode::Esc, ALT), key(KeyCode::Esc, NONE)],
        ),
        (
            b"\x1b\x1ba",
            vec![key(KeyCode::Esc, NONE), key(KeyCode::Char('a'), ALT)],
        ),
        (b"\x1b\x1b[A", vec![key(KeyCode::Up, ALT)]),
        (b"\x1b\x1bOP", vec![key(KeyCode::F(1), ALT)]),
        (b"\x1b\x1b[1;5C", vec![key(KeyCode::Right, ALT | CTRL)]),
        (
            b"\x1b\x1b[I",
            vec![key(KeyCode::Esc, NONE), event(Event::FocusGained)],
        ),
    ]);
}

#[test]
fn introducers_with_nothing_after_them_are_alt_keys() {
    check_table(&[
        (b"\x1b[", vec![key(KeyCode::Char('['), ALT)]),
        (b"\x1bO", vec![key(KeyCode::Char('O'), ALT | SHIFT)]),
        (b"\x1b]", vec![key(KeyCode::Char(']'), ALT)]),
        (b"\x1bP", vec![key(KeyCode::Char('P'), ALT | SHIFT)]),
        (b"\x1b_", vec![key(KeyCode::Char('_'), ALT)]),
        (b"\x1b[[", vec![key(KeyCode::Char('['), ALT), ch('[')]),
        (
            b"\x1b\x1b[",
            vec![key(KeyCode::Esc, NONE), key(KeyCode::Char('['), ALT)],
        ),
    ]);
}

#[test]
fn introducers_a_key_follows_are_alt_keys() {
    check_table(&[
        (b"\x1b]x", vec![key(KeyCode::Char(']'), ALT), ch('x')]),
        (
            b"\x1bPp",
            vec![key(KeyCode::Char('P'), ALT | SHIFT), ch('p')],
        ),
        (b"\x1b_a", vec![key(KeyCode::Char('_'), ALT), ch('a')]),
        (
            b"\x1b[\r",
            vec![key(KeyCode::Char('['), ALT), key(KeyCode::Enter, NONE)],
        ),
        (
            b"\x1bO\x7f",
            vec![
                key(KeyCode::Char('O'), ALT | SHIFT),
                key(KeyCode::Backspace, NONE),
            ],
        ),
        (
            b"\x1b[[x",
            vec![key(KeyCode::Char('['), ALT), ch('['), ch('x')],
        ),
    ]);
}

#[test]
fn cursor_and_editing_keys() {
    check_table(&[
        (b"\x1b[A", vec![key(KeyCode::Up, NONE)]),
        (b"\x1b[B", vec![key(KeyCode::Down, NONE)]),
        (b"\x1b[C", vec![key(KeyCode::Right, NONE)]),
        (b"\x1b[D", vec![key(KeyCode::Left, NONE)]),
        (b"\x1b[H", vec![key(KeyCode::Home, NONE)]),
        (b"\x1b[F", vec![key(KeyCode::End, NONE)]),
        (b"\x1b[E", vec![key(KeyCode::KeypadBegin, NONE)]),
        (b"\x1bOA", vec![key(KeyCode::Up, NONE)]),
        (b"\x1bOB", vec![key(KeyCode::Down, NONE)]),
        (b"\x1bOC", vec![key(KeyCode::Right, NONE)]),
        (b"\x1bOD", vec![key(KeyCode::Left, NONE)]),
        (b"\x1bOH", vec![key(KeyCode::Home, NONE)]),
        (b"\x1bOF", vec![key(KeyCode::End, NONE)]),
        (b"\x1bOE", vec![key(KeyCode::KeypadBegin, NONE)]),
        (b"\x1b[Z", vec![key(KeyCode::BackTab, SHIFT)]),
        (b"\x1b[1;3Z", vec![key(KeyCode::BackTab, SHIFT | ALT)]),
        (b"\x1b[1~", vec![key(KeyCode::Home, NONE)]),
        (b"\x1b[2~", vec![key(KeyCode::Insert, NONE)]),
        (b"\x1b[3~", vec![key(KeyCode::Delete, NONE)]),
        (b"\x1b[4~", vec![key(KeyCode::End, NONE)]),
        (b"\x1b[5~", vec![key(KeyCode::PageUp, NONE)]),
        (b"\x1b[6~", vec![key(KeyCode::PageDown, NONE)]),
        (b"\x1b[7~", vec![key(KeyCode::Home, NONE)]),
        (b"\x1b[8~", vec![key(KeyCode::End, NONE)]),
        (b"\x1b[3;2~", vec![key(KeyCode::Delete, SHIFT)]),
        (b"\x1b[5;5~", vec![key(KeyCode::PageUp, CTRL)]),
        // The older form with the modifier as the only parameter.
        (b"\x1b[2D", vec![key(KeyCode::Left, SHIFT)]),
        (b"\x1b[5C", vec![key(KeyCode::Right, CTRL)]),
        // An empty first parameter.
        (b"\x1b[;5H", vec![key(KeyCode::Home, CTRL)]),
    ]);
}

#[test]
fn function_keys() {
    let tilde = [
        (11, 1),
        (12, 2),
        (13, 3),
        (14, 4),
        (15, 5),
        (17, 6),
        (18, 7),
        (19, 8),
        (20, 9),
        (21, 10),
        (23, 11),
        (24, 12),
        (25, 13),
        (26, 14),
        (28, 15),
        (29, 16),
        (31, 17),
        (32, 18),
        (33, 19),
        (34, 20),
    ];
    let mut rows: Vec<(Vec<u8>, Vec<Decoded>)> = Vec::new();
    for (code, n) in tilde {
        rows.push((
            format!("\x1b[{code}~").into_bytes(),
            vec![key(KeyCode::F(n), NONE)],
        ));
        rows.push((
            format!("\x1b[{code};5~").into_bytes(),
            vec![key(KeyCode::F(n), CTRL)],
        ));
    }
    for (letter, n) in [('P', 1), ('Q', 2), ('R', 3), ('S', 4)] {
        rows.push((
            format!("\x1bO{letter}").into_bytes(),
            vec![key(KeyCode::F(n), NONE)],
        ));
        rows.push((
            format!("\x1b[1;2{letter}").into_bytes(),
            vec![key(KeyCode::F(n), SHIFT)],
        ));
        // As some terminals send modified F1 to F4.
        rows.push((
            format!("\x1bO5{letter}").into_bytes(),
            vec![key(KeyCode::F(n), CTRL)],
        ));
    }
    // The kitty protocol drops the 1 when no modifier is held, and F3 has no such form.
    rows.push((b"\x1b[P".to_vec(), vec![key(KeyCode::F(1), NONE)]));
    rows.push((b"\x1b[Q".to_vec(), vec![key(KeyCode::F(2), NONE)]));
    rows.push((b"\x1b[S".to_vec(), vec![key(KeyCode::F(4), NONE)]));
    let rows: Vec<(&[u8], Vec<Decoded>)> = rows
        .iter()
        .map(|(bytes, expected)| (bytes.as_slice(), expected.clone()))
        .collect();
    check_table(&rows);
}

#[test]
fn linux_console_function_keys() {
    check_table(&[
        (b"\x1b[[A", vec![key(KeyCode::F(1), NONE)]),
        (b"\x1b[[B", vec![key(KeyCode::F(2), NONE)]),
        (b"\x1b[[C", vec![key(KeyCode::F(3), NONE)]),
        (b"\x1b[[D", vec![key(KeyCode::F(4), NONE)]),
        (b"\x1b[[E", vec![key(KeyCode::F(5), NONE)]),
        (b"\x1b\x1b[[A", vec![key(KeyCode::F(1), ALT)]),
    ]);
}

#[test]
fn keypad_in_application_mode() {
    check_table(&[
        (b"\x1bOM", vec![key(KeyCode::Enter, NONE)]),
        (b"\x1bOX", vec![ch('=')]),
        (b"\x1bOj", vec![ch('*')]),
        (b"\x1bOk", vec![ch('+')]),
        (b"\x1bOl", vec![ch(',')]),
        (b"\x1bOm", vec![ch('-')]),
        (b"\x1bOn", vec![ch('.')]),
        (b"\x1bOo", vec![ch('/')]),
        (b"\x1bOp", vec![ch('0')]),
        (b"\x1bOy", vec![ch('9')]),
    ]);
}

#[test]
fn every_modifier_parameter() {
    for param in 2..=64 {
        let expected = modifier_param(param);
        check_table(&[
            (
                format!("\x1b[1;{param}A").as_bytes(),
                vec![key(KeyCode::Up, expected)],
            ),
            (
                format!("\x1b[3;{param}~").as_bytes(),
                vec![key(KeyCode::Delete, expected)],
            ),
            (
                format!("\x1b[27;{param};13~").as_bytes(),
                vec![key(KeyCode::Enter, expected)],
            ),
        ]);
    }
}

#[test]
fn unknown_sequences_are_dropped() {
    check_table(&[
        (b"\x1b[99~x", vec![ch('x')]),
        (b"\x1b[1;5Yx", vec![ch('x')]),
        (b"\x1b[0nx", vec![ch('x')]),
        (b"\x1bOzx", vec![ch('x')]),
        (b"\x1b[1;999Ax", vec![ch('x')]),
        (b"\x1b[1 qx", vec![ch('x')]),
        (b"\x1b[?1;2;3$x", vec![]),
        (b"\x1b[1?Ax", vec![ch('x')]),
    ]);
}

#[test]
fn a_sequence_a_control_cuts_short_is_dropped_and_the_control_read() {
    check_table(&[
        (b"\x1b[1;5\rx", vec![key(KeyCode::Enter, NONE), ch('x')]),
        (b"\x1b[1;\x1b[A", vec![key(KeyCode::Up, NONE)]),
        (b"\x1bO5\x03", vec![key(KeyCode::Char('c'), CTRL)]),
    ]);
}

#[test]
fn modify_other_keys() {
    check_table(&[
        (b"\x1b[27;5;13~", vec![key(KeyCode::Enter, CTRL)]),
        (b"\x1b[27;2;13~", vec![key(KeyCode::Enter, SHIFT)]),
        (b"\x1b[27;3;13~", vec![key(KeyCode::Enter, ALT)]),
        (b"\x1b[27;2;9~", vec![key(KeyCode::BackTab, SHIFT)]),
        (b"\x1b[27;5;9~", vec![key(KeyCode::Tab, CTRL)]),
        (b"\x1b[27;5;127~", vec![key(KeyCode::Backspace, CTRL)]),
        (b"\x1b[27;3;27~", vec![key(KeyCode::Esc, ALT)]),
        (b"\x1b[27;5;32~", vec![key(KeyCode::Char(' '), CTRL)]),
        (b"\x1b[27;5;97~", vec![key(KeyCode::Char('a'), CTRL)]),
        (
            b"\x1b[27;6;65~",
            vec![key(KeyCode::Char('a'), CTRL | SHIFT)],
        ),
        // With Ctrl, a letter stays lowercase whichever the terminal sent, as crossterm has it.
        (
            b"\x1b[27;6;97~",
            vec![key(KeyCode::Char('a'), CTRL | SHIFT)],
        ),
        (b"\x1b[27;4;65~", vec![key(KeyCode::Char('a'), ALT | SHIFT)]),
        (b"\x1b[27;5;49~", vec![key(KeyCode::Char('1'), CTRL)]),
        (b"\x1b[27;7;59~", vec![key(KeyCode::Char(';'), CTRL | ALT)]),
        // formatOtherKeys 1 sends the same as the kitty protocol's CSI u.
        (b"\x1b[13;5u", vec![key(KeyCode::Enter, CTRL)]),
        // Missing the character, or a control code.
        (b"\x1b[27;5~x", vec![ch('x')]),
        (b"\x1b[27;5;1~x", vec![ch('x')]),
    ]);
}
