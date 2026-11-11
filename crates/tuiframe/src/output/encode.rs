//! The encoder, which turns the changes of a frame into one packet of bytes.
//!
//! The packet of a frame that changed cells begins a synchronized update, hides the cursor, writes
//! the runs, resets the pen, places and shows the cursor, and ends the update. A frame that changed
//! no cell writes only what the cursor needs, and nothing when it stays as it was.
//!
//! Moves take the shortest form from where the terminal's cursor is: none, `CR`, `CSI n C`,
//! `CSI n G` or `CSI row ; col H`. Styles change by the fewest SGR parameters from the pen the
//! terminal has, or by SGR 0 and the whole style when that's shorter, and no SGR is ever empty. A
//! run whose row ends in cells that look blank erases them with `CSI K`. After a cell whose width
//! terminals disagree on, the next cell is placed absolutely, so a terminal that measures it
//! differently misdraws that cell alone.

use unicode_segmentation::UnicodeSegmentation;

use super::color::{Lowering, ansi_index};
use super::diff::{self, Before, Run};
use super::{Cursor, CursorShape, FrameStats};
use crate::buffer::{Buffer, Cell};
use crate::caps::{Capabilities, ColorDepth, Support};
use crate::layout::{Position, Rect};
use crate::style::{Color, Modifier, UnderlineStyle};

const SYNC_BEGIN: &[u8] = b"\x1b[?2026h";
const SYNC_END: &[u8] = b"\x1b[?2026l";
const HIDE: &[u8] = b"\x1b[?25l";
const SHOW: &[u8] = b"\x1b[?25h";
const AUTOWRAP_OFF: &[u8] = b"\x1b[?7l";
const SGR_RESET: &[u8] = b"\x1b[0m";
const CLEAR: &[u8] = b"\x1b[2J";
const ERASE_LINE: &[u8] = b"\x1b[K";
const LINK_CLOSE: &[u8] = b"\x1b]8;;\x1b\\";

/// What the terminal's pen draws with, as the encoder last set it, with colours already lowered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Pen {
    fg: Color,
    bg: Color,
    underline_color: Color,
    /// The modifiers other than [`Modifier::UNDERLINED`], which `underline` holds.
    modifier: Modifier,
    underline: Option<UnderlineStyle>,
}

impl Pen {
    const DEFAULT: Self = Self {
        fg: Color::Reset,
        bg: Color::Reset,
        underline_color: Color::Reset,
        modifier: Modifier::empty(),
        underline: None,
    };
}

/// What the capabilities allow, worked out once a frame.
#[derive(Debug, Clone, Copy)]
struct Allowed {
    depth: ColorDepth,
    underline_styles: bool,
    underline_color: bool,
    hyperlinks: bool,
    sync: bool,
    cursor_shape: bool,
}

impl Allowed {
    fn new(caps: &Capabilities) -> Self {
        Self {
            depth: caps.colors,
            underline_styles: caps.underline_styles == Support::Yes,
            underline_color: caps.underline_color == Support::Yes
                && caps.colors != ColorDepth::None,
            hyperlinks: caps.hyperlinks == Support::Yes,
            sync: caps.synchronized_output != Support::No,
            cursor_shape: caps.cursor_shape != Support::No,
        }
    }
}

/// Turns frames into packets. It keeps what the terminal shows besides the cells, which is where
/// its cursor is, its shape and visibility, and the pen.
#[derive(Debug)]
pub(crate) struct Encoder {
    /// The row of the terminal's cursor, `None` when unknown.
    row: Option<u16>,
    /// Its column, `None` when unknown, as after a cell whose width terminals disagree on.
    col: Option<u16>,
    visible: Option<bool>,
    shape: Option<(CursorShape, bool)>,
    /// `None` when unknown, so the next style starts from SGR 0.
    pen: Option<Pen>,
    redraw: bool,
    runs: Vec<Run>,
    columns: Vec<(usize, usize)>,
    params: Vec<u8>,
    reset_params: Vec<u8>,
    joined: String,
    lowering: Lowering,
}

impl Encoder {
    /// An encoder for a terminal whose screen is blank and whose cursor shape is its default.
    pub(crate) fn new() -> Self {
        Self {
            row: None,
            col: None,
            visible: None,
            shape: Some((CursorShape::Default, false)),
            pen: None,
            redraw: false,
            runs: Vec::new(),
            columns: Vec::new(),
            params: Vec::new(),
            reset_params: Vec::new(),
            joined: String::new(),
            lowering: Lowering::new(),
        }
    }

