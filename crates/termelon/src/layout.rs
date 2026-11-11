//! Positions, sizes and rectangles of cells.
//!
//! Columns grow to the right and rows down from 0 at the top-left. Every method saturates at
//! `u16::MAX` rather than overflowing or panicking, so a rectangle near the edge of the coordinate
//! space shrinks instead of wrapping.

use core::iter::FusedIterator;

/// A cell's column and row.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Position {
    /// The column, from 0 at the left.
    pub x: u16,
    /// The row, from 0 at the top.
    pub y: u16,
}

impl Position {
    /// The top-left cell.
    pub const ORIGIN: Self = Self::new(0, 0);

    /// The cell at column `x` and row `y`.
    pub const fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }
}

impl From<(u16, u16)> for Position {
    /// Takes `(x, y)`.
    fn from((x, y): (u16, u16)) -> Self {
        Self::new(x, y)
    }
}

impl From<Position> for (u16, u16) {
    fn from(position: Position) -> Self {
        (position.x, position.y)
    }
}

/// A width and height in cells.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Size {
    /// Columns.
    pub width: u16,
    /// Rows.
    pub height: u16,
}

impl Size {
    /// No cells.
    pub const ZERO: Self = Self::new(0, 0);

    /// `width` columns by `height` rows.
    pub const fn new(width: u16, height: u16) -> Self {
        Self { width, height }
    }

    /// The number of cells.
    pub const fn area(self) -> u32 {
        self.width as u32 * self.height as u32
    }
}

impl From<(u16, u16)> for Size {
    /// Takes `(width, height)`.
    fn from((width, height): (u16, u16)) -> Self {
        Self::new(width, height)
    }
}

impl From<Size> for (u16, u16) {
    fn from(size: Size) -> Self {
        (size.width, size.height)
    }
}

/// Space to leave on each side of a rectangle, for [`Rect::inner`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Margin {
    /// Columns to leave on the left and on the right.
    pub horizontal: u16,
    /// Rows to leave at the top and at the bottom.
    pub vertical: u16,
}

impl Margin {
    /// `horizontal` columns on each side and `vertical` rows above and below.
    pub const fn new(horizontal: u16, vertical: u16) -> Self {
        Self {
            horizontal,
            vertical,
        }
    }
}

/// A rectangle of cells, from its top-left cell.
///
/// [`right`](Self::right) and [`bottom`](Self::bottom) are the first column and row outside it.
/// They saturate at `u16::MAX`, so the last column and row of the coordinate space are never inside
/// a rectangle. [`Rect::new`] shrinks a rectangle to fit, and the methods treat one built with a
/// struct literal that doesn't fit as if it had been shrunk.
///
/// ```
/// use termelon::layout::{Position, Rect};
///
/// let area = Rect::new(0, 0, 80, 24);
/// let status = Rect::new(area.x, area.bottom() - 1, area.width, 1);
/// assert!(status.contains(Position::new(79, 23)));
/// assert!(!status.contains(Position::new(80, 23)));
/// ```
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rect {
    /// The left column.
    pub x: u16,
    /// The top row.
    pub y: u16,
    /// Columns.
    pub width: u16,
    /// Rows.
    pub height: u16,
}

impl Rect {
    /// The empty rectangle at the origin.
    pub const ZERO: Self = Self::new(0, 0, 0, 0);

    /// The rectangle at column `x` and row `y`, shrunk so that its right and bottom edges fit in a
    /// `u16`.
    pub const fn new(x: u16, y: u16, width: u16, height: u16) -> Self {
        Self {
            x,
            y,
            width: x.saturating_add(width) - x,
            height: y.saturating_add(height) - y,
        }
    }

    /// The number of cells.
    pub const fn area(self) -> u32 {
        self.width as u32 * self.height as u32
    }

    /// Whether it holds no cells.
    pub const fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    /// The first column inside, `x`.
    pub const fn left(self) -> u16 {
        self.x
    }

