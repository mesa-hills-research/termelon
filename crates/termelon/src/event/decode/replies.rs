//! The terminal's answers to the probe's queries, and the colour scheme reports.

use super::tokenizer::Csi;
use super::{ColorSlot, ModeValue, Reply};
use crate::caps::Rgb;
use crate::event::{Event, Scheme};

fn u16_of(value: u32) -> u16 {
    u16::try_from(value).unwrap_or(u16::MAX)
}

/// A reply in a control sequence, other than the cursor report.
pub(super) fn csi(csi: &Csi) -> Option<Reply> {
    let params = &csi.params;
    match (csi.private, csi.intermediates(), csi.final_byte) {
        (Some(b'?'), [], b'c') => Some(Reply::Da1 {
            attributes: params.values().flatten().map(u16_of).collect(),
        }),
        (Some(b'>'), [], b'c') => Some(Reply::Da2 {
            kind: u16_of(params.value(0).unwrap_or(0)),
            version: params.value(1).unwrap_or(0),
        }),
        (private @ (Some(b'?') | None), [b'$'], b'y') => Some(Reply::ModeReport {
            mode: u16_of(params.value(0)?),
            private: private.is_some(),
            value: match params.value(1).unwrap_or(0) {
                0 => ModeValue::NotRecognized,
                1 => ModeValue::Set,
                2 => ModeValue::Reset,
                3 => ModeValue::PermanentlySet,
                4 => ModeValue::PermanentlyReset,
                _ => return None,
            },
        }),
        (Some(b'?'), [], b'u') => Some(Reply::KittyKeyboard(
            u8::try_from(params.value(0).unwrap_or(0)).ok()?,
        )),
        (None, [], b't') if params.value(0) == Some(8) => Some(Reply::TerminalSize {
            rows: u16_of(params.value(1)?),
            columns: u16_of(params.value(2)?),
        }),
        _ => None,
    }
}

/// `CSI row ; column R`, as 0-based.
pub(super) fn cursor_position(csi: &Csi) -> Reply {
    let at = |n| u16_of(csi.params.value(n).unwrap_or(1).saturating_sub(1));
    Reply::CursorPosition {
        row: at(0),
        column: at(1),
    }
}

/// `CSI ? 997 ; 1 n` for a dark theme and `; 2 n` for a light one.
pub(super) fn color_scheme(csi: &Csi) -> Option<Event> {
    if csi.params.value(0) != Some(997) {
        return None;
    }
    match csi.params.value(1)? {
        1 => Some(Event::ColorScheme(Scheme::Dark)),
        2 => Some(Event::ColorScheme(Scheme::Light)),
        _ => None,
    }
}

/// A DCS string, XTVERSION's `> |` or XTGETTCAP's `1 + r` and `0 + r`.
pub(super) fn dcs(header: &Csi, data: &[u8]) -> Vec<Reply> {
    let params = &header.params;
    match (header.private, header.intermediates(), header.final_byte) {
        (Some(b'>'), [], b'|') if params.count() == 0 => {
            vec![Reply::XtVersion(String::from_utf8_lossy(data).into_owned())]
        }
        (None, [b'+'], b'r') => {
            let found = match params.value(0) {
                Some(1) => true,
                Some(0) => false,
                _ => return Vec::new(),
            };
            data.split(|&byte| byte == b';')
                .filter_map(|entry| xtgettcap(entry, found))
                .collect()
        }
        _ => Vec::new(),
    }
}

/// One `name=value` of XTGETTCAP, in hex. A capability without a value, such as a flag, has an
/// empty one, and a name the terminal doesn't know has none.
fn xtgettcap(entry: &[u8], found: bool) -> Option<Reply> {
    let mut parts = entry.splitn(2, |&byte| byte == b'=');
    let name = from_hex(parts.next()?)?;
    if name.is_empty() {
        return None;
    }
    let value = match (found, parts.next()) {
        (false, _) => None,
        (true, Some(value)) => Some(from_hex(value)?),
        (true, None) => Some(String::new()),
    };
    Some(Reply::XtGetTcap { name, value })
}

/// Text written as pairs of hex digits.
fn from_hex(hex: &[u8]) -> Option<String> {
    let (pairs, rest) = hex.as_chunks::<2>();
    if !rest.is_empty() {
        return None;
    }
    let bytes = pairs
        .iter()
        .map(|&[high, low]| Some(hex_digit(high)? << 4 | hex_digit(low)?))
        .collect::<Option<Vec<u8>>>()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn hex_digit(byte: u8) -> Option<u8> {
    char::from(byte)
        .to_digit(16)
        .and_then(|digit| u8::try_from(digit).ok())
}

/// An OSC string, OSC 10 and 11 with colours. `OSC 10 ; a ; b` answers for 10 and then 11, as
/// xterm reads it.
pub(super) fn osc(data: &[u8]) -> Vec<Reply> {
    let mut parts = data.split(|&byte| byte == b';');
    let first = match parts.next() {
        Some(b"10") => 10,
        Some(b"11") => 11,
        _ => return Vec::new(),
    };
    parts
        .zip(first..)
        .filter_map(|(spec, n)| {
            let which = match n {
                10 => ColorSlot::Foreground,
                11 => ColorSlot::Background,
                _ => return None,
            };
            Some(Reply::Color {
                which,
                rgb: color(spec)?,
            })
        })
        .collect()
}

/// A colour in the forms terminals answer with, `rgb:r/g/b` and `rgba:r/g/b/a` with 1 to 4 hex
/// digits a channel, scaled to 8 bits, and `#rgb` with 1 to 4 digits a channel, which are its
/// high bits.
pub(super) fn color(spec: &[u8]) -> Option<Rgb> {
    let lower = spec.to_ascii_lowercase();
    if let Some(hex) = lower.strip_prefix(b"#") {
        let digits = hex.len() / 3;
        if !hex.len().is_multiple_of(3) || !(1..=4).contains(&digits) {
            return None;
        }
        let mut channels = hex.chunks_exact(digits).map(|channel| {
            let value = hex_value(channel)?;
            // The digits are the channel's high bits, so `#f00` is red 0xf0.
            Some(match digits {
                1 => value << 4,
                2 => value,
                3 => value >> 4,
                _ => value >> 8,
            })
        });
        let mut next = || channels.next().flatten().and_then(|v| u8::try_from(v).ok());
        return Some(Rgb::new(next()?, next()?, next()?));
    }
    let body = lower
        .strip_prefix(b"rgba:")
        .or_else(|| lower.strip_prefix(b"rgb:"))?;
    let channels: Vec<&[u8]> = body.split(|&byte| byte == b'/').collect();
    let wanted = if lower.starts_with(b"rgba:") { 4 } else { 3 };
    if channels.len() != wanted {
        return None;
    }
    let mut scaled = channels.iter().map(|channel| scale(channel));
    let mut next = || scaled.next().flatten();
    Some(Rgb::new(next()?, next()?, next()?))
}

/// A channel of 1 to 4 hex digits, scaled to 8 bits with rounding.
fn scale(channel: &[u8]) -> Option<u8> {
    if !(1..=4).contains(&channel.len()) {
        return None;
    }
    let value = hex_value(channel)?;
    let bits = u32::try_from(channel.len()).ok()? * 4;
    let max = (1 << bits) - 1;
    u8::try_from((value * 255 * 2 + max) / (max * 2)).ok()
}

fn hex_value(digits: &[u8]) -> Option<u32> {
    if digits.is_empty() || digits.len() > 4 {
        return None;
    }
    digits.iter().try_fold(0, |value, &byte| {
        Some(value << 4 | u32::from(hex_digit(byte)?))
    })
}
