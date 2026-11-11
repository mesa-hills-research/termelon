# Termelon

A terminal user interface library for Rust. It covers the whole stack a full-screen terminal program
needs, in pure Rust:

- terminal setup and raw mode on Linux, macOS and Windows (conhost and Windows Terminal)
- input, with the kitty keyboard protocol, Windows key records, SGR mouse, bracketed paste and focus
- a 16-byte cell buffer with grapheme clusters, wide characters, true colour, underline styles and
  hyperlinks
- diffed output, written as one synchronized update per frame by a thread of its own
- capability detection in one round trip, with fallbacks for terminals that lack a feature
- text, a small layout, and the basic widgets
- a test terminal backed by a terminal emulator, for goldens of what a terminal really shows

It has no C dependencies, and its unsafe code is limited to the operating system calls that need it,
each explained in [Why this is safe](https://github.com/mesa-hills-research/termelon/blob/main/docs/why-this-is-safe.md).

Terminal programs go wrong where the layers meet. A program measures a family emoji as two cells
while the terminal draws it in six, or writes true colour to a terminal that shows 256. Termelon owns
the path from the terminal's modes to the bytes of each frame, so it measures text the way the
terminal does, asks once what the terminal supports, and lowers what it writes to match. Each frame
is diffed against the last and written by a thread of its own, so drawing never waits for a slow
terminal.

## Add it

```sh
cargo add termelon
```

To test a program against an emulated terminal, add the `testing` feature as a dev-dependency:

```sh
cargo add --dev termelon --features testing
```

## Example

```rust
use std::time::Duration;

use termelon::widgets::{Block, Paragraph};
use termelon::{Drawn, Event, KeyCode, KeyEventKind, Options, Terminal};

fn main() -> Result<(), termelon::Error> {
    let mut terminal = Terminal::open(Options::default())?;
    let mut events = terminal.events();
    let mut presses = 0;
    loop {
        let drawn = terminal.draw(|frame| {
            let text = format!("Space pressed {presses} times. Press q to quit.");
            let block = Block::bordered().title(" Termelon ");
            frame.render_widget(Paragraph::new(text).block(block), frame.area());
        })?;
        // A frame isn't drawn while the terminal is still taking the last one, so look again soon.
        let wait = if matches!(drawn, Drawn::Held) {
            Duration::from_millis(10)
        } else {
            Duration::from_secs(60)
        };
        match events.next_timeout(wait)? {
            Some(Event::Key(key)) if key.kind != KeyEventKind::Release => match key.code {
                KeyCode::Char('q') => break,
                KeyCode::Char(' ') => presses += 1,
                _ => {}
            },
            _ => {}
        }
    }
    terminal.close()
}
```

[`examples/todo.rs`](https://github.com/mesa-hills-research/termelon/blob/main/crates/termelon/examples/todo.rs)
is a larger program. Run it from a checkout with `cargo run --example todo`.

## Against ratatui and crossterm

The output layer against ratatui 0.30 with crossterm 0.29, on frames recorded from a terminal coding
agent's screen. Lower is better in every row.

| Workload | Measure | Termelon | ratatui + crossterm |
|---|---|---|---|
| Idle, 200×60 (only a spinner, a shimmer and a timer change) | Diff and encode time | 5.3 µs | 79.7 µs |
| Idle, 200×60 (only a spinner, a shimmer and a timer change) | Bytes per frame | 158.5 | 220.1 |
| Idle, 200×60 (only a spinner, a shimmer and a timer change) | Allocations per frame | 0 | 2.1 |
| Streaming, 120×40 (a reply grows a word a frame) | Diff and encode time | 2.6 µs | 32.4 µs |
| Streaming, 120×40 (a reply grows a word a frame) | Bytes per frame | 632.0 | 1,361.6 |
| Streaming, 120×40 (a reply grows a word a frame) | Allocations per frame | 0 | 3.3 |
| Full redraw, 300×100 (after a resize) | Diff and encode time | 177.9 µs | 554.3 µs |
| Full redraw, 300×100 (after a resize) | Bytes per frame | 6,579.0 | 35,889.3 |
| Full redraw, 300×100 (after a resize) | Allocations per frame | 0 | 26.3 |

Times are the median frame and bytes and allocations the mean frame. They come from 99 recorded
frames of the idle workload, 299 of streaming and 6 of the full redraw, played through both stacks in
turns 10 times on one core of a Ryzen 7 9800X3D with rustc 1.97.1 in release mode (`tools/bench`, with
recordings that aren't in the repository).

The times leave out filling the buffer, which takes Termelon 1.3 to 1.5 times as long as ratatui in the
same run, and the hand-off to the writer thread. The recorded screens paint a background on every
cell, which is why a full redraw writes so little. Where the terminal doesn't erase with the current
background (tmux and GNU screen), a recorded full redraw writes about 0.96 of ratatui's bytes.
[RESULTS.md](https://github.com/mesa-hills-research/termelon/blob/main/tools/bench/RESULTS.md) has the
whole run, the machine and the command.

## Platforms and Rust version

Linux, macOS and Windows (conhost and Windows Terminal) are built and tested in CI, and FreeBSD is
checked to build. The minimum supported Rust version is 1.89, which CI tests too. A harness also runs
the library in real terminals, among them tmux, kitty, xterm, WezTerm, VTE, Alacritty, Konsole, foot
and Ghostty, and compares what each shows with the emulator.

## Docs

- [How it works](https://github.com/mesa-hills-research/termelon/blob/main/docs/how-it-works.md)
- [Capabilities](https://github.com/mesa-hills-research/termelon/blob/main/docs/capabilities.md), what's detected and the fallbacks
- [Testing](https://github.com/mesa-hills-research/termelon/blob/main/docs/testing.md), with the test terminal and goldens
- [Porting from ratatui and crossterm](https://github.com/mesa-hills-research/termelon/blob/main/docs/porting-from-ratatui.md)
- [Testing in real terminals](https://github.com/mesa-hills-research/termelon/blob/main/docs/real-terminals.md)
- [Internals](https://github.com/mesa-hills-research/termelon/blob/main/docs/internals.md), the private interfaces between the parts
- [Changelog](https://github.com/mesa-hills-research/termelon/blob/main/CHANGELOG.md)

`termelon-vt`, the emulator the test terminal uses, is a crate of its own with no dependency on
`termelon`.

## License

MIT or Apache-2.0, at your option.
