# Changelog

## Unreleased

- A scroll down with blank rows below the end, as after a fold closed near the end, moved every
  row down by those rows as the reader went back to following. A scroll down now stops where the
  end reaches the last row and moves nothing where the end is above it, and the reader follows
  from there.
- `Feed::follow_in_place` brought the end down to the last row when more blank rows showed below
  it than the gap limit. Now the rows stay where they are and the reader doesn't follow. A scroll
  down follows once content has filled the blank rows to within the limit. `is_following` says
  no from the call until the next layout decides.
- A place and a scroll down in one frame scroll on from where the feed's start puts the place,
  and reach the end as before.
- Following, when the top item on screen and every item after it went in one frame, rows the
  reader never saw showed above blank rows. The end now comes down to the last row, as it does
  when they go from the end up.

## 0.2.0 (2026-11-14)

This release is `termelon` 0.2.0. `termelon-vt` hasn't changed and stays at 0.1.0.

- `FeedView` draws a long, live list, such as a transcript, and keeps the reader's place in it.
  Nothing off screen moves the screen, a scroll of n rows moves every row by n until the start or
  the end stops it, a new width keeps the text at the top, and a frame measures only the items it
  shows and those a scroll crosses. `feed::Feed` is the scroll core under it, for other renderers.
  [docs/feed.md](docs/feed.md) says how to use them.
- While the reader follows the end, `Feed::set_gap_limit` keeps the rows still when the end gets
  shorter, with up to that many blank rows below it. `Feed::follow_in_place` follows again without
  closing them, and `Feed::stay` stops following where the last frame left the rows.
- `FeedView::layout` lays out the next frame without drawing it.
- `Buffer::note_scroll` says that a widget's rows moved. Where the new
  `Capabilities::scroll_regions` is `Yes`, the encoder scrolls them in the terminal and writes
  only the rows that came in, when that saves bytes.
- The probe's `scroll` scene checks scrolled regions in each terminal of the matrix.

## 0.1.0 (2026-11-12)

The first release, with two crates.

- `termelon` sets up the terminal on Linux, macOS and Windows, decodes input (the kitty keyboard
  protocol, mouse, paste and focus), draws text and the `Block`, `Clear` and `Paragraph` widgets into
  a cell buffer, and writes diffed frames as synchronized updates from a thread of its own. It finds
  out what the terminal supports and falls back where a feature is missing.
- The `testing` feature adds `TestTerminal` and golden files, and the `stream` feature adds `Stream`
  for `Events`.
- `termelon-vt` is a terminal emulator library for tests. It has profiles of common terminals, and
  fails on any sequence it doesn't know. It draws screens as SVG, still or animated, for docs.
