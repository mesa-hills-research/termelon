//! The feed's rules, checked on seeded random feeds against a list kept the plain way, with
//! every height known.

use std::collections::HashMap;

use super::*;

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }

    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn index(&mut self, len: usize) -> usize {
        usize::try_from(self.below(len as u64)).unwrap()
    }

    fn small(&mut self, n: u64) -> u32 {
        u32::try_from(self.below(n)).unwrap()
    }

    fn signed(&mut self, n: u64) -> i64 {
        i64::try_from(self.below(n)).unwrap()
    }

    /// A height as a transcript's items have them, mostly a few rows, now and then a long
    /// output, sometimes none (a hidden item).
    fn height(&mut self) -> u32 {
        match self.below(20) {
            0 => 0,
            1 => 40 + self.small(200),
            2..=5 => 6 + self.small(20),
            _ => 1 + self.small(5),
        }
    }
}

/// The items as they really are, their order and every true height, keyed by the number the
/// feed's key holds.
#[derive(Default)]
struct World {
    order: Vec<u64>,
    heights: HashMap<u64, u32>,
    ids: HashMap<u64, ItemId>,
    next_key: u64,
    calls: u64,
}

impl Measure<u64> for World {
    fn height(&mut self, key: &u64) -> u32 {
        self.calls += 1;
        self.heights[key]
    }
}

impl World {
    /// The true distance from the start to the item's top.
    fn top_of(&self, key: u64) -> u64 {
        let mut at = 0;
        for k in &self.order {
            if *k == key {
                return at;
            }
            at += u64::from(self.heights[k]);
        }
        panic!("{key} isn't in the world");
    }

    fn total(&self) -> u64 {
        self.order
            .iter()
            .map(|key| u64::from(self.heights[key]))
            .sum()
    }

    fn new_key(&mut self, height: u32) -> u64 {
        let key = self.next_key;
        self.next_key += 1;
        self.heights.insert(key, height);
        key
    }

    fn push(&mut self, feed: &mut Feed<u64>, height: u32, hint: u32) -> u64 {
        let key = self.new_key(height);
        self.order.push(key);
        self.ids.insert(key, feed.push(key, hint));
        key
    }

    fn remove(&mut self, feed: &mut Feed<u64>, at: usize) -> u64 {
        let key = self.order.remove(at);
        self.heights.remove(&key);
        assert_eq!(feed.remove(self.ids.remove(&key).unwrap()), Some(key));
        key
    }

    fn id(&self, key: u64) -> ItemId {
        self.ids[&key]
    }
}

/// A feed and its world of `n` items, with hints from `hint`.
fn build(n: usize, rng: &mut Rng, hint: &mut dyn FnMut(u32) -> u32) -> (Feed<u64>, World) {
    let mut world = World::default();
    let mut feed = Feed::new();
    for _ in 0..n {
        let height = rng.height();
        let hint = hint(height);
        world.push(&mut feed, height, hint);
    }
    (feed, world)
}

fn key_of(feed: &Feed<u64>, placed: &Placed) -> u64 {
    *feed.key(placed.item).unwrap()
}

/// The true distance from the start of the viewport's first row.
fn true_top(feed: &Feed<u64>, frame: &Placement, world: &World) -> i64 {
    let first = &frame.items[0];
    i64::try_from(world.top_of(key_of(feed, first))).unwrap() - first.y
}

/// The frame's items are the world's that have rows, in order, at their true heights, each
/// under the one before, and they cover the viewport or end with the feed.
fn assert_exact(feed: &Feed<u64>, frame: &Placement, world: &World, viewport: u32) {
    let v = i64::from(viewport);
    if frame.items.is_empty() {
        assert!(world.order.iter().all(|key| world.heights[key] == 0));
        assert_eq!(frame.gap, viewport);
        return;
    }
    let first = &frame.items[0];
    assert!(first.y <= 0, "the first item starts at or above row 0");
    let first_key = key_of(feed, first);
    let at = world
        .order
        .iter()
        .position(|key| *key == first_key)
        .unwrap();
    let rest: Vec<u64> = world.order[at..]
        .iter()
        .copied()
        .filter(|key| world.heights[key] > 0)
        .collect();
    let mut y = first.y;
    for (placed, key) in frame.items.iter().zip(&rest) {
        assert_eq!(key_of(feed, placed), *key);
        assert_eq!(placed.height, world.heights[key]);
        assert_eq!(placed.y, y);
        assert!(placed.y < v);
        y += i64::from(placed.height);
    }
    if y < v {
        assert_eq!(
            frame.items.len(),
            rest.len(),
            "rows are blank only past the end"
        );
        assert_eq!(i64::from(frame.gap), v - y);
    } else {
        assert_eq!(frame.gap, 0);
    }
}

#[test]
fn a_scroll_moves_the_view_by_exactly_what_was_asked() {
    for seed in 1..=100 {
        let mut rng = Rng::new(seed);
        let n = 50 + rng.index(1500);
        let viewport = 5 + rng.small(60);
        // Hints that are wrong in every way, none, the truth, and too much.
        let mut hints = Rng::new(seed + 1000);
        let (mut feed, mut world) = build(n, &mut rng, &mut |h| match hints.below(3) {
            0 => 0,
            1 => h,
            _ => h * 3 + 7,
        });
        let total = i64::try_from(world.total()).unwrap();
        let range = (total - i64::from(viewport)).max(0);
        let frame = feed.layout(viewport, &mut world);
        assert_exact(&feed, &frame, &world, viewport);
        assert!(frame.following);
        let mut top = true_top(&feed, &frame, &world);
        assert_eq!(top, range);
        for _ in 0..1000 {
            let delta = match rng.below(10) {
                0 => rng.signed(4000) - 2000,
                1 => i64::from(viewport),
                2 => -i64::from(viewport),
                _ => rng.signed(13) - 6,
            };
            feed.scroll_by(delta);
            let frame = feed.layout(viewport, &mut world);
            assert_exact(&feed, &frame, &world, viewport);
            let wanted = (top + delta).clamp(0, range);
            top = true_top(&feed, &frame, &world);
            assert_eq!(top, wanted, "seed {seed}, delta {delta}");
            // Reaching the end by scrolling down follows it, and nothing else does.
            if frame.following {
                assert_eq!(top, range);
            }
            if delta > 0 && top == range {
                assert!(frame.following);
            }
            if delta < 0 && range > 0 && wanted < range {
                assert!(!frame.following);
            }
        }
    }
}

