//! The [`FeedView`] widget.

use core::fmt;
use core::mem;
use core::ops::Range;

use crate::buffer::Buffer;
use crate::feed::{Feed, ItemId, Measure, Placement, Point};
use crate::layout::{Position, Rect};

use super::Widget;

/// The rows of a feed's items, which a [`FeedView`] measures and draws.
///
/// [`Measure::height`] gives an item's height at the width [`set_width`](Self::set_width) last
/// gave, and [`Measure::mark_at`] and [`Measure::resolve`] keep the reader's text on its row when
/// the width changes. Wrapping is the usual measure, and a source that keeps each item's wrapped
/// rows until the width or the item changes draws a frame for the cost of the items on screen.
pub trait FeedRows<K>: Measure<K> {
    /// Measures and draws at `width` columns from now on. The view calls it before its first
    /// frame and whenever its width changes, and counts every height as a hint from then on.
    /// Whatever else changes what the source measures, such as another width set from outside
    /// or a new theme, calls [`Feed::set_geometry`] on [`FeedView::feed_mut`].
    fn set_width(&mut self, width: u16);

    /// Draws the rows `rows` of the item into `area`, with row `rows.start` on the area's first
    /// row. The area is as wide as the view and as high as `rows` is long.
    fn render(&mut self, key: &K, rows: Range<u32>, area: Rect, buf: &mut Buffer);
}

/// A [`Feed`] drawn into an area, for long, live lists such as a transcript.
///
/// Each frame draws the items the reader's place shows, measuring only those and the ones a
/// scroll crosses. Items that come, go or grow off screen leave the screen as it was, a width change keeps the text at the top,
/// and following the end keeps its last row on the area's last. When the rows moved since the
/// last frame, as they do on a scroll, the view says so with
/// [`Buffer::note_scroll`](crate::Buffer::note_scroll), so a terminal that can scroll a region
/// gets the rows that came in and no others.
///
/// The application keeps its items and reports what happens to them through
/// [`feed_mut`](Self::feed_mut), and passes a [`FeedRows`] to draw with.
///
/// ```
/// use std::ops::Range;
///
/// use termelon::buffer::Buffer;
/// use termelon::feed::Measure;
/// use termelon::layout::Rect;
/// use termelon::style::Style;
/// use termelon::widgets::{FeedRows, FeedView};
///
/// /// Items of one row each, their text kept by index.
/// struct Lines(Vec<String>);
///
/// impl Measure<usize> for Lines {
///     fn height(&mut self, _key: &usize) -> u32 {
///         1
///     }
/// }
///
/// impl FeedRows<usize> for Lines {
///     fn set_width(&mut self, _width: u16) {}
///
///     fn render(&mut self, key: &usize, _rows: Range<u32>, area: Rect, buf: &mut Buffer) {
///         buf.set_stringn(area.x, area.y, &self.0[*key], usize::from(area.width), Style::new());
///     }
/// }
///
/// let mut lines = Lines((0..50).map(|i| format!("line {i}")).collect());
/// let mut view = FeedView::new();
/// for key in 0..50 {
///     view.feed_mut().push(key, 1);
/// }
/// let area = Rect::new(0, 0, 10, 3);
/// let mut buffer = Buffer::empty(area);
/// view.render(area, &mut buffer, &mut lines);
/// let row = |buffer: &Buffer, y| (0..10).map(|x| buffer.symbol((x, y))).collect::<String>();
/// assert_eq!(row(&buffer, 2), "line 49   ");
///
/// // A wheel step up.
/// view.feed_mut().scroll_by(-3);
/// buffer.reset();
/// view.render(area, &mut buffer, &mut lines);
/// assert_eq!(row(&buffer, 0), "line 44   ");
/// assert!(!view.feed().is_following());
/// ```
pub struct FeedView<K> {
    feed: Feed<K>,
    /// The width the items were measured at.
    width: Option<u16>,
    /// What the last frame showed.
    shown: Placement,
    /// The frame being laid out, kept for its memory.
    next: Placement,
    /// The area of the last frame.
    area: Option<Rect>,
}

impl<K> fmt::Debug for FeedView<K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FeedView")
            .field("feed", &self.feed)
            .field("width", &self.width)
            .field("shown", &self.shown)
            .field("area", &self.area)
            .finish_non_exhaustive()
    }
}

impl<K> Default for FeedView<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K> FeedView<K> {
    /// A view of an empty feed.
    pub fn new() -> Self {
        Self::with_feed(Feed::new())
    }

    /// A view of `feed`.
    pub fn with_feed(feed: Feed<K>) -> Self {
        Self {
            feed,
            width: None,
            shown: Placement::default(),
            next: Placement::default(),
            area: None,
        }
    }

    /// The feed.
    pub fn feed(&self) -> &Feed<K> {
        &self.feed
    }

    /// The feed, to report what happened to its items and to move the reader.
    pub fn feed_mut(&mut self) -> &mut Feed<K> {
        &mut self.feed
    }

