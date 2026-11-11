//! Properties over random bytes. No input panics, splitting the input across reads changes
//! nothing, the output and the held state stay within bounds, and decoding recovers after garbage.

use proptest::prelude::*;

use super::tokenizer::MAX_STRING;
use super::*;

/// Bytes weighted toward the ones that start, continue and end sequences, so the random input
/// reaches every state.
fn noisy_bytes(max_len: usize) -> impl Strategy<Value = Vec<u8>> {
    let interesting = prop::sample::select(
        b"\x1b\x1b\x1b[[]PO_\\;;:0125789<?>$ ~uMmRcyt\x07\x18\x1a\r\n\x7f\x08\xc3\xe2\xf0\x80\xbf\x9b\xff"
            .to_vec(),
    );
    prop::collection::vec(prop_oneof![3 => interesting, 1 => any::<u8>()], 0..max_len)
}

/// Sequences that decode to known events, to follow the garbage.
const KNOWN: &[&[u8]] = &[
    b"a",
    b"\x1b[A",
    b"\x1b[1;5C",
    b"\x1bOP",
    b"\x1b[15;2~",
    b"\x1b[97;5u",
    b"\x1b[<0;3;4M",
    b"\x1b[200~p\r\n\x1b[201~",
    b"\x1b[I",
    "€".as_bytes(),
];

fn feed_all(decoder: &mut Decoder, bytes: &[u8]) -> Vec<Decoded> {
    let mut out = Vec::new();
    decoder.feed(bytes, &mut out);
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn no_input_panics(bytes in noisy_bytes(512), flushes in prop::collection::vec(any::<bool>(), 0..32)) {
        let mut decoder = Decoder::new(DecoderOptions::default());
        let mut out = Vec::new();
        let chunk = bytes.len() / flushes.len().max(1) + 1;
        for (piece, flush) in bytes.chunks(chunk).zip(flushes.iter().chain(std::iter::repeat(&true))) {
            decoder.feed(piece, &mut out);
            if *flush {
                decoder.flush_pending(&mut out);
                prop_assert!(!decoder.has_pending());
            }
        }
        decoder.flush_pending(&mut out);
        prop_assert!(!decoder.has_pending());
    }

    #[test]
    fn splitting_the_input_changes_nothing(bytes in noisy_bytes(256), cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..8)) {
        let whole = feed_all(&mut Decoder::new(DecoderOptions::default()), &bytes);
        let mut points: Vec<usize> = cuts.iter().map(|cut| cut.index(bytes.len() + 1)).collect();
        points.sort_unstable();
        let mut decoder = Decoder::new(DecoderOptions::default());
        let mut out = Vec::new();
        let mut start = 0;
        for point in points.into_iter().chain([bytes.len()]) {
            decoder.feed(&bytes[start..point], &mut out);
            start = point;
        }
        prop_assert_eq!(out, whole);
    }

    #[test]
    fn output_and_held_state_stay_within_bounds(bytes in noisy_bytes(2048)) {
        let mut decoder = Decoder::new(DecoderOptions::default());
        let mut out = Vec::new();
        decoder.feed(&bytes, &mut out);
        decoder.flush_pending(&mut out);
        // No byte decodes to more than one event or reply.
        prop_assert!(out.len() <= bytes.len());
        // Ending any paste lets its memory go, and strings stop at the bound.
        decoder.feed(b"\x1b[201~", &mut out);
        prop_assert!(decoder.buffer_capacity() <= MAX_STRING);
    }

    #[test]
    fn decoding_recovers_after_garbage(garbage in noisy_bytes(512), known in prop::sample::select(KNOWN)) {
        let expected = decode(known);
        prop_assert!(!expected.is_empty());
        // A paste's end and then CAN, which ends every sequence and string.
        let mut bytes = garbage;
        bytes.extend_from_slice(b"\x1b[201~\x18");
        bytes.extend_from_slice(known);
        let decoded = decode(&bytes);
        prop_assert!(decoded.ends_with(&expected), "{:?} ended {:?}", known, decoded);
    }
}
