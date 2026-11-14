//! A scroll core for long, live feeds, such as a transcript that streams, where nothing that
//! happens off screen moves what the reader sees.
//!
//! A [`Feed`] is an ordered list of items, each with a whole-number height in rows. The reader's
//! place is an item, a row of it and the viewport row that row is on. It's never a distance from
//! the start, so items that come, go or grow above or below the screen leave it where it is. A
//! reader who follows the end keeps the place the last frame showed too, which a scroll up starts
//! from.
//!
//! A height is exact once the item was measured at the current geometry (a width, a font), and a
//! hint until then. [`Feed::layout`] places what the viewport shows, measuring the items it
//! crosses and no others. Hints decide only where a seek or a scroll longer than the exact limit
//! lands, and the size of a scrollbar, and [`Feed::sweep`] makes them exact in idle time. A new
//! width makes every height a hint again without visiting the items, and the reader's row keeps
//! its text through a mark, a name for that row's content that [`Measure`] gives.
//!
//! The application reports what happens to its items. [`Change::Appended`] says rows came at an
//! item's end, [`Change::Reflowed`] that they changed some other way, and
//! [`Feed::set_height`] gives a height the application knows without measuring.
//!
//! ```
//! use termelon::feed::{Change, Feed, Measure};
//!
//! /// Items whose heights are known, one entry each.
//! struct Heights(Vec<u32>);
//!
//! impl Measure<usize> for Heights {
//!     fn height(&mut self, key: &usize) -> u32 {
//!         self.0[*key]
//!     }
//! }
//!
//! let mut heights = Heights(vec![3; 100]);
//! let mut feed = Feed::new();
//! let ids: Vec<_> = (0..100).map(|key| feed.push(key, 1)).collect();
//!
//! // The reader follows the end, then scrolls up a screen.
//! let frame = feed.layout(10, &mut heights);
//! assert!(frame.following);
//! feed.scroll_by(-10);
//! let frame = feed.layout(10, &mut heights);
//! let top = frame.items[0];
//!
//! // An item above the screen grows, and the screen doesn't move.
//! heights.0[5] = 40;
//! feed.changed(ids[5], Change::Reflowed);
//! let frame = feed.layout(10, &mut heights);
//! assert_eq!(frame.items[0], top);
//! assert!(!frame.following);
//! ```

mod index;
#[cfg(test)]
mod tests;

use core::cmp::Ordering;
use core::fmt;

use index::{Cur, Index};

/// An item's id. The calls that add an item give it, and every later call about the item takes
/// it. The id of an item that was removed finds nothing. An id belongs to the feed that gave it,
/// and another feed may take it for one of its own items.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ItemId {
    slot: u32,
    generation: u32,
}

/// Measures a feed's items at the current geometry, for [`Feed::layout`] and [`Feed::sweep`].
pub trait Measure<K> {
    /// The item's exact height.
    fn height(&mut self, key: &K) -> u32;

    /// A name for the content of the item's row `row` that survives a new layout, such as the
    /// byte of the source the row starts at. A mark should grow down through an item. `None`,
    /// the default, keeps the row's number instead, which a new width may give other text.
    fn mark_at(&mut self, _key: &K, _row: u32) -> Option<u64> {
        None
    }

    /// The row the content `mark` names is on now.
    fn resolve(&mut self, _key: &K, _mark: u64) -> Option<u32> {
        None
    }
}

/// Where the reader is, exact after a layout. A call that moves the reader, such as removing
/// their item, leaves a rough place until the next layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Anchor {
    /// At the end, staying there as the feed grows.
    Follow,
    /// The row `offset` of the item `item` is on the viewport's row `row`.
    #[non_exhaustive]
    At {
        /// The item.
        item: ItemId,
        /// The row of the item.
        offset: u32,
        /// The viewport's row it's on, 0 after a layout.
        row: i64,
        /// The row's mark, which keeps its text at the top when the item's rows change.
        mark: Option<u64>,
    },
}

/// An item the viewport shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Placed {
    /// The item.
    pub item: ItemId,
    /// The viewport row of its top, negative when its first rows are above the viewport.
    pub y: i64,
    /// Its height, exact.
    pub height: u32,
}

