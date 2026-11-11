//! The emulator: the screens, the cursor, the pen and the modes, driven by the parser.

mod csi;
mod keys;
mod readback;
mod strings;

use unicode_segmentation::UnicodeSegmentation;

use crate::event::{ClipboardWrite, Event, FeedError, Pos, UnknownSequence};
use crate::grid::{Cell, Grid};
use crate::parser::{Csi, Dcs, Parser, Perform, StringKind, Terminator};
use crate::profile::Profile;
use crate::style::Style;
use crate::width::WidthRules;

/// The shape of the cursor, from DECSCUSR (`CSI n SP q`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CursorShape {
    /// The terminal's default, `CSI 0 SP q` or never set.
    #[default]
    Default,
    /// A block, `CSI 1 SP q` or `CSI 2 SP q`.
    Block,
    /// An underline, `CSI 3 SP q` or `CSI 4 SP q`.
    Underline,
    /// A bar, `CSI 5 SP q` or `CSI 6 SP q`.
    Bar,
}

/// The cursor as a test sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Cursor {
    /// The column, from 0.
    pub x: u16,
    /// The row, from 0.
    pub y: u16,
    /// Whether it shows (mode 25).
    pub visible: bool,
    /// Its shape.
    pub shape: CursorShape,
    /// Whether it blinks, from DECSCUSR or mode 12.
    pub blinking: bool,
    /// Whether a character was just drawn in the last column, so the next one wraps first if
    /// autowrap is on, and overwrites the last column if it's off.
    pub pending_wrap: bool,
}

/// Which mouse events the program asked for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MouseTracking {
    #[default]
    Off,
    /// Mode 9, presses only.
    X10,
    /// Mode 1000, presses, releases and the wheel.
    Normal,
    /// Mode 1002, with drags.
    Button,
    /// Mode 1003, with every motion.
    Any,
}

impl MouseTracking {
    fn mode(self) -> Option<u16> {
        match self {
            MouseTracking::Off => None,
            MouseTracking::X10 => Some(9),
            MouseTracking::Normal => Some(1000),
            MouseTracking::Button => Some(1002),
            MouseTracking::Any => Some(1003),
        }
    }
}

/// The DEC private modes the emulator knows. Any other is an unknown sequence.
const KNOWN_MODES: [u16; 18] = [
    1, 6, 7, 9, 12, 25, 47, 1000, 1002, 1003, 1004, 1006, 1047, 1048, 1049, 2004, 2026, 2027,
];

/// The kitty keyboard flags of one screen: a stack, and the flags in force when it's empty.
#[derive(Clone, Debug, Default)]
struct KittyFlags {
    stack: Vec<u8>,
    base: u8,
}

/// The kitty protocol leaves the depth to the terminal, and drops the oldest entry past it.
const KITTY_STACK_DEPTH: usize = 16;
/// xterm keeps ten titles.
const TITLE_STACK_DEPTH: usize = 10;

impl KittyFlags {
    fn current(&self) -> u8 {
        self.stack.last().copied().unwrap_or(self.base)
    }

    fn push(&mut self, flags: u8) {
        if self.stack.len() == KITTY_STACK_DEPTH {
            self.stack.remove(0);
        }
        self.stack.push(flags & 0x1f);
    }

    fn pop(&mut self, n: usize) {
        let len = self.stack.len().saturating_sub(n);
        self.stack.truncate(len);
        if self.stack.is_empty() {
            self.base = 0;
        }
    }

    fn set(&mut self, flags: u8, how: u16) {
        let current = match self.stack.last_mut() {
            Some(top) => top,
            None => &mut self.base,
        };
        let flags = flags & 0x1f;
        match how {
            2 => *current |= flags,
            3 => *current &= !flags,
            _ => *current = flags,
        }
    }
}

/// What DECSC saves and DECRC restores.
#[derive(Clone, Debug)]
struct Saved {
    x: u16,
    y: u16,
    pending_wrap: bool,
    pen: Style,
    origin: bool,
    line_drawing: bool,
}

