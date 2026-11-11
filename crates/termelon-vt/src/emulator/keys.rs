//! Encoding input the way the profile's terminal sends it, by the modes the program set.

use std::fmt::Write as _;

use super::{Emulator, MouseTracking};
use crate::input::{Key, KeyKind, Mods, Mouse, MouseButton, MouseKind};
use crate::profile::KeyEncoding;

const ESC: u8 = 0x1b;

/// The kitty protocol's progressive enhancement flags.
const DISAMBIGUATE: u8 = 1;
const REPORT_EVENTS: u8 = 2;
const ALTERNATES: u8 = 4;
const ALL_KEYS: u8 = 8;
const TEXT: u8 = 16;

impl Emulator {
    /// The bytes the terminal sends for a key press.
    ///
    /// They follow the kitty keyboard flags the program pushed, then modifyOtherKeys if it set
    /// that, and the terminal's legacy encodings otherwise, which take application cursor keys
    /// (mode 1) into account.
    pub fn encode_key(&self, key: Key, mods: Mods) -> Vec<u8> {
        self.encode_key_event(key, mods, KeyKind::Press)
    }

    /// The bytes the terminal sends for a key press, repeat or release. Without the kitty
    /// protocol's flag 2, a repeat sends what a press does and a release sends nothing.
    pub fn encode_key_event(&self, key: Key, mods: Mods, kind: KeyKind) -> Vec<u8> {
        if self.profile.bound_keys.contains(&(key, mods)) {
            return Vec::new();
        }
        let flags = if self.profile.kitty_keyboard {
            self.keyboard_flags()
        } else {
            0
        };
        if flags != 0 {
            return self.kitty_key(key, mods, kind, flags);
        }
        if kind == KeyKind::Release {
            return Vec::new();
        }
        if self.profile.keys == KeyEncoding::Tmux34 {
            return self.tmux_3_4_key(key, mods);
        }
        if self.profile.modify_other_keys
            && let Some(bytes) = self.modify_other_key(key, mods)
        {
            return bytes;
        }
        self.legacy_key(key, mods)
    }

    /// The bytes the terminal sends for a mouse event, by the mouse modes the program set: none
    /// without one, SGR reports with mode 1006, and X10 reports otherwise. An X10 report can't
    /// hold a column or row past 222, so there it sends nothing.
    pub fn encode_mouse(&self, mouse: Mouse) -> Vec<u8> {
        let reported = match (self.mouse, mouse.kind) {
            (MouseTracking::Off, _) => false,
            (MouseTracking::X10, kind) => matches!(kind, MouseKind::Press(_)),
            (MouseTracking::Normal, kind) => !matches!(kind, MouseKind::Drag(_) | MouseKind::Move),
            (MouseTracking::Button, kind) => kind != MouseKind::Move,
            (MouseTracking::Any, _) => true,
        };
        if !reported {
            return Vec::new();
        }
        let button = |b: MouseButton| match b {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
        };
        let mut code: u32 = match mouse.kind {
            MouseKind::Press(b) | MouseKind::Release(b) => button(b),
            MouseKind::Drag(b) => button(b) + 32,
            MouseKind::Move => 35,
            MouseKind::ScrollUp => 64,
            MouseKind::ScrollDown => 65,
            MouseKind::ScrollLeft => 66,
            MouseKind::ScrollRight => 67,
        };
        if self.mouse != MouseTracking::X10 {
            for (held, bit) in [(Mods::SHIFT, 4), (Mods::ALT, 8), (Mods::CTRL, 16)] {
                if mouse.mods.contains(held) {
                    code += bit;
                }
            }
        }
        let release = matches!(mouse.kind, MouseKind::Release(_));
        let (x, y) = (u32::from(mouse.x) + 1, u32::from(mouse.y) + 1);
        if self.sgr_mouse {
            return format!("\x1b[<{code};{x};{y}{}", if release { 'm' } else { 'M' }).into_bytes();
        }
        if release {
            // X10 reports don't say which button came up.
            code = (code & !3) | 3;
        }
        let encoded: Option<Vec<u8>> = [code, x, y]
            .iter()
            .map(|&v| u8::try_from(v + 32).ok())
            .collect();
        match encoded {
            Some(bytes) => [b"\x1b[M".as_slice(), &bytes].concat(),
            None => Vec::new(),
        }
    }

