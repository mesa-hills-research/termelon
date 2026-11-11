//! Turning what terminals report about their screens into emulator screens.
//!
//! tmux's `capture-pane -e`, kitty's `get-text --ansi` and WezTerm's `get-text --escapes` give the
//! rows as text with SGR and OSC 8. xterm's `CSI 10 i` dump is XHTML with inline styles, and GNU
//! screen's hardcopy is text with one byte per cell. Each becomes rows fed to a `termelon-vt`
//! emulator, whose cells can then be compared with the screen it computes for the program's bytes.

use std::fmt::Write as _;

use termelon_vt::{Color, Emulator, Profile, WidthRules};

use crate::driver::Error;

/// The profile a readback is parsed with: one that keeps everything a readback can say.
fn readback_profile(width: WidthRules) -> Profile {
    Profile {
        name: "readback".to_owned(),
        width,
        hyperlinks: true,
        ..Profile::xterm()
    }
}

fn feed(screen: &mut Emulator, bytes: &[u8]) -> Result<(), Error> {
    screen
        .feed(bytes)
        .map_err(|error| Error::Parse(error.to_string()))
}

/// Rows of text with SGR and OSC 8, one per line, rebuilt as a screen of `cols` by `rows`.
///
/// The SGR state carries from one line to the next, as tmux writes it. Lines past `rows` are
/// ignored, and a `\r` before a line's end is dropped.
///
/// # Errors
///
/// When the text holds a sequence `termelon-vt` doesn't know.
pub fn ansi_screen(
    text: &[u8],
    cols: u16,
    rows: u16,
    width: WidthRules,
) -> Result<Emulator, Error> {
    let mut screen = Emulator::with_profile(readback_profile(width), cols, rows);
    feed(&mut screen, b"\x1b[?7l")?;
    for (y, line) in text.split(|&b| b == b'\n').enumerate() {
        let Ok(y) = u16::try_from(y) else { break };
        if y >= rows {
            break;
        }
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        feed(&mut screen, format!("\x1b[{};1H", y + 1).as_bytes())?;
        feed(&mut screen, &fix_tmux_overline(line))?;
    }
    Ok(screen)
}

/// tmux 3.6 writes overline as `5:3` in `capture-pane -e`, where SGR has 53.
fn fix_tmux_overline(line: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = find(rest, b"\x1b[") {
        out.extend_from_slice(&rest[..start + 2]);
        rest = &rest[start + 2..];
        let end = rest
            .iter()
            .position(|&b| !(b.is_ascii_digit() || b == b';' || b == b':'));
        if let Some(end) = end
            && rest[end] == b'm'
        {
            let params: Vec<&[u8]> = rest[..end].split(|&b| b == b';').collect();
            let fixed: Vec<&[u8]> = params
                .into_iter()
                .map(|p| if p == b"5:3" { &b"53"[..] } else { p })
                .collect();
            out.extend_from_slice(&fixed.join(&b';'));
            rest = &rest[end..];
        }
    }
    out.extend_from_slice(rest);
    out
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Sets the cursor of a rebuilt screen.
///
/// # Errors
///
/// Never in practice, since the sequences are ones `termelon-vt` knows.
pub fn set_cursor(
    screen: &mut Emulator,
    x: u16,
    y: u16,
    visible: bool,
    shape: Option<u16>,
) -> Result<(), Error> {
    let mut bytes = format!(
        "\x1b[{};{}H\x1b[?25{}",
        y + 1,
        x + 1,
        if visible { 'h' } else { 'l' }
    );
    if let Some(shape) = shape {
        let _ = write!(bytes, "\x1b[{shape} q");
    }
    feed(screen, bytes.as_bytes())
}

/// GNU screen's hardcopy, one line per row. Screen 4.9 writes one byte per cell, the low byte of
/// the character, and 0xFF in the second cell of a wide one. This reads valid UTF-8 as UTF-8 and
/// anything else as Latin-1, and puts U+FFFD for a control byte, so every byte keeps its cell.
///
/// # Errors
///
/// Never in practice, since the text holds no sequences.
pub fn hardcopy_screen(bytes: &[u8], cols: u16, rows: u16) -> Result<Emulator, Error> {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => bytes.iter().map(|&b| char::from(b)).collect(),
    };
    let cells: String = text
        .chars()
        .map(|c| {
            if c != '\n' && c.is_control() {
                char::REPLACEMENT_CHARACTER
            } else {
                c
            }
        })
        .collect();
    ansi_screen(cells.as_bytes(), cols, rows, WidthRules::default())
}

