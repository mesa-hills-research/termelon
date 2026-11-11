//! The [`Block`] widget, its [`Borders`], [`BorderSet`]s and [`Padding`].

use crate::buffer::Buffer;
use crate::flags::flags;
use crate::layout::{Alignment, Position, Rect};
use crate::style::Style;
use crate::text::Line;

use super::{Widget, cells, line_width, render_line};

flags! {
    /// The sides of a [`Block`] that have a border, combined with `|`.
    ///
    /// ```
    /// use termelon::widgets::Borders;
    ///
    /// let open_below = Borders::ALL - Borders::BOTTOM;
    /// assert!(open_below.contains(Borders::TOP | Borders::LEFT | Borders::RIGHT));
    /// assert_eq!(Borders::default(), Borders::NONE);
    /// ```
    pub struct Borders(u8);
    empty = "NONE";
    /// The top side.
    const TOP = 1;
    /// The right side.
    const RIGHT = 1 << 1;
    /// The bottom side.
    const BOTTOM = 1 << 2;
    /// The left side.
    const LEFT = 1 << 3;
}

impl Borders {
    /// No side.
    pub const NONE: Self = Self::empty();
    /// Every side.
    pub const ALL: Self = Self::all();
}

/// The symbols a [`Block`] draws its border with, one for each corner and side.
///
/// The fields are ratatui's `symbols::border::Set`, so a program can read them to draw boxes of
/// its own, or build a set field by field.
///
/// ```
/// use termelon::widgets::BorderSet;
///
/// let top = format!(
///     "{}{}{}",
///     BorderSet::ROUNDED.top_left,
///     BorderSet::ROUNDED.horizontal_top.repeat(3),
///     BorderSet::ROUNDED.top_right,
/// );
/// assert_eq!(top, "╭───╮");
///
/// let dotted = BorderSet { horizontal_top: "·", horizontal_bottom: "·", ..BorderSet::PLAIN };
/// assert_eq!(dotted.vertical_left, "│");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BorderSet {
    /// The top-left corner.
    pub top_left: &'static str,
    /// The top-right corner.
    pub top_right: &'static str,
    /// The bottom-left corner.
    pub bottom_left: &'static str,
    /// The bottom-right corner.
    pub bottom_right: &'static str,
    /// The left side.
    pub vertical_left: &'static str,
    /// The right side.
    pub vertical_right: &'static str,
    /// The top side.
    pub horizontal_top: &'static str,
    /// The bottom side.
    pub horizontal_bottom: &'static str,
}

impl BorderSet {
    /// Thin lines with square corners, `┌─┐`.
    pub const PLAIN: Self = Self::lines(["┌", "┐", "└", "┘"], "│", "─");
    /// Thin lines with rounded corners, `╭─╮`.
    pub const ROUNDED: Self = Self::lines(["╭", "╮", "╰", "╯"], "│", "─");
    /// Heavy lines, `┏━┓`.
    pub const THICK: Self = Self::lines(["┏", "┓", "┗", "┛"], "┃", "━");
    /// Double lines, `╔═╗`.
    pub const DOUBLE: Self = Self::lines(["╔", "╗", "╚", "╝"], "║", "═");
    /// Plus signs, bars and dashes, `+-+`, for fonts without box drawing.
    pub const ASCII: Self = Self::lines(["+", "+", "+", "+"], "|", "-");

    /// The set with `corners` (top left, top right, bottom left, bottom right) and the same
    /// symbol on both sides of each direction.
    const fn lines(
        corners: [&'static str; 4],
        vertical: &'static str,
        horizontal: &'static str,
    ) -> Self {
        let [top_left, top_right, bottom_left, bottom_right] = corners;
        Self {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
            vertical_left: vertical,
            vertical_right: vertical,
            horizontal_top: horizontal,
            horizontal_bottom: horizontal,
        }
    }
}

impl Default for BorderSet {
    /// [`BorderSet::PLAIN`].
    fn default() -> Self {
        Self::PLAIN
    }
}

/// The empty cells a [`Block`] keeps between its border and its inside.
///
/// ```
/// use termelon::layout::Rect;
/// use termelon::widgets::{Block, Borders, Padding};
///
/// let block = Block::bordered().padding(Padding::horizontal(1));
/// assert_eq!(block.inner(Rect::new(0, 0, 10, 4)), Rect::new(2, 1, 6, 2));
/// ```
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Padding {
    /// Columns on the left.
    pub left: u16,
    /// Columns on the right.
    pub right: u16,
    /// Rows at the top.
    pub top: u16,
    /// Rows at the bottom.
    pub bottom: u16,
}

impl Padding {
    /// No padding.
    pub const ZERO: Self = Self::new(0, 0, 0, 0);