    /// The bytes the terminal sends for a paste, bracketed with mode 2004. Line ends become CR,
    /// as terminals paste them.
    pub fn encode_paste(&self, text: &str) -> Vec<u8> {
        let body = text.replace("\r\n", "\r").replace('\n', "\r");
        if self.bracketed_paste {
            format!("\x1b[200~{body}\x1b[201~").into_bytes()
        } else {
            body.into_bytes()
        }
    }

    /// The bytes the terminal sends when its window gains or loses focus, nothing without mode
    /// 1004.
    pub fn encode_focus(&self, focused: bool) -> Vec<u8> {
        match (self.focus_reports, focused) {
            (false, _) => Vec::new(),
            (true, true) => b"\x1b[I".to_vec(),
            (true, false) => b"\x1b[O".to_vec(),
        }
    }

    /// xterm's encodings, with the profile's differences.
    fn legacy_key(&self, key: Key, mods: Mods) -> Vec<u8> {
        let linux = self.profile.keys == KeyEncoding::LinuxConsole;
        let param = if mods.is_empty() || linux {
            None
        } else {
            Some(u32::from(mods.bits()) + 1)
        };
        let letter = |final_byte: u8| -> Vec<u8> {
            match param {
                Some(m) => format!("\x1b[1;{m}{}", char::from(final_byte)).into_bytes(),
                None if self.app_cursor => vec![ESC, b'O', final_byte],
                None => vec![ESC, b'[', final_byte],
            }
        };
        let tilde = |n: u32| -> Vec<u8> {
            match param {
                Some(m) => format!("\x1b[{n};{m}~").into_bytes(),
                None => format!("\x1b[{n}~").into_bytes(),
            }
        };
        let alt = |bytes: Vec<u8>| -> Vec<u8> {
            if mods.contains(Mods::ALT) {
                [&[ESC][..], &bytes].concat()
            } else {
                bytes
            }
        };
        let home_end_tilde = param.is_none()
            && matches!(
                self.profile.keys,
                KeyEncoding::Tmux | KeyEncoding::Tmux34 | KeyEncoding::LinuxConsole
            );
        let konsole = self.profile.keys == KeyEncoding::Konsole;
        let scrolls = konsole
            && !self.alternate_screen()
            && mods.contains(Mods::SHIFT)
            && match key {
                Key::Up | Key::Down => !mods.intersects(Mods::ALT | Mods::CTRL),
                Key::PageUp | Key::PageDown | Key::Home | Key::End => true,
                _ => false,
            };
        match key {
            _ if scrolls => Vec::new(),
            Key::Enter if konsole && mods.contains(Mods::SHIFT) => alt(b"\x1bOM".to_vec()),
            Key::F(n @ 1..=4) if konsole && param.is_some() => {
                let m = param.unwrap_or(1);
                format!("\x1bO{m}{}", char::from(b'P' + (n - 1))).into_bytes()
            }
            Key::Up => letter(b'A'),
            Key::Down => letter(b'B'),
            Key::Right => letter(b'C'),
            Key::Left => letter(b'D'),
            Key::Home if home_end_tilde => tilde(1),
            Key::End if home_end_tilde => tilde(4),
            Key::Home => letter(b'H'),
            Key::End => letter(b'F'),
            Key::Insert => tilde(2),
            Key::Delete => tilde(3),
            Key::PageUp => tilde(5),
            Key::PageDown => tilde(6),
            Key::F(n) if linux && (1..=5).contains(&n) => vec![ESC, b'[', b'[', b'A' + (n - 1)],
            Key::F(n @ 1..=4) => match param {
                Some(m) => format!("\x1b[1;{m}{}", char::from(b'P' + (n - 1))).into_bytes(),
                None => vec![ESC, b'O', b'P' + (n - 1)],
            },
            Key::F(n @ 5..=12) => tilde(function_key_number(n)),
            Key::F(_) => Vec::new(),
            Key::Enter => alt(b"\r".to_vec()),
            Key::Tab if mods.contains(Mods::SHIFT) => alt(b"\x1b[Z".to_vec()),
            Key::Tab => alt(b"\t".to_vec()),
            Key::Backspace if mods.contains(Mods::CTRL) => alt(vec![0x08]),
            Key::Backspace => alt(vec![0x7f]),
            Key::Escape => alt(vec![ESC]),
            Key::Char(c) => {
                let typed = typed_char(c, mods);
                let bytes = match control_byte(c) {
                    Some(byte) if mods.contains(Mods::CTRL) => vec![byte],
                    _ => typed.to_string().into_bytes(),
                };
                alt(bytes)
            }
        }
    }

