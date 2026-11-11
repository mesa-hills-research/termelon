//! The styled-text format, a plain text record of frames.
//!
//! ```text
//! size 80 24
//! cursor 2 23 visible bar
//! text 0 "  ⣷ Working…"
//! style 0 2 3 fg=#7f8ea3
//! style 0 4 13 fg=#d8dee9 bold
//! ```
//!
//! A frame starts with its `size` line, and frames follow each other. `cursor` is `hidden`, or
//! `visible` with a shape (`block`, `underline`, `bar` or `default`). A row has at most one `text`
//! line, a JSON string with the continuation cells of wide characters left out and the trailing
//! spaces trimmed. A `style` line gives a row, the first column and the column after the last, then
//! attributes, in this order when written: `fg=`, `bg=`, `ul=`, `underline` or
//! `underline=double|curly|dotted|dashed`, `bold`, `dim`, `italic`, `blink`, `rapid-blink`,
//! `reverse`, `hidden`, `strike`, `overline` and `link=<url>`. Colours are `#rrggbb`, `idx:N` or one
//! of ratatui's names. Columns count cells, and a run may go past the trimmed text.
//!
//! The writer puts each row's `style` lines after its `text` line, with every line ending in a newline.
//! The reader takes the lines of a frame in any order, skips blank lines and lines starting with `#`,
//! takes a shape after `hidden`, and lets a later `style` line patch over an earlier one.

use std::fmt::{self, Write as _};

use unicode_segmentation::UnicodeSegmentation;

use crate::model::{Color, Cursor, CursorShape, Frame, Mods, NAMED, Style, Underline, width_of};

/// The largest frame the reader accepts, in cells.
const MOST_CELLS: usize = 1 << 22;

/// Why a file could not be read.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParseError {
    /// The line, counting from 1.
    pub(crate) line: usize,
    pub(crate) message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

/// A `style` line, held until the frame's text is in.
struct Run {
    row: u16,
    from: u16,
    to: u16,
    style: Style,
    link: Option<String>,
}

/// A frame being read.
struct Pending {
    frame: Frame,
    has_text: Vec<bool>,
    runs: Vec<Run>,
}

impl Pending {
    fn finish(self) -> Frame {
        let Pending {
            mut frame, runs, ..
        } = self;
        for run in runs {
            let link = run.link.as_deref().map(|url| frame.link(url));
            for x in run.from..run.to {
                if let Some(cell) = frame.cell(x, run.row).copied() {
                    let mut style = patch(cell.style, run.style);
                    if let Some(link) = link {
                        style.link = link;
                    }
                    frame.set_style(run.row, x, x + 1, style);
                }
            }
        }
        frame
    }
}

/// `base` with every attribute `top` sets.
fn patch(base: Style, top: Style) -> Style {
    let pick = |a: Color, b: Color| if b == Color::Default { a } else { b };
    Style {
        fg: pick(base.fg, top.fg),
        bg: pick(base.bg, top.bg),
        ul: pick(base.ul, top.ul),
        underline: if top.underline == Underline::None {
            base.underline
        } else {
            top.underline
        },
        mods: base.mods.with(top.mods),
        link: base.link,
    }
}

/// Reads every frame in `input`.
///
/// # Errors
///
/// Fails with the line and the reason when a line is unknown, malformed or outside its frame.
pub(crate) fn parse(input: &str) -> Result<Vec<Frame>, ParseError> {
    let mut frames = Vec::new();
    let mut current: Option<Pending> = None;
    for (at, raw) in input.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim_start();
        let fail = |message: String| ParseError {
            line: at + 1,
            message,
        };
        if word == "size" {
            let (width, height) = size(rest).map_err(fail)?;
            frames.extend(current.take().map(Pending::finish));
            current = Some(Pending {
                frame: Frame::new(width, height),
                has_text: vec![false; usize::from(height)],
                runs: Vec::new(),
            });
            continue;
        }
        let Some(pending) = current.as_mut() else {
            return Err(fail(format!("`{word}` comes before any `size` line")));
        };
        match word {
            "cursor" => pending.frame.cursor = cursor(rest, &pending.frame).map_err(fail)?,
            "text" => text(rest, pending).map_err(fail)?,
            "style" => {
                let run = style(rest, &pending.frame).map_err(fail)?;
                pending.runs.push(run);
            }
            _ => return Err(fail(format!("unknown line `{word}`"))),
        }
    }
    frames.extend(current.map(Pending::finish));
    Ok(frames)
}

