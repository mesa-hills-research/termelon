//! Colours, text modifiers, underline styles and the [`Style`] that combines them.
//!
//! The names and semantics are ratatui's, so code that builds styles with ratatui ports by
//! changing its imports.
//!
//! ```
//! use tuiframe::style::{Color, Modifier, Style};
//!
//! const HEADING: Style = Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD);
//! let plain = HEADING.remove_modifier(Modifier::BOLD);
//! assert_eq!(plain.fg, Some(Color::Cyan));
//! assert_eq!(plain.sub_modifier, Modifier::BOLD);
//! ```

use crate::flags::flags;

/// A colour for text, its background or its underline.
///
/// The sixteen named colours stay apart from `Indexed(0..=15)`, because they're written with
/// different codes (`SGR 31` against `38;5;1`), and a terminal with only 16 colours may know only
/// the first. The encoder lowers each colour to the terminal's depth, so a program paints in the
/// colours it means.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Color {
    /// The terminal's default colour, `SGR 39`, `49` or `59`.
    #[default]
    Reset,
    /// ANSI black, `SGR 30`.
    Black,
    /// ANSI red, `SGR 31`.
    Red,
    /// ANSI green, `SGR 32`.
    Green,
    /// ANSI yellow, `SGR 33`.
    Yellow,
    /// ANSI blue, `SGR 34`.
    Blue,
    /// ANSI magenta, `SGR 35`.
    Magenta,
    /// ANSI cyan, `SGR 36`.
    Cyan,
    /// ANSI white, which most terminals show as light grey, `SGR 37`.
    Gray,
    /// ANSI bright black, which most terminals show as dark grey, `SGR 90`.
    DarkGray,
    /// ANSI bright red, `SGR 91`.
    LightRed,
    /// ANSI bright green, `SGR 92`.
    LightGreen,
    /// ANSI bright yellow, `SGR 93`.
    LightYellow,
    /// ANSI bright blue, `SGR 94`.
    LightBlue,
    /// ANSI bright magenta, `SGR 95`.
    LightMagenta,
    /// ANSI bright cyan, `SGR 96`.
    LightCyan,
    /// ANSI bright white, `SGR 97`.
    White,
    /// An entry of the 256-colour palette, `SGR 38;5;n`.
    Indexed(u8),
    /// A true colour, `SGR 38;2;r;g;b`.
    Rgb(u8, u8, u8),
}

flags! {
    /// Text attributes such as bold and italic, combined with `|`.
    ///
    /// ```
    /// use tuiframe::style::Modifier;
    ///
    /// let mut m = Modifier::BOLD | Modifier::ITALIC;
    /// m -= Modifier::BOLD;
    /// assert_eq!(m, Modifier::ITALIC);
    /// assert!(m.intersects(Modifier::ITALIC | Modifier::DIM));
    /// ```
    pub struct Modifier(u16);
    empty = "empty";
    /// Bold or increased intensity, `SGR 1`.
    const BOLD = 1;
    /// Faint or decreased intensity, `SGR 2`.
    const DIM = 1 << 1;
    /// Italic, `SGR 3`.
    const ITALIC = 1 << 2;
    /// Underlined, `SGR 4`, in the shape [`Style::underline_style`] gives.
    const UNDERLINED = 1 << 3;
    /// Slow blink, `SGR 5`.
    const SLOW_BLINK = 1 << 4;
    /// Rapid blink, `SGR 6`.
    const RAPID_BLINK = 1 << 5;
    /// Foreground and background swapped, `SGR 7`.
    const REVERSED = 1 << 6;
    /// Invisible text, `SGR 8`.
    const HIDDEN = 1 << 7;
    /// Struck through, `SGR 9`.
    const CROSSED_OUT = 1 << 8;
    /// A line above the text, `SGR 53`.
    const OVERLINED = 1 << 9;
}

/// The shape of an underline, sent as `SGR 4:n` where the terminal knows the form and as a single
/// underline elsewhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UnderlineStyle {
    /// One straight line, `4:1`.
    Single,
    /// Two straight lines, `4:2`.
    Double,
    /// A wavy line, `4:3`.
    Curly,
    /// A dotted line, `4:4`.
    Dotted,
    /// A dashed line, `4:5`.
    Dashed,
}

