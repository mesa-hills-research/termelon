//! tuiframe's widgets draw what ratatui's draw. Each case builds the same widget for both, draws
//! it over the same cells, and compares the screens, each cell a terminal would show with its
//! colours and modifiers.
//!
//! The cases leave out the places where tuiframe draws differently on purpose. Its buffer skips
//! zero-width clusters where ratatui appends them to the cell before, and a write over half of a
//! wide character blanks the rest of it. A control character takes no cells in a line's width,
//! where ratatui's `Line::width` counts one for it and then draws nothing.

use ratatui::buffer::Buffer as RBuffer;
use ratatui::layout::{Alignment as RAlignment, Rect as RRect};
use ratatui::style::{Color as RColor, Modifier as RModifier, Style as RStyle};
use ratatui::text::{Line as RLine, Span as RSpan, Text as RText};
use ratatui::widgets::{Clear as RClear, Widget as RWidget};
use unicode_width::UnicodeWidthStr;

use tuiframe::buffer::Buffer;
use tuiframe::layout::{Alignment, Rect};
use tuiframe::style::{Color, Modifier, Style};
use tuiframe::text::{Line, Span, Text};
use tuiframe::widgets::{Clear, Widget};

fn r_color(color: Color) -> RColor {
    match color {
        Color::Reset => RColor::Reset,
        Color::Black => RColor::Black,
        Color::Red => RColor::Red,
        Color::Green => RColor::Green,
        Color::Yellow => RColor::Yellow,
        Color::Blue => RColor::Blue,
        Color::Magenta => RColor::Magenta,
        Color::Cyan => RColor::Cyan,
        Color::Gray => RColor::Gray,
        Color::DarkGray => RColor::DarkGray,
        Color::LightRed => RColor::LightRed,
        Color::LightGreen => RColor::LightGreen,
        Color::LightYellow => RColor::LightYellow,
        Color::LightBlue => RColor::LightBlue,
        Color::LightMagenta => RColor::LightMagenta,
        Color::LightCyan => RColor::LightCyan,
        Color::White => RColor::White,
        Color::Indexed(i) => RColor::Indexed(i),
        Color::Rgb(r, g, b) => RColor::Rgb(r, g, b),
    }
}

fn r_style(style: Style) -> RStyle {
    RStyle {
        fg: style.fg.map(r_color),
        bg: style.bg.map(r_color),
        add_modifier: RModifier::from_bits_truncate(style.add_modifier.bits()),
        sub_modifier: RModifier::from_bits_truncate(style.sub_modifier.bits()),
        ..RStyle::default()
    }
}

fn r_alignment(alignment: Alignment) -> RAlignment {
    match alignment {
        Alignment::Left => RAlignment::Left,
        Alignment::Center => RAlignment::Center,
        Alignment::Right => RAlignment::Right,
    }
}

fn r_rect(rect: Rect) -> RRect {
    RRect::new(rect.x, rect.y, rect.width, rect.height)
}

