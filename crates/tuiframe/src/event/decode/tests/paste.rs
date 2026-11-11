//! Bracketed paste.

use super::*;

fn paste(text: &str) -> Decoded {
    event(Event::Paste(text.to_owned()))
}

#[test]
fn a_paste_is_one_event_with_its_line_breaks_as_lf() {
    check_table(&[
        (b"\x1b[200~hello\x1b[201~", vec![paste("hello")]),
        (b"\x1b[200~\x1b[201~", vec![paste("")]),
        (b"\x1b[200~a\rb\r\nc\nd\x1b[201~", vec![paste("a\nb\nc\nd")]),
        (b"\x1b[200~a\r\r\nb\r\x1b[201~", vec![paste("a\n\nb\n")]),
        (
            "\x1b[200~日本\t€\x1b[201~".as_bytes(),
            vec![paste("日本\t€")],
        ),
        // What looks like keys inside it is text.
        (
            b"\x1b[200~\x1b[A\x03\x1b[200~\x1b[20\x1b[201~",
            vec![paste("\x1b[A\x03\x1b[200~\x1b[20")],
        ),
        // Bytes that aren't UTF-8 become U+FFFD.
        (b"\x1b[200~a\xffb\x1b[201~", vec![paste("a\u{fffd}b")]),
        (b"x\x1b[200~p\x1b[201~y", vec![ch('x'), paste("p"), ch('y')]),
    ]);
}

#[test]
fn a_paste_end_with_no_start_is_dropped() {
    check_table(&[(b"\x1b[201~x", vec![ch('x')])]);
}

#[test]
fn a_paste_split_at_every_offset_decodes_whole() {
    let bytes = b"a\x1b[200~one\r\ntwo\x1b[2\x1b[201~b";
    let expected = [ch('a'), paste("one\ntwo\x1b[2"), ch('b')];
    for split in 0..=bytes.len() {
        let mut decoder = Decoder::new(DecoderOptions::default());
        let mut out = Vec::new();
        decoder.feed(&bytes[..split], &mut out);
        decoder.feed(&bytes[split..], &mut out);
        decoder.flush_pending(&mut out);
        assert_eq!(out, expected, "split at {split}");
    }
}

#[test]
fn a_paste_split_at_every_offset_decodes_whole_with_a_flush_between_reads() {
    // The reader flushes when a read leaves nothing more ready, which only an ESC or `ESC [` with
    // nothing after them turns into keys, so every split past the first two bytes of the start
    // decodes whole. (Splits there need the escape timeout.)
    let bytes = b"\x1b[200~one\r\ntwo\x1b[201~";
    let expected = [paste("one\ntwo")];
    for split in 3..=bytes.len() {
        let mut decoder = Decoder::new(DecoderOptions::default());
        let mut out = Vec::new();
        decoder.feed(&bytes[..split], &mut out);
        decoder.flush_pending(&mut out);
        assert!(!decoder.has_pending(), "split at {split}");
        decoder.feed(&bytes[split..], &mut out);
        decoder.flush_pending(&mut out);
        assert_eq!(out, expected, "split at {split}");
    }
}

#[test]
fn a_paste_fed_a_byte_at_a_time_decodes_whole() {
    let bytes = b"\x1b[200~line\rnext\x1b[201~";
    let mut decoder = Decoder::new(DecoderOptions::default());
    let mut out = Vec::new();
    for (i, byte) in bytes.iter().enumerate() {
        decoder.feed(std::slice::from_ref(byte), &mut out);
        if i >= 2 {
            decoder.flush_pending(&mut out);
        }
    }
    assert_eq!(out, [paste("line\nnext")]);
}

#[test]
fn a_large_paste_is_kept_whole_and_its_memory_let_go() {
    let body = "x".repeat(1 << 20);
    let bytes = format!("\x1b[200~{body}\x1b[201~");
    let mut decoder = Decoder::new(DecoderOptions::default());
    let mut out = Vec::new();
    decoder.feed(bytes.as_bytes(), &mut out);
    assert_eq!(out, [paste(&body)]);
    assert!(decoder.buffer_capacity() <= tokenizer::MAX_STRING);
}

#[test]
fn an_esc_before_a_paste_is_the_esc_key() {
    check_table(&[(
        b"\x1b\x1b[200~p\x1b[201~",
        vec![key(KeyCode::Esc, KeyModifiers::NONE), paste("p")],
    )]);
}
