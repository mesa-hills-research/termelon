//! A byte parser in the shape of Paul Williams' VT500 state machine.
//!
//! It decodes UTF-8 in the ground state and splits everything else into escape sequences, control
//! sequences with their private marker, parameters and intermediates, and the string sequences (OSC,
//! DCS, APC, PM and SOS). It holds at most [`MAX_PARAMS`] parameters, [`MAX_INTERMEDIATES`]
//! intermediates and [`MAX_STRING`] bytes of a string. A sequence that goes past a bound is consumed
//! to its end and reported as invalid, so the parser's memory stays bounded on any input.

/// The most parameters a control sequence holds, sub-parameters included.
pub(crate) const MAX_PARAMS: usize = 32;
/// The most intermediate bytes a sequence holds.
pub(crate) const MAX_INTERMEDIATES: usize = 2;
/// The most bytes of an OSC, DCS, APC, PM or SOS string.
pub(crate) const MAX_STRING: usize = 64 * 1024;
/// The most bytes of a sequence kept for an error report.
const MAX_RAW: usize = 256;

const ESC: u8 = 0x1b;
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;
const BEL: u8 = 0x07;
const DEL: u8 = 0x7f;

/// Parameters of a control sequence. Each value is a parameter or a sub-parameter of the one
/// before it (written with `:`). An empty parameter reads as 0.
#[derive(Clone, Debug, Default)]
pub(crate) struct Params {
    values: [u16; MAX_PARAMS],
    sub: [bool; MAX_PARAMS],
    len: usize,
}

impl Params {
    /// The number of values, sub-parameters included.
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Whether the sequence has no parameters at all.
    pub(crate) fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The value at an index, 0 when it's missing or empty.
    pub(crate) fn get(&self, index: usize) -> u16 {
        if index < self.len {
            self.values[index]
        } else {
            0
        }
    }

    /// Whether any parameter has sub-parameters.
    pub(crate) fn has_subparams(&self) -> bool {
        self.sub[..self.len].iter().any(|&sub| sub)
    }

    /// The parameters, each a slice of its value followed by its sub-parameters.
    pub(crate) fn groups(&self) -> Groups<'_> {
        Groups {
            params: self,
            at: 0,
        }
    }

    /// The parameters' first values only, as a list.
    pub(crate) fn values(&self) -> impl Iterator<Item = u16> + '_ {
        self.groups()
            .map(|group| group.first().copied().unwrap_or(0))
    }

    fn clear(&mut self) {
        self.len = 0;
    }

    /// Adds a value. Returns false when the list is full.
    fn push(&mut self, value: u16, sub: bool) -> bool {
        if self.len == MAX_PARAMS {
            return false;
        }
        self.values[self.len] = value;
        self.sub[self.len] = sub;
        self.len += 1;
        true
    }
}

/// An iterator over parameter groups, a value and its sub-parameters.
#[derive(Debug)]
pub(crate) struct Groups<'a> {
    params: &'a Params,
    at: usize,
}

impl<'a> Iterator for Groups<'a> {
    type Item = &'a [u16];

    fn next(&mut self) -> Option<&'a [u16]> {
        let params = self.params;
        if self.at >= params.len {
            return None;
        }
        let start = self.at;
        let mut end = start + 1;
        while end < params.len && params.sub[end] {
            end += 1;
        }
        self.at = end;
        Some(&params.values[start..end])
    }
}

/// A control sequence, `CSI` with its private marker, parameters, intermediates and final byte.
#[derive(Debug)]
pub(crate) struct Csi<'a> {
    /// `?`, `>`, `<` or `=`, when the sequence starts with one.
    pub(crate) private: Option<u8>,
    pub(crate) params: &'a Params,
    pub(crate) intermediates: &'a [u8],
    pub(crate) final_byte: u8,
}

/// A device control string, with its header and data.
#[derive(Debug)]
pub(crate) struct Dcs<'a> {
    pub(crate) private: Option<u8>,
    pub(crate) params: &'a Params,
    pub(crate) intermediates: &'a [u8],
    pub(crate) final_byte: u8,
    pub(crate) data: &'a [u8],
}

/// How a string sequence ended, which a reply copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Terminator {
    Bel,
    St,
}

/// The kind of a string sequence other than OSC and DCS.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StringKind {
    Apc,
    Pm,
    Sos,
}