/// One step of a seeded session, an edit away from the reader's row, a measurement, or
/// nothing.
fn edit_elsewhere(
    feed: &mut Feed<u64>,
    world: &mut World,
    rng: &mut Rng,
    top_key: u64,
    hint: &mut dyn FnMut(u32) -> u32,
) {
    match rng.below(9) {
        // An item comes at the end.
        0 | 1 => {
            let height = rng.height();
            let hint = hint(height);
            world.push(feed, height, hint);
        }
        // One comes somewhere else, before or after the reader.
        2 => {
            let at = rng.index(world.order.len());
            let height = rng.height();
            let key = world.new_key(height);
            let before = world.id(world.order[at]);
            world.order.insert(at, key);
            let id = feed.insert_before(before, key, hint(height));
            world.ids.insert(key, id);
        }
        // Older ones come at the start.
        3 => {
            let height = rng.height();
            let key = world.new_key(height);
            world.order.insert(0, key);
            let id = feed.push_front(key, hint(height));
            world.ids.insert(key, id);
        }
        // One that isn't the reader's goes.
        4 => {
            let at = rng.index(world.order.len());
            if world.order[at] != top_key {
                world.remove(feed, at);
            }
        }
        // One that isn't the reader's changes its height.
        5 | 6 => {
            let at = rng.index(world.order.len());
            let key = world.order[at];
            if key != top_key {
                world.heights.insert(key, rng.height());
                feed.changed(world.id(key), Change::Reflowed);
            }
        }
        // Rows come at the end of the reader's own item.
        7 => {
            *world.heights.get_mut(&top_key).unwrap() += rng.small(4);
            feed.changed(world.id(top_key), Change::Appended);
        }
        // Idle time measures some of the rest.
        _ => {
            feed.sweep(1 + rng.index(40), world);
        }
    }
}

#[test]
fn nothing_that_changes_elsewhere_moves_the_readers_row() {
    for seed in 1..=100 {
        let mut rng = Rng::new(seed);
        let n = 200 + rng.index(2000);
        let viewport = 5 + rng.small(60);
        let mut hints = Rng::new(seed + 7);
        let mut hint = move |h: u32| match hints.below(3) {
            0 => 0,
            1 => h,
            _ => h * 2 + 3,
        };
        let (mut feed, mut world) = build(n, &mut rng, &mut hint);
        feed.layout(viewport, &mut world);
        // The reader scrolls up to somewhere in the feed.
        feed.scroll_by(-(1 + i64::try_from(rng.below(world.total() / 2)).unwrap()));
        let frame = feed.layout(viewport, &mut world);
        assert!(!frame.following);
        let top = (key_of(&feed, &frame.items[0]), frame.items[0].y);
        for step in 0..1000 {
            edit_elsewhere(&mut feed, &mut world, &mut rng, top.0, &mut hint);
            let frame = feed.layout(viewport, &mut world);
            assert_exact(&feed, &frame, &world, viewport);
            assert_eq!(
                (key_of(&feed, &frame.items[0]), frame.items[0].y),
                top,
                "seed {seed}, step {step}"
            );
            assert!(!frame.following);
            assert_eq!(frame.outcome, Outcome::Kept);
        }
    }
}

/// A frame as keys, rows and heights, its gap, whether it follows, and its outcome.
type Seen = (Vec<(u64, i64, u32)>, u32, bool, Outcome);

#[test]
fn hints_never_decide_what_is_on_screen() {
    for seed in 1..=32 {
        let run = |hint_kind: u64| -> Vec<Seen> {
            let mut rng = Rng::new(seed);
            let mut hints = Rng::new(seed * 31 + hint_kind);
            let mut hint = move |h: u32| match hint_kind {
                0 => 0,
                1 => h,
                2 => 1000,
                _ => hints.small(60),
            };
            let n = 300 + rng.index(1500);
            let viewport = 10 + rng.small(50);
            let (mut feed, mut world) = build(n, &mut rng, &mut hint);
            let mut frames = Vec::new();
            let mut frame = feed.layout(viewport, &mut world);
            for _ in 0..800 {
                let top_key = key_of(&feed, &frame.items[0]);
                match rng.below(4) {
                    0 => feed.scroll_by(rng.signed(61) - 30),
                    1 => feed.set_geometry(),
                    _ => edit_elsewhere(&mut feed, &mut world, &mut rng, top_key, &mut hint),
                }
                frame = feed.layout(viewport, &mut world);
                // (How many it measured depends on what the sweep did before, so it's left
                // out.)
                let items = frame
                    .items
                    .iter()
                    .map(|p| (key_of(&feed, p), p.y, p.height))
                    .collect();
                frames.push((items, frame.gap, frame.following, frame.outcome));
            }
            frames
        };
        let exact = run(1);
        for kind in [0, 2, 3] {
            assert!(run(kind) == exact, "seed {seed}, hints of kind {kind}");
        }
    }
}

/// Every item on screen in both frames is at or above its row in the first, but an item whose
/// row at the top went with its last rows, which has no row to keep.
fn assert_nothing_moved_down(feed: &Feed<u64>, before: &Placement, after: &Placement, at: &str) {
    for placed in &after.items {
        if let Some(was) = before.items.iter().find(|was| was.item == placed.item)
            && i64::from(placed.height) > -was.y
        {
            assert!(
                placed.y <= was.y,
                "{at}: item {} went from row {} to {}",
                key_of(feed, placed),
                was.y,
                placed.y
            );
        }
    }
}

#[test]
fn following_never_moves_a_row_down_with_a_gap_limit() {
    for seed in 1..=100 {
        let mut rng = Rng::new(seed);
        let n = 30 + rng.index(300);
        let viewport = 8 + rng.small(50);
        let (mut feed, mut world) = build(n, &mut rng, &mut |h| h);
        feed.set_gap_limit(u32::MAX);
        let mut before = feed.layout(viewport, &mut world);
        assert!(before.following);
        for step in 0..1000 {
            let last = *world.order.last().unwrap();
            match rng.below(6) {
                // The last item grows, as a reply does while it streams.
                0..=2 => {
                    *world.heights.get_mut(&last).unwrap() += 1 + rng.small(3);
                    feed.changed(world.id(last), Change::Appended);
                }
                // It gets shorter, as text does when its markup closes.
                3 => {
                    let height = world.heights.get_mut(&last).unwrap();
                    *height = height.saturating_sub(1 + rng.small(3));
                    feed.changed(world.id(last), Change::Reflowed);
                }
                // An item comes.
                4 => {
                    let height = 1 + rng.small(6);
                    world.push(&mut feed, height, 0);
                }
                // The last one goes, as a status line does.
                _ => {
                    if world.order.len() > 2 {
                        world.remove(&mut feed, world.order.len() - 1);
                    }
                }
            }
            let frame = feed.layout(viewport, &mut world);
            assert_exact(&feed, &frame, &world, viewport);
            assert!(frame.following);
            assert_nothing_moved_down(&feed, &before, &frame, &format!("seed {seed}, step {step}"));
            before = frame;
        }
        // Going to the end closes the blank rows.
        feed.follow();
        let frame = feed.layout(viewport, &mut world);
        if world.total() >= u64::from(viewport) {
            assert_eq!(frame.gap, 0);
        }
    }
}