fn r_span(span: &Span<'_>) -> RSpan<'static> {
    RSpan::styled(span.content.to_string(), r_style(span.style))
}

fn r_line(line: &Line<'_>) -> RLine<'static> {
    let mut r =
        RLine::from(line.spans.iter().map(r_span).collect::<Vec<_>>()).style(r_style(line.style));
    r.alignment = line.alignment.map(r_alignment);
    r
}

fn r_text(text: &Text<'_>) -> RText<'static> {
    let mut r =
        RText::from(text.lines.iter().map(r_line).collect::<Vec<_>>()).style(r_style(text.style));
    r.alignment = text.alignment.map(r_alignment);
    r
}

/// What a terminal shows of a row, cell by cell, with a wide character's covered cells left out.
#[derive(Debug, PartialEq, Eq)]
struct Shown {
    x: u16,
    symbol: String,
    fg: String,
    bg: String,
    modifier: u16,
}

/// Each row's text, then its runs of cells in one style.
fn screen(rows: &[Vec<Shown>]) -> Vec<String> {
    let mut out = Vec::new();
    for (y, row) in rows.iter().enumerate() {
        out.push(format!(
            "{y}: {}",
            row.iter().map(|c| c.symbol.as_str()).collect::<String>()
        ));
        let mut start = 0;
        for (i, cell) in row.iter().enumerate() {
            let next = row.get(i + 1);
            let same = next
                .is_some_and(|n| (&n.fg, &n.bg, n.modifier) == (&cell.fg, &cell.bg, cell.modifier));
            if !same {
                let from = row[start].x;
                out.push(format!(
                    "{y}: {from}..={} fg={} bg={} modifier={}",
                    cell.x, cell.fg, cell.bg, cell.modifier
                ));
                start = i + 1;
            }
        }
    }
    out
}

fn ours(buf: &Buffer) -> Vec<String> {
    let area = buf.area();
    let rows: Vec<Vec<Shown>> = area
        .rows()
        .map(|row| {
            row.positions()
                .filter_map(|p| {
                    let cell = buf.cell(p)?;
                    (cell.width() > 0).then(|| Shown {
                        x: p.x,
                        symbol: cell.symbol().to_owned(),
                        fg: format!("{:?}", cell.fg()),
                        bg: format!("{:?}", cell.bg()),
                        modifier: cell.modifier().bits(),
                    })
                })
                .collect()
        })
        .collect();
    screen(&rows)
}

fn theirs(buf: &RBuffer) -> Vec<String> {
    let area = buf.area;
    let rows: Vec<Vec<Shown>> = (area.top()..area.bottom())
        .map(|y| {
            let mut row = Vec::new();
            let mut x = area.left();
            while x < area.right() {
                let cell = &buf[(x, y)];
                row.push(Shown {
                    x,
                    symbol: cell.symbol().to_owned(),
                    fg: format!("{:?}", cell.fg),
                    bg: format!("{:?}", cell.bg),
                    modifier: cell.modifier.bits(),
                });
                x += u16::try_from(cell.symbol().width().max(1)).unwrap_or(1);
            }
            row
        })
        .collect();
    screen(&rows)
}

const PREFILL: Style = Style::new().fg(Color::Green).bg(Color::Indexed(17));

/// A pair of buffers of `width` by `height`, filled with digits in the same style.
fn buffers(width: u16, height: u16) -> (Buffer, RBuffer) {
    let mut a = Buffer::empty(Rect::new(0, 0, width, height));
    let mut b = RBuffer::empty(RRect::new(0, 0, width, height));
    let digits: String = (0..width)
        .map(|i| char::from_digit(u32::from(i % 10), 10).unwrap_or('0'))
        .collect();
    for y in 0..height {
        a.set_string(0, y, &digits, PREFILL);
        b.set_string(0, y, &digits, r_style(PREFILL));
    }
    (a, b)
}

#[track_caller]
fn same(a: &Buffer, b: &RBuffer, case: &dyn std::fmt::Debug) {
    let (a, b) = (ours(a), theirs(b));
    assert!(a == b, "{case:#?}\nours   {a:#?}\ntheirs {b:#?}");
}

const RED: Style = Style::new().fg(Color::Red);
const ON_BLUE: Style = Style::new().bg(Color::Blue);
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);

fn styles() -> [Style; 8] {
    [
        Style::new(),
        RED,
        ON_BLUE,
        BOLD,
        Style::new()
            .fg(Color::Rgb(1, 2, 3))
            .add_modifier(Modifier::ITALIC),
        Style::new()
            .remove_modifier(Modifier::BOLD)
            .fg(Color::Reset),
        Style::new().fg(Color::Indexed(236)).bg(Color::Black),
        Style::new().add_modifier(Modifier::REVERSED | Modifier::UNDERLINED),
    ]
}

fn sample_lines() -> Vec<Line<'static>> {
    vec![
        Line::from("hello"),
        Line::from(vec![
            Span::styled("ab", RED),
            Span::styled("界c", BOLD),
            Span::raw("de"),
        ])
        .style(ON_BLUE),
        Line::from("a界b界c界"),
        Line::from(vec![Span::raw("e\u{301}x"), Span::styled("q", RED)]),
        Line::from("").style(ON_BLUE),
        Line::from(vec![
            Span::raw(""),
            Span::styled("abc", BOLD),
            Span::raw("  "),
        ]),
        Line::from("界界界界界"),
    ]
}

const ALIGNMENTS: [Option<Alignment>; 4] = [
    None,
    Some(Alignment::Left),
    Some(Alignment::Center),
    Some(Alignment::Right),
];

#[test]
fn strings_and_spans_draw_as_ratatuis() {
    let strings = ["abc", "a界b", "界界", "x\ty", "e\u{301}e\u{301}"];
    for s in strings {
        for width in 0..6 {
            let area = Rect::new(1, 1, width, 1);
            let (mut a, mut b) = buffers(7, 3);
            Widget::render(s, area, &mut a);
            RWidget::render(s, r_rect(area), &mut b);
            same(&a, &b, &(s, area));
            let (mut a, mut b) = buffers(7, 3);
            Widget::render(s.to_owned(), area, &mut a);
            RWidget::render(s.to_owned(), r_rect(area), &mut b);
            same(&a, &b, &(s, area));
            for style in styles() {
                let span = Span::styled(s, style);
                let (mut a, mut b) = buffers(7, 3);
                (&span).render(area, &mut a);
                r_span(&span).render(r_rect(area), &mut b);
                same(&a, &b, &(&span, area));
            }
        }
    }
}

#[test]
fn lines_draw_as_ratatuis() {
    for line in sample_lines() {
        for alignment in ALIGNMENTS {
            let mut line = line.clone();
            line.alignment = alignment;
            for x in [0, 1] {
                for width in 0..12 {
                    let area = Rect::new(x, 1, width, 2);
                    let (mut a, mut b) = buffers(13, 3);
                    (&line).render(area, &mut a);
                    r_line(&line).render(r_rect(area), &mut b);
                    same(&a, &b, &(&line, area));
                }
            }
        }
    }
}

#[test]
fn text_draws_as_ratatuis() {
    let lines = sample_lines();
    for alignment in ALIGNMENTS {
        for style in styles() {
            let mut text = Text::from(lines.clone()).style(style);
            text.alignment = alignment;
            for area in [
                Rect::new(0, 0, 9, 4),
                Rect::new(2, 1, 5, 9),
                Rect::new(1, 2, 12, 2),
            ] {
                let (mut a, mut b) = buffers(10, 6);
                (&text).render(area, &mut a);
                r_text(&text).render(r_rect(area), &mut b);
                same(&a, &b, &(&text, area));
            }
        }
    }
}

#[test]
fn clear_draws_as_ratatuis() {
    for area in [
        Rect::new(0, 0, 0, 0),
        Rect::new(1, 1, 3, 2),
        Rect::new(0, 0, 6, 4),
        Rect::new(4, 2, 9, 9),
    ] {
        let (mut a, mut b) = buffers(6, 4);
        Clear.render(area, &mut a);
        RClear.render(r_rect(area), &mut b);
        same(&a, &b, &area);
    }
}
