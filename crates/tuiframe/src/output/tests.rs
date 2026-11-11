//! The diff and the encoder against `tuiframe-vt`, and the bytes of small frames.

use proptest::prelude::*;
use tuiframe_vt::{Emulator, Event, Profile, Underline, WidthRules};

use super::encode::Encoder;
use super::{CompletedFrame, Cursor, CursorShape, FrameStats};
use crate::buffer::Buffer;
use crate::caps::{Capabilities, ColorDepth, Support, WidthProfile};
use crate::layout::{Position, Rect};
use crate::output::color::lower;
use crate::style::{Color, Modifier, Style, UnderlineStyle};
use crate::text::{Line, Span};

/// A terminal that supports everything the encoder can use.
fn full_caps() -> Capabilities {
    Capabilities {
        colors: ColorDepth::TrueColor,
        synchronized_output: Support::Yes,
        hyperlinks: Support::Yes,
        underline_styles: Support::Yes,
        underline_color: Support::Yes,
        cursor_shape: Support::Yes,
        ..Capabilities::default()
    }
}

/// The bytes with ESC shown as `␛` and CR as `␍`, so goldens read as text.
fn show(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes)
        .replace('\x1b', "␛")
        .replace('\r', "␍")
}

/// Encodes `next` over `prev` and returns the packet as text.
fn packet(
    encoder: &mut Encoder,
    prev: &Buffer,
    next: &Buffer,
    cursor: Option<Cursor>,
    caps: &Capabilities,
) -> String {
    let mut out = Vec::new();
    let stats = encoder.encode(prev, next, cursor.as_ref(), caps, &mut out);
    assert_eq!(stats.bytes, out.len());
    show(&out)
}

fn text(area: Rect, rows: &[(u16, u16, &str, Style)]) -> Buffer {
    let mut buffer = Buffer::empty(area);
    for &(x, y, s, style) in rows {
        buffer.set_string(x, y, s, style);
    }
    buffer
}

/// An encoder that has drawn `first` from a blank screen, so its pen and cursor are known.
fn primed(first: &Buffer, caps: &Capabilities) -> Encoder {
    let mut encoder = Encoder::new();
    let blank = Buffer::empty(first.area());
    let mut out = Vec::new();
    encoder.encode(&blank, first, None, caps, &mut out);
    encoder
}

const AREA: Rect = Rect::new(0, 0, 10, 3);

