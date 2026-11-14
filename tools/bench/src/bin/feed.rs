//! What a feed costs at 1,000, 100,000 and 1,000,000 items: a wheel step of three rows and a
//! frame after a new width, for the scroll core alone and for `FeedView` wrapping and drawing real
//! text, and the bytes a wheel step writes with and without scroll regions.
//!
//! ```sh
//! cargo run --release -p bench --bin feed
//! ```
//!
//! The core alone measures an item by a table lookup, so its figures are the core's own cost.
//! The view wraps each item's text with `termelon::text::wrap` the first time it's shown at a
//! width and keeps the rows until the width changes.

use std::collections::HashMap;
use std::hint::black_box;
use std::io::{self, Write};
use std::ops::Range;
use std::time::Instant;

use termelon::caps::{Capabilities, Support};
use termelon::feed::{Feed, Measure};
use termelon::layout::Rect;
use termelon::text::{self, Line};
use termelon::unstable::Encoder;
use termelon::{Buffer, FeedRows, FeedView, Style};

#[expect(clippy::disallowed_methods, reason = "this tool prints its report")]
fn stdout() -> io::Stdout {
    io::stdout()
}

struct Rng(u64);

impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n.max(1)
    }
}

/// An item's word count, from its key, as a transcript's items have them: mostly a few words, now
/// and then a long reply or output.
fn word_count(key: u64) -> u64 {
    let mut rng = Rng(key.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    match rng.below(10) {
        0 => 150 + rng.below(400),
        1..=3 => 20 + rng.below(60),
        _ => 1 + rng.below(12),
    }
}

/// True heights by key, for the core alone.
struct Table(Vec<u32>);

impl Measure<u64> for Table {
    fn height(&mut self, key: &u64) -> u32 {
        self.0[usize::try_from(*key).unwrap_or(0)]
    }
}

/// Items of text made from their keys, wrapped at the view's width when first measured.
#[derive(Default)]
struct Wrapped {
    width: u16,
    rows: HashMap<u64, Vec<String>>,
    wraps: u64,
}

impl Wrapped {
    fn rows(&mut self, key: u64) -> &[String] {
        let width = self.width;
        let wraps = &mut self.wraps;
        self.rows.entry(key).or_insert_with(|| {
            *wraps += 1;
            let text: Vec<String> = (0..word_count(key))
                .map(|n| format!("i{key}w{n}"))
                .collect();
            let text = text.join(" ");
            let line = Line::from(text.as_str());
            text::wrap(&line, width, true)
                .iter()
                .map(|row| row.spans.iter().map(|span| span.content.as_ref()).collect())
                .collect()
        })
    }
}

impl Measure<u64> for Wrapped {
    fn height(&mut self, key: &u64) -> u32 {
        u32::try_from(self.rows(*key).len()).unwrap_or(u32::MAX)
    }

    fn mark_at(&mut self, key: &u64, row: u32) -> Option<u64> {
        // The words before the row.
        let rows = self.rows(*key);
        let before = rows.get(..row as usize)?;
        Some(before.iter().map(|r| r.split(' ').count() as u64).sum())
    }

    fn resolve(&mut self, key: &u64, mark: u64) -> Option<u32> {
        let mut seen = 0;
        for (row, text) in self.rows(*key).iter().enumerate() {
            seen += text.split(' ').count() as u64;
            if mark < seen {
                return u32::try_from(row).ok();
            }
        }
        None
    }
}

impl FeedRows<u64> for Wrapped {
    fn set_width(&mut self, width: u16) {
        self.width = width;
        self.rows.clear();
    }

    fn render(&mut self, key: &u64, rows: Range<u32>, area: Rect, buf: &mut Buffer) {
        let shown = self.rows(*key);
        let shown = shown
            .get(rows.start as usize..rows.end as usize)
            .unwrap_or(&[]);
        for (y, row) in (area.y..).zip(shown) {
            buf.set_string(area.x, y, row, Style::new());
        }
    }
}

/// Microseconds at the given shares of the sorted samples, in nanoseconds.
fn at(samples: &mut [u64], shares: &[f64]) -> Vec<f64> {
    samples.sort_unstable();
    shares
        .iter()
        .map(|share| {
            #[expect(
                clippy::cast_precision_loss,
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "an index into the samples from a share of their count"
            )]
            let i = ((samples.len() - 1) as f64 * share) as usize;
            #[expect(clippy::cast_precision_loss, reason = "nanoseconds shown in µs")]
            let micros = samples[i] as f64 / 1000.;
            micros
        })
        .collect()
}