/// The default colours and the rows of an HTML screen dump, xterm's `CSI 10 i` or VTE's
/// `get_text_format`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HtmlDump {
    /// The default foreground, from the dump's outer style, if it gives one.
    pub foreground: Option<(u8, u8, u8)>,
    /// The default background.
    pub background: Option<(u8, u8, u8)>,
    /// The screen.
    pub ansi: Vec<u8>,
}

/// Reads an HTML screen dump: the text inside `<pre>`, with colours as `#rrggbb` or `rgb()` and
/// attributes as CSS in `<span style>`, as xterm's classes (`bd`, `it`, `ul`, `st` and `lu`) or as
/// `<b>`, `<i>`, `<u>`, `<strike>`, `<blink>` and `<font color>` (VTE). The runs become SGR with
/// RGB colours, and a colour equal to the dump's default becomes the default.
///
/// # Errors
///
/// When the dump has no `<pre>`.
pub fn html_dump(html: &str) -> Result<HtmlDump, Error> {
    let pre_start = html
        .find("<pre")
        .ok_or_else(|| Error::Parse("the dump has no <pre>".to_owned()))?;
    let open_end = html[pre_start..]
        .find('>')
        .map_or(html.len(), |i| pre_start + i + 1);
    let close = html[open_end..]
        .find("</pre>")
        .map_or(html.len(), |i| open_end + i);
    // The default colours are on the <pre> or on an element around it.
    let outer = &html[..open_end];
    let defaults = outer
        .rfind("style=")
        .map(|i| css(attribute_value(&outer[i + 6..])))
        .unwrap_or_default();
    let (foreground, background) = (defaults.fg, defaults.bg);
    let body = &html[open_end..close];
    let mut ansi = Vec::new();
    let mut stack: Vec<Css> = Vec::new();
    let mut rest = body.strip_prefix('\n').unwrap_or(body);
    while !rest.is_empty() {
        if rest.starts_with("</") {
            stack.pop();
            rest = rest.find('>').map_or("", |end| &rest[end + 1..]);
        } else if rest.starts_with('<') {
            let end = rest.find('>').unwrap_or(rest.len());
            let tag = &rest[1..end];
            rest = rest.get(end + 1..).unwrap_or("");
            if tag.ends_with('/') || tag.starts_with('!') || tag.starts_with('?') {
                continue;
            }
            let inherited = stack.last().cloned().unwrap_or_default();
            stack.push(inherited.with(&tag_css(tag)));
        } else {
            let end = rest.find('<').unwrap_or(rest.len());
            let text = unescape(&rest[..end]);
            let style = stack.last().cloned().unwrap_or_default();
            for (i, line) in text.split('\n').enumerate() {
                if i > 0 {
                    ansi.extend_from_slice(b"\x1b[0m\n");
                }
                if !line.is_empty() {
                    ansi.extend_from_slice(style.sgr(foreground, background).as_bytes());
                    ansi.extend_from_slice(line.as_bytes());
                }
            }
            rest = &rest[end..];
        }
    }
    Ok(HtmlDump {
        foreground,
        background,
        ansi,
    })
}

/// What an opening tag says about the text inside it: its `style`, xterm's classes, and VTE's
/// tags, `<b>`, `<i>`, `<u>`, `<strike>`, `<blink>` and `<font color>`.
fn tag_css(tag: &str) -> Css {
    let name = tag
        .split(|c: char| c.is_whitespace() || c == '/')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let mut out = attribute(tag, "style").map(css).unwrap_or_default();
    // xterm's `html.c` writes bold, italic, underline and strike as classes.
    for class in attribute(tag, "class").unwrap_or("").split_whitespace() {
        match class {
            "bd" => out.bold = true,
            "it" => out.italic = true,
            "ul" => out.underline = true,
            "st" => out.strike = true,
            "lu" => (out.underline, out.strike) = (true, true),
            _ => {}
        }
    }
    match name.as_str() {
        "b" | "strong" => out.bold = true,
        "i" | "em" => out.italic = true,
        "u" => out.underline = true,
        "s" | "strike" | "del" => out.strike = true,
        "blink" => out.blink = true,
        "font" => out.fg = attribute(tag, "color").and_then(hex_color).or(out.fg),
        _ => {}
    }
    out
}

