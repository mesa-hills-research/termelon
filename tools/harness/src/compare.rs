//! Comparing a terminal's readback with the screen `termelon-vt` computes for the same bytes.

use std::fmt;

use termelon_vt::{Cell, Color, Emulator, Style};
use unicode_normalization::UnicodeNormalization as _;

use crate::driver::{Colors, Readback};

/// The most mismatches a comparison lists.
pub const MAX_MISMATCHES: usize = 100;

/// One way a cell or the cursor differs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mismatch {
    /// The column.
    pub x: u16,
    /// The row.
    pub y: u16,
    /// What differs: `size`, `text`, `fg`, `bg`, `ul`, an attribute's name, or `cursor`.
    pub what: &'static str,
    /// What `termelon-vt` computed.
    pub expected: String,
    /// What the terminal showed.
    pub got: String,
}

impl fmt::Display for Mismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{},{} {}: expected {}, got {}",
            self.x, self.y, self.what, self.expected, self.got
        )
    }
}

/// The ways a readback differs from the expected screen, at most [`MAX_MISMATCHES`]. Text, in
/// Unicode's composed form, and colours always count. Attributes and the cursor count only where
/// the readback reports them.
pub fn compare(expected: &Emulator, readback: &Readback) -> Vec<Mismatch> {
    let mut out = Vec::new();
    let got = &readback.screen;
    if expected.size() != got.size() {
        let size = |(c, r): (u16, u16)| format!("{c}x{r}");
        out.push(Mismatch {
            x: 0,
            y: 0,
            what: "size",
            expected: size(expected.size()),
            got: size(got.size()),
        });
        return out;
    }
    let (cols, rows) = expected.size();
    let reports = readback.reports;
    for y in 0..rows {
        // The cells after the last one with text, which a readback that trims rows drops.
        let text_end = (0..cols)
            .rev()
            .find(|&x| expected.cell(x, y).is_some_and(|cell| cell.text() != " "))
            .map_or(0, |x| x + 1);
        for x in 0..cols {
            let (Some(e), Some(g)) = (expected.cell(x, y), got.cell(x, y)) else {
                continue;
            };
            // A space shows its foreground only through an underline, a line or reverse video.
            let shows_fg = e.text() != " "
                || e.style().reverse
                || e.style().strike
                || e.style().overline
                || e.style().underline != termelon_vt::Underline::None;
            let mut push = |what: &'static str, a: String, b: String| {
                if what == "fg" && !shows_fg {
                    return;
                }
                if a != b && out.len() < MAX_MISMATCHES {
                    out.push(Mismatch {
                        x,
                        y,
                        what,
                        expected: a,
                        got: b,
                    });
                }
            };
            // A letter and its combining mark show the same as the precomposed character, which
            // some terminals store instead.
            if (reports.wide_text || narrow_text(expected, x, y))
                && e.text().nfc().ne(g.text().nfc())
            {
                push("text", format!("{:?}", e.text()), format!("{:?}", g.text()));
            }
            if reports.trims_rows && x >= text_end {
                continue;
            }
            compare_colors(
                e.style(),
                g.style(),
                reports.colors,
                reports.indexed_as_named,
                &mut push,
            );
            compare_attributes(e, g, readback, &mut push);
        }
    }
    if reports.cursor {
        let (mut e, g) = (expected.cursor(), got.cursor());
        // A hidden cursor's place doesn't show.
        if !e.visible && !g.visible {
            (e.x, e.y) = (g.x, g.y);
        }
        let place = |x: u16, y: u16, visible: bool| {
            format!("{x},{y} {}", if visible { "visible" } else { "hidden" })
        };
        if (e.x, e.y, e.visible) != (g.x, g.y, g.visible) && out.len() < MAX_MISMATCHES {
            out.push(Mismatch {
                x: e.x,
                y: e.y,
                what: "cursor",
                expected: place(e.x, e.y, e.visible),
                got: place(g.x, g.y, g.visible),
            });
        }
    }
    out
}

/// Whether a cell's text, and the cell its cluster starts in, is one character up to U+00FF, which
/// a hardcopy can hold.
fn narrow_text(screen: &Emulator, x: u16, y: u16) -> bool {
    let mut leader = x;
    while leader > 0 && screen.cell(leader, y).is_some_and(Cell::is_continuation) {
        leader -= 1;
    }
    let Some(cell) = screen.cell(leader, y) else {
        return false;
    };
    let mut chars = cell.text().chars();
    matches!((chars.next(), chars.next()), (Some(c), None) if u32::from(c) <= 0xff)
        && cell.width() == 1
}

