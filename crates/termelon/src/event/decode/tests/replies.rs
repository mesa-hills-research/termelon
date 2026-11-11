//! Replies to the probe's queries, focus reports and the colour scheme reports.

use super::*;
use crate::event::Scheme;

fn reply(reply: Reply) -> Decoded {
    Decoded::Reply(reply)
}

fn color(which: ColorSlot, r: u8, g: u8, b: u8) -> Decoded {
    reply(Reply::Color {
        which,
        rgb: Rgb::new(r, g, b),
    })
}

fn tcap(name: &str, value: Option<&str>) -> Decoded {
    reply(Reply::XtGetTcap {
        name: name.to_owned(),
        value: value.map(str::to_owned),
    })
}

#[test]
fn device_attributes() {
    check_table(&[
        (
            b"\x1b[?62;22c",
            vec![reply(Reply::Da1 {
                attributes: vec![62, 22],
            })],
        ),
        (
            b"\x1b[?65;1;9c",
            vec![reply(Reply::Da1 {
                attributes: vec![65, 1, 9],
            })],
        ),
        // The Linux console's.
        (
            b"\x1b[?6c",
            vec![reply(Reply::Da1 {
                attributes: vec![6],
            })],
        ),
        (
            b"\x1b[>1;4000;29c",
            vec![reply(Reply::Da2 {
                kind: 1,
                version: 4000,
            })],
        ),
        (
            b"\x1b[>0;276;0c",
            vec![reply(Reply::Da2 {
                kind: 0,
                version: 276,
            })],
        ),
        (
            b"\x1b[>65;7600;1c",
            vec![reply(Reply::Da2 {
                kind: 65,
                version: 7600,
            })],
        ),
        (
            b"\x1b[>84;0;0c",
            vec![reply(Reply::Da2 {
                kind: 84,
                version: 0,
            })],
        ),
    ]);
}

#[test]
fn mode_reports() {
    let mode = |mode, private, value| {
        vec![reply(Reply::ModeReport {
            mode,
            private,
            value,
        })]
    };
    check_table(&[
        (
            b"\x1b[?2026;0$y",
            mode(2026, true, ModeValue::NotRecognized),
        ),
        (b"\x1b[?2026;1$y", mode(2026, true, ModeValue::Set)),
        (b"\x1b[?2026;2$y", mode(2026, true, ModeValue::Reset)),
        (
            b"\x1b[?2004;3$y",
            mode(2004, true, ModeValue::PermanentlySet),
        ),
        (
            b"\x1b[?1004;4$y",
            mode(1004, true, ModeValue::PermanentlyReset),
        ),
        (b"\x1b[4;2$y", mode(4, false, ModeValue::Reset)),
        (b"\x1b[?2026;5$yx", vec![ch('x')]),
    ]);
}

#[test]
fn kitty_keyboard_flags() {
    check_table(&[
        (b"\x1b[?0u", vec![reply(Reply::KittyKeyboard(0))]),
        (b"\x1b[?1u", vec![reply(Reply::KittyKeyboard(1))]),
        (b"\x1b[?31u", vec![reply(Reply::KittyKeyboard(31))]),
        (b"\x1b[?999ux", vec![ch('x')]),
    ]);
}

#[test]
fn xtversion() {
    check_table(&[
        (
            b"\x1bP>|kitty(0.39.1)\x1b\\",
            vec![reply(Reply::XtVersion("kitty(0.39.1)".into()))],
        ),
        (
            b"\x1bP>|XTerm(390)\x1b\\",
            vec![reply(Reply::XtVersion("XTerm(390)".into()))],
        ),
        (
            b"\x1bP>|tmux 3.4\x1b\\",
            vec![reply(Reply::XtVersion("tmux 3.4".into()))],
        ),
        (
            b"\x1bP>|WezTerm 20240203-110809-5046fc22\x1b\\",
            vec![reply(Reply::XtVersion(
                "WezTerm 20240203-110809-5046fc22".into(),
            ))],
        ),
        // Ended with BEL.
        (
            b"\x1bP>|foot(1.25.0)\x07",
            vec![reply(Reply::XtVersion("foot(1.25.0)".into()))],
        ),
    ]);
}

#[test]
fn xtgettcap() {
    check_table(&[
        (b"\x1bP1+r5463\x1b\\", vec![tcap("Tc", Some(""))]),
        (b"\x1bP0+r5463\x1b\\", vec![tcap("Tc", None)]),
        (
            b"\x1bP1+r544e=787465726d2d6b69747479\x1b\\",
            vec![tcap("TN", Some("xterm-kitty"))],
        ),
        (b"\x1bP1+r524742=38\x1b\\", vec![tcap("RGB", Some("8"))]),
        (
            b"\x1bP1+r544E=78;5463\x1b\\",
            vec![tcap("TN", Some("x")), tcap("Tc", Some(""))],
        ),
        // Odd or non-hex digits.
        (b"\x1bP1+r546\x1b\\x", vec![ch('x')]),
        (b"\x1bP1+r54zz\x1b\\x", vec![ch('x')]),
        // Other strings are dropped.
        (b"\x1bP1$r0m\x1b\\x", vec![ch('x')]),
        (b"\x1bP!|00000000\x1b\\x", vec![ch('x')]),
    ]);
}

