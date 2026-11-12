//! Control sequences.

use super::{CursorShape, Emulator, KNOWN_MODES, MouseTracking, TITLE_STACK_DEPTH};
use crate::event::Event;
use crate::parser::{Csi, Params};
use crate::style::{Color, Style, Underline};

/// A parameter that defaults to 1, as cursor moves and counts do.
fn count(params: &Params, index: usize) -> u16 {
    params.get(index).max(1)
}

impl Emulator {
    /// Handles a control sequence. Returns false when it isn't known, after doing what it could.
    pub(super) fn csi(&mut self, csi: &Csi<'_>) -> bool {
        let params = csi.params;
        if params.has_subparams()
            && !(csi.private.is_none() && csi.intermediates.is_empty() && csi.final_byte == b'm')
        {
            return false;
        }
        match (csi.private, csi.intermediates, csi.final_byte) {
            (None, [], b'm') => self.sgr(params),
            (None, [], final_byte) => self.csi_plain(params, final_byte),
            (Some(b'?'), [], b'h') => self.set_modes(params, true),
            (Some(b'?'), [], b'l') => self.set_modes(params, false),
            (Some(b'?'), [b'$'], b'p') if params.len() == 1 => {
                self.decrqm(params.get(0), true);
                true
            }
            (None, [b'$'], b'p') if params.len() == 1 => {
                self.decrqm(params.get(0), false);
                true
            }
            (None, [b' '], b'q') if params.len() <= 1 => self.cursor_style(params.get(0)),
            (Some(b'?'), [], b'u') if params.is_empty() => {
                if self.profile.kitty_keyboard {
                    let flags = self.keyboard_flags();
                    self.reply(format!("\x1b[?{flags}u").as_bytes());
                }
                true
            }
            (Some(b'>'), [], b'u') if params.len() <= 1 => {
                if self.profile.kitty_keyboard {
                    let flags = u8::try_from(params.get(0)).unwrap_or(u8::MAX);
                    self.screen_mut().kitty.push(flags);
                }
                true
            }
            (Some(b'<'), [], b'u') if params.len() <= 1 => {
                if self.profile.kitty_keyboard {
                    self.screen_mut().kitty.pop(usize::from(count(params, 0)));
                }
                true
            }
            (Some(b'='), [], b'u') if params.len() <= 2 => {
                let how = params.get(1).max(1);
                if how > 3 {
                    return false;
                }
                if self.profile.kitty_keyboard {
                    let flags = u8::try_from(params.get(0)).unwrap_or(u8::MAX);
                    self.screen_mut().kitty.set(flags, how);
                }
                true
            }
            (Some(b'>'), [], b'c') if params.get(0) == 0 && params.len() <= 1 => {
                if let Some(answer) = self.profile.da2.clone() {
                    self.reply(answer.as_bytes());
                }
                true
            }
            (Some(b'>'), [], b'q') if params.get(0) == 0 && params.len() <= 1 => {
                if let Some(version) = self.profile.xtversion.clone() {
                    self.reply(format!("\x1bP>|{version}\x1b\\").as_bytes());
                }
                true
            }
            (Some(b'>'), [], b'm') if params.len() <= 2 && params.get(0) == 4 => {
                let level = params.get(1);
                if level > 2 {
                    return false;
                }
                if self.profile.modify_other_keys {
                    self.modify_other_keys = u8::try_from(level).unwrap_or(0);
                }
                true
            }
            _ => false,
        }
    }

