# Testing

`tuiframe::testing::TestTerminal` is a terminal in memory. It draws frames through the same code
as `Terminal` (the buffers, the diff and the encoder) and feeds the bytes to `tuiframe-vt`, a
terminal emulator, so a test checks what a terminal would show, the cursor included, rather than
what the buffer holds. Goldens keep a screen's text or styled text in a file beside the test.

The module comes with the `testing` feature.

```toml
[dev-dependencies]
tuiframe = { version = "0.1", features = ["testing"] }
```

## A test

A program that draws with a function of its own tests it by drawing into a `TestTerminal`.

```rust
use tuiframe::layout::{Position, Rect};
use tuiframe::testing::{TestTerminal, golden};
use tuiframe::text::Line;
use tuiframe::{CursorShape, Frame};

fn draw(frame: &mut Frame<'_>, prompt: &str) {
    let row = Rect::new(0, frame.area().height - 1, frame.area().width, 1);
    frame.render_widget(Line::from(format!("› {prompt}")), row);
    frame.set_cursor_position((2 + prompt.len() as u16, row.y));
}

#[test]
fn the_prompt_shows_on_the_last_row() {
    let mut term = TestTerminal::new(40, 10);
    term.draw(|frame| draw(frame, "ask")).unwrap();
    let cursor = term.cursor().unwrap();
    assert_eq!(cursor.position, Position::new(5, 9));
    assert_eq!(cursor.shape, CursorShape::Default);
    golden::assert_text("tests/goldens/prompt.txt", &term.screen().text());
    golden::assert_styled("tests/goldens/prompt.styled", &term.screen().styled());
}
```

`draw` returns the `CompletedFrame`, as `Terminal::draw` does, so a test can also look at the buffer
the frame drew. It fails when the frame wrote a sequence the emulator doesn't know, which no frame
should.

## The screen

`term.screen()` reads back what the terminal shows.

| Method | Gives |
|---|---|
| `text()` | The rows joined with `\n`, wide characters' continuation cells left out and trailing spaces trimmed |
| `rows()` | The same rows, one string each |
| `styled()` | The screen in the styled-text format, below |
| `to_html()` | An HTML fragment with the colours and attributes, for looking at a screen in a browser |
| `cursor()` | The cursor's position, shape and blinking, or `None` when it's hidden |
| `cell(x, y)` | One cell, with its text, its width and its style as the terminal has it |

Colours on the screen are the ones the terminal got. A frame that paints in RGB shows as the nearest
of 256 colours on a terminal with 256, and a space whose style doesn't show (bold with no
background) may arrive as a plain space, since the encoder writes what looks the same.

`term.emulator()` gives the whole emulator, for what the screen doesn't show, such as the modes set,
the title, OSC 52 writes, or the events, which list each character drawn and whether the cursor
showed then.

## Styled text

Styled text is a line format that reads well in a diff. It gives the size, the cursor, a `text`
line for each row, and after it a `style` line for each run of cells in one style other than the
default, with the row, the start and end columns and the attributes.

```text
size 12 2
cursor 0 1 hidden
text 0 "╭ build ─╮"
style 0 0 10 fg=#7f8ea3
text 1 "failed: 2"
style 1 0 7 fg=red bold
```

`tuiframe-vt`'s documentation gives the format in full, version 1.

## Goldens

`golden::assert_text(path, &text)` and `golden::assert_styled(path, &styled)` compare with the file
at `path`, relative to the package's directory, where `cargo test` runs tests. A difference fails
the test with the lines that differ, marked `-` for the golden and `+` for what the test got.

```text
the text differs from the golden tests/goldens/prompt.txt, whose lines are marked - and the test's +.
Run the test with UPDATE_GOLDENS=1 to accept it.
@@ line 8 @@
  
  
- › ask
+ › ask again
```

`UPDATE_GOLDENS=1 cargo test` writes each golden with what its test got, creating missing files and
directories, instead of failing. Review the diff of the golden files before committing them.

## Terminals and capabilities

A `TestTerminal` plays xterm unless `with_profile` names another terminal, one of the profiles of
`tuiframe-vt` (`xterm`, `kitty`, `konsole`, `windows_terminal`, `tmux_3_4`, `linux_console` and
`silent`), which `tuiframe::testing::vt` re-exports. A profile decides how the emulator measures
characters, which modes it has and what it answers. The capabilities are what the startup probe
finds in that terminal, as `Terminal::open` would, unless `with_caps` gives them, and
`with_options` sets the `Options` the setup follows.

```rust
use tuiframe::caps::{Capabilities, ColorDepth};
use tuiframe::testing::{TestTerminal, vt};

let mut caps = Capabilities::default();
caps.colors = ColorDepth::Ansi16;
let term = TestTerminal::new(80, 24).with_profile(vt::Profile::kitty()).with_caps(caps);
```

Frames measure text with the capabilities' width profile, and so does the program's own measuring
inside the draw closure, `Line::width` and `text::width` included. Code that measures outside it
can run under `text::with_width_profile`.

A profile can take quirks, behaviours some terminals have that a program shouldn't rely on, such as
ignoring the autowrap switch or leaving half of an overwritten wide character. A test that draws
under each of `vt::Quirk::ALL` checks that a screen shows right whatever the terminal does there.

```rust
for quirk in vt::Quirk::ALL {
    let mut term = TestTerminal::new(40, 10).with_profile(vt::Profile::xterm().with_quirk(quirk));
    term.draw(|frame| draw(frame, "ask")).unwrap();
    assert_eq!(term.screen().rows()[9], "› ask", "{quirk:?}");
}
```

## Partial frames, resizes and clears

`draw_regions(&regions, ...)` draws a partial frame as `Terminal::draw_regions` does. It returns
`Drawn::FullFrameDue` without calling the closure before the first frame, after `resize(width,
height)` and after `clear()`, and the next `draw` then redraws the whole screen.

## Bytes

`take_output()` gives the bytes written since the last call, as a terminal would have got them. The
first frame's begin with the setup a terminal gets at open (the alternate screen, the modes the
options turn on), so a test of a later frame's bytes takes the output once before drawing it.

```rust
let mut term = TestTerminal::new(20, 2);
term.draw(|frame| frame.render_widget("ready", frame.area())).unwrap();
term.take_output();
term.draw(|frame| frame.render_widget("ready", frame.area())).unwrap();
assert!(term.take_output().is_empty(), "an unchanged frame writes nothing");
```