fn number(word: Option<&str>, what: &str) -> Result<u16, String> {
    let word = word.ok_or_else(|| format!("{what} is missing"))?;
    word.parse()
        .map_err(|_| format!("{what} must be a number up to 65535, not `{word}`"))
}

fn size(rest: &str) -> Result<(u16, u16), String> {
    let mut words = rest.split_whitespace();
    let width = number(words.next(), "the width")?;
    let height = number(words.next(), "the height")?;
    if words.next().is_some() {
        return Err("`size` takes a width and a height".into());
    }
    if width == 0 || height == 0 {
        return Err("a frame needs at least one row and one column".into());
    }
    if usize::from(width) * usize::from(height) > MOST_CELLS {
        return Err(format!(
            "{width} by {height} is more than {MOST_CELLS} cells"
        ));
    }
    Ok((width, height))
}

fn cursor(rest: &str, frame: &Frame) -> Result<Cursor, String> {
    let mut words = rest.split_whitespace();
    let x = number(words.next(), "the cursor's column")?;
    let y = number(words.next(), "the cursor's row")?;
    if x >= frame.width() || y >= frame.height() {
        return Err(format!(
            "the cursor at {x} {y} is outside the {} by {} frame",
            frame.width(),
            frame.height()
        ));
    }
    let visible = match words.next() {
        Some("visible") => true,
        Some("hidden") => false,
        other => {
            return Err(format!(
                "the cursor is `visible` or `hidden`, not `{}`",
                other.unwrap_or("")
            ));
        }
    };
    let shape = match words.next() {
        None | Some("default") => CursorShape::Default,
        Some("block") => CursorShape::Block,
        Some("underline") => CursorShape::Underline,
        Some("bar") => CursorShape::Bar,
        Some(other) => return Err(format!("unknown cursor shape `{other}`")),
    };
    if words.next().is_some() {
        return Err("`cursor` takes a column, a row, visible or hidden, and a shape".into());
    }
    Ok(Cursor {
        x,
        y,
        visible,
        shape,
    })
}

fn text(rest: &str, pending: &mut Pending) -> Result<(), String> {
    let (row, quoted) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let row = number(Some(row).filter(|row| !row.is_empty()), "the row")?;
    let seen = pending
        .has_text
        .get_mut(usize::from(row))
        .ok_or_else(|| format!("row {row} is outside the {} rows", pending.frame.height()))?;
    if *seen {
        return Err(format!("row {row} has two `text` lines"));
    }
    let text = string(quoted.trim_start())?;
    let mut columns = 0_usize;
    for cluster in text.graphemes(true) {
        if cluster.chars().any(char::is_control) {
            return Err(format!(
                "the text has a control character after column {columns}"
            ));
        }
        let width = width_of(cluster);
        if width == 0 {
            return Err(format!("`{cluster}` after column {columns} has no width"));
        }
        columns += width;
    }
    if columns > usize::from(pending.frame.width()) {
        return Err(format!(
            "the text is {columns} columns wide and the frame {}",
            pending.frame.width()
        ));
    }
    *seen = true;
    let width = pending.frame.width();
    pending
        .frame
        .put_str(0, row, &text, Style::default(), width);
    Ok(())
}

/// A JSON-style string that fills `input`, with its quotes.
fn string(input: &str) -> Result<String, String> {
    let body = input
        .strip_prefix('"')
        .ok_or("the text must be a string in double quotes")?;
    let mut out = String::new();
    let mut chars = body.chars();
    loop {
        match chars.next() {
            None => return Err("the string has no closing quote".into()),
            Some('"') => break,
            Some('\\') => out.push(escape(&mut chars)?),
            Some(c) => out.push(c),
        }
    }
    if !chars.as_str().trim().is_empty() {
        return Err("there is more after the string".into());
    }
    Ok(out)
}

