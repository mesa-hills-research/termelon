//! The frame model both stacks draw from. It knows nothing of ratatui, crossterm or termelon.
//!
//! A frame is a grid of cells. A cell holds one grapheme cluster and a resolved style, so a cell the
//! program never styled is the same as one styled with the terminal's defaults.

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The sixteen named colours, in ratatui's names.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Named {
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    Gray,
    DarkGray,
    LightRed,
    LightGreen,
    LightYellow,
    LightBlue,
    LightMagenta,
    LightCyan,
    White,
}

/// Each named colour with its name in the styled-text format.
pub(crate) const NAMED: [(&str, Named); 16] = [
    ("black", Named::Black),
    ("red", Named::Red),
    ("green", Named::Green),
    ("yellow", Named::Yellow),
    ("blue", Named::Blue),
    ("magenta", Named::Magenta),
    ("cyan", Named::Cyan),
    ("gray", Named::Gray),
    ("dark-gray", Named::DarkGray),
    ("light-red", Named::LightRed),
    ("light-green", Named::LightGreen),
    ("light-yellow", Named::LightYellow),
    ("light-blue", Named::LightBlue),
    ("light-magenta", Named::LightMagenta),
    ("light-cyan", Named::LightCyan),
    ("white", Named::White),
];

impl Named {
    /// The name in the styled-text format.
    pub(crate) fn name(self) -> &'static str {
        NAMED
            .iter()
            .find(|(_, named)| *named == self)
            .map_or("black", |(name, _)| name)
    }
}

/// A colour, as a cell holds it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) enum Color {
    /// The terminal's own.
    #[default]
    Default,
    Named(Named),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// How an underline is drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Underline {
    #[default]
    None,
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

/// A set of text attributes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct Mods(u16);

impl Mods {
    pub(crate) const NONE: Mods = Mods(0);
    pub(crate) const BOLD: Mods = Mods(1);
    pub(crate) const DIM: Mods = Mods(1 << 1);
    pub(crate) const ITALIC: Mods = Mods(1 << 2);
    pub(crate) const BLINK: Mods = Mods(1 << 3);
    pub(crate) const RAPID_BLINK: Mods = Mods(1 << 4);
    pub(crate) const REVERSE: Mods = Mods(1 << 5);
    pub(crate) const HIDDEN: Mods = Mods(1 << 6);
    pub(crate) const STRIKE: Mods = Mods(1 << 7);
    pub(crate) const OVERLINE: Mods = Mods(1 << 8);

    pub(crate) const fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }

    pub(crate) const fn with(self, other: Mods) -> Mods {
        Mods(self.0 | other.0)
    }
}

/// A cell's resolved style.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Style {
    pub(crate) fg: Color,
    pub(crate) bg: Color,
    /// The underline's colour.
    pub(crate) ul: Color,
    pub(crate) underline: Underline,
    pub(crate) mods: Mods,
    /// One more than the index of the link in the frame's table, or 0 for none.
    pub(crate) link: u32,
}

impl Style {
    pub(crate) const fn fg(mut self, color: Color) -> Style {
        self.fg = color;
        self
    }

    pub(crate) const fn bg(mut self, color: Color) -> Style {
        self.bg = color;
        self
    }

    pub(crate) const fn mods(self, mods: Mods) -> Style {
        Style {
            mods: self.mods.with(mods),
            ..self
        }
    }

    pub(crate) const fn bold(self) -> Style {
        self.mods(Mods::BOLD)
    }

    pub(crate) const fn italic(self) -> Style {
        self.mods(Mods::ITALIC)
    }
}

/// What a cell shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Symbol {
    /// One scalar value, which most cells hold.
    Char(char),
    /// A grapheme cluster of several scalar values, an index into the frame's table.
    Cluster(u32),
    /// The second and later cells of a wide character, which show nothing of their own.
    Continuation,
}

/// One cell of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cell {
    pub(crate) symbol: Symbol,
    pub(crate) style: Style,
}

impl Cell {
    /// Whether the cell leads a wide character, with `next` the cell after it.
    fn is_leader(&self, next: Option<&Cell>) -> bool {
        next.is_some_and(|next| next.symbol == Symbol::Continuation)
            && self.symbol != Symbol::Continuation
    }
}

/// The cursor's shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum CursorShape {
    #[default]
    Default,
    Block,
    Underline,
    Bar,
}

