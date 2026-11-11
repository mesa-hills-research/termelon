# Porting from ratatui and crossterm

Termelon takes the place of both. Its text, style and layout types and its widgets keep ratatui's
names, fields and builders, so most drawing code ports by changing its imports. The terminal,
events and tests change more, and this page goes through them.

```toml
[dependencies]
termelon = "0.1"

[dev-dependencies]
termelon = { version = "0.1", features = ["testing"] }
```

## Imports

| ratatui and crossterm | termelon |
|---|---|
| `ratatui::text::{Span, Line, Text}` | `termelon::text::{Span, Line, Text}` |
| `ratatui::style::{Style, Color, Modifier}` | `termelon::style::{Style, Color, Modifier}` |
| `ratatui::layout::{Rect, Position, Size, Layout, Constraint, Alignment}` | `termelon::layout::{...}`, the same names |
| `ratatui::buffer::Buffer` | `termelon::buffer::Buffer` |
| `ratatui::widgets::{Widget, Block, Borders, Clear, Paragraph, Wrap, Padding}` | `termelon::widgets::{...}`, the same names |
| `ratatui::symbols::border::{Set, PLAIN, ROUNDED, ...}` | `termelon::widgets::BorderSet`, with `BorderSet::PLAIN`, `ROUNDED`, `THICK`, `DOUBLE` and `ASCII` |
| `ratatui::{Terminal, Frame}` | `termelon::{Terminal, Frame}` |
| `crossterm::event::{Event, KeyEvent, KeyCode, KeyModifiers, KeyEventKind, MouseEvent, MouseEventKind}` | `termelon::event::{...}`, the same names |
| `ratatui::backend::TestBackend` | `termelon::testing::TestTerminal` |

The most used types are also at the crate's root, as in `use termelon::{Line, Span, Style}`.

## The terminal

One call takes the place of raw mode, the alternate screen, mouse capture, bracketed paste,
focus reports, the kitty keyboard flags and the backend. Closing, dropping or a panic gives all of
it back.

```rust
use termelon::{Drawn, Options, Terminal};

termelon::install_panic_hook();
let mut terminal = Terminal::open(Options::default())?;
match terminal.draw(|frame| ui(frame, &app))? {
    Drawn::Written(done) => { /* done.buffer(), done.cursor(), done.stats() */ }
    Drawn::Held => { /* drawn with the next event, or after terminal.written().await */ }
    _ => {}
}
terminal.close()?;
```

| ratatui and crossterm | termelon |
|---|---|
| `enable_raw_mode`, `EnterAlternateScreen`, `EnableMouseCapture`, `EnableBracketedPaste`, `EnableFocusChange`, `PushKeyboardEnhancementFlags` | `Terminal::open(options)`, with `Options` choosing the mouse mode, paste, focus and keyboard |
| `disable_raw_mode`, `LeaveAlternateScreen` and the rest | `terminal.close()`, or dropping it |
| `Terminal::new(CrosstermBackend::new(stdout()))` | Nothing. The terminal writes through a thread of its own |
| `terminal.draw(...)?` returning the completed frame | `terminal.draw(...)?` returning `Drawn`, `Held` while the terminal hasn't taken the last frame |
| `terminal.clear()` | `terminal.clear()`, which makes the next frame redraw everything without asking the terminal anything |
| `terminal.size()` | `terminal.size()`, read without a system call |
| `execute!(stdout, SetTitle(..))` | `terminal.set_title(..)` |
| An OSC 52 write of your own | `terminal.copy_to_clipboard(..)` |
| A BEL of your own | `terminal.bell()` |
| Printing after the program ends | `terminal.close_with_tail(bytes)` |

Nothing may write to standard output or standard error while the terminal is open, since its writer
thread owns the screen. A frame that's held isn't lost. The next one carries every change since the
last frame the terminal took, so a slow terminal gets one current frame rather than a backlog.

`terminal.draw_regions(&regions, ...)` is new. It redraws a few rectangles over the last frame, such
as a spinner, and writes only what changed in them.

## Frames and buffers

`Frame` has `area()`, `render_widget(widget, area)`, `buffer_mut()` and `set_cursor_position(pos)`
as in ratatui, and `set_cursor(Some(Cursor { position, shape, blinking }))` for a cursor shape. A
frame that sets no cursor hides it.

