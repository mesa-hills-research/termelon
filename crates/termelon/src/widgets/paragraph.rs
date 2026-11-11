//! The [`Paragraph`] widget.

use crate::buffer::Buffer;
use crate::layout::{Alignment, Position, Rect};
use crate::style::Style;
use crate::text::{self, Line, Text};

use super::{Block, Widget};

/// How a [`Paragraph`] wraps lines longer than its width, by words as [`text::wrap`] does.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Wrap {
    /// Whether rows lose the spaces they start with.
    pub trim: bool,
}

/// Text in an area, wrapped or cut at its width, aligned, and scrolled.
///
/// The builder and the rendering are ratatui's. Without [`wrap`](Self::wrap), each line takes
/// one row and is cut at the area's width. With it, lines break between words, as
/// [`text::wrap`] breaks them. [`scroll`](Self::scroll) skips rows at the top, and without
/// wrapping, cells at the left of left-aligned lines. The paragraph's style covers the area, and
/// the text's, lines' and spans' styles go over it in that order on the cells they write.
///
/// ```
/// use termelon::buffer::Buffer;
/// use termelon::layout::{Alignment, Rect};
/// use termelon::widgets::{Paragraph, Widget, Wrap};
///
/// let area = Rect::new(0, 0, 11, 3);
/// let mut buffer = Buffer::empty(area);
/// Paragraph::new("The quick brown fox jumps")
///     .wrap(Wrap { trim: true })
///     .alignment(Alignment::Center)
///     .render(area, &mut buffer);
/// let row = |y| (0..11).map(|x| buffer.symbol((x, y))).collect::<String>();
/// assert_eq!(row(0), " The quick ");
/// assert_eq!(row(1), " brown fox ");
/// assert_eq!(row(2), "   jumps   ");
/// ```
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Paragraph<'a> {
    block: Option<Block<'a>>,
    style: Style,
    wrap: Option<Wrap>,
    text: Text<'a>,
    scroll: Position,
    alignment: Alignment,
}

impl<'a> Paragraph<'a> {
    /// A paragraph of `text`, aligned as the text says, at the left when it doesn't.
    pub fn new<T: Into<Text<'a>>>(text: T) -> Self {
        let text = text.into();
        Self {
            alignment: text.alignment.unwrap_or_default(),
            text,
            ..Self::default()
        }
    }

    /// The paragraph drawn inside `block`.
    #[must_use]
    pub fn block(mut self, block: Block<'a>) -> Self {
        self.block = Some(block);
        self
    }

    /// The paragraph with `style` over its area, under the text.
    #[must_use]
    pub fn style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = style.into();
        self
    }

    /// The paragraph with lines wrapped by words.
    #[must_use]
    pub const fn wrap(mut self, wrap: Wrap) -> Self {
        self.wrap = Some(wrap);
        self
    }

    /// The paragraph scrolled down by `y` rows and, without wrapping, right by `x` cells, as
    /// ratatui's `(y, x)`.
    #[must_use]
    pub const fn scroll(mut self, (y, x): (u16, u16)) -> Self {
        self.scroll = Position::new(x, y);
        self
    }

    /// The paragraph with lines that have no alignment of their own placed as `alignment` says.
    #[must_use]
    pub const fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = alignment;
        self
    }

    /// The paragraph placed at the left.
    #[must_use]
    pub const fn left_aligned(self) -> Self {
        self.alignment(Alignment::Left)
    }

    /// The paragraph placed in the middle.
    #[must_use]
    pub const fn centered(self) -> Self {
        self.alignment(Alignment::Center)
    }

    /// The paragraph placed at the right.
    #[must_use]
    pub const fn right_aligned(self) -> Self {
        self.alignment(Alignment::Right)
    }

    /// Writes the text into `area`, the inside of the block.
    fn render_text(&self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }
        buf.set_style(area, self.style);
        let profile = buf.width_profile();
        let mut pieces = Vec::new();
        let mut rows = Vec::new();
        let mut skip = self.scroll.y;
        let mut y = area.y;
        for line in &self.text.lines {
            if y >= area.bottom() {
                break;
            }
            let alignment = line.alignment.unwrap_or(self.alignment);
            pieces.clear();
            rows.clear();
            if let Some(Wrap { trim }) = self.wrap {
                text::wrap::pieces(line, area.width, profile, &mut pieces);
                text::wrap::word_rows(&pieces, area.width, trim, &mut rows);
            } else if skip > 0 {
                skip -= 1;
                continue;
            } else {
                let offset = if alignment == Alignment::Left {
                    self.scroll.x
                } else {
                    0
                };
                text::wrap::pieces(line, area.width, profile, &mut pieces);
                rows.push(text::wrap::truncated_row(&pieces, area.width, offset));
            }
            for row in &rows {
                if skip > 0 {
                    skip -= 1;
                    continue;
                }
                if y >= area.bottom() {
                    break;
                }
                let lead = match alignment {
                    Alignment::Left => 0,
                    Alignment::Center => (area.width / 2).saturating_sub(row.width / 2),
                    Alignment::Right => area.width.saturating_sub(row.width),
                };
                let row = pieces.get(row.start..row.end).unwrap_or_default();
                render_row(
                    line,
                    row,
                    area.x.saturating_add(lead),
                    y,
                    self.text.style,
                    buf,
                );
                y += 1;
            }
        }
    }
}

/// Writes the clusters of `row`, pieces of `line`, from column `x` of row `y`, each in the
/// text's, line's and span's styles.
fn render_row(
    line: &Line<'_>,
    row: &[text::wrap::Piece<'_>],
    mut x: u16,
    y: u16,
    text_style: Style,
    buf: &mut Buffer,
) {
    let style = text_style.patch(line.style);
    for piece in row {
        if piece.width == 0 {
            continue;
        }
        let span = line.spans.get(piece.span).map_or(Style::new(), |s| s.style);
        buf.set_stringn(
            x,
            y,
            piece.symbol,
            usize::from(piece.width),
            style.patch(span),
        );
        x = x.saturating_add(piece.width);
    }
}

impl Widget for Paragraph<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        (&self).render(area, buf);
    }
}

impl Widget for &Paragraph<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = area.intersection(buf.area());
        buf.set_style(area, self.style);
        let inner = match &self.block {
            Some(block) => {
                block.render(area, buf);
                block.inner(area)
            }
            None => area,
        };
        self.render_text(inner, buf);
    }
}