/// Why a layout put the reader's row somewhere other than where it was, or where the scroll
/// asked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    /// The reader's row is where it was, or where the scroll put it.
    #[default]
    Kept,
    /// The feed's start reached the viewport's top.
    ClampedStart,
    /// The feed's end reached the viewport's bottom.
    ClampedEnd,
    /// The feed is shorter than the viewport.
    Short,
    /// The reader's item was removed, and a neighbour took its place.
    Replaced,
    /// The scroll was longer than [`Feed::set_exact_limit`] allows, and landed by hints.
    Approximate,
}

/// What a viewport shows, from [`Feed::layout`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Placement {
    /// The items on screen, top to bottom, the first starting at or above row 0.
    pub items: Vec<Placed>,
    /// Blank rows between the feed's end and the viewport's bottom.
    pub gap: u32,
    /// Whether the reader follows the end.
    pub following: bool,
    /// How many items the layout measured.
    pub measured: u32,
    /// Where the reader's row went.
    pub outcome: Outcome,
}

/// How an item changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Change {
    /// Content came at its end, so the rows it had keep their numbers.
    Appended,
    /// Its rows changed in some other way, such as a fold or a new parse.
    Reflowed,
}

/// The feed's height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Extent {
    /// The rows of every item, by the heights known so far.
    pub total: u64,
    /// Whether every item was measured at the current geometry.
    pub exact: bool,
}

/// One end of a selection. It names content rather than a screen row, so it keeps its text
/// across a new layout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Point {
    /// The item.
    pub item: ItemId,
    /// The mark of the row, from [`Measure::mark_at`].
    pub mark: Option<u64>,
    /// The row of the item, for an item without marks.
    pub row: u32,
    /// The column in the row.
    pub column: u32,
}

impl Point {
    /// The point at `column` of the item's row, which `mark` names, or `row` for an item
    /// without marks.
    pub fn new(item: ItemId, mark: Option<u64>, row: u32, column: u32) -> Self {
        Self {
            item,
            mark,
            row,
            column,
        }
    }
}

/// The reader's place, the row `offset` of the item on the viewport's row `row`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Place {
    item: ItemId,
    offset: u32,
    row: i64,
    /// The row's mark, which keeps its text when the item's rows change.
    mark: Option<u64>,
    /// The viewport's height the place was laid out at, `None` for a place a call gave.
    seen: Option<u32>,
}

impl Place {
    /// The place a call gives, with no mark.
    fn at(item: ItemId, offset: u32, row: i64) -> Self {
        Self {
            item,
            offset,
            row,
            mark: None,
            seen: None,
        }
    }

    /// The place on a viewport of `viewport` rows of a reader who follows, whose rows move
    /// with the viewport's bottom.
    fn following(self, viewport: u32) -> Self {
        let moved = i64::from(viewport) - i64::from(self.seen.unwrap_or(viewport));
        Self {
            row: self.row.saturating_add(moved),
            seen: None,
            ..self
        }
    }
}

/// An ordered list of items, their heights, and the reader's place in them. `K` is the
/// application's own key for an item, which [`Measure`] gets.
pub struct Feed<K> {
    index: Index<K>,
    /// The reader's place, the top row of the last frame or where a call put it. `None` while
    /// following puts the end on the viewport's last row.
    place: Option<Place>,
    /// Whether the reader follows the end, from `place` when there is one.
    following: bool,
    /// The scroll asked for since the last layout, down when positive.
    pending: i64,
    /// The reader's item was removed since the last layout.
    replaced: bool,
    /// The reader's place came from a jump since the last layout, a removal or a seek, so the
    /// end comes down to the viewport's last row if it's above it.
    settle: bool,
    /// The place's rows may have changed since the last layout, so its mark is resolved.
    resolve: bool,
    /// The most rows a scroll measures its way across.
    exact_limit: Option<u64>,
    /// The most blank rows kept below the end while following.
    gap_limit: u32,
}

impl<K> fmt::Debug for Feed<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Feed")
            .field("len", &self.len())
            .field("anchor", &self.anchor())
            .field("extent", &self.extent())
            .finish_non_exhaustive()
    }
}

