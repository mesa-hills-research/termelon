//! The cell [`Buffer`] a frame draws into.
//!
//! A buffer is a grid of 16-byte cells over a [`Rect`]. A cell holds one character's UTF-8 inline,
//! or a tag and an index into the buffer's cluster table for a longer grapheme cluster. Rare data
//! (an underline colour, a hyperlink) lives in the buffer's extras table, so the common cell stays
//! small, and comparing two rows is a comparison of plain bytes.
//!
//! A wide character takes its first cell and continuation cells after it. The buffer keeps one
//! invariant on every write. A wide character always has all of its continuation cells, and a
//! continuation cell always follows its character. Writing over part of a wide character turns the
//! rest of it into spaces.
//!
//! Text goes in with [`set_string`](Buffer::set_string), [`set_span`](Buffer::set_span) and
//! [`set_line`](Buffer::set_line), which keep ratatui's rules. They clip at a width and never wrap,
//! they split text into grapheme clusters measured by the buffer's [`WidthProfile`], they skip
//! clusters that hold a control character or have no width, and they stop at the first cluster
//! that doesn't fit.
//!
//! ```
//! use tuiframe::buffer::Buffer;
//! use tuiframe::layout::Rect;
//! use tuiframe::style::{Color, Style};
//!
//! let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 1));
//! buffer.set_string(0, 0, "a界b", Style::new().fg(Color::Red));
//! assert_eq!(buffer.symbol((1, 0)), "界");
//! assert_eq!(buffer.symbol((2, 0)), "");
//! assert_eq!(buffer.cell((1, 0)).map(|c| c.width()), Some(2));
//! assert_eq!(buffer.cell((3, 0)).map(|c| c.fg()), Some(Color::Red));
//! assert!(buffer.cell((6, 0)).is_none());
//! ```

use std::collections::HashMap;
use std::fmt;
use std::hash::{BuildHasherDefault, Hasher};

use unicode_segmentation::UnicodeSegmentation;

use crate::caps::WidthProfile;
use crate::layout::{Position, Rect};
use crate::style::{Color, Modifier, Style, UnderlineStyle};
use crate::text::{self, Line, Span};

/// The first byte of a cell whose cluster is in the table, followed by its 24-bit index.
const CLUSTER_TAG: u8 = 0xFF;
/// The first byte of a cell that a wide character to its left covers.
const CONTINUATION_TAG: u8 = 0xFE;
/// The most clusters a buffer's table holds, the reach of a 24-bit index.
const MAX_CLUSTERS: usize = 1 << 24;

/// One cell as the buffer stores it.
///
/// `symbol` holds a cluster of up to 4 bytes of UTF-8 inline, padded with zeros (a cluster never
/// holds NUL, which is a control character), or a tag. Neither tag is a valid UTF-8 lead byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(C)]
pub(crate) struct Cell {
    symbol: [u8; 4],
    fg: PackedColor,
    bg: PackedColor,
    attrs: Attrs,
    extras: u16,
}

const _: () = assert!(size_of::<Cell>() == 16);

impl Cell {
    /// A space in the terminal's default colours.
    pub(crate) const BLANK: Self = Self {
        symbol: [b' ', 0, 0, 0],
        fg: PackedColor::RESET,
        bg: PackedColor::RESET,
        attrs: Attrs::EMPTY,
        extras: 0,
    };

    const CONTINUATION_SYMBOL: [u8; 4] = [CONTINUATION_TAG, 0, 0, 0];
    const SPACE_SYMBOL: [u8; 4] = [b' ', 0, 0, 0];

    /// The cell as two numbers, the symbol and the text colour, then the background, the
    /// attributes and the extras.
    pub(crate) const fn words(&self) -> (u64, u64) {
        let [a, b, c, d] = self.symbol;
        let low = u32::from_le_bytes([a, b, c, d]) as u64 | (self.fg.0 as u64) << 32;
        let high = self.bg.0 as u64 | (self.attrs.0 as u64) << 32 | (self.extras as u64) << 48;
        (low, high)
    }

    /// The cell as one number, for comparing rows without a branch a cell.
    pub(crate) const fn bits(&self) -> u128 {
        let (low, high) = self.words();
        low as u128 | (high as u128) << 64
    }

    pub(crate) const fn is_continuation(&self) -> bool {
        self.symbol[0] == CONTINUATION_TAG
    }

    const fn cluster_index(&self) -> Option<usize> {
        if self.symbol[0] == CLUSTER_TAG {
            let [_, a, b, c] = self.symbol;
            Some(a as usize | (b as usize) << 8 | (c as usize) << 16)
        } else {
            None
        }
    }

    /// The inline UTF-8, empty for a tagged cell.
    pub(crate) fn inline_bytes(&self) -> &[u8] {
        match self.symbol[0] {
            CLUSTER_TAG | CONTINUATION_TAG => &[],
            _ => {
                let len = self.symbol.iter().position(|&b| b == 0).unwrap_or(4);
                &self.symbol[..len]
            }
        }
    }

    pub(crate) const fn fg(&self) -> Color {
        self.fg.unpack()
    }

    pub(crate) const fn bg(&self) -> Color {
        self.bg.unpack()
    }

    pub(crate) const fn modifier(&self) -> Modifier {
        self.attrs.modifier()
    }

    pub(crate) const fn underline_style(&self) -> Option<UnderlineStyle> {
        self.attrs.underline_style()
    }

    pub(crate) const fn uncertain_width(&self) -> bool {
        self.attrs.uncertain()
    }

    /// Applies `style` as ratatui's `Cell::set_style` does, except the underline colour, which
    /// lives in the extras and is the caller's to set.
    fn patch(&mut self, style: Style) {
        if let Some(fg) = style.fg {
            self.fg = PackedColor::pack(fg);
        }
        if let Some(bg) = style.bg {
            self.bg = PackedColor::pack(bg);
        }
        if let Some(shape) = style.underline_style {
            self.attrs.set_underline_style(Some(shape));
        }
        let mut modifier = self.attrs.modifier();
        modifier.insert(style.add_modifier);
        modifier.remove(style.sub_modifier);
        self.attrs.set_modifier(modifier);
    }
}

/// A symbol to write and the cells it takes.
#[derive(Debug, Clone, Copy)]
struct Glyph {
    symbol: [u8; 4],
    width: u8,
    uncertain: bool,
}

impl Glyph {
    const SPACE: Self = Self::ascii(b' ');

    const fn ascii(b: u8) -> Self {
        Self {
            symbol: [b, 0, 0, 0],
            width: 1,
            uncertain: false,
        }
    }
}

/// A [`Color`] in a `u32`, its kind in the top byte and its value below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PackedColor(u32);

impl PackedColor {
    const RESET: Self = Self(0);
    const NAMED: u32 = 1 << 24;
    const INDEXED: u32 = 2 << 24;
    const RGB: u32 = 3 << 24;

    pub(crate) fn pack(color: Color) -> Self {
        let named = |n: u32| Self(Self::NAMED | n);
        match color {
            Color::Reset => Self::RESET,
            Color::Black => named(0),
            Color::Red => named(1),
            Color::Green => named(2),
            Color::Yellow => named(3),
            Color::Blue => named(4),
            Color::Magenta => named(5),
            Color::Cyan => named(6),
            Color::Gray => named(7),
            Color::DarkGray => named(8),
            Color::LightRed => named(9),
            Color::LightGreen => named(10),
            Color::LightYellow => named(11),
            Color::LightBlue => named(12),
            Color::LightMagenta => named(13),
            Color::LightCyan => named(14),
            Color::White => named(15),
            Color::Indexed(i) => Self(Self::INDEXED | u32::from(i)),
            Color::Rgb(r, g, b) => Self(Self::RGB | u32::from_be_bytes([0, r, g, b])),
        }
    }

    pub(crate) const fn unpack(self) -> Color {
        let [kind, r, g, b] = self.0.to_be_bytes();
        match kind {
            1 => named_color(b),
            2 => Color::Indexed(b),
            3 => Color::Rgb(r, g, b),
            _ => Color::Reset,
        }
    }
}

/// The named colour with ANSI number `n`, 0 to 15.
pub(crate) const fn named_color(n: u8) -> Color {
    match n {
        0 => Color::Black,
        1 => Color::Red,
        2 => Color::Green,
        3 => Color::Yellow,
        4 => Color::Blue,
        5 => Color::Magenta,
        6 => Color::Cyan,
        7 => Color::Gray,
        8 => Color::DarkGray,
        9 => Color::LightRed,
        10 => Color::LightGreen,
        11 => Color::LightYellow,
        12 => Color::LightBlue,
        13 => Color::LightMagenta,
        14 => Color::LightCyan,
        _ => Color::White,
    }
}

/// A cell's modifiers in bits 0 to 9, its underline shape in bits 10 to 12 (0 for none), and in
/// bit 13 whether terminals may disagree on its width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Attrs(u16);

