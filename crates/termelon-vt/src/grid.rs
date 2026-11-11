//! The grid of cells.

use std::fmt;

use crate::style::Style;

/// The text of a cell, inline for one scalar and on the heap for a longer cluster.
#[derive(Clone, PartialEq, Eq)]
enum Symbol {
    Inline { bytes: [u8; 4], len: u8 },
    Cluster(Box<str>),
}

impl Symbol {
    const EMPTY: Symbol = Symbol::Inline {
        bytes: [0; 4],
        len: 0,
    };
    const SPACE: Symbol = Symbol::Inline {
        bytes: [b' ', 0, 0, 0],
        len: 1,
    };

    fn new(text: &str) -> Symbol {
        let mut chars = text.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => {
                let mut bytes = [0; 4];
                let len = c.encode_utf8(&mut bytes).len();
                Symbol::Inline {
                    bytes,
                    len: u8::try_from(len).unwrap_or(4),
                }
            }
            _ => Symbol::Cluster(text.into()),
        }
    }

    fn as_str(&self) -> &str {
        match self {
            Symbol::Inline { bytes, len } => {
                std::str::from_utf8(&bytes[..usize::from(*len)]).unwrap_or_default()
            }
            Symbol::Cluster(text) => text,
        }
    }
}

/// One cell of the screen.
///
/// A wide cluster takes its first cell, which holds the text and the width, and continuation cells
/// after it, which hold no text and have width 0. A continuation cell carries its leader's style.
#[derive(Clone, PartialEq, Eq)]
pub struct Cell {
    symbol: Symbol,
    width: u8,
    style: Style,
}

impl Cell {
    pub(crate) fn blank(style: Style) -> Cell {
        Cell {
            symbol: Symbol::SPACE,
            width: 1,
            style,
        }
    }

    fn continuation(style: Style) -> Cell {
        Cell {
            symbol: Symbol::EMPTY,
            width: 0,
            style,
        }
    }

    /// The grapheme cluster, a space for a blank cell, and empty for a continuation cell.
    pub fn text(&self) -> &str {
        self.symbol.as_str()
    }

    /// The cells the cluster takes, and 0 for a continuation cell.
    pub fn width(&self) -> u8 {
        self.width
    }

    /// Whether this cell continues the wide cluster before it.
    pub fn is_continuation(&self) -> bool {
        self.width == 0
    }

    /// The colours and attributes.
    pub fn style(&self) -> &Style {
        &self.style
    }
}

impl fmt::Debug for Cell {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Cell")
            .field("text", &self.text())
            .field("width", &self.width)
            .field("style", &self.style)
            .finish()
    }
}

/// The cells of one screen, row by row. Every operation keeps one invariant: a wide cluster always
/// has all of its continuation cells, and no continuation cell is without its leader.
#[derive(Clone, Debug)]
pub(crate) struct Grid {
    cols: u16,
    rows: Vec<Vec<Cell>>,
}

impl Grid {
    pub(crate) fn new(cols: u16, rows: u16) -> Grid {
        let row = vec![Cell::blank(Style::default()); usize::from(cols)];
        Grid {
            cols,
            rows: vec![row; usize::from(rows)],
        }
    }

    pub(crate) fn cell(&self, x: u16, y: u16) -> Option<&Cell> {
        self.rows.get(usize::from(y))?.get(usize::from(x))
    }

    pub(crate) fn row(&self, y: u16) -> Option<&[Cell]> {
        self.rows.get(usize::from(y)).map(Vec::as_slice)
    }

    /// Writes a cluster of `width` cells at `x`. The caller makes sure it fits.
    ///
    /// Parts of wide clusters that stick out of the cells written become blanks, or with
    /// `keep_halves`, keep the cluster's text in a narrow cell, as a terminal that leaves the other
    /// half of a wide character on the screen shows it.
    pub(crate) fn put(
        &mut self,
        x: u16,
        y: u16,
        text: &str,
        width: u8,
        style: &Style,
        keep_halves: bool,
    ) {
        let end = x.saturating_add(u16::from(width)).min(self.cols);
        self.break_wide(y, x, end, keep_halves);
        let Some(row) = self.rows.get_mut(usize::from(y)) else {
            return;
        };
        let (x, end) = (usize::from(x), usize::from(end));
        if x >= end {
            return;
        }
        row[x] = Cell {
            symbol: Symbol::new(text),
            width,
            style: style.clone(),
        };
        for cell in &mut row[x + 1..end] {
            *cell = Cell::continuation(style.clone());
        }
    }