The buffer can't be indexed, since a cell alone can't say its text when it holds a grapheme cluster.

| ratatui | termelon |
|---|---|
| `buf[(x, y)].symbol()` | `buf.symbol((x, y))`, or `buf.cell((x, y))?.symbol()` |
| `buf[(x, y)].fg`, `.bg`, `.modifier` | `buf.cell((x, y))?.fg()`, `.bg()`, `.modifier()` |
| `buf[(x, y)].set_symbol(s).set_style(style)` | `buf.cell_mut((x, y))?.set_symbol(s).set_style(style)` |
| `buf.area` | `buf.area()` |
| `buf.set_string`, `set_stringn`, `set_span`, `set_line`, `set_style` | The same |

Positions outside the buffer give `None` rather than a panic. The cells a wide character covers give
an empty symbol, so joining a row's symbols gives the row as the terminal shows it. Writing over
part of a wide character turns the rest of it into spaces, where ratatui leaves half a character
the terminal won't show.

## Text and widths

`Span`, `Line` and `Text` have ratatui's fields (`content`, `style`, `spans`, `lines`, `alignment`)
and constructors. `width()` measures with the terminal's width profile, which `Terminal::open` makes
the process's, so a program's wrapping and alignment agree with the cells it draws. That's where it
differs from ratatui's unicode-width measure. An emoji sequence the terminal doesn't join takes the
cells it really takes, and a control character takes none. `text::wrap` breaks a line into rows by
words, as a wrapped paragraph does.

## Widgets

`Block`, `Borders`, `Clear`, `Paragraph`, `Wrap` and `Padding` take ratatui's builders and draw what
ratatui's draw, cell for cell. One difference is on purpose. A wrapped paragraph moves a wide
character that would cross the right edge of a row to the next row, where ratatui lets it overflow
by a cell. `BorderType` and `border_type` aren't there, and `border_set(BorderSet::ROUNDED)` does
what `border_type(BorderType::Rounded)` did.

List, Table, Tabs, Gauge, Scrollbar, Sparkline, Chart, Canvas and the stateful widgets aren't in
termelon. A widget of your own ports with the `Widget` trait, whose `render(self, area, buf)` has
ratatui's shape.

## Events

`terminal.events()` gives an `Events`, which every call shares.

| crossterm | termelon |
|---|---|
| `event::poll(timeout)` and `event::read()` | `events.next_timeout(timeout)`, `Ok(None)` when nothing came |
| `event::read()` in a loop until `poll` says empty | `events.batch()`, everything already queued |
| `EventStream` | `events.next().await`, on any executor, and `Stream` with the `stream` feature |
| `Event::Resize(w, h)` | `Event::Resize(Size)` |
| `Event::Key(KeyEvent { code, modifiers, kind, .. })` | The same fields |

Key releases come from the Windows console, as in crossterm, and under the kitty protocol where
`Options::keyboard` asks for them, so a program keeps filtering `KeyEventKind::Release` where it
did. A typed capital is `Char('A')` with `SHIFT`, Shift+Tab is `BackTab`, and Ctrl+J differs from
Enter, as in crossterm. `Event::Paste` holds the whole paste, with its line breaks as `\n`. After a
suspend (`terminal.suspend()`) and a resume, the events get `Event::Resumed` and the next frame
redraws everything.

## Tests

`TestTerminal` replaces `TestBackend` with `Terminal::new`. It runs the real encoder and feeds a
terminal emulator, so a test sees what a terminal would show.

| ratatui | termelon |
|---|---|
| `Terminal::new(TestBackend::new(80, 24))` | `TestTerminal::new(80, 24)` |
| `terminal.draw(...)` | `term.draw(...)`, never held |
| `terminal.backend().buffer()` | `term.screen()` for what the terminal shows, or the buffer of the `CompletedFrame` that `draw` returns |
| `terminal.backend().assert_buffer_lines([...])` | `assert_eq!(term.screen().rows(), [...])`, or `golden::assert_text` |
| `terminal.get_cursor_position()` | `term.cursor()`, with the shape, and `None` when it's hidden |
| `backend.resize(w, h)` | `term.resize(w, h)` |

[testing.md](testing.md) covers goldens, styled text, terminal profiles and the bytes.