    /// The first column after it.
    pub const fn right(self) -> u16 {
        self.x.saturating_add(self.width)
    }

    /// The first row inside, `y`.
    pub const fn top(self) -> u16 {
        self.y
    }

    /// The first row below it.
    pub const fn bottom(self) -> u16 {
        self.y.saturating_add(self.height)
    }

    /// The top-left cell.
    pub const fn as_position(self) -> Position {
        Position::new(self.x, self.y)
    }

    /// The width and height.
    pub const fn as_size(self) -> Size {
        Size::new(self.width, self.height)
    }

    /// The rectangle with `margin` taken off each side. A margin wider or taller than the
    /// rectangle leaves it empty, at its middle.
    #[must_use]
    pub const fn inner(self, margin: Margin) -> Self {
        let width = self.right() - self.x;
        let height = self.bottom() - self.y;
        let left = min(margin.horizontal, width / 2);
        let top = min(margin.vertical, height / 2);
        Self {
            x: self.x + left,
            y: self.y + top,
            width: width.saturating_sub(margin.horizontal.saturating_mul(2)),
            height: height.saturating_sub(margin.vertical.saturating_mul(2)),
        }
    }

    /// The cells both rectangles hold. When they don't overlap, the result is empty.
    #[must_use]
    pub const fn intersection(self, other: Self) -> Self {
        let x = max(self.x, other.x);
        let y = max(self.y, other.y);
        let right = min(self.right(), other.right());
        let bottom = min(self.bottom(), other.bottom());
        Self {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        }
    }

    /// The smallest rectangle that holds both. An empty rectangle holds no cells, so the union
    /// with one is the other.
    #[must_use]
    pub const fn union(self, other: Self) -> Self {
        if other.is_empty() {
            return self;
        }
        if self.is_empty() {
            return other;
        }
        let x = min(self.x, other.x);
        let y = min(self.y, other.y);
        Self {
            x,
            y,
            width: max(self.right(), other.right()) - x,
            height: max(self.bottom(), other.bottom()) - y,
        }
    }

    /// Whether the two share a cell. An empty rectangle shares none.
    pub const fn intersects(self, other: Self) -> bool {
        !self.intersection(other).is_empty()
    }

    /// Whether the cell at `position` is inside.
    pub const fn contains(self, position: Position) -> bool {
        position.x >= self.x
            && position.x < self.right()
            && position.y >= self.y
            && position.y < self.bottom()
    }

    /// The rectangle moved, and shrunk where it's bigger, to fit inside `other`. Unlike
    /// [`intersection`](Self::intersection), it keeps as much of its size as fits.
    ///
    /// ```
    /// use termelon::layout::Rect;
    ///
    /// let screen = Rect::new(0, 0, 80, 24);
    /// assert_eq!(Rect::new(70, 20, 20, 2).clamp(screen), Rect::new(60, 20, 20, 2));
    /// ```
    #[must_use]
    pub const fn clamp(self, other: Self) -> Self {
        let width = min(self.width, other.right() - other.x);
        let height = min(self.height, other.bottom() - other.y);
        Self {
            x: min(max(self.x, other.x), other.right() - width),
            y: min(max(self.y, other.y), other.bottom() - height),
            width,
            height,
        }
    }

    /// Its rows, from the top, each a rectangle one row high.
    pub const fn rows(self) -> Rows {
        Rows {
            rect: self,
            front: self.y,
            back: self.bottom(),
        }
    }

    /// Its columns, from the left, each a rectangle one column wide.
    pub const fn columns(self) -> Columns {
        Columns {
            rect: self,
            front: self.x,
            back: self.right(),
        }
    }

    /// Its cells, a row at a time from the top-left.
    pub const fn positions(self) -> Positions {
        Positions {
            rect: self,
            next: self.as_position(),
        }
    }
}

impl From<(Position, Size)> for Rect {
    fn from((position, size): (Position, Size)) -> Self {
        Self::new(position.x, position.y, size.width, size.height)
    }
}