#[test]
fn following_without_a_gap_limit_keeps_the_end_on_the_last_row() {
    for seed in 1..=30 {
        let mut rng = Rng::new(seed);
        let (mut feed, mut world) = build(200, &mut rng, &mut |_| 0);
        let viewport = 8 + rng.small(40);
        for _ in 0..500 {
            let last = *world.order.last().unwrap();
            match rng.below(3) {
                0 => {
                    let height = world.heights.get_mut(&last).unwrap();
                    *height = height.saturating_sub(1 + rng.small(3));
                    feed.changed(world.id(last), Change::Reflowed);
                }
                1 if world.order.len() > 2 => {
                    world.remove(&mut feed, world.order.len() - 1);
                }
                _ => {
                    let height = 1 + rng.small(4);
                    world.push(&mut feed, height, 0);
                }
            }
            let frame = feed.layout(viewport, &mut world);
            assert_exact(&feed, &frame, &world, viewport);
            assert!(frame.following);
            if world.total() >= u64::from(viewport) {
                assert_eq!(frame.gap, 0);
                assert_eq!(
                    true_top(&feed, &frame, &world),
                    i64::try_from(world.total()).unwrap() - i64::from(viewport)
                );
            }
        }
    }
}

#[test]
fn following_a_feed_that_only_grows_keeps_its_end_on_the_last_row() {
    let mut rng = Rng::new(5);
    let (mut feed, mut world) = build(400, &mut rng, &mut |_| 0);
    feed.set_gap_limit(3);
    for _ in 0..4000 {
        let last = *world.order.last().unwrap();
        if rng.below(3) == 0 {
            let height = 1 + rng.small(4);
            world.push(&mut feed, height, 0);
        } else {
            *world.heights.get_mut(&last).unwrap() += 1;
            feed.changed(world.id(last), Change::Appended);
        }
        let frame = feed.layout(30, &mut world);
        assert_exact(&feed, &frame, &world, 30);
        assert!(frame.following);
        assert_eq!(frame.gap, 0);
        assert_eq!(
            true_top(&feed, &frame, &world),
            i64::try_from(world.total()).unwrap() - 30
        );
    }
}

#[test]
fn a_layout_measures_only_what_it_crosses() {
    let mut rng = Rng::new(9);
    let mut world = World::default();
    let mut feed = Feed::new();
    for _ in 0..100_000 {
        let height = 1 + rng.small(8);
        world.push(&mut feed, height, 3);
    }
    let viewport = 50;
    // Opening at the end measures a screen of items.
    let frame = feed.layout(viewport, &mut world);
    assert!(frame.measured <= viewport + 1, "{}", frame.measured);
    // A step of the wheel measures at most the rows it crosses.
    for _ in 0..2000 {
        feed.scroll_by(-3);
        let frame = feed.layout(viewport, &mut world);
        assert!(frame.measured <= 3, "{}", frame.measured);
    }
    // Another width makes every height a hint, and the next frame measures the screen's items
    // again and no others.
    let calls = world.calls;
    feed.set_geometry();
    assert_eq!(world.calls, calls);
    let frame = feed.layout(viewport, &mut world);
    assert!(frame.measured <= viewport + 1, "{}", frame.measured);
    assert!(!feed.extent().exact);
    // Idle time measures the rest, and the view doesn't move.
    let top = (frame.items[0].item, frame.items[0].y);
    while feed.sweep(500, &mut world) == 500 {
        let frame = feed.layout(viewport, &mut world);
        assert_eq!((frame.items[0].item, frame.items[0].y), top);
        assert_eq!(frame.measured, 0);
    }
    assert!(feed.extent().exact);
    assert_eq!(feed.extent().total, world.total());
    let frame = feed.layout(viewport, &mut world);
    assert_eq!(
        i64::try_from(feed.offset(viewport)).unwrap(),
        true_top(&feed, &frame, &world)
    );
}

#[test]
fn the_readers_item_going_leaves_them_at_its_neighbour() {
    let mut rng = Rng::new(3);
    let (mut feed, mut world) = build(500, &mut rng, &mut |_| 1);
    feed.layout(20, &mut world);
    feed.scroll_by(-300);
    let mut frame = feed.layout(20, &mut world);
    while frame.items.len() < 2 {
        feed.scroll_by(-7);
        frame = feed.layout(20, &mut world);
    }
    let gone = key_of(&feed, &frame.items[0]);
    let next = key_of(&feed, &frame.items[1]);
    let at = world.order.iter().position(|key| *key == gone).unwrap();
    world.remove(&mut feed, at);
    let frame = feed.layout(20, &mut world);
    assert_exact(&feed, &frame, &world, 20);
    assert_eq!(frame.outcome, Outcome::Replaced);
    // Items of no rows don't hold the top row.
    let first_shown = world.order[world.order.iter().position(|k| *k == next).unwrap()..]
        .iter()
        .find(|key| world.heights[key] > 0)
        .unwrap();
    assert_eq!(key_of(&feed, &frame.items[0]), *first_shown);
    // The next frame says nothing more.
    assert_eq!(feed.layout(20, &mut world).outcome, Outcome::Kept);
}

#[test]
fn a_fold_keeps_the_row_that_was_clicked() {
    let mut rng = Rng::new(4);
    let (mut feed, mut world) = build(800, &mut rng, &mut |_| 2);
    feed.layout(40, &mut world);
    feed.scroll_by(-500);
    let frame = feed.layout(40, &mut world);
    // The reader clicks the first row of an item in the middle of the screen.
    let clicked = *frame
        .items
        .iter()
        .find(|placed| placed.y >= 12 && placed.height > 0)
        .unwrap();
    let key = key_of(&feed, &clicked);
    // It opens, and so does one above it on the screen.
    *world.heights.get_mut(&key).unwrap() += 30;
    feed.changed(clicked.item, Change::Reflowed);
    let above = frame.items[1].item;
    *world.heights.get_mut(feed.key(above).unwrap()).unwrap() += 9;
    feed.changed(above, Change::Reflowed);
    feed.place(clicked.item, 0, clicked.y);
    let frame = feed.layout(40, &mut world);
    assert_exact(&feed, &frame, &world, 40);
    let now = frame.items.iter().find(|p| p.item == clicked.item).unwrap();
    assert_eq!(now.y, clicked.y);
}

#[test]
fn a_feed_shorter_than_the_viewport_sits_at_the_top_and_follows() {
    let mut world = World::default();
    let mut feed = Feed::new();
    for _ in 0..3 {
        world.push(&mut feed, 2, 0);
    }
    let frame = feed.layout(20, &mut world);
    assert_eq!(
        (frame.items[0].y, frame.gap, frame.following),
        (0, 14, true)
    );
    assert_eq!(frame.outcome, Outcome::Kept);
    // Scrolling up moves nothing, so the reader still follows.
    feed.scroll_by(-3);
    let frame = feed.layout(20, &mut world);
    assert_eq!(
        (frame.items[0].y, frame.gap, frame.following),
        (0, 14, true)
    );
    // It grows past the viewport, and the end stays on the last row.
    for _ in 0..20 {
        world.push(&mut feed, 2, 0);
    }
    let frame = feed.layout(20, &mut world);
    assert_exact(&feed, &frame, &world, 20);
    assert_eq!((frame.gap, frame.following), (0, true));
    assert_eq!(
        true_top(&feed, &frame, &world),
        i64::try_from(world.total()).unwrap() - 20
    );
}