#[test]
fn byte_goldens() {
    let caps = full_caps();
    let blank = Buffer::empty(AREA);
    let red = Style::new().fg(Color::Red);
    let cases: Vec<(&str, Buffer, Buffer, Option<Cursor>, &str)> = vec![
        (
            "the first frame",
            blank.clone(),
            text(AREA, &[(0, 0, "hi", Style::new())]),
            None,
            "␛[?2026h␛[?25l␛[H␛[0mhi␛[?2026l",
        ),
        (
            "a style and the reset at the end",
            blank.clone(),
            text(AREA, &[(2, 1, "x", red.add_modifier(Modifier::BOLD))]),
            None,
            "␛[?2026h␛[?25l␛[2;3H␛[0;1;31mx␛[0m␛[?2026l",
        ),
        (
            "two runs on a row, with a move between",
            blank.clone(),
            text(
                AREA,
                &[(0, 0, "a", Style::new()), (7, 0, "b", Style::new())],
            ),
            None,
            "␛[?2026h␛[?25l␛[H␛[0ma␛[8Gb␛[?2026l",
        ),
        (
            "a cheap gap is written",
            text(AREA, &[(0, 0, "abcd", Style::new())]),
            text(AREA, &[(0, 0, "AbCd", Style::new())]),
            None,
            "␛[?2026h␍AbC␛[?2026l",
        ),
        (
            "the next row from its start",
            blank.clone(),
            text(
                AREA,
                &[(0, 0, "a", Style::new()), (0, 1, "b", Style::new())],
            ),
            None,
            "␛[?2026h␛[?25l␛[H␛[0ma␛[2Hb␛[?2026l",
        ),
        (
            "the cursor after the cells",
            blank.clone(),
            text(AREA, &[(0, 2, "> ", Style::new())]),
            Some(Cursor::at(Position::new(2, 2))),
            "␛[?2026h␛[?25l␛[3H␛[0m>␛[3;3H␛[?25h␛[?2026l",
        ),
        (
            "a blank tail erased with CSI K",
            text(AREA, &[(0, 0, "abcdefghij", Style::new())]),
            text(AREA, &[(0, 0, "ab", Style::new())]),
            None,
            "␛[?2026h␛[3G␛[K␛[?2026l",
        ),
        (
            "a coloured tail written as spaces",
            text(AREA, &[(0, 0, "abcdefghij", Style::new())]),
            {
                let mut b = text(AREA, &[(0, 0, "ab", Style::new())]);
                b.set_style(Rect::new(2, 0, 8, 1), Style::new().bg(Color::Blue));
                b
            },
            None,
            "␛[?2026h␛[3G␛[44m        ␛[0m␛[?2026l",
        ),
        (
            "a wide character and its continuation cell",
            blank.clone(),
            text(AREA, &[(0, 0, "a界b", Style::new())]),
            None,
            "␛[?2026h␛[?25l␛[H␛[0ma界b␛[?2026l",
        ),
        (
            "an uncertain width is written over spaces and followed by an absolute move",
            blank.clone(),
            text(AREA, &[(0, 0, "a❤\u{fe0f}b", Style::new())]),
            None,
            "␛[?2026h␛[?25l␛[H␛[0ma ␛[2G❤\u{fe0f}␛[3Gb␛[?2026l",
        ),
        (
            "a link closes at the end of its run",
            blank.clone(),
            {
                let mut b = text(AREA, &[(0, 0, "go here", Style::new())]);
                b.set_link(Rect::new(3, 0, 4, 1), Some("https://example.com/"));
                b
            },
            None,
            "␛[?2026h␛[?25l␛[H␛[0mgo ␛]8;id=e0fbd8b4;https://example.com/␛\\here␛]8;;␛\\␛[?2026l",
        ),
        (
            "two regional indicators in two cells don't join",
            blank.clone(),
            {
                let mut b = Buffer::empty(AREA);
                b.cell_mut((0, 0)).unwrap().set_symbol("🇳");
                b.cell_mut((1, 0)).unwrap().set_symbol("🇿");
                b
            },
            None,
            "␛[?2026h␛[?25l␛[H␛[0m🇳␛[2G🇿␛[?2026l",
        ),
    ];
    let mut failures = Vec::new();
    for (name, prev, next, cursor, expected) in cases {
        let mut encoder = primed(&prev, &caps);
        if prev == blank {
            encoder = Encoder::new();
        }
        let got = packet(&mut encoder, &prev, &next, cursor, &caps);
        if got != expected {
            failures.push(format!("{name}\n  got      {got}\n  expected {expected}"));
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn a_frame_that_changes_nothing_writes_nothing() {
    let caps = full_caps();
    let frame = text(AREA, &[(0, 0, "same", Style::new())]);
    let mut encoder = primed(&frame, &caps);
    assert_eq!(packet(&mut encoder, &frame, &frame, None, &caps), "");
    let cursor = Some(Cursor::at(Position::new(1, 1)));
    assert_ne!(packet(&mut encoder, &frame, &frame, cursor, &caps), "");
    assert_eq!(packet(&mut encoder, &frame, &frame, cursor, &caps), "");
}

#[test]
fn a_frame_that_changes_nothing_but_the_cursor_writes_only_its_move() {
    let caps = full_caps();
    let frame = text(AREA, &[(0, 0, "same", Style::new())]);
    let mut encoder = primed(&frame, &caps);
    let cursor = Some(Cursor::at(Position::new(5, 2)));
    assert_eq!(
        packet(&mut encoder, &frame, &frame, cursor, &caps),
        "␛[3;6H␛[?25h"
    );
    let moved = Some(Cursor::at(Position::new(1, 0)));
    assert_eq!(
        packet(&mut encoder, &frame, &frame, moved, &caps),
        "␛[1;2H␛[?25h"
    );
}

#[test]
fn a_frame_without_a_cursor_hides_it() {
    let caps = full_caps();
    let frame = text(AREA, &[(0, 0, "same", Style::new())]);
    let mut encoder = primed(&frame, &caps);
    let cursor = Some(Cursor::at(Position::new(0, 1)));
    packet(&mut encoder, &frame, &frame, cursor, &caps);
    assert_eq!(packet(&mut encoder, &frame, &frame, None, &caps), "␛[?25l");
    assert_eq!(packet(&mut encoder, &frame, &frame, None, &caps), "");
    let other = text(AREA, &[(0, 0, "diff", Style::new())]);
    let shown = packet(&mut encoder, &frame, &other, cursor, &caps);
    assert!(shown.ends_with("␛[2H␛[?25h␛[?2026l"), "{shown}");
    let hidden = packet(&mut encoder, &other, &frame, None, &caps);
    assert!(hidden.starts_with("␛[?2026h␛[?25l"), "{hidden}");
    assert!(!hidden.contains("?25h"), "{hidden}");
}

#[test]
fn no_shape_is_sent_until_a_frame_asks_for_one() {
    let caps = Capabilities {
        cursor_shape: Support::Unknown,
        ..full_caps()
    };
    let frame = Buffer::empty(AREA);
    let mut encoder = primed(&frame, &caps);
    let plain = Some(Cursor::at(Position::new(0, 0)));
    encoder.redraw();
    let got = packet(&mut encoder, &frame, &frame, plain, &caps);
    assert!(!got.contains(" q"), "{got}");
    let underline = Some(Cursor {
        shape: CursorShape::Underline,
        ..Cursor::at(Position::new(0, 0))
    });
    assert_eq!(
        packet(&mut encoder, &frame, &frame, underline, &caps),
        "␛[4 q"
    );
    assert_eq!(packet(&mut encoder, &frame, &frame, plain, &caps), "␛[0 q");
}

#[test]
fn the_shape_is_sent_when_it_changes() {
    let caps = full_caps();
    let frame = Buffer::empty(AREA);
    let mut encoder = primed(&frame, &caps);
    let bar = Cursor {
        position: Position::new(0, 0),
        shape: CursorShape::Bar,
        blinking: true,
    };
    assert_eq!(
        packet(&mut encoder, &frame, &frame, Some(bar), &caps),
        "␛[H␛[5 q␛[?25h"
    );
    assert_eq!(packet(&mut encoder, &frame, &frame, Some(bar), &caps), "");
    let steady = Cursor {
        blinking: false,
        ..bar
    };
    assert_eq!(
        packet(&mut encoder, &frame, &frame, Some(steady), &caps),
        "␛[6 q"
    );
    let no_shapes = Capabilities {
        cursor_shape: Support::No,
        ..full_caps()
    };
    let block = Cursor {
        shape: CursorShape::Block,
        ..bar
    };
    assert_eq!(
        packet(&mut encoder, &frame, &frame, Some(block), &no_shapes),
        ""
    );
}

/// The SGR that takes the pen from `from` to `to`, as the encoder writes it between two cells.
fn transition(from: Style, to: Style, caps: &Capabilities) -> String {
    let prev = Buffer::empty(Rect::new(0, 0, 2, 1));
    let mut next = prev.clone();
    next.set_string(0, 0, "a", from);
    next.set_string(1, 0, "B", to);
    let got = packet(&mut Encoder::new(), &prev, &next, None, caps);
    let middle = got.split_once('a').map_or("", |(_, rest)| rest);
    middle.split_once('B').map_or("", |(sgr, _)| sgr).to_owned()
}

#[test]
fn styles_change_by_the_fewest_parameters() {
    let caps = full_caps();
    let grey = Style::new().fg(Color::Rgb(90, 90, 90));
    let bold_dim = grey.add_modifier(Modifier::BOLD | Modifier::DIM);
    let dim = grey.add_modifier(Modifier::DIM);
    assert_eq!(
        transition(bold_dim, dim, &caps),
        "␛[22;2m",
        "22 clears both, so dim comes back"
    );
    let bold = grey.add_modifier(Modifier::BOLD);
    assert_eq!(transition(bold, grey, &caps), "␛[22m");
    let busy = Style::new()
        .fg(Color::Rgb(1, 2, 3))
        .add_modifier(Modifier::BOLD | Modifier::ITALIC | Modifier::UNDERLINED);
    let plain_blue = Style::new().bg(Color::Blue);
    assert_eq!(
        transition(busy, plain_blue, &caps),
        "␛[0;44m",
        "starting over is shorter"
    );
    assert_eq!(
        transition(
            Style::new(),
            Style::new().add_modifier(Modifier::ITALIC),
            &caps
        ),
        "␛[3m"
    );
    let blink = Style::new().add_modifier(Modifier::SLOW_BLINK | Modifier::RAPID_BLINK);
    let slow = Style::new().add_modifier(Modifier::SLOW_BLINK);
    assert_eq!(
        transition(blink.patch(grey), slow.patch(grey), &caps),
        "␛[25;5m"
    );
    let reversed = Style::new().add_modifier(Modifier::REVERSED);
    assert_eq!(transition(reversed.patch(grey), grey, &caps), "␛[27m");
    assert_eq!(
        transition(reversed, Style::new(), &caps),
        "␛[0m",
        "SGR 0 is the shortest"
    );
    let bold = Style::new().add_modifier(Modifier::BOLD);
    assert_eq!(transition(bold.fg(Color::Red), bold, &caps), "␛[39m");
}

#[test]
fn named_colours_use_their_own_codes() {
    let caps = full_caps();
    let fg = |c| transition(Style::new(), Style::new().fg(c), &caps);
    let bg = |c| transition(Style::new(), Style::new().bg(c), &caps);
    assert_eq!(fg(Color::Red), "␛[31m");
    assert_eq!(fg(Color::Gray), "␛[37m");
    assert_eq!(fg(Color::DarkGray), "␛[90m");
    assert_eq!(fg(Color::White), "␛[97m");
    assert_eq!(bg(Color::Black), "␛[40m");
    assert_eq!(bg(Color::LightCyan), "␛[106m");
    assert_eq!(fg(Color::Indexed(1)), "␛[38;5;1m");
    assert_eq!(fg(Color::Rgb(10, 20, 30)), "␛[38;2;10;20;30m");
}

#[test]
fn colours_are_lowered_to_the_terminals_depth() {
    let at = |depth| Capabilities {
        colors: depth,
        ..full_caps()
    };
    let rgb = Style::new().fg(Color::Rgb(250, 10, 10));
    assert_eq!(
        transition(Style::new(), rgb, &at(ColorDepth::Ansi256)),
        "␛[38;5;196m"
    );
    assert_eq!(
        transition(Style::new(), rgb, &at(ColorDepth::Ansi16)),
        "␛[91m"
    );
    let indexed = Style::new().bg(Color::Indexed(4));
    assert_eq!(
        transition(Style::new(), indexed, &at(ColorDepth::Ansi16)),
        "␛[44m"
    );
    assert_eq!(transition(Style::new(), rgb, &at(ColorDepth::None)), "");
}

#[test]
fn no_colour_never_writes_an_empty_sgr() {
    let caps = Capabilities {
        colors: ColorDepth::None,
        ..full_caps()
    };
    let mut buffer = Buffer::empty(AREA);
    buffer.set_string(0, 0, "a", Style::new().fg(Color::Red).bg(Color::Blue));
    buffer.set_string(
        1,
        0,
        "b",
        Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
    );
    buffer.set_string(
        2,
        0,
        "c",
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    );
    buffer.set_string(3, 0, "d", Style::new().add_modifier(Modifier::REVERSED));
    let got = packet(
        &mut Encoder::new(),
        &Buffer::empty(AREA),
        &buffer,
        None,
        &caps,
    );
    assert_eq!(got, "␛[?2026h␛[?25l␛[H␛[0ma␛[1mbc␛[0;7md␛[0m␛[?2026l");
    assert!(!got.contains("␛[m") && !got.contains("␛[;"));
}

#[test]
fn underline_shapes_and_colours_follow_the_capabilities() {
    let curly = Style::new()
        .add_modifier(Modifier::UNDERLINED)
        .underline_style(UnderlineStyle::Curly)
        .underline_color(Color::Rgb(1, 2, 3));
    assert_eq!(
        transition(Style::new(), curly, &full_caps()),
        "␛[4:3;58;2;1;2;3m"
    );
    let plain = Capabilities {
        underline_styles: Support::Unknown,
        underline_color: Support::No,
        ..full_caps()
    };
    assert_eq!(transition(Style::new(), curly, &plain), "␛[4m");
    let shape_only = Style::new().underline_style(UnderlineStyle::Double);
    assert_eq!(
        transition(Style::new(), shape_only, &full_caps()),
        "",
        "no underline without UNDERLINED"
    );
}

#[test]
fn links_are_written_only_where_supported_with_ids_stable_across_frames() {
    let mut buffer = Buffer::empty(AREA);
    buffer.set_string(0, 0, "ab", Style::new());
    buffer.set_string(0, 1, "cd", Style::new());
    buffer.set_link(Rect::new(0, 0, 2, 2), Some("https://example.com/x"));
    let got = packet(
        &mut Encoder::new(),
        &Buffer::empty(AREA),
        &buffer,
        None,
        &full_caps(),
    );
    let opens: Vec<_> = got.match_indices("␛]8;id=").collect();
    assert_eq!(opens.len(), 2, "a link opens again on each row: {got}");
    let ids: Vec<_> = got.split("␛]8;id=").skip(1).map(|s| &s[..8]).collect();
    assert_eq!(ids[0], ids[1]);
    assert_eq!(got.matches("␛]8;;␛\\").count(), 2);
    let none = Capabilities {
        hyperlinks: Support::Unknown,
        ..full_caps()
    };
    let got = packet(
        &mut Encoder::new(),
        &Buffer::empty(AREA),
        &buffer,
        None,
        &none,
    );
    assert!(!got.contains("]8;"), "{got}");
    let mut bad = Buffer::empty(AREA);
    bad.set_string(0, 0, "x", Style::new());
    bad.set_link(
        Rect::new(0, 0, 1, 1),
        Some("https://example.com/\x1b]0;owned\x07"),
    );
    let got = packet(
        &mut Encoder::new(),
        &Buffer::empty(AREA),
        &bad,
        None,
        &full_caps(),
    );
    assert!(!got.contains("owned"), "{got}");
}

#[test]
fn a_full_redraw_clears_and_writes_what_isnt_blank() {
    let caps = full_caps();
    let frame = text(
        AREA,
        &[(0, 0, "a", Style::new()), (4, 2, "z", Style::new())],
    );
    let mut encoder = primed(&frame, &caps);
    encoder.redraw();
    let got = packet(&mut encoder, &frame, &frame, None, &caps);
    assert_eq!(got, "␛[?2026h␛[?25l␛[0m␛[?7l␛[2J␛[Ha␛[3;5Hz␛[?2026l");
    assert_eq!(
        packet(&mut encoder, &frame, &frame, None, &caps),
        "",
        "the redraw happens once"
    );
    // The shape is the user's until a frame asks for another, so a redraw leaves it alone.
    encoder.redraw();
    let cursor = Some(Cursor::at(Position::new(1, 0)));
    let got = packet(&mut encoder, &frame, &frame, cursor, &caps);
    assert!(got.ends_with("␛[1;2H␛[?25h␛[?2026l"), "{got}");
    let bar = Some(Cursor {
        shape: CursorShape::Bar,
        ..Cursor::at(Position::new(1, 0))
    });
    let got = packet(&mut encoder, &frame, &frame, bar, &caps);
    assert_eq!(got, "␛[6 q");
    encoder.redraw();
    let got = packet(&mut encoder, &frame, &frame, bar, &caps);
    assert!(
        got.ends_with("␛[1;2H␛[6 q␛[?25h␛[?2026l"),
        "a shape asked for is sent again: {got}"
    );
    // A redraw sends a shape asked for before again, the default too.
    let cursor = Some(Cursor::at(Position::new(1, 0)));
    encoder.redraw();
    let got = packet(&mut encoder, &frame, &frame, cursor, &caps);
    assert!(
        got.ends_with("␛[1;2H␛[0 q␛[?25h␛[?2026l"),
        "the shape is sent again: {got}"
    );
}

#[test]
fn without_synchronized_output_the_packet_has_no_update() {
    let caps = Capabilities {
        synchronized_output: Support::No,
        ..full_caps()
    };
    let got = packet(
        &mut Encoder::new(),
        &Buffer::empty(AREA),
        &text(AREA, &[(0, 0, "a", Style::new())]),
        None,
        &caps,
    );
    assert_eq!(got, "␛[?25l␛[H␛[0ma");
}

#[test]
fn partial_frames_touch_only_their_regions() {
    let caps = full_caps();
    let before = text(AREA, &[(0, 0, "abcdefghij", Style::new())]);
    let mut after = text(AREA, &[(0, 0, "ABCDEFGHIJ", Style::new())]);
    after.set_string(0, 2, "zz", Style::new());
    let mut encoder = primed(&before, &caps);
    let mut out = Vec::new();
    let stats = encoder.encode_regions(
        &before,
        &after,
        &[Rect::new(2, 0, 2, 1)],
        None,
        &caps,
        &mut out,
    );
    assert_eq!(show(&out), "␛[?2026h␛[3GCD␛[?2026l");
    assert_eq!(stats.cells, 2);
}

#[test]
fn partial_frames_take_in_wide_characters_across_their_edges() {
    let caps = full_caps();
    let area = Rect::new(0, 0, 10, 1);
    let before = text(area, &[(0, 0, "a界b界cdef", Style::new())]);
    let mut after = before.clone();
    let region = Rect::new(2, 0, 3, 1);
    after.set_clip(&[region]);
    after.set_string(2, 0, "x", Style::new());
    after.set_string(4, 0, "界", Style::new());
    after.clear_clip();
    assert_eq!(after.symbol((1, 0)), " ");
    let mut term = emulator(WidthProfile::default(), area, true);
    let mut encoder = Encoder::new();
    feed(
        &mut term,
        &packet(&mut encoder, &Buffer::empty(area), &before, None, &caps),
    );
    let mut out = Vec::new();
    encoder.encode_regions(&before, &after, &[region], None, &caps, &mut out);
    feed(&mut term, &show(&out));
    check_screen(&term, &after, None, &caps).unwrap();
}

#[test]
fn stats_count_cells_and_bytes() {
    let caps = full_caps();
    let next = text(AREA, &[(0, 0, "a界", Style::new())]);
    let mut out = Vec::new();
    let stats = Encoder::new().encode(&Buffer::empty(AREA), &next, None, &caps, &mut out);
    assert_eq!(
        stats,
        FrameStats {
            cells: 3,
            bytes: out.len()
        }
    );
    let frame = CompletedFrame::new(&next, None, stats);
    assert_eq!(frame.stats().cells, 3);
    assert_eq!(frame.cursor(), None);
    assert_eq!(frame.buffer().symbol((1, 0)), "界");
}

#[test]
fn streaming_one_row_up_writes_one_row_of_text() {
    let caps = full_caps();
    let area = Rect::new(0, 0, 30, 12);
    let rows: Vec<String> = (0..13)
        .map(|i| format!("line {i:02} of a long reply"))
        .collect();
    let mut before = Buffer::empty(area);
    let mut after = Buffer::empty(area);
    for y in 0..12u16 {
        before.set_string(0, y, &rows[usize::from(y)], Style::new());
        after.set_string(0, y, &rows[usize::from(y) + 1], Style::new());
    }
    let mut term = emulator(WidthProfile::default(), area, true);
    let mut encoder = Encoder::new();
    encoder.set_scroll_detection(true);
    let first = packet(&mut encoder, &Buffer::empty(area), &before, None, &caps);
    feed(&mut term, &first);
    let got = packet(&mut encoder, &before, &after, None, &caps);
    assert_eq!(
        got,
        "␛[?2026h␛[1;12r␛[S␛[r␛[12Hline 12 of a long reply␛[?2026l"
    );
    feed(&mut term, &got);
    check_screen(&term, &after, None, &caps).unwrap();
}

#[test]
fn the_cursor_is_never_visible_on_a_cell_being_drawn() {
    let caps = full_caps();
    let area = Rect::new(0, 0, 20, 3);
    let mut term = emulator(WidthProfile::default(), area, true);
    let mut encoder = Encoder::new();
    let mut prev = Buffer::empty(area);
    for i in 0..5u16 {
        let next = text(
            area,
            &[
                (0, i % 3, "working…", Style::new()),
                (0, 2, "› typed", Style::new()),
            ],
        );
        let cursor = Cursor::at(Position::new(7, 2));
        feed(
            &mut term,
            &packet(&mut encoder, &prev, &next, Some(cursor), &caps),
        );
        prev = next;
    }
    let drawn_visible = term.events().iter().any(|e| {
        matches!(
            e,
            Event::Drawn {
                cursor_visible: true,
                ..
            }
        )
    });
    assert!(!drawn_visible);
}

// The emulator checks.

fn emulator(profile: WidthProfile, area: Rect, bce: bool) -> Emulator {
    let mut vt = Profile::xterm();
    vt.hyperlinks = true;
    vt.bce = bce;
    vt.width = WidthRules {
        joins_emoji: profile.joins_emoji,
        vs16_widens: profile.vs16_widens,
        ambiguous_wide: profile.ambiguous_wide,
    };
    let mut term = Emulator::with_profile(vt, area.right(), area.bottom());
    feed(&mut term, "␛[?7l");
    term
}

fn feed(term: &mut Emulator, shown: &str) {
    let bytes = shown.replace('␛', "\x1b");
    if let Err(e) = term.feed(bytes.as_bytes()) {
        panic!("the emulator refused {shown:?}: {e:?}");
    }
}

fn vt_color(c: Color) -> tuiframe_vt::Color {
    use tuiframe_vt::Color as Vt;
    match c {
        Color::Reset => Vt::Default,
        Color::Indexed(n) => Vt::Indexed(n),
        Color::Rgb(r, g, b) => Vt::Rgb(r, g, b),
        named => super::color::ansi_index(named)
            .and_then(Vt::named)
            .unwrap_or(Vt::Default),
    }
}

/// The underline's colour as the encoder writes it, a named colour as its palette entry, since
/// `SGR 58` has no named form.
fn vt_underline_color(c: Color) -> tuiframe_vt::Color {
    match super::color::ansi_index(c) {
        Some(n) => tuiframe_vt::Color::Indexed(n),
        None => vt_color(c),
    }
}

fn looks_blank(style: &tuiframe_vt::Style, text: &str) -> bool {
    text == " "
        && style.bg == tuiframe_vt::Color::Default
        && style.underline == Underline::None
        && !(style.reverse || style.strike || style.overline || style.blink || style.rapid_blink)
        && style.link.is_none()
}

/// Whether the emulator's screen shows what the buffer holds, cell for cell, with colours at the
/// terminal's depth, and the cursor where the frame put it.
fn check_screen(
    term: &Emulator,
    buffer: &Buffer,
    cursor: Option<Cursor>,
    caps: &Capabilities,
) -> Result<(), String> {
    check_cells(term, buffer, caps, |_, _| false)?;
    check_cursor(term, buffer, cursor, caps)
}

fn check_cells(
    term: &Emulator,
    buffer: &Buffer,
    caps: &Capabilities,
    skip: impl Fn(u16, u16) -> bool,
) -> Result<(), String> {
    let area = buffer.area();
    let depth = caps.colors;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if skip(x, y) {
                continue;
            }
            let ours = buffer.cell((x, y)).ok_or("a cell is missing")?;
            let theirs = term.cell(x, y).ok_or("the emulator lacks a cell")?;
            let at = || format!("({x}, {y}) of\n{:?}\n{:?}", term.rows(), buffer);
            if ours.width() == 0 {
                let mut lead = x;
                while lead > area.left() && buffer.cell((lead, y)).is_some_and(|c| c.width() == 0) {
                    lead -= 1;
                }
                let leader = buffer.cell((lead, y)).ok_or("a leader is missing")?;
                let room = usize::from(area.right() - lead);
                if leader.has_uncertain_width()
                    && usize::from(crate::text::widest(leader.symbol(), buffer.width_profile()))
                        > room
                {
                    continue;
                }
                if !theirs.is_continuation() {
                    return Err(format!("not a continuation cell at {}", at()));
                }
                continue;
            }
            // An uncertain cluster that a wider measure wouldn't fit before the right edge is
            // written as spaces.
            let room = usize::from(area.right() - x);
            if ours.has_uncertain_width()
                && usize::from(crate::text::widest(ours.symbol(), buffer.width_profile())) > room
            {
                for k in 0..ours.width() {
                    let blank = term.cell(x + k, y).is_some_and(|c| c.text() == " ");
                    if !blank {
                        return Err(format!(
                            "{:?} near the edge isn't blank at {}",
                            ours.symbol(),
                            at()
                        ));
                    }
                }
                continue;
            }
            let style = theirs.style();
            let blank = buffer.cell((x, y)).is_some_and(|c| {
                c.symbol() == " "
                    && c.bg() == Color::Reset
                    && !c.modifier().intersects(
                        Modifier::REVERSED
                            | Modifier::UNDERLINED
                            | Modifier::CROSSED_OUT
                            | Modifier::OVERLINED
                            | Modifier::SLOW_BLINK
                            | Modifier::RAPID_BLINK,
                    )
                    && (c.link().is_none() || caps.hyperlinks != Support::Yes)
            });
            if blank && looks_blank(style, theirs.text()) {
                continue;
            }
            if theirs.text() != ours.symbol() || u16::from(theirs.width()) != ours.width() {
                return Err(format!(
                    "{:?} of width {} where the buffer has {:?} of width {} at {}",
                    theirs.text(),
                    theirs.width(),
                    ours.symbol(),
                    ours.width(),
                    at()
                ));
            }
            let m = ours.modifier();
            let underline = m.contains(Modifier::UNDERLINED).then(|| {
                match ours
                    .underline_style()
                    .filter(|_| caps.underline_styles == Support::Yes)
                {
                    None | Some(UnderlineStyle::Single) => Underline::Single,
                    Some(UnderlineStyle::Double) => Underline::Double,
                    Some(UnderlineStyle::Curly) => Underline::Curly,
                    Some(UnderlineStyle::Dotted) => Underline::Dotted,
                    Some(UnderlineStyle::Dashed) => Underline::Dashed,
                }
            });
            let checks = [
                ("fg", style.fg == vt_color(lower(ours.fg(), depth))),
                ("bg", style.bg == vt_color(lower(ours.bg(), depth))),
                (
                    "underline",
                    style.underline == underline.unwrap_or(Underline::None),
                ),
                ("bold", style.bold == m.contains(Modifier::BOLD)),
                ("dim", style.dim == m.contains(Modifier::DIM)),
                ("italic", style.italic == m.contains(Modifier::ITALIC)),
                ("blink", style.blink == m.contains(Modifier::SLOW_BLINK)),
                (
                    "rapid blink",
                    style.rapid_blink == m.contains(Modifier::RAPID_BLINK),
                ),
                ("reverse", style.reverse == m.contains(Modifier::REVERSED)),
                ("hidden", style.hidden == m.contains(Modifier::HIDDEN)),
                ("strike", style.strike == m.contains(Modifier::CROSSED_OUT)),
                (
                    "overline",
                    style.overline == m.contains(Modifier::OVERLINED),
                ),
                (
                    "underline colour",
                    underline.is_none()
                        || caps.underline_color != Support::Yes
                        || depth == ColorDepth::None
                        || style.underline_color
                            == vt_underline_color(lower(ours.underline_color(), depth)),
                ),
                (
                    "link",
                    style.link.as_ref().map(|l| l.url.as_str())
                        == ours.link().filter(|_| caps.hyperlinks == Support::Yes),
                ),
            ];
            if let Some((what, _)) = checks.iter().find(|(_, ok)| !ok) {
                return Err(format!(
                    "{what} differs, {style:?} for {:?} at {}",
                    ours.style(),
                    at()
                ));
            }
        }
    }
    Ok(())
}

fn check_cursor(
    term: &Emulator,
    buffer: &Buffer,
    cursor: Option<Cursor>,
    caps: &Capabilities,
) -> Result<(), String> {
    let theirs = term.cursor();
    match cursor {
        None if theirs.visible => Err("the cursor shows in a frame without one".to_owned()),
        None => Ok(()),
        Some(c) => {
            let area = buffer.area();
            let x = c.position.x.clamp(area.x, area.right() - 1);
            let y = c.position.y.clamp(area.y, area.bottom() - 1);
            let shape = match c.shape {
                CursorShape::Default => tuiframe_vt::CursorShape::Default,
                CursorShape::Block => tuiframe_vt::CursorShape::Block,
                CursorShape::Underline => tuiframe_vt::CursorShape::Underline,
                CursorShape::Bar => tuiframe_vt::CursorShape::Bar,
            };
            let shape_ok = caps.cursor_shape == Support::No || theirs.shape == shape;
            if theirs.visible && (theirs.x, theirs.y) == (x, y) && shape_ok {
                Ok(())
            } else {
                Err(format!("the cursor is {theirs:?}, not {c:?}"))
            }
        }
    }
}

/// Text that exercises every kind of cluster the buffer holds.
const PIECES: &[&str] = &[
    "a",
    "word",
    " ",
    "  ",
    "界",
    "ｗｉｄｅ",
    "e\u{301}",
    "👍",
    "👍🏽",
    "🇳🇿",
    "❤\u{fe0f}",
    "1\u{fe0f}\u{20e3}",
    "👨\u{200d}👩\u{200d}👧",
    "±",
    "─",
    "\t",
    "\u{1b}[31m",
    "क",
    "\u{903}",
    "\u{1100}",
    "🏽",
    "🇳",
    "🫨",
    "é",
    "Ω",
    "ｶﾞ",
    "♥\u{fe0f}",
];

const STYLES: &[Style] = &[
    Style::new(),
    Style::new().fg(Color::Red).add_modifier(Modifier::BOLD),
    Style::new().add_modifier(Modifier::DIM),
    Style::new().bg(Color::Blue),
    Style::new()
        .fg(Color::Rgb(200, 100, 50))
        .bg(Color::Indexed(236)),
    Style::new()
        .add_modifier(Modifier::UNDERLINED)
        .underline_style(UnderlineStyle::Curly)
        .underline_color(Color::Rgb(255, 0, 0)),
    Style::new().add_modifier(Modifier::REVERSED.union(Modifier::ITALIC)),
    Style::new()
        .fg(Color::Indexed(9))
        .add_modifier(Modifier::BOLD.union(Modifier::DIM)),
    Style::new().add_modifier(
        Modifier::SLOW_BLINK
            .union(Modifier::CROSSED_OUT)
            .union(Modifier::OVERLINED),
    ),
    Style::new()
        .fg(Color::LightCyan)
        .bg(Color::DarkGray)
        .add_modifier(Modifier::HIDDEN),
    Style::new()
        .remove_modifier(Modifier::BOLD)
        .bg(Color::Reset),
    Style::new().add_modifier(Modifier::UNDERLINED),
];

#[derive(Debug, Clone)]
enum Op {
    Line {
        x: u16,
        y: u16,
        spans: Vec<(String, usize)>,
    },
    Style {
        area: Rect,
        style: usize,
    },
    Symbol {
        x: u16,
        y: u16,
        symbol: &'static str,
    },
    Link {
        area: Rect,
        url: usize,
    },
}

#[derive(Debug, Clone)]
struct Frame {
    ops: Vec<Op>,
    /// Rows moved from the last frame first, up when positive, as a stream of text does.
    shift: i8,
    cursor: Option<(u16, u16, u8, bool)>,
    redraw: bool,
    /// The regions of a partial frame, drawn under a clip over the last frame.
    regions: Option<Vec<Rect>>,
}

const W: u16 = 12;
const H: u16 = 5;

fn piece_text() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(PIECES), 1..4).prop_map(|p| p.concat())
}