impl From<Size> for Rect {
    /// The rectangle of that size at the origin.
    fn from(size: Size) -> Self {
        Self::new(0, 0, size.width, size.height)
    }
}

/// The rows of a [`Rect`], from [`Rect::rows`].
#[derive(Debug, Clone)]
pub struct Rows {
    rect: Rect,
    front: u16,
    back: u16,
}

impl Iterator for Rows {
    type Item = Rect;

    fn next(&mut self) -> Option<Rect> {
        if self.front >= self.back {
            return None;
        }
        let row = Rect::new(self.rect.x, self.front, self.rect.width, 1);
        self.front += 1;
        Some(row)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = usize::from(self.back.saturating_sub(self.front));
        (n, Some(n))
    }
}

impl DoubleEndedIterator for Rows {
    fn next_back(&mut self) -> Option<Rect> {
        if self.front >= self.back {
            return None;
        }
        self.back -= 1;
        Some(Rect::new(self.rect.x, self.back, self.rect.width, 1))
    }
}

impl ExactSizeIterator for Rows {}
impl FusedIterator for Rows {}

/// The columns of a [`Rect`], from [`Rect::columns`].
#[derive(Debug, Clone)]
pub struct Columns {
    rect: Rect,
    front: u16,
    back: u16,
}

impl Iterator for Columns {
    type Item = Rect;

    fn next(&mut self) -> Option<Rect> {
        if self.front >= self.back {
            return None;
        }
        let column = Rect::new(self.front, self.rect.y, 1, self.rect.height);
        self.front += 1;
        Some(column)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = usize::from(self.back.saturating_sub(self.front));
        (n, Some(n))
    }
}

impl DoubleEndedIterator for Columns {
    fn next_back(&mut self) -> Option<Rect> {
        if self.front >= self.back {
            return None;
        }
        self.back -= 1;
        Some(Rect::new(self.back, self.rect.y, 1, self.rect.height))
    }
}

impl ExactSizeIterator for Columns {}
impl FusedIterator for Columns {}

/// The cells of a [`Rect`], from [`Rect::positions`].
#[derive(Debug, Clone)]
pub struct Positions {
    rect: Rect,
    next: Position,
}

impl Iterator for Positions {
    type Item = Position;

    fn next(&mut self) -> Option<Position> {
        let (right, bottom) = (self.rect.right(), self.rect.bottom());
        if self.rect.x >= right || self.next.y >= bottom {
            return None;
        }
        let position = self.next;
        self.next.x += 1;
        if self.next.x >= right {
            self.next.x = self.rect.x;
            self.next.y += 1;
        }
        Some(position)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let (right, bottom) = (self.rect.right(), self.rect.bottom());
        if self.rect.x >= right || self.next.y >= bottom {
            return (0, Some(0));
        }
        let width = usize::from(right - self.rect.x);
        let rows_after = usize::from(bottom - self.next.y - 1);
        let n = rows_after * width + usize::from(right - self.next.x);
        (n, Some(n))
    }
}

impl ExactSizeIterator for Positions {}
impl FusedIterator for Positions {}

/// Where text sits across a wider area.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Alignment {
    /// At the left.
    #[default]
    Left,
    /// In the middle, with the odd cell on the right.
    Center,
    /// At the right.
    Right,
}

/// The size [`Layout`] gives one of the areas it splits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Constraint {
    /// Exactly this many cells.
    Length(u16),
    /// At least this many cells, and more when no [`Fill`](Self::Fill) takes the space left.
    Min(u16),
    /// At most this many cells.
    Max(u16),
    /// This percentage of the area, rounded down. Above 100 counts as 100.
    Percentage(u16),
    /// This fraction of the area, rounded down. A fraction over 1 counts as 1, and a zero
    /// denominator as 0.
    Ratio(u32, u32),
    /// A share of the space left, in proportion to the weight.
    Fill(u16),
}