    /// Padding of `left`, `right`, `top` and `bottom` cells.
    pub const fn new(left: u16, right: u16, top: u16, bottom: u16) -> Self {
        Self {
            left,
            right,
            top,
            bottom,
        }
    }

    /// `value` cells on every side.
    pub const fn uniform(value: u16) -> Self {
        Self::new(value, value, value, value)
    }

    /// `value` columns on the left and on the right.
    pub const fn horizontal(value: u16) -> Self {
        Self::new(value, value, 0, 0)
    }

    /// `value` rows at the top and at the bottom.
    pub const fn vertical(value: u16) -> Self {
        Self::new(0, 0, value, value)
    }

    /// `x` columns on the left and on the right, and `y` rows at the top and at the bottom.
    pub const fn symmetric(x: u16, y: u16) -> Self {
        Self::new(x, x, y, y)
    }
}

/// The side of a [`Block`] a title sits on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TitlePosition {
    /// On the top border.
    #[default]
    Top,
    /// On the bottom border.
    Bottom,
}

/// A box drawn around an area, with borders, titles on them, a background style and padding.
///
/// The builder is ratatui's. The block fills its area with its style, draws the sides it has
/// with the [`BorderSet`] in the border style, then writes its titles on the top and bottom rows.
/// A title sits after the left border, or in the middle or at the right as its own alignment or
/// [`title_alignment`](Self::title_alignment) says. Titles at the same place are drawn one after
/// another with a space between them, and a title too long for the row is cut. [`inner`](Self::inner)
/// gives the area left inside, for what the block holds.
///
/// ```
/// use termelon::buffer::Buffer;
/// use termelon::layout::Rect;
/// use termelon::style::{Color, Modifier, Style};
/// use termelon::text::{Line, Span};
/// use termelon::widgets::{Block, BorderSet, Borders, Widget};
///
/// let area = Rect::new(0, 0, 14, 3);
/// let block = Block::default()
///     .borders(Borders::ALL)
///     .border_set(BorderSet::ROUNDED)
///     .border_style(Style::new().fg(Color::DarkGray))
///     .style(Style::new().bg(Color::Black))
///     .title(Line::from(Span::styled(" Models ", Style::new().add_modifier(Modifier::BOLD))))
///     .title_bottom(Line::from(" esc ").right_aligned());
/// assert_eq!(block.inner(area), Rect::new(1, 1, 12, 1));
///
/// let mut buffer = Buffer::empty(area);
/// block.render(area, &mut buffer);
/// let row = |y| (0..14).map(|x| buffer.symbol((x, y))).collect::<String>();
/// assert_eq!(row(0), "╭ Models ────╮");
/// assert_eq!(row(1), "│            │");
/// assert_eq!(row(2), "╰─────── esc ╯");
/// ```
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Block<'a> {
    titles: Vec<(Option<TitlePosition>, Line<'a>)>,
    titles_style: Style,
    titles_alignment: Alignment,
    titles_position: TitlePosition,
    borders: Borders,
    border_style: Style,
    border_set: BorderSet,
    style: Style,
    padding: Padding,
}

impl<'a> Block<'a> {
    /// A block with no borders, titles or padding.
    pub const fn new() -> Self {
        Self {
            titles: Vec::new(),
            titles_style: Style::new(),
            titles_alignment: Alignment::Left,
            titles_position: TitlePosition::Top,
            borders: Borders::NONE,
            border_style: Style::new(),
            border_set: BorderSet::PLAIN,
            style: Style::new(),
            padding: Padding::ZERO,
        }
    }

    /// A block with a border on every side.
    pub const fn bordered() -> Self {
        Self::new().borders(Borders::ALL)
    }