#[test]
fn an_empty_feed_follows_and_shows_nothing() {
    let mut world = World::default();
    let mut feed: Feed<u64> = Feed::new();
    feed.scroll_by(-5);
    let frame = feed.layout(10, &mut world);
    assert_eq!(frame.items.len(), 0);
    assert_eq!((frame.gap, frame.following), (10, true));
    let key = world.push(&mut feed, 4, 0);
    let frame = feed.layout(10, &mut world);
    assert_eq!(frame.items.len(), 1);
    assert_eq!(key_of(&feed, &frame.items[0]), key);
    world.remove(&mut feed, 0);
    assert_eq!(feed.layout(10, &mut world).items.len(), 0);
}

/// Text of several source lines, wrapped at a width. A row's mark is its line's id and the
/// byte the row starts at, so a mark keeps naming its text when lines come before it.
struct Text {
    /// Each item's source lines, as an id and a length in columns.
    lines: HashMap<u64, Vec<(u32, u32)>>,
    width: u32,
    next_line: u32,
}

impl Text {
    fn rows(&self, len: u32) -> u32 {
        len.div_ceil(self.width).max(1)
    }

    fn line(&mut self, len: u32) -> (u32, u32) {
        self.next_line += 1;
        (self.next_line, len)
    }
}

impl Measure<u64> for Text {
    fn height(&mut self, key: &u64) -> u32 {
        self.lines[key].iter().map(|(_, len)| self.rows(*len)).sum()
    }

    fn mark_at(&mut self, key: &u64, offset: u32) -> Option<u64> {
        let mut row = 0;
        for (id, len) in &self.lines[key] {
            let rows = self.rows(*len);
            if offset < row + rows {
                let byte = (offset - row) * self.width;
                return Some(u64::from(*id) << 32 | u64::from(byte));
            }
            row += rows;
        }
        None
    }

    fn resolve(&mut self, key: &u64, mark: u64) -> Option<u32> {
        let id = u32::try_from(mark >> 32).unwrap();
        let byte = u32::try_from(mark & u64::from(u32::MAX)).unwrap();
        let mut row = 0;
        for (line, len) in &self.lines[key] {
            if *line == id {
                return Some(row + byte / self.width);
            }
            row += self.rows(*len);
        }
        None
    }
}

fn text_feed(rng: &mut Rng) -> (Feed<u64>, Text) {
    let mut text = Text {
        lines: HashMap::new(),
        width: 80,
        next_line: 0,
    };
    let mut feed = Feed::new();
    for key in 0..300u64 {
        let lines = (0..=rng.below(30))
            .map(|_| {
                let len = 1 + rng.small(400);
                text.line(len)
            })
            .collect();
        text.lines.insert(key, lines);
        feed.push(key, 0);
    }
    (feed, text)
}

#[test]
fn another_width_keeps_the_text_at_the_top() {
    for seed in 1..=40 {
        let mut rng = Rng::new(seed);
        let (mut feed, mut text) = text_feed(&mut rng);
        feed.layout(40, &mut text);
        feed.scroll_by(-(200 + rng.signed(3000)));
        feed.layout(40, &mut text);
        for round in 0..30 {
            let Anchor::At { item, mark, .. } = feed.anchor() else {
                panic!("the reader scrolled up");
            };
            let mark = mark.unwrap();
            text.width = 20 + rng.small(160);
            feed.set_geometry();
            // Idle time may measure the reader's item before the next frame, which changes
            // nothing.
            if round % 2 == 1 {
                feed.sweep(5, &mut text);
            }
            let frame = feed.layout(40, &mut text);
            // The byte that started the top row is in the top row still.
            assert_eq!(frame.items[0].item, item);
            let Anchor::At { offset, .. } = feed.anchor() else {
                panic!("the reader hasn't moved");
            };
            let key = *feed.key(item).unwrap();
            assert_eq!(text.resolve(&key, mark), Some(offset), "seed {seed}");
            assert_eq!(i64::from(offset), -frame.items[0].y);
        }
    }
}

#[test]
fn widths_one_after_another_keep_the_first_text_at_the_top() {
    for seed in 1..=40 {
        let mut rng = Rng::new(seed);
        let (mut feed, mut text) = text_feed(&mut rng);
        feed.layout(40, &mut text);
        feed.scroll_by(-(200 + rng.signed(3000)));
        feed.layout(40, &mut text);
        let Anchor::At { item, mark, .. } = feed.anchor() else {
            panic!("the reader scrolled up");
        };
        let (key, mark) = (*feed.key(item).unwrap(), mark.unwrap());
        // A window's edge dragged across many widths, a column at a time.
        for width in (30..=120).rev().chain(31..=90) {
            text.width = width;
            feed.set_geometry();
            let frame = feed.layout(40, &mut text);
            assert_eq!(frame.items[0].item, item, "seed {seed}");
            assert_eq!(
                text.resolve(&key, mark),
                Some(u32::try_from(-frame.items[0].y).unwrap()),
                "seed {seed} at {width}: the first byte isn't in the top row"
            );
        }
    }
}

#[test]
fn a_reflow_of_the_readers_item_keeps_its_text_at_the_top() {
    let mut rng = Rng::new(11);
    let (mut feed, mut text) = text_feed(&mut rng);
    feed.layout(40, &mut text);
    feed.scroll_by(-1500);
    let frame = feed.layout(40, &mut text);
    let item = frame.items[0].item;
    let Anchor::At { mark, .. } = feed.anchor() else {
        panic!("the reader scrolled up");
    };
    let key = *feed.key(item).unwrap();
    // A line opens above the top row, in the same item, as a fold does.
    let line = text.line(500);
    text.lines.get_mut(&key).unwrap().insert(0, line);
    feed.changed(item, Change::Reflowed);
    let frame = feed.layout(40, &mut text);
    assert_eq!(frame.items[0].item, item);
    assert_eq!(
        text.resolve(&key, mark.unwrap()),
        Some(u32::try_from(-frame.items[0].y).unwrap())
    );
    assert!(
        frame.items[0].y < -6,
        "the line that opened is above the screen"
    );
}