/// An attribute's value in a tag's text, in single or double quotes.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    loop {
        let at = rest.find(name)?;
        let before = rest[..at].chars().next_back();
        let after = &rest[at + name.len()..];
        if before.is_some_and(char::is_whitespace) && after.starts_with('=') {
            return Some(attribute_value(&after[1..]));
        }
        rest = &rest[at + name.len()..];
    }
}

/// The value of an HTML attribute, in single or double quotes, at the start of `text`.
fn attribute_value(text: &str) -> &str {
    let Some(quote) = text.chars().next().filter(|&c| c == '\'' || c == '"') else {
        return "";
    };
    let body = &text[1..];
    body.find(quote).map_or(body, |end| &body[..end])
}

/// What a dump's CSS says about a run.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Css {
    fg: Option<(u8, u8, u8)>,
    bg: Option<(u8, u8, u8)>,
    bold: bool,
    italic: bool,
    underline: bool,
    /// The underline's style, SGR 4's sub-parameter.
    underline_style: Option<u8>,
    underline_color: Option<(u8, u8, u8)>,
    strike: bool,
    overline: bool,
    blink: bool,
}

impl Css {
    fn with(&self, inner: &Css) -> Css {
        Css {
            fg: inner.fg.or(self.fg),
            bg: inner.bg.or(self.bg),
            bold: self.bold || inner.bold,
            italic: self.italic || inner.italic,
            underline: self.underline || inner.underline,
            underline_style: inner.underline_style.or(self.underline_style),
            underline_color: inner.underline_color.or(self.underline_color),
            strike: self.strike || inner.strike,
            overline: self.overline || inner.overline,
            blink: self.blink || inner.blink,
        }
    }

    fn sgr(&self, default_fg: Option<(u8, u8, u8)>, default_bg: Option<(u8, u8, u8)>) -> String {
        let mut params = vec!["0".to_owned()];
        if let Some((r, g, b)) = self.fg.filter(|&c| Some(c) != default_fg) {
            params.push(format!("38;2;{r};{g};{b}"));
        }
        if let Some((r, g, b)) = self.bg.filter(|&c| Some(c) != default_bg) {
            params.push(format!("48;2;{r};{g};{b}"));
        }
        for (on, p) in [
            (self.bold, "1"),
            (self.italic, "3"),
            (self.blink, "5"),
            (self.strike, "9"),
            (self.overline, "53"),
        ] {
            if on {
                params.push(p.to_owned());
            }
        }
        if self.underline {
            params.push(format!("4:{}", self.underline_style.unwrap_or(1)));
            if let Some((r, g, b)) = self.underline_color {
                params.push(format!("58;2;{r};{g};{b}"));
            }
        }
        format!("\x1b[{}m", params.join(";"))
    }
}

fn css(style: &str) -> Css {
    let mut out = Css::default();
    for declaration in style.split(';') {
        let Some((property, value)) = declaration.split_once(':') else {
            continue;
        };
        let (property, value) = (property.trim(), value.trim());
        match property {
            "color" => out.fg = css_color(value),
            "background" | "background-color" => out.bg = css_color(value),
            "font-weight" => {
                out.bold = value == "bold" || value.parse::<u16>().is_ok_and(|w| w >= 600);
            }
            "font-style" => out.italic = value == "italic" || value == "oblique",
            "text-decoration" | "text-decoration-line" => {
                for word in value.split_whitespace() {
                    match word {
                        "underline" => out.underline = true,
                        "line-through" => out.strike = true,
                        "overline" => out.overline = true,
                        "blink" => out.blink = true,
                        _ => {}
                    }
                }
            }
            "text-decoration-style" => {
                out.underline_style = match value {
                    "solid" => Some(1),
                    "double" => Some(2),
                    "wavy" => Some(3),
                    "dotted" => Some(4),
                    "dashed" => Some(5),
                    _ => None,
                };
            }
            "text-decoration-color" => out.underline_color = css_color(value),
            _ => {}
        }
    }
    out
}

