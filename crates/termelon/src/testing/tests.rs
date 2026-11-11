use super::*;
use crate::caps::{ColorDepth, Support, WidthProfile};
use crate::style::{Color, Modifier, Style};
use crate::terminal::MouseMode;
use crate::text::{Line, Span};
use crate::widgets::{Block, BorderSet, Clear};

fn bytes(term: &mut TestTerminal) -> String {
    String::from_utf8_lossy(&term.take_output()).into_owned()
}

#[test]
fn the_cursor_is_where_the_terminal_shows_it() {
    let mut term = TestTerminal::new(12, 3).with_profile(vt::Profile::kitty());
    let bar = Cursor {
        position: Position::new(4, 2),
        shape: CursorShape::Bar,
        blinking: true,
    };
    let done = term
        .draw(|frame| {
            frame.render_widget("› ask", Rect::new(0, 2, 12, 1));
            frame.set_cursor(Some(bar));
        })
        .unwrap();
    assert_eq!(done.cursor(), Some(bar));
    assert_eq!(term.cursor(), Some(bar));
    term.draw(|frame| frame.render_widget("› ask", Rect::new(0, 2, 12, 1)))
        .unwrap();
    assert_eq!(term.cursor(), None);
    assert!(term.screen().styled().contains("cursor 4 2 hidden"));
}

#[test]
fn a_shape_the_terminal_lacks_shows_as_its_default() {
    let mut profile = vt::Profile::xterm();
    profile.cursor_shape = false;
    let mut term = TestTerminal::new(5, 1).with_profile(profile);
    term.draw(|frame| {
        frame.set_cursor(Some(Cursor {
            position: Position::new(2, 0),
            shape: CursorShape::Underline,
            blinking: false,
        }));
    })
    .unwrap();
    assert_eq!(term.cursor(), Some(Cursor::at(Position::new(2, 0))));
}

#[test]
fn the_screen_reads_back_as_styled_text_and_html() {
    let mut term = TestTerminal::new(10, 3)
        .with_profile(vt::Profile::kitty())
        .with_caps(Capabilities {
            colors: ColorDepth::TrueColor,
            ..Capabilities::default()
        });
    term.draw(|frame| {
        let area = frame.area();
        frame.render_widget(Clear, area);
        let block = Block::bordered()
            .border_set(BorderSet::ROUNDED)
            .border_style(Style::new().fg(Color::Rgb(0x7f, 0x8e, 0xa3)))
            .title(Line::from(Span::styled(
                " Go ",
                Style::new().add_modifier(Modifier::BOLD),
            )));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(Span::styled("ok", Style::new().fg(Color::Green)), inner);
        frame.set_cursor_position((3, 1));
    })
    .unwrap();
    let screen = term.screen();
    assert_eq!(screen.text(), "╭ Go ────╮\n│ok      │\n╰────────╯");
    // (The title's first space shows plain. A bold space in the border's colour looks blank, and
    // a full frame skips cells that look blank.)
    assert_eq!(
        screen.styled(),
        "size 10 3\n\
         cursor 3 1 visible default\n\
         text 0 \"╭ Go ────╮\"\n\
         style 0 0 1 fg=#7f8ea3\n\
         style 0 2 5 fg=#7f8ea3 bold\n\
         style 0 5 10 fg=#7f8ea3\n\
         text 1 \"│ok      │\"\n\
         style 1 0 1 fg=#7f8ea3\n\
         style 1 1 3 fg=green\n\
         style 1 9 10 fg=#7f8ea3\n\
         text 2 \"╰────────╯\"\n\
         style 2 0 10 fg=#7f8ea3\n"
    );
    let html = screen.to_html();
    assert!(html.starts_with("<pre"), "{html}");
    assert!(html.contains("ok"));
    let cell = screen.cell(1, 1).unwrap();
    assert_eq!((cell.text(), cell.style().fg), ("o", vt::Color::Green));
    assert!(screen.cell(10, 0).is_none());
}

#[test]
fn colours_reach_the_screen_as_the_terminal_shows_them() {
    let gray = Style::new().fg(Color::Rgb(0x80, 0x80, 0x80));
    let draw = |term: &mut TestTerminal| {
        term.draw(|frame| frame.render_widget(Span::styled("x", gray), frame.area()))
            .unwrap();
        term.screen().cell(0, 0).map(|c| c.style().fg)
    };
    let mut term = TestTerminal::new(2, 1).with_profile(vt::Profile::kitty());
    assert_eq!(draw(&mut term), Some(vt::Color::Rgb(0x80, 0x80, 0x80)));
    let sixteen = Capabilities {
        colors: ColorDepth::Ansi16,
        ..Capabilities::default()
    };
    let mut term = TestTerminal::new(2, 1).with_caps(sixteen);
    assert_eq!(draw(&mut term), Some(vt::Color::DarkGray));
}

