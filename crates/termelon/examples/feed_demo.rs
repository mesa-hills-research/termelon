//! A coding agent's transcript that never stops. Prompts come, "Working…" comes and goes, tools
//! run and their output grows, replies stream word by word, and folds open and close. Scroll it
//! while it streams, and switch between three ways of keeping the reader's place.
//!
//! ```sh
//! cargo run --release --example feed_demo
//! cargo run --release --example feed_demo -- --seed 7
//! ```
//!
//! | Key | Does |
//! |---|---|
//! | 1 | The place is the block at the top and a share of its height, rescaled when the block changes. Following pins the end to the last row, and reaching the bottom by any means follows again. This is the rule Arko 0.20's transcript has |
//! | 2 | `FeedView`, following with the end on the last row |
//! | 3 | `FeedView` with `set_gap_limit(3)`, so a shorter end leaves blank rows below it |
//! | The wheel, ↑ and ↓, Page Up and Page Down | Scroll |
//! | Home, End | Go to the start, or to the end and follow it |
//! | q, Ctrl+C | Quit |
//!
//! The session is the same in every mode, and switching keeps the row at the top. Mode 1 writes
//! every row that changed, as Arko does today, and modes 2 and 3 scroll a region of rows where the
//! terminal can.

#![cfg_attr(
    not(any(unix, windows)),
    expect(dead_code, reason = "the terminal is on Unix and Windows")
)]

use std::collections::{HashMap, VecDeque};
use std::ops::Range;

use termelon::buffer::Buffer;
use termelon::feed::{Change, ItemId, Measure};
use termelon::layout::Rect;
use termelon::style::{Color, Modifier, Style};
use termelon::widgets::{FeedRows, FeedView};

/// The most items kept. The oldest go past it.
const KEEP: usize = 3_000;

/// Ticks of the session played before the first frame, so there's a history to scroll.
const HISTORY: u32 = 6_000;

const WORDS: &[&str] = &[
    "the", "row", "anchor", "scroll", "frame", "block", "height", "reply", "wraps", "at", "a",
    "width", "and", "keeps", "its", "place", "while", "text", "streams", "in", "below", "reader",
    "of", "is", "to", "screen", "feed", "item", "each", "measured", "when", "it", "shows", "so",
    "nothing", "above", "moves", "terminal", "line", "mark", "offset", "end", "follows", "every",
    "change", "fold", "opens", "closes", "output", "grows", "under", "cursor", "never", "jumps",
];

const FILES: &[&str] = &[
    "src/tui/transcript.rs",
    "src/tui/wrap.rs",
    "src/tui/markdown.rs",
    "src/app.rs",
    "src/agent/turn.rs",
    "src/feed/index.rs",
    "docs/scrolling.md",
];

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

struct Rng(u64);

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

    fn between(&mut self, low: u32, high: u32) -> u32 {
        low + u32::try_from(self.below(u64::from(high - low + 1))).unwrap_or(0)
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    fn pick<'a>(&mut self, from: &[&'a str]) -> &'a str {
        from[usize::try_from(self.below(from.len() as u64)).unwrap_or(0)]
    }

    fn words(&mut self, count: u32) -> String {
        let words: Vec<&str> = (0..count).map(|_| self.pick(WORDS)).collect();
        words.join(" ")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Prompt,
    Reply,
    Tool,
    Working,
}

/// A line of an item's source, with an id that stays with it, which marks name.
struct Line {
    id: u64,
    text: String,
}

/// An item, its lines, and for a tool its output, which shows when the fold is open.
struct Item {
    kind: Kind,
    head: Vec<Line>,
    body: Vec<Line>,
    open: bool,
}

/// A row as drawn, with the line it shows and the byte of the line it starts at.
struct Row {
    text: String,
    line: u64,
    byte: usize,
    style: Style,
}

/// The transcript's text, and its rows at the width, made when an item is first shown.
#[derive(Default)]
struct Doc {
    items: HashMap<u64, Item>,
    order: VecDeque<u64>,
    width: usize,
    rows: HashMap<u64, Vec<Row>>,
    next_line: u64,
}