impl Attrs {
    const EMPTY: Self = Self(0);
    const MODIFIER_MASK: u16 = (1 << 10) - 1;
    const UNDERLINE_SHIFT: u16 = 10;
    const UNDERLINE_MASK: u16 = 0b111 << Self::UNDERLINE_SHIFT;
    const UNCERTAIN: u16 = 1 << 13;

    const fn modifier(self) -> Modifier {
        Modifier::from_bits_truncate(self.0 & Self::MODIFIER_MASK)
    }

    const fn set_modifier(&mut self, modifier: Modifier) {
        self.0 = (self.0 & !Self::MODIFIER_MASK) | modifier.bits();
    }

    const fn underline_style(self) -> Option<UnderlineStyle> {
        match (self.0 & Self::UNDERLINE_MASK) >> Self::UNDERLINE_SHIFT {
            1 => Some(UnderlineStyle::Single),
            2 => Some(UnderlineStyle::Double),
            3 => Some(UnderlineStyle::Curly),
            4 => Some(UnderlineStyle::Dotted),
            5 => Some(UnderlineStyle::Dashed),
            _ => None,
        }
    }

    const fn set_underline_style(&mut self, shape: Option<UnderlineStyle>) {
        let bits = match shape {
            None => 0,
            Some(UnderlineStyle::Single) => 1,
            Some(UnderlineStyle::Double) => 2,
            Some(UnderlineStyle::Curly) => 3,
            Some(UnderlineStyle::Dotted) => 4,
            Some(UnderlineStyle::Dashed) => 5,
        };
        self.0 = (self.0 & !Self::UNDERLINE_MASK) | bits << Self::UNDERLINE_SHIFT;
    }

    const fn uncertain(self) -> bool {
        self.0 & Self::UNCERTAIN != 0
    }

    const fn set_uncertain(&mut self, uncertain: bool) {
        if uncertain {
            self.0 |= Self::UNCERTAIN;
        } else {
            self.0 &= !Self::UNCERTAIN;
        }
    }
}

/// FNV-1a, for the tables' indexes, which hash short strings and small structs.
#[derive(Debug, Clone, Copy)]
struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }
}

impl Hasher for Fnv {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

type FnvMap<K, V> = HashMap<K, V, BuildHasherDefault<Fnv>>;

fn hash_str(s: &str) -> u64 {
    let mut hasher = Fnv::default();
    hasher.write(s.as_bytes());
    hasher.finish()
}

/// Strings stored once each, by index. Clearing keeps the capacity, so a frame allocates nothing
/// once the table reaches its working size.
#[derive(Debug, Clone, Default)]
struct Interner {
    text: String,
    ends: Vec<u32>,
    index: FnvMap<u64, u32>,
}

impl Interner {
    fn clear(&mut self) {
        self.text.clear();
        self.ends.clear();
        self.index.clear();
    }

    #[cfg(test)]
    fn len_for_tests(&self) -> usize {
        self.ends.len()
    }

    fn get(&self, i: usize) -> &str {
        let end = self.ends.get(i).map_or(0, |&e| e as usize);
        let start = match i.checked_sub(1) {
            Some(before) => self.ends.get(before).map_or(0, |&e| e as usize),
            None => 0,
        };
        self.text.get(start..end).unwrap_or("")
    }

    /// The index of `s`, stored if it's new. `None` when the table is full.
    fn intern(&mut self, s: &str, limit: usize) -> Option<u32> {
        let hash = hash_str(s);
        if let Some(&i) = self.index.get(&hash)
            && self.get(i as usize) == s
        {
            return Some(i);
        }
        let i = u32::try_from(self.ends.len()).ok()?;
        let end = u32::try_from(self.text.len() + s.len()).ok()?;
        if self.ends.len() >= limit {
            return None;
        }
        self.text.push_str(s);
        self.ends.push(end);
        self.index.entry(hash).or_insert(i);
        Some(i)
    }
}

/// What a cell holds besides its symbol, colours and attributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Extra {
    underline_color: PackedColor,
    /// 0, or 1 more than the link's index in the links table.
    link: u32,
}

impl Extra {
    const NONE: Self = Self {
        underline_color: PackedColor::RESET,
        link: 0,
    };
}

/// The extras of a buffer's cells, each stored once, by an index from 1.
#[derive(Debug, Clone, Default)]
struct ExtrasTable {
    entries: Vec<Extra>,
    index: FnvMap<Extra, u16>,
    links: Interner,
}

impl ExtrasTable {
    fn clear(&mut self) {
        self.entries.clear();
        self.index.clear();
        self.links.clear();
    }

    fn get(&self, i: u16) -> Extra {
        usize::from(i)
            .checked_sub(1)
            .and_then(|i| self.entries.get(i))
            .copied()
            .unwrap_or(Extra::NONE)
    }

    /// The index of `extra`, 0 for none. A frame with more than 65,535 distinct extras drops the
    /// rest. `shortcut: extras beyond u16, widen the index if a program ever reaches it`.
    fn intern(&mut self, extra: Extra) -> u16 {
        if extra == Extra::NONE {
            return 0;
        }
        if let Some(&i) = self.index.get(&extra) {
            return i;
        }
        let Ok(i) = u16::try_from(self.entries.len() + 1) else {
            return 0;
        };
        self.entries.push(extra);
        self.index.insert(extra, i);
        i
    }

    fn link(&self, extra: Extra) -> Option<&str> {
        let i = usize::try_from(extra.link).ok()?.checked_sub(1)?;
        Some(self.links.get(i))
    }
}

/// A grid of cells over an area, which a frame draws into and the diff compares.
///
/// Positions are absolute, so a buffer over `Rect::new(10, 5, 20, 3)` holds the cells from
/// `(10, 5)` to `(29, 7)`. Writes outside the area are dropped, and reads outside it give `None`.
#[derive(Clone)]
pub struct Buffer {
    area: Rect,
    cells: Vec<Cell>,
    clusters: Interner,
    extras: ExtrasTable,
    /// Rows that hold a cluster or an extra, whose cells the diff compares by their resolved
    /// values.
    rich_rows: Vec<bool>,
    width_profile: WidthProfile,
    /// The regions writes may change, as rows `top..bottom` and columns `left..right` counted
    /// inside the area, when `clipped`.
    clip: Vec<ClipRegion>,
    clipped: bool,
    /// For each row, the columns outside the clip that writes under it changed, through wide
    /// characters across its edges, from the first to the one after the last.
    spill: Vec<Option<(usize, usize)>>,
}

/// A region of a clip, in rows and columns counted inside the buffer's area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClipRegion {
    top: usize,
    bottom: usize,
    left: usize,
    right: usize,
}

impl Buffer {
    /// A buffer of blank cells over `area`, measuring text with the current
    /// [`text::width_profile`], the terminal's once it's open.
    pub fn empty(area: Rect) -> Self {
        Self::with_width_profile(area, text::width_profile())
    }

    /// A buffer of blank cells over `area`, measuring text with `profile`.
    pub fn with_width_profile(area: Rect, profile: WidthProfile) -> Self {
        let mut buffer = Self {
            area: Rect::ZERO,
            cells: Vec::new(),
            clusters: Interner::default(),
            extras: ExtrasTable::default(),
            rich_rows: Vec::new(),
            width_profile: profile,
            clip: Vec::new(),
            clipped: false,
            spill: Vec::new(),
        };
        buffer.resize(area);
        buffer
    }

    /// The area it covers.
    pub fn area(&self) -> Rect {
        self.area
    }

    /// How it measures text.
    pub fn width_profile(&self) -> WidthProfile {
        self.width_profile
    }

    /// Measures the text written from now on with `profile`. Cells already written keep their
    /// widths.
    pub fn set_width_profile(&mut self, profile: WidthProfile) {
        self.width_profile = profile;
    }

    /// Blanks every cell and empties the cluster and extras tables, keeping their memory. Under a
    /// clip, it blanks the cells the clip allows and keeps the tables.
    pub fn reset(&mut self) {
        if self.clipped {
            let width = usize::from(self.area.width);
            for i in 0..self.clip.len() {
                let region = self.clip[i];
                for row in region.top..region.bottom {
                    for col in region.left..region.right {
                        self.reset_cell(row * width + col);
                    }
                }
            }
            return;
        }
        self.cells.fill(Cell::BLANK);
        self.clusters.clear();
        self.extras.clear();
        self.rich_rows.fill(false);
    }

    /// Covers `area` instead, with every cell blank and no clip.
    pub fn resize(&mut self, area: Rect) {
        let area = Rect::new(area.x, area.y, area.width, area.height);
        self.area = area;
        self.clip.clear();
        self.clipped = false;
        self.spill.clear();
        self.cells.clear();
        self.cells.resize(area.area() as usize, Cell::BLANK);
        self.rich_rows.clear();
        self.rich_rows.resize(usize::from(area.height), false);
        self.clusters.clear();
        self.extras.clear();
    }

    /// Writes `string` from `(x, y)` to the right edge of the area, merging `style` over each
    /// cell's.
    pub fn set_string<T: AsRef<str>, S: Into<Style>>(
        &mut self,
        x: u16,
        y: u16,
        string: T,
        style: S,
    ) {
        self.set_stringn(x, y, string, usize::MAX, style);
    }

