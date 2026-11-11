//! Property tests: any input keeps the emulator within bounds, and styled text written with CUP and
//! SGR reads back cell for cell.

use proptest::prelude::*;
use tuiframe_vt::{Color, Emulator, Profile, Style, Underline};

fn profiles() -> Vec<Profile> {
    vec![
        Profile::xterm(),
        Profile::kitty(),
        Profile::konsole(),
        Profile::windows_terminal(),
        Profile::tmux_3_4(),
        Profile::linux_console(),
        Profile::silent(),
    ]
}

/// Pieces that make up terminal output, so random streams hit real sequences.
fn fragment() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        proptest::collection::vec(any::<u8>(), 1..8),
        "[a-z ]{1,8}".prop_map(String::into_bytes),
        Just(b"\x1b[".to_vec()),
        Just(b"\x1b]".to_vec()),
        Just(b"\x1bP".to_vec()),
        Just(b"\x1b".to_vec()),
        Just(b"\x07".to_vec()),
        Just(b"\x1b\\".to_vec()),
        Just(b"\r\n".to_vec()),
        Just(b"\x08\t".to_vec()),
        "[0-9;:?<>=$ ]{1,6}".prop_map(String::into_bytes),
        "[@A-Za-z`~]".prop_map(String::into_bytes),
        (0u16..300, 0u16..300).prop_map(|(y, x)| format!("\x1b[{y};{x}H").into_bytes()),
        (0u16..30, 0u16..30).prop_map(|(t, b)| format!("\x1b[{t};{b}r").into_bytes()),
        prop::sample::select(vec![
            1u16, 6, 7, 25, 47, 1000, 1003, 1006, 1047, 1048, 1049, 2004, 2026, 2027
        ])
        .prop_flat_map(|mode| prop::bool::ANY.prop_map(move |on| format!(
            "\x1b[?{mode}{}",
            if on { 'h' } else { 'l' }
        )
        .into_bytes())),
        prop::sample::select(vec![
            "字",
            "e\u{301}",
            "\u{301}",
            "\u{200d}",
            "\u{fe0f}",
            "\u{1f468}",
            "\u{1f469}",
            "\u{2764}",
            "\u{1f1fa}",
            "\u{1f3fd}",
            "…",
            "⣷",
            "\u{2460}",
        ])
        .prop_map(|s| s.as_bytes().to_vec()),
        prop::sample::select(vec![
            "\x1b[38;2;1;2;3m",
            "\x1b[4:3m",
            "\x1b[0m",
            "\x1b[41m",
            "\x1b[J",
            "\x1b[1K",
            "\x1b[S",
            "\x1b[2T",
            "\x1bM",
            "\x1b7",
            "\x1b8",
            "\x1b[>1u",
            "\x1b[<u",
            "\x1b[?u",
            "\x1b[6n",
            "\x1b[c",
            "\x1b]8;;http://a\x1b\\",
            "\x1b]2;t\x07",
            "\x1b[22;0t",
            "\x1b[23;0t",
            "\x1bc",
            "\x1b(0",
            "\x1b(B",
            "\x1b[5 q",
        ])
        .prop_map(|s| s.as_bytes().to_vec()),
    ]
}

/// Checks the cursor is on the screen and every wide cluster has exactly its continuation cells.
fn check(term: &Emulator) -> Result<(), TestCaseError> {
    let (cols, rows) = term.size();
    let cursor = term.cursor();
    prop_assert!(
        cursor.x < cols && cursor.y < rows,
        "cursor at {},{} on {cols}x{rows}",
        cursor.x,
        cursor.y
    );
    let (top, bottom) = term.scroll_region();
    prop_assert!(top < bottom || rows == 1);
    prop_assert!(bottom < rows);
    for y in 0..rows {
        let mut x = 0;
        while x < cols {
            let Some(cell) = term.cell(x, y) else {
                return Err(TestCaseError::fail(format!("no cell at {x},{y}")));
            };
            let width = u16::from(cell.width());
            prop_assert!(width > 0, "continuation without a leader at {x},{y}");
            prop_assert!(x + width <= cols, "cluster past the edge at {x},{y}");
            for i in 1..width {
                prop_assert!(
                    term.cell(x + i, y)
                        .is_some_and(tuiframe_vt::Cell::is_continuation),
                    "missing continuation at {},{y}",
                    x + i
                );
            }
            prop_assert!(!cell.text().chars().any(char::is_control));
            x += width;
        }
        prop_assert!(term.cell(cols, y).is_none());
    }
    prop_assert!(term.cell(0, rows).is_none());
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn any_input_stays_in_bounds(
        pieces in proptest::collection::vec(fragment(), 0..120),
        cols in 1u16..30,
        rows in 1u16..10,
        profile in 0..7usize,
        resize in proptest::option::of((1u16..30, 1u16..10)),
    ) {
        let mut term = Emulator::with_profile(profiles()[profile].clone(), cols, rows);
        term.set_strict(false);
        let bytes = pieces.concat();
        let middle = bytes.len() / 2;
        term.feed(&bytes[..middle]).unwrap();
        if let Some((cols, rows)) = resize {
            term.resize(cols, rows);
        }
        term.feed(&bytes[middle..]).unwrap();
        check(&term)?;
        let styled = term.styled();
        prop_assert!(styled.ends_with('\n'));
        let _ = term.to_html();
    }

    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..4000), profile in 0..7usize) {
        let mut term = Emulator::with_profile(profiles()[profile].clone(), 13, 5);
        term.set_strict(false);
        term.feed(&bytes).unwrap();
        check(&term)?;
    }
}

