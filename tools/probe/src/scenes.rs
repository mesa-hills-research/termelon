//! The probe's scenes, drawn with the framework. Each fits in 80 by 24 cells and ends with its
//! marker on the last row, which the harness waits for.

use std::ops::Range;

use termelon::feed::Measure;
use termelon::{
    Buffer, Color, Cursor, CursorShape, FeedRows, FeedView, Modifier, Position, Rect, Style,
    UnderlineStyle,
};

/// A scene: its name, how it draws, and where it leaves the cursor (`None` hides it).
pub(crate) struct Scene {
    /// The name the harness asks for.
    pub(crate) name: &'static str,
    draw: fn(&mut Buffer),
    /// Frames drawn before the scene's own, which has the marker.
    pub(crate) before: &'static [fn(&mut Buffer)],
    /// The cursor the frame asks for.
    pub(crate) cursor: Option<Cursor>,
}

/// Every scene.
pub(crate) const SCENES: [Scene; 4] = [
    Scene {
        name: "text",
        draw: text,
        before: &[],
        cursor: None,
    },
    Scene {
        name: "styles",
        draw: styles,
        before: &[],
        cursor: None,
    },
    Scene {
        name: "cursor",
        draw: cursor,
        before: &[],
        cursor: Some(Cursor {
            position: Position::new(20, 10),
            shape: CursorShape::Bar,
            blinking: false,
        }),
    },
    Scene {
        name: "scroll",
        draw: |b| scroll(b, 2),
        before: &[|b| scroll(b, 0), |b| scroll(b, 1)],
        cursor: None,
    },
];

/// The scene with a name.
pub(crate) fn find(name: &str) -> Option<&'static Scene> {
    SCENES.iter().find(|scene| scene.name == name)
}

/// The marker the harness waits for.
pub(crate) fn marker(scene: &Scene) -> String {
    format!("probe-done:{}", scene.name)
}

/// Draws a scene and its marker into a buffer.
pub(crate) fn draw(scene: &Scene, buffer: &mut Buffer) {
    (scene.draw)(buffer);
    let bottom = buffer.area().height.saturating_sub(1);
    buffer.set_string(0, bottom, marker(scene), Style::new());
}

fn text(b: &mut Buffer) {
    let plain = Style::new();
    b.set_string(0, 0, "probe: text", plain);
    b.set_string(
        0,
        2,
        "plain ASCII, punctuation !\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~",
        plain,
    );
    b.set_string(0, 3, "Latin-1: café ñ ß ü ¿", plain);
    b.set_string(0, 4, "CJK: 字幕 한국어 カタカナ", plain);
    b.set_string(0, 5, "Combining: e\u{301} a\u{308} n\u{303}", plain);
    b.set_string(0, 6, "Box: ┌──┐ │ok│ └──┘ ═ ║", plain);
    b.set_string(0, 7, "Symbols: ⣷ … → ✓ • ± °", plain);
    b.set_string(39, 9, "column 40", plain);
    let right = b.area().width.saturating_sub(5);
    b.set_string(right, 11, "edge!", plain);
}