impl<K> Default for Feed<K> {
    fn default() -> Self {
        Self::new()
    }
}

/// The rows from an item's top on `top`, at or above the viewport's first row, to that row.
fn rows_above(top: i64) -> u32 {
    u32::try_from(top.saturating_neg()).unwrap_or(0)
}

impl<K> Feed<K> {
    /// An empty feed, following its end.
    pub fn new() -> Self {
        Self {
            index: Index::new(),
            place: None,
            following: true,
            pending: 0,
            replaced: false,
            settle: false,
            resolve: false,
            exact_limit: None,
            gap_limit: 0,
        }
    }

    /// How many items it has.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether it has no items.
    pub fn is_empty(&self) -> bool {
        self.index.len() == 0
    }

    /// Whether the item is in the feed.
    pub fn contains(&self, item: ItemId) -> bool {
        self.index.locate(item).is_some()
    }

    /// The item's key.
    pub fn key(&self, item: ItemId) -> Option<&K> {
        self.index.locate(item).map(|cur| self.index.key(cur))
    }

    /// The first item.
    pub fn first(&self) -> Option<ItemId> {
        self.index.first().map(|cur| self.index.id(cur))
    }

    /// The last item.
    pub fn last(&self) -> Option<ItemId> {
        self.index.last().map(|cur| self.index.id(cur))
    }

    /// The item after this one.
    pub fn next(&self, item: ItemId) -> Option<ItemId> {
        let cur = self.index.next(self.index.locate(item)?)?;
        Some(self.index.id(cur))
    }

    /// The item before this one.
    pub fn prev(&self, item: ItemId) -> Option<ItemId> {
        let cur = self.index.prev(self.index.locate(item)?)?;
        Some(self.index.id(cur))
    }

    /// Adds an item at the end, counted at `hint` rows until it's measured.
    pub fn push(&mut self, key: K, hint: u32) -> ItemId {
        self.index.push(key, hint)
    }

    /// Adds an item at the start.
    pub fn push_front(&mut self, key: K, hint: u32) -> ItemId {
        match self.index.first() {
            Some(cur) => self.index.insert(cur, key, hint),
            None => self.index.push(key, hint),
        }
    }

    /// Adds an item before the item `before`, or at the end when that isn't in the feed.
    pub fn insert_before(&mut self, before: ItemId, key: K, hint: u32) -> ItemId {
        match self.index.locate(before) {
            Some(cur) => self.index.insert(cur, key, hint),
            None => self.index.push(key, hint),
        }
    }

    /// Removes an item and gives its key back. When the reader's row was in it, the item after
    /// it takes its place. For a reader above the end, it starts where the removed one started,
    /// or on the viewport's first row if that was higher, and with no item after it, the end
    /// comes down to the viewport's last row. The next layout says [`Outcome::Replaced`]. For a
    /// reader who follows, the item after it stays where it was.
    pub fn remove(&mut self, item: ItemId) -> Option<K> {
        let cur = self.index.locate(item)?;
        if let Some(place) = self.place
            && place.item == item
        {
            let height = self.index.height(cur);
            let offset = place.offset.min(height.saturating_sub(1));
            let top = place.row.saturating_sub(i64::from(offset));
            let neighbour = if let Some(next) = self.index.next(cur) {
                let row = if self.following {
                    top.saturating_add(i64::from(height))
                } else {
                    top.max(0)
                };
                Some((next, 0, row))
            } else {
                // The item before it that has rows, its last row just above where it started.
                let mut prev = self.index.prev(cur);
                while let Some(at) = prev
                    && self.index.is_fresh(at)
                    && self.index.height(at) == 0
                {
                    prev = self.index.prev(at);
                }
                prev.map(|prev| (prev, u32::MAX, top.saturating_sub(1)))
            };
            self.place = neighbour.map(|(next, offset, row)| Place {
                seen: place.seen,
                ..Place::at(self.index.id(next), offset, row)
            });
            if self.place.is_none() {
                self.following = true;
            } else if !self.following {
                self.replaced = true;
                self.settle = true;
            }
            self.resolve = false;
        }
        Some(self.index.remove(cur))
    }

