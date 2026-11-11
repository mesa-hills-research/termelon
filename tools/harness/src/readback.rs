//! Turning what terminals report about their screens into emulator screens.
//!
//! tmux's `capture-pane -e`, kitty's `get-text --ansi` and WezTerm's `get-text --escapes` give the
//! rows as text with SGR and OSC 8. xterm's `CSI 10 i` dump is XHTML with inline styles, and GNU
//! screen's hardcopy is text with one byte per cell. Each becomes rows fed to a `tuiframe-vt`
//! emulator, whose cells can then be compared with the screen it computes for the program's bytes.

use std::fmt::Write as _;

use tuiframe_vt::{Color, Emulator, Profile, WidthRules};

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
/// When the text holds a sequence `tuiframe-vt` doesn't know.
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
/// Never in practice, since the sequences are ones `tuiframe-vt` knows.
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

/// The default colours and the rows of xterm's `CSI 10 i` XHTML dump.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XtermDump {
    /// The default foreground, from the dump's outer style, if it gives one.
    pub foreground: Option<(u8, u8, u8)>,
    /// The default background.
    pub background: Option<(u8, u8, u8)>,
    /// The screen.
    pub ansi: Vec<u8>,
}

/// Reads xterm's XHTML dump: the text inside `<pre>`, with `<span style>` runs giving colours as
/// `#rrggbb` and attributes as CSS. The runs become SGR with RGB colours, and a colour equal to the
/// dump's default becomes the default.
///
/// # Errors
///
/// When the dump has no `<pre>`.
pub fn xterm_html(html: &str) -> Result<XtermDump, Error> {
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
        if let Some(tag) = rest.strip_prefix("<span") {
            let end = tag.find('>').unwrap_or(tag.len());
            let style = tag[..end]
                .find("style=")
                .map(|i| css(attribute_value(&tag[i + 6..end])))
                .unwrap_or_default();
            let inherited = stack.last().cloned().unwrap_or_default();
            stack.push(inherited.with(&style));
            rest = tag.get(end + 1..).unwrap_or("");
        } else if let Some(after) = rest.strip_prefix("</span>") {
            stack.pop();
            rest = after;
        } else if rest.starts_with('<') {
            let end = rest.find('>').map_or(rest.len(), |i| i + 1);
            rest = &rest[end..];
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
    Ok(XtermDump {
        foreground,
        background,
        ansi,
    })
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
            (self.underline, "4"),
            (self.blink, "5"),
            (self.strike, "9"),
            (self.overline, "53"),
        ] {
            if on {
                params.push(p.to_owned());
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
            "color" => out.fg = hex_color(value),
            "background" | "background-color" => out.bg = hex_color(value),
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
            _ => {}
        }
    }
    out
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

    #[test]
    fn json_fields() {
        let json = r#"[{"window_id":0,"cursor_x":12, "cursor_y" : 3,"cursor_shape":"Default","cursor_visibility":"Visible"}]"#;
        assert_eq!(json_field(json, "cursor_x"), Some("12"));
        assert_eq!(json_field(json, "cursor_y"), Some("3"));
        assert_eq!(json_field(json, "cursor_visibility"), Some("Visible"));
        assert_eq!(json_field(json, "missing"), None);
    }
}
