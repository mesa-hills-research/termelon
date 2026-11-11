//! Goldens for each widget, as the rows a terminal would show and the runs of styled cells.

use super::*;
use crate::caps::WidthProfile;
use crate::style::{Color, Modifier};

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
const BOLD: Style = Style::new().add_modifier(Modifier::BOLD);

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
    assert_eq!(styles(&buf), Vec::<String>::new());
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

#[test]
fn each_border_set_draws_its_box() {
    let sets = [
        (BorderSet::PLAIN, ["┌──┐", "│  │", "└──┘"]),
        (BorderSet::ROUNDED, ["╭──╮", "│  │", "╰──╯"]),
        (BorderSet::THICK, ["┏━━┓", "┃  ┃", "┗━━┛"]),
        (BorderSet::DOUBLE, ["╔══╗", "║  ║", "╚══╝"]),
        (BorderSet::ASCII, ["+--+", "|  |", "+--+"]),
    ];
    for (set, expected) in sets {
        let buf = draw(4, 3, Block::bordered().border_set(set));
        assert_eq!(rows(&buf), expected, "{set:?}");
    }
    assert_eq!(BorderSet::default(), BorderSet::PLAIN);
}

#[test]
fn a_border_set_of_its_own_draws_each_side_with_its_symbol() {
    let set = BorderSet {
        top_left: "1",
        top_right: "2",
        bottom_left: "3",
        bottom_right: "4",
        vertical_left: "l",
        vertical_right: "r",
        horizontal_top: "t",
        horizontal_bottom: "b",
    };
    let buf = draw(4, 3, Block::bordered().border_set(set));
    assert_eq!(rows(&buf), ["1tt2", "l  r", "3bb4"]);
}

#[test]
fn borders_draw_only_the_sides_they_name() {
    let cases = [
        (Borders::NONE, ["    ", "    ", "    "]),
        (Borders::TOP, ["────", "    ", "    "]),
        (Borders::BOTTOM, ["    ", "    ", "────"]),
        (Borders::LEFT, ["│   ", "│   ", "│   "]),
        (Borders::RIGHT, ["   │", "   │", "   │"]),
        (Borders::TOP | Borders::LEFT, ["┌───", "│   ", "│   "]),
        (Borders::ALL - Borders::TOP, ["│  │", "│  │", "└──┘"]),
        (Borders::LEFT | Borders::RIGHT, ["│  │", "│  │", "│  │"]),
    ];
    for (borders, expected) in cases {
        let buf = draw(4, 3, Block::new().borders(borders));
        assert_eq!(rows(&buf), expected, "{borders:?}");
    }
}

#[test]
fn small_areas_draw_corners_over_sides() {
    assert_eq!(rows(&draw(1, 1, Block::bordered())), ["┌"]);
    assert_eq!(rows(&draw(2, 1, Block::bordered())), ["┌┐"]);
    assert_eq!(rows(&draw(1, 2, Block::bordered())), ["┌", "└"]);
    assert_eq!(rows(&draw(2, 2, Block::bordered())), ["┌┐", "└┘"]);
    let mut buf = Buffer::empty(Rect::new(0, 0, 2, 2));
    Block::bordered().render(Rect::new(1, 1, 0, 3), &mut buf);
    Block::bordered().render(Rect::new(5, 5, 3, 3), &mut buf);
    assert_eq!(rows(&buf), ["  ", "  "]);
}

#[test]
fn the_block_style_goes_under_its_borders() {
    let block = Block::bordered()
        .style(Style::new().fg(Color::White).bg(Color::Black))
        .border_style(RED);
    let buf = draw(4, 3, block);
    assert_eq!(
        styles(&buf),
        [
            "0 0..4 fg=Red bg=Black",
            "1 0..1 fg=Red bg=Black",
            "1 1..3 fg=White bg=Black",
            "1 3..4 fg=Red bg=Black",
            "2 0..4 fg=Red bg=Black",
        ]
    );
}