    /// Says that an item's height may have changed. It counts its old height until it's
    /// measured again.
    pub fn changed(&mut self, item: ItemId, change: Change) {
        let Some(cur) = self.index.locate(item) else {
            return;
        };
        self.index.invalidate(cur);
        // (Rows that came at the end leave the rows before them as they were, so the reader's
        // row number still holds.)
        if change == Change::Reflowed && self.place.is_some_and(|place| place.item == item) {
            self.resolve = true;
        }
    }

    /// Gives an item's exact height, for one whose owner knows it without measuring, such as
    /// output that grew by a line. It counts as [`Change::Appended`].
    pub fn set_height(&mut self, item: ItemId, height: u32) {
        if let Some(cur) = self.index.locate(item) {
            self.index.set(cur, height);
        }
    }

    /// Counts an item at `hint` rows until it's measured, as an estimate from its owner. A
    /// height measured at the current geometry stays.
    pub fn set_hint(&mut self, item: ItemId, hint: u32) {
        if let Some(cur) = self.index.locate(item) {
            self.index.set_hint(cur, hint);
        }
    }

    /// Says that every height may have changed, after another width, font or scale. Each item
    /// counts its old height until it's measured again. This visits no item, only the index's
    /// leaves, one for 64 items. Following, the end comes back to the viewport's last row.
    pub fn set_geometry(&mut self) {
        self.index.bump_epoch();
        if self.following {
            self.place = None;
        }
        self.resolve = self.place.is_some();
    }

    /// Scrolls by `rows` at the next layout, down when positive.
    pub fn scroll_by(&mut self, rows: i64) {
        self.pending = self.pending.saturating_add(rows);
    }

    /// Goes to the end and follows it.
    pub fn follow(&mut self) {
        self.follow_in_place();
        self.place = None;
        self.resolve = false;
    }

    /// Follows the end from the reader's place, as if they had followed it all along. Where the
    /// end is in the viewport and the rows above it stay within the gap limit, they stay where
    /// they are, with blank rows below the end. Otherwise it goes to the end, as
    /// [`follow`](Self::follow) does. An application that holds the view still while the mouse is
    /// down calls it when the button comes up.
    pub fn follow_in_place(&mut self) {
        self.following = true;
        self.pending = 0;
        self.replaced = false;
        self.settle = false;
    }

    /// Stops following, with the rows where the last layout put them, as a press that holds
    /// the view still does. Before the first layout, and after [`follow`](Self::follow) or a new
    /// geometry, there's no such layout, and the reader follows on.
    pub fn stay(&mut self) {
        if self.place.is_some() {
            self.following = false;
        }
    }

    /// Goes to the start.
    pub fn scroll_to_start(&mut self) {
        if let Some(first) = self.first() {
            self.place(first, 0, 0);
        }
    }

    /// Puts the row `offset` of the item on the viewport's row `row` at the next layout, in
    /// place of any scroll asked for. A fold that opens under the pointer uses it with the row
    /// the pointer is on, so the line clicked stays there. A row below the viewport counts as
    /// the row just below it.
    pub fn place(&mut self, item: ItemId, offset: u32, row: i64) {
        if !self.contains(item) {
            return;
        }
        self.place = Some(Place::at(item, offset, row));
        self.following = false;
        self.pending = 0;
        self.replaced = false;
        self.settle = false;
        self.resolve = false;
    }