    /// Replaces the text of the cluster at `x`, keeping its width and style.
    pub(crate) fn set_text(&mut self, x: u16, y: u16, text: &str) {
        if let Some(cell) = self
            .rows
            .get_mut(usize::from(y))
            .and_then(|row| row.get_mut(usize::from(x)))
        {
            cell.symbol = Symbol::new(text);
        }
    }

    /// The first cell of the cluster that covers `x`.
    pub(crate) fn leader(&self, x: u16, y: u16) -> u16 {
        let Some(row) = self.rows.get(usize::from(y)) else {
            return x;
        };
        let mut x = usize::from(x).min(row.len().saturating_sub(1));
        while x > 0 && row[x].is_continuation() {
            x -= 1;
        }
        u16::try_from(x).unwrap_or(0)
    }

    /// Blanks the cells from `start` to `end` (exclusive) with `blank`.
    pub(crate) fn erase(&mut self, y: u16, start: u16, end: u16, blank: &Cell) {
        let end = end.min(self.cols);
        if start >= end {
            return;
        }
        self.break_wide(y, start, end, false);
        if let Some(row) = self.rows.get_mut(usize::from(y)) {
            for cell in &mut row[usize::from(start)..usize::from(end)] {
                cell.clone_from(blank);
            }
        }
    }

    /// Blanks whole rows from `start` to `end` (exclusive).
    pub(crate) fn erase_rows(&mut self, start: u16, end: u16, blank: &Cell) {
        for y in start..end.min(self.height()) {
            self.erase(y, 0, self.cols, blank);
        }
    }

    /// Moves the rows from `top` to `bottom` (inclusive) up by `n`, blanking those that come in.
    pub(crate) fn scroll_up(&mut self, top: u16, bottom: u16, n: u16, blank: &Cell) {
        let (top, bottom) = (
            usize::from(top),
            usize::from(bottom).min(self.rows.len().saturating_sub(1)),
        );
        if top > bottom {
            return;
        }
        let n = usize::from(n).min(bottom - top + 1);
        self.rows[top..=bottom].rotate_left(n);
        for row in &mut self.rows[bottom + 1 - n..=bottom] {
            row.fill(blank.clone());
        }
    }

    /// Moves the rows from `top` to `bottom` (inclusive) down by `n`, blanking those that come in.
    pub(crate) fn scroll_down(&mut self, top: u16, bottom: u16, n: u16, blank: &Cell) {
        let (top, bottom) = (
            usize::from(top),
            usize::from(bottom).min(self.rows.len().saturating_sub(1)),
        );
        if top > bottom {
            return;
        }
        let n = usize::from(n).min(bottom - top + 1);
        self.rows[top..=bottom].rotate_right(n);
        for row in &mut self.rows[top..top + n] {
            row.fill(blank.clone());
        }
    }

    pub(crate) fn height(&self) -> u16 {
        u16::try_from(self.rows.len()).unwrap_or(u16::MAX)
    }

    /// Changes the size, keeping the top-left of the content.
    pub(crate) fn resize(&mut self, cols: u16, rows: u16) {
        let blank = Cell::blank(Style::default());
        self.rows.resize(
            usize::from(rows),
            vec![blank.clone(); usize::from(self.cols)],
        );
        for row in &mut self.rows {
            row.resize(usize::from(cols), blank.clone());
            // A wide cluster cut by the new right edge becomes a blank.
            if let Some(last) = row.iter().rposition(|cell| !cell.is_continuation()) {
                let width = usize::from(row[last].width);
                if last + width > row.len() {
                    for cell in &mut row[last..] {
                        cell.symbol = Symbol::SPACE;
                        cell.width = 1;
                    }
                }
            }
        }
        self.cols = cols;
    }