    /// Writes `string` from `(x, y)` into at most `max_width` cells, merging `style` over each
    /// cell's, and returns the position after the last cluster written.
    ///
    /// It skips clusters that hold a control character or take no cells, and stops at the first
    /// cluster that doesn't fit. Nothing is written when `(x, y)` is outside the area.
    pub fn set_stringn<T: AsRef<str>, S: Into<Style>>(
        &mut self,
        x: u16,
        y: u16,
        string: T,
        max_width: usize,
        style: S,
    ) -> (u16, u16) {
        let style = style.into();
        let string = string.as_ref();
        let Some(row) = self.row_start(y) else {
            return (x, y);
        };
        if x < self.area.x || x >= self.area.right() {
            return (x, y);
        }
        let max_width = u16::try_from(max_width).unwrap_or(u16::MAX);
        let mut remaining = (self.area.right() - x).min(max_width);
        let mut col = usize::from(x - self.area.x);
        let mut written = 0;
        if string.is_ascii() {
            for &b in string.as_bytes() {
                if b.is_ascii_control() {
                    continue;
                }
                if remaining == 0 {
                    break;
                }
                self.put(row, col, Glyph::ascii(b), Some(style));
                col += 1;
                written += 1;
                remaining -= 1;
            }
        } else {
            let profile = self.width_profile;
            for cluster in string.graphemes(true) {
                let m = text::measure(cluster, profile);
                if m.width == 0 {
                    continue;
                }
                let width = u16::from(m.width);
                if width > remaining {
                    break;
                }
                self.put_cluster(row, col, cluster, m, Some(style));
                col += usize::from(m.width);
                written += width;
                remaining -= width;
            }
        }
        (x + written, y)
    }

    /// Writes `span` from `(x, y)` into at most `max_width` cells, as
    /// [`set_stringn`](Self::set_stringn) does, in the span's style.
    pub fn set_span(&mut self, x: u16, y: u16, span: &Span<'_>, max_width: u16) -> (u16, u16) {
        self.set_stringn(x, y, &span.content, usize::from(max_width), span.style)
    }

    /// Writes `line` from `(x, y)` into at most `max_width` cells, each span in the line's style
    /// patched with its own, and returns the position after the last cluster written. The line's
    /// alignment is for widgets and is ignored here.
    pub fn set_line(&mut self, x: u16, y: u16, line: &Line<'_>, max_width: u16) -> (u16, u16) {
        let mut remaining = max_width;
        let mut x = x;
        for span in &line.spans {
            if remaining == 0 {
                break;
            }
            let end = self.set_stringn(
                x,
                y,
                &span.content,
                usize::from(remaining),
                line.style.patch(span.style),
            );
            let w = end.0.saturating_sub(x);
            x = end.0;
            remaining = remaining.saturating_sub(w);
        }
        (x, y)
    }

    /// Merges `style` over every cell of `area` inside the buffer. Colours that are `Some`
    /// replace the cell's, modifiers are added and removed, and the symbols stay.
    pub fn set_style<S: Into<Style>>(&mut self, area: Rect, style: S) {
        let style = style.into();
        let area = self.area.intersection(area);
        for y in area.top()..area.bottom() {
            let Some(row) = self.row_start(y) else {
                continue;
            };
            let from = usize::from(area.x - self.area.x);
            let to = usize::from(area.right() - self.area.x);
            for col in from..to {
                self.patch_cell(row + col, style);
            }
            if style.underline_color.is_some_and(|c| c != Color::Reset) {
                self.mark_rich(y);
            }
        }
    }

    /// Makes every cell of `area` inside the buffer a hyperlink to `url`, or no link with `None`.
    /// Cells with the same link next to each other are one link to the terminal, across rows too.
    pub fn set_link(&mut self, area: Rect, url: Option<&str>) {
        let area = self.area.intersection(area);
        let link = self.link_ref(url);
        for y in area.top()..area.bottom() {
            let Some(row) = self.row_start(y) else {
                continue;
            };
            let from = usize::from(area.x - self.area.x);
            let to = usize::from(area.right() - self.area.x);
            for col in from..to {
                self.set_cell_link(row + col, link);
            }
            if link != 0 {
                self.mark_rich(y);
            }
        }
    }