#[test]
fn a_block_over_text_keeps_the_text_inside() {
    let mut buf = filled(5, 3);
    Block::bordered().render(buf.area(), &mut buf);
    assert_eq!(rows(&buf), ["┌───┐", "│xxx│", "└───┘"]);
}

#[test]
fn a_border_over_a_wide_character_blanks_the_rest_of_it() {
    let mut buf = Buffer::empty(Rect::new(0, 0, 5, 2));
    buf.set_string(0, 0, "a界界", Style::new());
    Block::new()
        .borders(Borders::LEFT)
        .render(Rect::new(2, 0, 3, 2), &mut buf);
    assert_eq!(rows(&buf), ["a │界", "  │  "]);
}

/// An overlay's box, a bordered block with a bold title over a cleared area.
#[test]
fn a_titled_box_draws_over_a_cleared_area() {
    let mut buf = filled(14, 4);
    let area = Rect::new(1, 0, 12, 4);
    Clear.render(area, &mut buf);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(BorderSet::ROUNDED)
        .border_style(Style::new().fg(Color::DarkGray))
        .style(Style::new().bg(Color::Indexed(236)).fg(Color::Gray))
        .title(Line::from(Span::styled(" Keys ", BOLD.fg(Color::White))));
    let inner = block.inner(area);
    block.render(area, &mut buf);
    assert_eq!(inner, Rect::new(2, 1, 10, 2));
    assert_eq!(
        rows(&buf),
        [
            "x╭ Keys ────╮x",
            "x│          │x",
            "x│          │x",
            "x╰──────────╯x",
        ]
    );
    assert_eq!(
        styles(&buf),
        [
            "0 1..2 fg=DarkGray bg=Indexed(236)",
            "0 2..8 fg=White bg=Indexed(236) Modifier(BOLD)",
            "0 8..13 fg=DarkGray bg=Indexed(236)",
            "1 1..2 fg=DarkGray bg=Indexed(236)",
            "1 2..12 fg=Gray bg=Indexed(236)",
            "1 12..13 fg=DarkGray bg=Indexed(236)",
            "2 1..2 fg=DarkGray bg=Indexed(236)",
            "2 2..12 fg=Gray bg=Indexed(236)",
            "2 12..13 fg=DarkGray bg=Indexed(236)",
            "3 1..13 fg=DarkGray bg=Indexed(236)",
        ]
    );
}

#[test]
fn titles_sit_left_in_the_middle_or_right_on_either_border() {
    let block = Block::bordered()
        .title("L1")
        .title("L2")
        .title(Line::from("C").centered())
        .title(Line::from("R1").right_aligned())
        .title(Line::from("R2").right_aligned())
        .title_bottom("b")
        .title_bottom(Line::from("e").right_aligned());
    let buf = draw(16, 3, block);
    assert_eq!(
        rows(&buf),
        ["┌L1─L2─C──R1─R2┐", "│              │", "└b────────────e┘"]
    );
}

#[test]
fn the_title_position_and_alignment_set_the_default_place() {
    let block = Block::bordered()
        .title_position(TitlePosition::Bottom)
        .title_alignment(Alignment::Center)
        .title("mid")
        .title_top(Line::from("top").left_aligned());
    let buf = draw(9, 3, block);
    assert_eq!(rows(&buf), ["┌top────┐", "│       │", "└──mid──┘"]);
}

#[test]
fn titles_without_borders_take_their_row_and_the_inner_area_skips_it() {
    let block = Block::new().title("t").title_bottom("b");
    assert_eq!(block.inner(Rect::new(0, 0, 3, 4)), Rect::new(0, 1, 3, 2));
    assert_eq!(rows(&draw(3, 4, block)), ["t  ", "   ", "   ", "b  "]);
}