fn escape(chars: &mut std::str::Chars<'_>) -> Result<char, String> {
    Ok(match chars.next() {
        Some('"') => '"',
        Some('\\') => '\\',
        Some('/') => '/',
        Some('b') => '\u{8}',
        Some('f') => '\u{c}',
        Some('n') => '\n',
        Some('r') => '\r',
        Some('t') => '\t',
        Some('u') => {
            let first = hex4(chars)?;
            match first {
                0xd800..=0xdbff => {
                    if chars.next() != Some('\\') || chars.next() != Some('u') {
                        return Err("a high surrogate needs a low one after it".into());
                    }
                    let second = hex4(chars)?;
                    if !(0xdc00..=0xdfff).contains(&second) {
                        return Err("a high surrogate needs a low one after it".into());
                    }
                    let scalar = 0x1_0000 + ((first - 0xd800) << 10) + (second - 0xdc00);
                    char::from_u32(scalar).ok_or("a surrogate pair is not a character")?
                }
                0xdc00..=0xdfff => return Err("a low surrogate comes without a high one".into()),
                _ => char::from_u32(first).ok_or("`\\u` is not a character")?,
            }
        }
        Some(other) => return Err(format!("unknown escape `\\{other}`")),
        None => return Err("the string ends in a backslash".into()),
    })
}

fn hex4(chars: &mut std::str::Chars<'_>) -> Result<u32, String> {
    let mut value = 0;
    for _ in 0..4 {
        let digit = chars
            .next()
            .and_then(|c| c.to_digit(16))
            .ok_or("`\\u` needs four hex digits")?;
        value = value * 16 + digit;
    }
    Ok(value)
}

fn style(rest: &str, frame: &Frame) -> Result<Run, String> {
    let mut words = rest.split_whitespace();
    let row = number(words.next(), "the row")?;
    let from = number(words.next(), "the first column")?;
    let to = number(words.next(), "the end column")?;
    if row >= frame.height() {
        return Err(format!("row {row} is outside the {} rows", frame.height()));
    }
    if from >= to || to > frame.width() {
        return Err(format!(
            "columns {from} to {to} are not inside the {} columns",
            frame.width()
        ));
    }
    let mut style = Style::default();
    let mut link = None;
    for word in words {
        attribute(word, &mut style, &mut link)?;
    }
    Ok(Run {
        row,
        from,
        to,
        style,
        link,
    })
}

fn attribute(word: &str, style: &mut Style, link: &mut Option<String>) -> Result<(), String> {
    let colour = |value: &str| {
        color(value).ok_or_else(|| format!("`{value}` is not a colour (#rrggbb, idx:N or a name)"))
    };
    if let Some(value) = word.strip_prefix("fg=") {
        style.fg = colour(value)?;
    } else if let Some(value) = word.strip_prefix("bg=") {
        style.bg = colour(value)?;
    } else if let Some(value) = word.strip_prefix("ul=") {
        style.ul = colour(value)?;
    } else if let Some(url) = word.strip_prefix("link=") {
        if url.is_empty() {
            return Err("`link=` needs a URL".into());
        }
        *link = Some(url.to_string());
    } else if let Some(kind) = word.strip_prefix("underline=") {
        style.underline = match kind {
            "single" => Underline::Single,
            "double" => Underline::Double,
            "curly" => Underline::Curly,
            "dotted" => Underline::Dotted,
            "dashed" => Underline::Dashed,
            other => return Err(format!("unknown underline `{other}`")),
        };
    } else if word == "underline" {
        style.underline = Underline::Single;
    } else if let Some((_, mods)) = MODS.iter().find(|(name, _)| *name == word) {
        style.mods = style.mods.with(*mods);
    } else {
        return Err(format!("unknown attribute `{word}`"));
    }
    Ok(())
}

