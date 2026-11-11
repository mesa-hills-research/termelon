//! Which cells a frame changes, as runs of cells to write.
//!
//! A row whose cells are equal in both frames, as a comparison of plain bytes, is skipped, unless
//! either holds a cluster or an extra, whose table indexes mean nothing across buffers. Otherwise
//! the row is compared a unit at a time, a wide character and its continuation cells being one
//! unit, and two changed runs merge when writing the unchanged cells between them costs no more
//! than moving the cursor over them.

use crate::buffer::{Buffer, Cell};

/// Cells of one row to write, by row and columns counted inside the buffer's area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Run {
    /// The row.
    pub(crate) row: usize,
    /// The first column.
    pub(crate) start: usize,
    /// The column after the last.
    pub(crate) end: usize,
}

/// What a row of the next frame is compared with.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Before<'a> {
    /// The row of that buffer, counted inside its area.
    Row(&'a Buffer, usize),
    /// A blank row, as a cleared screen or a scroll leaves.
    Blank,
}

/// The longest gap of unchanged cells worth writing instead of moving over.
const MAX_GAP: usize = 8;

/// Appends the changed cells of row `y` of `next` to `runs`, looking only at `columns`, sorted
/// spans of columns that don't overlap. Runs in two spans stay apart, so cells between them are
/// never written.
pub(crate) fn diff_row(
    before: Before<'_>,
    next: &Buffer,
    y: usize,
    columns: &[(usize, usize)],
    runs: &mut Vec<Run>,
) {
    let row = next.row(y);
    let prev = match before {
        Before::Row(prev, py) => {
            let prev_row = prev.row(py);
            if !prev.is_rich_row(py) && !next.is_rich_row(y) && Buffer::rows_equal(prev_row, row) {
                return;
            }
            Some((prev, prev_row))
        }
        Before::Blank => None,
    };
    for &(from, to) in columns {
        let first = runs.len();
        let to = to.min(row.len());
        let mut x = from.min(row.len());
        while x > 0 && row.get(x).is_some_and(Cell::is_continuation) {
            x -= 1;
        }
        while x < to {
            let width = unit_width(row, x);
            if unit_changed(prev, next, row, x, width) {
                push(runs, first, y, x, x + width);
            }
            x += width;
        }
        merge_gaps(next, row, runs, first);
    }
}

/// The cells of the unit that starts at `x`: 1, and 1 more for each continuation cell after it.
pub(crate) fn unit_width(row: &[Cell], x: usize) -> usize {
    1 + row
        .get(x + 1..)
        .unwrap_or(&[])
        .iter()
        .take_while(|c| c.is_continuation())
        .count()
}

/// Whether the unit at `x` of `width` cells shows something else than the previous frame there.
fn unit_changed(
    prev: Option<(&Buffer, &[Cell])>,
    next: &Buffer,
    row: &[Cell],
    x: usize,
    width: usize,
) -> bool {
    let Some(cell) = row.get(x) else {
        return false;
    };
    let Some((prev, prev_row)) = prev else {
        return width > 1 || !next.looks_blank(cell);
    };
    let Some(before) = prev_row.get(x) else {
        return true;
    };
    if before.is_continuation() || unit_width(prev_row, x) != width {
        return true;
    }
    !(prev.same_cell(before, next, cell) || (prev.looks_blank(before) && next.looks_blank(cell)))
}

/// Adds the cells `start..end` of row `y` to the runs from `first`, joining the last run when
/// they touch it.
fn push(runs: &mut Vec<Run>, first: usize, y: usize, start: usize, end: usize) {
    if runs.len() > first
        && let Some(last) = runs.last_mut()
        && last.end >= start
    {
        last.end = last.end.max(end);
        return;
    }
    runs.push(Run { row: y, start, end });
}

