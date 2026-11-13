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
//! | + and −, Space | Stream faster or slower, and pause it |
//! | r | Count again |
//! | q, Ctrl+C | Quit |
//!
//! The status line counts the frames where text moved under the reader without their input, the
//! row at the top while scrolled up and any row moving down while following, for each mode. Those
//! rows flash for 300 ms. The row above it suggests what to try.
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
    /// The words of the streaming reply so far.
    streamed: u32,
    /// The lines left of the code block streaming.
    code: u32,
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
            streamed: 0,
            code: 0,
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
            rng.between(1_200, 2_500)
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
                    self.streamed = first;
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
        // A long reply now and then shows a code block, which comes a line or a few at a time.
        if self.code > 0 || (bold.is_none() && words > 300 && self.rng.chance(4)) {
            self.stream_code(key, words);
            return;
        }
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
        self.streamed += count;
        self.changed(key, change);
        if words > count {
            self.plan.push_front(Act::Reply(words - count));
        } else {
            self.reply = None;
        }
    }
}

impl Session {
    /// Lines of a code block in the reply, which count as 6 words each.
    fn stream_code(&mut self, key: u64, words: u32) {
        let mut lines = Vec::new();
        if self.code == 0 {
            self.code = self.rng.between(15, 60);
            lines.push(self.doc.line(String::new()));
            lines.push(self.doc.line("```rust".to_owned()));
        }
        for _ in 0..self.rng.between(1, 3).min(self.code) {
            let indent = "    ".repeat(usize::try_from(self.rng.below(3)).unwrap_or(0));
            let text = match self.rng.below(4) {
                0 => format!(
                    "{indent}let {} = {}({});",
                    self.ident(),
                    self.ident(),
                    self.ident()
                ),
                1 => format!("{indent}if {}.{}() {{", self.ident(), self.ident()),
                2 => format!(
                    "{indent}{}.{}({}, {});",
                    self.ident(),
                    self.ident(),
                    self.ident(),
                    self.ident()
                ),
                _ => format!("{indent}}}"),
            };
            lines.push(self.doc.line(text));
            self.code -= 1;
        }
        if self.code == 0 {
            lines.push(self.doc.line("```".to_owned()));
            lines.push(self.doc.line(String::new()));
            lines.push(self.doc.line(String::new()));
        }
        let count = u32::try_from(lines.len()).unwrap_or(1) * 6;
        if let Some(item) = self.doc.item(key) {
            item.head.extend(lines);
        }
        self.changed(key, Change::Appended);
        self.streamed += count;
        if words > count {
            self.plan.push_front(Act::Reply(words - count));
        } else {
            self.reply = None;
            self.code = 0;
        }
    }

    fn ident(&mut self) -> String {
        let n = self.rng.between(1, 2);
        self.rng.words(n).replace(' ', "_")
    }