/// The modifiers with their names, in the order they are written.
const MODS: [(&str, Mods); 9] = [
    ("bold", Mods::BOLD),
    ("dim", Mods::DIM),
    ("italic", Mods::ITALIC),
    ("blink", Mods::BLINK),
    ("rapid-blink", Mods::RAPID_BLINK),
    ("reverse", Mods::REVERSE),
    ("hidden", Mods::HIDDEN),
    ("strike", Mods::STRIKE),
    ("overline", Mods::OVERLINE),
];

fn color(value: &str) -> Option<Color> {
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() != 6 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let channel = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
        return Some(Color::Rgb(channel(0)?, channel(2)?, channel(4)?));
    }
    if let Some(index) = value.strip_prefix("idx:") {
        return index.parse().ok().map(Color::Indexed);
    }
    NAMED
        .iter()
        .find(|(name, _)| *name == value)
        .map(|(_, named)| Color::Named(*named))
}

/// Writes `frames` in the format, ending in a newline.
pub(crate) fn write(frames: &[Frame]) -> String {
    let mut out = String::new();
    for frame in frames {
        write_frame(frame, &mut out);
    }
    out
}

fn write_frame(frame: &Frame, out: &mut String) {
    // Writing to a `String` can't fail, so the results are dropped.
    let _ = writeln!(out, "size {} {}", frame.width(), frame.height());
    let cursor = frame.cursor;
    let _ = write!(out, "cursor {} {} ", cursor.x, cursor.y);
    out.push_str(if cursor.visible { "visible " } else { "hidden" });
    if cursor.visible {
        out.push_str(match cursor.shape {
            CursorShape::Default => "default",
            CursorShape::Block => "block",
            CursorShape::Underline => "underline",
            CursorShape::Bar => "bar",
        });
    }
    out.push('\n');
    for y in 0..frame.height() {
        let _ = write!(out, "text {y} ");
        quote(&frame.row_text(y), out);
        out.push('\n');
        let row = frame.row(y);
        let mut x = 0;
        while let Some(cell) = row.get(x) {
            let run = row[x..]
                .iter()
                .take_while(|next| next.style == cell.style)
                .count();
            if cell.style != Style::default() {
                let _ = write!(out, "style {y} {x} {}", x + run);
                attributes(frame, cell.style, out);
                out.push('\n');
            }
            x += run;
        }
    }
}