    /// Control sequences without a private marker or intermediates, but SGR.
    fn csi_plain(&mut self, params: &Params, final_byte: u8) -> bool {
        let n = count(params, 0);
        let few = |max: usize| params.len() <= max;
        match final_byte {
            b'A' if few(1) => {
                let stop = if self.y >= self.top { self.top } else { 0 };
                self.y = self.y.saturating_sub(n).max(stop);
                self.pending_wrap = false;
            }
            b'B' if few(1) => self.cursor_down(n),
            b'C' if few(1) => {
                self.x = self.x.saturating_add(n).min(self.cols - 1);
                self.pending_wrap = false;
            }
            b'D' if few(1) => {
                self.x = self.x.saturating_sub(n);
                self.pending_wrap = false;
            }
            b'E' if few(1) => {
                self.cursor_down(n);
                self.x = 0;
            }
            b'F' if few(1) => {
                let stop = if self.y >= self.top { self.top } else { 0 };
                self.y = self.y.saturating_sub(n).max(stop);
                self.x = 0;
                self.pending_wrap = false;
            }
            b'G' | b'`' if few(1) => {
                self.x = (n - 1).min(self.cols - 1);
                self.pending_wrap = false;
            }
            b'H' | b'f' if few(2) => self.goto(count(params, 1) - 1, n - 1),
            b'd' if few(1) => {
                let x = self.x;
                self.goto(x, n - 1);
            }
            b'J' if few(1) => return self.erase_display(params.get(0)),
            b'K' if few(1) => return self.erase_line(params.get(0)),
            b'S' | b'T' if few(1) && self.profile.quirks.no_scroll_up_down => {}
            b'S' if few(1) => self.scroll_up(n),
            b'T' if few(1) => self.scroll_down(n),
            b'L' | b'M' if few(1) && self.profile.quirks.no_insert_delete_lines => {}
            b'L' if few(1) => self.insert_lines(n),
            b'M' if few(1) => self.delete_lines(n),
            b'r' if few(2) => self.set_scroll_region(params),
            b's' if params.is_empty() => self.save_cursor(),
            b'u' if params.is_empty() => self.restore_cursor(),
            b'c' if few(1) && params.get(0) == 0 => {
                if let Some(answer) = self.profile.da1.clone() {
                    self.reply(answer.as_bytes());
                }
            }
            b'n' if few(1) => return self.device_status(params.get(0)),
            b't' if few(3) => return self.window_op(params),
            _ => return false,
        }
        true
    }

    fn cursor_down(&mut self, n: u16) {
        let stop = if self.y <= self.bottom {
            self.bottom
        } else {
            self.rows - 1
        };
        self.y = self.y.saturating_add(n).min(stop);
        self.pending_wrap = false;
    }

    fn erase_display(&mut self, how: u16) -> bool {
        let blank = self.blank();
        let (x, y, rows, cols) = (self.x, self.y, self.rows, self.cols);
        match how {
            0 => {
                let grid = self.grid_mut();
                grid.erase(y, x, cols, &blank);
                grid.erase_rows(y + 1, rows, &blank);
                self.pending_wrap = false;
            }
            1 => {
                let grid = self.grid_mut();
                grid.erase_rows(0, y, &blank);
                grid.erase(y, 0, x + 1, &blank);
            }
            2 => self.grid_mut().erase_rows(0, rows, &blank),
            // The emulator keeps no scrollback, so there is nothing to clear.
            3 => {}
            _ => return false,
        }
        true
    }

    fn erase_line(&mut self, how: u16) -> bool {
        let blank = self.blank();
        let (x, y, cols) = (self.x, self.y, self.cols);
        match how {
            0 => {
                self.grid_mut().erase(y, x, cols, &blank);
                self.pending_wrap = false;
            }
            1 => self.grid_mut().erase(y, 0, x + 1, &blank),
            2 => self.grid_mut().erase(y, 0, cols, &blank),
            _ => return false,
        }
        true
    }

    /// DECSTBM. A region of less than two rows is ignored, as xterm does.
    fn set_scroll_region(&mut self, params: &Params) {
        let top = count(params, 0);
        let bottom = match params.get(1) {
            0 => self.rows,
            b => b.min(self.rows),
        };
        if top < bottom {
            self.top = top - 1;
            self.bottom = bottom - 1;
            self.goto(0, 0);
        }
    }

    fn device_status(&mut self, what: u16) -> bool {
        match what {
            5 => {
                if self.profile.cursor_report {
                    self.reply(b"\x1b[0n");
                }
            }
            6 => {
                if self.profile.cursor_report {
                    let row = if self.origin {
                        self.y.saturating_sub(self.top)
                    } else {
                        self.y
                    };
                    let answer = format!("\x1b[{};{}R", row + 1, self.x + 1);
                    self.reply(answer.as_bytes());
                }
            }
            _ => return false,
        }
        true
    }