#[test]
fn colours_in_every_form() {
    use ColorSlot::{Background, Foreground};
    check_table(&[
        (
            b"\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\",
            vec![color(Background, 30, 30, 46)],
        ),
        (
            b"\x1b]11;rgb:1e1e/1e1e/2e2e\x07",
            vec![color(Background, 30, 30, 46)],
        ),
        (
            b"\x1b]10;rgb:ffff/ffff/ffff\x1b\\",
            vec![color(Foreground, 255, 255, 255)],
        ),
        (
            b"\x1b]11;rgb:8080/0000/ffff\x1b\\",
            vec![color(Background, 128, 0, 255)],
        ),
        (
            b"\x1b]11;rgb:ff/80/00\x1b\\",
            vec![color(Background, 255, 128, 0)],
        ),
        (
            b"\x1b]11;rgb:f/8/0\x1b\\",
            vec![color(Background, 255, 136, 0)],
        ),
        (
            b"\x1b]11;rgb:fff/800/000\x1b\\",
            vec![color(Background, 255, 128, 0)],
        ),
        (
            b"\x1b]11;RGB:FFFF/0000/0000\x1b\\",
            vec![color(Background, 255, 0, 0)],
        ),
        (
            b"\x1b]11;rgba:0000/ffff/0000/c8c8\x1b\\",
            vec![color(Background, 0, 255, 0)],
        ),
        (
            b"\x1b]11;#1e1e2e\x1b\\",
            vec![color(Background, 30, 30, 46)],
        ),
        (b"\x1b]11;#f80\x1b\\", vec![color(Background, 240, 128, 0)]),
        (
            b"\x1b]11;#fff800000\x1b\\",
            vec![color(Background, 255, 128, 0)],
        ),
        (
            b"\x1b]11;#ffff80000000\x1b\\",
            vec![color(Background, 255, 128, 0)],
        ),
        // Both at once, as xterm reads `OSC 10 ; ? ; ?`.
        (
            b"\x1b]10;rgb:ffff/ffff/ffff;rgb:0000/0000/0000\x1b\\",
            vec![color(Foreground, 255, 255, 255), color(Background, 0, 0, 0)],
        ),
        // Malformed colours and other OSC strings are dropped.
        (b"\x1b]11;rgb:ff/ff\x1b\\x", vec![ch('x')]),
        (b"\x1b]11;rgb:fffff/0/0\x1b\\x", vec![ch('x')]),
        (b"\x1b]11;rgb:gg/00/00\x1b\\x", vec![ch('x')]),
        (b"\x1b]11;#12345\x1b\\x", vec![ch('x')]),
        (b"\x1b]11;?\x1b\\x", vec![ch('x')]),
        (b"\x1b]52;c;aGk=\x1b\\x", vec![ch('x')]),
        (b"\x1b]4;1;rgb:ff/00/00\x1b\\x", vec![ch('x')]),
    ]);
}

#[test]
fn terminal_size_and_colour_scheme() {
    check_table(&[
        (
            b"\x1b[8;24;80t",
            vec![reply(Reply::TerminalSize {
                rows: 24,
                columns: 80,
            })],
        ),
        (b"\x1b[4;600;800tx", vec![ch('x')]),
        (b"\x1b[8;24tx", vec![ch('x')]),
        (
            b"\x1b[?997;1n",
            vec![event(Event::ColorScheme(Scheme::Dark))],
        ),
        (
            b"\x1b[?997;2n",
            vec![event(Event::ColorScheme(Scheme::Light))],
        ),
        (b"\x1b[?997;3nx", vec![ch('x')]),
        (b"\x1b[?996;1nx", vec![ch('x')]),
    ]);
}

#[test]
fn focus() {
    check_table(&[
        (b"\x1b[I", vec![event(Event::FocusGained)]),
        (b"\x1b[O", vec![event(Event::FocusLost)]),
        (b"\x1b[1Ix", vec![ch('x')]),
    ]);
}