    /// The block with `title` added at the default position, the top unless
    /// [`title_position`](Self::title_position) says otherwise.
    #[must_use]
    pub fn title<T: Into<Line<'a>>>(mut self, title: T) -> Self {
        self.titles.push((None, title.into()));
        self
    }

    /// The block with `title` added on the top border.
    #[must_use]
    pub fn title_top<T: Into<Line<'a>>>(mut self, title: T) -> Self {
        self.titles.push((Some(TitlePosition::Top), title.into()));
        self
    }

    /// The block with `title` added on the bottom border.
    #[must_use]
    pub fn title_bottom<T: Into<Line<'a>>>(mut self, title: T) -> Self {
        self.titles
            .push((Some(TitlePosition::Bottom), title.into()));
        self
    }

    /// The block with `style` under every title, below the titles' own styles.
    #[must_use]
    pub fn title_style<S: Into<Style>>(mut self, style: S) -> Self {
        self.titles_style = style.into();
        self
    }

    /// The block with titles that have no alignment of their own placed as `alignment` says.
    #[must_use]
    pub const fn title_alignment(mut self, alignment: Alignment) -> Self {
        self.titles_alignment = alignment;
        self
    }

    /// The block with titles added by [`title`](Self::title) placed at `position`.
    #[must_use]
    pub const fn title_position(mut self, position: TitlePosition) -> Self {
        self.titles_position = position;
        self
    }

    /// The block with its borders drawn in `style`.
    #[must_use]
    pub fn border_style<S: Into<Style>>(mut self, style: S) -> Self {
        self.border_style = style.into();
        self
    }

    /// The block with `style` over its whole area, under the borders and titles and whatever is
    /// drawn inside.
    #[must_use]
    pub fn style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = style.into();
        self
    }

    /// The block with borders on the sides `borders` has.
    #[must_use]
    pub const fn borders(mut self, borders: Borders) -> Self {
        self.borders = borders;
        self
    }

    /// The block with its borders drawn with `set`.
    #[must_use]
    pub const fn border_set(mut self, set: BorderSet) -> Self {
        self.border_set = set;
        self
    }

    /// The block with `padding` inside its borders.
    #[must_use]
    pub const fn padding(mut self, padding: Padding) -> Self {
        self.padding = padding;
        self
    }

    /// The area inside the block drawn over `area`, without its borders, the rows its titles take
    /// and its padding. A title row counts even on a side with no border.
    #[must_use]
    pub fn inner(&self, area: Rect) -> Rect {
        let mut inner = area;
        if self.borders.contains(Borders::LEFT) {
            inner.x = inner.x.saturating_add(1).min(inner.right());
            inner.width = inner.width.saturating_sub(1);
        }
        if self.borders.contains(Borders::TOP) || self.has_titles(TitlePosition::Top) {
            inner.y = inner.y.saturating_add(1).min(inner.bottom());
            inner.height = inner.height.saturating_sub(1);
        }
        if self.borders.contains(Borders::RIGHT) {
            inner.width = inner.width.saturating_sub(1);
        }
        if self.borders.contains(Borders::BOTTOM) || self.has_titles(TitlePosition::Bottom) {
            inner.height = inner.height.saturating_sub(1);
        }
        let padding = self.padding;
        inner.x = inner.x.saturating_add(padding.left);
        inner.y = inner.y.saturating_add(padding.top);
        inner.width = inner
            .width
            .saturating_sub(padding.left.saturating_add(padding.right));
        inner.height = inner
            .height
            .saturating_sub(padding.top.saturating_add(padding.bottom));
        inner
    }

    fn has_titles(&self, position: TitlePosition) -> bool {
        self.titles
            .iter()
            .any(|(at, _)| at.unwrap_or(self.titles_position) == position)
    }

    /// The titles at `position` placed as `alignment` says, in the order they were added.
    fn titles(
        &self,
        position: TitlePosition,
        alignment: Alignment,
    ) -> impl DoubleEndedIterator<Item = &Line<'a>> {
        self.titles
            .iter()
            .filter(move |(at, line)| {
                at.unwrap_or(self.titles_position) == position
                    && line.alignment.unwrap_or(self.titles_alignment) == alignment
            })
            .map(|(_, line)| line)
    }

    /// Draws the sides, each the length of the area, and then the corners over them.
    fn render_borders(&self, area: Rect, buf: &mut Buffer) {
        let set = &self.border_set;
        let right = area.right() - 1;
        let bottom = area.bottom() - 1;
        let sides = [
            (Borders::LEFT, Rect { width: 1, ..area }, set.vertical_left),
            (Borders::TOP, Rect { height: 1, ..area }, set.horizontal_top),
            (
                Borders::RIGHT,
                Rect {
                    x: right,
                    width: 1,
                    ..area
                },
                set.vertical_right,
            ),
            (
                Borders::BOTTOM,
                Rect {
                    y: bottom,
                    height: 1,
                    ..area
                },
                set.horizontal_bottom,
            ),
        ];
        for (side, rect, symbol) in sides {
            if self.borders.contains(side) {
                for position in rect.positions() {
                    self.put_border(buf, position, symbol);
                }
            }
        }
        let corners = [
            (
                Borders::RIGHT | Borders::BOTTOM,
                right,
                bottom,
                set.bottom_right,
            ),
            (Borders::RIGHT | Borders::TOP, right, area.y, set.top_right),
            (
                Borders::LEFT | Borders::BOTTOM,
                area.x,
                bottom,
                set.bottom_left,
            ),
            (Borders::LEFT | Borders::TOP, area.x, area.y, set.top_left),
        ];
        for (sides, x, y, symbol) in corners {
            if self.borders.contains(sides) {
                self.put_border(buf, Position::new(x, y), symbol);
            }
        }
    }

    fn put_border(&self, buf: &mut Buffer, position: Position, symbol: &str) {
        if let Some(mut cell) = buf.cell_mut(position) {
            cell.set_symbol(symbol).set_style(self.border_style);
        }
    }

    /// The row titles at `position` go in, between the left and right borders.
    fn titles_area(&self, area: Rect, position: TitlePosition) -> Rect {
        let left = u16::from(self.borders.contains(Borders::LEFT));
        let right = u16::from(self.borders.contains(Borders::RIGHT));
        Rect {
            x: area.x.saturating_add(left),
            y: match position {
                TitlePosition::Top => area.y,
                TitlePosition::Bottom => area.bottom().saturating_sub(1),
            },
            width: area.width.saturating_sub(left).saturating_sub(right),
            height: 1,
        }
    }

    fn render_title(
        &self,
        title: &Line<'_>,
        area: Rect,
        buf: &mut Buffer,
        alignment: Option<Alignment>,
    ) {
        buf.set_style(area, self.titles_style);
        render_line(title, area, buf, alignment);
    }

    /// Writes the left titles from the left, each cut at the room left.
    fn render_left_titles(&self, position: TitlePosition, area: Rect, buf: &mut Buffer) {
        let profile = buf.width_profile();
        let mut room = self.titles_area(area, position);
        for title in self.titles(position, Alignment::Left) {
            if room.is_empty() {
                break;
            }
            let width = cells(line_width(title, profile));
            let at = Rect {
                width: width.min(room.width),
                ..room
            };
            self.render_title(title, at, buf, title.alignment);
            let advance = width.saturating_add(1);
            room.x = room.x.saturating_add(advance);
            room.width = room.width.saturating_sub(advance);
        }
    }

    /// Writes the centred titles side by side in the middle. When they don't fit, the row shows
    /// their middle.
    fn render_center_titles(&self, position: TitlePosition, area: Rect, buf: &mut Buffer) {
        let profile = buf.width_profile();
        let mut room = self.titles_area(area, position);
        let titles: Vec<(&Line<'_>, u16)> = self
            .titles(position, Alignment::Center)
            .map(|title| (title, cells(line_width(title, profile))))
            .collect();
        let total = titles
            .iter()
            .map(|(_, width)| width.saturating_add(1))
            .fold(0, u16::saturating_add)
            .saturating_sub(1);
        if total <= room.width {
            room.x = room.x.saturating_add((room.width - total) / 2);
            for (title, width) in titles {
                let at = Rect { width, ..room };
                self.render_title(title, at, buf, title.alignment);
                let advance = width.saturating_add(1);
                room.x = room.x.saturating_add(advance);
                room.width = room.width.saturating_sub(advance);
            }
            return;
        }
        let mut cut = (total - room.width) / 2;
        for (title, width) in titles {
            if room.is_empty() {
                break;
            }
            let shown = room.width.min(width).saturating_sub(cut);
            let at = Rect {
                width: shown,
                ..room
            };
            if cut > 0 {
                self.render_title(title, at, buf, Some(Alignment::Right));
                cut = cut.saturating_sub(shown).saturating_sub(1);
            } else {
                self.render_title(title, at, buf, Some(Alignment::Left));
            }
            let advance = shown.saturating_add(1);
            room.x = room.x.saturating_add(advance);
            room.width = room.width.saturating_sub(advance);
        }
    }

    /// Writes the right titles from the right, the last added rightmost, each cut at the room
    /// left.
    fn render_right_titles(&self, position: TitlePosition, area: Rect, buf: &mut Buffer) {
        let profile = buf.width_profile();
        let mut room = self.titles_area(area, position);
        for title in self.titles(position, Alignment::Right).rev() {
            if room.is_empty() {
                break;
            }
            let width = cells(line_width(title, profile));
            let at = Rect {
                x: room.right().saturating_sub(width).max(room.x),
                width: width.min(room.width),
                ..room
            };
            self.render_title(title, at, buf, title.alignment);
            room.width = room.width.saturating_sub(width).saturating_sub(1);
        }
    }
}

impl Widget for Block<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        (&self).render(area, buf);
    }
}

impl Widget for &Block<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = area.intersection(buf.area());
        if area.is_empty() {
            return;
        }
        buf.set_style(area, self.style);
        self.render_borders(area, buf);
        for position in [TitlePosition::Top, TitlePosition::Bottom] {
            self.render_left_titles(position, area, buf);
            self.render_center_titles(position, area, buf);
            self.render_right_titles(position, area, buf);
        }
    }
}