    /// XTWINOPS: the size report and the title stack.
    fn window_op(&mut self, params: &Params) -> bool {
        match (params.get(0), params.get(1)) {
            (18, _) if params.len() == 1 => {
                if self.profile.size_report {
                    let answer = format!("\x1b[8;{};{}t", self.rows, self.cols);
                    self.reply(answer.as_bytes());
                }
            }
            // A push saves both titles, and a pop restores the ones its parameter names (0 both, 1
            // the icon title, 2 the window title).
            (22, 0..=2) if params.len() <= 2 => {
                if self.profile.title_stack {
                    if self.title_stack.len() == TITLE_STACK_DEPTH {
                        self.title_stack.remove(0);
                    }
                    self.title_stack
                        .push((self.title.clone(), self.icon.clone()));
                }
            }
            (23, which @ 0..=2) if params.len() <= 2 => {
                if self.profile.title_stack
                    && let Some((title, icon)) = self.title_stack.pop()
                {
                    if which != 1 {
                        self.title.clone_from(&title);
                        self.event(Event::TitleSet(title));
                    }
                    if which != 2 {
                        self.icon = icon;
                    }
                }
            }
            _ => return false,
        }
        true
    }

    /// DECSCUSR.
    fn cursor_style(&mut self, style: u16) -> bool {
        let (shape, blinking) = match style {
            0 => (CursorShape::Default, false),
            1 => (CursorShape::Block, true),
            2 => (CursorShape::Block, false),
            3 => (CursorShape::Underline, true),
            4 => (CursorShape::Underline, false),
            5 => (CursorShape::Bar, true),
            6 => (CursorShape::Bar, false),
            _ => return false,
        };
        if self.profile.cursor_shape {
            self.cursor_shape = shape;
            self.cursor_blink = blinking;
        }
        true
    }

    fn set_modes(&mut self, params: &Params, on: bool) -> bool {
        if params.is_empty() {
            return false;
        }
        let mut known = true;
        for mode in params.values() {
            known &= self.set_mode(mode, on);
        }
        known
    }

    /// Sets or resets one DEC private mode. Returns false for a mode the emulator doesn't know.
    fn set_mode(&mut self, mode: u16, on: bool) -> bool {
        if !KNOWN_MODES.contains(&mode) {
            return false;
        }
        if !self.profile.has_mode(mode) {
            return true;
        }
        match mode {
            1 => self.app_cursor = on,
            6 => {
                self.origin = on;
                self.goto(0, 0);
            }
            7 if !on && self.profile.quirks.ignores_autowrap_off => return true,
            7 => self.autowrap = on,
            9 | 1000 | 1002 | 1003 => {
                self.mouse = match (on, mode) {
                    (false, _) => MouseTracking::Off,
                    (true, 9) => MouseTracking::X10,
                    (true, 1000) => MouseTracking::Normal,
                    (true, 1002) => MouseTracking::Button,
                    (true, _) => MouseTracking::Any,
                };
            }
            12 => self.cursor_blink = on,
            25 => {
                self.cursor_visible = on;
                let at = self.pos();
                self.event(if on {
                    Event::CursorShown { at }
                } else {
                    Event::CursorHidden { at }
                });
                return true;
            }
            47 | 1047 if on => self.enter_alternate(false, false),
            47 => self.leave_alternate(false, false),
            1047 => self.leave_alternate(false, true),
            1048 if on => self.save_cursor(),
            1048 => self.restore_cursor(),
            1049 if on => self.enter_alternate(true, true),
            1049 => self.leave_alternate(true, false),
            1004 => self.focus_reports = on,
            1006 => self.sgr_mouse = on,
            2004 => self.bracketed_paste = on,
            2026 => {
                self.sync = on;
                self.event(if on { Event::SyncBegin } else { Event::SyncEnd });
                return true;
            }
            2027 => self.graphemes = on,
            _ => {}
        }
        self.event(if on {
            Event::ModeSet(mode)
        } else {
            Event::ModeReset(mode)
        });
        true
    }

    /// DECRQM, answered with 0 for a mode the profile lacks, 1 for set and 2 for reset. The
    /// emulator models no ANSI modes, so those are all 0.
    fn decrqm(&mut self, mode: u16, private: bool) {
        if !self.profile.decrqm {
            return;
        }
        let state = if private && KNOWN_MODES.contains(&mode) && self.profile.has_mode(mode) {
            if self.mode(mode) { 1 } else { 2 }
        } else {
            0
        };
        let marker = if private { "?" } else { "" };
        self.reply(format!("\x1b[{marker}{mode};{state}$y").as_bytes());
    }

