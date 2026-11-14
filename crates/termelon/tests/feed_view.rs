//! `FeedView` through the test terminal. Each test reads the emulator's screen, or the bytes the
//! terminal got, rather than the view's own state.

#![cfg(feature = "testing")]

use std::collections::HashMap;
use std::ops::Range;

use termelon::buffer::Buffer;
use termelon::caps::Support;
use termelon::feed::{Change, ItemId, Measure, Outcome, Point};
use termelon::layout::{Position, Rect};
use termelon::style::Style;
use termelon::testing::TestTerminal;
use termelon::widgets::{FeedRows, FeedView};

struct Rng(u64);

#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }

    fn index(&mut self, len: usize) -> usize {
        usize::try_from(self.below(len as u64)).unwrap()
    }
}

/// Words that name their item and their place in it, so a screen row says what it shows.
fn words(key: u64, from: u64, count: u64) -> String {
    (from..from + count)
        .map(|n| format!("i{key}w{n}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Wraps source lines at `width`, each row with the byte of the item's text it starts at. The
/// text is the lines joined with newlines, and an empty line is a blank row.
fn wrap(lines: &[String], width: usize) -> Vec<(usize, String)> {
    let mut rows = Vec::new();
    let mut base = 0;
    for line in lines {
        let mut row = String::new();
        let mut start = base;
        let mut at = base;
        for word in line.split(' ') {
            if !row.is_empty() && row.len() + 1 + word.len() > width {
                rows.push((start, std::mem::take(&mut row)));
            }
            if row.is_empty() {
                start = at;
            } else {
                row.push(' ');
            }
            row.push_str(word);
            at += word.len() + 1;
        }
        rows.push((start, row));
        base += line.len() + 1;
    }
    rows
}

/// The feed's text, kept by key as source lines and wrapped at the view's width when it's
/// measured or drawn.
#[derive(Default)]
struct Doc {
    items: HashMap<u64, Vec<String>>,
    width: usize,
    wrapped: HashMap<u64, Vec<(usize, String)>>,
    /// How many items were wrapped.
    wraps: u64,
}

#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
impl Doc {
    fn rows(&mut self, key: u64) -> &[(usize, String)] {
        if !self.wrapped.contains_key(&key) {
            self.wraps += 1;
            let rows = wrap(&self.items[&key], self.width);
            self.wrapped.insert(key, rows);
        }
        &self.wrapped[&key]
    }

    fn edit(&mut self, key: u64) -> &mut Vec<String> {
        self.wrapped.remove(&key);
        self.items.get_mut(&key).unwrap()
    }

    fn text(&self, key: u64) -> String {
        self.items[&key].join("\n")
    }
}

#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
impl Measure<u64> for Doc {
    fn height(&mut self, key: &u64) -> u32 {
        u32::try_from(self.rows(*key).len()).unwrap()
    }

    /// The byte of the text the row starts at.
    fn mark_at(&mut self, key: &u64, row: u32) -> Option<u64> {
        let rows = self.rows(*key);
        rows.get(usize::try_from(row).unwrap())
            .map(|(start, _)| *start as u64)
    }

    /// The row that byte is on now.
    fn resolve(&mut self, key: &u64, mark: u64) -> Option<u32> {
        let rows = self.rows(*key);
        let row = rows.iter().rposition(|(start, _)| *start as u64 <= mark)?;
        u32::try_from(row).ok()
    }
}

impl FeedRows<u64> for Doc {
    fn set_width(&mut self, width: u16) {
        self.width = usize::from(width);
        self.wrapped.clear();
    }

    fn render(&mut self, key: &u64, rows: Range<u32>, area: Rect, buf: &mut Buffer) {
        let shown = &self.rows(*key)[rows.start as usize..rows.end as usize];
        for (y, (_, text)) in (area.y..).zip(shown) {
            buf.set_string(area.x, y, text, Style::new());
        }
    }
}

/// A terminal with a feed view, its text, and the order of the keys.
struct Session {
    term: TestTerminal,
    view: FeedView<u64>,
    doc: Doc,
    ids: HashMap<u64, ItemId>,
    order: Vec<u64>,
    next_key: u64,
}

#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
impl Session {
    fn new(width: u16, height: u16) -> Self {
        Self {
            term: TestTerminal::new(width, height),
            view: FeedView::new(),
            doc: Doc::default(),
            ids: HashMap::new(),
            order: Vec::new(),
            next_key: 0,
        }
    }

    /// A session of `n` items of a few words to a few hundred, following the end.
    fn with_items(width: u16, height: u16, n: u64, rng: &mut Rng) -> Self {
        let mut session = Self::new(width, height);
        for _ in 0..n {
            let count = match rng.below(10) {
                0 => 150 + rng.below(400),
                1..=3 => 20 + rng.below(60),
                _ => 1 + rng.below(12),
            };
            let key = session.next_key;
            session.push(vec![words(key, 0, count)]);
        }
        session.draw();
        session
    }

    fn key(&mut self) -> u64 {
        let key = self.next_key;
        self.next_key += 1;
        key
    }

    fn push(&mut self, lines: Vec<String>) -> u64 {
        let key = self.key();
        self.doc.items.insert(key, lines);
        self.ids.insert(key, self.view.feed_mut().push(key, 2));
        self.order.push(key);
        key
    }

    /// Adds an item before the one at `at` in the order.
    fn insert(&mut self, at: usize, lines: Vec<String>) -> u64 {
        let key = self.key();
        self.doc.items.insert(key, lines);
        let before = self.ids[&self.order[at]];
        let id = self.view.feed_mut().insert_before(before, key, 0);
        self.ids.insert(key, id);
        self.order.insert(at, key);
        key
    }

    fn remove(&mut self, key: u64) {
        let id = self.ids.remove(&key).unwrap();
        assert_eq!(self.view.feed_mut().remove(id), Some(key));
        self.order.retain(|k| *k != key);
        self.doc.items.remove(&key);
        self.doc.wrapped.remove(&key);
    }

    /// Changes an item's lines and reports how.
    fn change(&mut self, key: u64, change: Change, edit: impl FnOnce(&mut Vec<String>)) {
        edit(self.doc.edit(key));
        self.view.feed_mut().changed(self.ids[&key], change);
    }

    fn draw(&mut self) {
        self.term
            .draw(|frame| {
                let area = frame.area();
                self.view.render(area, frame.buffer_mut(), &mut self.doc);
            })
            .unwrap();
    }

    fn rows(&self) -> Vec<String> {
        self.term.screen().rows()
    }

    /// The feed's last row, at the view's width.
    fn last_row(&self) -> String {
        let last = self.order.last().unwrap();
        let rows = wrap(&self.doc.items[last], self.doc.width);
        rows.last().unwrap().1.clone()
    }

    fn scroll(&mut self, rows: i64) {
        self.view.feed_mut().scroll_by(rows);
        self.draw();
    }

    /// The order's place of the first and last items on screen.
    fn on_screen(&self) -> Range<usize> {
        let items = &self.view.placement().items;
        let place = |item: ItemId| {
            let key = *self.view.feed().key(item).unwrap();
            self.order.iter().position(|k| *k == key).unwrap()
        };
        place(items[0].item)..place(items[items.len() - 1].item) + 1
    }

    /// The text from one end of a selection to the other, from the items' text alone.
    fn copy(&self, from: &Point, to: &Point) -> String {
        let feed = self.view.feed();
        let byte =
            |point: &Point| usize::try_from(point.mark.unwrap()).unwrap() + point.column as usize;
        let mut item = from.item;
        let mut out = String::new();
        loop {
            let text = self.doc.text(*feed.key(item).unwrap());
            let start = if item == from.item { byte(from) } else { 0 };
            if item == to.item {
                out.push_str(&text[start..byte(to)]);
                return out;
            }
            out.push_str(&text[start..]);
            out.push('\n');
            item = feed.next(item).unwrap();
        }
    }
}

/// Nothing that happens off screen changes the screen or writes a byte: replies streaming at
/// the end, items coming above and below, items above growing, and idle measuring.
#[test]
fn nothing_off_screen_moves_the_screen() {
    for seed in 1..=3 {
        let mut rng = Rng::new(seed);
        let mut s = Session::with_items(80, 24, 5_000, &mut rng);
        for _ in 0..40 {
            s.scroll(-3);
        }
        let screen = s.term.screen().text();
        s.term.take_output();
        for step in 0..400 {
            let shown = s.on_screen();
            let last = *s.order.last().unwrap();
            match rng.below(8) {
                // The reply at the end streams.
                0 | 1 => s.change(last, Change::Appended, |lines| {
                    let line = lines.last_mut().unwrap();
                    line.push_str(" more");
                }),
                2 => {
                    let key = s.next_key;
                    s.push(vec![words(key, 0, 1 + rng.below(80))]);
                }
                // One comes above the reader, as a result under its call does.
                3 => {
                    let at = rng.index(shown.start);
                    let key = s.next_key;
                    s.insert(at, vec![words(key, 0, 1 + rng.below(80))]);
                }
                // One comes below the screen.
                4 if shown.end < s.order.len() => {
                    let at = shown.end + rng.index(s.order.len() - shown.end);
                    let key = s.next_key;
                    s.insert(at, vec![words(key, 0, 1 + rng.below(80))]);
                }
                // One above grows, as a job's output does, or is parsed again.
                5 => {
                    let key = s.order[rng.index(shown.start)];
                    let change = if rng.below(2) == 0 {
                        Change::Appended
                    } else {
                        Change::Reflowed
                    };
                    let more = words(key, 1000 + rng.below(1000), 1 + rng.below(200));
                    s.change(key, change, |lines| lines.push(more));
                }
                // One above goes.
                6 => {
                    let key = s.order[rng.index(shown.start)];
                    s.remove(key);
                }
                // Idle time measures items the reader hasn't seen.
                _ => {
                    s.view.feed_mut().sweep(50, &mut s.doc);
                }
            }
            s.draw();
            assert_eq!(s.term.screen().text(), screen, "seed {seed}, step {step}");
            assert_eq!(s.term.take_output(), b"", "seed {seed}, step {step}");
            assert_eq!(s.view.placement().outcome, Outcome::Kept);
        }
    }
}

/// A change to an item on screen leaves every row above it as it was.
#[test]
fn a_change_on_screen_moves_only_what_is_below_it() {
    let mut rng = Rng::new(7);
    let mut s = Session::with_items(70, 30, 3_000, &mut rng);
    s.scroll(-2_000);
    for round in 0..200 {
        let before = s.rows();
        let items = s.view.placement().items.clone();
        let placed = items[rng.index(items.len())];
        let key = *s.view.feed().key(placed.item).unwrap();
        let bottom = placed.y + i64::from(placed.height);
        // Rows come at its end, so its last row may change, and the rows above it don't.
        let same = if round % 2 == 0 || placed.y <= 0 {
            let more = words(key, 5000 + round, 1 + rng.below(40));
            s.change(key, Change::Appended, |lines| {
                let line = lines.last_mut().unwrap();
                line.push(' ');
                line.push_str(&more);
            });
            bottom - 1
        } else {
            // A line opens at its start, as a fold does.
            let more = words(key, 7000 + round, 1 + rng.below(90));
            s.change(key, Change::Reflowed, |lines| lines.insert(0, more));
            placed.y
        };
        s.draw();
        let after = s.rows();
        let same = usize::try_from(same.clamp(0, 30)).unwrap();
        assert_eq!(after[..same], before[..same], "round {round}");
    }
}

/// A scroll of `d` rows moves every row by `d`, for `d` up to a viewport, before and after a
/// resize.
#[test]
fn a_scroll_of_d_moves_the_content_by_d() {
    let mut rng = Rng::new(3);
    let height = 30;
    let mut s = Session::with_items(70, height, 3_000, &mut rng);
    s.scroll(-5_000);
    for width in [70, 47] {
        s.term.resize(width, height);
        s.draw();
        for d in [
            1i64, 2, 3, 5, 10, 29, 30, -1, -3, -10, -29, -30, 3, -3, 7, -7,
        ] {
            let before = s.rows();
            s.scroll(d);
            let after = s.rows();
            assert!(!s.view.feed().is_following());
            for (i, row) in after.iter().enumerate() {
                let Some(was) = usize::try_from(i64::try_from(i).unwrap() + d)
                    .ok()
                    .and_then(|from| before.get(from))
                else {
                    continue;
                };
                assert_eq!(row, was, "width {width}, scroll {d}, row {i}");
            }
        }
    }
}

/// A wheel step at 120 by 40 writes the rows that came in, after scrolling the rest in the
/// terminal, and comes to one write of the writer's 4,096 bytes. A terminal without scroll
/// regions gets every row.
#[test]
fn a_wheel_step_writes_the_rows_that_came_in() {
    let step = |scroll_regions| {
        let mut rng = Rng::new(5);
        let mut s = Session::with_items(120, 40, 2_000, &mut rng);
        let mut caps = s.term.capabilities().clone();
        caps.scroll_regions = scroll_regions;
        s.term = TestTerminal::new(120, 40).with_caps(caps);
        s.draw();
        s.scroll(-1_000);
        let mut sizes = Vec::new();
        for _ in 0..50 {
            s.term.take_output();
            s.scroll(-3);
            sizes.push(s.term.take_output());
        }
        sizes
    };
    let with = step(Support::Yes);
    for bytes in &with {
        let text = String::from_utf8_lossy(bytes);
        assert!(text.contains("\x1b[1;40r"), "{text:?}");
        assert!(bytes.len() < 4_096, "{}", bytes.len());
    }
    let without = step(Support::No);
    let total = |sizes: &[Vec<u8>]| sizes.iter().map(Vec::len).sum::<usize>();
    assert!(total(&with) * 4 < total(&without));
    assert!(
        without
            .iter()
            .all(|b| !String::from_utf8_lossy(b).contains("\x1b[1;40r"))
    );
}

/// Following a feed that only grows, the last screen row holds the feed's last line.
#[test]
fn following_keeps_the_last_line_on_the_last_row() {
    let mut rng = Rng::new(11);
    let mut s = Session::with_items(60, 20, 300, &mut rng);
    for step in 0..500 {
        let last = *s.order.last().unwrap();
        if rng.below(4) == 0 {
            let key = s.next_key;
            s.push(vec![words(key, 0, 1 + rng.below(30))]);
        } else {
            let added = words(last, 10_000 + step, 1 + rng.below(6));
            s.change(last, Change::Appended, |lines| {
                let line = lines.last_mut().unwrap();
                line.push(' ');
                line.push_str(&added);
            });
        }
        s.draw();
        assert_eq!(s.rows()[19], s.last_row(), "step {step}");
        assert!(s.view.feed().is_following());
    }
}

/// A turn of a coding agent, as a feed sees it: a status line of three rows that comes and goes,
/// and replies that stream and now and then get shorter as their markup closes.
#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
fn agent_turns(
    s: &mut Session,
    rng: &mut Rng,
    steps: usize,
    mut check: impl FnMut(&Session, usize),
) {
    let mut working: Option<u64> = None;
    let mut reply: Option<u64> = None;
    let mut spins = 0;
    for step in 0..steps {
        match (working, rng.below(10)) {
            // The status line goes as the reply starts, or as a tool's call shows.
            (Some(key), 0..=2) => {
                s.remove(key);
                working = None;
                let key = s.next_key;
                reply = Some(s.push(vec![words(key, 0, 1 + rng.below(8))]));
            }
            (None, 0) => {
                spins += 1;
                working = Some(s.push(vec![
                    String::new(),
                    format!("⠋ Working… {spins}"),
                    String::new(),
                ]));
                reply = None;
            }
            (None, 1..=6) => {
                if let Some(key) = reply {
                    let n = s.doc.text(key).split(' ').count() as u64;
                    let more = words(key, n, 1 + rng.below(9));
                    s.change(key, Change::Appended, |lines| {
                        let line = lines.last_mut().unwrap();
                        line.push(' ');
                        line.push_str(&more);
                    });
                }
            }
            // A marker closes, and the reply wraps shorter.
            (None, 7) => {
                if let Some(key) = reply {
                    s.change(key, Change::Reflowed, |lines| {
                        let line = lines.last_mut().unwrap();
                        let cut = line.rfind(' ').unwrap_or(0);
                        if cut > 0 {
                            line.truncate(cut);
                        }
                    });
                }
            }
            _ => {}
        }
        s.draw();
        check(s, step);
    }
}

/// Each row of `after` that shows the same text as a row of `before` is at or above that row.
fn assert_nothing_moved_down(before: &[String], after: &[String], at: &str) {
    let unique = |rows: &[String], text: &String| rows.iter().filter(|r| *r == text).count() == 1;
    for (i, row) in after.iter().enumerate() {
        if row.is_empty() || !unique(after, row) || !unique(before, row) {
            continue;
        }
        let was = before.iter().position(|r| r == row).unwrap_or(i);
        assert!(i <= was, "{at}: {row:?} went from row {was} to {i}");
    }
}

/// With blank rows allowed below the end, following never moves a row down, with a status line
/// coming and going and replies getting shorter.
#[test]
fn following_with_blank_rows_never_moves_a_row_down() {
    let mut rng = Rng::new(13);
    let mut s = Session::with_items(60, 20, 200, &mut rng);
    s.view.feed_mut().set_gap_limit(u32::MAX);
    let mut before = s.rows();
    let mut gaps = 0;
    agent_turns(&mut s, &mut rng, 3_000, |s, step| {
        let after = s.rows();
        assert!(s.view.feed().is_following());
        assert_nothing_moved_down(&before, &after, &format!("step {step}"));
        gaps += usize::from(s.view.placement().gap > 0);
        before = after;
    });
    assert!(gaps > 0, "the end never got shorter");
}

/// Without a gap limit, the default, the end stays on the last row.
#[test]
fn following_without_a_gap_limit_keeps_the_end_on_the_last_row() {
    let mut rng = Rng::new(13);
    let mut s = Session::with_items(60, 20, 200, &mut rng);
    agent_turns(&mut s, &mut rng, 3_000, |s, step| {
        assert_eq!(s.view.placement().gap, 0, "step {step}");
        assert_eq!(s.rows()[19], s.last_row(), "step {step}");
    });
}

/// A reader a few rows above the end isn't pulled into following when the end gets shorter,
/// and keeps their rows.
#[test]
fn a_reader_above_the_end_stays_when_the_end_gets_shorter() {
    let mut rng = Rng::new(17);
    let mut s = Session::with_items(60, 20, 200, &mut rng);
    s.push(vec![String::new(), "⠋ Working…".to_owned(), String::new()]);
    let key = s.next_key;
    s.push(vec![words(key, 0, 60)]);
    s.draw();
    s.scroll(-2);
    assert!(!s.view.feed().is_following());
    let before = s.rows();
    // The reply is parsed again, shorter, and the status line goes.
    s.change(key, Change::Reflowed, |lines| lines[0] = words(key, 0, 20));
    let working = s.order[s.order.len() - 2];
    s.remove(working);
    s.draw();
    assert!(!s.view.feed().is_following());
    let after = s.rows();
    // Blank rows show below the end.
    assert!(s.view.placement().gap > 0);
    assert_eq!(after[19], "");
    // The rows above the status line are where they were.
    let status = before.iter().position(|r| r.contains("Working")).unwrap();
    assert_eq!(after[..status - 1], before[..status - 1]);
    // Scrolling down to the end follows it again.
    s.scroll(100);
    assert!(s.view.feed().is_following());
}

/// A fold toggled under the pointer keeps the clicked line on its row, opening and closing, with
/// its item's top above the screen and an item above it changing too.
#[test]
fn a_fold_keeps_the_clicked_line_on_its_row() {
    let mut rng = Rng::new(19);
    let mut s = Session::with_items(70, 24, 400, &mut rng);
    let above = s.order[s.order.len() - 10];
    let key = s.next_key;
    let intro = words(key, 0, 120);
    s.push(vec![
        intro,
        format!("▸ fold of i{key}"),
        words(key, 500, 30),
    ]);
    for _ in 0..20 {
        let key = s.next_key;
        s.push(vec![words(key, 0, 30)]);
    }
    s.draw();
    // Scroll until the fold's line is on row 6 and its intro starts above the screen.
    loop {
        let rows = s.rows();
        if let Some(row) = rows
            .iter()
            .position(|r| r.contains(&format!("fold of i{key}")))
            && row >= 6
        {
            s.scroll(i64::try_from(row).unwrap() - 6);
            break;
        }
        s.scroll(-1);
    }
    let rows = s.rows();
    let clicked_row = rows
        .iter()
        .position(|r| r.contains(&format!("fold of i{key}")))
        .unwrap();
    assert_eq!(clicked_row, 6);
    let first = s.view.placement().items[0];
    assert_eq!(*s.view.feed().key(first.item).unwrap(), key);
    assert!(first.y < 0, "the fold's item starts above the screen");
    for open in [true, false, true] {
        let (item, row) = s.view.item_at(Position::new(3, 6)).unwrap();
        s.change(key, Change::Reflowed, |lines| {
            if open {
                lines[1] = format!("▾ fold of i{key}");
                let body: Vec<String> = (0..12).map(|n| words(key, 900 + n * 10, 10)).collect();
                lines.splice(2..2, body);
            } else {
                lines[1] = format!("▸ fold of i{key}");
                lines.drain(2..14);
            }
        });
        // An item above the screen opens at the same time.
        s.change(above, Change::Reflowed, |lines| {
            lines.push(words(above, 3000, 50));
        });
        s.view.feed_mut().place(item, row, 6);
        s.draw();
        let rows = s.rows();
        assert!(rows[6].contains(&format!("fold of i{key}")), "{rows:#?}");
    }
}

/// Another width keeps the first word of the screen on the first row, through seven widths, and
/// wraps only the items on screen.
#[test]
fn another_width_keeps_the_first_word_on_the_first_row() {
    let mut rng = Rng::new(0x5EED);
    let height = 30;
    let mut s = Session::with_items(100, height, 5_000, &mut rng);
    s.scroll(-700);
    for width in [72u16, 131, 40, 100, 57, 200, 100] {
        let first = s.rows()[0].split(' ').next().unwrap().to_owned();
        s.term.resize(width, height);
        let wraps = s.doc.wraps;
        s.draw();
        let row = s.rows()[0].clone();
        assert!(
            row.split(' ').any(|word| word == first),
            "{first} isn't on the first row at {width} columns: {row}"
        );
        assert!(s.doc.wraps - wraps <= u64::from(height) + 1);
    }
}

/// A selection made before a resize copies the same text after it.
#[test]
fn a_selection_copies_the_same_text_after_a_resize() {
    let mut rng = Rng::new(23);
    let mut s = Session::with_items(80, 24, 1_000, &mut rng);
    s.scroll(-900);
    let rows = s.rows();
    // From a long row near the top to one further down, inside their text.
    let long = |from: usize| (from..24).find(|y| rows[*y].len() > 30).unwrap();
    let (top, bottom) = (long(2), long(14));
    let at = |y: usize, x| Position::new(x, u16::try_from(y).unwrap());
    let from = s.view.point_at(at(top, 5), &mut s.doc).unwrap();
    let to = s.view.point_at(at(bottom, 20), &mut s.doc).unwrap();
    assert_eq!(
        s.view.feed().cmp_points(&from, &to),
        Some(std::cmp::Ordering::Less)
    );
    let copied = s.copy(&from, &to);
    // It starts and ends with what the screen showed there.
    assert!(copied.starts_with(&rows[top][5..]), "{copied:?}");
    assert!(copied.ends_with(&rows[bottom][..20]), "{copied:?}");
    for width in [50, 113, 80] {
        s.term.resize(width, 24);
        s.draw();
        assert_eq!(s.copy(&from, &to), copied, "at {width} columns");
    }
}

/// The reader's item removed leaves them at its neighbour, and the frame says so.
#[test]
fn the_readers_item_removed_leaves_them_at_its_neighbour() {
    let mut rng = Rng::new(29);
    let mut s = Session::with_items(80, 24, 1_000, &mut rng);
    s.scroll(-600);
    let top = s.view.placement().items[0];
    let key = *s.view.feed().key(top.item).unwrap();
    let next = s.order[s.order.iter().position(|k| *k == key).unwrap() + 1];
    s.remove(key);
    s.draw();
    assert_eq!(s.view.placement().outcome, Outcome::Replaced);
    assert!(
        s.rows()[0].starts_with(&format!("i{next}w0")),
        "{:?}",
        s.rows()[0]
    );
    s.draw();
    assert_eq!(s.view.placement().outcome, Outcome::Kept);
}

/// Heights measured before the view's first frame, at a width it never draws at, count as hints,
/// so scrolling up through them shows their rows and no blank ones.
#[test]
fn heights_measured_before_the_first_frame_are_measured_again() {
    let build = |sweep_first: bool| {
        let mut rng = Rng::new(37);
        let mut s = Session::new(160, 40);
        for _ in 0..600 {
            let key = s.next_key;
            s.push(vec![words(key, 0, 1 + rng.below(120))]);
        }
        if sweep_first {
            // Idle time ran before the first frame, with the text set for 80 columns.
            s.doc.width = 80;
            s.view.feed_mut().sweep(600, &mut s.doc);
        }
        s.draw();
        let mut screens = vec![s.term.screen().text()];
        for _ in 0..8 {
            s.scroll(-30);
            screens.push(s.term.screen().text());
        }
        screens
    };
    assert_eq!(build(true), build(false));
}

/// The view's rows are what the emulator shows, and a row maps back to its item.
#[test]
fn a_screen_row_maps_to_its_item_and_row() {
    let mut rng = Rng::new(31);
    let mut s = Session::with_items(40, 12, 100, &mut rng);
    s.scroll(-37);
    let rows = s.rows();
    for (y, shown) in (0u16..).zip(&rows) {
        let (item, row) = s.view.item_at(Position::new(0, y)).unwrap();
        let key = *s.view.feed().key(item).unwrap();
        let text = &s.doc.rows(key)[row as usize].1;
        assert_eq!(shown, text);
    }
    assert_eq!(s.view.item_at(Position::new(0, 12)), None);
    assert_eq!(s.view.item_at(Position::new(40, 0)), None);
}

/// A view prints in a test's failure, also when its keys don't.
#[test]
fn a_view_prints_whatever_its_keys_are() {
    struct Key;
    let mut view = FeedView::new();
    view.feed_mut().push(Key, 1);
    assert!(format!("{view:?}").starts_with("FeedView"));
}