    /// modifyOtherKeys, as `CSI 27 ; mods ; code ~`. Level 2 sends it for every key with a
    /// modifier, Shift on a character too, but Shift+Tab, which stays `CSI Z`, and Ctrl+Backspace,
    /// which stays BS and is BS's code with other modifiers, as xterm 390 does (`ModifyOtherKeys` in
    /// its `input.c`, and a run on Ubuntu 24.04). Level 1 sends it only where the legacy bytes would
    /// lose a modifier.
    fn modify_other_key(&self, key: Key, mods: Mods) -> Option<Vec<u8>> {
        if self.modify_other_keys == 0 || mods.is_empty() {
            return None;
        }
        let level_2 = self.modify_other_keys >= 2;
        let code = match key {
            Key::Enter => 13,
            Key::Tab => 9,
            Key::Backspace if level_2 && mods.contains(Mods::CTRL) => 8,
            Key::Backspace => 127,
            Key::Escape => 27,
            Key::Char(c) => u32::from(typed_char(c, mods)),
            _ => return None,
        };
        let lost = match key {
            Key::Enter => mods != Mods::ALT,
            Key::Tab => mods.contains(Mods::CTRL),
            Key::Backspace => mods.contains(Mods::SHIFT),
            Key::Escape => mods.intersects(Mods::CTRL | Mods::SHIFT),
            Key::Char(c) => {
                mods.contains(Mods::SUPER)
                    || (mods.contains(Mods::CTRL)
                        && (control_byte(c).is_none()
                            || (mods.contains(Mods::SHIFT) && c.is_alphabetic())))
            }
            _ => false,
        };
        let send = match key {
            _ if !level_2 => lost,
            Key::Tab => mods != Mods::SHIFT,
            Key::Backspace => !mods.without(Mods::CTRL).is_empty(),
            _ => true,
        };
        send.then(|| format!("\x1b[27;{};{code}~", u32::from(mods.bits()) + 1).into_bytes())
    }

