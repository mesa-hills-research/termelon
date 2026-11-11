# Changelog

## 0.1.0

The first release, with two crates.

- `termelon` sets up the terminal on Linux, macOS and Windows, decodes input (the kitty keyboard
  protocol, mouse, paste and focus), draws text and the `Block`, `Clear` and `Paragraph` widgets into
  a cell buffer, and writes diffed frames as synchronized updates from a thread of its own. It finds
  out what the terminal supports and falls back where a feature is missing.
- The `testing` feature adds `TestTerminal` and golden files, and the `stream` feature adds `Stream`
  for `Events`.
- `termelon-vt` is a terminal emulator library for tests. It has profiles of common terminals, and
  fails on any sequence it doesn't know.