fn rect() -> impl Strategy<Value = Rect> {
    (0..W, 0..H, 1..W + 1, 1..H + 1).prop_map(|(x, y, w, h)| Rect::new(x, y, w, h))
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        5 => (0..W, 0..H, prop::collection::vec((piece_text(), 0..STYLES.len()), 1..4))
            .prop_map(|(x, y, spans)| Op::Line { x, y, spans }),
        2 => (rect(), 0..STYLES.len()).prop_map(|(area, style)| Op::Style { area, style }),
        2 => (0..W, 0..H, prop::sample::select(PIECES)).prop_map(|(x, y, symbol)| Op::Symbol { x, y, symbol }),
        1 => (rect(), 0..3usize).prop_map(|(area, url)| Op::Link { area, url }),
    ]
}

fn frame() -> impl Strategy<Value = Frame> {
    (
        prop::collection::vec(op(), 0..8),
        prop_oneof![6 => Just(0i8), 1 => -2i8..3],
        prop::option::of((0..W + 2, 0..H + 2, 0..4u8, any::<bool>())),
        prop::bool::weighted(0.1),
        prop::option::weighted(0.3, prop::collection::vec(rect(), 1..3)),
    )
        .prop_map(|(ops, shift, cursor, redraw, regions)| Frame {
            ops,
            shift,
            cursor,
            redraw,
            regions,
        })
}