/// Splits a rectangle into a row or a column of areas by [`Constraint`]s.
///
/// The solver takes one pass, so the same constraints always give the same areas.
///
/// 1. `Length(n)`, `Min(n)` and `Max(n)` start at n cells, `Percentage` and `Ratio` at their part
///    of the area, and `Fill` at none.
/// 2. When those overflow the area, `Percentage`, `Ratio` and `Max` shrink first, then `Length` and
///    `Min`. In each group the last constraints shrink first, each to nothing before the one before
///    it shrinks.
/// 3. Otherwise the space left goes to the `Fill` constraints by weight. With no `Fill` (or only
///    zero weights), the `Min` constraints share it equally.
/// 4. Space nothing takes stays empty after the last area.
///
/// Shares are rounded down, and the cells left over go one each to the first constraints that
/// take a share.
///
/// ```
/// use termelon::layout::{Constraint, Layout, Rect};
///
/// let [title, body, status] = Layout::vertical([
///     Constraint::Length(1),
///     Constraint::Fill(1),
///     Constraint::Length(1),
/// ])
/// .areas(Rect::new(0, 0, 80, 24));
/// assert_eq!(title, Rect::new(0, 0, 80, 1));
/// assert_eq!(body, Rect::new(0, 1, 80, 22));
/// assert_eq!(status, Rect::new(0, 23, 80, 1));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Layout {
    vertical: bool,
    constraints: Vec<Constraint>,
}