/// `text` wrapped at `width` columns between words, each row with the byte it starts at.
fn wrap(text: &str, width: usize) -> Vec<(usize, String)> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut row = String::new();
    let mut start = 0;
    let mut at = 0;
    for word in text.split(' ') {
        let len = word.chars().count();
        let row_len = row.chars().count();
        if row_len > 0 && row_len + 1 + len > width {
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
    rows
}

impl Doc {
    fn line(&mut self, text: String) -> Line {
        self.next_line += 1;
        Line {
            id: self.next_line,
            text,
        }
    }

    fn item(&mut self, key: u64) -> Option<&mut Item> {
        self.rows.remove(&key);
        self.items.get_mut(&key)
    }

    fn rows(&mut self, key: u64) -> &[Row] {
        let width = self.width;
        let items = &self.items;
        self.rows.entry(key).or_insert_with(|| {
            items
                .get(&key)
                .map_or_else(Vec::new, |item| layout(item, width))
        })
    }
}

/// An item's rows at `width`, with a blank row before it.
fn layout(item: &Item, width: usize) -> Vec<Row> {
    let blank = || Row {
        text: String::new(),
        line: 0,
        byte: 0,
        style: Style::new(),
    };
    let mut rows = vec![blank()];
    if item.kind == Kind::Working {
        let text = item.head.first().map_or("", |line| line.text.as_str());
        rows.push(Row {
            text: text.to_owned(),
            line: 0,
            byte: 0,
            style: Style::new().fg(Color::Yellow),
        });
        rows.push(blank());
        return rows;
    }
    let style = match item.kind {
        Kind::Prompt => Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        Kind::Tool => Style::new().fg(Color::Green),
        _ => Style::new(),
    };
    for line in &item.head {
        for (byte, text) in wrap(&line.text, width) {
            rows.push(Row {
                text,
                line: line.id,
                byte,
                style,
            });
        }
    }
    let dim = Style::new().fg(Color::DarkGray);
    if item.open {
        for (i, line) in item.body.iter().enumerate() {
            for (j, (byte, text)) in wrap(&line.text, width.saturating_sub(4))
                .into_iter()
                .enumerate()
            {
                let lead = if i == 0 && j == 0 { "  ⎿ " } else { "    " };
                rows.push(Row {
                    text: format!("{lead}{text}"),
                    line: line.id,
                    byte,
                    style: dim,
                });
            }
        }
    } else if !item.body.is_empty() {
        rows.push(Row {
            text: format!("  ⎿ {} lines", item.body.len()),
            line: 0,
            byte: 0,
            style: dim,
        });
    }
    rows
}

/// A mark is a line's id and a byte of it.
const BYTE_BITS: u32 = 24;

impl Measure<u64> for Doc {
    fn height(&mut self, key: &u64) -> u32 {
        u32::try_from(self.rows(*key).len()).unwrap_or(u32::MAX)
    }

    fn mark_at(&mut self, key: &u64, row: u32) -> Option<u64> {
        let row = self.rows(*key).get(usize::try_from(row).ok()?)?;
        let byte = u64::try_from(row.byte).ok()?;
        (row.line != 0).then_some(row.line << BYTE_BITS | byte)
    }

    fn resolve(&mut self, key: &u64, mark: u64) -> Option<u32> {
        let (line, byte) = (mark >> BYTE_BITS, mark & ((1 << BYTE_BITS) - 1));
        let rows = self.rows(*key);
        let found = rows
            .iter()
            .rposition(|row| row.line == line && row.byte as u64 <= byte)
            // (A line inside a fold that closed is on the fold's last row.)
            .or_else(|| rows.len().checked_sub(1))?;
        u32::try_from(found).ok()
    }
}

impl FeedRows<u64> for Doc {
    fn set_width(&mut self, width: u16) {
        self.width = usize::from(width);
        self.rows.clear();
    }

    fn render(&mut self, key: &u64, rows: Range<u32>, area: Rect, buf: &mut Buffer) {
        let all = self.rows(*key);
        let shown = all
            .get(rows.start as usize..rows.end as usize)
            .unwrap_or_default();
        for (y, row) in (area.y..).zip(shown) {
            buf.set_stringn(area.x, y, &row.text, usize::from(area.width), row.style);
        }
    }
}

/// What the session does next, over one tick or several.
enum Act {
    Prompt,
    Working(u32),
    /// A tool's call, whose result comes a few ticks later.
    Tool,
    /// The result of the tool call that's this item.
    Result(u64),
    /// A tool whose output grows a line at a time while its fold is open.
    Running(u32),
    /// A reply of this many words, streaming.
    Reply(u32),
    /// A recent tool's fold opens or closes.
    Fold,
    Wait(u32),
}

/// The session, the items as the feed has them, and the two views.
struct Session {
    doc: Doc,
    feed: FeedView<u64>,
    share: ShareView,
    ids: HashMap<u64, ItemId>,
    next_key: u64,
    rng: Rng,
    plan: VecDeque<Act>,
    working: Option<u64>,
    /// The reply streaming, and the byte its open bold marker is at.
    reply: Option<(u64, Option<usize>)>,
    /// The tool whose output is growing.
    running: Option<u64>,
    tick: u64,
}

impl Session {
    fn new(seed: u64) -> Self {
        Self {
            doc: Doc {
                width: 80,
                ..Doc::default()
            },
            feed: FeedView::new(),
            share: ShareView::default(),
            ids: HashMap::new(),
            next_key: 0,
            rng: Rng::new(seed),
            plan: VecDeque::new(),
            working: None,
            reply: None,
            running: None,
            tick: 0,
        }
    }

    fn push(&mut self, kind: Kind, head: Vec<String>, body: Vec<String>, open: bool) -> u64 {
        let key = self.next_key;
        self.next_key += 1;
        let head = head.into_iter().map(|text| self.doc.line(text)).collect();
        let body = body.into_iter().map(|text| self.doc.line(text)).collect();
        self.doc.items.insert(
            key,
            Item {
                kind,
                head,
                body,
                open,
            },
        );
        self.doc.order.push_back(key);
        self.ids.insert(key, self.feed.feed_mut().push(key, 2));
        while self.doc.order.len() > KEEP {
            if let Some(old) = self.doc.order.pop_front() {
                self.forget(old);
            }
        }
        key
    }

    fn forget(&mut self, key: u64) {
        self.doc.items.remove(&key);
        self.doc.rows.remove(&key);
        if let Some(id) = self.ids.remove(&key) {
            self.feed.feed_mut().remove(id);
        }
    }

    fn remove(&mut self, key: u64) {
        if let Some(at) = self.doc.order.iter().rposition(|k| *k == key) {
            self.doc.order.remove(at);
        }
        self.forget(key);
    }

    fn changed(&mut self, key: u64, change: Change) {
        if let Some(id) = self.ids.get(&key) {
            self.feed.feed_mut().changed(*id, change);
        }
    }

    /// "Working…" gives way to what comes next.
    fn stop_working(&mut self) {
        if let Some(key) = self.working.take() {
            self.remove(key);
        }
    }

    fn plan_turn(&mut self) {
        let rng = &mut self.rng;
        self.plan.push_back(Act::Prompt);
        self.plan.push_back(Act::Working(rng.between(10, 40)));
        for _ in 0..rng.between(1, 3) {
            self.plan.push_back(if rng.chance(40) {
                Act::Running(rng.between(15, 90))
            } else {
                Act::Tool
            });
            self.plan.push_back(Act::Working(rng.between(5, 30)));
        }
        let words = if rng.chance(30) {
            rng.between(250, 700)
        } else {
            rng.between(20, 160)
        };
        self.plan.push_back(Act::Reply(words));
        if rng.chance(60) {
            self.plan.push_back(Act::Fold);
        }
        self.plan.push_back(Act::Wait(rng.between(10, 60)));
    }

    /// One tick of the session.
    fn step(&mut self) {
        self.tick += 1;
        let Some(act) = self.plan.pop_front() else {
            self.plan_turn();
            return;
        };
        match act {
            Act::Prompt => {
                let words = self.rng.between(5, 30);
                let text = format!("> {}", self.rng.words(words));
                self.push(Kind::Prompt, vec![text], Vec::new(), false);
            }
            Act::Working(ticks) => {
                match self.working {
                    None => {
                        let key = self.push(Kind::Working, vec![working(0)], Vec::new(), false);
                        self.working = Some(key);
                    }
                    Some(key) => {
                        let spin = usize::try_from(self.tick).unwrap_or(0);
                        if let Some(item) = self.doc.item(key)
                            && let Some(line) = item.head.first_mut()
                        {
                            line.text = working(spin);
                        }
                        self.changed(key, Change::Reflowed);
                    }
                }
                if ticks > 1 {
                    self.plan.push_front(Act::Working(ticks - 1));
                }
            }
            Act::Tool => {
                self.stop_working();
                let verb = self.rng.pick(&["Read", "Grep", "Edit", "Glob"]);
                let file = self.rng.pick(FILES);
                let head = vec![format!("⏺ {verb} {file}")];
                let key = self.push(Kind::Tool, head, Vec::new(), false);
                self.plan.push_front(Act::Result(key));
                let wait = self.rng.between(3, 15);
                self.plan.push_front(Act::Wait(wait));
            }
            Act::Result(key) => {
                let lines = self.rng.between(3, 60);
                let body: Vec<Line> = (0..lines)
                    .map(|_| {
                        let n = self.rng.between(2, 14);
                        let text = self.rng.words(n);
                        self.doc.line(text)
                    })
                    .collect();
                if let Some(item) = self.doc.item(key) {
                    item.body = body;
                }
                self.changed(key, Change::Appended);
            }
            Act::Running(lines) => {
                let Some(key) = self.running else {
                    self.stop_working();
                    let head = vec!["⏺ Bash cargo test --workspace".to_owned()];
                    self.running = Some(self.push(Kind::Tool, head, Vec::new(), true));
                    self.plan.push_front(Act::Running(lines));
                    return;
                };
                // (Output comes in chunks of a line or a few.)
                let chunk = self.rng.between(1, 4).min(lines);
                let mut new = Vec::new();
                for _ in 0..chunk {
                    let n = self.rng.between(1, 4);
                    let text = format!("test {} ... ok", self.rng.words(n).replace(' ', "_"));
                    new.push(self.doc.line(text));
                }
                if let Some(item) = self.doc.item(key) {
                    item.body.extend(new);
                }
                self.changed(key, Change::Appended);
                if lines > chunk {
                    self.plan.push_front(Act::Running(lines - chunk));
                    let pause = self.rng.between(0, 2);
                    if pause > 0 {
                        self.plan.push_front(Act::Wait(pause));
                    }
                } else {
                    // Done, and the output folds away.
                    self.running = None;
                    if let Some(item) = self.doc.item(key) {
                        item.open = false;
                    }
                    self.changed(key, Change::Reflowed);
                }
            }
            Act::Reply(words) => {
                if self.reply.is_none() {
                    self.stop_working();
                    let first = self.rng.between(1, 3);
                    let text = self.rng.words(first);
                    let key = self.push(Kind::Reply, vec![text], Vec::new(), false);
                    self.reply = Some((key, None));
                    if words > first {
                        self.plan.push_front(Act::Reply(words - first));
                    } else {
                        self.reply = None;
                    }
                    return;
                }
                self.stream(words);
            }
            Act::Fold => {
                let tools: Vec<u64> =
                    self.doc
                        .order
                        .iter()
                        .rev()
                        .take(400)
                        .copied()
                        .filter(|key| {
                            self.doc.items.get(key).is_some_and(|item| {
                                item.kind == Kind::Tool && !item.body.is_empty()
                            })
                        })
                        .collect();
                if !tools.is_empty() {
                    let at = usize::try_from(self.rng.below(tools.len() as u64)).unwrap_or(0);
                    let key = tools[at];
                    if let Some(item) = self.doc.item(key) {
                        item.open = !item.open;
                    }
                    self.changed(key, Change::Reflowed);
                }
            }
            Act::Wait(ticks) => {
                if ticks > 1 {
                    self.plan.push_front(Act::Wait(ticks - 1));
                }
            }
        }
    }

    /// A few more words of the reply, now and then a burst, a new paragraph, or a bold phrase
    /// whose marker closes, which re-wraps it shorter.
    fn stream(&mut self, words: u32) {
        let Some((key, bold)) = self.reply else {
            return;
        };
        let burst = if self.rng.chance(8) {
            self.rng.between(10, 30)
        } else {
            self.rng.between(1, 4)
        };
        let count = burst.min(words);
        let mut text = self.rng.words(count);
        let mut change = Change::Appended;
        let mut bold = bold;
        let paragraph = self.rng.chance(6);
        let open_bold = bold.is_none() && self.rng.chance(10);
        let close_bold = bold.is_some() && self.rng.chance(35);
        let new_lines = if paragraph {
            vec![self.doc.line(String::new()), self.doc.line(String::new())]
        } else {
            Vec::new()
        };
        if let Some(item) = self.doc.item(key) {
            if paragraph && bold.is_none() {
                item.head.extend(new_lines);
            }
            if let Some(line) = item.head.last_mut() {
                if open_bold {
                    text = format!("**{text}");
                }
                if !line.text.is_empty() {
                    line.text.push(' ');
                }
                if open_bold {
                    bold = Some(line.text.len());
                }
                line.text.push_str(&text);
                if close_bold && let Some(at) = bold.take() {
                    // The marker closed, so the phrase shows without it.
                    if line.text.get(at..at + 2) == Some("**") {
                        line.text.replace_range(at..at + 2, "");
                        change = Change::Reflowed;
                    }
                }
            }
        }
        self.reply = Some((key, bold));
        self.changed(key, change);
        if words > count {
            self.plan.push_front(Act::Reply(words - count));
        } else {
            self.reply = None;
        }
    }
}

fn working(tick: usize) -> String {
    format!("{} Working…", SPINNER[tick % SPINNER.len()])
}

/// The rule of a transcript that keeps the block at the top and a share of its height, rescaled
/// when the block's height changes. Following pins the end to the last row, and a view that
/// reaches the bottom, by a scroll or by the end getting shorter, follows again.
#[derive(Default)]
struct ShareView {
    /// The block at the top, the rows of it above the screen, and its height then.
    anchor: Option<(u64, usize, usize)>,
    pending: i64,
}

impl ShareView {
    /// Draws the transcript into `area` and gives the rows below the screen.
    fn render(&mut self, doc: &mut Doc, area: Rect, buf: &mut Buffer) -> usize {
        let width = usize::from(area.width);
        if doc.width != width {
            doc.set_width(area.width);
        }
        let height = usize::from(area.height);
        let keys: Vec<u64> = doc.order.iter().copied().collect();
        let heights: Vec<usize> = keys.iter().map(|key| doc.rows(*key).len()).collect();
        let mut starts = Vec::with_capacity(keys.len());
        let mut total = 0;
        for h in &heights {
            starts.push(total);
            total += h;
        }
        let bottom = total.saturating_sub(height);
        let mut top = match self.anchor {
            None => bottom,
            Some((key, offset, was)) => {
                let index = keys
                    .iter()
                    .position(|k| *k == key)
                    .or_else(|| keys.iter().position(|k| *k > key));
                match index {
                    Some(index) => {
                        let now = heights[index];
                        let offset = if was == 0 || now == was {
                            offset
                        } else {
                            offset * now / was
                        };
                        starts[index] + offset.min(now.saturating_sub(1))
                    }
                    None => bottom,
                }
            }
        };
        if self.pending != 0 {
            let moved = i64::try_from(top)
                .unwrap_or(i64::MAX)
                .saturating_add(self.pending);
            top = usize::try_from(moved.max(0)).unwrap_or(0);
            self.pending = 0;
        }
        top = top.min(bottom);
        let index = starts
            .partition_point(|start| *start <= top)
            .saturating_sub(1);
        self.anchor = if keys.is_empty() || top >= bottom {
            None
        } else {
            Some((keys[index], top - starts[index], heights[index]))
        };
        // The rows from `top` down.
        let mut y = 0;
        let mut at = index;
        let mut skip = top.saturating_sub(starts.get(index).copied().unwrap_or(0));
        while y < height && at < keys.len() {
            let rows = heights[at].saturating_sub(skip).min(height - y);
            if rows > 0 {
                let (Ok(from), Ok(to), Ok(row), Ok(high)) = (
                    u32::try_from(skip),
                    u32::try_from(skip + rows),
                    u16::try_from(y),
                    u16::try_from(rows),
                ) else {
                    break;
                };
                let rect = Rect::new(area.x, area.y + row, area.width, high);
                doc.render(&keys[at], from..to, rect, buf);
            }
            y += rows;
            skip = 0;
            at += 1;
        }
        bottom - top
    }

    fn scroll_by(&mut self, rows: i64) {
        self.pending = self.pending.saturating_add(rows);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Share,
    Feed,
    Gap,
}

impl Session {
    /// Moves to `mode`, keeping the row at the top.
    fn switch(&mut self, from: Mode, to: Mode) {
        self.feed
            .feed_mut()
            .set_gap_limit(if to == Mode::Gap { 3 } else { 0 });
        match (from, to) {
            (Mode::Share, Mode::Feed | Mode::Gap) => match self.share.anchor {
                None => self.feed.feed_mut().follow(),
                Some((key, offset, _)) => {
                    if let Some(id) = self.ids.get(&key) {
                        let offset = u32::try_from(offset).unwrap_or(0);
                        self.feed.feed_mut().place(*id, offset, 0);
                    }
                }
            },
            (Mode::Feed | Mode::Gap, Mode::Share) => {
                let first = self.feed.placement().items.first().copied();
                self.share.anchor = match first {
                    Some(placed) if !self.feed.feed().is_following() => {
                        let key = self.feed.feed().key(placed.item).copied();
                        key.map(|key| {
                            let offset = usize::try_from(-placed.y).unwrap_or(0);
                            let height = usize::try_from(placed.height).unwrap_or(0);
                            (key, offset, height)
                        })
                    }
                    _ => None,
                };
            }
            _ => {}
        }
    }

    fn scroll_by(&mut self, mode: Mode, rows: i64) {
        match mode {
            Mode::Share => self.share.scroll_by(rows),
            Mode::Feed | Mode::Gap => self.feed.feed_mut().scroll_by(rows),
        }
    }

    fn follow(&mut self, mode: Mode) {
        match mode {
            Mode::Share => self.share = ShareView::default(),
            Mode::Feed | Mode::Gap => self.feed.feed_mut().follow(),
        }
    }

    fn go_to_start(&mut self, mode: Mode) {
        match mode {
            Mode::Share => {
                self.share.pending = 0;
                if let Some(first) = self.doc.order.front().copied() {
                    let height = self.doc.rows(first).len();
                    self.share.anchor = Some((first, 0, height));
                }
            }
            Mode::Feed | Mode::Gap => self.feed.feed_mut().scroll_to_start(),
        }
    }

    /// Draws the transcript and the status line.
    fn draw(&mut self, mode: Mode, buf: &mut Buffer) {
        let area = buf.area();
        if area.height < 2 {
            return;
        }
        let list = Rect::new(area.x, area.y, area.width, area.height - 1);
        let below = match mode {
            Mode::Share => self.share.render(&mut self.doc, list, buf),
            Mode::Feed | Mode::Gap => {
                self.feed.render(list, buf, &mut self.doc);
                let feed = self.feed.feed();
                let viewport = u64::from(list.height);
                let seen = feed.offset(u32::from(list.height)) + viewport;
                usize::try_from(feed.extent().total.saturating_sub(seen)).unwrap_or(0)
            }
        };
        let following = match mode {
            Mode::Share => self.share.anchor.is_none(),
            Mode::Feed | Mode::Gap => self.feed.feed().is_following(),
        };
        let y = area.bottom() - 1;
        let bar = Style::new().fg(Color::Black).bg(Color::Gray);
        buf.set_style(Rect::new(area.x, y, area.width, 1), bar);
        let mut x = area.x;
        for (key, name, this) in [
            ("1", "Arko 0.20 rule", Mode::Share),
            ("2", "feed", Mode::Feed),
            ("3", "feed, blank rows", Mode::Gap),
        ] {
            let style = if this == mode {
                Style::new()
                    .fg(Color::White)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            } else {
                bar
            };
            let label = format!(" {key} {name} ");
            let (next, _) = buf.set_stringn(x, y, &label, usize::from(area.right() - x), style);
            x = next.saturating_add(1);
        }
        let place = if following {
            "following".to_owned()
        } else {
            format!("{below} rows below")
        };
        let right = format!("{place}   q quits ");
        let width = u16::try_from(right.chars().count()).unwrap_or(0);
        if x + width <= area.right() {
            buf.set_string(area.right() - width, y, right, bar);
        }
    }
}

#[cfg(any(unix, windows))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};

    use termelon::{
        Drawn, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind, Options, Terminal,
    };

    // (Replies stream at a frame every 33 ms.)
    const TICK: Duration = Duration::from_millis(33);

    let args: Vec<String> = std::env::args().skip(1).collect();
    let seed = match args.as_slice() {
        [] => 1,
        [flag, seed] if flag == "--seed" => seed.parse()?,
        _ => return Err("the arguments are none, or --seed and a number".into()),
    };
    let mut session = Session::new(seed);
    for _ in 0..HISTORY {
        session.step();
    }
    let mut terminal = Terminal::open(Options::default())?;
    let mut events = terminal.events();
    let mut mode = Mode::Share;
    let mut next_tick = Instant::now();
    let mut dirty = true;
    loop {
        let now = Instant::now();
        if now >= next_tick {
            session.step();
            // Idle time measures items not yet measured, for the count of rows below.
            session.feed.feed_mut().sweep(64, &mut session.doc);
            next_tick = (next_tick + TICK).max(now);
            dirty = true;
        }
        if dirty {
            let drawn = terminal.draw(|frame| session.draw(mode, frame.buffer_mut()))?;
            dirty = matches!(drawn, Drawn::Held);
        }
        let wait = if dirty {
            Duration::from_millis(5)
        } else {
            next_tick.saturating_duration_since(Instant::now())
        };
        let Some(event) = events.next_timeout(wait)? else {
            continue;
        };
        let page = i64::from(terminal.size().height.saturating_sub(3).max(1));
        match event {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                match key.code {
                    KeyCode::Char('q') => break,
                    KeyCode::Char('c') if ctrl => break,
                    KeyCode::Char(digit @ '1'..='3') => {
                        let to = match digit {
                            '1' => Mode::Share,
                            '2' => Mode::Feed,
                            _ => Mode::Gap,
                        };
                        session.switch(mode, to);
                        mode = to;
                    }
                    KeyCode::Up => session.scroll_by(mode, -1),
                    KeyCode::Down => session.scroll_by(mode, 1),
                    KeyCode::PageUp => session.scroll_by(mode, -page),
                    KeyCode::PageDown => session.scroll_by(mode, page),
                    KeyCode::Home => session.go_to_start(mode),
                    KeyCode::End => session.follow(mode),
                    _ => continue,
                }
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollUp => session.scroll_by(mode, -3),
                MouseEventKind::ScrollDown => session.scroll_by(mode, 3),
                _ => continue,
            },
            Event::Resize(_) | Event::Resumed => {}
            _ => continue,
        }
        dirty = true;
    }
    Ok(terminal.close()?)
}

#[cfg(not(any(unix, windows)))]
fn main() {}