fn depth() -> impl Strategy<Value = ColorDepth> {
    prop::sample::select(
        &[
            ColorDepth::TrueColor,
            ColorDepth::Ansi256,
            ColorDepth::Ansi16,
            ColorDepth::None,
        ][..],
    )
}

fn support() -> impl Strategy<Value = Support> {
    prop::sample::select(&[Support::Yes, Support::No, Support::Unknown][..])
}

fn caps() -> impl Strategy<Value = Capabilities> {
    (depth(), support(), support(), support(), support()).prop_map(
        |(colors, links, styles, color, shapes)| Capabilities {
            colors,
            hyperlinks: links,
            underline_styles: styles,
            underline_color: color,
            cursor_shape: shapes,
            synchronized_output: Support::Yes,
            ..Capabilities::default()
        },
    )
}

fn width_profile() -> impl Strategy<Value = WidthProfile> {
    (any::<bool>(), any::<bool>(), any::<bool>()).prop_map(|(j, v, a)| WidthProfile {
        joins_emoji: j,
        vs16_widens: v,
        ambiguous_wide: a,
    })
}

const URLS: [&str; 3] = [
    "https://example.com/a",
    "https://example.com/b;c",
    "file:///home/ada/notes.txt",
];

/// Draws `frame` into a new buffer, starting from the last frame's rows moved by its shift.
fn draw(frame: &Frame, last: &Buffer, profile: WidthProfile) -> Buffer {
    let area = last.area();
    let mut next = Buffer::with_width_profile(area, profile);
    if let Some(regions) = &frame.regions {
        next = last.clone();
        next.set_clip(regions);
    } else if frame.shift != 0 {
        for y in area.top()..area.bottom() {
            let from = i32::from(y) + i32::from(frame.shift);
            let Ok(from) = u16::try_from(from) else {
                continue;
            };
            if from >= area.bottom() {
                continue;
            }
            for x in area.left()..area.right() {
                let Some(cell) = last.cell((x, from)) else {
                    continue;
                };
                if cell.width() == 0 {
                    continue;
                }
                let (symbol, style, link) = (
                    cell.symbol().to_owned(),
                    cell.style(),
                    cell.link().map(str::to_owned),
                );
                if let Some(mut target) = next.cell_mut((x, y)) {
                    target
                        .set_symbol(&symbol)
                        .set_style(style)
                        .set_link(link.as_deref());
                }
            }
        }
    }
    for op in &frame.ops {
        match op {
            Op::Line { x, y, spans } => {
                let line = Line::from(
                    spans
                        .iter()
                        .map(|(s, i)| Span::styled(s.as_str(), STYLES[*i]))
                        .collect::<Vec<_>>(),
                );
                next.set_line(*x, *y, &line, W);
            }
            Op::Style { area, style } => next.set_style(*area, STYLES[*style]),
            Op::Symbol { x, y, symbol } => {
                if let Some(mut cell) = next.cell_mut((*x, *y)) {
                    cell.set_symbol(symbol);
                }
            }
            Op::Link { area, url } => next.set_link(*area, Some(URLS[*url])),
        }
    }
    next
}