    /// Makes the next frame a full redraw, which clears the screen, writes every cell that isn't
    /// blank and sends the cursor's state and the pen again, since a screen that needs redrawing
    /// may have lost them.
    pub(crate) fn redraw(&mut self) {
        self.redraw = true;
        self.row = None;
        self.col = None;
        self.visible = None;
        self.shape = None;
        self.pen = None;
    }

    /// Appends to `out` the packet that takes the terminal from `prev` to `next` and puts the
    /// cursor where `cursor` says, or hides it.
    pub(crate) fn encode(
        &mut self,
        prev: &Buffer,
        next: &Buffer,
        cursor: Option<&Cursor>,
        caps: &Capabilities,
        out: &mut Vec<u8>,
    ) -> FrameStats {
        self.frame(prev, next, None, cursor, caps, out)
    }

    /// The partial frame. It compares `before` and `after` inside `regions` only, and `before`
    /// may hold anything outside them.
    pub(crate) fn encode_regions(
        &mut self,
        before: &Buffer,
        after: &Buffer,
        regions: &[Rect],
        cursor: Option<&Cursor>,
        caps: &Capabilities,
        out: &mut Vec<u8>,
    ) -> FrameStats {
        self.frame(before, after, Some(regions), cursor, caps, out)
    }

    fn frame(
        &mut self,
        prev: &Buffer,
        next: &Buffer,
        regions: Option<&[Rect]>,
        cursor: Option<&Cursor>,
        caps: &Capabilities,
        out: &mut Vec<u8>,
    ) -> FrameStats {
        let start = out.len();
        let allowed = Allowed::new(caps);
        let area = next.area();
        let full = self.redraw || prev.area() != area;
        let width = usize::from(area.width);
        let height = usize::from(area.height);
        let mut runs = core::mem::take(&mut self.runs);
        runs.clear();
        if full {
            for y in 0..height {
                diff::diff_row(Before::Blank, next, y, &[(0, width)], &mut runs);
            }
        } else if let Some(regions) = regions {
            for y in 0..height {
                self.region_columns(area, regions, y);
                if !self.columns.is_empty() {
                    diff::diff_row(Before::Row(prev, y), next, y, &self.columns, &mut runs);
                }
            }
        } else {
            for y in 0..height {
                diff::diff_row(Before::Row(prev, y), next, y, &[(0, width)], &mut runs);
            }
        }
        let mut cells = 0;
        if full || !runs.is_empty() {
            if allowed.sync {
                out.extend_from_slice(SYNC_BEGIN);
            }
            if self.visible != Some(false) {
                out.extend_from_slice(HIDE);
                self.visible = Some(false);
            }
            if full {
                self.set_pen(Pen::DEFAULT, allowed, out);
                out.extend_from_slice(AUTOWRAP_OFF);
                out.extend_from_slice(CLEAR);
            }
            let mut erased = None;
            let mut blank_from = (usize::MAX, 0);
            for &run in &runs {
                if erased == Some(run.row) {
                    continue;
                }
                if blank_from.0 != run.row {
                    blank_from = (run.row, blank_tail(next, next.row(run.row)));
                }
                let (written, erased_row) = self.write_run(next, run, blank_from.1, allowed, out);
                cells += written;
                if erased_row {
                    erased = Some(run.row);
                }
            }
            if self.pen != Some(Pen::DEFAULT) {
                out.extend_from_slice(SGR_RESET);
                self.pen = Some(Pen::DEFAULT);
            }
            self.place_cursor(cursor, area, allowed, out);
            if allowed.sync {
                out.extend_from_slice(SYNC_END);
            }
            if full {
                self.redraw = false;
            }
        } else {
            self.place_cursor(cursor, area, allowed, out);
        }
        self.runs = runs;
        FrameStats {
            cells,
            bytes: out.len() - start,
        }
    }