fn nanos(start: Instant) -> u64 {
    u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

fn show(values: &[f64]) -> String {
    values
        .iter()
        .map(|v| format!("{v:.2}"))
        .collect::<Vec<_>>()
        .join(" / ")
}

const WIDTH: u16 = 120;
const HEIGHT: u16 = 50;
const STEPS: usize = 6_000;
const WIDTHS: [u16; 6] = [100, 140, 90, 120, 80, 160];

fn main() {
    let mut w = stdout().lock();
    let _ = write!(
        w,
        "feed bench, release build, {WIDTH} by {HEIGHT}, times in µs as p50 / p99 / max\n\n"
    );
    for n in [1_000u64, 100_000, 1_000_000] {
        // (A short feed takes fewer steps, so they don't run into its start.)
        let steps = STEPS.min(usize::try_from(n).unwrap_or(STEPS));
        // Items come at the end, one at a time, three times over. A push slow in every round
        // is slow by itself, and one slow in a single round was the machine's.
        let mut fastest = vec![u64::MAX; usize::try_from(n).unwrap_or(0)];
        let mut push = u64::MAX;
        let mut feed = Feed::new();
        for _ in 0..3 {
            feed = Feed::new();
            let start = Instant::now();
            for (key, best) in (0..n).zip(&mut fastest) {
                let one = Instant::now();
                feed.push(key, 2);
                *best = (*best).min(nanos(one));
            }
            push = push.min(nanos(start) / n);
        }
        let worst = fastest.iter().max().copied().unwrap_or(0);
        let _ = writeln!(
            w,
            "{n} items: push {push} ns on average, the slowest {worst} ns (each push's fastest of three)"
        );
        // Items that come away from the end, at the start or before a status line kept last.
        let mut front = Feed::new();
        let start = Instant::now();
        for key in 0..n {
            front.push_front(key, 2);
        }
        let front = nanos(start) / n;
        let mut before = Feed::new();
        let status = before.push(u64::MAX, 1);
        let start = Instant::now();
        for key in 0..n {
            before.insert_before(status, key, 2);
        }
        let before = nanos(start) / n;
        let _ = writeln!(
            w,
            "  push_front {front} ns on average, insert_before the last item {before} ns"
        );
        // The core alone, measuring by table lookup.
        let mut table = Table(
            (0..n)
                .map(|key| u32::try_from(word_count(key) * 7 / u64::from(WIDTH) + 1).unwrap_or(1))
                .collect(),
        );
        feed.layout(u32::from(HEIGHT), &mut table);
        let mut wheel = Vec::with_capacity(steps * 3);
        for rows in [-3i64, 3, -3] {
            for _ in 0..steps {
                feed.scroll_by(rows);
                let start = Instant::now();
                black_box(feed.layout(u32::from(HEIGHT), &mut table));
                wheel.push(nanos(start));
            }
        }
        let mut width = Vec::new();
        for _ in 0..200 {
            let start = Instant::now();
            feed.set_geometry();
            black_box(feed.layout(u32::from(HEIGHT), &mut table));
            width.push(nanos(start));
        }
        // Idle time measures the rest, in slices of 256.
        let start = Instant::now();
        let mut swept = 0;
        loop {
            let done = feed.sweep(256, &mut table);
            swept += done;
            if done < 256 {
                break;
            }
        }
        let sweep = nanos(start) / u64::try_from(swept.max(1)).unwrap_or(1);
        let _ = write!(
            w,
            "  core, wheel step: {}\n  core, frame after a new width: {}\n  core, idle measuring: {sweep} ns an item\n",
            show(&at(&mut wheel, &[0.5, 0.99, 1.])),
            show(&at(&mut width, &[0.5, 0.99, 1.]))
        );
        drop(feed);
        drop(table);

        // The view, wrapping and drawing real text into a buffer.
        let mut view = FeedView::new();
        for key in 0..n {
            view.feed_mut().push(key, 2);
        }
        let mut rows = Wrapped::default();
        let area = Rect::new(0, 0, WIDTH, HEIGHT);
        let mut buffer = Buffer::empty(area);
        view.render(area, &mut buffer, &mut rows);
        let mut first = Vec::with_capacity(steps);
        let mut again = Vec::with_capacity(steps);
        for (round, step) in [-3i64, 3, -3].into_iter().enumerate() {
            for _ in 0..steps {
                view.feed_mut().scroll_by(step);
                buffer.reset();
                let start = Instant::now();
                view.render(area, &mut buffer, &mut rows);
                let took = nanos(start);
                match round {
                    0 => first.push(took),
                    2 => again.push(took),
                    _ => {}
                }
            }
        }
        let _ = write!(
            w,
            "  view, wheel step over rows never wrapped: {}\n  view, wheel step over rows wrapped before: {}\n",
            show(&at(&mut first, &[0.5, 0.99, 1.])),
            show(&at(&mut again, &[0.5, 0.99, 1.]))
        );
        let mut width = Vec::new();
        let mut wraps = 0;
        for _ in 0..10 {
            for columns in WIDTHS {
                let area = Rect::new(0, 0, columns, HEIGHT);
                let mut buffer = Buffer::empty(area);
                let before = rows.wraps;
                let start = Instant::now();
                view.render(area, &mut buffer, &mut rows);
                width.push(nanos(start));
                wraps = wraps.max(rows.wraps - before);
            }
        }
        let _ = write!(
            w,
            "  view, frame after a new width: {} (at most {wraps} items wrapped)\n\n",
            show(&at(&mut width, &[0.5, 0.99, 1.]))
        );
    }

    // Bytes: a wheel step at 120 by 40 through the encoder, with and without scroll regions.
    for regions in [Support::Yes, Support::No] {
        let mut caps = Capabilities::default();
        caps.scroll_regions = regions;
        caps.synchronized_output = Support::Yes;
        let mut view = FeedView::new();
        for key in 0..100_000u64 {
            view.feed_mut().push(key, 2);
        }
        let mut rows = Wrapped::default();
        let area = Rect::new(0, 0, 120, 40);
        let (mut prev, mut next) = (Buffer::empty(area), Buffer::empty(area));
        let mut encoder = Encoder::new();
        let mut bytes = Vec::new();
        let mut times = Vec::new();
        let mut packet = Vec::new();
        view.render(area, &mut prev, &mut rows);
        encoder.encode(&Buffer::empty(area), &prev, None, &caps, &mut packet);
        for step in 0..2_000 {
            view.feed_mut().scroll_by(if step < 1_000 { -3 } else { 3 });
            next.reset();
            view.render(area, &mut next, &mut rows);
            packet.clear();
            let start = Instant::now();
            encoder.encode(&prev, &next, None, &caps, &mut packet);
            times.push(nanos(start));
            bytes.push(packet.len());
            std::mem::swap(&mut prev, &mut next);
        }
        let label = if regions == Support::Yes {
            "with scroll regions"
        } else {
            "without"
        };
        bytes.sort_unstable();
        let _ = writeln!(
            w,
            "a 3-row step at 120 by 40 {label}: {} bytes at the median, {} at most, encoded in {} µs",
            bytes[bytes.len() / 2],
            bytes[bytes.len() - 1],
            show(&at(&mut times, &[0.5, 0.99]))
        );
    }
}