fn frame_cursor(frame: &Frame) -> Option<Cursor> {
    frame.cursor.map(|(x, y, shape, blinking)| Cursor {
        position: Position::new(x, y),
        shape: [
            CursorShape::Default,
            CursorShape::Block,
            CursorShape::Underline,
            CursorShape::Bar,
        ][usize::from(shape)],
        blinking,
    })
}

/// Draws `frames` one after another and checks the emulator against each, as the property test
/// does.
fn run_frames(
    profile: WidthProfile,
    caps: &Capabilities,
    bce: bool,
    scroll: bool,
    frames: &[Frame],
) -> Result<(), String> {
    let area = Rect::new(0, 0, W, H);
    let mut term = emulator(profile, area, bce);
    let mut encoder = Encoder::new();
    encoder.set_scroll_detection(scroll);
    let mut prev = Buffer::with_width_profile(area, profile);
    for (i, frame) in frames.iter().enumerate() {
        let mut next = draw(frame, &prev, profile);
        let cursor = frame_cursor(frame);
        if frame.redraw {
            encoder.redraw();
        }
        let mut out = Vec::new();
        match &frame.regions {
            Some(regions) => {
                encoder.encode_regions(&prev, &next, regions, cursor.as_ref(), caps, &mut out)
            }
            None => encoder.encode(&prev, &next, cursor.as_ref(), caps, &mut out),
        };
        next.clear_clip();
        term.take_events();
        term.feed(&out)
            .map_err(|e| format!("frame {i}: the emulator refused {:?}: {e:?}", show(&out)))?;
        let visible = term.events().iter().any(|e| {
            matches!(
                e,
                Event::Drawn {
                    cursor_visible: true,
                    ..
                }
            )
        });
        if visible {
            return Err(format!(
                "frame {i}: drew with the cursor showing: {}",
                show(&out)
            ));
        }
        check_screen(&term, &next, cursor, caps)
            .map_err(|e| format!("frame {i}: {e}\nbytes {}", show(&out)))?;
        prev = next;
    }
    Ok(())
}