fn compare_colors(
    e: &Style,
    g: &Style,
    colors: Colors,
    indexed_as_named: bool,
    push: &mut impl FnMut(&'static str, String, String),
) {
    match colors {
        Colors::None => {}
        Colors::AsWritten => {
            let written = |color: Color| match color {
                Color::Indexed(n) if indexed_as_named => Color::named(n).unwrap_or(color),
                _ => color,
            };
            push("fg", format!("{:?}", written(e.fg)), format!("{:?}", g.fg));
            push("bg", format!("{:?}", written(e.bg)), format!("{:?}", g.bg));
        }
        Colors::Rgb {
            foreground,
            background,
            blink,
            bold_brightens,
        } => {
            let rgb = |color: Color, default: (u8, u8, u8)| color.rgb().unwrap_or(default);
            let shown_fg = match e.fg.named_index() {
                Some(n @ 0..8) if bold_brightens && e.bold => Color::named(n + 8).unwrap_or(e.fg),
                _ => e.fg,
            };
            let (mut fg, mut bg) = (rgb(shown_fg, foreground), rgb(e.bg, background));
            if let Some(colors) = blink.filter(|_| e.blink || e.rapid_blink) {
                (fg, bg) = colors;
            }
            if e.reverse {
                (fg, bg) = (bg, fg);
            }
            let hex = |(r, g, b): (u8, u8, u8)| format!("#{r:02x}{g:02x}{b:02x}");
            // A dim cell's colour is the terminal's own blend, which RGB readbacks give as is.
            if !e.dim {
                push("fg", hex(fg), hex(rgb(g.fg, foreground)));
            }
            push("bg", hex(bg), hex(rgb(g.bg, background)));
        }
    }
}

fn compare_attributes(
    e: &Cell,
    g: &Cell,
    readback: &Readback,
    push: &mut impl FnMut(&'static str, String, String),
) {
    let attributes = readback.reports.attributes;
    let (e, g) = (e.style(), g.style());
    let flags = [
        (attributes.bold, "bold", e.bold, g.bold),
        (attributes.dim, "dim", e.dim, g.dim),
        (attributes.italic, "italic", e.italic, g.italic),
        (
            attributes.blink,
            "blink",
            e.blink || e.rapid_blink,
            g.blink || g.rapid_blink,
        ),
        (attributes.reverse, "reverse", e.reverse, g.reverse),
        (attributes.hidden, "hidden", e.hidden, g.hidden),
        (attributes.strike, "strike", e.strike, g.strike),
        (attributes.overline, "overline", e.overline, g.overline),
    ];
    for (reported, name, a, b) in flags {
        if reported {
            push(name, a.to_string(), b.to_string());
        }
    }
    if attributes.underline_style {
        let underline = if attributes.unreported_underlines.contains(&e.underline) {
            termelon_vt::Underline::None
        } else {
            e.underline
        };
        push(
            "underline",
            format!("{underline:?}"),
            format!("{:?}", g.underline),
        );
        // An underline's colour shows only with the underline.
        if readback.reports.colors == Colors::AsWritten && underline != termelon_vt::Underline::None
        {
            push(
                "ul",
                format!("{:?}", e.underline_color),
                format!("{:?}", g.underline_color),
            );
        }
    } else if attributes.underline {
        let on = |style: &Style| style.underline != termelon_vt::Underline::None;
        push("underline", on(e).to_string(), on(g).to_string());
    }
    if attributes.link {
        let url = |style: &Style| {
            style
                .link
                .as_ref()
                .map_or_else(String::new, |link| link.url.clone())
        };
        push("link", url(e), url(g));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver::{Readback, Reports};
    use termelon_vt::Profile;

    fn screen(bytes: &str) -> Emulator {
        let mut screen = Emulator::with_profile(Profile::xterm(), 4, 1);
        screen.feed(bytes.as_bytes()).unwrap();
        screen
    }

    /// A letter with its combining mark matches the precomposed letter, and nothing else.
    #[test]
    fn text_compares_composed() {
        let reports = Reports {
            wide_text: true,
            ..Reports::default()
        };
        let expected = screen("e\u{301}x");
        let precomposed = Readback::new(screen("\u{e9}x"), reports);
        assert_eq!(compare(&expected, &precomposed), []);
        let bare = Readback::new(screen("ex"), reports);
        assert_eq!(compare(&expected, &bare).len(), 1);
    }
}