/// A colour as `#rrggbb`, `#rgb`, or `rgb()` with numbers or percentages, as xterm writes it.
fn css_color(value: &str) -> Option<(u8, u8, u8)> {
    let Some(args) = value
        .strip_prefix("rgb(")
        .and_then(|rest| rest.strip_suffix(')'))
    else {
        return hex_color(value);
    };
    let channel = |part: &str| -> Option<u8> {
        let part = part.trim();
        let Some(percent) = part.strip_suffix('%') else {
            return part.parse().ok();
        };
        // In hundredths of a percent, so 80.39% is 8039.
        let (whole, fraction) = percent.split_once('.').unwrap_or((percent, ""));
        if whole.len() > 3 {
            return None;
        }
        let fraction = format!("{fraction:0<2}");
        let hundredths =
            whole.parse::<u32>().ok()? * 100 + fraction.get(..2)?.parse::<u32>().ok()?;
        u8::try_from((hundredths * 255 + 5000) / 10_000).ok()
    };
    let mut parts = args.split(',');
    let rgb = (
        channel(parts.next()?)?,
        channel(parts.next()?)?,
        channel(parts.next()?)?,
    );
    parts.next().is_none().then_some(rgb)
}

fn hex_color(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.strip_prefix('#')?;
    let channel = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    match hex.len() {
        6 => Some((channel(0)?, channel(2)?, channel(4)?)),
        3 => {
            let short = |i: usize| u8::from_str_radix(hex.get(i..=i)?, 16).ok().map(|v| v * 17);
            Some((short(0)?, short(1)?, short(2)?))
        }
        _ => None,
    }
}

/// HTML's entities: the five named ones and numeric references.
fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];
        let Some(semi) = rest.find(';') else { break };
        let entity = &rest[1..semi];
        let decoded = match entity {
            "lt" => Some('<'),
            "gt" => Some('>'),
            "amp" => Some('&'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        if let Some(c) = decoded {
            out.push(c);
            rest = &rest[semi + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Pulls integer and string fields out of the first object of WezTerm's `cli list --format json`,
/// for the cursor. The full output is a list of panes with many more fields.
pub fn json_field<'a>(json: &'a str, field: &str) -> Option<&'a str> {
    let key = format!("\"{field}\"");
    let at = json.find(&key)? + key.len();
    let rest = json[at..].trim_start().strip_prefix(':')?.trim_start();
    if let Some(string) = rest.strip_prefix('"') {
        return string.find('"').map(|end| &string[..end]);
    }
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .unwrap_or(rest.len());
    Some(&rest[..end])
}

/// A colour in xterm's palette, with the default resolved to a terminal's defaults.
pub fn resolve(color: Color, default: (u8, u8, u8)) -> (u8, u8, u8) {
    color.rgb().unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tmux_overline_becomes_sgr_53() {
        assert_eq!(
            fix_tmux_overline(b"a\x1b[5:3mb\x1b[1;5:3;4:3mc"),
            b"a\x1b[53mb\x1b[1;53;4:3mc"
        );
        assert_eq!(fix_tmux_overline(b"\x1b]8;;x\x1b\\"), b"\x1b]8;;x\x1b\\");
    }

    #[test]
    fn css_and_entities() {
        let style = css(
            "color:#cd0000; background-color: #fff; font-weight:bold; text-decoration: underline line-through",
        );
        assert_eq!(style.fg, Some((0xcd, 0, 0)));
        assert_eq!(style.bg, Some((0xff, 0xff, 0xff)));
        assert!(style.bold && style.underline && style.strike && !style.italic);
        assert_eq!(unescape("a&lt;b&amp;c&#x41;&#66;&bogus"), "a<b&cAB&bogus");
    }

    /// xterm writes colours as percentages of 65535 with two decimals.
    #[test]
    fn rgb_colours() {
        assert_eq!(
            css_color("rgb(80.39%, 0.00%, 100.00%)"),
            Some((0xcd, 0, 0xff))
        );
        assert_eq!(css_color("rgb(1, 2, 3)"), Some((1, 2, 3)));
        assert_eq!(css_color("rgb(1, 2)"), None);
        assert_eq!(css_color("#0000ee"), Some((0, 0, 0xee)));
        let tag = "span class=' od bd lu it ' style='color: rgb(100.00%, 0.00%, 0.00%);'";
        let style = tag_css(tag);
        assert!(style.bold && style.italic && style.underline && style.strike);
        assert_eq!(style.fg, Some((0xff, 0, 0)));
    }

    #[test]
    fn json_fields() {
        let json = r#"[{"window_id":0,"cursor_x":12, "cursor_y" : 3,"cursor_shape":"Default","cursor_visibility":"Visible"}]"#;
        assert_eq!(json_field(json, "cursor_x"), Some("12"));
        assert_eq!(json_field(json, "cursor_y"), Some("3"));
        assert_eq!(json_field(json, "cursor_visibility"), Some("Visible"));
        assert_eq!(json_field(json, "missing"), None);
    }
}
