# termelon-vt

A terminal emulator library for Rust, for testing what terminal programs write.

It keeps the screen a terminal would show for the bytes a program writes, answers queries as a chosen
terminal would, and encodes keys the way that terminal sends them. A sequence it doesn't know is an
error, so a test fails on anything nobody tested. Its only dependencies are `unicode-width` and
`unicode-segmentation`.

```sh
cargo add --dev termelon-vt
```

```rust
use termelon_vt::{Emulator, Event, Profile};

let mut term = Emulator::with_profile(Profile::kitty(), 40, 10);
term.feed(b"\x1b[?25l\x1b[3;1H\x1b[38;2;216;222;233mready\x1b[0m\x1b[?25h").unwrap();
assert_eq!(term.row_text(2).as_deref(), Some("ready"));

// No character was drawn while the cursor showed.
assert!(!term.events().iter().any(|e| matches!(e, Event::Drawn { cursor_visible: true, .. })));

// Queries are answered as kitty answers them.
term.feed(b"\x1b[?u\x1b[c").unwrap();
assert_eq!(term.take_output(), b"\x1b[?0u\x1b[?62;c");
```

The [documentation](https://docs.rs/termelon-vt) covers what it models, the profiles of xterm, kitty,
Konsole, Windows Terminal, tmux and the Linux console, and the styled-text format that makes screens
readable in a diff. [Termelon](https://github.com/mesa-hills-research/termelon), a terminal UI
library, uses it for its test terminal and its own tests.

## License

MIT or Apache-2.0, at your option.
