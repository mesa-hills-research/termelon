//! Bytes to tokens, in the shape of Paul Williams' VT500 parser, with the changes input needs.
//!
//! A terminal's input mixes what the user types with the terminal's own reports, so a few things
//! differ from a parser of output. An ESC before a key is the Alt prefix, so `ESC` followed by any
//! byte that starts no sequence is that key with Alt. A sequence cut short by a control byte, an
//! ESC or a byte past ASCII is dropped and the byte read again, since it's the user's typing. A
//! string starts only where its first byte fits a report (a digit after `ESC ]`, a parameter or
//! intermediate after `ESC P`, `G` after `ESC _`), so Alt+], Alt+Shift+P and Alt+_ stay keys, and
//! `ESC ^` and `ESC X` are always keys.
//!
//! The bounds are fixed. Parameters stop at [`MAX_PARAMS`], strings at [`MAX_STRING`] bytes with
//! the rest dropped to the string's end, and a paste is kept whole until its terminator, or until
//! it reaches the decoder's paste bound, when what came so far is delivered as a paste of its own.

/// The most parameters and sub-parameters a sequence keeps. Later ones are dropped.
pub(crate) const MAX_PARAMS: usize = 32;
/// The most bytes an OSC, DCS or APC string keeps. A longer string is dropped whole.
pub(crate) const MAX_STRING: usize = 64 * 1024;
/// What ends a bracketed paste.
const PASTE_END: &[u8] = b"\x1b[201~";

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;

/// A sequence's numeric parameters, split at `;` into groups, each with its `:` sub-parameters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Params {
    values: [Option<u32>; MAX_PARAMS],
    len: usize,
    /// Bit `i` is set when value `i` follows a `:`, as a sub-parameter of the value before.
    sub: u32,
    /// Set once a value didn't fit, so its digits are dropped too.
    full: bool,
}

impl Params {
    fn push(&mut self, sub: bool) {
        if self.len == MAX_PARAMS {
            self.full = true;
            return;
        }
        if sub {
            self.sub |= 1 << self.len;
        }
        self.len += 1;
    }

    fn digit(&mut self, digit: u8) {
        if self.len == 0 {
            self.push(false);
        }
        if self.full {
            return;
        }
        if let Some(slot) = self
            .len
            .checked_sub(1)
            .and_then(|last| self.values.get_mut(last))
        {
            let value = slot.unwrap_or(0);
            *slot = Some(value.saturating_mul(10).saturating_add(u32::from(digit)));
        }
    }

    fn separator(&mut self, colon: bool) {
        if self.len == 0 {
            self.push(false);
        }
        self.push(colon);
    }

    fn is_sub(&self, index: usize) -> bool {
        index < MAX_PARAMS && self.sub & (1 << index) != 0
    }

    /// The number of `;` groups.
    pub(crate) fn count(&self) -> usize {
        (0..self.len).filter(|&i| !self.is_sub(i)).count()
    }

    /// Group `n`, its value first and its sub-parameters after. Empty when there's no such group.
    pub(crate) fn group(&self, n: usize) -> &[Option<u32>] {
        let mut start = None;
        let mut seen = 0;
        for i in 0..self.len {
            if self.is_sub(i) {
                continue;
            }
            if let Some(start) = start {
                return self.values.get(start..i).unwrap_or(&[]);
            }
            if seen == n {
                start = Some(i);
            }
            seen += 1;
        }
        start
            .and_then(|start| self.values.get(start..self.len))
            .unwrap_or(&[])
    }

    /// Group `n`'s value, or `None` when it's missing or empty.
    pub(crate) fn value(&self, n: usize) -> Option<u32> {
        self.group(n).first().copied().flatten()
    }

    /// Sub-parameter `sub` of group `n`, counting the group's value as 0.
    pub(crate) fn sub(&self, n: usize, sub: usize) -> Option<u32> {
        self.group(n).get(sub).copied().flatten()
    }

    /// Every group's value, empty ones as `None`.
    pub(crate) fn values(&self) -> impl Iterator<Item = Option<u32>> + '_ {
        (0..self.count()).map(|n| self.value(n))
    }
}