/// What the parser found, handed to the emulator one at a time.
pub(crate) trait Perform {
    /// A character to draw.
    fn print(&mut self, c: char);
    /// A C0 control.
    fn execute(&mut self, byte: u8);
    /// An escape sequence, `ESC` with intermediates and a final byte.
    fn esc_dispatch(&mut self, intermediates: &[u8], final_byte: u8, raw: &[u8]);
    /// A control sequence.
    fn csi_dispatch(&mut self, csi: &Csi<'_>, raw: &[u8]);
    /// An operating system command, its data without `ESC ]` and the terminator.
    fn osc_dispatch(&mut self, data: &[u8], terminator: Terminator, raw: &[u8]);
    /// A device control string.
    fn dcs_dispatch(&mut self, dcs: &Dcs<'_>, raw: &[u8]);
    /// An APC, PM or SOS string.
    fn string_dispatch(&mut self, kind: StringKind, data: &[u8], raw: &[u8]);
    /// A single shift 3 (`ESC O`) and the byte after it.
    fn ss3_dispatch(&mut self, byte: u8, raw: &[u8]);
    /// The Linux console's palette commands, `ESC ] P` (or `ESC P`) with seven hex digits, and
    /// `ESC ] R`, which resets the palette.
    fn palette_dispatch(&mut self, digits: Option<&[u8]>, raw: &[u8]);
    /// Bytes that form no valid sequence: invalid UTF-8, a sequence with a byte it can't hold, or
    /// one past the parser's bounds.
    fn invalid(&mut self, raw: &[u8]);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    CsiIgnore,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsPassthrough,
    DcsIgnore,
    OscString,
    String(StringKind),
    Ss3,
    /// The Linux console after `ESC ]`.
    LinuxNonstd,
    /// The Linux console reading a palette command's hex digits.
    LinuxPalette,
}

/// The parser. It keeps its state between calls, so a sequence may be split across them.
#[derive(Clone, Debug)]
pub(crate) struct Parser {
    state: State,
    /// Whether `ESC ]` and `ESC P` follow the Linux console's rules.
    linux: bool,
    params: Params,
    /// The parameter being read.
    current: u32,
    /// Whether the parameter being read had a digit or ended with a separator.
    current_started: bool,
    /// Whether the parameter being read follows a `:`.
    current_sub: bool,
    private: Option<u8>,
    intermediates: [u8; MAX_INTERMEDIATES],
    intermediates_len: usize,
    /// Whether the current sequence went past a bound or held a byte it can't.
    overflow: bool,
    string: Vec<u8>,
    raw: Vec<u8>,
    raw_truncated: bool,
    /// An OSC that ended at `ESC`, so a following `\` belongs to it.
    after_string_esc: bool,
    utf8: Utf8,
    palette: [u8; 7],
    palette_len: usize,
}

impl Parser {
    pub(crate) fn new(linux: bool) -> Self {
        Self {
            state: State::Ground,
            linux,
            params: Params::default(),
            current: 0,
            current_started: false,
            current_sub: false,
            private: None,
            intermediates: [0; MAX_INTERMEDIATES],
            intermediates_len: 0,
            overflow: false,
            string: Vec::new(),
            raw: Vec::new(),
            raw_truncated: false,
            after_string_esc: false,
            utf8: Utf8::default(),
            palette: [0; 7],
            palette_len: 0,
        }
    }

    /// Whether the parser is in the middle of a sequence or a UTF-8 character.
    pub(crate) fn is_pending(&self) -> bool {
        self.state != State::Ground || self.utf8.need > 0
    }

    /// Parses bytes, handing what it finds to `perform`.
    pub(crate) fn advance(&mut self, perform: &mut impl Perform, bytes: &[u8]) {
        for &byte in bytes {
            self.byte(perform, byte);
        }
    }

    fn byte(&mut self, perform: &mut impl Perform, byte: u8) {
        // A partial UTF-8 character ends at any byte that can't continue it.
        if self.state == State::Ground && self.utf8.need > 0 {
            if (0x80..=0xbf).contains(&byte) && self.utf8.accepts(byte) {
                if let Some(c) = self.utf8.push(byte) {
                    perform.print(c);
                }
                return;
            }
            let bad = self.utf8.take();
            perform.invalid(&bad);
            perform.print(char::REPLACEMENT_CHARACTER);
        }

        let after_esc = std::mem::take(&mut self.after_string_esc);

        // Bytes that act the same in every state but the strings and the console's palette.
        match self.state {
            State::Ground => return self.ground(perform, byte),
            State::LinuxNonstd | State::LinuxPalette => return self.linux_string(perform, byte),
            State::OscString | State::String(_) | State::DcsPassthrough | State::DcsIgnore => {
                return self.string_byte(perform, byte);
            }
            _ => {}
        }
        match byte {
            CAN | SUB => {
                self.record(byte);
                let raw = self.take_raw();
                perform.invalid(&raw);
                self.state = State::Ground;
                return;
            }
            ESC => {
                // A sequence cut short by a new one, unless this follows the `ESC` that ended a
                // string.
                if !(self.state == State::Escape && after_esc) {
                    let raw = self.take_raw();
                    perform.invalid(&raw);
                }
                self.enter_escape();
                return;
            }
            0x00..=0x1f => {
                perform.execute(byte);
                return;
            }
            DEL => return,
            0x80..=0xff => {
                // Not part of any sequence. The sequence is dropped and the byte starts afresh.
                let raw = self.take_raw();
                perform.invalid(&raw);
                self.state = State::Ground;
                return self.ground(perform, byte);
            }
            _ => {}
        }
        self.record(byte);
        match self.state {
            State::Escape => self.escape(perform, byte, after_esc),
            State::EscapeIntermediate => match byte {
                0x20..=0x2f => self.collect(byte),
                _ => self.esc_dispatch(perform, byte),
            },
            State::CsiEntry => match byte {
                0x20..=0x2f => {
                    self.collect(byte);
                    self.state = State::CsiIntermediate;
                }
                b'0'..=b'9' | b';' | b':' => {
                    self.param(byte);
                    self.state = State::CsiParam;
                }
                0x3c..=0x3f => {
                    self.private = Some(byte);
                    self.state = State::CsiParam;
                }
                _ => self.csi_dispatch(perform, byte),
            },
            State::CsiParam => match byte {
                b'0'..=b'9' | b';' | b':' => self.param(byte),
                0x3c..=0x3f => self.state = State::CsiIgnore,
                0x20..=0x2f => {
                    self.collect(byte);
                    self.state = State::CsiIntermediate;
                }
                _ => self.csi_dispatch(perform, byte),
            },
            State::CsiIntermediate => match byte {
                0x20..=0x2f => self.collect(byte),
                0x30..=0x3f => self.state = State::CsiIgnore,
                _ => self.csi_dispatch(perform, byte),
            },
            State::CsiIgnore => {
                if (0x40..=0x7e).contains(&byte) {
                    let raw = self.take_raw();
                    perform.invalid(&raw);
                    self.state = State::Ground;
                }
            }
            State::DcsEntry => match byte {
                0x20..=0x2f => {
                    self.collect(byte);
                    self.state = State::DcsIntermediate;
                }
                b'0'..=b'9' | b';' | b':' => {
                    self.param(byte);
                    self.state = State::DcsParam;
                }
                0x3c..=0x3f => {
                    self.private = Some(byte);
                    self.state = State::DcsParam;
                }
                _ => self.dcs_hook(byte),
            },
            State::DcsParam => match byte {
                b'0'..=b'9' | b';' | b':' => self.param(byte),
                0x3c..=0x3f => self.state = State::DcsIgnore,
                0x20..=0x2f => {
                    self.collect(byte);
                    self.state = State::DcsIntermediate;
                }
                _ => self.dcs_hook(byte),
            },
            State::DcsIntermediate => match byte {
                0x20..=0x2f => self.collect(byte),
                0x30..=0x3f => self.state = State::DcsIgnore,
                _ => self.dcs_hook(byte),
            },
            State::Ss3 => {
                let raw = self.take_raw();
                perform.ss3_dispatch(byte, &raw);
                self.state = State::Ground;
            }
            State::Ground
            | State::OscString
            | State::String(_)
            | State::DcsPassthrough
            | State::DcsIgnore
            | State::LinuxNonstd
            | State::LinuxPalette => {}
        }
    }

    fn ground(&mut self, perform: &mut impl Perform, byte: u8) {
        match byte {
            ESC => self.enter_escape(),
            0x00..=0x1f => perform.execute(byte),
            0x20..=0x7e => perform.print(char::from(byte)),
            DEL => {}
            _ => match self.utf8.start(byte) {
                Some(c) => perform.print(c),
                None if self.utf8.need > 0 => {}
                None => {
                    perform.invalid(&[byte]);
                    perform.print(char::REPLACEMENT_CHARACTER);
                }
            },
        }
    }

    fn escape(&mut self, perform: &mut impl Perform, byte: u8, after_string: bool) {
        match byte {
            0x20..=0x2f => {
                self.collect(byte);
                self.state = State::EscapeIntermediate;
            }
            b'[' => self.state = State::CsiEntry,
            b']' if self.linux => self.state = State::LinuxNonstd,
            b'P' if self.linux => {
                self.palette_len = 0;
                self.state = State::LinuxPalette;
            }
            b']' => {
                self.string.clear();
                self.state = State::OscString;
            }
            b'P' => self.state = State::DcsEntry,
            // The Linux console has no string sequences but its palette commands.
            b'_' | b'^' | b'X' if self.linux => self.esc_dispatch(perform, byte),
            b'_' => self.enter_string(StringKind::Apc),
            b'^' => self.enter_string(StringKind::Pm),
            b'X' => self.enter_string(StringKind::Sos),
            b'O' => self.state = State::Ss3,
            b'\\' if after_string => {
                // The `\` of an ST that ended a string, which was handled at its `ESC`.
                self.state = State::Ground;
                self.raw.clear();
            }
            _ => self.esc_dispatch(perform, byte),
        }
    }

    fn enter_escape(&mut self) {
        self.state = State::Escape;
        self.params.clear();
        self.current = 0;
        self.current_started = false;
        self.current_sub = false;
        self.private = None;
        self.intermediates_len = 0;
        self.overflow = false;
        self.raw.clear();
        self.raw_truncated = false;
        self.raw.push(ESC);
    }

    fn enter_string(&mut self, kind: StringKind) {
        self.string.clear();
        self.state = State::String(kind);
    }

    fn record(&mut self, byte: u8) {
        if self.raw.len() < MAX_RAW {
            self.raw.push(byte);
        } else {
            self.raw_truncated = true;
        }
    }

    /// The bytes of the current sequence, for a report.
    fn take_raw(&mut self) -> Vec<u8> {
        let mut raw = std::mem::take(&mut self.raw);
        if self.raw_truncated {
            raw.extend_from_slice("…".as_bytes());
        }
        raw
    }

    fn collect(&mut self, byte: u8) {
        if self.intermediates_len < MAX_INTERMEDIATES {
            self.intermediates[self.intermediates_len] = byte;
            self.intermediates_len += 1;
        } else {
            self.overflow = true;
        }
    }

    fn param(&mut self, byte: u8) {
        match byte {
            b';' | b':' => {
                self.finish_param();
                self.current_started = true;
                self.current_sub = byte == b':';
            }
            _ => {
                self.current =
                    (self.current * 10 + u32::from(byte - b'0')).min(u32::from(u16::MAX));
                self.current_started = true;
            }
        }
    }

    fn finish_param(&mut self) {
        let value = u16::try_from(self.current).unwrap_or(u16::MAX);
        if !self.params.push(value, self.current_sub) {
            self.overflow = true;
        }
        self.current = 0;
        self.current_started = false;
        self.current_sub = false;
    }

    fn end_params(&mut self) {
        if self.current_started {
            self.finish_param();
        }
    }

    fn esc_dispatch(&mut self, perform: &mut impl Perform, byte: u8) {
        let raw = self.take_raw();
        self.state = State::Ground;
        if self.overflow {
            perform.invalid(&raw);
        } else {
            perform.esc_dispatch(&self.intermediates[..self.intermediates_len], byte, &raw);
        }
    }

    fn csi_dispatch(&mut self, perform: &mut impl Perform, byte: u8) {
        self.end_params();
        let raw = self.take_raw();
        self.state = State::Ground;
        if self.overflow {
            perform.invalid(&raw);
            return;
        }
        let csi = Csi {
            private: self.private,
            params: &self.params,
            intermediates: &self.intermediates[..self.intermediates_len],
            final_byte: byte,
        };
        perform.csi_dispatch(&csi, &raw);
    }

    fn dcs_hook(&mut self, byte: u8) {
        self.end_params();
        // The final byte is kept as the first byte of the string, and split off at the end.
        self.string.clear();
        self.string.push(byte);
        self.state = State::DcsPassthrough;
    }

    /// A byte inside an OSC, DCS, APC, PM or SOS string.
    fn string_byte(&mut self, perform: &mut impl Perform, byte: u8) {
        match byte {
            CAN | SUB => {
                self.record(byte);
                let raw = self.take_raw();
                perform.invalid(&raw);
                self.state = State::Ground;
            }
            ESC => {
                self.end_string(perform, Terminator::St);
                self.enter_escape();
                self.after_string_esc = true;
            }
            BEL if self.state == State::OscString => {
                self.record(byte);
                self.end_string(perform, Terminator::Bel);
                self.state = State::Ground;
            }
            _ => {
                self.record(byte);
                let keep = match self.state {
                    State::OscString | State::String(_) => byte >= 0x20 || byte == b'\t',
                    State::DcsPassthrough => byte != DEL,
                    _ => false,
                };
                if keep {
                    if self.string.len() < MAX_STRING {
                        self.string.push(byte);
                    } else {
                        self.overflow = true;
                    }
                }
            }
        }
    }

    fn end_string(&mut self, perform: &mut impl Perform, terminator: Terminator) {
        let mut raw = self.take_raw();
        if terminator == Terminator::St {
            raw.extend_from_slice(b"\x1b\\");
        }
        let state = std::mem::replace(&mut self.state, State::Ground);
        if self.overflow {
            perform.invalid(&raw);
            return;
        }
        match state {
            State::OscString => perform.osc_dispatch(&self.string, terminator, &raw),
            State::String(kind) => perform.string_dispatch(kind, &self.string, &raw),
            State::DcsPassthrough => {
                let (final_byte, data) = match self.string.split_first() {
                    Some((&final_byte, data)) => (final_byte, data),
                    None => (0, &[][..]),
                };
                let dcs = Dcs {
                    private: self.private,
                    params: &self.params,
                    intermediates: &self.intermediates[..self.intermediates_len],
                    final_byte,
                    data,
                };
                perform.dcs_dispatch(&dcs, &raw);
            }
            _ => perform.invalid(&raw),
        }
    }

    /// The Linux console after `ESC ]` or `ESC P`. A palette command reads seven hex digits, and
    /// any other byte ends the escape there, so what follows prints as text.
    fn linux_string(&mut self, perform: &mut impl Perform, byte: u8) {
        if byte == ESC {
            let raw = self.take_raw();
            perform.invalid(&raw);
            self.enter_escape();
            return;
        }
        self.record(byte);
        if self.state == State::LinuxNonstd {
            match byte {
                b'P' => {
                    self.palette_len = 0;
                    self.state = State::LinuxPalette;
                }
                b'R' => {
                    let raw = self.take_raw();
                    perform.palette_dispatch(None, &raw);
                    self.state = State::Ground;
                }
                _ => {
                    let raw = self.take_raw();
                    perform.invalid(&raw);
                    self.state = State::Ground;
                }
            }
            return;
        }
        if byte.is_ascii_hexdigit() {
            self.palette[self.palette_len] = byte;
            self.palette_len += 1;
            if self.palette_len == self.palette.len() {
                let raw = self.take_raw();
                let digits = self.palette;
                perform.palette_dispatch(Some(&digits), &raw);
                self.state = State::Ground;
            }
        } else {
            let raw = self.take_raw();
            perform.invalid(&raw);
            self.state = State::Ground;
        }
    }
}

/// A UTF-8 decoder that checks each byte as it comes, so it rejects overlong forms, surrogates and
/// values past U+10FFFF at the first byte that shows them.
#[derive(Clone, Debug, Default)]
struct Utf8 {
    bytes: [u8; 4],
    len: usize,
    need: usize,
    value: u32,
}

impl Utf8 {
    /// Starts a character with a byte of 0x80 or more. Returns nothing while it needs more bytes,
    /// and nothing with `need` left at 0 when the byte can't start a character.
    fn start(&mut self, byte: u8) -> Option<char> {
        let (need, value) = match byte {
            0xc2..=0xdf => (1, u32::from(byte & 0x1f)),
            0xe0..=0xef => (2, u32::from(byte & 0x0f)),
            0xf0..=0xf4 => (3, u32::from(byte & 0x07)),
            _ => return None,
        };
        self.bytes[0] = byte;
        self.len = 1;
        self.need = need;
        self.value = value;
        None
    }

    /// Whether a continuation byte may follow what's been read.
    fn accepts(&self, byte: u8) -> bool {
        if self.len != 1 {
            return true;
        }
        match self.bytes[0] {
            0xe0 => byte >= 0xa0,
            0xed => byte <= 0x9f,
            0xf0 => byte >= 0x90,
            0xf4 => byte <= 0x8f,
            _ => true,
        }
    }

    fn push(&mut self, byte: u8) -> Option<char> {
        self.bytes[self.len] = byte;
        self.len += 1;
        self.value = (self.value << 6) | u32::from(byte & 0x3f);
        self.need -= 1;
        if self.need > 0 {
            return None;
        }
        self.len = 0;
        char::from_u32(self.value)
    }

    /// The bytes of a character cut short.
    fn take(&mut self) -> Vec<u8> {
        let bytes = self.bytes[..self.len].to_vec();
        self.len = 0;
        self.need = 0;
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Default)]
    struct Log(Vec<String>);

    impl Perform for Log {
        fn print(&mut self, c: char) {
            self.0.push(format!("print {c}"));
        }
        fn execute(&mut self, byte: u8) {
            self.0.push(format!("execute {byte:02x}"));
        }
        fn esc_dispatch(&mut self, intermediates: &[u8], final_byte: u8, _raw: &[u8]) {
            self.0.push(format!(
                "esc {} {}",
                String::from_utf8_lossy(intermediates),
                char::from(final_byte)
            ));
        }
        fn csi_dispatch(&mut self, csi: &Csi<'_>, _raw: &[u8]) {
            let groups: Vec<String> = csi
                .params
                .groups()
                .map(|g| g.iter().map(u16::to_string).collect::<Vec<_>>().join(":"))
                .collect();
            self.0.push(format!(
                "csi {} [{}] {} {}",
                csi.private.map_or('-', char::from),
                groups.join(";"),
                String::from_utf8_lossy(csi.intermediates),
                char::from(csi.final_byte)
            ));
        }
        fn osc_dispatch(&mut self, data: &[u8], terminator: Terminator, _raw: &[u8]) {
            self.0.push(format!(
                "osc {} {terminator:?}",
                String::from_utf8_lossy(data)
            ));
        }
        fn dcs_dispatch(&mut self, dcs: &Dcs<'_>, _raw: &[u8]) {
            self.0.push(format!(
                "dcs {} {} {}",
                String::from_utf8_lossy(dcs.intermediates),
                char::from(dcs.final_byte),
                String::from_utf8_lossy(dcs.data)
            ));
        }
        fn string_dispatch(&mut self, kind: StringKind, data: &[u8], _raw: &[u8]) {
            self.0
                .push(format!("{kind:?} {}", String::from_utf8_lossy(data)));
        }
        fn ss3_dispatch(&mut self, byte: u8, _raw: &[u8]) {
            self.0.push(format!("ss3 {}", char::from(byte)));
        }
        fn palette_dispatch(&mut self, digits: Option<&[u8]>, _raw: &[u8]) {
            self.0.push(format!(
                "palette {}",
                String::from_utf8_lossy(digits.unwrap_or(b"reset"))
            ));
        }
        fn invalid(&mut self, raw: &[u8]) {
            self.0.push(format!("invalid {raw:02x?}"));
        }
    }

    fn parse(bytes: &[u8]) -> Vec<String> {
        let mut parser = Parser::new(false);
        let mut log = Log::default();
        parser.advance(&mut log, bytes);
        log.0
    }

    #[test]
    fn prints_ascii_and_utf8() {
        assert_eq!(
            parse("aé字🙂".as_bytes()),
            ["print a", "print é", "print 字", "print 🙂"]
        );
    }

    #[test]
    fn utf8_split_across_calls() {
        let mut parser = Parser::new(false);
        let mut log = Log::default();
        let bytes = "字".as_bytes();
        parser.advance(&mut log, &bytes[..1]);
        assert!(parser.is_pending());
        parser.advance(&mut log, &bytes[1..]);
        assert_eq!(log.0, ["print 字"]);
    }

    #[test]
    fn invalid_utf8_prints_a_replacement() {
        assert_eq!(parse(b"\xff"), ["invalid [ff]", "print \u{fffd}"]);
        assert_eq!(
            parse(b"\xe5\x41"),
            ["invalid [e5]", "print \u{fffd}", "print A"]
        );
        // An overlong form and a surrogate fail at their second byte.
        assert_eq!(
            parse(b"\xe0\x80"),
            [
                "invalid [e0]",
                "print \u{fffd}",
                "invalid [80]",
                "print \u{fffd}"
            ]
        );
        assert_eq!(
            parse(b"\xed\xa0"),
            [
                "invalid [ed]",
                "print \u{fffd}",
                "invalid [a0]",
                "print \u{fffd}"
            ]
        );
    }

    #[test]
    fn csi_with_private_marker_params_and_intermediates() {
        assert_eq!(parse(b"\x1b[?1049h"), ["csi ? [1049]  h"]);
        assert_eq!(parse(b"\x1b[1;2H"), ["csi - [1;2]  H"]);
        assert_eq!(parse(b"\x1b[;H"), ["csi - [0;0]  H"]);
        assert_eq!(parse(b"\x1b[m"), ["csi - []  m"]);
        assert_eq!(parse(b"\x1b[2 q"), ["csi - [2]   q"]);
        assert_eq!(parse(b"\x1b[?2026$p"), ["csi ? [2026] $ p"]);
    }

    #[test]
    fn colon_sub_parameters_group_with_their_parameter() {
        assert_eq!(
            parse(b"\x1b[38:2::10:20:30;1m"),
            ["csi - [38:2:0:10:20:30;1]  m"]
        );
        assert_eq!(parse(b"\x1b[4:3m"), ["csi - [4:3]  m"]);
    }

    #[test]
    fn controls_inside_a_sequence_execute() {
        assert_eq!(parse(b"\x1b[1\n;2H"), ["execute 0a", "csi - [1;2]  H"]);
    }

    #[test]
    fn can_and_sub_cancel_a_sequence() {
        assert_eq!(
            parse(b"\x1b[12\x18A"),
            ["invalid [1b, 5b, 31, 32, 18]", "print A"]
        );
    }

    #[test]
    fn a_new_escape_cuts_a_sequence_short() {
        assert_eq!(
            parse(b"\x1b[1\x1b[2J"),
            ["invalid [1b, 5b, 31]", "csi - [2]  J"]
        );
    }

    #[test]
    fn private_marker_after_params_is_ignored_to_the_final_byte() {
        assert_eq!(
            parse(b"\x1b[1?hA"),
            ["invalid [1b, 5b, 31, 3f, 68]", "print A"]
        );
    }

    #[test]
    fn too_many_params_are_invalid() {
        let mut bytes = b"\x1b[".to_vec();
        for _ in 0..40 {
            bytes.extend_from_slice(b"1;");
        }
        bytes.push(b'm');
        let log = parse(&bytes);
        assert_eq!(log.len(), 1);
        assert!(log[0].starts_with("invalid"));
    }

    #[test]
    fn parameter_values_saturate() {
        assert_eq!(parse(b"\x1b[99999999C"), ["csi - [65535]  C"]);
    }

    #[test]
    fn esc_sequences() {
        assert_eq!(parse(b"\x1b7\x1b(B\x1bM"), ["esc  7", "esc ( B", "esc  M"]);
    }

    #[test]
    fn osc_ends_at_bel_or_st() {
        assert_eq!(parse(b"\x1b]0;hi\x07"), ["osc 0;hi Bel"]);
        assert_eq!(
            parse(b"\x1b]2;t\xc3\xa9\x1b\\A"),
            ["osc 2;té St", "print A"]
        );
    }

    #[test]
    fn dcs_and_apc_strings() {
        assert_eq!(parse(b"\x1bP+q5463\x1b\\"), ["dcs + q 5463"]);
        assert_eq!(parse(b"\x1b_Gx\x1b\\"), ["Apc Gx"]);
        assert_eq!(parse(b"\x1bOA"), ["ss3 A"]);
    }

    #[test]
    fn oversized_strings_are_invalid_and_bounded() {
        let mut bytes = b"\x1b]2;".to_vec();
        bytes.extend(std::iter::repeat_n(b'x', MAX_STRING + 10));
        bytes.extend_from_slice(b"\x07A");
        let log = parse(&bytes);
        assert_eq!(log.len(), 2);
        assert!(log[0].starts_with("invalid"));
        assert_eq!(log[1], "print A");
    }

    #[test]
    fn linux_console_strings() {
        let mut parser = Parser::new(true);
        let mut log = Log::default();
        parser.advance(&mut log, b"\x1b]P0ff0000\x1b]R\x1b]11;?\x07");
        assert_eq!(
            log.0,
            [
                "palette 0ff0000",
                "palette reset",
                "invalid [1b, 5d, 31]",
                "print 1",
                "print ;",
                "print ?",
                "execute 07"
            ]
        );
    }
}
