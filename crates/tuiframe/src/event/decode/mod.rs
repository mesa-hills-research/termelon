//! The decoder, a pure state machine from the terminal's bytes, or Windows console records, to
//! [`Event`]s and the replies to the library's queries. It does no I/O, and the reader thread owns
//! one.

mod keys;
mod mouse;
mod replies;
mod tokenizer;
mod windows;

#[cfg(test)]
mod tests;

use self::tokenizer::{Csi, StringKind, Token, Tokenizer};
use self::windows::WindowsState;
use super::records::InputRecord;
use super::{Event, KeyCode, KeyEvent, KeyModifiers};
use crate::caps::Rgb;

/// How the terminal sends some keys, read from its settings at open, and the decoder's bounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DecoderOptions {
    /// The byte Backspace sends, termios `VERASE`, `0x7f` or `0x08`. Any other value counts as
    /// `0x7f`.
    pub(crate) erase: u8,
    /// The most bytes of a paste held at once, 64 MiB by default. A longer paste arrives as
    /// several [`Event::Paste`]s of about this size, in order.
    pub(crate) max_paste: usize,
}

/// The default bound of a paste.
pub(crate) const MAX_PASTE: usize = 64 << 20;

impl Default for DecoderOptions {
    fn default() -> Self {
        Self {
            erase: 0x7f,
            max_paste: MAX_PASTE,
        }
    }
}

/// What the decoder found, an event for the program or a reply for the probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Decoded {
    Event(Event),
    Reply(Reply),
}

/// A query whose answer the decoder must be told to expect, because the answer reads like a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Query {
    /// `CSI 6 n`, answered with `CSI row ; column R`, which is also a modified F3.
    CursorPosition,
}

/// A terminal's answer to a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Reply {
    /// `CSI ? a ; b ... c`.
    Da1 { attributes: Vec<u16> },
    /// `CSI > kind ; version ; ... c`.
    Da2 { kind: u16, version: u32 },
    /// DECRPM, `CSI ? mode ; value $ y`, or without `?` for an ANSI mode.
    ModeReport {
        mode: u16,
        private: bool,
        value: ModeValue,
    },
    /// `CSI row ; column R`, 0-based, decoded only while one is expected.
    CursorPosition { row: u16, column: u16 },
    /// `CSI ? flags u`.
    KittyKeyboard(u8),
    /// `DCS > | text ST`.
    XtVersion(String),
    /// `DCS 1 + r name = value ST`, decoded from hex, with no value when the terminal doesn't know
    /// the name (`DCS 0 + r name ST`).
    XtGetTcap { name: String, value: Option<String> },
    /// `OSC 10` or `OSC 11` with a colour, each channel scaled to 8 bits.
    Color { which: ColorSlot, rgb: Rgb },
    /// `CSI 8 ; rows ; columns t`.
    TerminalSize { rows: u16, columns: u16 },
}

/// The state DECRPM reports for a mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModeValue {
    NotRecognized,
    Set,
    Reset,
    PermanentlySet,
    PermanentlyReset,
}

/// Which colour an OSC colour reply is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ColorSlot {
    /// `OSC 10`.
    Foreground,
    /// `OSC 11`.
    Background,
}

/// The tokenizer's state, the outstanding queries and what Windows records need remembered.
#[derive(Debug)]
pub(crate) struct Decoder {
    tokenizer: Tokenizer,
    context: Context,
    windows: WindowsState,
}

/// What decoding a token needs besides the token.
#[derive(Debug)]
struct Context {
    erase: u8,
    /// Cursor reports asked for and not yet answered.
    cursor_reports: u32,
}

impl Decoder {
    pub(crate) fn new(options: DecoderOptions) -> Self {
        Self {
            tokenizer: Tokenizer::new(options.max_paste),
            context: Context {
                erase: if options.erase == 0x08 { 0x08 } else { 0x7f },
                cursor_reports: 0,
            },
            windows: WindowsState::default(),
        }
    }

    /// Decodes what it can of `bytes` and keeps an incomplete sequence for the next call, so a
    /// sequence split across reads decodes as if it came whole.
    pub(crate) fn feed(&mut self, bytes: &[u8], out: &mut Vec<Decoded>) {
        let context = &mut self.context;
        let mut emit = |token: Token<'_>| context.token(&token, out);
        for &byte in bytes {
            self.tokenizer.advance(byte, &mut emit);
        }
    }

    /// Whether it holds an ESC, or a sequence's introducer, with nothing after it. The reader then
    /// waits up to the escape timeout for more before calling [`Decoder::flush_pending`].
    pub(crate) fn has_pending(&self) -> bool {
        self.tokenizer.has_pending()
    }

    /// Decodes what [`Decoder::has_pending`] counts as keys typed, since no more input follows, so
    /// a lone ESC becomes Esc and `ESC [` becomes Alt+[. Afterwards nothing is pending. A sequence
    /// with more of it (`ESC [ 1 ;`), a string, a paste or part of a UTF-8 character stays for the
    /// next read, since only a split read leaves those unfinished.
    pub(crate) fn flush_pending(&mut self, out: &mut Vec<Decoded>) {
        let context = &mut self.context;
        let mut emit = |token: Token<'_>| context.token(&token, out);
        self.tokenizer.flush(&mut emit);
    }