/// Where the cursor is and whether it shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Cursor {
    pub(crate) x: u16,
    pub(crate) y: u16,
    pub(crate) visible: bool,
    pub(crate) shape: CursorShape,
}

/// The width of a grapheme cluster in cells, measured as ratatui measures it.
pub(crate) fn width_of(cluster: &str) -> usize {
    cluster.width()
}

/// A screen: a grid of cells, the cursor and the tables its cells point into.
#[derive(Clone, Debug)]
pub(crate) struct Frame {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
    pub(crate) cursor: Cursor,
    clusters: Vec<Box<str>>,
    links: Vec<Box<str>>,
}

impl PartialEq for Frame {
    /// Frames are equal when they show the same text, styles and cursor, whatever order their tables
    /// were filled in.
    fn eq(&self, other: &Frame) -> bool {
        self.width == other.width
            && self.height == other.height
            && self.cursor == other.cursor
            && self.cells.iter().zip(&other.cells).all(|(a, b)| {
                self.same_symbol(a.symbol, other, b.symbol)
                    && Style { link: 0, ..a.style } == Style { link: 0, ..b.style }
                    && self.link_of(a.style) == other.link_of(b.style)
            })
    }
}

impl Frame {
    /// A frame of blank cells with the terminal's own colours and a hidden cursor.
    pub(crate) fn new(width: u16, height: u16) -> Frame {
        Frame::filled(width, height, Style::default())
    }

    /// A frame of spaces in `style`, as Arko paints its base colours over the whole screen first.
    pub(crate) fn filled(width: u16, height: u16, style: Style) -> Frame {
        let cell = Cell {
            symbol: Symbol::Char(' '),
            style,
        };
        Frame {
            width,
            height,
            cells: vec![cell; usize::from(width) * usize::from(height)],
            cursor: Cursor::default(),
            clusters: Vec::new(),
            links: Vec::new(),
        }
    }

    pub(crate) fn width(&self) -> u16 {
        self.width
    }

    pub(crate) fn height(&self) -> u16 {
        self.height
    }

    /// The cells, row by row.
    pub(crate) fn cells(&self) -> &[Cell] {
        &self.cells
    }

    fn index(&self, x: u16, y: u16) -> Option<usize> {
        (x < self.width && y < self.height)
            .then(|| usize::from(y) * usize::from(self.width) + usize::from(x))
    }

