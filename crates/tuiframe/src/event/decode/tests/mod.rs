//! The decoder's tests, a file for each encoding.

mod keys_kitty;
mod keys_legacy;
mod mouse;
mod parity;
mod paste;
mod properties;
mod replies;
mod round_trips;
mod tokens;
mod windows;

use super::*;
use crate::event::KeyEventKind;

/// Everything `bytes` decode to, fed in one read and then flushed, as the reader does when no more
/// input is ready.
pub(super) fn decode(bytes: &[u8]) -> Vec<Decoded> {
    decode_with(DecoderOptions::default(), bytes)
}

/// [`decode`] with options.
pub(super) fn decode_with(options: DecoderOptions, bytes: &[u8]) -> Vec<Decoded> {
    let mut decoder = Decoder::new(options);
    let mut out = Vec::new();
    decoder.feed(bytes, &mut out);
    decoder.flush_pending(&mut out);
    out
}

/// A key press, decoded.
pub(super) fn key(code: KeyCode, modifiers: KeyModifiers) -> Decoded {
    Decoded::Event(Event::Key(KeyEvent::new(code, modifiers)))
}

/// A key with an event type, decoded.
pub(super) fn key_kind(code: KeyCode, modifiers: KeyModifiers, kind: KeyEventKind) -> Decoded {
    Decoded::Event(Event::Key(KeyEvent::new_with_kind(code, modifiers, kind)))
}

/// A character typed with no modifiers, decoded.
pub(super) fn ch(c: char) -> Decoded {
    key(KeyCode::Char(c), KeyModifiers::NONE)
}

/// An event, decoded.
pub(super) fn event(event: Event) -> Decoded {
    Decoded::Event(event)
}

/// The modifiers in a modifier parameter, from the bits as xterm and the kitty protocol list them,
/// written out independently of the decoder.
pub(super) fn modifier_param(param: u32) -> KeyModifiers {
    let bits = param - 1;
    let table = [
        (1, KeyModifiers::SHIFT),
        (2, KeyModifiers::ALT),
        (4, KeyModifiers::CONTROL),
        (8, KeyModifiers::SUPER),
        (16, KeyModifiers::HYPER),
        (32, KeyModifiers::META),
    ];
    table
        .iter()
        .filter(|(bit, _)| bits & bit != 0)
        .fold(KeyModifiers::NONE, |all, &(_, m)| all | m)
}

/// Checks each row of a table, naming the input of the first that fails.
#[track_caller]
pub(super) fn check_table(rows: &[(&[u8], Vec<Decoded>)]) {
    for (bytes, expected) in rows {
        assert_eq!(
            &decode(bytes),
            expected,
            "decoding {:?}",
            String::from_utf8_lossy(bytes)
        );
    }
}