    /// The cell at `position`, or `None` outside the area.
    pub fn cell<P: Into<Position>>(&self, position: P) -> Option<CellRef<'_>> {
        let index = self.index_of(position.into())?;
        Some(CellRef {
            buffer: self,
            index,
        })
    }

    /// The cell at `position` to change, or `None` outside the area.
    pub fn cell_mut<P: Into<Position>>(&mut self, position: P) -> Option<CellMut<'_>> {
        let position = position.into();
        let index = self.index_of(position)?;
        Some(CellMut {
            buffer: self,
            index,
            y: position.y,
        })
    }

    /// The text of the cell at `position`, empty for the continuation cell of a wide character
    /// and outside the area. Joining a row's symbols gives the row as a terminal shows it.
    pub fn symbol<P: Into<Position>>(&self, position: P) -> &str {
        self.index_of(position.into())
            .map_or("", |index| self.symbol_at(index))
    }

    /// The text of the cell at `index`, empty for a continuation cell.
    pub(crate) fn symbol_at(&self, index: usize) -> &str {
        let Some(cell) = self.cells.get(index) else {
            return "";
        };
        self.cell_symbol(cell)
    }

    /// The text of `cell`, a cell of this buffer.
    pub(crate) fn cell_symbol<'a>(&'a self, cell: &'a Cell) -> &'a str {
        match cell.cluster_index() {
            Some(i) => self.clusters.get(i),
            None => core::str::from_utf8(cell.inline_bytes()).unwrap_or(" "),
        }
    }

    /// The underline colour of `cell`, a cell of this buffer.
    pub(crate) fn cell_underline_color(&self, cell: &Cell) -> Color {
        self.extras.get(cell.extras).underline_color.unpack()
    }

    /// The hyperlink of `cell`, a cell of this buffer.
    pub(crate) fn cell_link(&self, cell: &Cell) -> Option<&str> {
        self.extras.link(self.extras.get(cell.extras))
    }

    /// Whether `a`, a cell of this buffer, shows what `b`, a cell of `other`, shows.
    pub(crate) fn same_cell(&self, a: &Cell, other: &Self, b: &Cell) -> bool {
        if a == b && a.symbol[0] != CLUSTER_TAG && a.extras == 0 {
            return true;
        }
        a.fg == b.fg
            && a.bg == b.bg
            && a.attrs == b.attrs
            && a.is_continuation() == b.is_continuation()
            && self.cell_symbol(a) == other.cell_symbol(b)
            && self.cell_underline_color(a) == other.cell_underline_color(b)
            && self.cell_link(a) == other.cell_link(b)
    }

    fn index_of(&self, position: Position) -> Option<usize> {
        if !self.area.contains(position) {
            return None;
        }
        let x = usize::from(position.x - self.area.x);
        let y = usize::from(position.y - self.area.y);
        Some(y * usize::from(self.area.width) + x)
    }

    /// The index of the first cell of row `y`, or `None` outside the area.
    fn row_start(&self, y: u16) -> Option<usize> {
        if y < self.area.y || y >= self.area.bottom() || self.area.width == 0 {
            return None;
        }
        Some(usize::from(y - self.area.y) * usize::from(self.area.width))
    }

    fn mark_rich(&mut self, y: u16) {
        if let Some(flag) = y
            .checked_sub(self.area.y)
            .and_then(|row| self.rich_rows.get_mut(usize::from(row)))
        {
            *flag = true;
        }
    }

    /// Writes `cluster`, measured as `m`, at column `col` of the row starting at `row`. The caller
    /// has checked that it fits.
    fn put_cluster(
        &mut self,
        row: usize,
        col: usize,
        cluster: &str,
        m: text::Measure,
        style: Option<Style>,
    ) {
        let bytes = cluster.as_bytes();
        let symbol = if bytes.len() <= 4 {
            let mut symbol = [0; 4];
            symbol[..bytes.len()].copy_from_slice(bytes);
            symbol
        } else if let Some(i) = self.clusters.intern(cluster, MAX_CLUSTERS) {
            let [a, b, c, _] = i.to_le_bytes();
            [CLUSTER_TAG, a, b, c]
        } else {
            for k in 0..usize::from(m.width) {
                self.put(row, col + k, Glyph::SPACE, style);
            }
            return;
        };
        let glyph = Glyph {
            symbol,
            width: m.width,
            uncertain: m.uncertain,
        };
        self.put(row, col, glyph, style);
    }

    /// Writes `glyph` at column `col` of the row starting at `row`, merging `style` over the
    /// cell's, and keeps the wide-character invariant. The caller has checked that it fits.
    fn put(&mut self, row: usize, col: usize, glyph: Glyph, style: Option<Style>) {
        let width = usize::from(glyph.width.max(1));
        let row_len = usize::from(self.area.width);
        if !self.may_change(row + col) {
            return;
        }
        self.note_spill(row, col, col + width);
        self.break_wide(row, row_len, col, col + width);
        let i = row + col;
        if let Some(style) = style {
            self.patch_cell(i, style);
        }
        let Some(cell) = self.cells.get_mut(i) else {
            return;
        };
        cell.symbol = glyph.symbol;
        cell.attrs.set_uncertain(glyph.uncertain);
        let mut continuation = *cell;
        continuation.symbol = Cell::CONTINUATION_SYMBOL;
        continuation.attrs.set_uncertain(false);
        let rich = glyph.symbol[0] == CLUSTER_TAG || cell.extras != 0;
        let end = (i + width).min(row + row_len);
        if let Some(cells) = self.cells.get_mut(i + 1..end) {
            cells.fill(continuation);
        }
        if rich && let Some(flag) = self.rich_rows.get_mut(row / row_len.max(1)) {
            *flag = true;
        }
    }

    /// Turns into spaces the parts outside columns `from..to` of any wide character that crosses
    /// either end, in the row starting at `row`.
    fn break_wide(&mut self, row: usize, row_len: usize, from: usize, to: usize) {
        let Some(cells) = self.cells.get_mut(row..row + row_len) else {
            return;
        };
        if cells.get(from).is_some_and(Cell::is_continuation) {
            for cell in cells[..from].iter_mut().rev() {
                let leader = !cell.is_continuation();
                cell.symbol = Cell::SPACE_SYMBOL;
                cell.attrs.set_uncertain(false);
                if leader {
                    break;
                }
            }
        }
        for cell in cells.iter_mut().skip(to) {
            if !cell.is_continuation() {
                break;
            }
            cell.symbol = Cell::SPACE_SYMBOL;
        }
    }

    /// Merges `style` over the cell at `index`, its underline colour included.
    fn patch_cell(&mut self, index: usize, style: Style) {
        if !self.may_change(index) {
            return;
        }
        let Some(cell) = self.cells.get_mut(index) else {
            return;
        };
        cell.patch(style);
        if let Some(color) = style.underline_color {
            let mut extra = self.extras.get(cell.extras);
            extra.underline_color = PackedColor::pack(color);
            cell.extras = self.extras.intern(extra);
        }
    }

    /// The link value an [`Extra`] holds for `url`.
    fn link_ref(&mut self, url: Option<&str>) -> u32 {
        url.and_then(|url| self.extras.links.intern(url, usize::MAX))
            .map_or(0, |i| i.saturating_add(1))
    }

    fn set_cell_link(&mut self, index: usize, link: u32) {
        if !self.may_change(index) {
            return;
        }
        let Some(cell) = self.cells.get_mut(index) else {
            return;
        };
        let mut extra = self.extras.get(cell.extras);
        extra.link = link;
        cell.extras = self.extras.intern(extra);
    }

    /// Writes the first cluster of `symbol` at `index`, in row `y`, keeping the cell's style.
    fn set_cell_symbol(&mut self, index: usize, y: u16, symbol: &str) {
        let Some(row) = self.row_start(y) else {
            return;
        };
        let col = index - row;
        let row_len = usize::from(self.area.width);
        let cluster = symbol.graphemes(true).next().unwrap_or(" ");
        let m = text::measure(cluster, self.width_profile);
        if m.width == 0 || col + usize::from(m.width) > row_len {
            self.put(row, col, Glyph::SPACE, None);
        } else {
            self.put_cluster(row, col, cluster, m, None);
        }
    }

    /// Makes the cell at `index` blank.
    fn reset_cell(&mut self, index: usize) {
        let width = usize::from(self.area.width);
        if width == 0 || index >= self.cells.len() {
            return;
        }
        let (row, col) = (index - index % width, index % width);
        if !self.may_change(index) {
            return;
        }
        self.note_spill(row, col, col + 1);
        self.break_wide(row, width, col, col + 1);
        if let Some(cell) = self.cells.get_mut(index) {
            *cell = Cell::BLANK;
        }
    }

    /// Records the cells outside the clip that writing columns `from..to` of the row starting at
    /// `row` changes, those columns and the parts of wide characters it turns into spaces.
    fn note_spill(&mut self, row: usize, from: usize, to: usize) {
        if !self.clipped {
            return;
        }
        let row_len = usize::from(self.area.width);
        let Some(cells) = self.cells.get(row..row + row_len) else {
            return;
        };
        let mut start = from;
        while start > 0 && cells.get(start).is_some_and(Cell::is_continuation) {
            start -= 1;
        }
        let mut end = to.min(row_len);
        while end < row_len && cells[end].is_continuation() {
            end += 1;
        }
        let y = row / row_len.max(1);
        for col in start..end {
            if !self.may_change(row + col)
                && let Some(spill) = self.spill.get_mut(y)
            {
                *spill = Some(spill.map_or((col, col + 1), |(a, b)| (a.min(col), b.max(col + 1))));
            }
        }
    }

    /// Whether the clip allows changing the cell at `index`.
    fn may_change(&self, index: usize) -> bool {
        if !self.clipped {
            return true;
        }
        let width = usize::from(self.area.width).max(1);
        let (row, col) = (index / width, index % width);
        self.clip
            .iter()
            .any(|r| r.top <= row && row < r.bottom && r.left <= col && col < r.right)
    }

    /// The text of each row, with continuation cells left out.
    fn row_texts(&self) -> Vec<String> {
        let width = usize::from(self.area.width).max(1);
        self.cells
            .chunks(width)
            .map(|row| row.iter().map(|cell| self.cell_symbol(cell)).collect())
            .collect()
    }
}

/// The clip that partial frames draw under.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "`Terminal::draw_regions` sets the clip, and comes with the terminal API"
    )
)]
impl Buffer {
    /// Drops every write from now on to a cell outside `regions`, whatever the call, until
    /// [`clear_clip`](Self::clear_clip). Text goes on past the cells it couldn't write, so a
    /// string over two regions lands in both. A wide character that starts inside is written
    /// whole, and one that starts outside is turned into spaces when a write inside covers part
    /// of it, so the cells a write changes outside are the parts of wide characters that cross a
    /// region's edge, which the diff of a partial frame takes in.
    pub(crate) fn set_clip(&mut self, regions: &[Rect]) {
        self.clip.clear();
        let area = self.area;
        for region in regions {
            let r = region.intersection(area);
            if r.is_empty() {
                continue;
            }
            self.clip.push(ClipRegion {
                top: usize::from(r.y - area.y),
                bottom: usize::from(r.bottom() - area.y),
                left: usize::from(r.x - area.x),
                right: usize::from(r.right() - area.x),
            });
        }
        self.clipped = true;
        self.spill.clear();
        self.spill.resize(usize::from(area.height), None);
    }

    /// Lets writes change every cell again. What writes under the clip spilled past it stays
    /// known until the next clip.
    pub(crate) fn clear_clip(&mut self) {
        self.clip.clear();
        self.clipped = false;
    }

    /// The columns of row `y`, counted inside the area, outside the last clip that writes under
    /// it changed, from the first to the one after the last.
    pub(crate) fn clip_spill(&self, y: usize) -> Option<(usize, usize)> {
        self.spill.get(y).copied().flatten()
    }
}

/// Reading for the diff and the encoder.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the encoder reads cells through these, and `Terminal` runs the encoder"
    )
)]
impl Buffer {
    /// Modifiers that show on a space.
    const VISIBLE_ON_BLANK: Modifier = Modifier::REVERSED
        .union(Modifier::UNDERLINED)
        .union(Modifier::SLOW_BLINK)
        .union(Modifier::RAPID_BLINK)
        .union(Modifier::CROSSED_OUT)
        .union(Modifier::OVERLINED);

    /// The cells of row `y` of the area, counted from its top.
    pub(crate) fn row(&self, y: usize) -> &[Cell] {
        let width = usize::from(self.area.width);
        self.cells.get(y * width..(y + 1) * width).unwrap_or(&[])
    }

    /// Whether two rows hold the same cells, compared as plain numbers a chunk at a time so the
    /// comparison vectorises.
    pub(crate) fn rows_equal(a: &[Cell], b: &[Cell]) -> bool {
        a.len() == b.len()
            && a.chunks(16).zip(b.chunks(16)).all(|(x, y)| {
                x.iter()
                    .zip(y)
                    .fold(0, |acc, (p, q)| acc | (p.bits() ^ q.bits()))
                    == 0
            })
    }

    /// Whether row `y` of the area, counted from its top, may hold a cluster or an extra, so that
    /// its cells must be compared by what they show.
    pub(crate) fn is_rich_row(&self, y: usize) -> bool {
        self.rich_rows.get(y).copied().unwrap_or(false)
    }

    /// Which link `cell`, a cell of this buffer, belongs to, as a number that's the same for
    /// cells of this buffer with the same link, and 0 for none.
    pub(crate) fn link_index(&self, cell: &Cell) -> u32 {
        self.extras.get(cell.extras).link
    }