/// A control sequence's parts, also used for a DCS string's header.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Csi {
    /// The private marker (`<`, `=`, `>` or `?`) before the parameters.
    pub(crate) private: Option<u8>,
    pub(crate) params: Params,
    intermediates: [u8; 2],
    intermediate_len: usize,
    /// The final byte, from `@` to `~`.
    pub(crate) final_byte: u8,
}

impl Csi {
    /// The intermediate bytes, from space to `/`, before the final byte.
    pub(crate) fn intermediates(&self) -> &[u8] {
        self.intermediates
            .get(..self.intermediate_len)
            .unwrap_or(&[])
    }

    /// Whether it has no private marker and no intermediates, as most keys have.
    pub(crate) fn is_plain(&self) -> bool {
        self.private.is_none() && self.intermediate_len == 0
    }

    /// Adds an intermediate, or says it didn't fit.
    fn intermediate(&mut self, byte: u8) -> bool {
        match self.intermediates.get_mut(self.intermediate_len) {
            Some(slot) => {
                *slot = byte;
                self.intermediate_len += 1;
                true
            }
            None => false,
        }
    }
}

/// The kinds of string a terminal sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StringKind {
    /// `ESC ]`, an operating system command.
    Osc,
    /// `ESC P`, a device control string.
    Dcs,
    /// `ESC _`, an application program command.
    Apc,
}

impl StringKind {
    /// The byte after ESC that starts it, which is also the key Alt applies to when it isn't one.
    fn introducer(self) -> u8 {
        match self {
            Self::Osc => b']',
            Self::Dcs => b'P',
            Self::Apc => b'_',
        }
    }

    /// Whether `byte`, right after the introducer, starts a string a terminal would send.
    fn starts_with(self, byte: u8) -> bool {
        match self {
            Self::Osc => byte.is_ascii_digit(),
            Self::Dcs => (0x20..=0x3f).contains(&byte),
            Self::Apc => byte == b'G',
        }
    }
}

