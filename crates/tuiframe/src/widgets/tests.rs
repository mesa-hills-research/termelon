//! Goldens for each widget, as the rows a terminal would show and the runs of styled cells.

use super::*;
use crate::style::Color;

/// Each row's text, continuation cells left out.
fn rows(buf: &Buffer) -> Vec<String> {
    buf.area()
        .rows()
        .map(|row| row.positions().map(|p| buf.symbol(p)).collect())
        .collect()
}

/// Each run of cells in one style other than the default, as `row start..end attributes`.
fn styles(buf: &Buffer) -> Vec<String> {
    let mut runs = Vec::new();
    for row in buf.area().rows() {
        let mut run: Option<(u16, u16, String)> = None;
        for p in row.positions() {
            let Some(cell) = buf.cell(p) else { continue };
            if cell.width() == 0 {
                if let Some((_, end, _)) = &mut run {
                    *end = p.x + 1;
                }
                continue;
            }
            let mut attrs = Vec::new();
            if cell.fg() != Color::Reset {
                attrs.push(format!("fg={:?}", cell.fg()));
            }
            if cell.bg() != Color::Reset {
                attrs.push(format!("bg={:?}", cell.bg()));
            }
            if !cell.modifier().is_empty() {
                attrs.push(format!("{:?}", cell.modifier()));
            }
            let attrs = attrs.join(" ");
            match &mut run {
                Some((_, end, a)) if *a == attrs => *end = p.x + 1,
                _ => {
                    if let Some((start, end, a)) = run.take()
                        && !a.is_empty()
                    {
                        runs.push(format!("{} {start}..{end} {a}", row.y));
                    }
                    run = Some((p.x, p.x + 1, attrs));
                }
            }
        }
        if let Some((start, end, a)) = run
            && !a.is_empty()
        {
            runs.push(format!("{} {start}..{end} {a}", row.y));
        }
    }
    runs
}

fn draw(width: u16, height: u16, widget: impl Widget) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    widget.render(buf.area(), &mut buf);
    buf
}

/// A buffer of `width` by `height` filled with `x`.
fn filled(width: u16, height: u16) -> Buffer {
    let mut buf = Buffer::empty(Rect::new(0, 0, width, height));
    for y in 0..height {
        buf.set_string(0, y, "x".repeat(usize::from(width)), Style::new());
    }
    buf
}

const RED: Style = Style::new().fg(Color::Red);
const ON_BLUE: Style = Style::new().bg(Color::Blue);

#[test]
fn strings_write_the_first_row_cut_at_the_width() {
    let mut buf = filled(5, 2);
    "ab界cd".render(Rect::new(1, 0, 3, 2), &mut buf);
    assert_eq!(rows(&buf), ["xabxx", "xxxxx"]);
    let mut buf = filled(4, 1);
    String::from("界界").render(Rect::new(0, 0, 3, 1), &mut buf);
    assert_eq!(rows(&buf), ["界xx"]);
    let mut buf = filled(4, 1);
    (&String::from("yz")).render(Rect::new(3, 0, 5, 1), &mut buf);
    assert_eq!(rows(&buf), ["xxxy"]);
    let mut buf = filled(3, 1);
    "ab".render(Rect::new(0, 0, 3, 0), &mut buf);
    assert_eq!(rows(&buf), ["xxx"]);
}

#[test]
fn spans_write_in_their_style() {
    let buf = draw(5, 1, Span::styled("a\tb", RED));
    assert_eq!(rows(&buf), ["ab   "]);
    assert_eq!(styles(&buf), ["0 0..2 fg=Red"]);
    let span = Span::styled("abc", ON_BLUE);
    let mut buf = filled(4, 1);
    (&span).render(Rect::new(2, 0, 9, 1), &mut buf);
    assert_eq!(rows(&buf), ["xxab"]);
    assert_eq!(styles(&buf), ["0 2..4 bg=Blue"]);
}

