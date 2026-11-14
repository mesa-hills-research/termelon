# Changelog

## Unreleased

- A scroll down no longer moves rows down. It stops where the end reaches the last row, and moves
  nothing where the end is above it already, as after a fold closed near the end. Before,
  reaching the end moved every row down by the blank rows below it.
- `Feed::follow_in_place` keeps the rows where they are, with any blank rows below the end. Where
  those are more than the gap limit, the reader stays above the end rather than having it brought
  down, and a scroll down follows again once content has filled them to within the limit.
  `Feed::is_following` says no until the next layout decides. With
  `Feed::set_gap_limit(u32::MAX)`, following moves no line down, except the lines below output
  that grows on screen (into the blank rows), to give back what a shorter viewport took, after a
  new width, or where no row would show.
- A scroll up from following, and `Feed::scroll_to_start`, stop following exactly where they move
  a line, also with the end still in sight. One that moves nothing, as in a feed that shows from
  its start, leaves the reader following.
- A viewport that gets shorter while following takes the blank rows first, then moves the rows up.
  One that gets taller moves them back down by what shorter ones took since the reader last chose
  to follow, and the rest of its height is blank rows, as far as the gap limit allows. A menu that
  opens and closes under the feed leaves every row where it was.
- Following, when the top item on screen and every item after it went in one frame, rows the
  reader never saw showed above blank rows. The end now comes down to the last row.
- `Placement::scrolled` says how far the frame's scroll moved the lines. `Feed::place_settled`
  places the reader as `place` does, and brings the end down where it would stand above the last
  row.

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