#[test]
fn a_partial_frame_over_wide_characters_at_its_edge() {
    let line = |x, spans: &[(&str, usize)]| Op::Line {
        x,
        y: 0,
        spans: spans.iter().map(|&(s, i)| (s.to_owned(), i)).collect(),
    };
    let frame = Frame {
        ops: vec![
            line(1, &[("ᄀ", 0), ("ｗｉｄｅ", 3)]),
            line(2, &[(" ᄀ🫨", 0), ("word", 0)]),
        ],
        shift: 0,
        cursor: None,
        redraw: false,
        regions: Some(vec![Rect::new(1, 0, 3, 1)]),
    };
    let profile = WidthProfile {
        ambiguous_wide: true,
        ..WidthProfile::default()
    };
    run_frames(profile, &full_caps(), false, false, &[frame]).unwrap();
}

#[test]
fn a_partial_frame_writes_what_a_wide_character_spilled_past_its_region() {
    let line = |x, spans: &[(&str, usize)]| Op::Line {
        x,
        y: 1,
        spans: spans.iter().map(|&(s, i)| (s.to_owned(), i)).collect(),
    };
    // The 👍 starts in the region and ends past it, and the "3" then breaks it, which leaves a
    // space in its background outside the region, in neither frame a wide character.
    let frame = Frame {
        ops: vec![
            line(5, &[("worde\u{301}👍", 3), ("a", 0)]),
            line(2, &[("a👍🏽", 0), ("a", 0), ("❤\u{fe0f}\u{1b}[31m", 0)]),
        ],
        shift: 0,
        cursor: None,
        redraw: false,
        regions: Some(vec![Rect::new(7, 0, 4, 2)]),
    };
    run_frames(
        WidthProfile::default(),
        &full_caps(),
        false,
        false,
        &[frame],
    )
    .unwrap();
}