/// Joins the runs from `first`, all in one row, where writing the cells between two of them costs
/// no more than moving the cursor over them.
fn merge_gaps(next: &Buffer, row: &[Cell], runs: &mut Vec<Run>, first: usize) {
    let mut kept = first;
    for i in first..runs.len() {
        let run = runs[i];
        if i > first && gap_is_cheap(next, row, runs[kept].end, run.start) {
            runs[kept].end = run.end;
        } else {
            if i > first {
                kept += 1;
            }
            runs[kept] = run;
        }
    }
    runs.truncate(if runs.len() > first { kept + 1 } else { first });
}

/// Whether writing the unchanged cells `start..end` costs no more than `CSI n C` over them, with
/// no change of style from the cell before.
fn gap_is_cheap(next: &Buffer, row: &[Cell], start: usize, end: usize) -> bool {
    let gap = end.saturating_sub(start);
    if gap == 0 {
        return true;
    }
    if gap > MAX_GAP || start == 0 {
        return false;
    }
    let mut leader = start - 1;
    while leader > 0 && row[leader].is_continuation() {
        leader -= 1;
    }
    let Some(before) = row.get(leader) else {
        return false;
    };
    if before.uncertain_width() {
        return false;
    }
    let move_cost = if gap == 1 { 3 } else { 3 + digits(gap) };
    let mut bytes = 0;
    let mut x = start;
    while x < end {
        let Some(cell) = row.get(x) else {
            return false;
        };
        if cell.uncertain_width() || !Buffer::same_style(cell, before) {
            return false;
        }
        bytes += next.cell_symbol(cell).len();
        x += unit_width(row, x);
    }
    bytes <= move_cost
}

/// The number of decimal digits of `n`.
pub(crate) fn digits(n: usize) -> usize {
    let mut digits = 1;
    let mut n = n / 10;
    while n > 0 {
        digits += 1;
        n /= 10;
    }
    digits
}

/// A shift of whole rows that the terminal can do itself, with a scroll region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Scroll {
    /// The region's first row, counted inside the buffer's area.
    pub(crate) top: usize,
    /// The row after the region's last.
    pub(crate) bottom: usize,
    /// How many rows the content moves.
    pub(crate) by: usize,
    /// Whether it moves up (`CSI n S`) rather than down (`CSI n T`).
    pub(crate) up: bool,
}

impl Scroll {
    /// What row `y` of the next frame is compared with once the terminal has scrolled.
    pub(crate) fn before(self, prev: &Buffer, y: usize) -> Before<'_> {
        if y < self.top || y >= self.bottom {
            return Before::Row(prev, y);
        }
        let source = if self.up {
            Some(y + self.by).filter(|&s| s < self.bottom)
        } else {
            y.checked_sub(self.by).filter(|&s| s >= self.top)
        };
        source.map_or(Before::Blank, |s| Before::Row(prev, s))
    }
}

/// The bytes scrolling takes, for the region, the scroll and the region's reset.
const SCROLL_COST: usize = 16;

/// The rows a scroll region needs, since terminals ignore a region of one row.
const MIN_REGION: usize = 2;

/// Reused memory for [`detect_scroll`].
#[derive(Debug, Default)]
pub(crate) struct ScrollScratch {
    prev: Vec<u64>,
    next: Vec<u64>,
    /// Prefix sums over the rows of the cost of each row that changed, without scrolling.
    changed: Vec<usize>,
    /// Prefix sums over the rows of the cost of each row that isn't blank.
    drawn: Vec<usize>,
}