fn styles(b: &mut Buffer) {
    let plain = Style::new();
    let mut x = 0;
    for (word, modifier) in [
        ("bold", Modifier::BOLD),
        ("dim", Modifier::DIM),
        ("italic", Modifier::ITALIC),
        ("underline", Modifier::UNDERLINED),
        ("reverse", Modifier::REVERSED),
        ("strike", Modifier::CROSSED_OUT),
        ("blink", Modifier::SLOW_BLINK),
        ("overline", Modifier::OVERLINED),
    ] {
        b.set_string(x, 0, word, plain.add_modifier(modifier));
        x += u16::try_from(word.len()).unwrap_or(0) + 1;
    }
    let mut x = 0;
    for (word, shape, color) in [
        ("double", UnderlineStyle::Double, Color::Reset),
        ("curly", UnderlineStyle::Curly, Color::Reset),
        ("dotted", UnderlineStyle::Dotted, Color::Reset),
        ("dashed", UnderlineStyle::Dashed, Color::Reset),
        ("red line", UnderlineStyle::Single, Color::Rgb(255, 0, 0)),
        ("indexed line", UnderlineStyle::Single, Color::Indexed(33)),
    ] {
        let style = plain
            .add_modifier(Modifier::UNDERLINED)
            .underline_style(shape)
            .underline_color(color);
        b.set_string(x, 1, word, style);
        x += u16::try_from(word.chars().count()).unwrap_or(0) + 1;
    }
    let named = [
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
    for (i, &color) in (0u16..).zip(named.iter()) {
        b.set_string(i * 3, 2, format!("{i:>2}"), plain.fg(color));
        b.set_string(i * 3, 3, format!("{i:>2}"), plain.bg(color));
    }
    b.set_string(0, 4, "idx1", plain.fg(Color::Indexed(1)));
    b.set_string(5, 4, "idx196", plain.fg(Color::Indexed(196)));
    b.set_string(12, 4, " bg236 ", plain.bg(Color::Indexed(236)));
    b.set_string(0, 5, "rgb fg", plain.fg(Color::Rgb(216, 222, 233)));
    b.set_string(7, 5, " rgb bg ", plain.bg(Color::Rgb(30, 40, 50)));
    let stacked = plain
        .fg(Color::Red)
        .add_modifier(Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED);
    b.set_string(0, 6, "stacked", stacked);
    b.set_string(
        8,
        6,
        "bold dim",
        plain.add_modifier(Modifier::BOLD | Modifier::DIM),
    );
    b.set_string(0, 7, "a link", plain);
    b.set_link(Rect::new(0, 7, 6, 1), Some("https://example.com/docs"));
    b.set_string(7, 7, "after", plain);
    b.set_string(0, 8, "  blue  ", plain.bg(Color::Blue));
    b.set_string(
        10,
        8,
        " green reversed ",
        plain.fg(Color::Green).add_modifier(Modifier::REVERSED),
    );
}

fn cursor(b: &mut Buffer) {
    b.set_string(0, 0, "the cursor ends at 20,10 as a bar", Style::new());
}

/// The words of the scroll scene's rows.
const WORDS: [&str; 6] = ["rows", "move", "up", "and", "down", "here"];

/// Items of one to three rows, some in colour, for the scroll scene's feed.
struct Rows;

impl Measure<u16> for Rows {
    fn height(&mut self, key: &u16) -> u32 {
        u32::from(key % 3 + 1)
    }
}

impl FeedRows<u16> for Rows {
    fn set_width(&mut self, _width: u16) {}

    fn render(&mut self, key: &u16, rows: Range<u32>, area: Rect, buf: &mut Buffer) {
        let style = match key % 5 {
            0 => Style::new().fg(Color::Cyan),
            3 => Style::new().bg(Color::Indexed(236)),
            _ => Style::new(),
        };
        for (y, row) in (area.y..).zip(rows) {
            let n = usize::from(*key) * 3 + row as usize;
            let words: Vec<&str> = (0..6).map(|i| WORDS[(n + i * 5) % WORDS.len()]).collect();
            let text = format!("item {key:02}.{row} {}", words.join(" "));
            buf.set_stringn(area.x, y, text, usize::from(area.width), style);
        }
    }
}

/// A feed between a title and a column of notes, three rows down from its start in frame 1 and
/// one back up in frame 2, so the terminal scrolls a region one way and then the other. The
/// notes share the feed's rows, so they're written again after each scroll.
fn scroll(b: &mut Buffer, frame: u8) {
    let area = b.area();
    let feed = Rect::new(0, 1, 60, area.height.saturating_sub(2));
    let mut view = FeedView::new();
    for key in 0..60 {
        view.feed_mut().push(key, 1);
    }
    view.feed_mut().scroll_to_start();
    let mut rows = Rows;
    let mut scratch = Buffer::empty(area);
    for step in 0..=frame {
        match step {
            1 => view.feed_mut().scroll_by(3),
            2 => view.feed_mut().scroll_by(-1),
            _ => {}
        }
        let target = if step == frame { &mut *b } else { &mut scratch };
        target.set_string(0, 0, "probe: scroll", Style::new());
        view.render(feed, target, &mut rows);
        for y in feed.top()..feed.bottom() {
            target.set_string(
                62,
                y,
                format!("note {y:02}"),
                Style::new().fg(Color::Yellow),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each scene draws with sequences `termelon-vt` knows, and shows its text and marker there.
    #[test]
    fn each_scene_shows_in_the_emulator() {
        for scene in &SCENES {
            let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 24));
            draw(scene, &mut buffer);
            let mut term = termelon::testing::TestTerminal::new(80, 24);
            for before in scene.before {
                term.draw(|frame| before(frame.buffer_mut())).unwrap();
            }
            term.take_output();
            term.draw(|frame| {
                draw(scene, frame.buffer_mut());
                frame.set_cursor(scene.cursor);
            })
            .unwrap();
            // The scroll scene's last frame scrolls a region, in a terminal that has them.
            if !scene.before.is_empty() {
                let bytes = String::from_utf8_lossy(&term.take_output()).into_owned();
                assert!(bytes.contains("\x1b[2;23r"), "{bytes:?}");
                assert!(!bytes.contains("item 03.0"), "{bytes:?}");
            }
            let rows = term.screen().rows();
            for (y, shown) in (0..24).zip(&rows) {
                let row: String = (0..80).map(|x| buffer.symbol((x, y)).to_owned()).collect();
                assert_eq!(shown, row.trim_end(), "{} row {y}", scene.name);
            }
            assert!(term.screen().text().contains(&marker(scene)));
        }
    }
}