    /// tmux 3.4's keys, after `input_key` in its `input-keys.c`. Where a key with Ctrl has a
    /// control character, it's that. Ctrl+Tab and the special keys come from its table. Any other
    /// key with Shift or Ctrl sends nothing, or `CSI code ; mods u` once the program has asked for
    /// modifyOtherKeys. tmux has no Super, so a key with Super is the key without it.
    fn tmux_3_4_key(&self, key: Key, mods: Mods) -> Vec<u8> {
        /// The characters tmux keeps Ctrl on, rather than turning into a control character.
        const KEEPS_CTRL: &str = "!#()+,-.0123456789:;<=>'`/";
        let mods = mods.without(Mods::SUPER);
        let extended = self.profile.modify_other_keys && self.modify_other_keys != 0;
        let esc = |bytes: &[u8]| -> Vec<u8> {
            if mods.contains(Mods::ALT) {
                [&[ESC][..], bytes].concat()
            } else {
                bytes.to_vec()
            }
        };
        let (code, mods) = match key {
            Key::Tab if mods.contains(Mods::CTRL) => {
                let shift = mods.contains(Mods::SHIFT);
                let bytes: &[u8] = match (shift, extended && !mods.contains(Mods::ALT)) {
                    (false, false) => b"\t",
                    (false, true) => b"\x1b[9;5u",
                    (true, false) => b"\x1b[Z",
                    (true, true) => b"\x1b[1;5Z",
                };
                return esc(bytes);
            }
            // Shift+Tab is tmux's BTab, in its table.
            Key::Enter | Key::Escape | Key::Backspace | Key::Tab
                if mods.intersects(Mods::SHIFT | Mods::CTRL)
                    && !(key == Key::Tab && mods.without(Mods::ALT) == Mods::SHIFT) =>
            {
                let code = match key {
                    Key::Enter => '\r',
                    Key::Escape => '\x1b',
                    Key::Backspace => '\x7f',
                    _ => '\t',
                };
                (code, mods)
            }
            Key::Char(c) if mods.contains(Mods::CTRL) => {
                let typed = typed_char(c, mods);
                if !KEEPS_CTRL.contains(typed) && typed.is_ascii() {
                    return match typed {
                        'a'..='z' | '@'..='_' | ' ' | '?' => {
                            control_byte(typed).map_or_else(Vec::new, |byte| esc(&[byte]))
                        }
                        // tmux has no name for the others.
                        _ => Vec::new(),
                    };
                }
                // Shift is in a letter, and stays a modifier on the others.
                if c.is_alphabetic() {
                    (typed, mods.without(Mods::SHIFT))
                } else {
                    (typed, mods)
                }
            }
            _ => return self.legacy_key(key, mods),
        };
        if extended {
            return format!("\x1b[{};{}u", u32::from(code), mods.bits() + 1).into_bytes();
        }
        match code {
            _ if mods != Mods::CTRL => Vec::new(),
            '2' => vec![0],
            '6' => vec![0x1e],
            '-' | '/' => vec![0x1f],
            _ => Vec::new(),
        }
    }

    /// The kitty keyboard protocol, by the flags in force.
    fn kitty_key(&self, key: Key, mods: Mods, kind: KeyKind, flags: u8) -> Vec<u8> {
        let report_events = flags & REPORT_EVENTS != 0;
        if kind == KeyKind::Release && !report_events {
            return Vec::new();
        }
        let kind = if report_events { kind } else { KeyKind::Press };
        let all_keys = flags & ALL_KEYS != 0;
        let disambiguate = flags & DISAMBIGUATE != 0;
        let release = kind == KeyKind::Release;
        let mut code = KittyCode {
            key: 0,
            shifted: None,
            mods,
            kind,
            report_events,
            text: None,
            final_byte: b'u',
        };
        match key {
            Key::Char(c) => {
                let typed = typed_char(c, mods);
                let types_text = !mods.intersects(Mods::CTRL | Mods::ALT | Mods::SUPER);
                if !all_keys && !release && (types_text || !disambiguate) {
                    return self.legacy_key(key, mods);
                }
                code.key = u32::from(base_char(c));
                if flags & ALTERNATES != 0 && mods.contains(Mods::SHIFT) && typed != base_char(c) {
                    code.shifted = Some(u32::from(typed));
                }
                if flags & TEXT != 0 && all_keys && types_text && !release {
                    code.text = Some(typed);
                }
            }
            Key::Enter | Key::Tab | Key::Backspace | Key::Escape => {
                code.key = match key {
                    Key::Enter => 13,
                    Key::Tab => 9,
                    Key::Backspace => 127,
                    _ => 27,
                };
                let legacy =
                    !all_keys && !(disambiguate && (key == Key::Escape || !mods.is_empty()));
                // Where they keep their legacy bytes, Enter, Tab and Backspace report no release,
                // and Escape reports its release as other keys do.
                if legacy && !(key == Key::Escape && release) {
                    return if release {
                        Vec::new()
                    } else {
                        self.legacy_key(key, mods)
                    };
                }
            }
            Key::Up | Key::Down | Key::Right | Key::Left | Key::Home | Key::End
                if self.app_cursor && !disambiguate && !report_events && mods.is_empty() =>
            {
                // Only the first two flags turn off application cursor keys.
                return self.legacy_key(key, mods);
            }
            Key::Up | Key::Down | Key::Right | Key::Left | Key::Home | Key::End => {
                code.key = 1;
                code.final_byte = match key {
                    Key::Up => b'A',
                    Key::Down => b'B',
                    Key::Right => b'C',
                    Key::Left => b'D',
                    Key::Home => b'H',
                    _ => b'F',
                };
            }
            Key::Insert | Key::Delete | Key::PageUp | Key::PageDown => {
                code.key = match key {
                    Key::Insert => 2,
                    Key::Delete => 3,
                    Key::PageUp => 5,
                    _ => 6,
                };
                code.final_byte = b'~';
            }
            Key::F(n @ (1 | 2 | 4)) => {
                code.key = 1;
                code.final_byte = b'P' + (n - 1);
            }
            Key::F(3) => {
                code.key = 13;
                code.final_byte = b'~';
            }
            Key::F(n @ 5..=12) => {
                code.key = function_key_number(n);
                code.final_byte = b'~';
            }
            Key::F(n @ 13..=35) => code.key = 57376 + u32::from(n - 13),
            Key::F(_) => return Vec::new(),
        }
        code.serialize()
    }
}

