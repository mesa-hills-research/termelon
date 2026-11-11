//! The probe's scenes, drawn with the framework. Each fits in 80 by 24 cells and ends with its
//! marker on the last row, which the harness waits for.

use tuiframe::{
    Buffer, Color, Cursor, CursorShape, Modifier, Position, Rect, Style, UnderlineStyle,
};

/// A scene: its name, how it draws, and where it leaves the cursor (`None` hides it).
pub(crate) struct Scene {
    /// The name the harness asks for.
    pub(crate) name: &'static str,
    draw: fn(&mut Buffer),
    /// The cursor the frame asks for.
    pub(crate) cursor: Option<Cursor>,
}

/// Every scene.
pub(crate) const SCENES: [Scene; 3] = [
    Scene {
        name: "text",
        draw: text,
        cursor: None,
    },
    Scene {
        name: "styles",
        draw: styles,
        cursor: None,
    },
    Scene {
        name: "cursor",
        draw: cursor,
        cursor: Some(Cursor {
            position: Position::new(20, 10),
            shape: CursorShape::Bar,
            blinking: false,
        }),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_scene_reads_back_from_its_recording() {
        for scene in &SCENES {
            let mut buffer = Buffer::empty(Rect::new(0, 0, 80, 24));
            draw(scene, &mut buffer);
            let mut term = tuiframe_vt::Emulator::new(80, 24);
            let allowed = crate::record::Allowed::ALL;
            term.feed(&crate::record::render(&buffer, scene.cursor, allowed))
                .unwrap();
            for y in 0..24 {
                let row: String = (0..80).map(|x| buffer.symbol((x, y)).to_owned()).collect();
                assert_eq!(
                    term.row_text(y).unwrap(),
                    row.trim_end(),
                    "{} row {y}",
                    scene.name
                );
            }
            assert!(term.text().contains(&marker(scene)));
        }
    }
}