    /// Whether it holds anything at all: what [`Decoder::has_pending`] counts, a sequence with more
    /// of it, a string, a paste or part of a character. The reader calls
    /// [`Decoder::flush_held`] once the input has been silent for a while with something held.
    pub(crate) fn has_held(&self) -> bool {
        self.tokenizer.has_held()
    }

    /// Gives up on what it holds, as the input has been silent too long for it to be finished.
    /// What [`Decoder::flush_pending`] takes becomes keys, a paste is delivered as it stands, and
    /// a sequence, a string or part of a character is dropped. Afterwards nothing is held.
    pub(crate) fn flush_held(&mut self, out: &mut Vec<Decoded>) {
        let context = &mut self.context;
        let mut emit = |token: Token<'_>| context.token(&token, out);
        self.tokenizer.flush_held(&mut emit);
    }

    /// Forgets the queries whose answers are outstanding, for a probe that stopped waiting.
    pub(crate) fn cancel_expected(&mut self) {
        self.context.cursor_reports = 0;
    }

    /// Records one query whose answer is outstanding. Each matching reply uses one up, and a DA1
    /// reply clears them all, since terminals answer in order.
    pub(crate) fn expect(&mut self, query: Query) {
        match query {
            Query::CursorPosition => {
                self.context.cursor_reports = self.context.cursor_reports.saturating_add(1);
            }
        }
    }

    /// Decodes Windows console records into the same events as [`Decoder::feed`] gives for bytes.
    pub(crate) fn decode_records(&mut self, records: &[InputRecord], out: &mut Vec<Decoded>) {
        for record in records {
            self.windows.decode(record, self.context.erase, out);
        }
    }

    /// The bytes held for a string or a paste, for tests of its bounds.
    #[cfg(test)]
    fn buffer_capacity(&self) -> usize {
        self.tokenizer.buffer_capacity()
    }
}

impl Context {
    fn token(&mut self, token: &Token<'_>, out: &mut Vec<Decoded>) {
        match *token {
            Token::Char { ch, alt } => key(out, keys::char_key(ch), alt),
            Token::Control { byte, alt } => key(out, keys::control(byte, self.erase), alt),
            Token::Esc { alt } => key(out, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), alt),
            Token::Csi { csi, alt } => self.csi(csi, alt, out),
            Token::Ss3 { csi, alt } => maybe_key(out, keys::ss3(csi), alt),
            Token::LinuxFunction { letter, alt } => {
                let n = letter - b'A' + 1;
                key(out, KeyEvent::new(KeyCode::F(n), KeyModifiers::NONE), alt);
            }
            Token::X10Mouse { bytes, alt } => {
                other(
                    out,
                    mouse::x10(bytes).map(Event::Mouse).map(Decoded::Event),
                    alt,
                );
            }
            Token::String { kind, csi, data } => {
                let replies = match kind {
                    StringKind::Osc => replies::osc(data),
                    StringKind::Dcs => replies::dcs(csi, data),
                    StringKind::Apc => Vec::new(),
                };
                out.extend(replies.into_iter().map(Decoded::Reply));
            }
            Token::Paste(body) => out.push(Decoded::Event(Event::Paste(paste(body)))),
        }
    }

    fn csi(&mut self, csi: &Csi, alt: bool, out: &mut Vec<Decoded>) {
        if let Some(event) = keys::csi(csi, self.cursor_reports > 0) {
            return key(out, event, alt);
        }
        let decoded = match (csi.private, csi.intermediates(), csi.final_byte) {
            (None, [], b'R') if self.cursor_reports > 0 => {
                self.cursor_reports -= 1;
                Some(Decoded::Reply(replies::cursor_position(csi)))
            }
            (None, [], b'I') if csi.params.count() == 0 => Some(Decoded::Event(Event::FocusGained)),
            (None, [], b'O') if csi.params.count() == 0 => Some(Decoded::Event(Event::FocusLost)),
            (Some(b'<'), [], b'M' | b'm') | (None, [], b'M') => {
                mouse::csi(csi).map(|event| Decoded::Event(Event::Mouse(event)))
            }
            (Some(b'?'), [], b'n') => replies::color_scheme(csi).map(Decoded::Event),
            _ => {
                let reply = replies::csi(csi);
                if matches!(reply, Some(Reply::Da1 { .. })) {
                    self.cursor_reports = 0;
                }
                reply.map(Decoded::Reply)
            }
        };
        other(out, decoded, alt);
    }
}

/// Pushes a key, with Alt when an ESC came before it.
fn key(out: &mut Vec<Decoded>, mut event: KeyEvent, alt: bool) {
    if alt {
        event.modifiers |= KeyModifiers::ALT;
    }
    out.push(Decoded::Event(Event::Key(event)));
}

fn maybe_key(out: &mut Vec<Decoded>, event: Option<KeyEvent>, alt: bool) {
    match event {
        Some(event) => key(out, event, alt),
        None => other(out, None, alt),
    }
}

/// Pushes what isn't a key. An ESC before it was the Esc key, so that comes first.
fn other(out: &mut Vec<Decoded>, decoded: Option<Decoded>, alt: bool) {
    if alt {
        key(out, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE), false);
    }
    out.extend(decoded);
}

/// A paste's text, with CR and CRLF as LF.
fn paste(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\r' {
            chars.next_if_eq(&'\n');
            out.push('\n');
        } else {
            out.push(ch);
        }
    }
    out
}