#[test]
fn rows_coming_or_going_below_the_readers_row_leave_it_still() {
    // Keeping the same share of a block would move the reader in each of these, 30 of 40
    // becoming 31 of 42 and 29 of 39.
    for (old, new) in [(40, 42), (40, 41), (40, 39), (10, 210)] {
        let mut world = World::default();
        let mut feed = Feed::new();
        for height in [50, old, 50] {
            world.push(&mut feed, height, 0);
        }
        let item = world.id(1);
        feed.layout(20, &mut world);
        feed.place(item, 5, 0);
        let frame = feed.layout(20, &mut world);
        assert_eq!((frame.items[0].item, frame.items[0].y), (item, -5));
        world.heights.insert(1, new);
        feed.changed(item, Change::Appended);
        let frame = feed.layout(20, &mut world);
        assert_eq!((frame.items[0].item, frame.items[0].y), (item, -5));
    }
}

#[test]
fn a_shorter_end_leaves_blank_rows_up_to_the_limit() {
    let mut world = World::default();
    let mut feed = Feed::new();
    for _ in 0..40 {
        world.push(&mut feed, 3, 0);
    }
    // A status line of three rows at the end.
    world.push(&mut feed, 3, 0);
    feed.set_gap_limit(3);
    let before = feed.layout(20, &mut world);
    // It goes, and the rows above it stay where they are.
    world.remove(&mut feed, world.order.len() - 1);
    let frame = feed.layout(20, &mut world);
    assert_eq!(frame.gap, 3);
    assert_eq!(frame.items[0], before.items[0]);
    // Text comes in its place, two rows and then four more. The first two fill blank rows, and
    // the rest scroll.
    let text = world.push(&mut feed, 2, 0);
    let frame = feed.layout(20, &mut world);
    assert_eq!(frame.gap, 1);
    assert_eq!(frame.items[0], before.items[0]);
    world.heights.insert(text, 6);
    feed.changed(world.id(text), Change::Appended);
    let frame = feed.layout(20, &mut world);
    assert_eq!(frame.gap, 0);
    assert_eq!(
        true_top(&feed, &frame, &world),
        i64::try_from(world.total()).unwrap() - 20
    );
    // More than the limit goes at once, and the end comes down.
    for _ in 0..3 {
        world.remove(&mut feed, world.order.len() - 1);
    }
    let frame = feed.layout(20, &mut world);
    assert_eq!(frame.gap, 0);
    assert_exact(&feed, &frame, &world, 20);
}

#[test]
fn following_again_in_place_keeps_the_rows_where_they_are() {
    let mut world = World::default();
    let mut feed = Feed::new();
    for _ in 0..40 {
        world.push(&mut feed, 3, 0);
    }
    // A status line of three rows at the end.
    world.push(&mut feed, 3, 0);
    feed.set_gap_limit(3);
    let before = feed.layout(20, &mut world);
    // The view holds still while the mouse is down, so the reader is placed where they are.
    let top = before.items[0];
    feed.place(top.item, rows_above(top.y), 0);
    // The status line goes meanwhile, and the rows stay with blank rows below the end.
    world.remove(&mut feed, world.order.len() - 1);
    let frame = feed.layout(20, &mut world);
    assert!(!frame.following);
    assert_eq!((frame.items[0], frame.gap), (top, 3));
    // The button comes up. The reader follows again, and nothing moves.
    feed.follow_in_place();
    let frame = feed.layout(20, &mut world);
    assert!(frame.following);
    assert_eq!((frame.items[0], frame.gap), (top, 3));
    // Text comes, filling the blank rows before it scrolls.
    let text = world.push(&mut feed, 2, 0);
    let frame = feed.layout(20, &mut world);
    assert_eq!((frame.items[0], frame.gap), (top, 1));
    world.heights.insert(text, 6);
    feed.changed(world.id(text), Change::Appended);
    let frame = feed.layout(20, &mut world);
    assert_exact(&feed, &frame, &world, 20);
    assert_eq!(frame.gap, 0);
    // `follow` from such a place brings the end down to the last row instead.
    world.push(&mut feed, 2, 0);
    let frame = feed.layout(20, &mut world);
    let top = frame.items[0];
    feed.place(top.item, rows_above(top.y), 0);
    world.remove(&mut feed, world.order.len() - 1);
    assert_eq!(feed.layout(20, &mut world).gap, 2);
    feed.follow();
    let frame = feed.layout(20, &mut world);
    assert_eq!(frame.gap, 0);
    assert_ne!(frame.items[0], top);
    assert_exact(&feed, &frame, &world, 20);
}

#[test]
fn following_again_in_place_goes_to_the_end_when_it_is_out_of_sight() {
    let mut world = World::default();
    let mut feed = Feed::new();
    for _ in 0..40 {
        world.push(&mut feed, 3, 0);
    }
    feed.set_gap_limit(3);
    feed.layout(20, &mut world);
    // Far above the end.
    feed.scroll_by(-60);
    let up = feed.layout(20, &mut world);
    assert!(!up.following);
    feed.follow_in_place();
    let frame = feed.layout(20, &mut world);
    assert!(frame.following);
    assert_eq!(frame.gap, 0);
    assert_eq!(
        true_top(&feed, &frame, &world),
        i64::try_from(world.total()).unwrap() - 20
    );
    // With more blank rows below the end than the limit, the end comes down too.
    let top = frame.items[0];
    feed.place(top.item, rows_above(top.y), 0);
    for _ in 0..2 {
        world.remove(&mut feed, world.order.len() - 1);
    }
    let held = feed.layout(20, &mut world);
    assert_eq!(held.gap, 6);
    feed.follow_in_place();
    let frame = feed.layout(20, &mut world);
    assert_eq!(frame.gap, 0);
    assert_exact(&feed, &frame, &world, 20);
}

#[test]
fn a_scroll_up_brings_back_the_rows_blank_rows_below_the_end_held_above() {
    let mut world = World::default();
    let mut feed = Feed::new();
    for _ in 0..3 {
        world.push(&mut feed, 3, 0);
    }
    // A status line of two rows, so the feed is a row taller than the viewport.
    world.push(&mut feed, 2, 0);
    feed.set_gap_limit(3);
    let before = feed.layout(10, &mut world);
    assert_eq!(before.items[0].y, -1);
    // It goes. The feed fits now, and its first row stays above the viewport.
    world.remove(&mut feed, 3);
    let held = feed.layout(10, &mut world);
    assert_eq!((held.items[0], held.gap), (before.items[0], 2));
    // A row up shows it, and the reader still follows.
    feed.scroll_by(-1);
    let frame = feed.layout(10, &mut world);
    assert_eq!((frame.items[0].y, frame.gap), (0, 1));
    assert!(frame.following);
    // Nothing is left above to scroll to.
    feed.scroll_by(-1);
    let again = feed.layout(10, &mut world);
    assert_eq!((&again.items, again.gap), (&frame.items, frame.gap));
    assert!(again.following);
}