    /// Before cells from `start` to `end` change, blanks the parts of wide clusters that stick out
    /// of that range, keeping their style. With `keep_halves`, the first cell of each part keeps the
    /// cluster's text instead.
    fn break_wide(&mut self, y: u16, start: u16, end: u16, keep_halves: bool) {
        let Some(row) = self.rows.get_mut(usize::from(y)) else {
            return;
        };
        let (start, end) = (usize::from(start), usize::from(end));
        let leader_of = |row: &[Cell], mut x: usize| {
            while x > 0 && row[x].is_continuation() {
                x -= 1;
            }
            x
        };
        let right = (end < row.len() && row[end].is_continuation())
            .then(|| row[leader_of(row, end)].symbol.clone());
        if start < row.len() && row[start].is_continuation() {
            let leader = leader_of(row, start);
            let text = row[leader].symbol.clone();
            for cell in &mut row[leader..start] {
                cell.symbol = Symbol::SPACE;
                cell.width = 1;
            }
            if keep_halves {
                row[leader].symbol = text;
            }
        }
        let mut x = end;
        while x < row.len() && row[x].is_continuation() {
            row[x].symbol = Symbol::SPACE;
            row[x].width = 1;
            x += 1;
        }
        if let (true, Some(text)) = (keep_halves && end < row.len(), right) {
            row[end].symbol = text;
        }
    }

    /// Checks the invariant, for tests.
    #[cfg(test)]
    pub(crate) fn check(&self) -> Result<(), String> {
        for (y, row) in self.rows.iter().enumerate() {
            let mut x = 0;
            while x < row.len() {
                let width = usize::from(row[x].width);
                if width == 0 {
                    return Err(format!("continuation without a leader at {x},{y}"));
                }
                if x + width > row.len() {
                    return Err(format!("cluster past the edge at {x},{y}"));
                }
                for (i, cell) in row[x + 1..x + width].iter().enumerate() {
                    if !cell.is_continuation() {
                        return Err(format!("missing continuation at {},{y}", x + 1 + i));
                    }
                }
                x += width;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(grid: &Grid, y: u16) -> String {
        grid.row(y).unwrap().iter().map(Cell::text).collect()
    }

    #[test]
    fn writing_over_half_a_wide_cluster_blanks_the_other_half() {
        let mut grid = Grid::new(6, 1);
        let style = Style::default();
        grid.put(0, 0, "字", 2, &style, false);
        grid.put(2, 0, "字", 2, &style, false);
        grid.put(1, 0, "a", 1, &style, false);
        assert_eq!(text(&grid, 0), " a字  ");
        grid.put(2, 0, "b", 1, &style, false);
        assert_eq!(text(&grid, 0), " ab   ");
        grid.check().unwrap();
    }

    #[test]
    fn writing_over_half_a_wide_cluster_can_leave_the_other_half() {
        let mut grid = Grid::new(6, 1);
        let style = Style::default();
        grid.put(0, 0, "字", 2, &style, true);
        grid.put(2, 0, "字", 2, &style, true);
        grid.put(1, 0, "a", 1, &style, true);
        assert_eq!(text(&grid, 0), "字a字  ");
        grid.put(2, 0, "b", 1, &style, true);
        assert_eq!(text(&grid, 0), "字ab字  ");
        grid.check().unwrap();
    }

    #[test]
    fn erasing_part_of_a_wide_cluster_blanks_all_of_it() {
        let mut grid = Grid::new(4, 1);
        let style = Style::default();
        grid.put(0, 0, "字", 2, &style, false);
        grid.put(2, 0, "字", 2, &style, false);
        grid.erase(0, 1, 3, &Cell::blank(Style::default()));
        assert_eq!(text(&grid, 0), "    ");
        grid.check().unwrap();
    }

    #[test]
    fn scrolling_a_region() {
        let mut grid = Grid::new(1, 4);
        let style = Style::default();
        for (y, c) in ["a", "b", "c", "d"].iter().enumerate() {
            grid.put(0, u16::try_from(y).unwrap(), c, 1, &style, false);
        }
        let blank = Cell::blank(Style::default());
        grid.scroll_up(1, 2, 1, &blank);
        assert_eq!((0..4).map(|y| text(&grid, y)).collect::<String>(), "ac d");
        grid.scroll_down(0, 3, 2, &blank);
        assert_eq!((0..4).map(|y| text(&grid, y)).collect::<String>(), "  ac");
    }

    #[test]
    fn resizing_cuts_a_wide_cluster_at_the_edge() {
        let mut grid = Grid::new(4, 1);
        grid.put(2, 0, "字", 2, &Style::default(), false);
        grid.resize(3, 2);
        assert_eq!(text(&grid, 0), "   ");
        grid.check().unwrap();
    }
}