#[test]
fn a_partial_frame_changes_its_regions_alone() {
    let mut term = TestTerminal::new(8, 2);
    let spinner = Rect::new(0, 1, 1, 1);
    let early = term.draw_regions(&[spinner], |_| unreachable!()).unwrap();
    assert!(matches!(early, Drawn::FullFrameDue));
    term.draw(|frame| {
        frame.render_widget("⠋ Working", Rect::new(0, 1, 8, 1));
        frame.render_widget("title", Rect::new(0, 0, 8, 1));
    })
    .unwrap();
    term.take_output();
    let drawn = term
        .draw_regions(&[spinner], |frame| {
            frame.render_widget("⠙ dropped", Rect::new(0, 1, 8, 1));
        })
        .unwrap();
    let Drawn::Written(done) = drawn else {
        panic!("a partial frame was due");
    };
    assert_eq!(done.stats().cells, 1);
    assert_eq!(term.screen().rows(), ["title", "⠙ Workin"]);
    let partial = bytes(&mut term);
    assert!(
        partial.contains('⠙') && !partial.contains("dropped"),
        "{partial:?}"
    );
    term.clear();
    let due = term.draw_regions(&[spinner], |_| unreachable!()).unwrap();
    assert!(matches!(due, Drawn::FullFrameDue));
}

#[test]
fn a_resize_makes_the_next_frame_redraw_everything() {
    let mut term = TestTerminal::new(10, 3);
    term.draw(|frame| frame.render_widget("first", frame.area()))
        .unwrap();
    term.take_output();
    term.resize(6, 2);
    assert_eq!(term.size(), Size::new(6, 2));
    assert!(matches!(
        term.draw_regions(&[Rect::new(0, 0, 1, 1)], |_| unreachable!()),
        Ok(Drawn::FullFrameDue)
    ));
    let done = term
        .draw(|frame| {
            assert_eq!(frame.area(), Rect::new(0, 0, 6, 2));
            frame.render_widget("second line", Rect::new(0, 1, 6, 1));
        })
        .unwrap();
    assert_eq!(done.buffer().area(), Rect::new(0, 0, 6, 2));
    assert!(bytes(&mut term).contains("\x1b[2J"));
    assert_eq!(term.screen().rows(), ["", "second"]);
    term.draw(|frame| frame.render_widget("second line", Rect::new(0, 1, 6, 1)))
        .unwrap();
    assert_eq!(term.take_output(), b"", "an unchanged frame writes nothing");
}

#[test]
fn the_output_starts_with_the_setup_for_the_options() {
    let mut term = TestTerminal::new(4, 1).with_profile(vt::Profile::kitty());
    term.draw(|_| {}).unwrap();
    let output = bytes(&mut term);
    assert!(
        output.starts_with("\x1b[22;0t\x1b[?1049h\x1b[?25l\x1b[?7l"),
        "{output:?}"
    );
    assert!(output.contains("\x1b[?1002h"));
    assert!(term.emulator().alternate_screen());
    assert!(term.emulator().mode(1002));
    let mut term = TestTerminal::new(4, 1)
        .with_profile(vt::Profile::kitty())
        .with_options(Options::default().mouse(MouseMode::Off));
    term.draw(|_| {}).unwrap();
    assert!(!bytes(&mut term).contains("\x1b[?1002h"));
    assert!(!term.emulator().mode(1002));
}

#[test]
fn capabilities_come_from_probing_the_profile_unless_given() {
    let xterm = TestTerminal::new(4, 1);
    assert_eq!(xterm.capabilities().synchronized_output, Support::No);
    let kitty = TestTerminal::new(4, 1).with_profile(vt::Profile::kitty());
    assert_eq!(kitty.capabilities().synchronized_output, Support::Yes);
    assert_eq!(kitty.capabilities().terminal.name.as_deref(), Some("kitty"));
    let given = Capabilities::default();
    let term = TestTerminal::new(4, 1)
        .with_caps(given.clone())
        .with_profile(vt::Profile::kitty());
    assert_eq!(term.capabilities(), &given);

    let mut kitty = kitty;
    kitty
        .draw(|frame| frame.render_widget("a", frame.area()))
        .unwrap();
    assert!(bytes(&mut kitty).contains("\x1b[?2026h"));
    let mut xterm = xterm;
    xterm
        .draw(|frame| frame.render_widget("a", frame.area()))
        .unwrap();
    assert!(!bytes(&mut xterm).contains("\x1b[?2026h"));
}

#[test]
fn text_measures_with_the_terminals_width_profile_while_drawing() {
    let family = "👨\u{200d}👩\u{200d}👧";
    let joining = Capabilities {
        width_profile: WidthProfile {
            joins_emoji: true,
            ..WidthProfile::default()
        },
        ..Capabilities::default()
    };
    let mut term = TestTerminal::new(8, 1)
        .with_profile(vt::Profile::kitty())
        .with_caps(joining);
    let mut measured = 0;
    term.draw(|frame| {
        measured = Line::from(family).width();
        frame.render_widget(format!("{family}ab"), frame.area());
    })
    .unwrap();
    assert_eq!(measured, 2);
    assert_eq!(term.screen().rows(), [format!("{family}ab")]);
    assert_eq!(term.screen().cell(2, 0).map(vt::Cell::text), Some("a"));
}

#[test]
fn a_sequence_the_emulator_doesnt_know_is_an_error() {
    let mut term = TestTerminal::new(6, 1);
    term.draw(|frame| frame.render_widget("ok", frame.area()))
        .unwrap();
    // (The encoder knows nothing the emulator doesn't, so this feeds the emulator directly.)
    assert!(term.send(b"\x1b[?9999h").is_err());
    assert_eq!(term.screen().rows(), ["ok"]);
}