/// The scroll that saves the most bytes, if any does, when more than half of the rows changed.
///
/// Each row is hashed in both frames. The hashes only choose the scroll, since the rows are then
/// compared cell by cell, so a collision costs bytes and never misdraws. For each shift, a run of rows that match once shifted makes
/// a region, and the cost of the frame with that scroll is the scroll's bytes, the rows that
/// still differ outside the region, and the rows the scroll brings in blank. A row's cost is
/// estimated as its cells that aren't blank, plus a move.
pub(crate) fn detect_scroll(
    prev: &Buffer,
    next: &Buffer,
    scratch: &mut ScrollScratch,
) -> Option<Scroll> {
    let height = usize::from(next.area().height);
    if height < MIN_REGION + 1 || prev.area() != next.area() {
        return None;
    }
    let changed_rows = (0..height)
        .filter(|&y| {
            prev.is_rich_row(y)
                || next.is_rich_row(y)
                || !Buffer::rows_equal(prev.row(y), next.row(y))
        })
        .count();
    if changed_rows * 2 <= height {
        return None;
    }
    scratch.prev.clear();
    scratch.next.clear();
    scratch.prev.extend((0..height).map(|y| prev.hash_row(y)));
    scratch.next.extend((0..height).map(|y| next.hash_row(y)));
    scratch.changed.clear();
    scratch.drawn.clear();
    scratch.changed.push(0);
    scratch.drawn.push(0);
    for y in 0..height {
        let cost = row_cost(next, y);
        let differs = scratch.prev[y] != scratch.next[y];
        let changed = scratch.changed[y] + if differs { cost } else { 0 };
        let drawn = scratch.drawn[y] + if cost > 4 { cost } else { 0 };
        scratch.changed.push(changed);
        scratch.drawn.push(drawn);
    }
    let changed = &scratch.changed;
    let drawn = &scratch.drawn;
    let outside = |top: usize, bottom: usize| changed[top] + (changed[height] - changed[bottom]);
    let mut best: Option<(usize, Scroll)> = None;
    let mut consider = |cost: usize, scroll: Scroll| {
        if scroll.bottom - scroll.top >= MIN_REGION && best.is_none_or(|(c, _)| cost < c) {
            best = Some((cost, scroll));
        }
    };
    for by in 1..height {
        // Moving up, row y of the next frame is row y + by of the last.
        for (start, end) in matching_runs(&scratch.next, &scratch.prev[by..]) {
            let (top, bottom) = (start, end + by);
            let cost = SCROLL_COST + outside(top, bottom) + drawn[bottom] - drawn[end];
            consider(
                cost,
                Scroll {
                    top,
                    bottom,
                    by,
                    up: true,
                },
            );
        }
        // Moving down, row y + by of the next frame is row y of the last.
        for (start, end) in matching_runs(&scratch.next[by..], &scratch.prev) {
            let (top, bottom) = (start, end + by);
            let cost = SCROLL_COST + outside(top, bottom) + drawn[start + by] - drawn[start];
            consider(
                cost,
                Scroll {
                    top,
                    bottom,
                    by,
                    up: false,
                },
            );
        }
    }
    let (cost, scroll) = best?;
    (cost < changed[height]).then_some(scroll)
}

/// The runs of indexes where `a` and `b` hold the same hash, as `(start, end)`.
fn matching_runs<'a>(a: &'a [u64], b: &'a [u64]) -> impl Iterator<Item = (usize, usize)> + 'a {
    let len = a.len().min(b.len());
    let mut y = 0;
    core::iter::from_fn(move || {
        while y < len && a[y] != b[y] {
            y += 1;
        }
        if y >= len {
            return None;
        }
        let start = y;
        while y < len && a[y] == b[y] {
            y += 1;
        }
        Some((start, y))
    })
}