impl Layout {
    /// Areas stacked from the top, each as wide as the rectangle.
    pub fn vertical<I>(constraints: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<Constraint>,
    {
        Self {
            vertical: true,
            constraints: constraints.into_iter().map(Into::into).collect(),
        }
    }

    /// Areas side by side from the left, each as tall as the rectangle.
    pub fn horizontal<I>(constraints: I) -> Self
    where
        I: IntoIterator,
        I::Item: Into<Constraint>,
    {
        Self {
            vertical: false,
            constraints: constraints.into_iter().map(Into::into).collect(),
        }
    }

    /// The areas, one for each constraint, in order.
    pub fn split(&self, area: Rect) -> Vec<Rect> {
        let (start, total) = if self.vertical {
            (area.y, area.bottom() - area.y)
        } else {
            (area.x, area.right() - area.x)
        };
        let sizes = solve(&self.constraints, total);
        let mut offset = start;
        sizes
            .into_iter()
            .map(|size| {
                let rect = if self.vertical {
                    Rect::new(area.x, offset, area.width, size)
                } else {
                    Rect::new(offset, area.y, size, area.height)
                };
                offset = offset.saturating_add(size);
                rect
            })
            .collect()
    }

    /// The areas as an array, for destructuring. When `N` differs from the number of
    /// constraints, the extra constraints are left out, or the extra areas are empty at the end.
    pub fn areas<const N: usize>(&self, area: Rect) -> [Rect; N] {
        let split = self.split(area);
        let end = if self.vertical {
            Rect::new(area.x, area.bottom(), area.width, 0)
        } else {
            Rect::new(area.right(), area.y, 0, area.height)
        };
        core::array::from_fn(|i| split.get(i).copied().unwrap_or(end))
    }
}

/// The size of each constraint along `total` cells, by the rules on [`Layout`].
fn solve(constraints: &[Constraint], total: u16) -> Vec<u16> {
    let whole = u64::from(total);
    let mut sizes: Vec<u16> = constraints
        .iter()
        .map(|&c| match c {
            Constraint::Length(n) | Constraint::Min(n) | Constraint::Max(n) => n,
            Constraint::Percentage(p) => part(whole * u64::from(p.min(100)) / 100),
            Constraint::Ratio(_, 0) | Constraint::Fill(_) => 0,
            Constraint::Ratio(a, b) => part(whole * u64::from(a.min(b)) / u64::from(b)),
        })
        .collect();
    let used: u64 = sizes.iter().map(|&s| u64::from(s)).sum();
    if used > whole {
        let mut excess = used - whole;
        // Parts of the area and maximums give way first, then lengths and minimums.
        for firm in [false, true] {
            for (size, &c) in sizes.iter_mut().zip(constraints).rev() {
                if matches!(c, Constraint::Length(_) | Constraint::Min(_)) != firm {
                    continue;
                }
                let cut = excess.min(u64::from(*size));
                *size -= part(cut);
                excess -= cut;
            }
        }
        return sizes;
    }
    let fills: Vec<u64> = constraints
        .iter()
        .map(|&c| match c {
            Constraint::Fill(w) => u64::from(w),
            _ => 0,
        })
        .collect();
    let weights = if fills.iter().any(|&w| w > 0) {
        fills
    } else {
        constraints
            .iter()
            .map(|&c| u64::from(matches!(c, Constraint::Min(_))))
            .collect()
    };
    share(&mut sizes, &weights, whole - used);
    sizes
}

/// Adds `free` cells to `sizes` in proportion to `weights`, rounding down, then one each from the
/// first with a weight until none are left.
fn share(sizes: &mut [u16], weights: &[u64], free: u64) {
    let total_weight: u64 = weights.iter().sum();
    if total_weight == 0 {
        return;
    }
    let mut left = free;
    for (size, &w) in sizes.iter_mut().zip(weights) {
        let add = free * w / total_weight;
        *size = size.saturating_add(part(add));
        left -= add;
    }
    for (size, _) in sizes.iter_mut().zip(weights).filter(|(_, w)| **w > 0) {
        if left == 0 {
            break;
        }
        *size = size.saturating_add(1);
        left -= 1;
    }
}

/// A count of cells known to fit in the area, as a `u16`.
fn part(cells: u64) -> u16 {
    u16::try_from(cells).unwrap_or(u16::MAX)
}

const fn min(a: u16, b: u16) -> u16 {
    if a < b { a } else { b }
}

const fn max(a: u16, b: u16) -> u16 {
    if a > b { a } else { b }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: u16 = u16::MAX;

    /// A rectangle built as a literal that runs past the edge of the coordinate space.
    const OVERHANG: Rect = Rect {
        x: MAX - 2,
        y: MAX - 1,
        width: 10,
        height: 10,
    };

    #[test]
    fn new_shrinks_to_fit() {
        assert_eq!(
            Rect::new(MAX - 3, MAX, 10, 10),
            Rect::new(MAX - 3, MAX, 3, 0)
        );
        assert_eq!(
            Rect::new(0, 0, MAX, MAX).area(),
            u32::from(MAX) * u32::from(MAX)
        );
        assert_eq!(Size::new(MAX, MAX).area(), u32::from(MAX) * u32::from(MAX));
    }

    #[test]
    fn edges_saturate() {
        assert_eq!(OVERHANG.right(), MAX);
        assert_eq!(OVERHANG.bottom(), MAX);
        assert_eq!(OVERHANG.left(), MAX - 2);
        assert_eq!(OVERHANG.top(), MAX - 1);
        let r = Rect::new(3, 4, 5, 6);
        assert_eq!((r.left(), r.top(), r.right(), r.bottom()), (3, 4, 8, 10));
        assert_eq!(r.as_position(), Position::new(3, 4));
        assert_eq!(r.as_size(), Size::new(5, 6));
    }

    #[test]
    fn empty_rects() {
        assert!(Rect::ZERO.is_empty());
        assert!(Rect::new(5, 5, 0, 3).is_empty());
        assert!(Rect::new(5, 5, 3, 0).is_empty());
        assert!(!Rect::new(5, 5, 1, 1).is_empty());
        assert!(!Rect::new(5, 5, 0, 0).contains(Position::new(5, 5)));
        assert_eq!(Rect::new(5, 5, 0, 4).rows().count(), 4);
        assert_eq!(Rect::new(5, 5, 0, 4).positions().count(), 0);
    }

    #[test]
    fn contains_is_inclusive_at_the_top_left_only() {
        let r = Rect::new(1, 2, 3, 4);
        assert!(r.contains(Position::new(1, 2)));
        assert!(r.contains(Position::new(3, 5)));
        assert!(!r.contains(Position::new(4, 5)));
        assert!(!r.contains(Position::new(3, 6)));
        assert!(!r.contains(Position::new(0, 2)));
        assert!(!Rect::new(0, 0, MAX, MAX).contains(Position::new(MAX, 0)));
    }

    #[test]
    fn inner_takes_the_margin_off_each_side() {
        let r = Rect::new(10, 10, 20, 8);
        assert_eq!(r.inner(Margin::new(1, 1)), Rect::new(11, 11, 18, 6));
        assert_eq!(r.inner(Margin::new(0, 0)), r);
        assert_eq!(r.inner(Margin::new(10, 4)), Rect::new(20, 14, 0, 0));
        assert_eq!(r.inner(Margin::new(MAX, MAX)), Rect::new(20, 14, 0, 0));
        assert_eq!(
            OVERHANG.inner(Margin::new(1, 0)),
            Rect::new(MAX - 1, MAX - 1, 0, 1)
        );
        assert_eq!(Rect::ZERO.inner(Margin::new(3, 3)), Rect::ZERO);
    }

    #[test]
    fn intersection_and_intersects() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersection(b), Rect::new(5, 5, 5, 5));
        assert!(a.intersects(b));
        let far = Rect::new(20, 20, 5, 5);
        assert!(a.intersection(far).is_empty());
        assert!(!a.intersects(far));
        let touching = Rect::new(10, 0, 5, 5);
        assert!(!a.intersects(touching));
        assert!(!a.intersects(Rect::new(3, 3, 0, 0)));
        assert_eq!(
            OVERHANG.intersection(Rect::new(0, 0, MAX, MAX)),
            Rect::new(MAX - 2, MAX - 1, 2, 1)
        );
    }

    #[test]
    fn union_ignores_empty_rects() {
        let a = Rect::new(2, 3, 4, 5);
        let b = Rect::new(10, 1, 2, 2);
        assert_eq!(a.union(b), Rect::new(2, 1, 10, 7));
        assert_eq!(a.union(Rect::ZERO), a);
        assert_eq!(Rect::ZERO.union(a), a);
        assert_eq!(
            a.union(Rect::new(MAX - 1, MAX - 1, 9, 9)),
            Rect::new(2, 3, MAX - 2, MAX - 3)
        );
        assert_eq!(
            Rect::new(0, 0, 1, 1).union(OVERHANG),
            Rect::new(0, 0, MAX, MAX)
        );
    }

    #[test]
    fn clamp_moves_then_shrinks() {
        let screen = Rect::new(0, 0, 80, 24);
        assert_eq!(
            Rect::new(70, 20, 20, 10).clamp(screen),
            Rect::new(60, 14, 20, 10)
        );
        assert_eq!(Rect::new(10, 10, 100, 100).clamp(screen), screen);
        let inside = Rect::new(5, 5, 3, 3);
        assert_eq!(inside.clamp(screen), inside);
        let area = Rect::new(10, 10, 5, 5);
        assert_eq!(Rect::new(0, 0, 2, 2).clamp(area), Rect::new(10, 10, 2, 2));
        assert_eq!(Rect::new(0, 0, 9, 9).clamp(Rect::ZERO), Rect::ZERO);
        assert_eq!(
            Rect::new(0, 0, 9, 9).clamp(OVERHANG),
            Rect::new(MAX - 2, MAX - 1, 2, 1)
        );
    }

    #[test]
    fn rows_and_columns() {
        let r = Rect::new(1, 2, 3, 2);
        assert_eq!(
            r.rows().collect::<Vec<_>>(),
            [Rect::new(1, 2, 3, 1), Rect::new(1, 3, 3, 1)]
        );
        assert_eq!(
            r.rows().rev().collect::<Vec<_>>(),
            [Rect::new(1, 3, 3, 1), Rect::new(1, 2, 3, 1)]
        );
        assert_eq!(
            r.columns().collect::<Vec<_>>(),
            [
                Rect::new(1, 2, 1, 2),
                Rect::new(2, 2, 1, 2),
                Rect::new(3, 2, 1, 2)
            ]
        );
        assert_eq!(r.columns().next_back(), Some(Rect::new(3, 2, 1, 2)));
        assert_eq!(r.columns().len(), 3);
        let mut rows = r.rows();
        assert_eq!(rows.next_back(), Some(Rect::new(1, 3, 3, 1)));
        assert_eq!(rows.next(), Some(Rect::new(1, 2, 3, 1)));
        assert_eq!(rows.next(), None);
        assert_eq!(rows.next_back(), None);
        assert_eq!(OVERHANG.columns().len(), 2);
        assert_eq!(OVERHANG.rows().len(), 1);
        assert_eq!(
            OVERHANG.rows().next(),
            Some(Rect::new(MAX - 2, MAX - 1, 2, 1))
        );
    }

    #[test]
    fn positions_go_row_by_row() {
        let r = Rect::new(4, 7, 2, 2);
        let mut positions = r.positions();
        assert_eq!(positions.len(), 4);
        assert_eq!(positions.next(), Some(Position::new(4, 7)));
        assert_eq!(positions.len(), 3);
        assert_eq!(
            positions.collect::<Vec<_>>(),
            [
                Position::new(5, 7),
                Position::new(4, 8),
                Position::new(5, 8)
            ]
        );
        assert_eq!(
            OVERHANG.positions().collect::<Vec<_>>(),
            [
                Position::new(MAX - 2, MAX - 1),
                Position::new(MAX - 1, MAX - 1)
            ]
        );
        assert_eq!(
            Rect::new(0, 0, MAX, 1).positions().count(),
            usize::from(MAX)
        );
    }

    fn sizes(constraints: &[Constraint], total: u16) -> Vec<u16> {
        let rects =
            Layout::horizontal(constraints.iter().copied()).split(Rect::new(0, 0, total, 1));
        let mut x = 0;
        for r in &rects {
            assert_eq!(r.x, x, "areas are packed from the left");
            x += r.width;
        }
        rects.iter().map(|r| r.width).collect()
    }

    use Constraint::{Fill, Length, Max, Min, Percentage, Ratio};

    #[test]
    fn fixed_sizes_come_first() {
        assert_eq!(sizes(&[Length(3), Length(5)], 20), [3, 5]);
        assert_eq!(sizes(&[Percentage(25), Percentage(50)], 20), [5, 10]);
        assert_eq!(sizes(&[Percentage(33)], 10), [3]);
        assert_eq!(sizes(&[Percentage(250)], 10), [10]);
        assert_eq!(sizes(&[Ratio(1, 3), Ratio(2, 3)], 10), [3, 6]);
        assert_eq!(sizes(&[Ratio(5, 2), Ratio(1, 0)], 10), [10, 0]);
        assert_eq!(sizes(&[Min(4), Max(6)], 10), [4, 6]);
    }

    #[test]
    fn overflow_shrinks_the_last_constraints_first() {
        assert_eq!(sizes(&[Length(5), Length(5), Length(5)], 12), [5, 5, 2]);
        assert_eq!(sizes(&[Length(5), Min(5), Length(5)], 7), [5, 2, 0]);
        assert_eq!(sizes(&[Length(5), Fill(1), Length(5)], 6), [5, 0, 1]);
        assert_eq!(sizes(&[Length(u16::MAX), Length(u16::MAX)], 3), [3, 0]);
    }

    #[test]
    fn overflow_shrinks_parts_and_maximums_before_lengths_and_minimums() {
        assert_eq!(sizes(&[Percentage(100), Min(20)], 50), [30, 20]);
        assert_eq!(sizes(&[Min(20), Percentage(100)], 50), [20, 30]);
        assert_eq!(sizes(&[Length(3), Ratio(1, 1), Max(4)], 10), [3, 7, 0]);
        assert_eq!(
            sizes(&[Percentage(50), Percentage(50), Length(4)], 10),
            [5, 1, 4]
        );
        assert_eq!(sizes(&[Percentage(100), Length(8), Min(8)], 10), [0, 8, 2]);
    }

    #[test]
    fn the_rest_goes_to_fill_by_weight() {
        assert_eq!(sizes(&[Length(2), Fill(1), Fill(3)], 10), [2, 2, 6]);
        assert_eq!(sizes(&[Fill(1), Fill(1), Fill(1)], 10), [4, 3, 3]);
        assert_eq!(sizes(&[Fill(0), Fill(2)], 10), [0, 10]);
        assert_eq!(sizes(&[Min(3), Fill(1)], 10), [3, 7]);
        assert_eq!(sizes(&[Max(3), Fill(1)], 10), [3, 7]);
    }

    #[test]
    fn without_fill_min_constraints_share_the_rest_equally() {
        assert_eq!(sizes(&[Length(1), Min(0), Length(1)], 24), [1, 22, 1]);
        assert_eq!(sizes(&[Min(2), Min(4)], 11), [5, 6]);
        assert_eq!(sizes(&[Min(1), Fill(0)], 5), [5, 0]);
    }

    #[test]
    fn space_nothing_takes_stays_empty_at_the_end() {
        assert_eq!(sizes(&[Length(2), Max(3)], 10), [2, 3]);
        assert_eq!(sizes(&[], 10), Vec::<u16>::new());
    }

    #[test]
    fn vertical_layouts_stack_rows_inside_the_area() {
        let area = Rect::new(3, 4, 10, 6);
        let rects = Layout::vertical([Length(1), Fill(1), Length(2)]).split(area);
        assert_eq!(
            rects,
            [
                Rect::new(3, 4, 10, 1),
                Rect::new(3, 5, 10, 3),
                Rect::new(3, 8, 10, 2)
            ]
        );
        let rects = Layout::horizontal([Percentage(50), Fill(1)]).split(area);
        assert_eq!(rects, [Rect::new(3, 4, 5, 6), Rect::new(8, 4, 5, 6)]);
    }

    #[test]
    fn areas_pads_or_drops_to_its_length() {
        let area = Rect::new(0, 0, 10, 4);
        let [a] = Layout::vertical([Length(1), Length(1)]).areas(area);
        assert_eq!(a, Rect::new(0, 0, 10, 1));
        let [a, b, c] = Layout::horizontal([Length(4)]).areas(area);
        assert_eq!(a, Rect::new(0, 0, 4, 4));
        assert_eq!((b, c), (Rect::new(10, 0, 0, 4), Rect::new(10, 0, 0, 4)));
        assert_eq!(
            Layout::vertical([Fill(1)]).areas::<1>(OVERHANG),
            [Rect::new(MAX - 2, MAX - 1, 2, 1)]
        );
    }

    #[test]
    fn conversions() {
        assert_eq!(Position::from((3, 4)), Position::new(3, 4));
        assert_eq!(<(u16, u16)>::from(Position::new(3, 4)), (3, 4));
        assert_eq!(Size::from((5, 6)), Size::new(5, 6));
        assert_eq!(<(u16, u16)>::from(Size::new(5, 6)), (5, 6));
        assert_eq!(
            Rect::from((Position::new(1, 2), Size::new(3, 4))),
            Rect::new(1, 2, 3, 4)
        );
        assert_eq!(Rect::from(Size::new(MAX, 2)), Rect::new(0, 0, MAX, 2));
    }
}