/// What the tokenizer finds. `alt` is set when an ESC came before it, which is Alt for a key.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Token<'a> {
    /// A character, printable.
    Char { ch: char, alt: bool },
    /// A C0 control or DEL, other than ESC.
    Control { byte: u8, alt: bool },
    /// ESC with nothing after it, at a flush. With `alt`, two of them.
    Esc { alt: bool },
    /// A control sequence, `ESC [`, its parameters and its final byte.
    Csi { csi: &'a Csi, alt: bool },
    /// `ESC O`, its parameters if any and its final byte.
    Ss3 { csi: &'a Csi, alt: bool },
    /// The Linux console's `ESC [ [` and a letter from `A` to `E`, its F1 to F5.
    LinuxFunction { letter: u8, alt: bool },
    /// An X10 mouse report, `ESC [ M` and three bytes.
    X10Mouse { bytes: [u8; 3], alt: bool },
    /// A complete string, without its introducer and terminator. A DCS string's header is in `csi`.
    String {
        kind: StringKind,
        csi: &'a Csi,
        data: &'a [u8],
    },
    /// A bracketed paste's body, without its markers.
    Paste(&'a [u8]),
}

/// Where the tokenizer is. `alt` says an ESC came before the sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    /// Part of a UTF-8 character, in `utf8`.
    Utf8 {
        alt: bool,
    },
    Escape {
        alt: bool,
    },
    /// `ESC [` and nothing yet.
    CsiEntry {
        alt: bool,
    },
    /// Parameters, in a CSI or a DCS header.
    Param {
        alt: bool,
        dcs: bool,
    },
    Intermediate {
        alt: bool,
        dcs: bool,
    },
    /// A malformed sequence, read to its final byte and dropped.
    Ignore {
        dcs: bool,
    },
    /// `ESC [ [`, the Linux console's function keys.
    LinuxFunction {
        alt: bool,
    },
    /// `ESC [ M`, waiting for its three bytes.
    X10 {
        alt: bool,
    },
    /// `ESC O` and nothing yet.
    Ss3Entry {
        alt: bool,
    },
    /// `ESC O` and parameters, as some terminals send modified F1 to F4.
    Ss3Param {
        alt: bool,
    },
    /// The byte after a string's introducer decides whether it's a string or a key.
    StringIntro {
        kind: StringKind,
        alt: bool,
    },
    String {
        kind: StringKind,
    },
    /// An ESC inside a string, which with `\` ends it.
    StringEscape {
        kind: StringKind,
    },
    Paste,
}

/// The tokenizer's state. Feed it bytes one at a time with [`Tokenizer::advance`].
#[derive(Debug)]
pub(crate) struct Tokenizer {
    state: State,
    utf8: [u8; 4],
    utf8_len: usize,
    utf8_need: usize,
    csi: Csi,
    x10: [u8; 3],
    x10_len: usize,
    /// A string's or a paste's bytes.
    buf: Vec<u8>,
    /// Set when a string passed [`MAX_STRING`], so it's dropped at its end.
    overflow: bool,
    /// The most bytes of a paste held at once. A longer paste comes in pieces of about this size.
    max_paste: usize,
}

impl Tokenizer {
    pub(crate) fn new(max_paste: usize) -> Self {
        Self {
            state: State::Ground,
            utf8: [0; 4],
            utf8_len: 0,
            utf8_need: 0,
            csi: Csi::default(),
            x10: [0; 3],
            x10_len: 0,
            buf: Vec::new(),
            overflow: false,
            max_paste: max_paste.max(1),
        }
    }

    /// Whether it holds anything: what [`Tokenizer::has_pending`] counts, a sequence with more of
    /// it, a string, a paste or part of a character.
    pub(crate) fn has_held(&self) -> bool {
        self.state != State::Ground
    }

    /// Gives up on what it holds, as the input has been silent too long for it to be finished.
    /// What [`Tokenizer::flush`] takes is keys, a paste is delivered as it stands, and anything
    /// else is dropped. Afterwards nothing is held.
    pub(crate) fn flush_held(&mut self, emit: &mut impl FnMut(Token<'_>)) {
        if self.has_pending() {
            self.flush(emit);
        } else if self.state == State::Paste {
            emit(Token::Paste(&self.buf));
            self.clear_buf();
        } else {
            self.clear_buf();
        }
        self.state = State::Ground;
    }

    /// Whether it holds an ESC or a sequence's introducer with nothing after it, which
    /// [`Tokenizer::flush`] would turn into keys.
    pub(crate) fn has_pending(&self) -> bool {
        matches!(
            self.state,
            State::Escape { .. }
                | State::CsiEntry { .. }
                | State::LinuxFunction { .. }
                | State::X10 { .. }
                | State::Ss3Entry { .. }
                | State::StringIntro { .. }
        )
    }

    /// Takes what [`Tokenizer::has_pending`] counts as keys typed, since no more input follows. A
    /// sequence with more of it, a string, a paste or part of a character stays, since only a
    /// split read leaves those unfinished.
    pub(crate) fn flush(&mut self, emit: &mut impl FnMut(Token<'_>)) {
        match self.state {
            State::Escape { alt } => {
                self.state = State::Ground;
                emit(Token::Esc { alt });
            }
            State::CsiEntry { alt } => self.typed(b'[', alt, emit),
            State::LinuxFunction { alt } => {
                self.typed(b'[', alt, emit);
                self.advance(b'[', emit);
            }
            State::X10 { alt } => self.x10_typed(alt, emit),
            State::Ss3Entry { alt } => self.typed(b'O', alt, emit),
            State::StringIntro { kind, alt } => self.typed(kind.introducer(), alt, emit),
            _ => {}
        }
    }

    /// The bytes it holds for a string or a paste, for tests of its bounds.
    #[cfg(test)]
    pub(crate) fn buffer_capacity(&self) -> usize {
        self.buf.capacity()
    }

    /// Reads one byte, emitting any token it completes.
    pub(crate) fn advance(&mut self, byte: u8, emit: &mut impl FnMut(Token<'_>)) {
        match self.state {
            State::Ground => self.ground(byte, false, emit),
            State::Utf8 { alt } => self.utf8(byte, alt, emit),
            State::Escape { alt } => self.escape(byte, alt, emit),
            State::CsiEntry { alt } => self.csi_entry(byte, alt, emit),
            State::Param { alt, dcs } => self.param(byte, alt, dcs, emit),
            State::Intermediate { alt, dcs } => self.intermediate(byte, alt, dcs, emit),
            State::Ignore { dcs } => match byte {
                0x20..=0x3f => {}
                0x40..=0x7e if dcs => {
                    // Read to the string's end, then dropped.
                    self.begin_string(StringKind::Dcs, true);
                    self.overflow = true;
                }
                0x40..=0x7e => self.state = State::Ground,
                _ => self.abort(byte, emit),
            },
            State::LinuxFunction { alt } => {
                if (b'A'..=b'E').contains(&byte) {
                    self.state = State::Ground;
                    emit(Token::LinuxFunction { letter: byte, alt });
                } else {
                    self.typed(b'[', alt, emit);
                    self.advance(b'[', emit);
                    self.advance(byte, emit);
                }
            }
            State::X10 { alt } => self.x10(byte, alt, emit),
            State::Ss3Entry { alt } => match byte {
                b'0'..=b'9' | b';' => {
                    self.csi = Csi::default();
                    self.state = State::Ss3Param { alt };
                    self.param_byte(byte);
                }
                0x40..=0x7e => {
                    self.csi = Csi {
                        final_byte: byte,
                        ..Csi::default()
                    };
                    self.state = State::Ground;
                    emit(Token::Ss3 {
                        csi: &self.csi,
                        alt,
                    });
                }
                _ => {
                    self.typed(b'O', alt, emit);
                    self.advance(byte, emit);
                }
            },
            State::Ss3Param { alt } => match byte {
                b'0'..=b'9' | b';' => self.param_byte(byte),
                0x40..=0x7e => {
                    self.csi.final_byte = byte;
                    self.state = State::Ground;
                    emit(Token::Ss3 {
                        csi: &self.csi,
                        alt,
                    });
                }
                _ => self.abort(byte, emit),
            },
            State::StringIntro { kind, alt } => {
                if kind.starts_with(byte) {
                    if alt {
                        emit(Token::Esc { alt: false });
                    }
                    if kind == StringKind::Dcs {
                        self.csi = Csi::default();
                        self.state = State::Param {
                            alt: false,
                            dcs: true,
                        };
                        self.advance(byte, emit);
                    } else {
                        self.begin_string(kind, false);
                        self.string(byte, kind, emit);
                    }
                } else {
                    self.typed(kind.introducer(), alt, emit);
                    self.advance(byte, emit);
                }
            }
            State::String { kind } => self.string(byte, kind, emit),
            State::StringEscape { kind } => {
                if byte == b'\\' {
                    self.end_string(kind, emit);
                } else {
                    self.clear_buf();
                    self.state = State::Escape { alt: false };
                    self.advance(byte, emit);
                }
            }
            State::Paste => {
                self.buf.push(byte);
                if self.buf.ends_with(PASTE_END) {
                    let body = self.buf.len() - PASTE_END.len();
                    self.state = State::Ground;
                    emit(Token::Paste(self.buf.get(..body).unwrap_or(&[])));
                    self.clear_buf();
                } else if self.buf.len() >= self.max_paste + PASTE_END.len() {
                    self.paste_piece(emit);
                }
            }
        }
    }

    fn ground(&mut self, byte: u8, alt: bool, emit: &mut impl FnMut(Token<'_>)) {
        self.state = State::Ground;
        match byte {
            ESC => self.state = State::Escape { alt: false },
            0x00..=0x1f | 0x7f => emit(Token::Control { byte, alt }),
            0x20..=0x7e => emit(Token::Char {
                ch: char::from(byte),
                alt,
            }),
            0xc2..=0xdf => self.begin_utf8(byte, 2, alt),
            0xe0..=0xef => self.begin_utf8(byte, 3, alt),
            0xf0..=0xf4 => self.begin_utf8(byte, 4, alt),
            // A byte that starts no UTF-8 character, dropped.
            _ => {
                if alt {
                    emit(Token::Esc { alt: false });
                }
            }
        }
    }

    fn begin_utf8(&mut self, byte: u8, need: usize, alt: bool) {
        self.utf8[0] = byte;
        self.utf8_len = 1;
        self.utf8_need = need;
        self.state = State::Utf8 { alt };
    }

    fn utf8(&mut self, byte: u8, alt: bool, emit: &mut impl FnMut(Token<'_>)) {
        if byte & 0xc0 != 0x80 {
            // Cut short, so the part is dropped and the byte read again.
            self.state = State::Ground;
            if alt {
                emit(Token::Esc { alt: false });
            }
            self.advance(byte, emit);
            return;
        }
        if let Some(slot) = self.utf8.get_mut(self.utf8_len) {
            *slot = byte;
            self.utf8_len += 1;
        }
        if self.utf8_len < self.utf8_need {
            return;
        }
        self.state = State::Ground;
        let bytes = self.utf8.get(..self.utf8_len).unwrap_or(&[]);
        match std::str::from_utf8(bytes)
            .ok()
            .and_then(|s| s.chars().next())
        {
            Some(ch) => emit(Token::Char { ch, alt }),
            // Overlong, a surrogate or past U+10FFFF, dropped.
            None => {
                if alt {
                    emit(Token::Esc { alt: false });
                }
            }
        }
    }

    fn escape(&mut self, byte: u8, alt: bool, emit: &mut impl FnMut(Token<'_>)) {
        match byte {
            ESC if alt => {
                emit(Token::Esc { alt: true });
                self.state = State::Escape { alt: false };
            }
            ESC => self.state = State::Escape { alt: true },
            b'[' => self.state = State::CsiEntry { alt },
            b'O' => self.state = State::Ss3Entry { alt },
            b']' => {
                self.state = State::StringIntro {
                    kind: StringKind::Osc,
                    alt,
                }
            }
            b'P' => {
                self.state = State::StringIntro {
                    kind: StringKind::Dcs,
                    alt,
                }
            }
            b'_' => {
                self.state = State::StringIntro {
                    kind: StringKind::Apc,
                    alt,
                }
            }
            // ESC ESC and a key that starts no sequence are Esc and then Alt with that key.
            _ if alt => {
                emit(Token::Esc { alt: false });
                self.state = State::Escape { alt: false };
                self.advance(byte, emit);
            }
            _ => self.ground(byte, true, emit),
        }
    }

    fn csi_entry(&mut self, byte: u8, alt: bool, emit: &mut impl FnMut(Token<'_>)) {
        match byte {
            b'[' => self.state = State::LinuxFunction { alt },
            0x20..=0x7e => {
                self.csi = Csi::default();
                self.state = State::Param { alt, dcs: false };
                self.param(byte, alt, false, emit);
            }
            _ => {
                self.typed(b'[', alt, emit);
                self.advance(byte, emit);
            }
        }
    }

    fn param(&mut self, byte: u8, alt: bool, dcs: bool, emit: &mut impl FnMut(Token<'_>)) {
        match byte {
            b'0'..=b'9' | b':' | b';' => self.param_byte(byte),
            b'<'..=b'?' => {
                let first = self.csi.private.is_none() && self.csi.params.len == 0;
                if first {
                    self.csi.private = Some(byte);
                } else {
                    self.state = State::Ignore { dcs };
                }
            }
            0x20..=0x2f => {
                self.state = State::Intermediate { alt, dcs };
                self.intermediate(byte, alt, dcs, emit);
            }
            0x40..=0x7e => self.dispatch(byte, alt, dcs, emit),
            _ => self.abort(byte, emit),
        }
    }

    fn param_byte(&mut self, byte: u8) {
        match byte {
            b':' => self.csi.params.separator(true),
            b';' => self.csi.params.separator(false),
            _ => self.csi.params.digit(byte - b'0'),
        }
    }

    fn intermediate(&mut self, byte: u8, alt: bool, dcs: bool, emit: &mut impl FnMut(Token<'_>)) {
        match byte {
            0x20..=0x2f => {
                if !self.csi.intermediate(byte) {
                    self.state = State::Ignore { dcs };
                }
            }
            0x30..=0x3f => self.state = State::Ignore { dcs },
            0x40..=0x7e => self.dispatch(byte, alt, dcs, emit),
            _ => self.abort(byte, emit),
        }
    }

    fn dispatch(&mut self, byte: u8, alt: bool, dcs: bool, emit: &mut impl FnMut(Token<'_>)) {
        self.csi.final_byte = byte;
        if dcs {
            self.begin_string(StringKind::Dcs, true);
            return;
        }
        self.state = State::Ground;
        let plain = self.csi.is_plain();
        if plain && byte == b'M' && self.csi.params.len == 0 {
            self.x10_len = 0;
            self.state = State::X10 { alt };
        } else if plain && byte == b'~' && self.csi.params == paste_start() {
            if alt {
                emit(Token::Esc { alt: false });
            }
            self.clear_buf();
            self.state = State::Paste;
        } else {
            emit(Token::Csi {
                csi: &self.csi,
                alt,
            });
        }
    }

    fn x10(&mut self, byte: u8, alt: bool, emit: &mut impl FnMut(Token<'_>)) {
        // Every byte of a report is 32 or more, so a control byte means these were keys.
        if byte < 0x20 {
            self.x10_typed(alt, emit);
            self.advance(byte, emit);
            return;
        }
        if let Some(slot) = self.x10.get_mut(self.x10_len) {
            *slot = byte;
            self.x10_len += 1;
        }
        if self.x10_len == self.x10.len() {
            self.state = State::Ground;
            emit(Token::X10Mouse {
                bytes: self.x10,
                alt,
            });
        }
    }

    /// `ESC [ M` and what came after it, read as the keys Alt+[, M and those bytes.
    fn x10_typed(&mut self, alt: bool, emit: &mut impl FnMut(Token<'_>)) {
        let held = self.x10;
        let len = self.x10_len;
        self.typed(b'[', alt, emit);
        self.advance(b'M', emit);
        for &byte in held.get(..len).unwrap_or(&[]) {
            self.advance(byte, emit);
        }
    }

    /// The introducer `byte` was a key typed with Alt, with Esc before it when two ESCs came.
    fn typed(&mut self, byte: u8, alt: bool, emit: &mut impl FnMut(Token<'_>)) {
        self.state = State::Ground;
        if alt {
            emit(Token::Esc { alt: false });
        }
        emit(Token::Char {
            ch: char::from(byte),
            alt: true,
        });
    }

    /// Drops the sequence `byte` cut short, and reads `byte` again.
    fn abort(&mut self, byte: u8, emit: &mut impl FnMut(Token<'_>)) {
        self.state = State::Ground;
        self.advance(byte, emit);
    }

    fn begin_string(&mut self, kind: StringKind, keep_header: bool) {
        if !keep_header {
            self.csi = Csi::default();
        }
        self.clear_buf();
        self.overflow = false;
        self.state = State::String { kind };
    }

    fn string(&mut self, byte: u8, kind: StringKind, emit: &mut impl FnMut(Token<'_>)) {
        match byte {
            BEL => self.end_string(kind, emit),
            ESC => self.state = State::StringEscape { kind },
            CAN | SUB => {
                self.clear_buf();
                self.state = State::Ground;
            }
            0x00..=0x1f => {}
            _ if !self.overflow && self.buf.len() < MAX_STRING => self.buf.push(byte),
            _ => self.overflow = true,
        }
    }

    fn end_string(&mut self, kind: StringKind, emit: &mut impl FnMut(Token<'_>)) {
        self.state = State::Ground;
        if !self.overflow {
            emit(Token::String {
                kind,
                csi: &self.csi,
                data: &self.buf,
            });
        }
        self.clear_buf();
    }

    /// Delivers what a paste past the bound holds, as a paste of its own, and keeps the rest. It
    /// keeps what may be the start of the terminator, part of a character and a CR that an LF may
    /// follow, so the next piece takes them whole.
    fn paste_piece(&mut self, emit: &mut impl FnMut(Token<'_>)) {
        let mut cut = self.buf.len() - (PASTE_END.len() - 1);
        while cut > 0 && self.buf.get(cut).is_some_and(|&byte| byte & 0xc0 == 0x80) {
            cut -= 1;
        }
        if cut > 0 && self.buf.get(cut - 1) == Some(&b'\r') {
            cut -= 1;
        }
        if cut == 0 {
            return;
        }
        emit(Token::Paste(self.buf.get(..cut).unwrap_or(&[])));
        self.buf.drain(..cut);
    }

    /// Empties the string and paste buffer, and lets a large paste's memory go.
    fn clear_buf(&mut self) {
        if self.buf.capacity() > MAX_STRING {
            self.buf = Vec::new();
        } else {
            self.buf.clear();
        }
    }
}

/// The parameters of `CSI 200 ~`, which starts a paste.
fn paste_start() -> Params {
    let mut params = Params::default();
    for digit in [2, 0, 0] {
        params.digit(digit);
    }
    params
}
