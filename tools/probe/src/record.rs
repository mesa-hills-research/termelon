//! A plain rendering of a frame: the bytes that show the frame's cells and cursor on a cleared
//! screen, one cursor move per row and the full SGR for each change of style. The harness feeds
//! it to `tuiframe-vt` for the screen the terminal should show.
//!
//! It leaves out what the framework leaves out for the terminal: underline styles, underline
//! colours, links and cursor shapes where the terminal lacks them, and colours under `NO_COLOR`.
//! It writes RGB colours as they are, where the framework gives a terminal with fewer colours the
//! nearest it has.

use std::fmt::Write as _;

use tuiframe::caps::{ColorDepth, Support};
use tuiframe::{Buffer, Capabilities, Color, Cursor, CursorShape, Modifier, UnderlineStyle};

/// What the framework sends a terminal, from what it found the terminal supports.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Allowed {
    colors: bool,
    underline_styles: bool,
    underline_color: bool,
    hyperlinks: bool,
    cursor_shape: bool,
}

impl Allowed {
    /// Everything, as for a terminal that has it all.
    #[cfg(test)]
    pub(crate) const ALL: Allowed = Allowed {
        colors: true,
        underline_styles: true,
        underline_color: true,
        hyperlinks: true,
        cursor_shape: true,
    };

    /// As the framework decides it for a terminal's capabilities.
    pub(crate) fn new(caps: &Capabilities) -> Allowed {
        let colors = caps.colors != ColorDepth::None;
        Allowed {
            colors,
            underline_styles: caps.underline_styles == Support::Yes,
            underline_color: caps.underline_color == Support::Yes && colors,
            hyperlinks: caps.hyperlinks == Support::Yes,
            cursor_shape: caps.cursor_shape != Support::No,
        }
    }
}

/// The SGR parameters of a colour, or none for the default. The underline colour has no named
/// form, so its named colours go as their palette index.
fn color(color: Color, base: u8, bright: u8, extended: u8, out: &mut Vec<String>) {
    let named = |index: u8| {
        if base == 0 {
            format!("{extended};5;{index}")
        } else if index < 8 {
            (base + index).to_string()
        } else {
            (bright + index - 8).to_string()
        }
    };
    let code = match color {
        Color::Reset => return,
        Color::Black => named(0),
        Color::Red => named(1),
        Color::Green => named(2),
        Color::Yellow => named(3),
        Color::Blue => named(4),
        Color::Magenta => named(5),
        Color::Cyan => named(6),
        Color::Gray => named(7),
        Color::DarkGray => named(8),
        Color::LightRed => named(9),
        Color::LightGreen => named(10),
        Color::LightYellow => named(11),
        Color::LightBlue => named(12),
        Color::LightMagenta => named(13),
        Color::LightCyan => named(14),
        Color::White => named(15),
        Color::Indexed(n) => format!("{extended};5;{n}"),
        Color::Rgb(r, g, b) => format!("{extended};2;{r};{g};{b}"),
    };
    out.push(code);
}

/// The SGR that sets a cell's style from a reset.
fn sgr(
    fg: Color,
    bg: Color,
    modifier: Modifier,
    underline: Option<UnderlineStyle>,
    underline_color: Color,
    allowed: Allowed,
) -> String {
    let mut params = vec!["0".to_owned()];
    for (flag, code) in [
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::ITALIC, "3"),
        (Modifier::SLOW_BLINK, "5"),
        (Modifier::RAPID_BLINK, "6"),
        (Modifier::REVERSED, "7"),
        (Modifier::HIDDEN, "8"),
        (Modifier::CROSSED_OUT, "9"),
        (Modifier::OVERLINED, "53"),
    ] {
        if modifier.contains(flag) {
            params.push(code.to_owned());
        }
    }
    if modifier.contains(Modifier::UNDERLINED) || underline.is_some() {
        let shape = match underline.filter(|_| allowed.underline_styles) {
            None | Some(UnderlineStyle::Single) => 1,
            Some(UnderlineStyle::Double) => 2,
            Some(UnderlineStyle::Curly) => 3,
            Some(UnderlineStyle::Dotted) => 4,
            Some(UnderlineStyle::Dashed) => 5,
        };
        params.push(format!("4:{shape}"));
        if allowed.underline_color {
            color(underline_color, 0, 0, 58, &mut params);
        }
    }
    if allowed.colors {
        color(fg, 30, 90, 38, &mut params);
        color(bg, 40, 100, 48, &mut params);
    }
    format!("\x1b[{}m", params.join(";"))
}

/// The bytes that show `buffer` and `cursor` on a screen of the buffer's size.
pub(crate) fn render(buffer: &Buffer, cursor: Option<Cursor>, allowed: Allowed) -> Vec<u8> {
    let area = buffer.area();
    let mut out = String::from("\x1b[?7l\x1b[0m\x1b[H\x1b[2J");
    for y in area.top()..area.bottom() {
        let _ = write!(out, "\x1b[{};1H", y - area.top() + 1);
        let mut last = String::new();
        let mut link: Option<String> = None;
        for x in area.left()..area.right() {
            let Some(cell) = buffer.cell((x, y)) else {
                continue;
            };
            if cell.width() == 0 {
                continue;
            }
            let cell_link = cell
                .link()
                .filter(|_| allowed.hyperlinks)
                .map(str::to_owned);
            if cell_link != link {
                let _ = write!(out, "\x1b]8;;{}\x1b\\", cell_link.as_deref().unwrap_or(""));
                link = cell_link;
            }
            let style = sgr(
                cell.fg(),
                cell.bg(),
                cell.modifier(),
                cell.underline_style(),
                cell.underline_color(),
                allowed,
            );
            if style != last {
                out.push_str(&style);
                last = style;
            }
            let symbol = cell.symbol();
            out.push_str(if symbol.is_empty() { " " } else { symbol });
        }
        if link.is_some() {
            out.push_str("\x1b]8;;\x1b\\");
        }
        out.push_str("\x1b[0m");
    }
    match cursor {
        Some(cursor) => {
            let shape = match cursor.shape {
                CursorShape::Default => 0,
                CursorShape::Block => 2,
                CursorShape::Underline => 4,
                CursorShape::Bar => 6,
            } - u8::from(cursor.blinking && cursor.shape != CursorShape::Default);
            let position = cursor.position;
            let _ = write!(out, "\x1b[{};{}H", position.y + 1, position.x + 1);
            if allowed.cursor_shape {
                let _ = write!(out, "\x1b[{shape} q");
            }
            out.push_str("\x1b[?25h");
        }
        None => out.push_str("\x1b[?25l"),
    }
    out.into_bytes()
}