    /// Whether `cell`, a cell of this buffer, shows as a blank cell does, as a space on the default
    /// background with no modifier that shows on a space and no link.
    pub(crate) fn looks_blank(&self, cell: &Cell) -> bool {
        cell.symbol == Cell::SPACE_SYMBOL
            && cell.bg == PackedColor::RESET
            && !cell.modifier().intersects(Self::VISIBLE_ON_BLANK)
            && (cell.extras == 0 || self.link_index(cell) == 0)
    }

    /// Whether two cells of this buffer have the same colours, attributes and extras.
    pub(crate) fn same_style(a: &Cell, b: &Cell) -> bool {
        let mut attrs = (a.attrs, b.attrs);
        attrs.0.set_uncertain(false);
        attrs.1.set_uncertain(false);
        a.fg == b.fg && a.bg == b.bg && attrs.0 == attrs.1 && a.extras == b.extras
    }

    /// A hash of what row `y` shows, the same for equal rows of any two buffers.
    pub(crate) fn hash_row(&self, y: usize) -> u64 {
        const K: u64 = 0x9E37_79B9_7F4A_7C15;
        let mix = |h: u64, v: u64| (h.rotate_left(5) ^ v).wrapping_mul(K);
        let mut h = 0;
        for cell in self.row(y) {
            let (low, high) = cell.words();
            if cell.cluster_index().is_none() && cell.extras == 0 {
                h = mix(mix(h, low), high);
                continue;
            }
            let mut hasher = Fnv::default();
            match cell.cluster_index() {
                Some(i) => hasher.write(self.clusters.get(i).as_bytes()),
                None => hasher.write(&cell.symbol),
            }
            let extra = self.extras.get(cell.extras);
            hasher.write(&extra.underline_color.0.to_le_bytes());
            hasher.write(self.extras.link(extra).unwrap_or("").as_bytes());
            // The colours and attributes, without the symbol or the extras' index.
            h = mix(mix(h, hasher.finish()), low >> 32);
            h = mix(h, high & 0x0000_FFFF_FFFF_FFFF);
        }
        h
    }
}

impl PartialEq for Buffer {
    /// Two buffers are equal when they cover the same area and every cell shows the same, whatever
    /// their tables hold.
    fn eq(&self, other: &Self) -> bool {
        self.area == other.area
            && self
                .cells
                .iter()
                .zip(&other.cells)
                .all(|(a, b)| self.same_cell(a, other, b))
    }
}

impl Eq for Buffer {}

impl fmt::Debug for Buffer {
    /// Shows the area, the width profile and each row's text, without styles.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Buffer")
            .field("area", &self.area)
            .field("width_profile", &self.width_profile)
            .field("rows", &self.row_texts())
            .finish()
    }
}

/// A cell of a [`Buffer`], to read, from [`Buffer::cell`].
#[derive(Clone, Copy)]
pub struct CellRef<'a> {
    buffer: &'a Buffer,
    index: usize,
}

impl<'a> CellRef<'a> {
    fn raw(&self) -> Cell {
        self.buffer
            .cells
            .get(self.index)
            .copied()
            .unwrap_or(Cell::BLANK)
    }

    /// Its text, a grapheme cluster, or empty when a wide character to its left covers it.
    pub fn symbol(&self) -> &'a str {
        self.buffer.symbol_at(self.index)
    }

    /// The text's colour.
    pub fn fg(&self) -> Color {
        self.raw().fg()
    }

    /// The background colour.
    pub fn bg(&self) -> Color {
        self.raw().bg()
    }

    /// The modifiers.
    pub fn modifier(&self) -> Modifier {
        self.raw().modifier()
    }

    /// The underline's shape, if a style set one. The underline shows only with
    /// [`Modifier::UNDERLINED`].
    pub fn underline_style(&self) -> Option<UnderlineStyle> {
        self.raw().underline_style()
    }

    /// The underline's colour, [`Color::Reset`] for the text's colour.
    pub fn underline_color(&self) -> Color {
        self.buffer.cell_underline_color(&self.raw())
    }

    /// The hyperlink it belongs to.
    pub fn link(&self) -> Option<&'a str> {
        let cell = self.buffer.cells.get(self.index)?;
        self.buffer.cell_link(cell)
    }

    /// Everything above as a style, with every field set.
    pub fn style(&self) -> Style {
        let cell = self.raw();
        Style {
            fg: Some(cell.fg()),
            bg: Some(cell.bg()),
            underline_color: Some(self.underline_color()),
            underline_style: cell.underline_style(),
            add_modifier: cell.modifier(),
            sub_modifier: Modifier::empty(),
        }
    }

    /// The cells its text takes: 1 or more, or 0 for a cell that a wide character covers.
    pub fn width(&self) -> u16 {
        if self.raw().is_continuation() {
            return 0;
        }
        let width = usize::from(self.buffer.area.width).max(1);
        let row_end = (self.index / width + 1) * width;
        let following = self
            .buffer
            .cells
            .get(self.index + 1..row_end)
            .unwrap_or(&[])
            .iter()
            .take_while(|c| c.is_continuation())
            .count();
        u16::try_from(following + 1).unwrap_or(u16::MAX)
    }

    /// Whether terminals may disagree on its width, as they do for emoji sequences and East Asian
    /// ambiguous characters.
    pub fn has_uncertain_width(&self) -> bool {
        self.raw().uncertain_width()
    }
}

impl fmt::Debug for CellRef<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CellRef")
            .field("symbol", &self.symbol())
            .field("style", &self.style())
            .field("link", &self.link())
            .finish()
    }
}

/// A cell of a [`Buffer`], to change, from [`Buffer::cell_mut`].
///
/// Each change keeps the buffer's wide-character invariant. Writing a wide character also writes
/// its continuation cells, and writing over part of one turns the rest of it into spaces.
pub struct CellMut<'a> {
    buffer: &'a mut Buffer,
    index: usize,
    y: u16,
}

impl CellMut<'_> {
    /// Sets the text to the first grapheme cluster of `symbol`, keeping the style.
    ///
    /// A wide cluster takes the cells after this one too. A cluster that has no width, holds a
    /// control character, or is too wide to fit before the right edge becomes a space.
    pub fn set_symbol(&mut self, symbol: &str) -> &mut Self {
        self.buffer.set_cell_symbol(self.index, self.y, symbol);
        self
    }

    /// Sets the text to `c`, as [`set_symbol`](Self::set_symbol) does.
    pub fn set_char(&mut self, c: char) -> &mut Self {
        let mut bytes = [0; 4];
        self.set_symbol(c.encode_utf8(&mut bytes))
    }

    /// Merges `style` over the cell's, as [`Buffer::set_style`] does.
    pub fn set_style<S: Into<Style>>(&mut self, style: S) -> &mut Self {
        let style = style.into();
        self.buffer.patch_cell(self.index, style);
        if style.underline_color.is_some_and(|c| c != Color::Reset) {
            self.buffer.mark_rich(self.y);
        }
        self
    }

    /// Sets the text's colour.
    pub fn set_fg(&mut self, color: Color) -> &mut Self {
        self.set_style(Style::new().fg(color))
    }

    /// Sets the background colour.
    pub fn set_bg(&mut self, color: Color) -> &mut Self {
        self.set_style(Style::new().bg(color))
    }

    /// Makes the cell part of a hyperlink to `url`, or of none with `None`.
    pub fn set_link(&mut self, url: Option<&str>) -> &mut Self {
        let link = self.buffer.link_ref(url);
        self.buffer.set_cell_link(self.index, link);
        if link != 0 {
            self.buffer.mark_rich(self.y);
        }
        self
    }

    /// Makes the cell a space in the default colours.
    pub fn reset(&mut self) -> &mut Self {
        self.buffer.reset_cell(self.index);
        self
    }

    /// The cell's text.
    pub fn symbol(&self) -> &str {
        self.buffer.symbol_at(self.index)
    }

    /// The cell's style, as [`CellRef::style`] gives it.
    pub fn style(&self) -> Style {
        CellRef {
            buffer: self.buffer,
            index: self.index,
        }
        .style()
    }
}