/// A change of style, applied on top of what a cell already has.
///
/// A field that is `None` keeps what's below, `add_modifier` turns modifiers on and `sub_modifier`
/// turns them off. Applying styles one after another gives the same result as applying their
/// [`patch`](Self::patch).
///
/// The underline shows where [`Modifier::UNDERLINED`] is on, in the shape `underline_style` gives
/// (single when no style in the stack sets one) and the colour `underline_color` gives (the text's
/// colour when none does).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Style {
    /// The text's colour.
    pub fg: Option<Color>,
    /// The background colour.
    pub bg: Option<Color>,
    /// The underline's colour.
    pub underline_color: Option<Color>,
    /// The underline's shape.
    pub underline_style: Option<UnderlineStyle>,
    /// The modifiers this style turns on.
    pub add_modifier: Modifier,
    /// The modifiers this style turns off.
    pub sub_modifier: Modifier,
}

impl Style {
    /// A style that changes nothing.
    pub const fn new() -> Self {
        Self {
            fg: None,
            bg: None,
            underline_color: None,
            underline_style: None,
            add_modifier: Modifier::empty(),
            sub_modifier: Modifier::empty(),
        }
    }

    /// Sets the text's colour.
    #[must_use]
    pub const fn fg(mut self, color: Color) -> Self {
        self.fg = Some(color);
        self
    }

    /// Sets the background colour.
    #[must_use]
    pub const fn bg(mut self, color: Color) -> Self {
        self.bg = Some(color);
        self
    }

    /// Sets the underline's colour.
    #[must_use]
    pub const fn underline_color(mut self, color: Color) -> Self {
        self.underline_color = Some(color);
        self
    }

    /// Sets the underline's shape. The underline itself is [`Modifier::UNDERLINED`].
    #[must_use]
    pub const fn underline_style(mut self, style: UnderlineStyle) -> Self {
        self.underline_style = Some(style);
        self
    }

    /// Turns `modifier` on, undoing an earlier [`remove_modifier`](Self::remove_modifier) of it.
    #[must_use]
    pub const fn add_modifier(mut self, modifier: Modifier) -> Self {
        self.sub_modifier = self.sub_modifier.difference(modifier);
        self.add_modifier = self.add_modifier.union(modifier);
        self
    }

    /// Turns `modifier` off, undoing an earlier [`add_modifier`](Self::add_modifier) of it.
    #[must_use]
    pub const fn remove_modifier(mut self, modifier: Modifier) -> Self {
        self.add_modifier = self.add_modifier.difference(modifier);
        self.sub_modifier = self.sub_modifier.union(modifier);
        self
    }