    /// The columns of row `y` that `regions` cover, sorted and joined, into `self.columns`.
    fn region_columns(&mut self, area: Rect, regions: &[Rect], y: usize) {
        self.columns.clear();
        let Some(abs_y) = u16::try_from(y).ok().map(|y| area.y.saturating_add(y)) else {
            return;
        };
        for region in regions {
            let r = region.intersection(area);
            if r.is_empty() || abs_y < r.y || abs_y >= r.bottom() {
                continue;
            }
            self.columns
                .push((usize::from(r.x - area.x), usize::from(r.right() - area.x)));
        }
        self.columns.sort_unstable();
        let mut kept = 0;
        for i in 0..self.columns.len() {
            let (from, to) = self.columns[i];
            if i > 0 && from <= self.columns[kept].1 {
                self.columns[kept].1 = self.columns[kept].1.max(to);
            } else {
                if i > 0 {
                    kept += 1;
                }
                self.columns[kept] = (from, to);
            }
        }
        self.columns
            .truncate(kept + usize::from(!self.columns.is_empty()));
    }

    /// Writes one run, or erases its blank tail with `CSI K` from `blank_from`, the column from
    /// which the row looks blank to its end. Returns the cells written and whether it erased the
    /// rest of the row.
    fn write_run(
        &mut self,
        next: &Buffer,
        run: Run,
        blank_from: usize,
        allowed: Allowed,
        out: &mut Vec<u8>,
    ) -> (usize, bool) {
        let area = next.area();
        let row = next.row(run.row);
        let Some(y) = u16::try_from(run.row)
            .ok()
            .map(|r| area.y.saturating_add(r))
        else {
            return (0, false);
        };
        let mut cells = 0;
        let mut link = 0;
        let mut last: Option<usize> = None;
        let mut x = run.start;
        let mut erased = false;
        while x < run.end {
            let Some(cell) = row.get(x) else {
                break;
            };
            let Some(abs_x) = u16::try_from(x).ok().map(|x| area.x.saturating_add(x)) else {
                break;
            };
            if x >= blank_from && self.erase_is_shorter(run.end - x) {
                if link != 0 {
                    out.extend_from_slice(LINK_CLOSE);
                    link = 0;
                }
                self.move_to(abs_x, y, false, out);
                let neutral = self.pen.map_or(Pen::DEFAULT, |p| Pen {
                    bg: Color::Reset,
                    modifier: p.modifier - Modifier::REVERSED,
                    ..p
                });
                self.set_pen(neutral, allowed, out);
                out.extend_from_slice(ERASE_LINE);
                cells += run.end - x;
                erased = true;
                break;
            }
            let width = diff::unit_width(row, x);
            let breaks_join = last
                .and_then(|l| row.get(l))
                .is_some_and(|before| self.joins(next.cell_symbol(before), next.cell_symbol(cell)));
            self.move_to(abs_x, y, breaks_join, out);
            let wanted = if allowed.hyperlinks {
                next.link_index(cell)
            } else {
                0
            };
            if wanted != link {
                if link != 0 {
                    out.extend_from_slice(LINK_CLOSE);
                }
                let url = next.cell_link(cell).filter(|_| wanted != 0);
                link = if open_link(url, out) { wanted } else { 0 };
            }
            let pen = pen_of(next, cell, allowed, &mut self.lowering);
            self.set_pen(pen, allowed, out);
            match next.cell_symbol(cell) {
                "" => out.push(b' '),
                symbol => out.extend_from_slice(symbol.as_bytes()),
            }
            let end = usize::from(abs_x) + width;
            self.col = if cell.uncertain_width() || end >= usize::from(area.right()) {
                None
            } else {
                u16::try_from(end).ok()
            };
            last = Some(x);
            cells += width;
            x += width;
        }
        if link != 0 {
            out.extend_from_slice(LINK_CLOSE);
        }
        (cells, erased)
    }

    /// Whether erasing `cells` blank cells with `CSI K` is shorter than writing them as spaces,
    /// counting the SGR that makes the pen's background the default.
    fn erase_is_shorter(&self, cells: usize) -> bool {
        let neutral = self
            .pen
            .is_some_and(|p| p.bg == Color::Reset && !p.modifier.contains(Modifier::REVERSED));
        let cost = ERASE_LINE.len() + if neutral { 0 } else { 6 };
        cost < cells
    }

    /// Whether writing cluster `b` right after cluster `a` would make the terminal join them into
    /// one cluster, as two regional indicators or an emoji and a skin tone join.
    fn joins(&mut self, a: &str, b: &str) -> bool {
        let (Some(last), Some(first)) = (a.chars().next_back(), b.chars().next()) else {
            return false;
        };
        if starts_no_cluster(last) && starts_no_cluster(first) {
            return false;
        }
        self.joined.clear();
        self.joined.push_str(a);
        self.joined.push_str(b);
        !self
            .joined
            .grapheme_indices(true)
            .any(|(i, _)| i == a.len())
    }