impl fmt::Debug for CellMut<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CellMut")
            .field("symbol", &self.symbol())
            .field("style", &self.style())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(buffer: &Buffer, y: u16) -> String {
        let area = buffer.area();
        (area.x..area.right())
            .map(|x| buffer.symbol((x, y)))
            .collect()
    }

    #[test]
    fn a_cell_is_sixteen_bytes() {
        assert_eq!(size_of::<Cell>(), 16);
        assert_eq!(size_of::<PackedColor>(), 4);
    }

    #[test]
    fn colours_pack_and_unpack() {
        let mut colours = vec![Color::Reset];
        colours.extend((0..16).map(named_color));
        colours.extend((0..=255).map(Color::Indexed));
        colours.extend([
            Color::Rgb(0, 0, 0),
            Color::Rgb(1, 2, 3),
            Color::Rgb(255, 255, 255),
        ]);
        for c in colours {
            assert_eq!(PackedColor::pack(c).unpack(), c);
        }
        assert_eq!(PackedColor::pack(Color::Reset), PackedColor::RESET);
        assert_ne!(
            PackedColor::pack(Color::Red),
            PackedColor::pack(Color::Indexed(1))
        );
    }

    #[test]
    fn attrs_hold_modifiers_underline_and_the_uncertain_flag() {
        let mut attrs = Attrs::EMPTY;
        attrs.set_modifier(Modifier::all());
        attrs.set_underline_style(Some(UnderlineStyle::Dashed));
        attrs.set_uncertain(true);
        assert_eq!(attrs.modifier(), Modifier::all());
        assert_eq!(attrs.underline_style(), Some(UnderlineStyle::Dashed));
        assert!(attrs.uncertain());
        attrs.set_modifier(Modifier::BOLD);
        attrs.set_underline_style(None);
        assert_eq!(attrs.modifier(), Modifier::BOLD);
        assert_eq!(attrs.underline_style(), None);
        assert!(attrs.uncertain());
        attrs.set_uncertain(false);
        assert!(!attrs.uncertain());
        for shape in [
            UnderlineStyle::Single,
            UnderlineStyle::Double,
            UnderlineStyle::Curly,
            UnderlineStyle::Dotted,
            UnderlineStyle::Dashed,
        ] {
            attrs.set_underline_style(Some(shape));
            assert_eq!(attrs.underline_style(), Some(shape));
        }
    }

    #[test]
    fn empty_is_blank_and_positions_are_absolute() {
        let buffer = Buffer::empty(Rect::new(10, 5, 3, 2));
        assert_eq!(buffer.area(), Rect::new(10, 5, 3, 2));
        assert_eq!(row(&buffer, 5), "   ");
        let cell = buffer.cell((10, 5)).unwrap();
        assert_eq!(
            cell.style(),
            Style::new()
                .fg(Color::Reset)
                .bg(Color::Reset)
                .underline_color(Color::Reset)
        );
        assert_eq!(cell.width(), 1);
        assert!(buffer.cell((9, 5)).is_none());
        assert!(buffer.cell((13, 5)).is_none());
        assert!(buffer.cell((10, 7)).is_none());
        assert_eq!(buffer.symbol((0, 0)), "");
    }

    #[test]
    fn out_of_range_writes_are_dropped() {
        let mut buffer = Buffer::empty(Rect::new(2, 2, 4, 2));
        let before = buffer.clone();
        assert_eq!(buffer.set_stringn(0, 2, "abc", 10, Style::new()), (0, 2));
        assert_eq!(buffer.set_stringn(2, 9, "abc", 10, Style::new()), (2, 9));
        assert_eq!(buffer.set_stringn(6, 2, "abc", 10, Style::new()), (6, 2));
        buffer.set_style(Rect::new(0, 0, 2, 2), Style::new().bg(Color::Red));
        assert!(buffer.cell_mut((6, 3)).is_none());
        assert_eq!(buffer, before);
        let mut empty = Buffer::empty(Rect::ZERO);
        empty.set_string(0, 0, "a", Style::new());
        assert!(empty.cell((0, 0)).is_none());
    }

    #[test]
    fn set_string_clips_at_the_right_edge_and_never_wraps() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 2));
        buffer.set_string(1, 0, "abcdef", Style::new());
        assert_eq!(row(&buffer, 0), " abc");
        assert_eq!(row(&buffer, 1), "    ");
    }

    #[test]
    fn set_stringn_stops_at_max_width_and_returns_where_it_stopped() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 1));
        assert_eq!(buffer.set_stringn(1, 0, "abcdef", 3, Style::new()), (4, 0));
        assert_eq!(row(&buffer, 0), " abc    ");
        assert_eq!(buffer.set_stringn(0, 0, "界界界", 5, Style::new()), (4, 0));
        assert_eq!(row(&buffer, 0), "界界    ");
    }

    #[test]
    fn controls_and_zero_width_clusters_are_skipped() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 1));
        let end = buffer.set_stringn(0, 0, "a\tb\u{1b}[c\u{200b}d\u{7}\r\n", 8, Style::new());
        assert_eq!(row(&buffer, 0), "ab[cd   ");
        assert_eq!(end, (5, 0));
        buffer.set_string(0, 0, "\u{301}x界\u{9b}y", Style::new());
        assert_eq!(row(&buffer, 0), "x界yd   ");
    }

    #[test]
    fn combining_marks_stay_with_their_base() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        buffer.set_string(0, 0, "e\u{301}a\u{308}\u{301}", Style::new());
        assert_eq!(buffer.symbol((0, 0)), "e\u{301}");
        assert_eq!(buffer.symbol((1, 0)), "a\u{308}\u{301}");
        assert_eq!(buffer.symbol((2, 0)), " ");
    }

    #[test]
    fn a_wide_cluster_that_does_not_fit_stops_the_string() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 5, 1));
        buffer.set_string(0, 0, "ab界cd", Style::new());
        assert_eq!(row(&buffer, 0), "ab界c");
        buffer.reset();
        buffer.set_string(0, 0, "abcd界e", Style::new());
        assert_eq!(row(&buffer, 0), "abcd ");
        assert_eq!(buffer.cell((4, 0)).unwrap().width(), 1);
    }

    #[test]
    fn set_line_patches_span_styles_over_the_line_style_and_the_cell() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 1));
        buffer.set_style(buffer.area(), Style::new().bg(Color::Blue).fg(Color::White));
        let line = Line::from(vec![
            Span::raw("ab"),
            Span::styled(
                "cd",
                Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
            ),
        ])
        .style(Style::new().add_modifier(Modifier::ITALIC));
        assert_eq!(buffer.set_line(0, 0, &line, 6), (4, 0));
        let a = buffer.cell((0, 0)).unwrap();
        assert_eq!(
            (a.fg(), a.bg(), a.modifier()),
            (Color::White, Color::Blue, Modifier::ITALIC)
        );
        let c = buffer.cell((2, 0)).unwrap();
        assert_eq!(
            (c.fg(), c.bg(), c.modifier()),
            (Color::Red, Color::Blue, Modifier::ITALIC | Modifier::BOLD)
        );
        let after = buffer.cell((4, 0)).unwrap();
        assert_eq!(
            (after.fg(), after.modifier()),
            (Color::White, Modifier::empty())
        );
    }

    #[test]
    fn set_line_clips_each_span_and_moves_on_after_a_wide_cluster_that_does_not_fit() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 1));
        let line = Line::from(vec![Span::raw("abc"), Span::raw("界"), Span::raw("xyz")]);
        assert_eq!(buffer.set_line(0, 0, &line, 4), (4, 0));
        assert_eq!(row(&buffer, 0), "abcx      ");
        assert_eq!(buffer.set_line(0, 0, &Line::from("long text"), 0), (0, 0));
        let mut narrow = Buffer::empty(Rect::new(0, 0, 3, 1));
        assert_eq!(narrow.set_line(0, 0, &line, 10), (3, 0));
        assert_eq!(row(&narrow, 0), "abc");
    }

    #[test]
    fn set_span_writes_in_the_span_style() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        let span = Span::styled("hey!", Style::new().fg(Color::Green));
        assert_eq!(buffer.set_span(1, 0, &span, 2), (3, 0));
        assert_eq!(row(&buffer, 0), " he ");
        assert_eq!(buffer.cell((2, 0)).unwrap().fg(), Color::Green);
    }

    #[test]
    fn set_style_merges_and_keeps_symbols() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 1));
        buffer.set_string(
            0,
            0,
            "abc",
            Style::new()
                .fg(Color::Red)
                .bg(Color::Black)
                .add_modifier(Modifier::DIM | Modifier::BOLD),
        );
        buffer.set_style(
            Rect::new(1, 0, 5, 5),
            Style::new()
                .bg(Color::Indexed(17))
                .remove_modifier(Modifier::DIM)
                .add_modifier(Modifier::ITALIC),
        );
        assert_eq!(row(&buffer, 0), "abc");
        let a = buffer.cell((0, 0)).unwrap();
        assert_eq!(
            (a.fg(), a.bg(), a.modifier()),
            (Color::Red, Color::Black, Modifier::DIM | Modifier::BOLD)
        );
        let b = buffer.cell((1, 0)).unwrap();
        assert_eq!(
            (b.fg(), b.bg(), b.modifier()),
            (
                Color::Red,
                Color::Indexed(17),
                Modifier::BOLD | Modifier::ITALIC
            )
        );
    }

    #[test]
    fn cell_mut_writes_and_patches_one_cell() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 1));
        buffer.set_style(buffer.area(), Style::new().bg(Color::Blue));
        buffer
            .cell_mut((1, 0))
            .unwrap()
            .set_symbol("⠋")
            .set_style(Style::new().fg(Color::Yellow));
        let cell = buffer.cell((1, 0)).unwrap();
        assert_eq!(
            (cell.symbol(), cell.fg(), cell.bg()),
            ("⠋", Color::Yellow, Color::Blue)
        );
        let mut cell = buffer.cell_mut((2, 0)).unwrap();
        cell.set_char('x').set_fg(Color::Red).set_bg(Color::Green);
        assert_eq!(cell.symbol(), "x");
        assert_eq!(cell.style().bg, Some(Color::Green));
        cell.reset();
        assert_eq!(
            buffer.cell((2, 0)).unwrap().style(),
            Buffer::empty(Rect::new(0, 0, 1, 1))
                .cell((0, 0))
                .unwrap()
                .style()
        );
        buffer.cell_mut((0, 0)).unwrap().set_symbol("");
        assert_eq!(buffer.symbol((0, 0)), " ");
        buffer.cell_mut((0, 0)).unwrap().set_symbol("\t");
        assert_eq!(buffer.symbol((0, 0)), " ");
        buffer.cell_mut((0, 0)).unwrap().set_symbol("ab");
        assert_eq!(buffer.symbol((0, 0)), "a");
    }

    #[test]
    fn a_wide_symbol_takes_its_continuation_cells_or_becomes_a_space_at_the_edge() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 1));
        buffer.cell_mut((1, 0)).unwrap().set_symbol("界");
        assert_eq!(row(&buffer, 0), " 界");
        assert_eq!(buffer.cell((1, 0)).unwrap().width(), 2);
        assert_eq!(buffer.cell((2, 0)).unwrap().width(), 0);
        buffer.cell_mut((2, 0)).unwrap().set_symbol("界");
        assert_eq!(row(&buffer, 0), "   ");
        assert!((0..3).all(|x| buffer.cell((x, 0)).unwrap().width() == 1));
    }

    #[test]
    fn writing_over_part_of_a_wide_character_blanks_the_rest_and_keeps_its_style() {
        let style = Style::new().bg(Color::Magenta);
        let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 1));
        buffer.set_string(0, 0, "界界界", style);
        buffer.set_string(1, 0, "a", Style::new());
        assert_eq!(row(&buffer, 0), " a界界");
        assert_eq!(buffer.cell((0, 0)).unwrap().bg(), Color::Magenta);
        buffer.set_string(3, 0, "b", Style::new());
        assert_eq!(row(&buffer, 0), " a b界");
        buffer.cell_mut((5, 0)).unwrap().reset();
        assert_eq!(row(&buffer, 0), " a b  ");
        buffer.set_string(0, 0, "界界界", style);
        buffer.set_string(1, 0, "界界", style);
        assert_eq!(row(&buffer, 0), " 界界 ");
    }

    #[test]
    fn continuation_cells_take_the_style_of_their_character() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 2, 1));
        buffer.set_string(0, 0, "界", Style::new().fg(Color::Red).bg(Color::Blue));
        let cont = buffer.cell((1, 0)).unwrap();
        assert_eq!(
            (cont.symbol(), cont.fg(), cont.bg()),
            ("", Color::Red, Color::Blue)
        );
    }

    #[test]
    fn long_clusters_go_in_the_table_and_short_ones_stay_inline() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 1));
        let family = "👨\u{200d}👩\u{200d}👧";
        buffer.set_string(0, 0, family, Style::new());
        buffer.set_string(6, 0, family, Style::new());
        buffer.set_string(12, 0, "e\u{301}", Style::new());
        assert_eq!(buffer.symbol((0, 0)), family);
        assert_eq!(buffer.symbol((6, 0)), family);
        assert_eq!(buffer.cell((6, 0)).unwrap().width(), 6);
        assert!(buffer.cell((0, 0)).unwrap().has_uncertain_width());
        assert_eq!(buffer.clusters.len_for_tests(), 1);
        assert!(buffer.cells[12].cluster_index().is_none());
        assert_eq!(buffer.symbol((12, 0)), "e\u{301}");
        assert!(buffer.rich_rows[0]);
    }

    #[test]
    fn the_profile_decides_widths_when_text_is_written() {
        let joining = WidthProfile {
            joins_emoji: true,
            ..WidthProfile::default()
        };
        let family = "👨\u{200d}👩\u{200d}👧";
        let mut buffer = Buffer::with_width_profile(Rect::new(0, 0, 8, 1), joining);
        assert_eq!(buffer.width_profile(), joining);
        buffer.set_string(0, 0, family, Style::new());
        assert_eq!(buffer.cell((0, 0)).unwrap().width(), 2);
        buffer.set_width_profile(WidthProfile::default());
        buffer.set_string(2, 0, family, Style::new());
        assert_eq!(buffer.cell((0, 0)).unwrap().width(), 2);
        assert_eq!(buffer.cell((2, 0)).unwrap().width(), 6);
    }

    #[test]
    fn underline_colours_and_links_live_in_the_extras() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 2));
        buffer.set_string(
            0,
            0,
            "ab",
            Style::new()
                .add_modifier(Modifier::UNDERLINED)
                .underline_style(UnderlineStyle::Curly)
                .underline_color(Color::Rgb(9, 8, 7)),
        );
        buffer.set_link(Rect::new(1, 0, 2, 2), Some("https://example.com/a"));
        let a = buffer.cell((0, 0)).unwrap();
        assert_eq!(a.underline_color(), Color::Rgb(9, 8, 7));
        assert_eq!(a.underline_style(), Some(UnderlineStyle::Curly));
        assert_eq!(a.link(), None);
        let b = buffer.cell((1, 0)).unwrap();
        assert_eq!(
            (b.underline_color(), b.link()),
            (Color::Rgb(9, 8, 7), Some("https://example.com/a"))
        );
        assert_eq!(
            buffer.cell((2, 1)).unwrap().link(),
            Some("https://example.com/a")
        );
        assert_eq!(buffer.extras.entries.len(), 3);
        assert_eq!(buffer.extras.links.len_for_tests(), 1);
        buffer.cell_mut((2, 1)).unwrap().set_link(None);
        assert_eq!(buffer.cell((2, 1)).unwrap().link(), None);
        assert_eq!(
            buffer
                .cell_mut((2, 1))
                .unwrap()
                .set_link(Some("x"))
                .style()
                .underline_color,
            Some(Color::Reset)
        );
        assert!(buffer.rich_rows[0] && buffer.rich_rows[1]);
    }

    #[test]
    fn reset_blanks_every_cell_and_keeps_the_tables_memory() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 40, 4));
        for y in 0..4 {
            buffer.set_string(
                0,
                y,
                "👨\u{200d}👩\u{200d}👧 🇳🇿 e\u{301}",
                Style::new().underline_color(Color::Red),
            );
            buffer.set_link(Rect::new(0, y, 3, 1), Some("https://example.com"));
        }
        let capacities = |b: &Buffer| {
            (
                b.cells.capacity(),
                b.clusters.text.capacity(),
                b.clusters.ends.capacity(),
                b.extras.entries.capacity(),
                b.extras.links.text.capacity(),
            )
        };
        let before = capacities(&buffer);
        buffer.reset();
        assert_eq!(buffer, Buffer::empty(Rect::new(0, 0, 40, 4)));
        assert_eq!(capacities(&buffer), before);
        assert_eq!(buffer.clusters.len_for_tests(), 0);
        assert!(!buffer.rich_rows[0]);
    }

    #[test]
    fn resize_covers_the_new_area_with_blank_cells() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
        buffer.set_string(0, 0, "abcd", Style::new());
        buffer.resize(Rect::new(1, 1, 2, 3));
        assert_eq!(buffer, Buffer::empty(Rect::new(1, 1, 2, 3)));
        buffer.resize(Rect::new(u16::MAX - 1, 0, 10, 1));
        assert_eq!(buffer.area(), Rect::new(u16::MAX - 1, 0, 1, 1));
    }

    #[test]
    fn equality_compares_what_cells_show_not_table_indexes() {
        let family = "👨\u{200d}👩\u{200d}👧";
        let flag = "🇳🇿";
        let mut a = Buffer::empty(Rect::new(0, 0, 8, 1));
        a.set_string(0, 0, flag, Style::new());
        a.set_string(2, 0, family, Style::new());
        let mut b = Buffer::empty(Rect::new(0, 0, 8, 1));
        b.set_string(2, 0, family, Style::new());
        b.set_string(0, 0, flag, Style::new());
        assert_ne!(a.cells, b.cells);
        assert_eq!(a, b);
        b.set_link(Rect::new(0, 0, 1, 1), Some("u"));
        assert_ne!(a, b);
        a.set_link(Rect::new(0, 0, 1, 1), Some("u"));
        assert_eq!(a, b);
        a.set_style(Rect::new(7, 0, 1, 1), Style::new().fg(Color::Red));
        assert_ne!(a, b);
    }

    #[test]
    fn a_clip_drops_writes_outside_its_regions() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 12, 2));
        buffer.set_string(0, 0, "............", Style::new());
        buffer.set_clip(&[Rect::new(0, 0, 3, 1), Rect::new(6, 0, 3, 2)]);
        assert_eq!(
            buffer.set_stringn(0, 0, "abcdefghijkl", 12, Style::new()),
            (12, 0)
        );
        assert_eq!(row(&buffer, 0), "abc...ghi...");
        buffer.set_style(Rect::new(0, 0, 12, 2), Style::new().bg(Color::Red));
        let reds: Vec<_> = (0..12)
            .filter(|&x| buffer.cell((x, 1)).unwrap().bg() == Color::Red)
            .collect();
        assert_eq!(reds, [6, 7, 8]);
        buffer.set_link(Rect::new(0, 0, 12, 1), Some("u"));
        assert_eq!(buffer.cell((3, 0)).unwrap().link(), None);
        assert_eq!(buffer.cell((2, 0)).unwrap().link(), Some("u"));
        buffer
            .cell_mut((4, 0))
            .unwrap()
            .set_symbol("x")
            .set_fg(Color::Blue)
            .reset();
        assert_eq!(buffer.symbol((4, 0)), ".");
        assert_eq!(buffer.cell((4, 0)).unwrap().fg(), Color::Reset);
        buffer.clear_clip();
        buffer.set_string(3, 0, "XYZ", Style::new());
        assert_eq!(row(&buffer, 0), "abcXYZghi...");
    }

    #[test]
    fn a_clip_writes_a_wide_character_across_its_edge_whole() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 8, 1));
        buffer.set_string(0, 0, "a界b界cd", Style::new());
        buffer.set_clip(&[Rect::new(2, 0, 3, 1)]);
        buffer.set_string(2, 0, "x", Style::new());
        assert_eq!(
            row(&buffer, 0),
            "a xb界cd",
            "the 界 that starts outside is broken"
        );
        buffer.set_string(3, 0, "yz", Style::new());
        assert_eq!(
            row(&buffer, 0),
            "a xyz cd",
            "and so is the one that ends outside"
        );
        buffer.set_string(4, 0, "界", Style::new());
        assert_eq!(
            row(&buffer, 0),
            "a xy界cd",
            "one that starts inside is written whole"
        );
        buffer.set_string(5, 0, "w", Style::new());
        assert_eq!(
            row(&buffer, 0),
            "a xy界cd",
            "a write that starts outside is dropped"
        );
        let mut wide = Buffer::empty(Rect::new(0, 0, 6, 1));
        wide.set_clip(&[Rect::new(0, 0, 3, 1)]);
        wide.set_string(0, 0, "界界界", Style::new());
        assert_eq!(row(&wide, 0), "界界  ");
    }

    #[test]
    fn reset_under_a_clip_blanks_only_the_clip() {
        let mut buffer = Buffer::empty(Rect::new(2, 1, 6, 2));
        buffer.set_string(2, 1, "abcdef", Style::new().fg(Color::Red));
        buffer.set_string(2, 2, "👨\u{200d}👩\u{200d}👧", Style::new());
        buffer.set_clip(&[Rect::new(3, 1, 2, 2), Rect::new(100, 100, 5, 5)]);
        buffer.reset();
        assert_eq!(row(&buffer, 1), "a  def");
        assert_eq!(
            buffer.symbol((2, 2)),
            " ",
            "the family starts outside the clip and is broken by a reset inside it"
        );
        assert_eq!(buffer.cell((3, 1)).unwrap().fg(), Color::Reset);
        buffer.resize(Rect::new(0, 0, 3, 1));
        buffer.set_string(0, 0, "abc", Style::new());
        assert_eq!(row(&buffer, 0), "abc", "resizing ends the clip");
    }

    #[test]
    fn debug_shows_rows_as_text() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 2));
        buffer.set_string(0, 1, "a界", Style::new());
        let debug = format!("{buffer:?}");
        assert!(debug.contains(r#"rows: ["   ", "a界"]"#), "{debug}");
    }

    mod clip {
        use proptest::prelude::*;

        use super::*;

        const PIECES: &[&str] = &[
            "a",
            "bc",
            "界",
            "e\u{301}",
            "👍🏽",
            "👨\u{200d}👩\u{200d}👧",
            "\t",
            " ",
        ];

        fn text() -> impl Strategy<Value = String> {
            prop::collection::vec(prop::sample::select(PIECES), 0..6).prop_map(|p| p.concat())
        }

        fn rect() -> impl Strategy<Value = Rect> {
            (0..10u16, 0..4u16, 0..10u16, 0..4u16).prop_map(|(x, y, w, h)| Rect::new(x, y, w, h))
        }

        #[derive(Debug, Clone)]
        enum Op {
            Text(u16, u16, String),
            Symbol(u16, u16, &'static str),
            Style(Rect),
            Link(Rect),
            Reset(u16, u16),
            ResetAll,
        }

        fn op() -> impl Strategy<Value = Op> {
            prop_oneof![
                (0..10u16, 0..4u16, text()).prop_map(|(x, y, t)| Op::Text(x, y, t)),
                (0..10u16, 0..4u16, prop::sample::select(PIECES))
                    .prop_map(|(x, y, s)| Op::Symbol(x, y, s)),
                rect().prop_map(Op::Style),
                rect().prop_map(Op::Link),
                (0..10u16, 0..4u16).prop_map(|(x, y)| Op::Reset(x, y)),
                Just(Op::ResetAll),
            ]
        }

        /// The cells of row `y` the clip reaches, its own and those of the wide characters, in
        /// either buffer, that cross into them, and so on.
        fn reach(
            before: &Buffer,
            after: &Buffer,
            y: u16,
            inside: &impl Fn(u16, u16) -> bool,
        ) -> Vec<bool> {
            let width = before.area().width;
            let mut reach: Vec<bool> = (0..width).map(|x| inside(x, y)).collect();
            loop {
                let mut grew = false;
                for buffer in [before, after] {
                    let mut x = 0;
                    while x < width {
                        let w = buffer.cell((x, y)).map_or(1, |c| c.width().max(1));
                        let unit = usize::from(x)..usize::from(x + w).min(usize::from(width));
                        let cells = &mut reach[unit];
                        if cells.iter().any(|&r| r) && !cells.iter().all(|&r| r) {
                            cells.fill(true);
                            grew = true;
                        }
                        x += w;
                    }
                }
                if !grew {
                    return reach;
                }
            }
        }

        proptest! {
            /// Under a clip, a write changes a cell outside it only through a wide character that
            /// crosses its edge, before or after the write, and wide characters stay whole.
            #[test]
            fn writes_under_a_clip_change_only_the_clip(
                setup in prop::collection::vec((0..4u16, text()), 0..4),
                regions in prop::collection::vec(rect(), 0..3),
                ops in prop::collection::vec(op(), 1..20),
            ) {
                let area = Rect::new(0, 0, 9, 3);
                let mut buffer = Buffer::empty(area);
                for (y, t) in &setup {
                    buffer.set_string(0, *y, t, Style::new().fg(Color::Green));
                }
                let original = buffer.clone();
                buffer.set_clip(&regions);
                let inside = |x: u16, y: u16| regions.iter().any(|r| r.contains(Position::new(x, y)));
                for op in &ops {
                    let before = buffer.clone();
                    match op {
                        Op::Text(x, y, t) => {
                            buffer.set_string(*x, *y, t, Style::new().bg(Color::Blue));
                        }
                        Op::Symbol(x, y, s) => {
                            if let Some(mut cell) = buffer.cell_mut((*x, *y)) {
                                cell.set_symbol(s);
                            }
                        }
                        Op::Style(r) => buffer.set_style(*r, Style::new().add_modifier(Modifier::BOLD)),
                        Op::Link(r) => buffer.set_link(*r, Some("u")),
                        Op::Reset(x, y) => {
                            if let Some(mut cell) = buffer.cell_mut((*x, *y)) {
                                cell.reset();
                            }
                        }
                        Op::ResetAll => buffer.reset(),
                    }
                    for y in 0..area.height {
                        let mut x = 0;
                        while x < area.width {
                            let cell = buffer.cell((x, y)).unwrap();
                            prop_assert!(cell.width() > 0, "an orphan continuation at ({}, {}) after {:?}", x, y, op);
                            prop_assert_eq!(usize::from(cell.width()), text::width_with(cell.symbol(), WidthProfile::default()));
                            x += cell.width();
                        }
                        let reach = reach(&before, &buffer, y, &inside);
                        let spill = buffer.clip_spill(usize::from(y));
                        let spilled = |x: u16| spill.is_some_and(|(a, b)| a <= usize::from(x) && usize::from(x) < b);
                        for x in 0..area.width {
                            if !inside(x, y) && !spilled(x) {
                                let (a, b) = (buffer.cell((x, y)).unwrap(), original.cell((x, y)).unwrap());
                                prop_assert_eq!(a.symbol(), b.symbol(), "({}, {}) outside the clip and its spill after {:?}", x, y, op);
                                prop_assert_eq!(a.style(), b.style());
                            }
                            if !reach[usize::from(x)] && !spilled(x) {
                                let (a, b) = (buffer.cell((x, y)).unwrap(), before.cell((x, y)).unwrap());
                                prop_assert_eq!(a.symbol(), b.symbol(), "({}, {}) after {:?}", x, y, op);
                                prop_assert_eq!(a.style(), b.style(), "({}, {}) after {:?}", x, y, op);
                                prop_assert_eq!(a.link(), b.link());
                            }
                        }
                    }
                }
            }
        }
    }
}
