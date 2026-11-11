# tuiframe

A terminal UI framework in pure Rust. It covers the whole stack a full-screen terminal program needs:

- terminal setup and raw mode on Linux, macOS and Windows (conhost and Windows Terminal)
- input, with the kitty keyboard protocol, Windows key records, SGR mouse, bracketed paste and focus
- a 16-byte cell buffer with grapheme clusters, wide characters, true colour, underline styles and
  hyperlinks
- diffed output, written as one synchronized update per frame by a thread of its own
- capability detection in one round trip, with fallbacks for terminals that lack a feature
- text, a small layout, and the basic widgets
- a test terminal backed by a terminal emulator, for goldens of what a terminal really shows

It has no C dependencies, and its unsafe code is limited to the operating system calls that need it,
each explained in [docs/why-this-is-safe.md](docs/why-this-is-safe.md).

The project is in its first milestones and isn't published yet. `cargo run --example todo` shows a
small program.

## Docs

- [How it works](docs/how-it-works.md)
- [Capabilities](docs/capabilities.md), what's detected and the fallbacks
- [Testing](docs/testing.md), with the test terminal and goldens
- [Porting from ratatui and crossterm](docs/porting-from-ratatui.md)
- [Testing in real terminals](docs/real-terminals.md)

## License

MIT or Apache-2.0, at your option.