fn color(slot: u8) -> impl Strategy<Value = Color> {
    prop_oneof![
        Just(Color::Default),
        (0u8..16).prop_map(|i| Color::named(i).unwrap_or_default()),
        any::<u8>().prop_map(Color::Indexed),
        any::<(u8, u8, u8)>().prop_map(|(r, g, b)| Color::Rgb(r, g, b)),
    ]
    .prop_map(move |color| match (slot, color) {
        // The underline colour has no named form.
        (2, c) => c.named_index().map_or(c, Color::Indexed),
        (_, c) => c,
    })
}

fn style() -> impl Strategy<Value = Style> {
    (
        (color(0), color(1), color(2)),
        prop::sample::select(vec![
            Underline::None,
            Underline::Single,
            Underline::Double,
            Underline::Curly,
            Underline::Dotted,
            Underline::Dashed,
        ]),
        proptest::collection::vec(any::<bool>(), 9),
    )
        .prop_map(|((fg, bg, underline_color), underline, flags)| Style {
            fg,
            bg,
            underline_color,
            underline,
            bold: flags[0],
            dim: flags[1],
            italic: flags[2],
            blink: flags[3],
            rapid_blink: flags[4],
            reverse: flags[5],
            hidden: flags[6],
            strike: flags[7],
            overline: flags[8],
            link: None,
        })
}

/// SGR for a whole style from a reset, in the semicolon or colon form.
fn sgr(style: &Style, colon: bool) -> String {
    let mut params = vec!["0".to_owned()];
    for (color, base, bright, extended) in [
        (style.fg, 30, 90, 38),
        (style.bg, 40, 100, 48),
        (style.underline_color, 0, 0, 58),
    ] {
        match color {
            Color::Default => {}
            Color::Indexed(n) if colon => params.push(format!("{extended}:5:{n}")),
            Color::Indexed(n) => params.push(format!("{extended};5;{n}")),
            Color::Rgb(r, g, b) if colon => params.push(format!("{extended}:2::{r}:{g}:{b}")),
            Color::Rgb(r, g, b) => params.push(format!("{extended};2;{r};{g};{b}")),
            named => {
                let i = named.named_index().unwrap_or_default();
                params.push(if i < 8 {
                    (base + i).to_string()
                } else {
                    (bright + i - 8).to_string()
                });
            }
        }
    }
    match style.underline {
        Underline::None => {}
        Underline::Single => params.push("4".into()),
        Underline::Double if colon => params.push("4:2".into()),
        Underline::Double => params.push("21".into()),
        Underline::Curly => params.push("4:3".into()),
        Underline::Dotted => params.push("4:4".into()),
        Underline::Dashed => params.push("4:5".into()),
    }
    for (on, p) in [
        (style.bold, "1"),
        (style.dim, "2"),
        (style.italic, "3"),
        (style.blink, "5"),
        (style.rapid_blink, "6"),
        (style.reverse, "7"),
        (style.hidden, "8"),
        (style.strike, "9"),
        (style.overline, "53"),
    ] {
        if on {
            params.push(p.into());
        }
    }
    format!("\x1b[{}m", params.join(";"))
}

const CLUSTERS: [(&str, u16); 8] = [
    ("a", 1),
    ("Z", 1),
    ("é", 1),
    ("e\u{301}", 1),
    ("字", 2),
    ("\u{1f600}", 2),
    ("…", 1),
    ("\u{1f1fa}\u{1f1f8}", 2),
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn styled_text_reads_back_cell_for_cell(
        cols in 2u16..24,
        rows in 1u16..6,
        items in proptest::collection::vec((0..CLUSTERS.len(), style(), any::<bool>(), any::<bool>()), 1..200),
    ) {
        let mut term = Emulator::new(cols, rows);
        let mut bytes = b"\x1b[?7l".to_vec();
        let mut expected = Vec::new();
        let mut items = items.into_iter();
        'rows: for y in 0..rows {
            bytes.extend_from_slice(format!("\x1b[{};1H", y + 1).as_bytes());
            let mut x = 0;
            while x < cols {
                let Some((which, style, jump, colon)) = items.next() else { break 'rows };
                let (text, width) = if CLUSTERS[which].1 + x > cols { CLUSTERS[0] } else { CLUSTERS[which] };
                if jump {
                    bytes.extend_from_slice(format!("\x1b[{};{}H", y + 1, x + 1).as_bytes());
                }
                bytes.extend_from_slice(sgr(&style, colon).as_bytes());
                bytes.extend_from_slice(text.as_bytes());
                expected.push((x, y, text, width, style));
                x += width;
            }
        }
        term.feed(&bytes).unwrap();
        for (x, y, text, width, style) in expected {
            let cell = term.cell(x, y).unwrap();
            prop_assert_eq!(cell.text(), text);
            prop_assert_eq!(u16::from(cell.width()), width);
            prop_assert_eq!(cell.style(), &style);
            for i in 1..width {
                let continuation = term.cell(x + i, y).unwrap();
                prop_assert!(continuation.is_continuation());
                prop_assert_eq!(continuation.style(), &style);
            }
        }
    }
}