/// One of the two screens, with what xterm keeps for each.
#[derive(Clone, Debug)]
struct Screen {
    grid: Grid,
    saved: Option<Saved>,
    kitty: KittyFlags,
}

impl Screen {
    fn new(cols: u16, rows: u16) -> Screen {
        Screen {
            grid: Grid::new(cols, rows),
            saved: None,
            kitty: KittyFlags::default(),
        }
    }
}

/// A terminal emulator for tests.
///
/// Feed it the bytes a program writes, then read the screen, the cursor, the events and the
/// answers the terminal would send back. It plays the terminal its [`Profile`] describes.
///
/// ```
/// use termelon_vt::Emulator;
///
/// let mut term = Emulator::new(20, 3);
/// term.feed(b"\x1b[2;3H\x1b[1;31mhi\x1b[0m").unwrap();
/// assert_eq!(term.row_text(1).as_deref(), Some("  hi"));
/// assert!(term.cell(2, 1).unwrap().style().bold);
///
/// // Strict mode, the default, refuses what it doesn't know.
/// assert!(term.feed(b"\x1b[?9999h").is_err());
/// ```
#[derive(Debug)]
pub struct Emulator {
    parser: Parser,
    profile: Profile,
    strict: bool,
    record_events: bool,
    cols: u16,
    rows: u16,
    main: Screen,
    alt: Screen,
    alt_active: bool,
    x: u16,
    y: u16,
    pending_wrap: bool,
    cursor_visible: bool,
    cursor_shape: CursorShape,
    cursor_blink: bool,
    pen: Style,
    line_drawing: bool,
    origin: bool,
    autowrap: bool,
    app_cursor: bool,
    app_keypad: bool,
    top: u16,
    bottom: u16,
    mouse: MouseTracking,
    sgr_mouse: bool,
    focus_reports: bool,
    bracketed_paste: bool,
    sync: bool,
    graphemes: bool,
    modify_other_keys: u8,
    title: String,
    icon: String,
    title_stack: Vec<(String, String)>,
    clipboard: Vec<ClipboardWrite>,
    events: Vec<Event>,
    output: Vec<u8>,
    unknown: Vec<UnknownSequence>,
    /// The cell of the cluster drawn last, while nothing else has happened since, so the next
    /// character may join it.
    last_print: Option<Pos>,
}

impl Emulator {
    /// An emulator of `cols` by `rows` cells playing xterm, in strict mode. A size of 0 counts as 1.
    pub fn new(cols: u16, rows: u16) -> Emulator {
        Emulator::with_profile(Profile::xterm(), cols, rows)
    }

    /// An emulator playing the terminal a profile describes, in strict mode.
    pub fn with_profile(profile: Profile, cols: u16, rows: u16) -> Emulator {
        let (cols, rows) = (cols.max(1), rows.max(1));
        Emulator {
            parser: Parser::new(profile.linux_console),
            profile,
            strict: true,
            record_events: true,
            cols,
            rows,
            main: Screen::new(cols, rows),
            alt: Screen::new(cols, rows),
            alt_active: false,
            x: 0,
            y: 0,
            pending_wrap: false,
            cursor_visible: true,
            cursor_shape: CursorShape::Default,
            cursor_blink: false,
            pen: Style::default(),
            line_drawing: false,
            origin: false,
            autowrap: true,
            app_cursor: false,
            app_keypad: false,
            top: 0,
            bottom: rows - 1,
            mouse: MouseTracking::Off,
            sgr_mouse: false,
            focus_reports: false,
            bracketed_paste: false,
            sync: false,
            graphemes: false,
            modify_other_keys: 0,
            title: String::new(),
            icon: String::new(),
            title_stack: Vec::new(),
            clipboard: Vec::new(),
            events: Vec::new(),
            output: Vec::new(),
            unknown: Vec::new(),
            last_print: None,
        }
    }

    /// The profile it plays.
    pub fn profile(&self) -> &Profile {
        &self.profile
    }

