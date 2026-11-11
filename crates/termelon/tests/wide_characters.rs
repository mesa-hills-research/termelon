//! Any sequence of writes leaves the buffer's wide-character invariant whole. A wide character
//! always has all of its continuation cells, and no continuation cell is ever without its
//! character, at every position, the right edge included.

use proptest::prelude::*;

use termelon::buffer::Buffer;
use termelon::caps::WidthProfile;
use termelon::layout::Rect;
use termelon::style::{Color, Modifier, Style};
use termelon::text::{self, Line, Span};

/// Pieces of text that exercise every kind of cluster: narrow, wide, combining, emoji sequences
/// of several widths, ambiguous, controls and zero-width characters.
const PIECES: &[&str] = &[
    "a",
    "xy",
    " ",
    "界",
    "界a",
    "ｗｉｄｅ",
    "e\u{301}",
    "\u{301}",
    "👍",
    "👍🏽",
    "🇳🇿",
    "❤\u{fe0f}",
    "1\u{fe0f}\u{20e3}",
    "👨\u{200d}👩\u{200d}👧",
    "🧑🏻\u{200d}🤝\u{200d}🧑🏼",
    "±",
    "\t",
    "\u{1b}[31m",
    "\u{200b}",
    "\r\n",
];

#[derive(Debug, Clone)]
enum Op {
    String {
        x: u16,
        y: u16,
        text: String,
        max: usize,
    },
    Line {
        x: u16,
        y: u16,
        spans: Vec<String>,
        max: u16,
    },
    Symbol {
        x: u16,
        y: u16,
        text: String,
    },
    Reset {
        x: u16,
        y: u16,
    },
    Style {
        area: Rect,
    },
    Link {
        area: Rect,
    },
    Clear,
}

const WIDTH: u16 = 9;
const HEIGHT: u16 = 3;

fn piece_text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 0..5).prop_map(|p| p.concat())
}

fn op() -> impl Strategy<Value = Op> {
    let x = 0..WIDTH + 2;
    let y = 0..HEIGHT + 1;
    let rect = (0..WIDTH + 1, 0..HEIGHT + 1, 0..WIDTH + 2, 0..HEIGHT + 2)
        .prop_map(|(x, y, w, h)| Rect::new(x, y, w, h));
    prop_oneof![
        4 => (x.clone(), y.clone(), piece_text(), 0..12usize)
            .prop_map(|(x, y, text, max)| Op::String { x, y, text, max }),
        2 => (x.clone(), y.clone(), prop::collection::vec(piece_text(), 0..4), 0..12u16)
            .prop_map(|(x, y, spans, max)| Op::Line { x, y, spans, max }),
        3 => (x.clone(), y.clone(), prop::sample::select(PIECES))
            .prop_map(|(x, y, text)| Op::Symbol { x, y, text: text.to_owned() }),
        1 => (x, y).prop_map(|(x, y)| Op::Reset { x, y }),
        1 => rect.clone().prop_map(|area| Op::Style { area }),
        1 => rect.prop_map(|area| Op::Link { area }),
        1 => Just(Op::Clear),
    ]
}

fn profile() -> impl Strategy<Value = WidthProfile> {
    (any::<bool>(), any::<bool>(), any::<bool>()).prop_map(|(j, v, a)| WidthProfile::new(j, v, a))
}

fn apply(buffer: &mut Buffer, op: &Op) {
    let style = Style::new().fg(Color::Red).add_modifier(Modifier::BOLD);
    match op {
        Op::String { x, y, text, max } => {
            buffer.set_stringn(*x, *y, text, *max, style);
        }
        Op::Line { x, y, spans, max } => {
            let line = Line::from(
                spans
                    .iter()
                    .map(|s| Span::raw(s.as_str()))
                    .collect::<Vec<_>>(),
            );
            buffer.set_line(*x, *y, &line, *max);
        }
        Op::Symbol { x, y, text } => {
            if let Some(mut cell) = buffer.cell_mut((*x, *y)) {
                cell.set_symbol(text);
            }
        }
        Op::Reset { x, y } => {
            if let Some(mut cell) = buffer.cell_mut((*x, *y)) {
                cell.reset();
            }
        }
        Op::Style { area } => buffer.set_style(*area, Style::new().bg(Color::Blue)),
        Op::Link { area } => buffer.set_link(*area, Some("https://example.com/")),
        Op::Clear => buffer.reset(),
    }
}

/// Walks each row from the left. Every cell reached is a character whose cells, counted by the
/// continuation cells after it, are the width of its symbol.
fn check(buffer: &Buffer) -> Result<(), String> {
    let area = buffer.area();
    let profile = buffer.width_profile();
    for y in area.top()..area.bottom() {
        let mut x = area.left();
        while x < area.right() {
            let cell = buffer
                .cell((x, y))
                .ok_or("a cell inside the area is missing")?;
            let width = cell.width();
            if width == 0 {
                return Err(format!(
                    "a continuation cell at ({x}, {y}) has no character"
                ));
            }
            let symbol = cell.symbol();
            let expected = text::width_with(symbol, profile);
            if usize::from(width) != expected {
                return Err(format!(
                    "{symbol:?} at ({x}, {y}) takes {width} cells, not its width {expected}"
                ));
            }
            for k in 1..width {
                let covered = buffer.cell((x + k, y)).ok_or("past the edge")?;
                if covered.symbol() != "" {
                    return Err(format!(
                        "({}, {y}) is covered but holds {:?}",
                        x + k,
                        covered.symbol()
                    ));
                }
            }
            x += width;
        }
    }
    Ok(())
}

proptest! {
    #[test]
    fn writes_keep_wide_characters_whole(
        profile in profile(),
        offset in (0u16..3, 0u16..3),
        ops in prop::collection::vec(op(), 1..40),
    ) {
        let area = Rect::new(offset.0, offset.1, WIDTH, HEIGHT);
        let mut buffer = Buffer::with_width_profile(area, profile);
        for op in &ops {
            apply(&mut buffer, op);
            if let Err(e) = check(&buffer) {
                prop_assert!(false, "after {:?}: {}\n{:?}", op, e, buffer);
            }
        }
    }
}