#[test]
fn a_cursor_report_is_one_only_while_one_is_expected() {
    let mut decoder = Decoder::new(DecoderOptions::default());
    let mut out = Vec::new();
    // Not expected, `CSI 1 ; 5 R` is Ctrl+F3.
    decoder.feed(b"\x1b[1;5R", &mut out);
    assert_eq!(out, [key(KeyCode::F(3), KeyModifiers::CONTROL)]);
    out.clear();
    decoder.expect(Query::CursorPosition);
    decoder.feed(b"\x1b[1;5R", &mut out);
    assert_eq!(out, [reply(Reply::CursorPosition { row: 0, column: 4 })]);
    out.clear();
    // The expectation was used up.
    decoder.feed(b"\x1b[1;5R", &mut out);
    assert_eq!(out, [key(KeyCode::F(3), KeyModifiers::CONTROL)]);
}

#[test]
fn each_expected_cursor_report_uses_one_up() {
    let mut decoder = Decoder::new(DecoderOptions::default());
    let mut out = Vec::new();
    for _ in 0..3 {
        decoder.expect(Query::CursorPosition);
    }
    decoder.feed(b"\x1b[3;7R\x1b[3;9R\x1b[1;2R\x1b[1;2R", &mut out);
    assert_eq!(
        out,
        [
            reply(Reply::CursorPosition { row: 2, column: 6 }),
            reply(Reply::CursorPosition { row: 2, column: 8 }),
            reply(Reply::CursorPosition { row: 0, column: 1 }),
            key(KeyCode::F(3), KeyModifiers::SHIFT),
        ]
    );
}

#[test]
fn a_da1_reply_clears_the_expected_cursor_reports() {
    let mut decoder = Decoder::new(DecoderOptions::default());
    let mut out = Vec::new();
    decoder.expect(Query::CursorPosition);
    decoder.expect(Query::CursorPosition);
    decoder.feed(b"\x1b[?62c\x1b[1;5R", &mut out);
    assert_eq!(
        out,
        [
            reply(Reply::Da1 {
                attributes: vec![62]
            }),
            key(KeyCode::F(3), KeyModifiers::CONTROL),
        ]
    );
}

#[test]
fn a_late_cursor_report_is_dropped_rather_than_typed() {
    // A row other than 1 is no key, so a report that came after the probe stopped waiting goes.
    check_table(&[
        (b"\x1b[24;80Rx", vec![ch('x')]),
        (b"\x1b[2;1Rx", vec![ch('x')]),
    ]);
}

#[test]
fn late_replies_are_replies_never_keys() {
    // The reader drops the replies nobody waits for, so none reaches the program as a key.
    let late: &[&[u8]] = &[
        b"\x1b[?62;22c",
        b"\x1b[>1;4000;29c",
        b"\x1b[?2026;2$y",
        b"\x1b[?1u",
        b"\x1bP>|kitty(0.39.1)\x1b\\",
        b"\x1bP1+r5463\x1b\\",
        b"\x1b]11;rgb:0000/0000/0000\x1b\\",
        b"\x1b[8;24;80t",
    ];
    for bytes in late {
        let decoded = decode(bytes);
        assert_eq!(decoded.len(), 1, "{:?}", String::from_utf8_lossy(bytes));
        assert!(
            matches!(decoded[0], Decoded::Reply(_)),
            "{:?} gave {decoded:?}",
            String::from_utf8_lossy(bytes)
        );
    }
}

#[test]
fn replies_split_across_reads_decode_whole() {
    let bytes = b"\x1b]11;rgb:1e1e/1e1e/2e2e\x1b\\\x1bP>|kitty(0.39.1)\x1b\\\x1b[?62c";
    let expected = decode(bytes);
    assert_eq!(expected.len(), 3);
    let mut pending = 0;
    for split in 0..=bytes.len() {
        let mut decoder = Decoder::new(DecoderOptions::default());
        let mut out = Vec::new();
        decoder.feed(&bytes[..split], &mut out);
        if decoder.has_pending() {
            // An ESC or an introducer ends the read, which the escape timeout is for.
            pending += 1;
            continue;
        }
        decoder.flush_pending(&mut out);
        decoder.feed(&bytes[split..], &mut out);
        decoder.flush_pending(&mut out);
        assert_eq!(out, expected, "split at {split}");
    }
    // After each of the three ESCs that start a reply, and after `ESC ]`, `ESC P` and `ESC [`.
    assert_eq!(pending, 6);
}

/// A probe that stopped waiting cancels what it expected, so a cursor report that comes later is
/// no reply.
#[test]
fn cancelled_expectations_make_no_reply() {
    let mut decoder = Decoder::new(DecoderOptions::default());
    let mut out = Vec::new();
    decoder.expect(Query::CursorPosition);
    decoder.expect(Query::CursorPosition);
    decoder.cancel_expected();
    decoder.feed(b"\x1b[1;5R", &mut out);
    assert_eq!(out, [key(KeyCode::F(3), KeyModifiers::CONTROL)]);
}
