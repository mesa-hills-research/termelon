//! The tokenizer's states and bounds.

use super::tokenizer::{Csi, MAX_PARAMS, MAX_STRING, Token, Tokenizer};
use super::*;

/// The control sequences `bytes` give, as owned copies.
fn sequences(bytes: &[u8]) -> Vec<Csi> {
    let mut tokenizer = Tokenizer::new();
    let mut found = Vec::new();
    let mut emit = |token: Token<'_>| {
        if let Token::Csi { csi, .. } = token {
            found.push(csi.clone());
        }
    };
    for &byte in bytes {
        tokenizer.advance(byte, &mut emit);
    }
    found
}

#[test]
fn parameters_split_into_groups_and_sub_parameters() {
    let found = sequences(b"\x1b[1:2:3;;4:5;:6u");
    let [csi] = found.as_slice() else {
        panic!("{found:?}");
    };
    let params = &csi.params;
    assert_eq!(params.count(), 4);
    assert_eq!(params.group(0), [Some(1), Some(2), Some(3)]);
    assert_eq!(params.group(1), [None]);
    assert_eq!(params.group(2), [Some(4), Some(5)]);
    assert_eq!(params.group(3), [None, Some(6)]);
    assert_eq!(params.group(4), []);
    assert_eq!(params.value(1), None);
    assert_eq!(params.sub(2, 1), Some(5));
    assert_eq!(csi.final_byte, b'u');
}

#[test]
fn private_markers_and_intermediates() {
    let found = sequences(b"\x1b[?2026;2$y\x1b[>c\x1b[2 q\x1b[1$$!p");
    assert_eq!(found.len(), 3, "{found:?}");
    assert_eq!(found[0].private, Some(b'?'));
    assert_eq!(found[0].intermediates(), b"$");
    assert_eq!(found[1].private, Some(b'>'));
    assert_eq!(found[1].params.count(), 0);
    assert_eq!(found[2].intermediates(), b" ");
    // Three intermediates are more than a sequence keeps, so it's dropped.
}

#[test]
fn parameters_stop_at_the_bound() {
    let many: Vec<String> = (1..=40).map(|n| n.to_string()).collect();
    let bytes = format!("\x1b[?{}c", many.join(";"));
    let decoded = decode(bytes.as_bytes());
    let expected: Vec<u16> = (1..=32).collect();
    assert_eq!(MAX_PARAMS, 32);
    assert_eq!(
        decoded,
        [Decoded::Reply(Reply::Da1 {
            attributes: expected
        })]
    );
}

#[test]
fn a_huge_parameter_saturates() {
    let found = sequences(b"\x1b[99999999999999999999;5A");
    assert_eq!(found[0].params.value(0), Some(u32::MAX));
    assert_eq!(decode(b"\x1b[99999999999999999999~x"), [ch('x')]);
}

#[test]
fn strings_stop_at_the_bound() {
    let at_bound = format!("\x1bP>|{}\x1b\\", "v".repeat(MAX_STRING));
    let decoded = decode(at_bound.as_bytes());
    assert!(
        matches!(&decoded[..], [Decoded::Reply(Reply::XtVersion(text))] if text.len() == MAX_STRING)
    );
    let past = format!("\x1bP>|{}\x1b\\x", "v".repeat(MAX_STRING + 1));
    assert_eq!(decode(past.as_bytes()), [ch('x')]);
}

#[test]
fn a_long_string_holds_no_more_than_the_bound() {
    let mut decoder = Decoder::new(DecoderOptions::default());
    let mut out = Vec::new();
    decoder.feed(b"\x1b]11;", &mut out);
    for _ in 0..64 {
        decoder.feed(&[b'0'; 16 * 1024], &mut out);
        assert!(decoder.buffer_capacity() <= MAX_STRING);
    }
    decoder.feed(b"\x07x", &mut out);
    assert_eq!(out, [ch('x')]);
}

#[test]
fn strings_end_at_bel_st_an_esc_can_and_sub() {
    let black = Decoded::Reply(Reply::Color {
        which: ColorSlot::Background,
        rgb: Rgb::new(0, 0, 0),
    });
    check_table(&[
        (b"\x1b]11;rgb:0/0/0\x07", vec![black.clone()]),
        (b"\x1b]11;rgb:0/0/0\x1b\\", vec![black.clone()]),
        // An ESC that starts something else ends the string unfinished, and is read.
        (
            b"\x1b]11;rgb:0/0/0\x1b[A",
            vec![key(KeyCode::Up, KeyModifiers::NONE)],
        ),
        (b"\x1b]11;rgb:0/0/0\x18x", vec![ch('x')]),
        (b"\x1b]11;rgb:0/0/0\x1ax", vec![ch('x')]),
        // Other controls inside are dropped.
        (b"\x1b]11;rgb:\r0/0\n/0\x07", vec![black]),
        // APC strings, such as kitty's graphics replies, are dropped.
        (b"\x1b_Gi=1;OK\x1b\\x", vec![ch('x')]),
        // A DCS with a malformed header is read to its end and dropped.
        (b"\x1bP1+?r5463\x1b\\x", vec![ch('x')]),
    ]);
}

#[test]
fn has_pending_is_an_esc_or_an_introducer_at_the_end() {
    let pending: &[&[u8]] = &[
        b"\x1b",
        b"\x1b\x1b",
        b"\x1b[",
        b"\x1b[[",
        b"\x1b[M",
        b"\x1b[M!!",
        b"\x1bO",
        b"\x1b]",
        b"\x1bP",
        b"\x1b_",
        b"a\x1b",
    ];
    let held: &[&[u8]] = &[
        b"",
        b"a",
        b"\x1b[1",
        b"\x1b[1;5",
        b"\x1b[?",
        b"\x1bO5",
        b"\x1b]11;rgb:",
        b"\x1b]11;rgb:0/0/0\x1b",
        b"\x1bP>|kitty",
        b"\x1b[200~",
        b"\x1b[200~text\x1b[20",
        &[0xe2, 0x82],
    ];
    for (bytes, expected) in pending
        .iter()
        .map(|b| (b, true))
        .chain(held.iter().map(|b| (b, false)))
    {
        let mut decoder = Decoder::new(DecoderOptions::default());
        let mut out = Vec::new();
        decoder.feed(bytes, &mut out);
        assert_eq!(
            decoder.has_pending(),
            expected,
            "{:?}",
            String::from_utf8_lossy(bytes)
        );
        decoder.flush_pending(&mut out);
        assert!(!decoder.has_pending());
    }
}