    /// What the last frame showed, with its items' rows counted from the top of
    /// [`area`](Self::area).
    pub fn placement(&self) -> &Placement {
        &self.shown
    }

    /// The area of the last frame.
    pub fn area(&self) -> Option<Rect> {
        self.area
    }

    /// Draws the feed into `area` of `buf` with `rows`, and returns what it showed. Rows below
    /// the feed's end are left as they are, which is blank in a full frame. An empty area shows
    /// no items and lays out nothing, so a scroll waits for the area to come back.
    pub fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        rows: &mut impl FeedRows<K>,
    ) -> &Placement {
        let area = area.intersection(buf.area());
        if area.is_empty() {
            self.area = None;
            self.shown.items.clear();
            return &self.shown;
        }
        let width_changed = self.measure_at(area.width, rows);
        self.feed
            .layout_into(u32::from(area.height), rows, &mut self.next);
        let height = i64::from(area.height);
        for placed in &self.next.items {
            let first = u32::try_from(-placed.y).unwrap_or(0);
            let end = i64::from(placed.height).min(height - placed.y);
            let end = u32::try_from(end).unwrap_or(0);
            if first >= end {
                continue;
            }
            let (Ok(top), Ok(high)) = (u16::try_from(placed.y.max(0)), u16::try_from(end - first))
            else {
                continue;
            };
            let Some(key) = self.feed.key(placed.item) else {
                continue;
            };
            let rect = Rect::new(area.x, area.y + top, area.width, high);
            rows.render(key, first..end, rect, buf);
        }
        if !width_changed
            && self.area == Some(area)
            && let Some(by) = self.moved()
        {
            buf.note_scroll(area, by);
        }
        self.area = Some(area);
        mem::swap(&mut self.shown, &mut self.next);
        &self.shown
    }

    /// Lays out the frame that [`render`](Self::render) into `area` would draw, without
    /// drawing it, so the application can act on that frame first, such as following again in
    /// place where it shows the end. [`placement`](Self::placement) and [`area`](Self::area)
    /// stay at the frame last drawn. The render after it lays out again, which gives the same
    /// rows unless the feed changed in between, with an outcome of its own. An empty area lays
    /// out nothing and gives the last frame drawn.
    pub fn layout(&mut self, area: Rect, rows: &mut impl FeedRows<K>) -> &Placement {
        if area.is_empty() {
            return &self.shown;
        }
        self.measure_at(area.width, rows);
        self.feed
            .layout_into(u32::from(area.height), rows, &mut self.next);
        &self.next
    }

    /// Has `rows` measure at `width` from now on, and returns whether that's another width.
    fn measure_at(&mut self, width: u16, rows: &mut impl FeedRows<K>) -> bool {
        let changed = self.width != Some(width);
        if changed {
            // (Heights measured before, the first frame's included, were at another width or
            // at one the view never drew at.)
            self.feed.set_geometry();
            rows.set_width(width);
            self.width = Some(width);
        }
        changed
    }

    /// The widget that draws the view with `rows`, for
    /// [`Frame::render_widget`](crate::Frame::render_widget).
    #[must_use]
    pub fn widget<'a, R: FeedRows<K>>(&'a mut self, rows: &'a mut R) -> impl Widget + 'a {
        Drawn { view: self, rows }
    }

    /// The item under `position` in the last frame, and the row of it there.
    pub fn item_at(&self, position: Position) -> Option<(ItemId, u32)> {
        let area = self.area?;
        if !area.contains(position) {
            return None;
        }
        let y = i64::from(position.y - area.y);
        self.shown
            .items
            .iter()
            .find(|placed| placed.y <= y && y < placed.y + i64::from(placed.height))
            .map(|placed| (placed.item, u32::try_from(y - placed.y).unwrap_or(0)))
    }

    /// The selection's end at `position` in the last frame, which names content rather than a
    /// row, so it keeps its text when the view changes width.
    pub fn point_at(&self, position: Position, rows: &mut impl FeedRows<K>) -> Option<Point> {
        let (item, row) = self.item_at(position)?;
        let area = self.area?;
        let key = self.feed.key(item)?;
        Some(Point::new(
            item,
            rows.mark_at(key, row),
            row,
            u32::from(position.x - area.x),
        ))
    }

    /// How far the rows moved up between the last frame and this one, by the first item in
    /// both at the same height, unless they stayed.
    fn moved(&self) -> Option<i32> {
        for placed in &self.next.items {
            let Some(was) = self
                .shown
                .items
                .iter()
                .find(|was| was.item == placed.item && was.height == placed.height)
            else {
                continue;
            };
            let by = was.y - placed.y;
            return if by == 0 {
                None
            } else {
                i32::try_from(by).ok()
            };
        }
        None
    }
}

/// A [`FeedView`] with what draws its rows, as a [`Widget`].
struct Drawn<'a, K, R> {
    view: &'a mut FeedView<K>,
    rows: &'a mut R,
}

impl<K, R: FeedRows<K>> Widget for Drawn<'_, K, R> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.view.render(area, buf, self.rows);
    }
}