    /// The style that applying `self` and then `other` gives. What `other` sets wins, and what it
    /// leaves as `None` keeps `self`'s.
    ///
    /// ```
    /// use tuiframe::style::{Color, Modifier, Style};
    ///
    /// let base = Style::new().fg(Color::White).add_modifier(Modifier::BOLD);
    /// let over = Style::new().bg(Color::Blue).remove_modifier(Modifier::BOLD);
    /// let both = base.patch(over);
    /// assert_eq!(both.fg, Some(Color::White));
    /// assert_eq!(both.bg, Some(Color::Blue));
    /// assert_eq!(both.add_modifier, Modifier::empty());
    /// assert_eq!(both.sub_modifier, Modifier::BOLD);
    /// ```
    #[must_use]
    pub const fn patch(mut self, other: Self) -> Self {
        if other.fg.is_some() {
            self.fg = other.fg;
        }
        if other.bg.is_some() {
            self.bg = other.bg;
        }
        if other.underline_color.is_some() {
            self.underline_color = other.underline_color;
        }
        if other.underline_style.is_some() {
            self.underline_style = other.underline_style;
        }
        self.add_modifier = self
            .add_modifier
            .difference(other.sub_modifier)
            .union(other.add_modifier);
        self.sub_modifier = self
            .sub_modifier
            .difference(other.add_modifier)
            .union(other.sub_modifier);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_operators_match_the_named_methods() {
        let bold_italic = Modifier::BOLD | Modifier::ITALIC;
        assert!(bold_italic.contains(Modifier::BOLD));
        assert!(!bold_italic.contains(Modifier::BOLD | Modifier::DIM));
        assert!(bold_italic.intersects(Modifier::BOLD | Modifier::DIM));
        assert!(!bold_italic.intersects(Modifier::DIM));
        assert_eq!(bold_italic & Modifier::ITALIC, Modifier::ITALIC);
        assert_eq!(bold_italic - Modifier::ITALIC, Modifier::BOLD);
        assert_eq!(Modifier::empty() - Modifier::BOLD, Modifier::empty());

        let mut m = Modifier::empty();
        assert!(m.is_empty());
        m |= Modifier::DIM;
        m.insert(Modifier::REVERSED);
        assert_eq!(m, Modifier::DIM | Modifier::REVERSED);
        m.remove(Modifier::DIM);
        assert_eq!(m, Modifier::REVERSED);
        m -= Modifier::REVERSED;
        assert!(m.is_empty());
        m = Modifier::all();
        m &= Modifier::HIDDEN | Modifier::OVERLINED;
        assert_eq!(m, Modifier::HIDDEN | Modifier::OVERLINED);
    }

    #[test]
    fn not_and_all_stay_within_the_named_flags() {
        assert_eq!(Modifier::all().bits(), 0b11_1111_1111);
        assert_eq!(!Modifier::empty(), Modifier::all());
        assert_eq!(!Modifier::all(), Modifier::empty());
        assert_eq!(!Modifier::BOLD, Modifier::all() - Modifier::BOLD);
        assert_eq!(Modifier::from_bits_truncate(u16::MAX), Modifier::all());
        assert!(Modifier::all().contains(Modifier::empty()));
    }

    #[test]
    fn modifier_debug_names_its_flags() {
        assert_eq!(format!("{:?}", Modifier::empty()), "Modifier(empty)");
        assert_eq!(format!("{:?}", Modifier::BOLD), "Modifier(BOLD)");
        assert_eq!(
            format!(
                "{:?}",
                Modifier::OVERLINED | Modifier::BOLD | Modifier::ITALIC
            ),
            "Modifier(BOLD | ITALIC | OVERLINED)"
        );
    }

    #[test]
    fn add_and_remove_undo_each_other() {
        let s = Style::new().add_modifier(Modifier::BOLD | Modifier::DIM);
        let s = s.remove_modifier(Modifier::DIM);
        assert_eq!(s.add_modifier, Modifier::BOLD);
        assert_eq!(s.sub_modifier, Modifier::DIM);
        let s = s.add_modifier(Modifier::DIM);
        assert_eq!(s.add_modifier, Modifier::BOLD | Modifier::DIM);
        assert_eq!(s.sub_modifier, Modifier::empty());
    }

    #[test]
    fn patch_keeps_what_the_other_leaves_unset() {
        let base = Style::new()
            .fg(Color::Red)
            .bg(Color::Black)
            .underline_color(Color::Indexed(4))
            .underline_style(UnderlineStyle::Curly);
        assert_eq!(base.patch(Style::new()), base);
        let over = Style::new()
            .fg(Color::Rgb(1, 2, 3))
            .underline_style(UnderlineStyle::Dashed);
        assert_eq!(
            base.patch(over),
            Style {
                fg: Some(Color::Rgb(1, 2, 3)),
                bg: Some(Color::Black),
                underline_color: Some(Color::Indexed(4)),
                underline_style: Some(UnderlineStyle::Dashed),
                ..Style::new()
            }
        );
    }

    #[test]
    fn patching_in_one_go_equals_patching_in_turn() {
        let styles = [
            Style::new(),
            Style::new().fg(Color::Yellow),
            Style::new().bg(Color::Yellow),
            Style::new().underline_style(UnderlineStyle::Double),
            Style::new().add_modifier(Modifier::BOLD),
            Style::new().remove_modifier(Modifier::BOLD),
            Style::new().add_modifier(Modifier::ITALIC),
            Style::new().remove_modifier(Modifier::ITALIC),
            Style::new().add_modifier(Modifier::ITALIC | Modifier::BOLD),
            Style::new().remove_modifier(Modifier::ITALIC | Modifier::BOLD),
        ];
        for &a in &styles {
            for &b in &styles {
                for &c in &styles {
                    assert_eq!(
                        Style::new().patch(a).patch(b).patch(c),
                        Style::new().patch(a.patch(b.patch(c)))
                    );
                }
            }
        }
    }

    #[test]
    fn named_colours_are_not_their_palette_indexes() {
        assert_ne!(Color::Red, Color::Indexed(1));
        assert_eq!(Color::default(), Color::Reset);
    }
}
