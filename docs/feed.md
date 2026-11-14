# Feeds

`FeedView` draws a long, live list, such as an agent's transcript or a log, and keeps the reader's
place in it. Items that come, go or grow off screen leave the screen as it is, a scroll of 3 rows
moves every row by 3 until the feed's start or end stops it, a new width keeps the text at the top,
and following the end keeps its last row on the bottom row. A frame measures only the items it
shows and those a scroll crosses, so a wheel step costs the same at a hundred items and at a
million.

Under it is `termelon::feed::Feed`, the scroll core, which knows nothing of terminals and works for
any renderer with whole-number heights.

## A view

The application keeps its items and gives the view a `FeedRows`, which says how many rows an item
takes at a width and draws them. The view asks only for the items it shows.

```rust
use std::ops::Range;

use termelon::feed::{Change, Measure};
use termelon::layout::Rect;
use termelon::text::{self, Line};
use termelon::widgets::{FeedRows, FeedView};
use termelon::{Buffer, Style};

/// Messages, wrapped at the view's width.
struct Messages {
    texts: Vec<String>,
    width: u16,
}

impl Messages {
    fn rows(&self, key: usize) -> Vec<String> {
        let line = Line::from(self.texts[key].as_str());
        text::wrap(&line, self.width, true)
            .iter()
            .map(|row| row.spans.iter().map(|span| span.content.as_ref()).collect())
            .collect()
    }
}

impl Measure<usize> for Messages {
    fn height(&mut self, key: &usize) -> u32 {
        self.rows(*key).len() as u32
    }
}

impl FeedRows<usize> for Messages {
    fn set_width(&mut self, width: u16) {
        self.width = width;
    }

    fn render(&mut self, key: &usize, rows: Range<u32>, area: Rect, buf: &mut Buffer) {
        let all = self.rows(*key);
        for (y, row) in (area.y..).zip(&all[rows.start as usize..rows.end as usize]) {
            buf.set_string(area.x, y, row, Style::new());
        }
    }
}

let mut messages = Messages { texts: Vec::new(), width: 80 };
let mut view = FeedView::new();

// A message comes, and a reply streams into it.
messages.texts.push("hello".into());
let reply = view.feed_mut().push(0, 1);
messages.texts[0].push_str(", and more");
view.feed_mut().changed(reply, Change::Appended);

// In the draw loop.
terminal.draw(|frame| {
    let area = frame.area();
    view.render(area, frame.buffer_mut(), &mut messages);
})?;

// On a wheel event.
view.feed_mut().scroll_by(-3);
```

`view.widget(&mut messages)` gives the same as a `Widget`, for `frame.render_widget`. A source
that keeps each item's rows until its text or the width changes draws a frame for the cost of
the items on screen. The example wraps on every call to stay short.

`view.layout(area, &mut messages)` lays out the frame the next render draws without drawing it,
for an application that acts on that frame first, such as following again where it shows the
end.

## Reporting changes

Each call that adds an item gives its `ItemId`, which the application keeps with the item. Every
later call about the item takes it, so there's no lookup by key and no table to grow.

| Call | When |
|---|---|
| `push`, `push_front`, `insert_before` | An item comes, counted at a hint of rows until it's measured |
| `remove` | An item goes |
| `changed(id, Change::Appended)` | Rows came at its end, as a reply streams |
| `changed(id, Change::Reflowed)` | Its rows changed some other way, as a fold or a new parse does |
| `set_height` | The application knows the new height without measuring, as output that grew by a line |
| `set_hint` | An estimate for an item not yet measured, for the scrollbar |
| `set_geometry` | Every height changed, after a new font or theme. The view calls it itself on a new width |

The view applies them at the next frame. A change off screen moves nothing on it, and a change on
screen moves only the rows below it, unless the reader follows the end.

A long reply that streams is best as several items, one for each paragraph or block, so a frame
wraps only the last one and a new parse above the reader changes items the reader isn't in.

An item with no rows has no place on screen, and a frame leaves it out. Each one near the screen
is still measured and walked past in every frame, so items that a filter hides are best removed
and added again, rather than measured at no rows.

## Marks

The reader's place is an item, a row of it and the screen row it's on. When the item's rows change,
or the width does, the row number may name other text. A mark names the text instead. `mark_at`
gives a number for a row's content, such as the byte of the source it starts at, and `resolve`
gives the row that content is on now. Here `row_starts` gives the byte each wrapped row starts at.

```rust
impl Measure<usize> for Messages {
    // height as above

    fn mark_at(&mut self, key: &usize, row: u32) -> Option<u64> {
        Some(self.row_starts(*key)[row as usize] as u64)
    }

    fn resolve(&mut self, key: &usize, mark: u64) -> Option<u32> {
        let starts = self.row_starts(*key);
        let row = starts.iter().rposition(|start| *start as u64 <= mark)?;
        Some(row as u32)
    }
}
```