    /// Whether a long reply is streaming, `rows` of it shown and much of it still to come.
    fn long_reply_streaming(&mut self, rows: usize) -> bool {
        let Some((key, _)) = self.reply else {
            return false;
        };
        matches!(self.plan.front(), Some(Act::Reply(left)) if *left >= 600)
            && self.doc.rows(key).len() >= rows
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
    /// Draws the transcript into `area`, and gives the rows below the screen and each block
    /// drawn with the row its top is on.
    fn render(&mut self, doc: &mut Doc, area: Rect, buf: &mut Buffer) -> (usize, Vec<(u64, i64)>) {
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
        let mut placed = Vec::new();
        let mut y = 0;
        let mut at = index;
        let mut skip = top.saturating_sub(starts.get(index).copied().unwrap_or(0));
        while y < height && at < keys.len() {
            let rows = heights[at].saturating_sub(skip).min(height - y);
            let top = i64::try_from(y).unwrap_or(0) - i64::try_from(skip).unwrap_or(0);
            placed.push((keys[at], top));
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
        (bottom - top, placed)
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

impl Mode {
    fn index(self) -> usize {
        match self {
            Mode::Share => 0,
            Mode::Feed => 1,
            Mode::Gap => 2,
        }
    }

    /// The best thing to try in the mode, for the hint row.
    fn hint(self) -> &'static str {
        match self {
            Mode::Share => "Try: press ↑ 5 times while a long reply streams. Rows that slip flash.",
            Mode::Feed => {
                "Try the same: ↑ 5 times during a long reply. Nothing flashes. Then End, and \
                 watch Working… go."
            }
            Mode::Gap => {
                "Try: End, and watch the bottom as Working… goes. Blank rows open instead."
            }
        }
    }
}

/// A row of text on screen, as its item, its line, the byte of the line the row starts at, and
/// the screen row it's on.
#[derive(Clone, Copy)]
struct Seen {
    key: u64,
    line: u64,
    byte: usize,
    y: i64,
}

/// What a frame showed, to compare the next one with.
struct Shown {
    area: Rect,
    mode: Mode,
    following: bool,
    rows: Vec<Seen>,
}

/// How long a row that moved under the reader shows highlighted.
const FLASH: std::time::Duration = std::time::Duration::from_millis(300);

/// Counts the frames where text moved under the reader, and highlights the rows that moved.
#[derive(Default)]
struct Watch {
    /// For each mode, the frames where text moved without the reader's input.
    moved: [u32; 3],
    /// For each mode, the ticks of the session it saw.
    ticks: [u64; 3],
    last: Option<Shown>,
    /// The reader scrolled, switched or resized since the last frame.
    input: bool,
    flashes: Vec<(Seen, std::time::Instant)>,
}

impl Watch {
    fn reset(&mut self) {
        self.moved = [0; 3];
        self.ticks = [0; 3];
        self.flashes.clear();
    }

    /// When the next highlight ends.
    fn next_flash_end(&self) -> Option<std::time::Instant> {
        self.flashes.iter().map(|(_, until)| *until).min()
    }
}

impl Doc {
    /// The row of the item that holds byte `byte` of line `line` now, if the line shows.
    fn row_of(&mut self, key: u64, line: u64, byte: usize) -> Option<usize> {
        self.rows(key)
            .iter()
            .rposition(|row| row.line == line && row.byte <= byte)
    }
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

    /// Draws the transcript, a hint and the status line, and counts and highlights the text
    /// that moved under the reader since the last frame.
    fn draw(&mut self, mode: Mode, watch: &mut Watch, status: &Status, buf: &mut Buffer) {
        let area = buf.area();
        if area.height < 3 {
            return;
        }
        let list = Rect::new(area.x, area.y, area.width, area.height - 2);
        let (below, placed) = match mode {
            Mode::Share => {
                let width = self.doc.width;
                let drawn = self.share.render(&mut self.doc, list, buf);
                // (The text is wrapped at another width now, so heights the feed knew are hints.)
                if self.doc.width != width {
                    self.feed.feed_mut().set_geometry();
                }
                drawn
            }
            Mode::Feed | Mode::Gap => {
                self.feed.render(list, buf, &mut self.doc);
                let feed = self.feed.feed();
                let viewport = u64::from(list.height);
                let seen = feed.offset(u32::from(list.height)) + viewport;
                let below = usize::try_from(feed.extent().total.saturating_sub(seen)).unwrap_or(0);
                let placed = self
                    .feed
                    .placement()
                    .items
                    .iter()
                    .filter_map(|p| Some((*feed.key(p.item)?, p.y)))
                    .collect();
                (below, placed)
            }
        };
        let following = match mode {
            Mode::Share => self.share.anchor.is_none(),
            Mode::Feed | Mode::Gap => self.feed.feed().is_following(),
        };
        let height = i64::from(list.height);
        let mut rows = Vec::new();
        for &(key, top) in &placed {
            for (i, row) in self.doc.rows(key).iter().enumerate() {
                let y = top + i64::try_from(i).unwrap_or(i64::MAX);
                if (0..height).contains(&y) && row.line != 0 {
                    rows.push(Seen {
                        key,
                        line: row.line,
                        byte: row.byte,
                        y,
                    });
                }
            }
        }
        // Where a row of the last frame is now. `None` when its text no longer shows, and
        // `Some(None)` when it's off the screen.
        let now_at = |doc: &mut Doc, seen: &Seen| -> Option<Option<i64>> {
            let i = i64::try_from(doc.row_of(seen.key, seen.line, seen.byte)?).ok()?;
            Some(
                placed
                    .iter()
                    .find(|(k, _)| *k == seen.key)
                    .map(|(_, top)| top + i),
            )
        };
        let last = watch.last.take();
        if let Some(last) = last
            && !watch.input
            && last.area == list
            && last.mode == mode
        {
            let mut moved = Vec::new();
            if last.following && following {
                // Following, a row that moves down moved under the reader.
                for seen in &last.rows {
                    if let Some(Some(y)) = now_at(&mut self.doc, seen)
                        && y > seen.y
                    {
                        moved.push(*seen);
                    }
                }
            } else if !last.following
                && let Some(top) = last.rows.first()
                && now_at(&mut self.doc, top).is_some_and(|y| y != Some(top.y))
            {
                // Scrolled up, the text at the top moved, and so did every row with it.
                for seen in &last.rows {
                    if now_at(&mut self.doc, seen).is_some_and(|y| y != Some(seen.y)) {
                        moved.push(*seen);
                    }
                }
            }
            if !moved.is_empty() {
                watch.moved[mode.index()] += 1;
                let until = status.now + FLASH;
                watch
                    .flashes
                    .extend(moved.into_iter().map(|seen| (seen, until)));
            }
        }
        watch.input = false;
        watch.last = Some(Shown {
            area: list,
            mode,
            following,
            rows,
        });
        watch.flashes.retain(|(_, until)| *until > status.now);
        let flash = Style::new().bg(Color::Indexed(238));
        for (seen, _) in &watch.flashes {
            if let Some(Some(y)) = now_at(&mut self.doc, seen)
                && let Ok(y) = u16::try_from(y)
                && y < list.height
            {
                buf.set_style(Rect::new(list.x, list.y + y, list.width, 1), flash);
            }
        }

        // The hint.
        let hint_y = area.bottom() - 2;
        let dim = Style::new().fg(Color::Gray);
        let keys = "  + − speed  space pause  r reset";
        buf.set_stringn(area.x, hint_y, mode.hint(), usize::from(area.width), dim);
        let hint_width = u16::try_from(mode.hint().chars().count()).unwrap_or(u16::MAX);
        let keys_width = u16::try_from(keys.chars().count()).unwrap_or(u16::MAX);
        if hint_width.saturating_add(keys_width) <= area.width {
            buf.set_string(
                area.right() - keys_width,
                hint_y,
                keys,
                Style::new().fg(Color::DarkGray),
            );
        }

        // The status line.
        let y = area.bottom() - 1;
        let bar = Style::new().fg(Color::Black).bg(Color::Gray);
        buf.set_style(Rect::new(area.x, y, area.width, 1), bar);
        let mut x = area.x;
        for (key, name, this) in [
            ("1", "Arko 0.20", Mode::Share),
            ("2", "feed", Mode::Feed),
            ("3", "feed + blank rows", Mode::Gap),
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
        let seconds = watch.ticks[mode.index()] * TICK_MS / 1_000;
        let mut parts = vec![format!(
            "moved under you: {} in {seconds} s",
            watch.moved[mode.index()]
        )];
        parts.push(if following {
            "following".to_owned()
        } else {
            format!("{below} rows below")
        });
        if status.paused {
            parts.push("paused".to_owned());
        } else if status.speed != "1×" {
            parts.push(status.speed.to_owned());
        }
        parts.push("q quits".to_owned());
        // (What doesn't fit goes, from the end.)
        let room = usize::from(area.right().saturating_sub(x));
        while parts.len() > 1 && parts.join("   ").chars().count() + 1 > room {
            parts.pop();
        }
        let right = format!("{} ", parts.join("   "));
        let width = u16::try_from(right.chars().count()).unwrap_or(u16::MAX);
        if x.saturating_add(width) <= area.right() {
            buf.set_string(area.right() - width, y, right, bar);
        }
    }
}

/// What the status line says besides the counts.
struct Status {
    now: std::time::Instant,
    paused: bool,
    speed: &'static str,
}

/// A tick at normal speed, as replies stream at a frame every 33 ms.
const TICK_MS: u64 = 33;

/// The speeds, as ticks of how many milliseconds, and their names.
const SPEEDS: [(u64, &str); 5] = [
    (264, "⅛×"),
    (132, "¼×"),
    (66, "½×"),
    (TICK_MS, "1×"),
    (16, "2×"),
];

#[cfg(any(unix, windows))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};

    use termelon::{
        Drawn, Event, KeyCode, KeyEventKind, KeyModifiers, MouseEventKind, Options, Terminal,
    };

    let args: Vec<String> = std::env::args().skip(1).collect();
    let seed = match args.as_slice() {
        [] => 1,
        [flag, seed] if flag == "--seed" => seed.parse()?,
        _ => return Err("the arguments are none, or --seed and a number".into()),
    };
    let mut terminal = Terminal::open(Options::default())?;
    let size = terminal.size();
    let mut session = Session::new(seed);
    session.doc.set_width(size.width);
    for _ in 0..HISTORY {
        session.step();
    }
    // (It opens in the middle of a long reply, two screens of it shown, which the first thing to
    // try needs.)
    let screens = usize::from(size.height) * 2;
    for _ in 0..200_000 {
        if session.long_reply_streaming(screens) {
            break;
        }
        session.step();
    }
    let mut events = terminal.events();
    let mut mode = Mode::Share;
    let mut watch = Watch::default();
    let mut speed = 3;
    let mut paused = false;
    let mut next_tick = Instant::now();
    let mut dirty = true;
    loop {
        let now = Instant::now();
        let tick = Duration::from_millis(SPEEDS[speed].0);
        if paused {
            next_tick = now + tick;
        } else if now >= next_tick {
            session.step();
            // Idle time measures items not yet measured, for the count of rows below.
            session.feed.feed_mut().sweep(64, &mut session.doc);
            watch.ticks[mode.index()] += 1;
            next_tick = (next_tick + tick).max(now);
            dirty = true;
        }
        if watch.next_flash_end().is_some_and(|end| end <= now) {
            dirty = true;
        }
        if dirty {
            let status = Status {
                now,
                paused,
                speed: SPEEDS[speed].1,
            };
            let drawn = terminal.draw(|frame| {
                session.draw(mode, &mut watch, &status, frame.buffer_mut());
            })?;
            dirty = matches!(drawn, Drawn::Held);
        }
        let mut wake = if paused {
            now + Duration::from_secs(1)
        } else {
            next_tick
        };
        if let Some(end) = watch.next_flash_end() {
            wake = wake.min(end);
        }
        let wait = if dirty {
            Duration::from_millis(5)
        } else {
            wake.saturating_duration_since(Instant::now())
        };
        let Some(event) = events.next_timeout(wait)? else {
            continue;
        };
        let page = i64::from(terminal.size().height.saturating_sub(4).max(1));
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
                        watch.input = true;
                    }
                    KeyCode::Char(' ') => paused = !paused,
                    KeyCode::Char('+' | '=') => speed = (speed + 1).min(SPEEDS.len() - 1),
                    KeyCode::Char('-' | '_') => speed = speed.saturating_sub(1),
                    KeyCode::Char('r') => watch.reset(),
                    code => {
                        match code {
                            KeyCode::Up => session.scroll_by(mode, -1),
                            KeyCode::Down => session.scroll_by(mode, 1),
                            KeyCode::PageUp => session.scroll_by(mode, -page),
                            KeyCode::PageDown => session.scroll_by(mode, page),
                            KeyCode::Home => session.go_to_start(mode),
                            KeyCode::End => session.follow(mode),
                            _ => continue,
                        }
                        watch.input = true;
                    }
                }
            }
            Event::Mouse(mouse) => {
                match mouse.kind {
                    MouseEventKind::ScrollUp => session.scroll_by(mode, -3),
                    MouseEventKind::ScrollDown => session.scroll_by(mode, 3),
                    _ => continue,
                }
                watch.input = true;
            }
            Event::Resize(_) | Event::Resumed => watch.input = true,
            _ => continue,
        }
        dirty = true;
    }
    Ok(terminal.close()?)
}

#[cfg(not(any(unix, windows)))]
fn main() {}
