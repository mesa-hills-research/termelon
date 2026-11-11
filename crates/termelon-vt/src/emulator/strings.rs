//! OSC and DCS strings.

use std::fmt::Write as _;
use std::sync::Arc;

use super::Emulator;
use crate::event::{ClipboardWrite, Event};
use crate::parser::{Dcs, Terminator};
use crate::style::Hyperlink;

impl Emulator {
    /// Handles an OSC string. Returns false when it isn't known.
    pub(super) fn osc(&mut self, data: &[u8], terminator: Terminator) -> bool {
        let text = String::from_utf8_lossy(data);
        let (number, rest) = match text.split_once(';') {
            Some((number, rest)) => (number, Some(rest)),
            None => (&*text, None),
        };
        match (number, rest) {
            ("0", Some(title)) => {
                title.clone_into(&mut self.title);
                title.clone_into(&mut self.icon);
                self.event(Event::TitleSet(title.to_owned()));
            }
            ("1", Some(icon)) => icon.clone_into(&mut self.icon),
            ("2", Some(title)) => {
                title.clone_into(&mut self.title);
                self.event(Event::TitleSet(title.to_owned()));
            }
            ("8", Some(rest)) => {
                let Some((params, url)) = rest.split_once(';') else {
                    return false;
                };
                if self.profile.hyperlinks {
                    self.pen.link = if url.is_empty() {
                        None
                    } else {
                        let id = params
                            .split(':')
                            .find_map(|param| param.strip_prefix("id="))
                            .map(str::to_owned);
                        Some(Arc::new(Hyperlink {
                            url: url.to_owned(),
                            id,
                        }))
                    };
                }
            }
            ("10" | "11", Some("?")) => {
                let color = if number == "10" {
                    self.profile.foreground
                } else {
                    self.profile.background
                };
                if let Some((r, g, b)) = color {
                    let end = match terminator {
                        Terminator::Bel => "\x07",
                        Terminator::St => "\x1b\\",
                    };
                    let answer = format!(
                        "\x1b]{number};rgb:{:04x}/{:04x}/{:04x}{end}",
                        u16::from(r) * 0x101,
                        u16::from(g) * 0x101,
                        u16::from(b) * 0x101
                    );
                    self.reply(answer.as_bytes());
                }
            }
            ("52", Some(rest)) => {
                let Some((selection, payload)) = rest.split_once(';') else {
                    return false;
                };
                // A read asks for the clipboard, which the emulator never gives.
                if payload != "?" {
                    let Some(data) = base64_decode(payload) else {
                        return false;
                    };
                    self.clipboard.push(ClipboardWrite {
                        selection: selection.to_owned(),
                        data,
                    });
                }
            }
            _ => return false,
        }
        true
    }

    /// Handles a DCS string. Returns false when it isn't known.
    pub(super) fn dcs(&mut self, dcs: &Dcs<'_>) -> bool {
        match (dcs.private, dcs.intermediates, dcs.final_byte) {
            (None, [b'+'], b'q') if dcs.params.is_empty() => {
                self.xtgettcap(dcs.data);
                true
            }
            _ => false,
        }
    }

    /// XTGETTCAP, answered with one string per name asked for, as kitty does.
    fn xtgettcap(&mut self, data: &[u8]) {
        let Some(table) = self.profile.termcap.clone() else {
            return;
        };
        for hex_name in data.split(|&b| b == b';') {
            let name = hex_decode(hex_name).and_then(|bytes| String::from_utf8(bytes).ok());
            let entry = name.and_then(|name| table.iter().find(|(known, _)| *known == name));
            let hex_name = String::from_utf8_lossy(hex_name);
            let answer = match entry {
                Some((_, Some(value))) => {
                    format!("\x1bP1+r{hex_name}={}\x1b\\", hex_encode(value.as_bytes()))
                }
                Some((_, None)) => format!("\x1bP1+r{hex_name}\x1b\\"),
                None => format!("\x1bP0+r{hex_name}\x1b\\"),
            };
            self.reply(answer.as_bytes());
        }
    }
}

fn hex_decode(hex: &[u8]) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    hex.chunks(2)
        .map(|pair| {
            let digit = |b: u8| char::from(b).to_digit(16);
            Some(u8::try_from(digit(pair[0])? * 16 + digit(pair[1])?).unwrap_or(0))
        })
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02X}");
    }
    out
}

/// Decodes standard base64, with or without padding. Returns `None` on any other byte.
pub(crate) fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0;
    let body = text.trim_end_matches('=');
    if text.len() - body.len() > 2 {
        return None;
    }
    for b in body.bytes() {
        let value = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xff).unwrap_or(0));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64() {
        assert_eq!(base64_decode("aGVsbG8=").as_deref(), Some(&b"hello"[..]));
        assert_eq!(base64_decode("aGVsbG8").as_deref(), Some(&b"hello"[..]));
        assert_eq!(base64_decode("").as_deref(), Some(&b""[..]));
        assert_eq!(base64_decode("a$"), None);
    }

    #[test]
    fn hex() {
        assert_eq!(hex_decode(b"5463").as_deref(), Some(&b"Tc"[..]));
        assert_eq!(hex_decode(b"546"), None);
        assert_eq!(hex_encode(b"256"), "323536");
    }
}