    /// Turns strict mode on or off. In strict mode, [`feed`](Emulator::feed) fails on any sequence
    /// the emulator doesn't know. Either way the sequence is skipped and recorded.
    pub fn set_strict(&mut self, strict: bool) {
        self.strict = strict;
    }

    /// Turns the recording of events on or off. It's on from the start.
    pub fn set_record_events(&mut self, record: bool) {
        self.record_events = record;
    }

    /// Processes bytes a program wrote. A sequence may be split across calls.
    ///
    /// # Errors
    ///
    /// In strict mode, the unknown sequences these bytes held, after processing all of them.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), FeedError> {
        let start = self.unknown.len();
        let mut parser = std::mem::replace(&mut self.parser, Parser::new(false));
        parser.advance(self, bytes);
        self.parser = parser;
        if self.strict && self.unknown.len() > start {
            return Err(FeedError {
                sequences: self.unknown[start..].to_vec(),
            });
        }
        Ok(())
    }

    /// Whether the bytes so far stop in the middle of a sequence or a UTF-8 character.
    pub fn in_sequence(&self) -> bool {
        self.parser.is_pending()
    }

    /// Every unknown sequence met so far, in strict mode or not.
    pub fn unknown_sequences(&self) -> &[UnknownSequence] {
        &self.unknown
    }

    /// The events recorded since the last [`take_events`](Emulator::take_events).
    pub fn events(&self) -> &[Event] {
        &self.events
    }

    /// Takes the events recorded so far, clearing them.
    pub fn take_events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    /// The bytes the terminal sent back so far: its answers to queries.
    pub fn output(&self) -> &[u8] {
        &self.output
    }

    /// Takes the bytes the terminal sent back, clearing them.
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.output)
    }

    /// The size, columns then rows.
    pub fn size(&self) -> (u16, u16) {
        (self.cols, self.rows)
    }

    /// Changes the size, as a terminal window does. Both screens keep their top-left content, the
    /// scroll region becomes the whole screen, and the cursor stays inside. A size of 0 counts as 1.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let (cols, rows) = (cols.max(1), rows.max(1));
        self.main.grid.resize(cols, rows);
        self.alt.grid.resize(cols, rows);
        self.cols = cols;
        self.rows = rows;
        self.top = 0;
        self.bottom = rows - 1;
        self.x = self.x.min(cols - 1);
        self.y = self.y.min(rows - 1);
        self.pending_wrap = false;
        for screen in [&mut self.main, &mut self.alt] {
            if let Some(saved) = &mut screen.saved {
                saved.x = saved.x.min(cols - 1);
                saved.y = saved.y.min(rows - 1);
            }
        }
        self.last_print = None;
    }

    /// The cursor.
    pub fn cursor(&self) -> Cursor {
        Cursor {
            x: self.x,
            y: self.y,
            visible: self.cursor_visible,
            shape: self.cursor_shape,
            blinking: self.cursor_blink,
            pending_wrap: self.pending_wrap,
        }
    }

    /// The pen, the style the next character is drawn with.
    pub fn pen(&self) -> &Style {
        &self.pen
    }

    /// A cell of the screen in use, or `None` outside it.
    pub fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        self.grid().cell(x, y)
    }

    /// Whether the alternate screen is in use.
    pub fn alternate_screen(&self) -> bool {
        self.alt_active
    }

    /// Whether a DEC private mode is set. Modes the emulator doesn't know read as reset.
    pub fn mode(&self, mode: u16) -> bool {
        match mode {
            1 => self.app_cursor,
            6 => self.origin,
            7 => self.autowrap,
            9 | 1000 | 1002 | 1003 => self.mouse.mode() == Some(mode),
            12 => self.cursor_blink,
            25 => self.cursor_visible,
            47 | 1047 | 1049 => self.alt_active,
            1004 => self.focus_reports,
            1006 => self.sgr_mouse,
            2004 => self.bracketed_paste,
            2026 => self.sync,
            2027 => self.graphemes,
            _ => false,
        }
    }

    /// Whether the keypad is in application mode (`ESC =`).
    pub fn application_keypad(&self) -> bool {
        self.app_keypad
    }

    /// The kitty keyboard flags in force on the screen in use.
    pub fn keyboard_flags(&self) -> u8 {
        self.screen().kitty.current()
    }

    /// The modifyOtherKeys level the program set (`CSI > 4 ; n m`), 0 when off.
    pub fn modify_other_keys(&self) -> u8 {
        self.modify_other_keys
    }

    /// The scroll region, its top and bottom rows (inclusive).
    pub fn scroll_region(&self) -> (u16, u16) {
        (self.top, self.bottom)
    }

    /// The window title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The icon title, from OSC 0 or OSC 1.
    pub fn icon_title(&self) -> &str {
        &self.icon
    }

    /// The title stack, the most recent push last, each a window title and an icon title.
    pub fn title_stack(&self) -> &[(String, String)] {
        &self.title_stack
    }

    /// The OSC 52 clipboard writes, in order.
    pub fn clipboard(&self) -> &[ClipboardWrite] {
        &self.clipboard
    }

    /// The width rules in force: the profile's, joining clusters while mode 2027 is set.
    pub fn width_rules(&self) -> WidthRules {
        let mut rules = self.profile.width;
        if self.graphemes {
            rules.joins_emoji = true;
        }
        rules
    }

    fn screen(&self) -> &Screen {
        if self.alt_active {
            &self.alt
        } else {
            &self.main
        }
    }

    fn screen_mut(&mut self) -> &mut Screen {
        if self.alt_active {
            &mut self.alt
        } else {
            &mut self.main
        }
    }

    fn grid(&self) -> &Grid {
        &self.screen().grid
    }

    fn grid_mut(&mut self) -> &mut Grid {
        &mut self.screen_mut().grid
    }

    fn pos(&self) -> Pos {
        Pos::new(self.x, self.y)
    }

    fn event(&mut self, event: Event) {
        if self.record_events {
            self.events.push(event);
        }
    }

    fn unknown(&mut self, raw: &[u8]) {
        self.unknown.push(UnknownSequence {
            bytes: raw.to_vec(),
        });
    }

    fn reply(&mut self, bytes: &[u8]) {
        self.output.extend_from_slice(bytes);
    }

    /// A blank cell as erasing and scrolling leave it, with the pen's background under BCE.
    fn blank(&self) -> Cell {
        if self.profile.bce {
            Cell::blank(Style {
                bg: self.pen.bg,
                ..Style::default()
            })
        } else {
            Cell::blank(Style::default())
        }
    }

    /// Records a cursor move made by a control or sequence.
    fn moved_from(&mut self, from: Pos) {
        let to = self.pos();
        if from != to {
            self.event(Event::CursorMoved { from, to });
        }
    }

    // Drawing.

    fn draw_char(&mut self, c: char) {
        let c = if self.line_drawing {
            line_drawing(c)
        } else {
            c
        };
        if self.join(c) {
            return;
        }
        self.last_print = None;
        let mut buf = [0; 4];
        let text = c.encode_utf8(&mut buf);
        let width = self.width_rules().width(text);
        if width == 0 {
            self.attach_zero_width(c);
        } else {
            self.draw_cluster(text, width);
        }
    }

    /// Joins a character to the cluster drawn just before, when the two make one grapheme
    /// cluster. The cluster is measured again and redrawn in place, as if it had come whole.
    fn join(&mut self, c: char) -> bool {
        let Some(at) = self.last_print else {
            return false;
        };
        let Some(cell) = self.grid().cell(at.x, at.y) else {
            return false;
        };
        if c.is_ascii() && cell.text().is_ascii() {
            return false;
        }
        let mut text = String::with_capacity(cell.text().len() + 4);
        text.push_str(cell.text());
        text.push(c);
        if text.graphemes(true).nth(1).is_some() {
            return false;
        }
        let old_width = cell.width();
        let width = self.width_rules().width(&text).max(1);
        let cursor = self.pos();
        let cursor_visible = self.cursor_visible;
        if width == old_width {
            self.grid_mut().set_text(at.x, at.y, &text);
            self.event(Event::Drawn {
                at,
                width,
                cursor,
                cursor_visible,
            });
            return true;
        }
        if at.x + u16::from(width) <= self.cols {
            let pen = self.pen.clone();
            self.grid_mut().put(at.x, at.y, &text, width, &pen, false);
            self.event(Event::Drawn {
                at,
                width,
                cursor,
                cursor_visible,
            });
            self.advance_past(at.x, width);
            return true;
        }
        // It no longer fits on its row. It's drawn again from where it started.
        let blank = Cell::blank(self.pen.clone());
        self.grid_mut()
            .erase(at.y, at.x, at.x + u16::from(old_width), &blank);
        self.x = at.x;
        self.y = at.y;
        self.pending_wrap = false;
        self.draw_cluster(&text, width);
        true
    }

    /// A zero-width character that starts no cluster joins the cell before the cursor, as xterm
    /// and Alacritty do. At the left edge it's dropped.
    fn attach_zero_width(&mut self, c: char) {
        let x = if self.pending_wrap {
            self.x
        } else if self.x > 0 {
            self.x - 1
        } else {
            return;
        };
        let (x, y) = (self.grid().leader(x, self.y), self.y);
        let Some(cell) = self.grid().cell(x, y) else {
            return;
        };
        let mut text = cell.text().to_owned();
        let width = cell.width();
        text.push(c);
        self.grid_mut().set_text(x, y, &text);
        let (cursor, cursor_visible) = (self.pos(), self.cursor_visible);
        self.event(Event::Drawn {
            at: Pos::new(x, y),
            width,
            cursor,
            cursor_visible,
        });
    }

    fn draw_cluster(&mut self, text: &str, width: u8) {
        let cursor = self.pos();
        let cursor_visible = self.cursor_visible;
        if self.pending_wrap && self.autowrap {
            self.wrap();
        }
        self.pending_wrap = false;
        let width16 = u16::from(width);
        if width16 > self.cols {
            return;
        }
        if self.x + width16 > self.cols {
            if self.autowrap {
                // The cells left on the row become blanks in the pen's style, as Alacritty
                // leaves them.
                let (x, y, cols) = (self.x, self.y, self.cols);
                let blank = Cell::blank(self.pen.clone());
                self.grid_mut().erase(y, x, cols, &blank);
                self.wrap();
            } else {
                self.x = self.cols - width16;
            }
        }
        let (x, y) = (self.x, self.y);
        let (pen, keep_halves) = (self.pen.clone(), self.profile.quirks.keeps_wide_halves);
        self.grid_mut().put(x, y, text, width, &pen, keep_halves);
        self.event(Event::Drawn {
            at: Pos::new(x, y),
            width,
            cursor,
            cursor_visible,
        });
        self.last_print = Some(Pos::new(x, y));
        self.advance_past(x, width);
    }

    /// Puts the cursor after a cluster drawn at `x`, or in the last column with a wrap pending,
    /// whether autowrap is on or not, as xterm and Alacritty keep it.
    fn advance_past(&mut self, x: u16, width: u8) {
        let next = x + u16::from(width);
        if next >= self.cols {
            self.x = self.cols - 1;
            self.pending_wrap = true;
        } else {
            self.x = next;
            self.pending_wrap = false;
        }
    }

    /// Moves to the start of the next row, scrolling at the bottom of the region.
    fn wrap(&mut self) {
        self.x = 0;
        self.index();
    }

    // Cursor movement and scrolling.

    fn index(&mut self) {
        if self.y == self.bottom {
            self.scroll_up(1);
        } else if self.y + 1 < self.rows {
            self.y += 1;
        }
        self.pending_wrap = false;
    }

    fn reverse_index(&mut self) {
        if self.y == self.top {
            self.scroll_down(1);
        } else if self.y > 0 {
            self.y -= 1;
        }
        self.pending_wrap = false;
    }

    fn scroll_up(&mut self, n: u16) {
        let (top, bottom, blank) = (self.top, self.bottom, self.blank());
        self.grid_mut().scroll_up(top, bottom, n, &blank);
    }

    fn scroll_down(&mut self, n: u16) {
        let (top, bottom, blank) = (self.top, self.bottom, self.blank());
        self.grid_mut().scroll_down(top, bottom, n, &blank);
    }

    /// IL, inside the scroll region only. The rows from the cursor's down move down by `n`, and
    /// the cursor goes to the first column, as DEC terminals do.
    fn insert_lines(&mut self, n: u16) {
        if self.y < self.top || self.y > self.bottom {
            return;
        }
        let (y, bottom, blank) = (self.y, self.bottom, self.blank());
        self.grid_mut().scroll_down(y, bottom, n, &blank);
        self.x = 0;
        self.pending_wrap = false;
    }

    /// DL, inside the scroll region only. The rows below the cursor's move up by `n`, and the
    /// cursor goes to the first column.
    fn delete_lines(&mut self, n: u16) {
        if self.y < self.top || self.y > self.bottom {
            return;
        }
        let (y, bottom, blank) = (self.y, self.bottom, self.blank());
        self.grid_mut().scroll_up(y, bottom, n, &blank);
        self.x = 0;
        self.pending_wrap = false;
    }

    fn tab(&mut self) {
        let next = (self.x / 8 + 1) * 8;
        self.x = next.min(self.cols - 1);
        self.pending_wrap = false;
    }

    fn backspace(&mut self) {
        self.x = self.x.saturating_sub(1);
        self.pending_wrap = false;
    }

    /// Moves to a row and column, from 0, inside the scroll region in origin mode.
    fn goto(&mut self, x: u16, y: u16) {
        let (top, bottom) = if self.origin {
            (self.top, self.bottom)
        } else {
            (0, self.rows - 1)
        };
        self.x = x.min(self.cols - 1);
        self.y = top.saturating_add(y).min(bottom);
        self.pending_wrap = false;
    }

    fn save_cursor(&mut self) {
        let saved = Saved {
            x: self.x,
            y: self.y,
            pending_wrap: self.pending_wrap,
            pen: self.pen.clone(),
            origin: self.origin,
            line_drawing: self.line_drawing,
        };
        self.screen_mut().saved = Some(saved);
    }

    /// Restores the saved cursor, or with none saved, goes home with the default pen.
    fn restore_cursor(&mut self) {
        let saved = self.screen().saved.clone().unwrap_or(Saved {
            x: 0,
            y: 0,
            pending_wrap: false,
            pen: Style::default(),
            origin: false,
            line_drawing: false,
        });
        self.x = saved.x.min(self.cols - 1);
        self.y = saved.y.min(self.rows - 1);
        self.pending_wrap = saved.pending_wrap;
        self.pen = saved.pen;
        self.origin = saved.origin;
        self.line_drawing = saved.line_drawing;
    }

    fn enter_alternate(&mut self, save: bool, clear: bool) {
        if self.alt_active {
            return;
        }
        if save {
            self.save_cursor();
        }
        self.alt_active = true;
        if clear {
            let (rows, blank) = (self.rows, self.blank());
            self.alt.grid.erase_rows(0, rows, &blank);
        }
    }

    fn leave_alternate(&mut self, restore: bool, clear: bool) {
        if !self.alt_active {
            return;
        }
        if clear {
            let (rows, blank) = (self.rows, self.blank());
            self.alt.grid.erase_rows(0, rows, &blank);
        }
        self.alt_active = false;
        if restore {
            self.restore_cursor();
        }
    }

    /// RIS, a full reset. The title and what's been recorded stay.
    fn full_reset(&mut self) {
        let fresh = Emulator::with_profile(self.profile.clone(), self.cols, self.rows);
        let keep = std::mem::replace(self, fresh);
        self.strict = keep.strict;
        self.record_events = keep.record_events;
        self.title = keep.title;
        self.icon = keep.icon;
        self.clipboard = keep.clipboard;
        self.events = keep.events;
        self.output = keep.output;
        self.unknown = keep.unknown;
    }

    fn esc(&mut self, intermediates: &[u8], final_byte: u8) -> bool {
        match (intermediates, final_byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => self.index(),
            ([], b'E') => {
                self.x = 0;
                self.index();
            }
            ([], b'M') => self.reverse_index(),
            ([], b'c') => self.full_reset(),
            ([], b'=') => self.app_keypad = true,
            ([], b'>') => self.app_keypad = false,
            // A lone string terminator does nothing.
            ([], b'\\') => {}
            ([b'('], b'B') => self.line_drawing = false,
            ([b'('], b'0') => self.line_drawing = true,
            _ => return false,
        }
        true
    }
}

