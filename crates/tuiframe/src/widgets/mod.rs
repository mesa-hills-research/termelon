//! Widgets, which draw themselves into an area of a [`Buffer`].
//!
//! [`Widget`] has ratatui's shape, so a widget written for one ports to the other by changing its
//! imports. Text is a widget (`&str`, `String`, [`Span`], [`Line`] and [`Text`]), and so are
//! [`Clear`], [`Block`] and [`Paragraph`]. Each draws only inside its area and the buffer, and
//! measures text with the buffer's [`WidthProfile`].
//!
//! ```
//! use tuiframe::buffer::Buffer;
//! use tuiframe::layout::Rect;
//! use tuiframe::widgets::{Block, Borders, Clear, Widget};
//!
//! let area = Rect::new(0, 0, 12, 3);
//! let mut buffer = Buffer::empty(area);
//! Clear.render(area, &mut buffer);
//! let block = Block::default().borders(Borders::ALL).title(" Notes ");
//! let inner = block.inner(area);
//! block.render(area, &mut buffer);
//! "Saved".render(inner, &mut buffer);
//!
//! let row = |y| (0..12).map(|x| buffer.symbol((x, y))).collect::<String>();
//! assert_eq!(row(0), "┌ Notes ───┐");
//! assert_eq!(row(1), "│Saved     │");
//! assert_eq!(row(2), "└──────────┘");
//! ```

mod block;
mod clear;
mod paragraph;
#[cfg(test)]
mod tests;

pub use block::{Block, BorderSet, Borders, Padding, TitlePosition};
pub use clear::Clear;
pub use paragraph::{Paragraph, Wrap};

use crate::buffer::Buffer;
use crate::caps::WidthProfile;
use crate::layout::{Alignment, Rect};
use crate::style::Style;
use crate::text::{self, Line, Span, Text};

/// Something that draws itself into an area of a buffer.
///
/// A widget is usually built for one frame and consumed by drawing it. Widgets that are worth
/// keeping implement it for a reference too, as [`Block`] and [`Paragraph`] do.
///
/// ```
/// use tuiframe::buffer::Buffer;
/// use tuiframe::layout::Rect;
/// use tuiframe::style::{Color, Style};
/// use tuiframe::text::Span;
/// use tuiframe::widgets::Widget;
///
/// struct Badge(&'static str);
///
/// impl Widget for Badge {
///     fn render(self, area: Rect, buf: &mut Buffer) {
///         Span::styled(self.0, Style::new().fg(Color::Black).bg(Color::Cyan)).render(area, buf);
///     }
/// }
///
/// let mut buffer = Buffer::empty(Rect::new(0, 0, 4, 1));
/// Badge("new").render(buffer.area(), &mut buffer);
/// assert_eq!(buffer.symbol((0, 0)), "n");
/// assert_eq!(buffer.cell((0, 0)).map(|c| c.bg()), Some(Color::Cyan));
/// ```
pub trait Widget {
    /// Draws into `area` of `buf`. Whatever falls outside the buffer is left out.
    fn render(self, area: Rect, buf: &mut Buffer);
}

/// Writes the string on the area's first row, cut at the area's width.
impl Widget for &str {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if !area.is_empty() {
            buf.set_stringn(area.x, area.y, self, usize::from(area.width), Style::new());
        }
    }
}

/// Writes the string on the area's first row, cut at the area's width.
impl Widget for String {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.as_str().render(area, buf);
    }
}

/// Writes the string on the area's first row, cut at the area's width.
impl Widget for &String {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.as_str().render(area, buf);
    }
}

/// Writes the span on the area's first row in its style, cut at the area's width.
impl Widget for Span<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        (&self).render(area, buf);
    }
}

/// Writes the span on the area's first row in its style, cut at the area's width.
impl Widget for &Span<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = area.intersection(buf.area());
        if !area.is_empty() {
            buf.set_span(area.x, area.y, self, area.width);
        }
    }
}

/// Writes the line on the area's first row, which takes the line's style, placed by its alignment.
/// A line too long for the area loses its end, or for a centred or right-aligned line, its ends or
/// its start.
impl Widget for Line<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        (&self).render(area, buf);
    }
}