fn quote(text: &str, out: &mut String) {
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn attributes(frame: &Frame, style: Style, out: &mut String) {
    for (key, value) in [("fg", style.fg), ("bg", style.bg), ("ul", style.ul)] {
        if value != Color::Default {
            let _ = write!(out, " {key}=");
            write_color(value, out);
        }
    }
    out.push_str(match style.underline {
        Underline::None => "",
        Underline::Single => " underline",
        Underline::Double => " underline=double",
        Underline::Curly => " underline=curly",
        Underline::Dotted => " underline=dotted",
        Underline::Dashed => " underline=dashed",
    });
    for (name, mods) in MODS {
        if style.mods.contains(mods) {
            let _ = write!(out, " {name}");
        }
    }
    if let Some(url) = frame.link_of(style) {
        let _ = write!(out, " link={url}");
    }
}

fn write_color(color: Color, out: &mut String) {
    let _ = match color {
        Color::Default => Ok(()),
        Color::Named(named) => out.write_str(named.name()),
        Color::Indexed(index) => write!(out, "idx:{index}"),
        Color::Rgb(r, g, b) => write!(out, "#{r:02x}{g:02x}{b:02x}"),
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Named, Symbol};

    fn is_cluster(symbol: Symbol) -> bool {
        matches!(symbol, Symbol::Cluster(_))
    }

    const EXAMPLE: &str = r#"size 80 24
cursor 2 23 visible bar
text 0 "  ⣷ Working…"
style 0 2 3 fg=#7f8ea3
style 0 4 13 fg=#d8dee9 bold
"#;

    fn one(input: &str) -> Frame {
        let mut frames = parse(input).unwrap();
        assert_eq!(frames.len(), 1);
        frames.remove(0)
    }

    fn fails(input: &str) -> ParseError {
        parse(input).unwrap_err()
    }

    #[test]
    fn reads_the_example() {
        let frame = one(EXAMPLE);
        assert_eq!((frame.width(), frame.height()), (80, 24));
        assert_eq!(
            frame.cursor,
            Cursor {
                x: 2,
                y: 23,
                visible: true,
                shape: CursorShape::Bar
            }
        );
        assert_eq!(frame.row_text(0), "  ⣷ Working…");
        assert_eq!(frame.row_text(1), "");
        let spinner = frame.cell(2, 0).unwrap();
        assert_eq!(spinner.symbol, Symbol::Char('⣷'));
        assert_eq!(
            spinner.style,
            Style::default().fg(Color::Rgb(0x7f, 0x8e, 0xa3))
        );
        assert_eq!(frame.cell(3, 0).unwrap().style, Style::default());
        let word = frame.cell(4, 0).unwrap();
        assert_eq!(word.symbol, Symbol::Char('W'));
        assert_eq!(
            word.style,
            Style::default().fg(Color::Rgb(0xd8, 0xde, 0xe9)).bold()
        );
        // The run reaches the ellipsis in column 11 and the blank cell after the text.
        assert_eq!(frame.cell(11, 0).unwrap().symbol, Symbol::Char('…'));
        assert_eq!(frame.cell(11, 0).unwrap().style.mods, Mods::BOLD);
        assert_eq!(frame.cell(12, 0).unwrap().symbol, Symbol::Char(' '));
        assert_eq!(frame.cell(12, 0).unwrap().style.mods, Mods::BOLD);
        assert_eq!(frame.cell(13, 0).unwrap().style, Style::default());
    }

    #[test]
    fn writing_the_example_gives_it_back() {
        let frame = one(EXAMPLE);
        let again = write(std::slice::from_ref(&frame));
        let rows: Vec<&str> = again.lines().collect();
        assert_eq!(&rows[..2], ["size 80 24", "cursor 2 23 visible bar"]);
        assert_eq!(rows[2], r#"text 0 "  ⣷ Working…""#);
        assert_eq!(rows[3], "style 0 2 3 fg=#7f8ea3");
        assert_eq!(rows[4], "style 0 4 13 fg=#d8dee9 bold");
        assert_eq!(rows[5], "text 1 \"\"");
        assert_eq!(rows.len(), 28);
        assert!(again.ends_with("text 23 \"\"\n"));
        assert_eq!(one(&again), frame);
    }

    #[test]
    fn frames_follow_each_other() {
        let frames =
            parse("size 3 1\ntext 0 \"abc\"\nsize 2 2\ncursor 1 1 hidden\ntext 1 \"x\"\n").unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!((frames[0].width(), frames[0].height()), (3, 1));
        assert_eq!(frames[0].row_text(0), "abc");
        assert_eq!((frames[1].width(), frames[1].height()), (2, 2));
        assert_eq!(frames[1].row_text(1), "x");
        assert!(!frames[1].cursor.visible);
        assert_eq!(frames[0].cursor, Cursor::default());
    }

    #[test]
    fn blank_lines_and_comments_are_skipped() {
        let frame = one("# recorded\n\nsize 2 1\n  # a note\ntext 0 \"ab\"  \n\n");
        assert_eq!(frame.row_text(0), "ab");
    }

    #[test]
    fn an_empty_file_has_no_frames() {
        assert_eq!(parse("").unwrap(), [] as [Frame; 0]);
        assert_eq!(parse("\n# nothing\n").unwrap(), [] as [Frame; 0]);
    }

    #[test]
    fn columns_count_cells_so_wide_characters_take_two() {
        let frame = one("size 8 1\ntext 0 \"漢字ab\"\nstyle 0 4 6 fg=idx:3\nstyle 0 0 2 bg=red\n");
        assert_eq!(frame.cell(0, 0).unwrap().symbol, Symbol::Char('漢'));
        assert_eq!(frame.cell(1, 0).unwrap().symbol, Symbol::Continuation);
        assert_eq!(frame.cell(2, 0).unwrap().symbol, Symbol::Char('字'));
        assert_eq!(frame.cell(4, 0).unwrap().symbol, Symbol::Char('a'));
        assert_eq!(frame.cell(4, 0).unwrap().style.fg, Color::Indexed(3));
        assert_eq!(frame.cell(5, 0).unwrap().style.fg, Color::Indexed(3));
        assert_eq!(frame.cell(6, 0).unwrap().style.fg, Color::Default);
        // The run on the first two cells covers the first character and its continuation.
        assert_eq!(frame.cell(0, 0).unwrap().style.bg, Color::Named(Named::Red));
        assert_eq!(frame.cell(1, 0).unwrap().style.bg, Color::Named(Named::Red));
        assert_eq!(frame.cell(2, 0).unwrap().style.bg, Color::Default);
        assert_eq!(frame.row_text(0), "漢字ab");
    }

    #[test]
    fn clusters_of_several_scalars_are_one_symbol() {
        let frame = one("size 6 1\ntext 0 \"👍🏽e\\u0301x\"\n");
        assert!(is_cluster(frame.cell(0, 0).unwrap().symbol));
        assert_eq!(frame.cell(1, 0).unwrap().symbol, Symbol::Continuation);
        assert!(is_cluster(frame.cell(2, 0).unwrap().symbol));
        assert_eq!(frame.cell(3, 0).unwrap().symbol, Symbol::Char('x'));
    }

    #[test]
    fn a_style_can_cover_the_spaces_the_text_dropped() {
        let frame = one("size 6 1\ntext 0 \"ab\"\nstyle 0 0 6 bg=#202020\n");
        assert_eq!(frame.row_text(0), "ab");
        assert!(
            frame
                .cells()
                .iter()
                .all(|cell| cell.style.bg == Color::Rgb(0x20, 0x20, 0x20))
        );
        assert_eq!(frame.cell(5, 0).unwrap().symbol, Symbol::Char(' '));
    }

    #[test]
    fn reads_every_colour_form() {
        let frame = one(concat!(
            "size 4 1\ntext 0 \"abcd\"\n",
            "style 0 0 1 fg=#0a0b0c bg=idx:200\n",
            "style 0 1 2 fg=light-magenta bg=dark-gray ul=white\n",
            "style 0 2 3 fg=gray\n",
        ));
        let style = |x| frame.cell(x, 0).unwrap().style;
        assert_eq!(style(0).fg, Color::Rgb(10, 11, 12));
        assert_eq!(style(0).bg, Color::Indexed(200));
        assert_eq!(style(1).fg, Color::Named(Named::LightMagenta));
        assert_eq!(style(1).bg, Color::Named(Named::DarkGray));
        assert_eq!(style(1).ul, Color::Named(Named::White));
        assert_eq!(style(2).fg, Color::Named(Named::Gray));
    }

    #[test]
    fn reads_every_attribute() {
        let frame = one(concat!(
            "size 3 1\ntext 0 \"abc\"\n",
            "style 0 0 1 underline bold dim italic blink rapid-blink reverse hidden strike overline\n",
            "style 0 1 2 underline=curly\n",
            "style 0 2 3 link=https://example.org/a?b=c\n",
        ));
        let first = frame.cell(0, 0).unwrap().style;
        assert_eq!(first.underline, Underline::Single);
        for (name, mods) in MODS {
            assert!(first.mods.contains(mods), "{name}");
        }
        assert_eq!(frame.cell(1, 0).unwrap().style.underline, Underline::Curly);
        let link = frame.cell(2, 0).unwrap().style;
        assert_eq!(frame.link_of(link), Some("https://example.org/a?b=c"));
        assert_eq!(frame.link_of(first), None);
    }

    #[test]
    fn every_underline_kind_is_known() {
        for (word, kind) in [
            ("single", Underline::Single),
            ("double", Underline::Double),
            ("curly", Underline::Curly),
            ("dotted", Underline::Dotted),
            ("dashed", Underline::Dashed),
        ] {
            let frame = one(&format!("size 1 1\nstyle 0 0 1 underline={word}\n"));
            assert_eq!(frame.cell(0, 0).unwrap().style.underline, kind);
        }
    }

    #[test]
    fn a_later_style_patches_over_an_earlier_one() {
        let frame = one("size 3 1\nstyle 0 0 3 bg=#101010 fg=red\nstyle 0 1 2 bold fg=blue\n");
        let style = |x| frame.cell(x, 0).unwrap().style;
        assert_eq!(style(1).bg, Color::Rgb(0x10, 0x10, 0x10));
        assert_eq!(style(1).fg, Color::Named(Named::Blue));
        assert_eq!(style(1).mods, Mods::BOLD);
        assert_eq!(style(0).fg, Color::Named(Named::Red));
    }

    #[test]
    fn strings_have_json_escapes() {
        let frame = one("size 12 1\ntext 0 \"q\\\"b\\\\s\\/\\u00e9\\ud83d\\udc4d\\u0041\"\n");
        assert_eq!(frame.row_text(0), "q\"b\\s/é👍A");
    }

    #[test]
    fn text_with_a_quote_and_a_backslash_is_written_with_escapes() {
        let mut frame = Frame::new(8, 1);
        frame.put_str(0, 0, "a\"b\\c", Style::default(), 8);
        let written = write(std::slice::from_ref(&frame));
        assert!(written.contains(r#"text 0 "a\"b\\c""#), "{written}");
        assert_eq!(one(&written), frame);
    }

    #[test]
    fn errors_name_the_line() {
        let error = fails("size 4 2\ntext 0 \"ab\"\nstyle 0 0 9 bold\n");
        assert_eq!(error.line, 3);
        assert!(error.message.contains("0 to 9"), "{error}");
        assert_eq!(error.to_string(), format!("line 3: {}", error.message));
    }

    #[test]
    fn rejects_lines_it_cannot_read() {
        let cases = [
            ("text 0 \"a\"\n", "before any `size`"),
            ("size 0 5\n", "at least one"),
            ("size 4\n", "height is missing"),
            ("size 4 x\n", "not `x`"),
            ("size 4 2 1\n", "takes a width"),
            ("size 65535 65535\n", "cells"),
            ("size 4 2\nwhat 1\n", "unknown line `what`"),
            ("size 4 2\ncursor 4 0 hidden\n", "outside"),
            ("size 4 2\ncursor 0 0\n", "visible"),
            ("size 4 2\ncursor 0 0 visible round\n", "shape"),
            ("size 4 2\ntext 2 \"a\"\n", "outside the 2 rows"),
            ("size 4 2\ntext 0 \"a\"\ntext 0 \"b\"\n", "two `text`"),
            ("size 4 2\ntext 0 abc\n", "double quotes"),
            ("size 4 2\ntext 0 \"abc\n", "closing quote"),
            ("size 4 2\ntext 0 \"abc\" x\n", "more after"),
            ("size 4 2\ntext 0 \"abcde\"\n", "5 columns"),
            ("size 4 2\ntext 0 \"漢字漢\"\n", "6 columns"),
            ("size 4 2\ntext 0 \"a\\qb\"\n", "unknown escape"),
            ("size 4 2\ntext 0 \"a\\u12\"\n", "four hex"),
            ("size 4 2\ntext 0 \"\\ud83d\"\n", "low one"),
            ("size 4 2\ntext 0 \"\\udc4d\"\n", "without a high"),
            ("size 4 2\ntext 0 \"a\\n\"\n", "control character"),
            ("size 4 2\ntext 0 \"a\\u200bb\"\n", "no width"),
            ("size 4 2\nstyle 2 0 1 bold\n", "outside the 2 rows"),
            ("size 4 2\nstyle 0 2 2 bold\n", "not inside"),
            ("size 4 2\nstyle 0 0 1 loud\n", "unknown attribute `loud`"),
            ("size 4 2\nstyle 0 0 1 fg=#12345\n", "not a colour"),
            ("size 4 2\nstyle 0 0 1 fg=idx:256\n", "not a colour"),
            ("size 4 2\nstyle 0 0 1 bg=orange\n", "not a colour"),
            (
                "size 4 2\nstyle 0 0 1 underline=wavy\n",
                "unknown underline",
            ),
            ("size 4 2\nstyle 0 0 1 link=\n", "needs a URL"),
        ];
        for (input, wanted) in cases {
            let error = parse(input).expect_err(input);
            assert!(error.message.contains(wanted), "{input:?} gave {error}");
        }
    }

    #[test]
    fn frames_with_every_kind_of_style_survive_a_round_trip() {
        let mut frame = Frame::new(10, 3);
        frame.cursor = Cursor {
            x: 9,
            y: 2,
            visible: true,
            shape: CursorShape::Underline,
        };
        let link = frame.link("https://example.org/x");
        let styles = [
            Style::default()
                .fg(Color::Rgb(0x12, 0x34, 0x56))
                .bg(Color::Indexed(9)),
            Style::default()
                .fg(Color::Named(Named::LightCyan))
                .bold()
                .italic(),
            Style {
                ul: Color::Rgb(0xff, 0x00, 0x00),
                underline: Underline::Dashed,
                mods: Mods::STRIKE.with(Mods::OVERLINE).with(Mods::REVERSE),
                ..Style::default()
            },
            Style {
                link,
                ..Style::default()
            },
        ];
        for (n, style) in styles.into_iter().enumerate() {
            let y = u16::try_from(n % 3).unwrap();
            let x = u16::try_from(n / 3 * 5).unwrap() + 2 * y;
            frame.put_str(x, y, "漢ab👍🏽", style, 10);
        }
        frame.set_style(2, 8, 10, Style::default().bg(Color::Named(Named::Blue)));
        let written = write(std::slice::from_ref(&frame));
        assert_eq!(one(&written), frame, "{written}");
        assert_eq!(write(&[one(&written)]), written);
    }

    #[test]
    fn a_hidden_cursor_is_written_without_a_shape_and_read_with_one() {
        let mut frame = Frame::new(2, 1);
        frame.cursor.x = 1;
        assert!(write(std::slice::from_ref(&frame)).contains("cursor 1 0 hidden\n"));
        let read = one("size 2 1\ncursor 1 0 hidden block\n");
        assert_eq!(read.cursor.shape, CursorShape::Block);
        assert!(!read.cursor.visible);
    }

    #[test]
    fn a_visible_cursor_always_has_a_shape() {
        let mut frame = Frame::new(2, 1);
        frame.cursor.visible = true;
        let written = write(std::slice::from_ref(&frame));
        assert!(
            written.contains("cursor 0 0 visible default\n"),
            "{written}"
        );
        assert_eq!(one(&written), frame);
        assert_eq!(one("size 2 1\ncursor 0 0 visible\n"), frame);
    }

    #[test]
    fn lines_may_come_in_any_order() {
        let a = one("size 3 1\nstyle 0 0 3 bg=red\ncursor 1 0 hidden\ntext 0 \"ab\"\n");
        let b = one("size 3 1\ncursor 1 0 hidden\ntext 0 \"ab\"\nstyle 0 0 3 bg=red\n");
        assert_eq!(a, b);
    }

    #[test]
    fn a_styled_wide_character_is_one_run_over_both_cells() {
        let mut frame = Frame::new(6, 1);
        frame.put_str(0, 0, "漢字ab", Style::default().bold(), 6);
        let written = write(std::slice::from_ref(&frame));
        assert!(written.contains("style 0 0 6 bold\n"), "{written}");
        assert_eq!(one(&written), frame);
    }

    #[test]
    fn an_underline_colour_needs_no_underline() {
        let frame = one("size 2 1\nstyle 0 0 2 ul=#112233\n");
        let style = frame.cell(1, 0).unwrap().style;
        assert_eq!(style.ul, Color::Rgb(0x11, 0x22, 0x33));
        assert_eq!(style.underline, Underline::None);
        assert!(write(std::slice::from_ref(&frame)).contains("style 0 0 2 ul=#112233\n"));
    }
}