Without marks, a new width keeps the row number at the top, which shows other text of the same
item.

## Moving the reader

| Call | Does |
|---|---|
| `scroll_by(rows)` | Scrolls, down when positive, as far as the start or the end. Scrolling down to the end follows it, with the end on the last row |
| `follow` | Goes to the end and follows it, as End does |
| `follow_in_place` | Follows the end from where the reader is, keeping the rows where they are while the end is in sight, as after a press that held the view still |
| `stay` | Stops following, with the rows where the last frame showed them, as a press that holds the view still does |
| `scroll_to_start` | Goes to the start, as Home does |
| `place(id, row, screen_row)` | Puts a row of an item on a screen row, for a fold toggled under the pointer |
| `seek(fraction, viewport)` | Goes to a share of the height, for a scrollbar's thumb. A share of 1 follows the end |
| `set_exact_limit(Some(rows))` | Scrolls further than `rows` land by the heights known so far, and say `Outcome::Approximate` |

Only the reader's own scroll to the end turns following on. A reader a few rows above the end
stays there when the end gets shorter, with blank rows below it. A scroll up from following starts
from the rows the last frame showed, also when the end grew since.

A fold that opens or closes under the pointer keeps the clicked line where it is. Find the item
and its row under the pointer with `view.item_at(position)`, change the item, report it, and
`place` that row on the same screen row.

## Following and blank rows

While following, the end sits on the last row, and the rows move with the viewport's bottom when
its height changes. When the end gets shorter, as when a status line goes or a closing marker
re-wraps a paragraph, the rows above move down to keep it there.

`feed.set_gap_limit(rows)` keeps them still instead. Up to `rows` blank rows show below the end
until content fills them, and past that the end comes back down. The default, 0, keeps the end on
the last row. Going to the end with `follow` and a new width close the blank rows, and
`follow_in_place` keeps them.

## The scrollbar and counts

A height counts as a hint until the item is measured at the current width. `extent()` gives the
total and whether it's exact, and `offset(viewport)` gives the top's distance from the start, for a
scrollbar. `extent_after(id)` gives the rows after an item, for a count of new lines below the
screen.

`sweep(max, &mut rows)` measures up to `max` items that are still hints, nearest the reader first.
Run it in idle time, a slice at a time, to make the extent exact. It never moves the screen.

## Selections

A selection's end is a `Point`, an item, a mark and a column, rather than a screen row, so a
selection survives a new width. `view.point_at(position, &mut rows)` gives the point under the
pointer, and `feed.cmp_points(a, b)` orders two. The application copies from its own text, from
one point to the other, so items never laid out copy too.

## Scroll regions

When its rows moved since the last frame, the view notes it with `Buffer::note_scroll`. Where
`Capabilities::scroll_regions` is `Yes`, the encoder scrolls the rows in the terminal and writes
only those that came in, so a wheel step at 120 by 40 writes about 400 bytes instead of about
3,300. Terminals scroll whole rows, so cells beside the view in those rows are written again where
they differ, and the encoder takes the scroll only when it saves bytes.

The capability table says `Yes` for the terminals whose runs in the terminal matrix showed scrolled
regions right ([Testing in real terminals](real-terminals.md)). `TERMELON_CAPS=scroll_regions=yes`
turns it on for another terminal.

## Trying it

`cargo run --release --example feed_demo` plays a coding agent's transcript that never stops,
with replies streaming, "Working…" coming and going and folds opening and closing. Scroll it
while it streams, and press 1, 2 and 3 to compare a place kept as a share of the top block with
`FeedView`, with and without blank rows below the end. The status line counts the frames where
text moved under the reader, and the rows that moved flash.

## Costs

On 6 cores of an i9-13980HX, release build, a 120 by 50 view of items of 1 to 550 words.

| | 100,000 items | 1,000,000 items |
|---|---|---|
| The core's wheel step of 3 rows, p50 / p99 | 0.25 / 0.45 µs | 0.27 / 0.46 µs |
| The core's frame after a new width, p50 / p99 | 3.3 / 8.6 µs | 33 / 93 µs |
| The view's wheel step, wrapping and drawing, p50 / p99 | 47 / 201 µs | 53 / 240 µs |
| The view's frame after a new width, p50 / p99 | 278 / 355 µs | 283 / 510 µs |
| A push, on average and at most | 58 ns, 8 µs | 57 ns, 61 µs |
| A `push_front`, on average | 0.36 µs | 5.1 µs |
| An `insert_before` the last item, on average | 0.37 µs | 4.8 µs |

The core's figures measure an item by a table lookup. The view's include wrapping the items that
come into view with `text::wrap` and writing their cells, and the frame after a new width wraps the
15 to 17 items on screen.

A push costs the same at any length. An item added anywhere else now and then splits a block of
the index and rebuilds its sums, so its cost grows with the feed, as the last two rows show.

```sh
cargo run --release -p bench --bin feed
```