/// Writes the line on the area's first row, which takes the line's style, placed by its alignment.
/// A line too long for the area loses its end, or for a centred or right-aligned line, its ends or
/// its start.
impl Widget for &Line<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        render_line(self, area, buf, self.alignment);
    }
}

/// Writes a line on each row of the area from the top, the whole area taking the text's style.
/// Lines below the area are left out, and each is cut at the area's width as a [`Line`] is.
impl Widget for Text<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        (&self).render(area, buf);
    }
}

/// Writes a line on each row of the area from the top, the whole area taking the text's style.
/// Lines below the area are left out, and each is cut at the area's width as a [`Line`] is.
impl Widget for &Text<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = area.intersection(buf.area());
        buf.set_style(area, self.style);
        for (line, row) in self.lines.iter().zip(area.rows()) {
            render_line(line, row, buf, line.alignment.or(self.alignment));
        }
    }
}

/// Draws the widget when there is one.
impl<W: Widget> Widget for Option<W> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if let Some(widget) = self {
            widget.render(area, buf);
        }
    }
}

/// The cells `line` takes under `profile`.
fn line_width(line: &Line<'_>, profile: WidthProfile) -> usize {
    line.spans
        .iter()
        .map(|span| text::width_with(&span.content, profile))
        .sum()
}

/// A `usize` count of cells as a `u16`, at most `u16::MAX`.
fn cells(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// `area` with its first `n` columns taken off.
fn indent(area: Rect, n: usize) -> Rect {
    let n = cells(n);
    Rect {
        x: area.x.saturating_add(n),
        width: area.width.saturating_sub(n),
        ..area
    }
}

/// Writes `line` on the first row of `area`, placed as `alignment` says, as ratatui's line widget
/// does.
///
/// The whole row takes the line's style, unless the line takes no cells. A line that fits is
/// indented to its place. One that doesn't is cut, from the right for the left alignment, from
/// the left for the right, and from both sides for the centre, and a wide character that the cut
/// crosses on the left leaves its cells blank.
fn render_line(line: &Line<'_>, area: Rect, buf: &mut Buffer, alignment: Option<Alignment>) {
    let area = area.intersection(buf.area());
    if area.is_empty() {
        return;
    }
    let area = Rect { height: 1, ..area };
    let profile = buf.width_profile();
    let width = line_width(line, profile);
    if width == 0 {
        return;
    }
    buf.set_style(area, line.style);
    let room = usize::from(area.width);
    if width <= room {
        let lead = match alignment {
            Some(Alignment::Center) => (room - width) / 2,
            Some(Alignment::Right) => room - width,
            Some(Alignment::Left) | None => 0,
        };
        render_spans(&line.spans, indent(area, lead), buf, 0);
    } else {
        let cut = match alignment {
            Some(Alignment::Center) => (width - room) / 2,
            Some(Alignment::Right) => width - room,
            Some(Alignment::Left) | None => 0,
        };
        render_spans(&line.spans, area, buf, cut);
    }
}

/// Writes `spans` from the left of `area`, leaving out their first `cut` cells.
fn render_spans(spans: &[Span<'_>], mut area: Rect, buf: &mut Buffer, mut cut: usize) {
    let profile = buf.width_profile();
    for span in spans {
        let width = text::width_with(&span.content, profile);
        if cut >= width {
            cut -= width;
            continue;
        }
        let (content, shown, lead) = if cut == 0 {
            (span.content.as_ref(), width, 0)
        } else {
            let room = width - cut;
            let (rest, shown) = cut_start(&span.content, room, profile);
            (rest, shown, room - shown)
        };
        cut = 0;
        area = indent(area, lead);
        if area.is_empty() {
            break;
        }
        buf.set_stringn(area.x, area.y, content, usize::from(area.width), span.style);
        area = indent(area, shown);
    }
}

/// The longest end of `s` that takes at most `room` cells, and the cells it takes.
fn cut_start(s: &str, room: usize, profile: WidthProfile) -> (&str, usize) {
    let mut start = s.len();
    let mut taken = 0;
    for (at, cluster) in unicode_segmentation::UnicodeSegmentation::grapheme_indices(s, true).rev()
    {
        let width = text::width_with(cluster, profile);
        if taken + width > room {
            break;
        }
        taken += width;
        start = at;
    }
    (s.get(start..).unwrap_or_default(), taken)
}
