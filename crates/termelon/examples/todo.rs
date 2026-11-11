//! A to-do list. The arrow keys choose an item, Space checks it off, and q or Esc quits.
//!
//! ```sh
//! cargo run --example todo
//! ```

#![cfg_attr(
    not(any(unix, windows)),
    expect(dead_code, reason = "the terminal is on Unix and Windows")
)]

#[cfg(any(unix, windows))]
fn main() -> Result<(), termelon::Error> {
    use std::time::Duration;

    use termelon::{Drawn, Event, KeyCode, KeyEventKind, Options, Terminal};

    let mut app = App::new(&[
        "Water the plants",
        "Answer the letter",
        "Fix the bike",
        "Bake bread",
    ]);
    let mut terminal = Terminal::open(Options::default())?;
    let mut events = terminal.events();
    loop {
        // (A frame the terminal hasn't taken yet is drawn again in a moment, with whatever changed
        // meanwhile.)
        let held = matches!(terminal.draw(|frame| draw(frame, &app))?, Drawn::Held);
        let wait = if held {
            Duration::from_millis(10)
        } else {
            Duration::from_secs(60)
        };
        let Some(event) = events.next_timeout(wait)? else {
            continue;
        };
        let Event::Key(key) = event else {
            continue;
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => break,
            KeyCode::Up => app.chosen = app.chosen.saturating_sub(1),
            KeyCode::Down => app.chosen = (app.chosen + 1).min(app.items.len() - 1),
            KeyCode::Char(' ') => {
                let item = &mut app.items[app.chosen];
                item.done = !item.done;
            }
            _ => {}
        }
    }
    terminal.close()
}

#[cfg(not(any(unix, windows)))]
fn main() -> Result<(), &'static str> {
    Err("the terminal is on Unix and Windows only")
}

/// An item of the list.
struct Item {
    text: String,
    done: bool,
}

/// What the program shows.
struct App {
    items: Vec<Item>,
    chosen: usize,
}

impl App {
    fn new(items: &[&str]) -> Self {
        Self {
            items: items
                .iter()
                .map(|text| Item {
                    text: (*text).to_owned(),
                    done: false,
                })
                .collect(),
            chosen: 0,
        }
    }
}

/// Draws the list in a box, and the keys on the last row.
fn draw(frame: &mut termelon::Frame<'_>, app: &App) {
    use termelon::layout::{Constraint, Layout};
    use termelon::style::{Color, Modifier, Style};
    use termelon::text::{Line, Span};
    use termelon::widgets::{Block, BorderSet};

    let [list, keys] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
    let left = app.items.iter().filter(|item| !item.done).count();
    let block = Block::bordered()
        .border_set(BorderSet::ROUNDED)
        .border_style(Style::new().fg(Color::DarkGray))
        .title(Line::from(Span::styled(
            " To do ",
            Style::new().add_modifier(Modifier::BOLD),
        )))
        .title_bottom(Line::from(format!(" {left} left ")).right_aligned());
    let inside = block.inner(list);
    frame.render_widget(block, list);
    for (i, (item, row)) in app.items.iter().zip(inside.rows()).enumerate() {
        let (mark, mut style) = if item.done {
            ("✓ ", Style::new().fg(Color::DarkGray))
        } else {
            ("· ", Style::new())
        };
        if i == app.chosen {
            style = style.add_modifier(Modifier::REVERSED);
        }
        let line = Line::from(vec![Span::raw(mark), Span::raw(item.text.as_str())]).style(style);
        frame.render_widget(line, row);
    }
    let dim = Style::new().fg(Color::DarkGray);
    let hints = Line::from(vec![
        Span::raw(" ↑↓"),
        Span::styled(" choose  ", dim),
        Span::raw("space"),
        Span::styled(" check  ", dim),
        Span::raw("q"),
        Span::styled(" quit", dim),
    ]);
    frame.render_widget(hints, keys);
}