#[test]
fn titles_too_long_for_the_border_are_cut() {
    let left = Block::bordered().title("abcdef").title("gh");
    assert_eq!(rows(&draw(6, 2, left))[0], "┌abcd┐");
    let right = Block::bordered()
        .title(Line::from("abcdef").right_aligned())
        .title(Line::from("gh").right_aligned());
    assert_eq!(rows(&draw(6, 2, right))[0], "┌f─gh┐");
    let centre = Block::bordered()
        .title(Line::from("abc").centered())
        .title(Line::from("def").centered());
    assert_eq!(rows(&draw(6, 2, centre))[0], "┌bc─d┐");
}

#[test]
fn the_title_style_goes_under_each_title() {
    let block = Block::bordered()
        .title_style(ON_BLUE)
        .title(Line::from(Span::styled("ab", RED)));
    let buf = draw(5, 2, block);
    assert_eq!(styles(&buf), ["0 1..3 fg=Red bg=Blue"]);
}

#[test]
fn padding_shrinks_the_inner_area() {
    let area = Rect::new(2, 1, 10, 6);
    let block = Block::bordered().padding(Padding::new(1, 2, 0, 1));
    assert_eq!(block.inner(area), Rect::new(4, 2, 5, 3));
    assert_eq!(
        Block::new().padding(Padding::uniform(9)).inner(area),
        Rect::new(11, 10, 0, 0)
    );
    assert_eq!(Padding::symmetric(1, 2), Padding::new(1, 1, 2, 2));
    assert_eq!(Padding::vertical(3), Padding::new(0, 0, 3, 3));
    assert_eq!(
        Block::bordered().inner(Rect::new(0, 0, 1, 1)),
        Rect::new(1, 1, 0, 0)
    );
    assert_eq!(Block::default(), Block::new());
}

#[test]
fn borders_combine_as_flags() {
    assert_eq!(
        Borders::ALL,
        Borders::TOP | Borders::RIGHT | Borders::BOTTOM | Borders::LEFT
    );
    assert_eq!(format!("{:?}", Borders::NONE), "Borders(NONE)");
    assert_eq!(
        format!("{:?}", Borders::TOP | Borders::LEFT),
        "Borders(TOP | LEFT)"
    );
}

#[test]
fn paragraphs_cut_long_lines_without_wrapping() {
    let text = Text::from("first line\nsecond line\nthird");
    let buf = draw(6, 2, Paragraph::new(text.clone()));
    assert_eq!(rows(&buf), ["first ", "second"]);
    let buf = draw(6, 2, Paragraph::new(text.clone()).scroll((1, 3)));
    assert_eq!(rows(&buf), ["ond li", "rd    "]);
    let buf = draw(6, 2, Paragraph::new(text).scroll((1, 3)).right_aligned());
    assert_eq!(rows(&buf), ["second", " third"]);
}

#[test]
fn scrolling_across_a_wide_character_shows_it_whole() {
    let buf = draw(4, 1, Paragraph::new("a界bc").scroll((0, 2)));
    assert_eq!(rows(&buf), ["界bc"]);
    let buf = draw(4, 1, Paragraph::new("a界bc").scroll((0, 3)));
    assert_eq!(rows(&buf), ["bc  "]);
}

#[test]
fn paragraphs_wrap_by_words() {
    let text = "Lorem ipsum dolor sit amet, consectetur";
    let buf = draw(12, 4, Paragraph::new(text).wrap(Wrap { trim: true }));
    assert_eq!(
        rows(&buf),
        [
            "Lorem ipsum ",
            "dolor sit   ",
            "amet,       ",
            "consectetur "
        ]
    );
    let buf = draw(
        12,
        2,
        Paragraph::new(text)
            .wrap(Wrap { trim: true })
            .scroll((2, 5)),
    );
    assert_eq!(rows(&buf), ["amet,       ", "consectetur "]);
}