#[test]
fn the_held_row_survives_its_item_going() {
    let mut world = World::default();
    let mut feed = Feed::new();
    for _ in 0..30 {
        world.push(&mut feed, 4, 0);
    }
    feed.set_gap_limit(10);
    let before = feed.layout(18, &mut world);
    // The item at the top goes and the end gets shorter in the same frame.
    let top = key_of(&feed, &before.items[0]);
    let at = world.order.iter().position(|key| *key == top).unwrap();
    world.remove(&mut feed, at);
    let last = *world.order.last().unwrap();
    world.heights.insert(last, 2);
    feed.changed(world.id(last), Change::Reflowed);
    let frame = feed.layout(18, &mut world);
    assert_exact(&feed, &frame, &world, 18);
    assert!(frame.following);
    assert_nothing_moved_down(&feed, &before, &frame, "after the top item went");
}

#[test]
fn points_order_by_item_then_mark_then_column() {
    let mut world = World::default();
    let mut feed = Feed::new();
    let a = world.push(&mut feed, 3, 0);
    let b = world.push(&mut feed, 3, 0);
    let point = |key, mark, row, column| Point {
        item: world.id(key),
        mark,
        row,
        column,
    };
    let early = point(a, Some(9), 2, 5);
    let late = point(b, Some(0), 0, 0);
    assert_eq!(feed.cmp_points(&early, &late), Some(Ordering::Less));
    assert_eq!(
        feed.cmp_points(&point(a, Some(3), 0, 0), &point(a, Some(2), 2, 9)),
        Some(Ordering::Greater)
    );
    assert_eq!(
        feed.cmp_points(&point(a, None, 1, 4), &point(a, None, 1, 6)),
        Some(Ordering::Less)
    );
    let gone = world.id(a);
    feed.remove(gone);
    assert_eq!(feed.cmp_points(&early, &late), None);
}

#[test]
fn ids_of_removed_items_are_ignored() {
    let mut world = World::default();
    let mut feed = Feed::new();
    let a = world.push(&mut feed, 3, 0);
    let gone = world.id(a);
    world.remove(&mut feed, 0);
    let b = world.push(&mut feed, 5, 0);
    assert!(!feed.contains(gone));
    assert_eq!(feed.key(gone), None);
    assert_eq!(feed.rank(gone), None);
    assert_eq!(feed.remove(gone), None);
    feed.changed(gone, Change::Reflowed);
    feed.set_height(gone, 40);
    feed.place(gone, 0, 0);
    assert!(feed.is_following());
    let item = feed.insert_before(gone, 99, 1);
    assert_eq!(feed.last(), Some(item));
    assert_eq!(feed.first(), Some(world.id(b)));
    assert_eq!(feed.next(world.id(b)), Some(item));
    assert_eq!(feed.prev(item), Some(world.id(b)));
    // (Nothing was measured, so the hints count, 0 and 1.)
    assert_eq!(feed.extent().total, 1);
}

#[test]
fn a_hint_counts_until_the_item_is_measured() {
    let mut world = World::default();
    let mut feed = Feed::new();
    let a = world.push(&mut feed, 7, 1);
    assert_eq!(
        feed.extent(),
        Extent {
            total: 1,
            exact: false
        }
    );
    feed.set_hint(world.id(a), 4);
    assert_eq!(feed.extent().total, 4);
    feed.layout(10, &mut world);
    assert_eq!(
        feed.extent(),
        Extent {
            total: 7,
            exact: true
        }
    );
    feed.set_hint(world.id(a), 2);
    assert_eq!(feed.extent().total, 7);
    feed.set_height(world.id(a), 9);
    assert_eq!(feed.extent_after(world.id(a)), Some(0));
    assert_eq!(feed.extent().total, 9);
}

#[test]
fn a_long_scroll_past_the_limit_lands_by_hints() {
    let mut rng = Rng::new(21);
    let (mut feed, mut world) = build(2000, &mut rng, &mut |h| h);
    feed.set_exact_limit(Some(200));
    feed.layout(30, &mut world);
    feed.scroll_by(-5000);
    let frame = feed.layout(30, &mut world);
    assert_eq!(frame.outcome, Outcome::Approximate);
    assert_exact(&feed, &frame, &world, 30);
    // With true hints it lands where it was asked.
    let range = i64::try_from(world.total()).unwrap() - 30;
    assert_eq!(true_top(&feed, &frame, &world), range - 5000);
    // A long scroll down to the end follows it.
    feed.scroll_by(100_000);
    let frame = feed.layout(30, &mut world);
    assert!(frame.following);
}

#[test]
fn any_session_lays_out_exactly() {
    for seed in 1..=100 {
        let mut rng = Rng::new(seed);
        let n = 1 + rng.index(700);
        let viewport = 1 + rng.small(70);
        let mut hint = |h: u32| h / 2;
        let (mut feed, mut world) = build(n, &mut rng, &mut hint);
        if seed % 3 == 0 {
            feed.set_exact_limit(Some(u64::from(viewport) * 4));
        }
        if seed % 4 == 0 {
            feed.set_gap_limit(rng.small(6));
        }
        for _ in 0..1000 {
            match rng.below(17) {
                0..=3 => feed.scroll_by(rng.signed(41) - 20),
                4 => feed.scroll_by(rng.signed(6000) - 3000),
                5 => feed.follow(),
                6 => feed.scroll_to_start(),
                7 => feed.set_geometry(),
                8 => {
                    #[expect(clippy::cast_precision_loss, reason = "a share from 0 to 1")]
                    let share = rng.below(1001) as f64 / 1000.;
                    feed.seek(share, viewport);
                }
                9 if !world.order.is_empty() => {
                    let key = world.order[rng.index(world.order.len())];
                    let row = i64::from(rng.small(u64::from(viewport)));
                    feed.place(world.id(key), rng.small(9), row);
                }
                // Any item goes, the reader's too.
                10 | 11 if !world.order.is_empty() => {
                    let at = rng.index(world.order.len());
                    world.remove(&mut feed, at);
                }
                // Any item changes, the reader's too.
                12 if !world.order.is_empty() => {
                    let key = world.order[rng.index(world.order.len())];
                    world.heights.insert(key, rng.height());
                    feed.changed(world.id(key), Change::Reflowed);
                }
                13 if !world.order.is_empty() => {
                    let key = world.order[rng.index(world.order.len())];
                    feed.set_hint(world.id(key), rng.small(50));
                }
                _ => {
                    if world.order.is_empty() {
                        let height = rng.height();
                        world.push(&mut feed, height, 1);
                    } else {
                        let top = world.order[0];
                        edit_elsewhere(&mut feed, &mut world, &mut rng, top, &mut hint);
                    }
                }
            }
            let frame = feed.layout(viewport, &mut world);
            assert_eq!(feed.len(), world.order.len());
            if world.order.is_empty() {
                assert_eq!(frame.items.len(), 0);
                continue;
            }
            assert_exact(&feed, &frame, &world, viewport);
        }
    }
}

/// Items of one-row lines, each line with an id that is its row's mark, so a mark names its
/// line wherever lines come or go.
#[derive(Default)]
struct Lines {
    lines: HashMap<u64, Vec<u64>>,
    next_line: u64,
}

