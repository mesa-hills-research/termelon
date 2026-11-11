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

const NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "gray",
    "dark-gray",
    "light-red",
    "light-green",
    "light-yellow",
    "light-blue",
    "light-magenta",
    "light-cyan",
    "white",
];

/// xterm's default palette for the first sixteen colours.
const PALETTE16: [(u8, u8, u8); 16] = [
    (0x00, 0x00, 0x00),
    (0xcd, 0x00, 0x00),
    (0x00, 0xcd, 0x00),
    (0xcd, 0xcd, 0x00),
    (0x00, 0x00, 0xee),
    (0xcd, 0x00, 0xcd),
    (0x00, 0xcd, 0xcd),
    (0xe5, 0xe5, 0xe5),
    (0x7f, 0x7f, 0x7f),
    (0xff, 0x00, 0x00),
    (0x00, 0xff, 0x00),
    (0xff, 0xff, 0x00),
    (0x5c, 0x5c, 0xff),
    (0xff, 0x00, 0xff),
    (0x00, 0xff, 0xff),
    (0xff, 0xff, 0xff),
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

    /// How the styled-text format writes the colour, or `None` for the default.
    pub(crate) fn styled_name(self) -> Option<String> {
        match self {
            Color::Default => None,
            Color::Indexed(n) => Some(format!("idx:{n}")),
            Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
            named => named
                .named_index()
                .map(|i| NAMES[usize::from(i)].to_owned()),
        }
    }

    /// The colour as RGB in xterm's default palette, or `None` for the default.
    pub(crate) fn rgb(self) -> Option<(u8, u8, u8)> {
        match self {
            Color::Default => None,
            Color::Rgb(r, g, b) => Some((r, g, b)),
            Color::Indexed(n) => Some(palette256(n)),
            named => named.named_index().map(palette256),
        }
    }
}

/// xterm's default 256-colour palette.
fn palette256(n: u8) -> (u8, u8, u8) {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match n {
        0..=15 => PALETTE16[usize::from(n)],
        16..=231 => {
            let i = n - 16;
            (
                LEVELS[usize::from(i / 36)],
                LEVELS[usize::from((i / 6) % 6)],
                LEVELS[usize::from(i % 6)],
            )
        }
        _ => {
            let level = 8 + (n - 232) * 10;
            (level, level, level)
        }
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

    /// The attributes as the styled-text format writes them, in its order, empty for the default
    /// style.
    pub(crate) fn styled_attrs(&self) -> Vec<String> {
        let mut attrs = Vec::new();
        for (key, color) in [
            ("fg", self.fg),
            ("bg", self.bg),
            ("ul", self.underline_color),
        ] {
            if let Some(name) = color.styled_name() {
                attrs.push(format!("{key}={name}"));
            }
        }
        match self.underline {
            Underline::None => {}
            Underline::Single => attrs.push("underline".to_owned()),
            Underline::Double => attrs.push("underline=double".to_owned()),
            Underline::Curly => attrs.push("underline=curly".to_owned()),
            Underline::Dotted => attrs.push("underline=dotted".to_owned()),
            Underline::Dashed => attrs.push("underline=dashed".to_owned()),
        }
        for (on, name) in [
            (self.bold, "bold"),
            (self.dim, "dim"),
            (self.italic, "italic"),
            (self.blink, "blink"),
            (self.rapid_blink, "rapid-blink"),
            (self.reverse, "reverse"),
            (self.hidden, "hidden"),
            (self.strike, "strike"),
            (self.overline, "overline"),
        ] {
            if on {
                attrs.push(name.to_owned());
            }
        }
        if let Some(link) = &self.link {
            attrs.push(format!("link={}", link.url));
        }
        attrs
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

    #[test]
    fn palette_matches_xterm() {
        assert_eq!(palette256(9), (0xff, 0, 0));
        assert_eq!(palette256(16), (0, 0, 0));
        assert_eq!(palette256(196), (255, 0, 0));
        assert_eq!(palette256(231), (255, 255, 255));
        assert_eq!(palette256(232), (8, 8, 8));
        assert_eq!(palette256(255), (238, 238, 238));
    }

    #[test]
    fn styled_names() {
        assert_eq!(Color::DarkGray.styled_name().as_deref(), Some("dark-gray"));
        assert_eq!(Color::Gray.styled_name().as_deref(), Some("gray"));
        assert_eq!(Color::Indexed(8).styled_name().as_deref(), Some("idx:8"));
        assert_eq!(
            Color::Rgb(0x7f, 0x8e, 0xa3).styled_name().as_deref(),
            Some("#7f8ea3")
        );
        assert_eq!(Color::Default.styled_name(), None);
    }
}