#[test]
fn a_partial_frame_that_breaks_a_wide_character_inside_it() {
    let line = |x, spans: &[(&str, usize)]| Op::Line {
        x,
        y: 3,
        spans: spans.iter().map(|&(s, i)| (s.to_owned(), i)).collect(),
    };
    let frame = Frame {
        ops: vec![line(1, &[(" 👍", 3)]), line(2, &[("word", 0)])],
        shift: 0,
        cursor: None,
        redraw: false,
        regions: Some(vec![Rect::new(1, 1, 3, 3)]),
    };
    let profile = WidthProfile {
        joins_emoji: true,
        ..WidthProfile::default()
    };
    let next = draw(
        &frame,
        &Buffer::with_width_profile(Rect::new(0, 0, W, H), profile),
        profile,
    );
    let bgs: Vec<_> = (0..6).map(|x| next.cell((x, 3)).unwrap().bg()).collect();
    assert_eq!(
        bgs,
        [
            Color::Reset,
            Color::Blue,
            Color::Blue,
            Color::Blue,
            Color::Reset,
            Color::Reset
        ]
    );
    run_frames(profile, &full_caps(), false, false, &[frame]).unwrap();
}

/// The clusters of `buffer` that a terminal measuring by `rules` gives another width, as their
/// rows and the columns of their own cells.
fn disagreements(buffer: &Buffer, rules: WidthRules) -> Vec<(u16, u16, u16)> {
    let area = buffer.area();
    let mut spans = Vec::new();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let Some(cell) = buffer.cell((x, y)) else {
                continue;
            };
            let mine = cell.width();
            if mine > 0 && mine != u16::from(rules.width(cell.symbol())) {
                spans.push((y, x, x + mine));
            }
        }
    }
    spans
}