#[test]
fn wrapped_rows_sit_as_aligned() {
    let text = Text::from(vec![
        Line::from("aa bb cc"),
        Line::from("dd ee").right_aligned(),
    ]);
    let buf = draw(
        6,
        3,
        Paragraph::new(text).wrap(Wrap { trim: true }).centered(),
    );
    assert_eq!(rows(&buf), [" aa bb", "  cc  ", " dd ee"]);
}

#[test]
fn wrapped_spans_keep_their_styles_across_breaks() {
    let line = Line::from(vec![
        Span::raw("one "),
        Span::styled("two three", RED),
        Span::raw(" four"),
    ]);
    let buf = draw(8, 3, Paragraph::new(line).wrap(Wrap { trim: true }));
    assert_eq!(rows(&buf), ["one two ", "three   ", "four    "]);
    assert_eq!(styles(&buf), ["0 4..7 fg=Red", "1 0..5 fg=Red"]);
}

#[test]
fn paragraph_styles_go_under_text_line_and_span_styles() {
    let line = Line::from(vec![Span::raw("ab"), Span::styled("c", RED)]).style(BOLD);
    let text = Text::from(line).style(Style::new().fg(Color::Green));
    let paragraph = Paragraph::new(text).style(ON_BLUE);
    let buf = draw(4, 2, paragraph);
    assert_eq!(
        styles(&buf),
        [
            "0 0..2 fg=Green bg=Blue Modifier(BOLD)",
            "0 2..3 fg=Red bg=Blue Modifier(BOLD)",
            "0 3..4 bg=Blue",
            "1 0..4 bg=Blue",
        ]
    );
}

#[test]
fn a_paragraph_in_a_block_draws_inside_it() {
    let paragraph = Paragraph::new("Hello, worlds!")
        .block(Block::bordered().title("Title"))
        .wrap(Wrap { trim: false });
    let buf = draw(13, 4, paragraph);
    assert_eq!(
        rows(&buf),
        [
            "┌Title──────┐",
            "│Hello,     │",
            "│worlds!    │",
            "└───────────┘"
        ]
    );
}

#[test]
fn a_wide_character_at_the_edge_goes_to_the_next_row() {
    let paragraph = Paragraph::new("abcd界 x").wrap(Wrap { trim: true });
    let mut buf = Buffer::empty(Rect::new(0, 0, 6, 3));
    paragraph.render(Rect::new(0, 0, 5, 3), &mut buf);
    assert_eq!(rows(&buf), ["abcd  ", "界 x  ", "      "]);
}

#[test]
fn paragraphs_measure_with_the_buffer_profile() {
    let joining = WidthProfile {
        joins_emoji: true,
        ..WidthProfile::default()
    };
    let family = "👨\u{200d}👩\u{200d}👧";
    let mut buf = Buffer::with_width_profile(Rect::new(0, 0, 4, 2), joining);
    Paragraph::new(format!("{family} {family}"))
        .wrap(Wrap { trim: true })
        .render(buf.area(), &mut buf);
    assert_eq!(rows(&buf), [format!("{family}  "), format!("{family}  ")]);
    let mut buf = Buffer::empty(Rect::new(0, 0, 4, 2));
    Paragraph::new(format!("{family} {family}"))
        .wrap(Wrap { trim: true })
        .render(buf.area(), &mut buf);
    assert_eq!(rows(&buf), ["    ", "    "]);
}

#[test]
fn empty_paragraphs_and_areas_draw_nothing_but_the_style() {
    let buf = draw(
        3,
        2,
        Paragraph::new("").style(ON_BLUE).wrap(Wrap { trim: true }),
    );
    assert_eq!(rows(&buf), ["   ", "   "]);
    assert_eq!(styles(&buf), ["0 0..3 bg=Blue", "1 0..3 bg=Blue"]);
    let mut buf = filled(3, 1);
    Paragraph::new("abc").render(Rect::new(0, 0, 0, 1), &mut buf);
    Paragraph::new("abc").render(Rect::new(7, 0, 2, 1), &mut buf);
    assert_eq!(rows(&buf), ["xxx"]);
}