/// An estimate of the bytes row `y` of `next` takes to write, from its cells that aren't blank and
/// a move.
fn row_cost(next: &Buffer, y: usize) -> usize {
    4 + next.row(y).iter().filter(|c| !next.looks_blank(c)).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Rect;
    use crate::style::{Color, Modifier, Style};

    fn runs(prev: &Buffer, next: &Buffer) -> Vec<(usize, usize, usize)> {
        let mut runs = Vec::new();
        let width = usize::from(next.area().width);
        for y in 0..usize::from(next.area().height) {
            diff_row(Before::Row(prev, y), next, y, &[(0, width)], &mut runs);
        }
        runs.iter().map(|r| (r.row, r.start, r.end)).collect()
    }

    fn buffer(rows: &[&str]) -> Buffer {
        let width = rows
            .iter()
            .map(|r| crate::text::width(r, crate::caps::WidthProfile::default()))
            .max()
            .unwrap_or(0);
        let mut b = Buffer::empty(Rect::new(
            0,
            0,
            u16::try_from(width).unwrap(),
            u16::try_from(rows.len()).unwrap(),
        ));
        for (y, row) in rows.iter().enumerate() {
            b.set_string(0, u16::try_from(y).unwrap(), row, Style::new());
        }
        b
    }

    #[test]
    fn equal_rows_give_no_runs() {
        let a = buffer(&["hello", "world"]);
        assert!(runs(&a, &a.clone()).is_empty());
        let mut rich = buffer(&["a👨\u{200d}👩\u{200d}👧b"]);
        rich.set_link(Rect::new(0, 0, 1, 1), Some("u"));
        let mut other = buffer(&["a👨\u{200d}👩\u{200d}👧b"]);
        other.set_string(0, 0, "x", Style::new());
        other.set_string(0, 0, "a", Style::new());
        other.set_link(Rect::new(0, 0, 1, 1), Some("u"));
        assert!(runs(&rich, &other).is_empty());
    }

    #[test]
    fn changed_cells_make_runs() {
        let a = buffer(&["hello world, again"]);
        let b = buffer(&["hellO world, agaiN"]);
        assert_eq!(runs(&a, &b), [(0, 4, 5), (0, 17, 18)]);
    }

    #[test]
    fn a_cheap_gap_joins_two_runs() {
        let a = buffer(&["abcdefgh"]);
        let b = buffer(&["Abcdefgh"]);
        assert_eq!(runs(&a, &buffer(&["AbCdefgh"])), [(0, 0, 3)]);
        assert_eq!(runs(&a, &buffer(&["AbcdEfgh"])), [(0, 0, 5)]);
        assert_eq!(
            runs(&a, &buffer(&["AbcdeFgh"])),
            [(0, 0, 6)],
            "a tie writes the gap"
        );
        assert_eq!(runs(&a, &buffer(&["AbcdefGh"])), [(0, 0, 1), (0, 6, 7)]);
        assert_eq!(runs(&a, &b), [(0, 0, 1)]);
    }

    #[test]
    fn a_gap_in_another_style_stays_a_gap() {
        let mut a = buffer(&["abcd"]);
        a.set_style(Rect::new(1, 0, 1, 1), Style::new().fg(Color::Red));
        let mut b = a.clone();
        b.set_string(0, 0, "A", Style::new());
        b.set_string(2, 0, "C", Style::new());
        assert_eq!(runs(&a, &b), [(0, 0, 1), (0, 2, 3)]);
    }

    #[test]
    fn a_wide_character_is_one_unit() {
        let a = buffer(&["ab界cd"]);
        let mut b = a.clone();
        b.set_style(Rect::new(3, 0, 1, 1), Style::new().fg(Color::Red));
        assert!(
            runs(&a, &b).is_empty(),
            "a continuation cell's style alone shows nothing"
        );
        b.set_style(Rect::new(2, 0, 1, 1), Style::new().fg(Color::Red));
        assert_eq!(runs(&a, &b), [(0, 2, 4)]);
        let c = buffer(&["abxycd"]);
        assert_eq!(runs(&a, &c), [(0, 2, 4)]);
        let d = buffer(&["a界ycd"]);
        assert_eq!(runs(&a, &d), [(0, 1, 4)]);
    }

    #[test]
    fn spaces_that_look_blank_are_equal() {
        let a = buffer(&["a   "]);
        let mut b = a.clone();
        b.set_style(
            Rect::new(1, 0, 3, 1),
            Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
        );
        assert!(runs(&a, &b).is_empty());
        b.set_style(
            Rect::new(2, 0, 1, 1),
            Style::new().add_modifier(Modifier::REVERSED),
        );
        assert_eq!(runs(&a, &b), [(0, 2, 3)]);
    }

    #[test]
    fn columns_limit_the_diff() {
        let a = buffer(&["abcdefgh"]);
        let b = buffer(&["ABCDEFGH"]);
        let mut out = Vec::new();
        diff_row(Before::Row(&a, 0), &b, 0, &[(1, 3), (6, 7)], &mut out);
        let got: Vec<_> = out.iter().map(|r| (r.start, r.end)).collect();
        assert_eq!(got, [(1, 3), (6, 7)]);
        let wide = buffer(&["a界b"]);
        let mut out = Vec::new();
        diff_row(Before::Blank, &wide, 0, &[(2, 3)], &mut out);
        assert_eq!(
            (out[0].start, out[0].end),
            (1, 3),
            "a span inside a wide character takes it whole"
        );
    }

    #[test]
    fn a_blank_before_gives_the_cells_that_show() {
        let b = buffer(&["  ab      c "]);
        let mut out = Vec::new();
        diff_row(Before::Blank, &b, 0, &[(0, 12)], &mut out);
        let got: Vec<_> = out.iter().map(|r| (r.start, r.end)).collect();
        assert_eq!(got, [(2, 4), (10, 11)]);
    }

    #[test]
    fn digits_counts_decimal_digits() {
        assert_eq!([0, 9, 10, 99, 100, 65535].map(digits), [1, 1, 2, 2, 3, 5]);
    }

    fn refs(v: &[String]) -> Vec<&str> {
        v.iter().map(String::as_str).collect()
    }

    fn lines(n: usize, from: usize) -> Vec<String> {
        (from..from + n)
            .map(|i| format!("line {i:03} of the transcript"))
            .collect()
    }

    #[test]
    fn streaming_one_row_up_is_found_as_a_scroll() {
        let before: Vec<String> = lines(10, 0);
        let mut after: Vec<String> = lines(9, 1);
        after.push(String::new());
        let a = buffer(&refs(&before));
        let mut b = Buffer::empty(a.area());
        for (y, row) in after.iter().enumerate() {
            b.set_string(0, u16::try_from(y).unwrap(), row, Style::new());
        }
        let mut scratch = ScrollScratch::default();
        let scroll = detect_scroll(&a, &b, &mut scratch).unwrap();
        assert_eq!(
            scroll,
            Scroll {
                top: 0,
                bottom: 10,
                by: 1,
                up: true
            }
        );
        assert!(matches!(scroll.before(&a, 9), Before::Blank));
        assert!(matches!(scroll.before(&a, 0), Before::Row(_, 1)));
        let down = detect_scroll(&b, &a, &mut scratch);
        assert_eq!(
            down,
            Some(Scroll {
                top: 0,
                bottom: 10,
                by: 1,
                up: false
            })
        );
    }

    #[test]
    fn a_fixed_status_row_stays_out_of_the_region() {
        let mut before = lines(8, 0);
        before.push("status: ready".to_owned());
        let mut after = lines(7, 1);
        after.push("line 008 of the transcript".to_owned());
        after.push("status: ready".to_owned());
        let (a, b) = (buffer(&refs(&before)), buffer(&refs(&after)));
        let scroll = detect_scroll(&a, &b, &mut ScrollScratch::default()).unwrap();
        assert_eq!(
            scroll,
            Scroll {
                top: 0,
                bottom: 8,
                by: 1,
                up: true
            }
        );
    }

    #[test]
    fn no_scroll_when_few_rows_changed() {
        let a = buffer(&["a", "b", "c", "d"]);
        let b = buffer(&["a", "b", "c", "e"]);
        assert_eq!(detect_scroll(&a, &b, &mut ScrollScratch::default()), None);
    }
}