#[test]
fn lines_take_their_style_across_the_row_and_sit_as_aligned() {
    let line = Line::from(vec![Span::styled("ab", RED), Span::raw("c")]).style(ON_BLUE);
    let buf = draw(7, 1, &line);
    assert_eq!(rows(&buf), ["abc    "]);
    assert_eq!(styles(&buf), ["0 0..2 fg=Red bg=Blue", "0 2..7 bg=Blue"]);
    assert_eq!(rows(&draw(7, 1, line.clone().centered())), ["  abc  "]);
    assert_eq!(rows(&draw(8, 1, line.clone().centered())), ["  abc   "]);
    assert_eq!(rows(&draw(7, 1, line.right_aligned())), ["    abc"]);
}

#[test]
fn an_empty_line_leaves_its_row_alone() {
    let buf = draw(3, 1, Line::from("").style(ON_BLUE));
    assert!(styles(&buf).is_empty());
}

#[test]
fn long_lines_lose_the_end_their_alignment_puts_out_of_sight() {
    let line = Line::from(vec![Span::raw("abc"), Span::styled("defg", RED)]);
    assert_eq!(rows(&draw(4, 1, &line)), ["abcd"]);
    assert_eq!(rows(&draw(4, 1, line.clone().centered())), ["bcde"]);
    let buf = draw(4, 1, line.right_aligned());
    assert_eq!(rows(&buf), ["defg"]);
    assert_eq!(styles(&buf), ["0 0..4 fg=Red"]);
}

#[test]
fn a_wide_character_cut_on_the_left_leaves_its_cells_blank() {
    let mut buf = filled(3, 1);
    Line::from("a界bc")
        .right_aligned()
        .render(buf.area(), &mut buf);
    assert_eq!(rows(&buf), ["xbc"]);
    let mut buf = filled(4, 1);
    Line::from("a界bc")
        .right_aligned()
        .render(buf.area(), &mut buf);
    assert_eq!(rows(&buf), ["界bc"]);
    let mut buf = filled(3, 1);
    Line::from("界界界").centered().render(buf.area(), &mut buf);
    assert_eq!(rows(&buf), ["x界"]);
}

#[test]
fn text_fills_its_area_with_its_style_and_writes_a_line_per_row() {
    let text = Text::from(vec![
        Line::from("one"),
        Line::from("two").right_aligned(),
        Line::from("three"),
    ])
    .style(ON_BLUE)
    .centered();
    let buf = draw(5, 2, &text);
    assert_eq!(rows(&buf), [" one ", "  two"]);
    assert_eq!(styles(&buf), ["0 0..5 bg=Blue", "1 0..5 bg=Blue"]);
    let mut buf = filled(4, 3);
    text.render(Rect::new(1, 1, 9, 9), &mut buf);
    assert_eq!(rows(&buf), ["xxxx", "xone", "xtwo"]);
}

#[test]
fn an_option_draws_what_it_holds() {
    assert_eq!(rows(&draw(2, 1, Some("ab"))), ["ab"]);
    assert_eq!(rows(&draw(2, 1, None::<&str>)), ["  "]);
}

#[test]
fn clear_blanks_its_area() {
    let mut buf = filled(4, 3);
    buf.set_style(buf.area(), RED.bg(Color::Blue));
    Clear.render(Rect::new(1, 1, 2, 5), &mut buf);
    assert_eq!(rows(&buf), ["xxxx", "x  x", "x  x"]);
    assert_eq!(
        styles(&buf),
        [
            "0 0..4 fg=Red bg=Blue",
            "1 0..1 fg=Red bg=Blue",
            "1 3..4 fg=Red bg=Blue",
            "2 0..1 fg=Red bg=Blue",
            "2 3..4 fg=Red bg=Blue",
        ]
    );
}

#[test]
fn clear_blanks_a_wide_character_across_its_edge_whole() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 6, 1));
    buf.set_string(0, 0, "a界b界", Style::new());
    (&Clear).render(Rect::new(2, 0, 3, 1), &mut buf);
    assert_eq!(rows(&buf), ["a     "]);
}