impl Lines {
    fn push(&mut self, feed: &mut Feed<u64>, rows: u32) -> (u64, ItemId) {
        let key = u64::try_from(self.lines.len()).unwrap();
        let ids = (0..rows).map(|_| self.line()).collect();
        self.lines.insert(key, ids);
        (key, feed.push(key, 1))
    }

    fn line(&mut self) -> u64 {
        self.next_line += 1;
        self.next_line
    }
}

impl Measure<u64> for Lines {
    fn height(&mut self, key: &u64) -> u32 {
        u32::try_from(self.lines[key].len()).unwrap()
    }

    fn mark_at(&mut self, key: &u64, row: u32) -> Option<u64> {
        self.lines[key].get(usize::try_from(row).unwrap()).copied()
    }

    fn resolve(&mut self, key: &u64, mark: u64) -> Option<u32> {
        let row = self.lines[key].iter().position(|line| *line == mark)?;
        u32::try_from(row).ok()
    }
}

/// Every item that started on screen in `before` and is still on it is lower by `by` rows in
/// `after`.
fn assert_moved_down(feed: &Feed<u64>, before: &Placement, after: &Placement, by: i64, v: i64) {
    for was in &before.items {
        if was.y < 0 || was.y + by >= v {
            continue;
        }
        let now = after.items.iter().find(|p| p.item == was.item).map(|p| p.y);
        assert_eq!(now, Some(was.y + by), "item {}", key_of(feed, was));
    }
}

#[test]
fn a_scroll_up_while_following_starts_from_the_last_frame() {
    // The end grows in the frame the wheel turns up in, by fewer rows than the scroll, as many,
    // and more.
    for (grew, limit) in [(0, 0), (1, 0), (3, 0), (5, 0), (3, 3)] {
        let mut world = World::default();
        let mut feed = Feed::new();
        feed.set_gap_limit(limit);
        for _ in 0..100 {
            world.push(&mut feed, 1, 0);
        }
        let before = feed.layout(10, &mut world);
        let last = *world.order.last().unwrap();
        *world.heights.get_mut(&last).unwrap() += grew;
        feed.changed(world.id(last), Change::Appended);
        feed.scroll_by(-1);
        let frame = feed.layout(10, &mut world);
        assert_exact(&feed, &frame, &world, 10);
        assert!(!frame.following, "grew {grew}");
        assert_moved_down(&feed, &before, &frame, 1, 10);
    }
    // The viewport gets taller in the same frame, and the rows move down with its bottom, then
    // by the scroll.
    let mut world = World::default();
    let mut feed = Feed::new();
    for _ in 0..100 {
        world.push(&mut feed, 1, 0);
    }
    let before = feed.layout(10, &mut world);
    feed.scroll_by(-1);
    let frame = feed.layout(12, &mut world);
    assert_moved_down(&feed, &before, &frame, 3, 12);
}

#[test]
fn a_taller_viewport_while_following_keeps_the_end_on_the_last_row() {
    for (height, viewports) in [(1, [20, 17, 20]), (1, [20, 18, 23]), (5, [4, 0, 4])] {
        let mut world = World::default();
        let mut feed = Feed::new();
        feed.set_gap_limit(3);
        for _ in 0..100 {
            world.push(&mut feed, height, 0);
        }
        for viewport in viewports {
            let frame = feed.layout(viewport, &mut world);
            assert!(frame.following);
            if viewport > 0 {
                assert_exact(&feed, &frame, &world, viewport);
                assert_eq!(frame.gap, 0, "{viewports:?} at {viewport}");
                assert_eq!(
                    true_top(&feed, &frame, &world),
                    i64::try_from(world.total()).unwrap() - i64::from(viewport)
                );
            }
        }
    }
}

#[test]
fn rows_lost_above_the_screen_while_following_move_nothing() {
    // Without marks. An item of 20 rows shows its last 6 on top, with 9 items of a row below.
    let mut world = World::default();
    let mut feed = Feed::new();
    feed.set_gap_limit(3);
    for _ in 0..40 {
        world.push(&mut feed, 1, 0);
    }
    let tall = world.push(&mut feed, 20, 0);
    for _ in 0..9 {
        world.push(&mut feed, 1, 0);
    }
    let before = feed.layout(15, &mut world);
    // Two of its rows above the screen go.
    world.heights.insert(tall, 18);
    feed.changed(world.id(tall), Change::Reflowed);
    let frame = feed.layout(15, &mut world);
    assert_exact(&feed, &frame, &world, 15);
    assert_eq!(frame.gap, 0);
    assert_moved_down(&feed, &before, &frame, 0, 15);

    // With marks, and the item is the last, as a reply taller than the screen is.
    let mut lines = Lines::default();
    let mut feed = Feed::new();
    feed.set_gap_limit(3);
    for _ in 0..40 {
        lines.push(&mut feed, 1);
    }
    let (key, reply) = lines.push(&mut feed, 30);
    let before = feed.layout(10, &mut lines);
    assert_eq!(before.items[0].y, -20);
    lines.lines.get_mut(&key).unwrap().drain(3..5);
    feed.changed(reply, Change::Reflowed);
    let frame = feed.layout(10, &mut lines);
    assert_eq!((frame.items[0].y, frame.gap), (-18, 0));
}

#[test]
fn following_with_a_gap_as_tall_as_the_viewport_brings_the_end_down() {
    let mut world = World::default();
    let mut feed = Feed::new();
    feed.set_gap_limit(3);
    for _ in 0..30 {
        world.push(&mut feed, 1, 0);
    }
    // A status line as tall as the viewport goes, and no row the reader saw is left.
    world.push(&mut feed, 3, 0);
    feed.layout(3, &mut world);
    world.remove(&mut feed, 30);
    let frame = feed.layout(3, &mut world);
    assert_exact(&feed, &frame, &world, 3);
    assert_eq!(frame.gap, 0);
}

#[test]
fn following_again_in_place_keeps_the_offset() {
    let mut world = World::default();
    let mut feed = Feed::new();
    for _ in 0..41 {
        world.push(&mut feed, 3, 3);
    }
    feed.set_gap_limit(3);
    let before = feed.layout(20, &mut world);
    let top = before.items[0];
    feed.place(top.item, rows_above(top.y), 0);
    world.remove(&mut feed, world.order.len() - 1);
    let held = feed.layout(20, &mut world);
    assert_eq!(held.gap, 3);
    let offset = feed.offset(20);
    feed.follow_in_place();
    assert_eq!(feed.offset(20), offset);
    feed.layout(20, &mut world);
    assert_eq!(feed.offset(20), offset);
}