    /// SGR, in the semicolon and colon forms. Returns false if any parameter was unknown, after
    /// applying the rest.
    fn sgr(&mut self, params: &Params) -> bool {
        if params.is_empty() {
            self.reset_pen();
            return true;
        }
        let groups: Vec<&[u16]> = params.groups().collect();
        let mut known = true;
        let mut i = 0;
        while i < groups.len() {
            let group = groups[i];
            let p = group[0];
            let sub = &group[1..];
            i += 1;
            let pen = &mut self.pen;
            let ok = match p {
                4 if !sub.is_empty() => {
                    pen.underline = match sub[0] {
                        0 => Underline::None,
                        1 => Underline::Single,
                        2 => Underline::Double,
                        3 => Underline::Curly,
                        4 => Underline::Dotted,
                        5 => Underline::Dashed,
                        _ => Underline::None,
                    };
                    sub.len() == 1 && sub[0] <= 5
                }
                38 | 48 | 58 => {
                    let color = if sub.is_empty() {
                        let (color, used) = extended_color_semicolon(&groups[i..]);
                        i += used;
                        color
                    } else {
                        extended_color_colon(sub)
                    };
                    match (color, p) {
                        (Some(color), 38) => pen.fg = color,
                        (Some(color), 48) => pen.bg = color,
                        (Some(color), _) => pen.underline_color = color,
                        (None, _) => {}
                    }
                    color.is_some()
                }
                _ if !sub.is_empty() => false,
                0 => {
                    self.reset_pen();
                    true
                }
                _ if self.profile.ignored_sgr.contains(&p) => true,
                1 | 2 if self.profile.one_intensity => {
                    (pen.bold, pen.dim) = (p == 1, p == 2);
                    true
                }
                _ => sgr_simple(pen, p),
            };
            known &= ok;
        }
        known
    }

    /// SGR 0, which leaves a hyperlink open.
    fn reset_pen(&mut self) {
        let link = self.pen.link.take();
        self.pen = Style {
            link,
            ..Style::default()
        };
    }
}

/// The SGR parameters that take no arguments.
fn sgr_simple(pen: &mut Style, p: u16) -> bool {
    match p {
        1 => pen.bold = true,
        2 => pen.dim = true,
        3 => pen.italic = true,
        4 => pen.underline = Underline::Single,
        5 => pen.blink = true,
        6 => pen.rapid_blink = true,
        7 => pen.reverse = true,
        8 => pen.hidden = true,
        9 => pen.strike = true,
        21 => pen.underline = Underline::Double,
        22 => {
            pen.bold = false;
            pen.dim = false;
        }
        23 => pen.italic = false,
        24 => pen.underline = Underline::None,
        25 => {
            pen.blink = false;
            pen.rapid_blink = false;
        }
        27 => pen.reverse = false,
        28 => pen.hidden = false,
        29 => pen.strike = false,
        30..=37 => pen.fg = named(p - 30),
        39 => pen.fg = Color::Default,
        40..=47 => pen.bg = named(p - 40),
        49 => pen.bg = Color::Default,
        53 => pen.overline = true,
        55 => pen.overline = false,
        59 => pen.underline_color = Color::Default,
        90..=97 => pen.fg = named(p - 90 + 8),
        100..=107 => pen.bg = named(p - 100 + 8),
        _ => return false,
    }
    true
}

fn named(index: u16) -> Color {
    u8::try_from(index)
        .ok()
        .and_then(Color::named)
        .unwrap_or_default()
}

fn byte(value: u16) -> Option<u8> {
    u8::try_from(value).ok()
}

/// `38;5;n` or `38;2;r;g;b`: the colour and how many groups after the 38 it used.
fn extended_color_semicolon(rest: &[&[u16]]) -> (Option<Color>, usize) {
    let value = |i: usize| rest.get(i).map(|group| group[0]);
    match value(0) {
        Some(5) => match value(1).and_then(byte) {
            Some(n) => (Some(Color::Indexed(n)), 2),
            None => (None, rest.len().min(2)),
        },
        Some(2) => match (
            value(1).and_then(byte),
            value(2).and_then(byte),
            value(3).and_then(byte),
        ) {
            (Some(r), Some(g), Some(b)) => (Some(Color::Rgb(r, g, b)), 4),
            _ => (None, rest.len().min(4)),
        },
        _ => (None, rest.len().min(1)),
    }
}

/// `38:5:n`, `38:2:r:g:b` or `38:2:space:r:g:b`, given what follows the 38.
fn extended_color_colon(sub: &[u16]) -> Option<Color> {
    match sub {
        [5, n] => byte(*n).map(Color::Indexed),
        [2, r, g, b] | [2, _, r, g, b] => Some(Color::Rgb(byte(*r)?, byte(*g)?, byte(*b)?)),
        _ => None,
    }
}