    /// Goes to the share `fraction` of the feed's height, by the heights known so far, as a
    /// drag of a scrollbar's thumb does. Where hints that were too high put the end above the
    /// viewport's last row, the end comes down to it, and a share of 1 follows the end.
    pub fn seek(&mut self, fraction: f64, viewport: u32) {
        if fraction >= 1. {
            self.follow();
            return;
        }
        let range = self.index.total().saturating_sub(u64::from(viewport));
        #[expect(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "a share of the height, rounded to a row, between 0 and the height"
        )]
        let target = (fraction.clamp(0., 1.) * range as f64).round() as u64;
        if let Some((cur, within)) = self.index.find(target) {
            let item = self.index.id(cur);
            self.place(item, within, 0);
            self.settle = true;
        }
    }

    /// A scroll longer than `rows` lands by the heights known so far instead of measuring every
    /// item it crosses, and says [`Outcome::Approximate`]. `None`, the default, measures any
    /// distance.
    pub fn set_exact_limit(&mut self, rows: Option<u64>) {
        self.exact_limit = rows;
    }

    /// While the reader follows the end and the end gets shorter, the rows above it stay where
    /// they are, and up to `rows` blank rows show below the end until content fills them. Past
    /// that, the end comes back down to the viewport's last row. 0, the default, always keeps
    /// the end on the last row, so the rows above move down as the end gets shorter.
    pub fn set_gap_limit(&mut self, rows: u32) {
        self.gap_limit = rows;
    }

    /// Whether the reader follows the end.
    pub fn is_following(&self) -> bool {
        self.anchor() == Anchor::Follow
    }

    /// Where the reader is.
    pub fn anchor(&self) -> Anchor {
        match self.place {
            Some(place) if !self.following => Anchor::At {
                item: place.item,
                offset: place.offset,
                row: place.row,
                mark: place.mark,
            },
            _ => Anchor::Follow,
        }
    }

    /// The feed's height by the heights known so far.
    pub fn extent(&self) -> Extent {
        Extent {
            total: self.index.total(),
            exact: self.index.all_fresh(),
        }
    }

    /// How far the viewport's top is from the feed's start, by the heights known so far, for a
    /// scrollbar. It's exact once [`extent`](Self::extent) is.
    pub fn offset(&self, viewport: u32) -> u64 {
        let place = self
            .place
            .and_then(|place| Some((place, self.index.locate(place.item)?)));
        let Some((mut place, cur)) = place else {
            return self.index.total().saturating_sub(u64::from(viewport));
        };
        if self.following {
            place = place.following(viewport);
        }
        let offset = place.offset.min(self.index.height(cur).saturating_sub(1));
        let top =
            i128::from(self.index.offset_of(cur)) + i128::from(offset) - i128::from(place.row);
        u64::try_from(top.max(0)).unwrap_or(u64::MAX)
    }

    /// The item's place in the order, for comparing the ends of a selection.
    pub fn rank(&self, item: ItemId) -> Option<usize> {
        self.index.locate(item).map(|cur| self.index.rank(cur))
    }

    /// The height of the items after this one, by the heights known so far, such as for a
    /// count of new lines below the screen.
    pub fn extent_after(&self, item: ItemId) -> Option<u64> {
        let cur = self.index.locate(item)?;
        let end = self.index.offset_of(cur) + u64::from(self.index.height(cur));
        Some(self.index.total().saturating_sub(end))
    }

    /// The order of two ends of a selection, by item, then mark (or row), then column. `None`
    /// when either item is gone.
    pub fn cmp_points(&self, a: &Point, b: &Point) -> Option<Ordering> {
        let by_item = self.rank(a.item)?.cmp(&self.rank(b.item)?);
        let by_row = match (a.mark, b.mark) {
            (Some(x), Some(y)) => x.cmp(&y),
            _ => a.row.cmp(&b.row),
        };
        Some(by_item.then(by_row).then(a.column.cmp(&b.column)))
    }

    /// Measures up to `max` items that aren't measured at the current geometry, nearest the
    /// reader first, which only makes [`extent`](Self::extent) exact. It returns how many it
    /// measured, which is less than `max` once every item is exact.
    pub fn sweep(&mut self, max: usize, m: &mut impl Measure<K>) -> usize {
        let near = match self.anchor() {
            Anchor::At { item, .. } => self.index.locate(item),
            _ => self.index.last(),
        };
        let Some(near) = near else {
            return 0;
        };
        let mut done = 0;
        while done < max {
            let Some(cur) = self.index.next_stale(near) else {
                break;
            };
            let height = m.height(self.index.key(cur));
            self.index.set(cur, height);
            done += 1;
        }
        done
    }

    /// Places the items a viewport of `viewport` rows shows, measuring the items it crosses,
    /// and takes the scroll asked for since the last layout.
    pub fn layout(&mut self, viewport: u32, m: &mut impl Measure<K>) -> Placement {
        let mut placement = Placement::default();
        self.layout_into(viewport, m, &mut placement);
        placement
    }

    /// [`layout`](Self::layout) into `out`, whose memory it reuses.
    pub fn layout_into(&mut self, viewport: u32, m: &mut impl Measure<K>, out: &mut Placement) {
        out.items.clear();
        let mut measured = 0;
        let v = i64::from(viewport);
        let resolve = core::mem::take(&mut self.resolve);
        let mut delta = core::mem::take(&mut self.pending);
        let replaced = core::mem::take(&mut self.replaced);
        let mut settle = core::mem::take(&mut self.settle);
        let Some(last) = self.index.last() else {
            self.place = None;
            self.following = true;
            out.gap = viewport;
            out.following = true;
            out.measured = 0;
            out.outcome = Outcome::Short;
            return;
        };
        let mut outcome = if replaced {
            Outcome::Replaced
        } else {
            Outcome::Kept
        };

        // A reader who follows the end and scrolls up reads from the rows of the last frame,
        // which move with the viewport's bottom. Scrolling down changes nothing.
        let mut left_follow = false;
        if self.anchor() == Anchor::Follow {
            if delta < 0 {
                let place = if let Some(place) = self.place {
                    place.following(viewport)
                } else {
                    let end = self.place_end(last, v, m, &mut measured);
                    Place::at(self.index.id(end.first), rows_above(end.top), 0)
                };
                self.place = Some(place);
                self.following = false;
                left_follow = true;
            } else {
                delta = 0;
            }
        }
        let down = delta > 0;

        // A scroll too long to measure across lands by the known heights.
        if let Some(limit) = self.exact_limit
            && !self.following
            && delta.unsigned_abs() > limit
        {
            let from = i128::from(self.offset(viewport));
            let range = i128::from(self.index.total().saturating_sub(u64::from(viewport)));
            let target = (from + i128::from(delta)).clamp(0, range);
            if let Some((cur, within)) = self.index.find(u64::try_from(target).unwrap_or(u64::MAX))
            {
                self.place = Some(Place::at(self.index.id(cur), within, 0));
            }
            delta = 0;
            outcome = Outcome::Approximate;
            settle = true;
        }

        // The reader's mark, while their row doesn't move, and the row it named.
        let mut kept = None;
        let placed = match self.place {
            Some(place) if !self.following => {
                let cur = self.index.locate(place.item).unwrap_or(last);
                let height = self.exact(cur, m, &mut measured);
                if height == 0 {
                    // An item with no rows has no place on screen, so the reader leaves it as
                    // they would leave one that went.
                    settle = true;
                    outcome = Outcome::Replaced;
                }
                let offset = self.resolved(cur, place, height, resolve, m);
                if delta == 0
                    && height > 0
                    && let Some(mark) = place.mark
                {
                    kept = Some((place.item, offset, mark));
                }
                // The item's top, moved by the scroll. (A row below the viewport counts as the
                // row just below it, so the walk up crosses a viewport at most.)
                let top = place
                    .row
                    .min(v)
                    .saturating_sub(i64::from(offset))
                    .saturating_sub(delta);
                let (first, top, clamped) = self.fill_up(cur, top, m, &mut measured);
                if clamped {
                    outcome = Outcome::ClampedStart;
                }
                let placed = self.skip_down(first, top, v, m, &mut measured);
                // (Their place is past the end, so none of their rows is left to keep.)
                settle |= placed.past;
                let above = placed.top < 0 || self.index.prev(placed.first).is_some();
                match placed.end {
                    // The reader scrolled down to the end, and follows it.
                    Some(end) if down => {
                        if end < v && above {
                            outcome = Outcome::ClampedEnd;
                        }
                        self.following = true;
                        self.place = None;
                        self.place_end(last, v, m, &mut measured)
                    }
                    // They scrolled up in a feed that fits the viewport, so they follow it still,
                    // from its start. (Rows that blank rows below the end held above the
                    // viewport come back into it.)
                    Some(_) if left_follow && !above => {
                        self.following = true;
                        placed
                    }
                    // Their place came from a jump, so the end comes down to fill.
                    Some(end) if end < v && above && settle => {
                        outcome = Outcome::ClampedEnd;
                        let (first, top, _) =
                            self.fill_up(placed.first, placed.top + (v - end), m, &mut measured);
                        self.skip_down(first, top, v, m, &mut measured)
                    }
                    // Content went from under a reader who didn't move. Their rows stay, and
                    // the rows below the end are blank.
                    Some(end) => {
                        if end < v && !above && outcome == Outcome::Kept {
                            outcome = Outcome::Short;
                        }
                        placed
                    }
                    None => placed,
                }
            }
            _ => {
                self.following = true;
                self.place_following(last, viewport, resolve, m, &mut measured, &mut kept)
            }
        };

        // The items on screen, top to bottom, and the first that starts on screen.
        let mut cur = Some(placed.first);
        let mut y = placed.top;
        let mut starts = None;
        while let Some(at) = cur {
            if y >= v {
                break;
            }
            let height = self.exact(at, m, &mut measured);
            if height > 0 {
                out.items.push(Placed {
                    item: self.index.id(at),
                    y,
                    height: u32::try_from(height).unwrap_or(0),
                });
                if y >= 0 && starts.is_none() {
                    starts = Some((at, y));
                }
            }
            y += height;
            cur = self.index.next(at);
        }

        let item = self.index.id(placed.first);
        let offset = rows_above(placed.top);
        let gap = placed
            .end
            .map_or(0, |end| u32::try_from((v - end).max(0)).unwrap_or(0));
        // (Taken again only when the reader's row moved, so widths one after another keep the
        // text it named rather than the start of each width's row.)
        let mark = match kept {
            _ if self.index.height(placed.first) == 0 => None,
            Some((at, row, mark)) if (at, row) == (item, offset) => Some(mark),
            _ => m.mark_at(self.index.key(placed.first), offset),
        };
        let place = Place {
            mark,
            seen: Some(viewport),
            ..Place::at(item, offset, 0)
        };
        self.place = if self.index.height(placed.first) == 0 {
            // No item has rows, so the reader follows the end as in an empty feed.
            self.following = true;
            None
        } else if self.following
            && mark.is_none()
            && let Some((at, y)) = starts
        {
            // A row known by its number alone may name other text when its item changes, so
            // following keeps the first row of the first item that starts on screen.
            Some(Place {
                seen: Some(viewport),
                ..Place::at(self.index.id(at), 0, y)
            })
        } else {
            Some(place)
        };
        out.gap = gap;
        out.following = self.following;
        out.measured = measured;
        out.outcome = outcome;
    }

    /// The item's height, measured first if it's a hint.
    fn exact(&mut self, cur: Cur, m: &mut impl Measure<K>, measured: &mut u32) -> i64 {
        if !self.index.is_fresh(cur) {
            let height = m.height(self.index.key(cur));
            self.index.set(cur, height);
            *measured += 1;
        }
        i64::from(self.index.height(cur))
    }

    /// The row of the item `cur`, `height` rows high, that `place` names, by its mark when the
    /// item's rows may have changed.
    fn resolved(
        &self,
        cur: Cur,
        place: Place,
        height: i64,
        resolve: bool,
        m: &mut impl Measure<K>,
    ) -> u32 {
        let mut offset = place.offset;
        if resolve
            && height > 0
            && let Some(mark) = place.mark
            && let Some(found) = m.resolve(self.index.key(cur), mark)
        {
            offset = found;
        }
        offset.min(u32::try_from(height - 1).unwrap_or(0))
    }

    /// What shows when the feed's end, the item `last`, is on the viewport's last row.
    fn place_end(
        &mut self,
        last: Cur,
        v: i64,
        m: &mut impl Measure<K>,
        measured: &mut u32,
    ) -> Laid {
        let height = self.exact(last, m, measured);
        let (first, top, _) = self.fill_up(last, v - height, m, measured);
        self.skip_down(first, top, v, m, measured)
    }

    /// What shows while following, the feed's end on the viewport's last row, or higher by the
    /// blank rows it takes to keep the reader's row from moving down on screen by more than a
    /// taller viewport's bottom did, up to the gap limit.
    fn place_following(
        &mut self,
        last: Cur,
        viewport: u32,
        resolve: bool,
        m: &mut impl Measure<K>,
        measured: &mut u32,
        kept: &mut Option<(ItemId, u32, u64)>,
    ) -> Laid {
        let v = i64::from(viewport);
        let end = self.place_end(last, v, m, measured);
        let Some((place, cur)) = self
            .place
            .and_then(|place| Some((place, self.index.locate(place.item)?)))
        else {
            return end;
        };
        let height = self.exact(cur, m, measured);
        let offset = self.resolved(cur, place, height, resolve, m);
        if height > 0
            && let Some(mark) = place.mark
        {
            *kept = Some((place.item, offset, mark));
        }
        // The row the end on the last row puts the reader's on. (An item with no rows at the
        // end is on the row after the last.)
        let mut at = Some(end.first);
        let mut y = end.top;
        let row = loop {
            let Some(here) = at.filter(|_| y <= v) else {
                return end;
            };
            if here == cur {
                break y + i64::from(offset);
            }
            y += self.exact(here, m, measured);
            at = self.index.next(here);
        };
        // (A taller viewport lets the rows move down with its bottom, and a shorter one lets
        // them move up, but not down.)
        let taller = (v - i64::from(place.seen.unwrap_or(viewport))).max(0);
        let down = row.saturating_sub(place.row).saturating_sub(taller);
        let gap = end
            .end
            .map_or(0, |end| (v - end).max(0))
            .saturating_add(down);
        if down > 0 && gap <= i64::from(self.gap_limit) && gap < v {
            self.skip_down(end.first, end.top.saturating_sub(down), v, m, measured)
        } else {
            end
        }
    }

    /// Walks up from the item `cur`, whose top is on row `top`, to the item that covers the
    /// viewport's first row. Returns it, its top's row, and whether the feed's start came
    /// first, which then sits on row 0.
    fn fill_up(
        &mut self,
        mut cur: Cur,
        mut top: i64,
        m: &mut impl Measure<K>,
        measured: &mut u32,
    ) -> (Cur, i64, bool) {
        while top > 0 {
            match self.index.prev(cur) {
                Some(prev) => {
                    top -= self.exact(prev, m, measured);
                    cur = prev;
                }
                None => return (cur, 0, true),
            }
        }
        (cur, top, false)
    }

    /// Walks down from the item `cur`, whose top is on row `top`, past the items that end above
    /// the viewport, and on to where the feed ends if that's in the viewport.
    fn skip_down(
        &mut self,
        mut cur: Cur,
        mut top: i64,
        v: i64,
        m: &mut impl Measure<K>,
        measured: &mut u32,
    ) -> Laid {
        let mut past = false;
        loop {
            let mut height = self.exact(cur, m, measured);
            if top.saturating_add(height) > 0 {
                break;
            }
            if let Some(next) = self.index.next(cur) {
                top += height;
                cur = next;
            } else {
                // Everything is above the viewport, so the last row of the last item that has
                // rows goes on the first row.
                past = true;
                while height == 0
                    && let Some(prev) = self.index.prev(cur)
                {
                    cur = prev;
                    height = self.exact(cur, m, measured);
                }
                top = (1 - height).min(0);
                break;
            }
        }
        let mut at = cur;
        let mut y = top;
        // (An item that ends on the last row may be the last one that has rows, so the walk
        // looks at the one after it too.)
        let end = loop {
            y += self.exact(at, m, measured);
            match self.index.next(at) {
                Some(_) if y > v => break None,
                Some(next) => at = next,
                None => break (y <= v).then_some(y),
            }
        };
        Laid {
            first: cur,
            top,
            end,
            past,
        }
    }
}

/// The first item on screen, the row its top is on (0 or above), and the row after the feed's
/// last one when the feed ends in the viewport.
#[derive(Clone, Copy)]
struct Laid {
    first: Cur,
    top: i64,
    end: Option<i64>,
    /// Whether the walk started past the feed's end, so its last row went on the first row.
    past: bool,
}