#[test]
fn an_item_with_no_rows_never_holds_the_reader() {
    // The reader is on the last item, which loses every row.
    let mut world = World::default();
    let mut feed = Feed::new();
    for height in [10, 10, 10, 5] {
        world.push(&mut feed, height, 0);
    }
    feed.layout(5, &mut world);
    feed.place(world.id(3), 0, 0);
    feed.layout(5, &mut world);
    world.heights.insert(3, 0);
    feed.changed(world.id(3), Change::Reflowed);
    let frame = feed.layout(5, &mut world);
    assert_eq!(frame.gap, 0);
    assert_eq!(frame.outcome, Outcome::ClampedEnd);
    assert!(frame.items.iter().all(|p| p.height > 0), "{frame:?}");
    assert_eq!(true_top(&feed, &frame, &world), 25);
    // The reader is placed on a last item that has no rows.
    let mut world = World::default();
    let mut feed = Feed::new();
    for height in [10, 10, 0] {
        world.push(&mut feed, height, 0);
    }
    feed.place(world.id(2), 0, 0);
    let frame = feed.layout(5, &mut world);
    assert_eq!(frame.gap, 0);
    assert!(frame.items.iter().all(|p| p.height > 0), "{frame:?}");
    assert_eq!(true_top(&feed, &frame, &world), 15);
}

#[test]
fn a_frame_lists_no_item_without_rows() {
    let mut world = World::default();
    let mut feed = Feed::new();
    world.push(&mut feed, 5, 0);
    for _ in 0..2_000 {
        world.push(&mut feed, 0, 0);
    }
    world.push(&mut feed, 5, 0);
    for _ in 0..2 {
        let frame = feed.layout(40, &mut world);
        assert_eq!(frame.items.len(), 2);
        assert_eq!((frame.items[0].y, frame.items[1].y, frame.gap), (0, 5, 30));
    }
    // A feed of items with no rows shows nothing, and follows.
    let mut world = World::default();
    let mut feed = Feed::new();
    for _ in 0..5 {
        world.push(&mut feed, 0, 3);
    }
    feed.scroll_by(-2);
    let frame = feed.layout(10, &mut world);
    assert_eq!(
        (frame.items.len(), frame.gap, frame.following),
        (0, 10, true)
    );
}

/// Heights by key, which records each mark asked for a row its item doesn't have.
struct Counted {
    heights: Vec<u32>,
    past: Vec<(usize, u32)>,
}

impl Measure<usize> for Counted {
    fn height(&mut self, key: &usize) -> u32 {
        self.heights[*key]
    }

    fn mark_at(&mut self, key: &usize, row: u32) -> Option<u64> {
        if row >= self.heights[*key] {
            self.past.push((*key, row));
        }
        Some(u64::from(row))
    }

    fn resolve(&mut self, _key: &usize, mark: u64) -> Option<u32> {
        u32::try_from(mark).ok()
    }
}

#[test]
fn no_mark_is_asked_for_a_row_an_item_lacks() {
    // The reader's item loses its rows, the item after the reader's has none when the
    // reader's goes, and the reader is placed on one with none.
    for case in 0..3 {
        let mut m = Counted {
            heights: vec![10, 10, 10, 5, 0],
            past: Vec::new(),
        };
        let mut feed = Feed::new();
        let ids: Vec<_> = (0..5).map(|key| feed.push(key, 1)).collect();
        feed.place(ids[3], 0, 0);
        feed.layout(5, &mut m);
        match case {
            0 => {
                m.heights[3] = 0;
                feed.changed(ids[3], Change::Reflowed);
            }
            1 => {
                feed.remove(ids[3]);
            }
            _ => feed.place(ids[4], 0, 0),
        }
        let frame = feed.layout(5, &mut m);
        assert_eq!(m.past, [], "case {case}");
        assert_eq!(frame.gap, 0, "case {case}");
    }
}

#[test]
fn a_seek_over_hints_that_are_too_high_shows_no_blank_rows_above_the_end() {
    for fraction in [0.95, 1.0] {
        let mut world = World::default();
        let mut feed = Feed::new();
        for _ in 0..50 {
            world.push(&mut feed, 1, 100);
        }
        feed.seek(fraction, 10);
        let frame = feed.layout(10, &mut world);
        assert_exact(&feed, &frame, &world, 10);
        assert_eq!(frame.gap, 0, "seek({fraction})");
    }
}

#[test]
fn a_seek_to_the_end_follows_it() {
    let mut world = World::default();
    let mut feed = Feed::new();
    for _ in 0..50 {
        world.push(&mut feed, 1, 1);
    }
    feed.layout(10, &mut world);
    feed.scroll_by(-20);
    feed.layout(10, &mut world);
    feed.seek(1.0, 10);
    assert!(feed.layout(10, &mut world).following);
    world.push(&mut feed, 1, 1);
    let frame = feed.layout(10, &mut world);
    assert!(frame.following);
    assert_eq!(true_top(&feed, &frame, &world), 41);
}

#[test]
fn a_place_on_any_row_lays_out_without_overflow() {
    for row in [i64::MIN, i64::MIN + 1, i64::MAX, i64::MAX - 1] {
        let (mut feed, mut world) = (Feed::new(), World::default());
        for _ in 0..10 {
            world.push(&mut feed, 3, 1);
        }
        feed.set_gap_limit(3);
        feed.place(world.id(5), 2, row);
        feed.offset(5);
        let frame = feed.layout(5, &mut world);
        assert_exact(&feed, &frame, &world, 5);
        feed.place(world.id(9), 2, row);
        world.remove(&mut feed, 9);
        feed.offset(5);
        let frame = feed.layout(5, &mut world);
        assert_exact(&feed, &frame, &world, 5);
        feed.place(world.id(8), 2, row);
        feed.follow_in_place();
        feed.offset(5);
        world.remove(&mut feed, 8);
        feed.offset(5);
        let frame = feed.layout(5, &mut world);
        assert_exact(&feed, &frame, &world, 5);
    }
}

#[test]
fn a_place_far_below_the_viewport_measures_about_a_screen() {
    let (mut feed, mut world) = (Feed::new(), World::default());
    for _ in 0..100_000 {
        world.push(&mut feed, 2, 1);
    }
    feed.layout(40, &mut world);
    feed.place(world.id(99_990), 0, 1_000_000_000);
    let frame = feed.layout(40, &mut world);
    assert!(frame.measured <= 80, "{}", frame.measured);
}

#[test]
fn staying_keeps_the_rows_of_the_last_frame() {
    let mut world = World::default();
    let mut feed = Feed::new();
    feed.stay();
    assert!(feed.is_following(), "no frame to keep yet");
    for _ in 0..100 {
        world.push(&mut feed, 1, 1);
    }
    let before = feed.layout(10, &mut world);
    // A press holds the view while the reply streams on.
    feed.stay();
    assert!(!feed.is_following());
    let last = *world.order.last().unwrap();
    *world.heights.get_mut(&last).unwrap() += 4;
    feed.changed(world.id(last), Change::Appended);
    world.push(&mut feed, 2, 2);
    let frame = feed.layout(10, &mut world);
    assert!(!frame.following);
    assert_eq!(frame.items[..9], before.items[..9]);
    // After going to the end there's no frame to keep until the next layout.
    feed.follow();
    feed.stay();
    assert!(feed.is_following());
}
