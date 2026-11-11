//! Colours and the attributes of a cell.

use std::sync::Arc;

/// A colour, as the program asked for it.
///
/// The sixteen named colours (`SGR 30` to `37` and `90` to `97`) stay apart from
/// [`Indexed`](Color::Indexed) (`SGR 38;5;n`), because programs write them differently and a terminal
/// with only sixteen colours may know only the first form. The names are ratatui's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Color {
    /// The terminal's default colour.
    #[default]
    Default,
    /// Palette 0.
    Black,
    /// Palette 1.
    Red,
    /// Palette 2.
    Green,
    /// Palette 3.
    Yellow,
    /// Palette 4.
    Blue,
    /// Palette 5.
    Magenta,
    /// Palette 6.
    Cyan,
    /// Palette 7.
    Gray,
    /// Palette 8.
    DarkGray,
    /// Palette 9.
    LightRed,
    /// Palette 10.
    LightGreen,
    /// Palette 11.
    LightYellow,
    /// Palette 12.
    LightBlue,
    /// Palette 13.
    LightMagenta,
    /// Palette 14.
    LightCyan,
    /// Palette 15.
    White,
    /// A colour of the 256-colour palette, from `38;5;n`.
    Indexed(u8),
    /// A true colour, from `38;2;r;g;b`.
    Rgb(u8, u8, u8),
}

const NAMED: [Color; 16] = [
    Color::Black,
    Color::Red,
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::Gray,
    Color::DarkGray,
    Color::LightRed,
    Color::LightGreen,
    Color::LightYellow,
    Color::LightBlue,
    Color::LightMagenta,
    Color::LightCyan,
    Color::White,
];

impl Color {
    /// The named colour for a palette index from 0 to 15.
    pub fn named(index: u8) -> Option<Color> {
        NAMED.get(usize::from(index)).copied()
    }

    /// The palette index of a named colour.
    pub fn named_index(self) -> Option<u8> {
        NAMED
            .iter()
            .position(|&c| c == self)
            .and_then(|i| u8::try_from(i).ok())
    }
}

/// The style of an underline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Underline {
    /// No underline.
    #[default]
    None,
    /// `SGR 4` or `4:1`.
    Single,
    /// `SGR 21` or `4:2`.
    Double,
    /// `SGR 4:3`.
    Curly,
    /// `SGR 4:4`.
    Dotted,
    /// `SGR 4:5`.
    Dashed,
}

/// A hyperlink from OSC 8.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Hyperlink {
    /// The address.
    pub url: String,
    /// The `id` parameter, which joins the cells of one link across rows.
    pub id: Option<String>,
}

/// The colours and attributes of a cell, and of the pen that draws them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Style {
    /// The foreground.
    pub fg: Color,
    /// The background.
    pub bg: Color,
    /// The underline's colour, from `SGR 58`.
    pub underline_color: Color,
    /// The underline's style.
    pub underline: Underline,
    /// `SGR 1`.
    pub bold: bool,
    /// `SGR 2`.
    pub dim: bool,
    /// `SGR 3`.
    pub italic: bool,
    /// `SGR 5`.
    pub blink: bool,
    /// `SGR 6`.
    pub rapid_blink: bool,
    /// `SGR 7`.
    pub reverse: bool,
    /// `SGR 8`.
    pub hidden: bool,
    /// `SGR 9`.
    pub strike: bool,
    /// `SGR 53`.
    pub overline: bool,
    /// The hyperlink the cell belongs to.
    pub link: Option<Arc<Hyperlink>>,
}

impl Style {
    /// Whether every colour is the default and no attribute or link is set.
    pub fn is_default(&self) -> bool {
        *self == Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_colours_round_trip_their_index() {
        for i in 0..16 {
            assert_eq!(Color::named(i).and_then(Color::named_index), Some(i));
        }
        assert_eq!(Color::named(16), None);
        assert_eq!(Color::Indexed(1).named_index(), None);
    }
}
