//! Reading the screen back: text, the styled-text format, and HTML.

use std::fmt::Write as _;

use super::{CursorShape, Emulator};
use crate::grid::Cell;
use crate::style::{Style, Underline};

/// The colours `to_html` uses for the default foreground and background, xterm's palette 7 and 0.
const HTML_FG: (u8, u8, u8) = (0xe5, 0xe5, 0xe5);
const HTML_BG: (u8, u8, u8) = (0, 0, 0);

impl Emulator {
    /// A row's text, with continuation cells left out and trailing spaces trimmed, or `None` past
    /// the last row.
    pub fn row_text(&self, y: u16) -> Option<String> {
        let row = self.grid().row(y)?;
        let text: String = row.iter().map(Cell::text).collect();
        Some(text.trim_end_matches(' ').to_owned())
    }

    /// Every row's text, as [`row_text`](Emulator::row_text) gives it.
    pub fn rows(&self) -> Vec<String> {
        (0..self.rows).filter_map(|y| self.row_text(y)).collect()
    }

    /// The screen's text, its rows joined with `\n`.
    pub fn text(&self) -> String {
        self.rows().join("\n")
    }

    /// The screen in the styled-text format, version 1, which the crate docs describe.
    pub fn styled(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "size {} {}", self.cols, self.rows);
        let cursor = self.cursor();
        if cursor.visible {
            let shape = match cursor.shape {
                CursorShape::Default => "default",
                CursorShape::Block => "block",
                CursorShape::Underline => "underline",
                CursorShape::Bar => "bar",
            };
            let _ = writeln!(out, "cursor {} {} visible {shape}", cursor.x, cursor.y);
        } else {
            let _ = writeln!(out, "cursor {} {} hidden", cursor.x, cursor.y);
        }
        for y in 0..self.rows {
            let Some(row) = self.grid().row(y) else {
                continue;
            };
            let text = self.row_text(y).unwrap_or_default();
            let _ = writeln!(out, "text {y} {}", json_string(&text));
            let mut start = 0;
            while start < row.len() {
                let attrs = row[start].style().styled_attrs();
                let mut end = start + 1;
                while end < row.len() && row[end].style().styled_attrs() == attrs {
                    end += 1;
                }
                if !attrs.is_empty() {
                    let _ = writeln!(out, "style {y} {start} {end} {}", attrs.join(" "));
                }
                start = end;
            }
        }
        out
    }

    /// The screen as an HTML fragment, a `<pre>` with a `<span>` for each run of cells in one
    /// style, for contact sheets. Colours follow xterm's default palette, the default colours are
    /// light grey on black, and the cursor's cell has the class `cursor` when it shows.
    pub fn to_html(&self) -> String {
        let mut out = String::new();
        let _ = write!(
            out,
            "<pre class=\"termelon-vt\" style=\"background:{};color:{}\">",
            hex(HTML_BG),
            hex(HTML_FG)
        );
        let cursor = self.cursor();
        for y in 0..self.rows {
            if y > 0 {
                out.push('\n');
            }
            let Some(row) = self.grid().row(y) else {
                continue;
            };
            let cells: Vec<(u16, &Cell)> = (0..self.cols)
                .zip(row.iter())
                .filter(|(_, cell)| !cell.is_continuation())
                .collect();
            let mut i = 0;
            while i < cells.len() {
                let (x, cell) = cells[i];
                let at_cursor = |x: u16| {
                    cursor.visible && cursor.y == y && self.grid().leader(cursor.x, y) == x
                };
                let is_cursor = at_cursor(x);
                let mut end = i + 1;
                while !is_cursor
                    && end < cells.len()
                    && cells[end].1.style() == cell.style()
                    && !at_cursor(cells[end].0)
                {
                    end += 1;
                }
                let text: String = cells[i..end].iter().map(|(_, cell)| cell.text()).collect();
                html_span(
                    &mut out,
                    cell.style(),
                    &text,
                    is_cursor.then_some(cursor.shape),
                );
                i = end;
            }
        }
        out.push_str("</pre>");
        out
    }
}

/// A string in double quotes with JSON's escapes, as `serde_json` writes it.
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if u32::from(c) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn hex((r, g, b): (u8, u8, u8)) -> String {
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn html_escape(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
}

fn html_span(out: &mut String, style: &Style, text: &str, cursor: Option<CursorShape>) {
    let mut css = String::new();
    let mut fg = style.fg.rgb();
    let mut bg = style.bg.rgb();
    if style.reverse {
        (fg, bg) = (Some(bg.unwrap_or(HTML_BG)), Some(fg.unwrap_or(HTML_FG)));
    }
    if style.hidden {
        fg = Some(bg.unwrap_or(HTML_BG));
    }
    if let Some(fg) = fg {
        let _ = write!(css, "color:{};", hex(fg));
    }
    if let Some(bg) = bg {
        let _ = write!(css, "background:{};", hex(bg));
    }
    if style.bold {
        css.push_str("font-weight:bold;");
    }
    if style.dim {
        css.push_str("opacity:0.5;");
    }
    if style.italic {
        css.push_str("font-style:italic;");
    }
    let mut lines = Vec::new();
    if style.underline != Underline::None {
        lines.push("underline");
    }
    if style.strike {
        lines.push("line-through");
    }
    if style.overline {
        lines.push("overline");
    }
    if !lines.is_empty() {
        let _ = write!(css, "text-decoration-line:{};", lines.join(" "));
        let decoration = match style.underline {
            Underline::Double => Some("double"),
            Underline::Curly => Some("wavy"),
            Underline::Dotted => Some("dotted"),
            Underline::Dashed => Some("dashed"),
            Underline::None | Underline::Single => None,
        };
        if let Some(decoration) = decoration {
            let _ = write!(css, "text-decoration-style:{decoration};");
        }
        if let Some(color) = style.underline_color.rgb() {
            let _ = write!(css, "text-decoration-color:{};", hex(color));
        }
    }
    let mut classes = Vec::new();
    if style.blink {
        classes.push("blink");
    }
    if style.rapid_blink {
        classes.push("rapid-blink");
    }
    if let Some(shape) = cursor {
        classes.push("cursor");
        css.push_str(match shape {
            CursorShape::Bar => "box-shadow:inset 2px 0 0 currentColor;",
            CursorShape::Underline => "box-shadow:inset 0 -2px 0 currentColor;",
            CursorShape::Block | CursorShape::Default => "outline:1px solid currentColor;",
        });
    }
    if let Some(link) = &style.link {
        out.push_str("<a href=\"");
        html_escape(out, &link.url);
        out.push_str("\">");
    }
    if css.is_empty() && classes.is_empty() {
        html_escape(out, text);
    } else {
        out.push_str("<span");
        if !classes.is_empty() {
            let _ = write!(out, " class=\"{}\"", classes.join(" "));
        }
        if !css.is_empty() {
            let _ = write!(out, " style=\"{css}\"");
        }
        out.push('>');
        html_escape(out, text);
        out.push_str("</span>");
    }
    if style.link.is_some() {
        out.push_str("</a>");
    }
}

#[cfg(test)]
mod tests {
    use super::json_string;

    #[test]
    fn json_escapes() {
        assert_eq!(json_string("a\"b\\c"), r#""a\"b\\c""#);
        assert_eq!(json_string("\u{1}\n"), r#""\u0001\n""#);
        assert_eq!(json_string("⣷ Working…"), "\"⣷ Working…\"");
    }
}