/// A key as the kitty protocol writes it: `CSI key[:shifted] ; mods[:event] ; text final`.
struct KittyCode {
    key: u32,
    shifted: Option<u32>,
    mods: Mods,
    kind: KeyKind,
    report_events: bool,
    text: Option<char>,
    final_byte: u8,
}

impl KittyCode {
    fn serialize(&self) -> Vec<u8> {
        let event = match self.kind {
            KeyKind::Press => 1,
            KeyKind::Repeat => 2,
            KeyKind::Release => 3,
        };
        let add_event = self.report_events && event != 1;
        let second = !self.mods.is_empty() || add_event;
        let mut out = String::from("\x1b[");
        if self.key != 1 || self.shifted.is_some() || second || self.text.is_some() {
            let _ = write!(out, "{}", self.key);
        }
        if let Some(shifted) = self.shifted {
            let _ = write!(out, ":{shifted}");
        }
        if second || self.text.is_some() {
            out.push(';');
            if second {
                let _ = write!(out, "{}", u32::from(self.mods.bits()) + 1);
                if add_event {
                    let _ = write!(out, ":{event}");
                }
            }
        }
        if let Some(text) = self.text {
            let _ = write!(out, ";{}", u32::from(text));
        }
        out.push(char::from(self.final_byte));
        out.into_bytes()
    }
}

/// The number in `CSI n ~` for F5 to F12.
fn function_key_number(n: u8) -> u32 {
    match n {
        5 => 15,
        6 => 17,
        7 => 18,
        8 => 19,
        9 => 20,
        10 => 21,
        11 => 23,
        _ => 24,
    }
}

/// The character a key types with the modifiers held: shift makes a letter uppercase.
fn typed_char(c: char, mods: Mods) -> char {
    if mods.contains(Mods::SHIFT) {
        let mut upper = c.to_uppercase();
        match (upper.next(), upper.next()) {
            (Some(u), None) => u,
            _ => c,
        }
    } else {
        c
    }
}

/// The key's own character, lowercase for a letter.
fn base_char(c: char) -> char {
    let mut lower = c.to_lowercase();
    match (lower.next(), lower.next()) {
        (Some(l), None) => l,
        _ => c,
    }
}

/// The C0 byte that Ctrl with a character sends, as xterm maps them.
fn control_byte(c: char) -> Option<u8> {
    match c {
        'a'..='z' => u8::try_from(u32::from(c) - u32::from('a') + 1).ok(),
        'A'..='Z' => u8::try_from(u32::from(c) - u32::from('A') + 1).ok(),
        '@' | ' ' | '2' => Some(0x00),
        '[' | '3' => Some(0x1b),
        '\\' | '4' => Some(0x1c),
        ']' | '5' => Some(0x1d),
        '^' | '6' | '~' => Some(0x1e),
        '_' | '7' | '/' => Some(0x1f),
        '8' | '?' => Some(0x7f),
        _ => None,
    }
}