    /// The cell at `x`, `y`.
    pub(crate) fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        self.index(x, y).and_then(|at| self.cells.get(at))
    }

    /// The row `y`.
    pub(crate) fn row(&self, y: u16) -> &[Cell] {
        let width = usize::from(self.width);
        let start = usize::from(y) * width;
        self.cells.get(start..start + width).unwrap_or(&[])
    }

    /// The text of `symbol`, with `scratch` to hold a single scalar value.
    pub(crate) fn symbol<'a>(&'a self, symbol: Symbol, scratch: &'a mut [u8; 4]) -> &'a str {
        match symbol {
            Symbol::Char(c) => c.encode_utf8(scratch),
            Symbol::Cluster(at) => self
                .clusters
                .get(at as usize)
                .map_or("", |cluster| &**cluster),
            Symbol::Continuation => "",
        }
    }

    fn same_symbol(&self, a: Symbol, other: &Frame, b: Symbol) -> bool {
        let (mut left, mut right) = ([0; 4], [0; 4]);
        self.symbol(a, &mut left) == other.symbol(b, &mut right)
    }

    /// The URL of `style`'s link.
    pub(crate) fn link_of(&self, style: Style) -> Option<&str> {
        let at = (style.link as usize).checked_sub(1)?;
        self.links.get(at).map(|link| &**link)
    }

    /// The table index to put in a style for `url`.
    pub(crate) fn link(&mut self, url: &str) -> u32 {
        let at = self
            .links
            .iter()
            .position(|link| &**link == url)
            .unwrap_or_else(|| {
                self.links.push(url.into());
                self.links.len() - 1
            });
        u32::try_from(at + 1).unwrap_or(0)
    }

    fn symbol_of(&mut self, cluster: &str) -> Symbol {
        let mut scalars = cluster.chars();
        if let (Some(c), None) = (scalars.next(), scalars.next()) {
            return Symbol::Char(c);
        }
        let at = self
            .clusters
            .iter()
            .position(|known| &**known == cluster)
            .unwrap_or_else(|| {
                self.clusters.push(cluster.into());
                self.clusters.len() - 1
            });
        u32::try_from(at).map_or(Symbol::Char(' '), Symbol::Cluster)
    }

    /// Blanks the other half of any wide character that writing `width` cells at `at` would cut.
    fn cut_wide(&mut self, at: usize, width: usize) {
        let columns = usize::from(self.width).max(1);
        let row_start = at - at % columns;
        if self.cells[at].symbol == Symbol::Continuation && at > row_start {
            self.cells[at - 1].symbol = Symbol::Char(' ');
        }
        let last = at + width - 1;
        if last + 1 < row_start + columns && self.cells[last].is_leader(Some(&self.cells[last + 1]))
        {
            self.cells[last + 1].symbol = Symbol::Char(' ');
        }
    }

    /// Writes `text` at `x`, `y` in `style`, stopping at column `limit` (exclusive), and returns the
    /// column after the last cell written. It follows ratatui's `set_stringn`. Clusters with no width
    /// are skipped and a wide cluster that doesn't fit ends the text.
    pub(crate) fn put_str(&mut self, x: u16, y: u16, text: &str, style: Style, limit: u16) -> u16 {
        let limit = limit.min(self.width);
        let mut col = x;
        if y >= self.height {
            return col;
        }
        for cluster in text.graphemes(true) {
            let width = width_of(cluster);
            let Ok(cells) = u16::try_from(width) else {
                break;
            };
            if cells == 0 {
                continue;
            }
            if col.saturating_add(cells) > limit {
                break;
            }
            let symbol = self.symbol_of(cluster);
            let Some(at) = self.index(col, y) else { break };
            self.cut_wide(at, width);
            if let Some(cell) = self.cells.get_mut(at) {
                *cell = Cell { symbol, style };
            }
            for rest in 1..width {
                if let Some(cell) = self.cells.get_mut(at + rest) {
                    *cell = Cell {
                        symbol: Symbol::Continuation,
                        style,
                    };
                }
            }
            col += cells;
        }
        col
    }

    /// Sets the style of the cells from column `from` up to `to` (exclusive) of row `y`, keeping
    /// their symbols.
    pub(crate) fn set_style(&mut self, y: u16, from: u16, to: u16, style: Style) {
        for x in from..to.min(self.width) {
            if let Some(at) = self.index(x, y)
                && let Some(cell) = self.cells.get_mut(at)
            {
                cell.style = style;
            }
        }
    }

    /// Replaces the cell at `x`, `y` with a single character. The cell must not be part of a wide
    /// character.
    pub(crate) fn set_char(&mut self, x: u16, y: u16, c: char, style: Style) {
        if let Some(at) = self.index(x, y) {
            self.cut_wide(at, 1);
            if let Some(cell) = self.cells.get_mut(at) {
                *cell = Cell {
                    symbol: Symbol::Char(c),
                    style,
                };
            }
        }
    }

    /// The text of row `y` as a terminal shows it, without trailing spaces.
    pub(crate) fn row_text(&self, y: u16) -> String {
        let mut text = String::new();
        let mut scratch = [0; 4];
        for cell in self.row(y) {
            text.push_str(self.symbol(cell.symbol, &mut scratch));
        }
        text.truncate(text.trim_end_matches(' ').len());
        text
    }

    /// Checks that every wide character has all its continuation cells and every continuation cell
    /// follows its character.
    #[cfg(test)]
    pub(crate) fn wide_characters_are_whole(&self) -> bool {
        (0..self.height).all(|y| {
            let row = self.row(y);
            let mut scratch = [0; 4];
            let mut at = 0;
            while let Some(cell) = row.get(at) {
                if cell.symbol == Symbol::Continuation {
                    return false;
                }
                let width = width_of(self.symbol(cell.symbol, &mut scratch)).max(1);
                let rest = row.get(at + 1..at + width);
                if rest.is_none_or(|rest| rest.iter().any(|c| c.symbol != Symbol::Continuation)) {
                    return false;
                }
                at += width;
            }
            true
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: Style = Style {
        fg: Color::Default,
        bg: Color::Default,
        ul: Color::Default,
        underline: Underline::None,
        mods: Mods(0),
        link: 0,
    };

    fn texts(frame: &Frame) -> Vec<String> {
        (0..frame.height()).map(|y| frame.row_text(y)).collect()
    }

    #[test]
    fn a_new_frame_is_blank() {
        let frame = Frame::new(3, 2);
        assert_eq!(frame.cells().len(), 6);
        let blank = Cell {
            symbol: Symbol::Char(' '),
            style: Style::default(),
        };
        assert!(frame.cells().iter().all(|cell| *cell == blank));
        assert_eq!(texts(&frame), ["", ""]);
    }

    #[test]
    fn text_is_clipped_at_the_limit() {
        let mut frame = Frame::new(6, 1);
        assert_eq!(frame.put_str(1, 0, "abcdefgh", PLAIN, 4), 4);
        assert_eq!(frame.row_text(0), " abc");
    }

    #[test]
    fn a_wide_character_takes_two_cells() {
        let mut frame = Frame::new(6, 1);
        assert_eq!(frame.put_str(0, 0, "a漢b", PLAIN, 6), 4);
        assert_eq!(frame.cell(1, 0).map(|c| c.symbol), Some(Symbol::Char('漢')));
        assert_eq!(
            frame.cell(2, 0).map(|c| c.symbol),
            Some(Symbol::Continuation)
        );
        assert_eq!(frame.row_text(0), "a漢b");
        assert!(frame.wide_characters_are_whole());
    }

    #[test]
    fn a_wide_character_that_does_not_fit_ends_the_text() {
        let mut frame = Frame::new(4, 1);
        assert_eq!(frame.put_str(0, 0, "ab漢c", PLAIN, 3), 2);
        assert_eq!(frame.row_text(0), "ab");
    }

    #[test]
    fn a_cluster_of_several_scalars_is_one_cell_group() {
        let mut frame = Frame::new(8, 1);
        // A thumbs up with a skin tone, and a letter with a combining accent.
        assert_eq!(frame.put_str(0, 0, "👍🏽e\u{301}x", PLAIN, 8), 4);
        assert!(matches!(
            frame.cell(0, 0).map(|c| c.symbol),
            Some(Symbol::Cluster(_))
        ));
        assert_eq!(
            frame.cell(1, 0).map(|c| c.symbol),
            Some(Symbol::Continuation)
        );
        assert!(matches!(
            frame.cell(2, 0).map(|c| c.symbol),
            Some(Symbol::Cluster(_))
        ));
        assert_eq!(frame.row_text(0), "👍🏽e\u{301}x");
    }

    #[test]
    fn writing_over_half_a_wide_character_blanks_the_other_half() {
        let mut frame = Frame::new(6, 1);
        frame.put_str(0, 0, "漢字", PLAIN, 6);
        frame.set_char(1, 0, 'x', PLAIN);
        assert_eq!(frame.row_text(0), " x字");
        assert!(frame.wide_characters_are_whole());
        frame.set_char(2, 0, 'y', PLAIN);
        assert_eq!(frame.row_text(0), " xy");
        assert!(frame.wide_characters_are_whole());
        frame.put_str(0, 0, "漢字", PLAIN, 6);
        frame.put_str(1, 0, "ab", PLAIN, 6);
        assert_eq!(frame.row_text(0), " ab");
        assert!(frame.wide_characters_are_whole());
    }

    #[test]
    fn zero_width_clusters_are_skipped() {
        let mut frame = Frame::new(4, 1);
        assert_eq!(frame.put_str(0, 0, "a\u{200b}b", PLAIN, 4), 2);
        assert_eq!(frame.row_text(0), "ab");
    }

    #[test]
    fn styles_apply_to_a_range_and_keep_the_symbols() {
        let mut frame = Frame::new(4, 1);
        frame.put_str(0, 0, "abcd", PLAIN, 4);
        let red = PLAIN.fg(Color::Named(Named::Red));
        frame.set_style(0, 1, 3, red);
        let styles: Vec<Style> = frame.row(0).iter().map(|cell| cell.style).collect();
        assert_eq!(styles, [PLAIN, red, red, PLAIN]);
        assert_eq!(frame.row_text(0), "abcd");
    }

    #[test]
    fn frames_with_tables_in_a_different_order_are_equal() {
        let mut a = Frame::new(6, 1);
        a.put_str(0, 0, "👍🏽", PLAIN, 6);
        a.put_str(2, 0, "e\u{301}", PLAIN, 6);
        let mut b = Frame::new(6, 1);
        b.put_str(2, 0, "e\u{301}", PLAIN, 6);
        b.put_str(0, 0, "👍🏽", PLAIN, 6);
        assert_eq!(a, b);
        b.put_str(4, 0, "x", PLAIN, 6);
        assert_ne!(a, b);
    }

    #[test]
    fn names_round_trip() {
        for (name, named) in NAMED {
            assert_eq!(named.name(), name);
        }
    }
}