    /// Moves the terminal's cursor to `(x, y)` the shortest way. `absolute` asks for a sequence
    /// even when the cursor is already there, to keep the terminal from joining what follows to
    /// what it wrote last.
    fn move_to(&mut self, x: u16, y: u16, absolute: bool, out: &mut Vec<u8>) {
        let col1 = usize::from(x) + 1;
        if self.row == Some(y) {
            if self.col == Some(x) && !absolute {
                return;
            }
            let forward = self
                .col
                .filter(|&c| x > c && !absolute)
                .map(|c| usize::from(x - c));
            let cha = 3 + if col1 == 1 { 0 } else { diff::digits(col1) };
            let cuf = forward.map(|n| 3 + if n == 1 { 0 } else { diff::digits(n) });
            if x == 0 && !absolute {
                out.push(b'\r');
            } else if let Some(n) = forward.filter(|_| cuf.is_some_and(|c| c < cha)) {
                out.extend_from_slice(b"\x1b[");
                if n > 1 {
                    push_number(out, n);
                }
                out.push(b'C');
            } else {
                out.extend_from_slice(b"\x1b[");
                if col1 > 1 {
                    push_number(out, col1);
                }
                out.push(b'G');
            }
        } else {
            push_cup(out, x, y);
        }
        self.row = Some(y);
        self.col = Some(x);
    }

    /// Changes the terminal's pen to `pen` with the fewest bytes, from what it has or from SGR 0.
    fn set_pen(&mut self, pen: Pen, allowed: Allowed, out: &mut Vec<u8>) {
        // The underline's colour doesn't show without an underline, so a change of style keeps
        // the terminal's, and SGR 0 resets it.
        let fresh = match pen.underline {
            None => Pen {
                underline_color: Color::Reset,
                ..pen
            },
            Some(_) => pen,
        };
        let kept = match (pen.underline, self.pen) {
            (None, Some(current)) => Pen {
                underline_color: current.underline_color,
                ..pen
            },
            _ => fresh,
        };
        if self.pen == Some(kept) {
            return;
        }
        self.reset_params.clear();
        push_reset_form(fresh, allowed, &mut self.reset_params);
        let (params, pen) = match self.pen {
            Some(current) => {
                self.params.clear();
                push_delta(current, kept, allowed, &mut self.params);
                if self.params.len() <= self.reset_params.len() {
                    (&self.params, kept)
                } else {
                    (&self.reset_params, fresh)
                }
            }
            None => (&self.reset_params, fresh),
        };
        if !params.is_empty() {
            out.extend_from_slice(b"\x1b[");
            out.extend_from_slice(params);
            out.push(b'm');
        }
        self.pen = Some(pen);
    }

    /// Places and shows the cursor, or hides it, as the frame asks.
    fn place_cursor(
        &mut self,
        cursor: Option<&Cursor>,
        area: Rect,
        allowed: Allowed,
        out: &mut Vec<u8>,
    ) {
        let Some(cursor) = cursor.filter(|_| !area.is_empty()) else {
            if self.visible != Some(false) {
                out.extend_from_slice(HIDE);
                self.visible = Some(false);
            }
            return;
        };
        let position = Position::new(
            cursor.position.x.clamp(area.x, area.right() - 1),
            cursor.position.y.clamp(area.y, area.bottom() - 1),
        );
        let there = self.row == Some(position.y) && self.col == Some(position.x);
        let shown = self.visible == Some(true);
        if !there || !shown {
            push_cup(out, position.x, position.y);
            self.row = Some(position.y);
            self.col = Some(position.x);
        }
        let shape = match cursor.shape {
            CursorShape::Default => (CursorShape::Default, false),
            shape => (shape, cursor.blinking),
        };
        if allowed.cursor_shape && self.shape != Some(shape) {
            let n = match shape {
                (CursorShape::Default, _) => 0,
                (CursorShape::Block, blinking) => 2 - u8::from(blinking),
                (CursorShape::Underline, blinking) => 4 - u8::from(blinking),
                (CursorShape::Bar, blinking) => 6 - u8::from(blinking),
            };
            out.extend_from_slice(b"\x1b[");
            push_number(out, usize::from(n));
            out.extend_from_slice(b" q");
            self.shape = Some(shape);
        }
        if !there || !shown {
            out.extend_from_slice(SHOW);
            self.visible = Some(true);
        }
    }
}