/// Whether the screen matches the buffer outside the cells of the clusters in `spans`, and shows
/// in those cells only blanks or the cluster itself, never text left from before.
fn check_damage(
    term: &Emulator,
    buffer: &Buffer,
    caps: &Capabilities,
    spans: &[(u16, u16, u16)],
) -> Result<(), String> {
    let within = |x: u16, y: u16| spans.iter().find(|&&(sy, a, b)| sy == y && a <= x && x < b);
    check_cells(term, buffer, caps, |x, y| within(x, y).is_some())?;
    for y in 0..buffer.area().height {
        for x in 0..buffer.area().width {
            let Some(&(_, start, _)) = within(x, y) else {
                continue;
            };
            let theirs = term.cell(x, y).ok_or("the emulator lacks a cell")?;
            let cluster = buffer.symbol((start, y));
            let text = theirs.text();
            if !(text == " " || theirs.is_continuation() || text == cluster) {
                return Err(format!(
                    "{text:?} at ({x}, {y}) is in the cells of {cluster:?}, which hold only blanks or it\n{:?}",
                    term.rows()
                ));
            }
        }
    }
    Ok(())
}

proptest! {
    /// For random sequences of random frames, the emulator's screen after the encoder's bytes
    /// equals the last buffer, cell for cell, and its cursor equals the frame's.
    #[test]
    fn the_terminal_shows_every_frame(
        profile in width_profile(),
        caps in caps(),
        bce in any::<bool>(),
        scroll in any::<bool>(),
        frames in prop::collection::vec(frame(), 1..6),
    ) {
        if let Err(e) = run_frames(profile, &caps, bce, scroll, &frames) {
            prop_assert!(false, "{}", e);
        }
    }

    /// With the emulator measuring emoji sequences by other rules than the buffer, a cluster they
    /// disagree on misdraws only its own cells, which show blanks rather than text from before,
    /// and the same after a frame that changes that cluster's cell alone.
    #[test]
    fn a_width_disagreement_misdraws_only_its_cluster(
        ours in width_profile(),
        theirs in (any::<bool>(), any::<bool>()),
        rows in prop::collection::vec(prop::collection::vec(piece_text(), 1..4), 1..H as usize + 1),
    ) {
        let theirs = WidthProfile { joins_emoji: theirs.0, vs16_widens: theirs.1, ..ours };
        let caps = full_caps();
        let area = Rect::new(0, 0, W, H);
        let mut term = emulator(theirs, area, true);
        let mut encoder = Encoder::new();
        let mut filler = Buffer::with_width_profile(area, ours);
        for y in 0..H {
            filler.set_string(0, y, "abcdefghijkl", Style::new());
        }
        let mut out = Vec::new();
        encoder.encode(&Buffer::empty(area), &filler, None, &caps, &mut out);
        term.feed(&out).unwrap();
        let mut next = Buffer::with_width_profile(area, ours);
        for (y, pieces) in (0..).zip(&rows) {
            next.set_string(0, y, pieces.concat(), Style::new());
        }
        out.clear();
        encoder.encode(&filler, &next, None, &caps, &mut out);
        term.feed(&out).unwrap();
        let rules = WidthRules { joins_emoji: theirs.joins_emoji, vs16_widens: theirs.vs16_widens, ambiguous_wide: theirs.ambiguous_wide };
        let spans = disagreements(&next, rules);
        if let Err(e) = check_damage(&term, &next, &caps, &spans) {
            prop_assert!(false, "{}\nspans {:?}\nbytes {}", e, spans, show(&out));
        }
        // A frame that changes only one such cluster's cell.
        if let Some(&(y, x, _)) = spans.first() {
            let mut restyled = next.clone();
            restyled.set_style(Rect::new(x, y, 1, 1), Style::new().add_modifier(Modifier::BOLD));
            out.clear();
            encoder.encode(&next, &restyled, None, &caps, &mut out);
            term.feed(&out).unwrap();
            if let Err(e) = check_damage(&term, &restyled, &caps, &spans) {
                prop_assert!(false, "after the cluster changed alone, {}\nspans {:?}\nbytes {}", e, spans, show(&out));
            }
        }
    }
}