impl Perform for Emulator {
    fn print(&mut self, c: char) {
        if c.is_control() {
            // A C1 control written as UTF-8, which no terminal acts on in UTF-8 mode.
            self.last_print = None;
            let mut buf = [0; 4];
            let raw = c.encode_utf8(&mut buf).as_bytes().to_vec();
            self.unknown(&raw);
            return;
        }
        self.draw_char(c);
    }

    fn execute(&mut self, byte: u8) {
        self.last_print = None;
        let from = self.pos();
        match byte {
            // NUL is padding, and CAN and SUB end a sequence, which in ground does nothing.
            0x00 | 0x18 | 0x1a => {}
            0x07 => self.event(Event::Bell),
            0x08 => self.backspace(),
            0x09 => self.tab(),
            0x0a..=0x0c => self.index(),
            0x0d => {
                self.x = 0;
                self.pending_wrap = false;
            }
            _ => self.unknown(&[byte]),
        }
        self.moved_from(from);
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], final_byte: u8, raw: &[u8]) {
        self.last_print = None;
        let from = self.pos();
        if !self.esc(intermediates, final_byte) {
            self.unknown(raw);
        }
        self.moved_from(from);
    }

    fn csi_dispatch(&mut self, csi: &Csi<'_>, raw: &[u8]) {
        self.last_print = None;
        let from = self.pos();
        if !self.csi(csi) {
            self.unknown(raw);
        }
        self.moved_from(from);
    }

    fn osc_dispatch(&mut self, data: &[u8], terminator: Terminator, raw: &[u8]) {
        self.last_print = None;
        if !self.osc(data, terminator) {
            self.unknown(raw);
        }
    }

    fn dcs_dispatch(&mut self, dcs: &Dcs<'_>, raw: &[u8]) {
        self.last_print = None;
        if !self.dcs(dcs) {
            self.unknown(raw);
        }
    }

    fn string_dispatch(&mut self, _kind: StringKind, _data: &[u8], raw: &[u8]) {
        self.last_print = None;
        self.unknown(raw);
    }

    fn ss3_dispatch(&mut self, _byte: u8, raw: &[u8]) {
        self.last_print = None;
        self.unknown(raw);
    }

    fn palette_dispatch(&mut self, _digits: Option<&[u8]>, _raw: &[u8]) {
        // The Linux console's palette changes colours the emulator doesn't model.
        self.last_print = None;
    }

    fn invalid(&mut self, raw: &[u8]) {
        self.last_print = None;
        self.unknown(raw);
    }
}

/// The DEC special graphics set that `ESC ( 0` selects, which maps lowercase letters to line
/// drawing.
fn line_drawing(c: char) -> char {
    match c {
        '_' => ' ',
        '`' => '◆',
        'a' => '▒',
        'b' => '␉',
        'c' => '␌',
        'd' => '␍',
        'e' => '␊',
        'f' => '°',
        'g' => '±',
        'h' => '␤',
        'i' => '␋',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        c => c,
    }
}
