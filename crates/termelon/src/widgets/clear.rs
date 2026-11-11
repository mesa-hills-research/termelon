//! The [`Clear`] widget.

use crate::buffer::Buffer;
use crate::layout::Rect;

use super::Widget;

/// Blanks its area to the default cell, a space in the terminal's colours, for drawing a popup
/// over what's below.
///
/// A wide character that crosses the area's edge is blanked whole, as the terminal would show it
/// once half of it is overwritten.
///
/// ```
/// use termelon::buffer::Buffer;
/// use termelon::layout::Rect;
/// use termelon::style::{Color, Style};
/// use termelon::widgets::{Clear, Widget};
///
/// let mut buffer = Buffer::empty(Rect::new(0, 0, 6, 1));
/// buffer.set_string(0, 0, "abcdef", Style::new().bg(Color::Blue));
/// Clear.render(Rect::new(2, 0, 2, 1), &mut buffer);
/// let row: String = (0..6).map(|x| buffer.symbol((x, 0))).collect();
/// assert_eq!(row, "ab  ef");
/// assert_eq!(buffer.cell((2, 0)).map(|c| c.bg()), Some(Color::Reset));
/// ```
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Clear;

impl Widget for Clear {
    fn render(self, area: Rect, buf: &mut Buffer) {
        (&self).render(area, buf);
    }
}

impl Widget for &Clear {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let area = area.intersection(buf.area());
        for position in area.positions() {
            if let Some(mut cell) = buf.cell_mut(position) {
                cell.reset();
            }
        }
    }
}