/// The pen that draws `cell`, a cell of `next`, on this terminal.
fn pen_of(next: &Buffer, cell: &Cell, allowed: Allowed, lowering: &mut Lowering) -> Pen {
    let modifier = cell.modifier();
    let underline = modifier.contains(Modifier::UNDERLINED).then(|| {
        cell.underline_style()
            .filter(|_| allowed.underline_styles)
            .unwrap_or(UnderlineStyle::Single)
    });
    let underline_color = if underline.is_some() && allowed.underline_color {
        lowering.lower(next.cell_underline_color(cell), allowed.depth)
    } else {
        Color::Reset
    };
    Pen {
        fg: lowering.lower(cell.fg(), allowed.depth),
        bg: lowering.lower(cell.bg(), allowed.depth),
        underline_color,
        modifier: modifier - Modifier::UNDERLINED,
        underline,
    }
}

/// The column from which every cell of `row` to its end looks blank, or the row's length.
fn blank_tail(next: &Buffer, row: &[Cell]) -> usize {
    row.iter()
        .rposition(|cell| !next.looks_blank(cell))
        .map_or(0, |i| i + 1)
}

/// Opens a hyperlink to `url` with an id from its address, so it's the same link in every frame
/// and across rows. Returns whether one is open, which it isn't when there's no address or it
/// holds a control character, which would end the sequence early.
fn open_link(url: Option<&str>, out: &mut Vec<u8>) -> bool {
    let Some(url) = url.filter(|u| !u.is_empty() && !u.bytes().any(|b| b < 0x20 || b == 0x7f))
    else {
        return false;
    };
    let mut hash: u32 = 0x811c_9dc5;
    for b in url.bytes() {
        hash = (hash ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    out.extend_from_slice(b"\x1b]8;id=");
    for shift in (0..8).rev() {
        let digit = (hash >> (shift * 4)) & 0xF;
        out.push(b"0123456789abcdef"[digit as usize]);
    }
    out.push(b';');
    out.extend_from_slice(url.as_bytes());
    out.extend_from_slice(b"\x1b\\");
    true
}

/// Whether `c` can neither continue a cluster before it nor be continued by one after it, for the
/// common scripts and symbols, so two cells holding such characters never need checking.
fn starts_no_cluster(c: char) -> bool {
    matches!(
        c,
        '\u{20}'..='\u{7E}'
            | '\u{A0}'..='\u{AC}'
            | '\u{AE}'..='\u{2FF}'
            | '\u{370}'..='\u{3FF}'
            | '\u{400}'..='\u{482}'
            | '\u{48A}'..='\u{52F}'
            | '\u{2010}'..='\u{2027}'
            | '\u{2030}'..='\u{205E}'
            | '\u{2190}'..='\u{2BFF}'
            | '\u{2E80}'..='\u{3029}'
            | '\u{3030}'..='\u{3098}'
            | '\u{309B}'..='\u{30FF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{AC00}'..='\u{D7A3}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{FF01}'..='\u{FF9D}'
    )
}

/// `CSI row ; col H`, with the parameters terminals take as 1 left out.
fn push_cup(out: &mut Vec<u8>, x: u16, y: u16) {
    out.extend_from_slice(b"\x1b[");
    if (x, y) != (0, 0) {
        push_number(out, usize::from(y) + 1);
    }
    if x != 0 {
        out.push(b';');
        push_number(out, usize::from(x) + 1);
    }
    out.push(b'H');
}

/// Writes `n` in decimal.
pub(crate) fn push_number(out: &mut Vec<u8>, n: usize) {
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    let mut n = n;
    loop {
        i -= 1;
        digits[i] = b'0' + u8::try_from(n % 10).unwrap_or(0);
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&digits[i..]);
}

/// Adds one SGR parameter, with the separator before it when it isn't the first.
fn param(params: &mut Vec<u8>, n: usize) {
    if !params.is_empty() {
        params.push(b';');
    }
    push_number(params, n);
}

/// The parameters that turn on `modifier`, in SGR order.
fn push_modifiers_on(params: &mut Vec<u8>, modifier: Modifier) {
    for (flag, code) in [
        (Modifier::BOLD, 1),
        (Modifier::DIM, 2),
        (Modifier::ITALIC, 3),
        (Modifier::SLOW_BLINK, 5),
        (Modifier::RAPID_BLINK, 6),
        (Modifier::REVERSED, 7),
        (Modifier::HIDDEN, 8),
        (Modifier::CROSSED_OUT, 9),
        (Modifier::OVERLINED, 53),
    ] {
        if modifier.contains(flag) {
            param(params, code);
        }
    }
}

/// The underline, as `4`, or as `4:n` where the terminal knows the shapes.
fn push_underline(params: &mut Vec<u8>, shape: UnderlineStyle, allowed: Allowed) {
    let n = match shape {
        UnderlineStyle::Single => 1,
        UnderlineStyle::Double => 2,
        UnderlineStyle::Curly => 3,
        UnderlineStyle::Dotted => 4,
        UnderlineStyle::Dashed => 5,
    };
    param(params, 4);
    if n > 1 && allowed.underline_styles {
        params.extend_from_slice(b":");
        push_number(params, n);
    }
}

/// A colour for the text (`base` 30), the background (40) or the underline (50). The named colours
/// use 30 to 37 and 90 to 97, or 40 to 47 and 100 to 107, which every terminal knows.
fn push_color(params: &mut Vec<u8>, color: Color, base: usize) {
    match (color, ansi_index(color)) {
        (Color::Reset, _) => param(params, base + 9),
        (_, Some(n)) if base == 50 => {
            param(params, 58);
            param(params, 5);
            param(params, usize::from(n));
        }
        (_, Some(n)) if n < 8 => param(params, base + usize::from(n)),
        (_, Some(n)) => param(params, base + 60 + usize::from(n) - 8),
        (Color::Indexed(n), None) => {
            param(params, base + 8);
            param(params, 5);
            param(params, usize::from(n));
        }
        (Color::Rgb(r, g, b), None) => {
            param(params, base + 8);
            param(params, 2);
            param(params, usize::from(r));
            param(params, usize::from(g));
            param(params, usize::from(b));
        }
        (_, None) => {}
    }
}

/// The parameters that take the pen from `from` to `to`. Turning off bold or dim takes 22, which
/// turns off both, so the other comes back after it, and the blinks share 25 the same way.
fn push_delta(from: Pen, to: Pen, allowed: Allowed, params: &mut Vec<u8>) {
    let off = from.modifier - to.modifier;
    let mut on = to.modifier - from.modifier;
    let intensity = Modifier::BOLD | Modifier::DIM;
    let blink = Modifier::SLOW_BLINK | Modifier::RAPID_BLINK;
    if off.intersects(intensity) {
        param(params, 22);
        on |= to.modifier & intensity;
    }
    if off.contains(Modifier::ITALIC) {
        param(params, 23);
    }
    if from.underline.is_some() && to.underline.is_none() {
        param(params, 24);
    }
    if off.intersects(blink) {
        param(params, 25);
        on |= to.modifier & blink;
    }
    for (flag, code) in [
        (Modifier::REVERSED, 27),
        (Modifier::HIDDEN, 28),
        (Modifier::CROSSED_OUT, 29),
        (Modifier::OVERLINED, 55),
    ] {
        if off.contains(flag) {
            param(params, code);
        }
    }
    push_modifiers_on(params, on);
    if let Some(shape) = to.underline
        && to.underline != from.underline
    {
        push_underline(params, shape, allowed);
    }
    if to.fg != from.fg {
        push_color(params, to.fg, 30);
    }
    if to.bg != from.bg {
        push_color(params, to.bg, 40);
    }
    if to.underline.is_some() && to.underline_color != from.underline_color {
        push_color(params, to.underline_color, 50);
    }
}

/// The parameters that set `pen` from SGR 0, which always has the 0, so it's never empty.
fn push_reset_form(pen: Pen, allowed: Allowed, params: &mut Vec<u8>) {
    param(params, 0);
    push_modifiers_on(params, pen.modifier);
    if let Some(shape) = pen.underline {
        push_underline(params, shape, allowed);
        if pen.underline_color != Color::Reset {
            push_color(params, pen.underline_color, 50);
        }
    }
    if pen.fg != Color::Reset {
        push_color(params, pen.fg, 30);
    }
    if pen.bg != Color::Reset {
        push_color(params, pen.bg, 40);
    }
}
