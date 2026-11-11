//! Styled text as [`Span`]s, [`Line`]s and [`Text`], and the measure of text in cells.
//!
//! The types, their fields and their constructors are ratatui's, so code that builds text with
//! ratatui ports by changing its imports. Their content is a [`Cow`], so lines built from cached
//! strings borrow them.
//!
//! ```
//! use tuiframe::style::{Color, Modifier, Style};
//! use tuiframe::text::{Line, Span};
//!
//! let line = Line::from(vec![
//!     Span::styled("error", Style::new().fg(Color::Red).add_modifier(Modifier::BOLD)),
//!     Span::raw(": file not found"),
//! ]);
//! assert_eq!(line.width(), 21);
//! ```
//!
//! [`graphemes`] and [`width`] measure text as the buffer does, cluster by cluster under the
//! terminal's [`WidthProfile`], so a program's wrapping, cursor columns and alignment agree with
//! the cells it draws.

use std::borrow::Cow;
use std::cell::Cell as StdCell;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::caps::WidthProfile;
use crate::layout::Alignment;
use crate::style::Style;

mod tables;

/// A piece of text in one style.
///
/// ```
/// use tuiframe::style::{Color, Style};
/// use tuiframe::text::Span;
///
/// let name = String::from("main.rs");
/// let span = Span::styled(name.as_str(), Style::new().fg(Color::Cyan));
/// assert_eq!(span.content, "main.rs");
/// assert_eq!(span.width(), 7);
/// ```
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Span<'a> {
    /// The text.
    pub content: Cow<'a, str>,
    /// Its style, applied over the line's.
    pub style: Style,
}

impl<'a> Span<'a> {
    /// The text with no style of its own.
    pub fn raw<T: Into<Cow<'a, str>>>(content: T) -> Self {
        Self {
            content: content.into(),
            style: Style::new(),
        }
    }

    /// The text in `style`.
    pub fn styled<T: Into<Cow<'a, str>>, S: Into<Style>>(content: T, style: S) -> Self {
        Self {
            content: content.into(),
            style: style.into(),
        }
    }

    /// The span with its text replaced.
    #[must_use]
    pub fn content<T: Into<Cow<'a, str>>>(mut self, content: T) -> Self {
        self.content = content.into();
        self
    }

    /// The span with its style replaced.
    #[must_use]
    pub fn style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = style.into();
        self
    }

    /// The span with `style` patched over its own.
    #[must_use]
    pub fn patch_style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = self.style.patch(style.into());
        self
    }

    /// The span with no style of its own.
    #[must_use]
    pub fn reset_style(self) -> Self {
        self.style(Style::new())
    }

    /// The cells the text takes, as [`width`] measures it.
    pub fn width(&self) -> usize {
        width(&self.content)
    }
}

impl<'a, T: Into<Cow<'a, str>>> From<T> for Span<'a> {
    fn from(content: T) -> Self {
        Self::raw(content)
    }
}

impl fmt::Display for Span<'_> {
    /// Writes the text without its style.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.content)
    }
}

/// One row of text, as spans in a line style.
///
/// The line's style goes under each span's, so a span's own colours win. Building a line from a
/// string drops its line breaks, each line of the string becoming a span.
///
/// ```
/// use tuiframe::style::{Color, Style};
/// use tuiframe::text::{Line, Span};
///
/// let line = Line::from(vec![Span::raw("› "), Span::styled("ask", Style::new().fg(Color::Gray))])
///     .style(Style::new().bg(Color::Black));
/// assert_eq!(line.spans.len(), 2);
/// assert_eq!(line.to_string(), "› ask");
/// ```
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Line<'a> {
    /// The spans, from the left.
    pub spans: Vec<Span<'a>>,
    /// The style under every span.
    pub style: Style,
    /// Where the line sits in a wider area, for widgets that place it. `None` leaves it to the
    /// widget.
    pub alignment: Option<Alignment>,
}

impl<'a> Line<'a> {
    /// The text with no style, each of its lines a span.
    pub fn raw<T: Into<Cow<'a, str>>>(content: T) -> Self {
        Self {
            spans: cow_to_spans(content.into()),
            ..Self::default()
        }
    }

    /// The text in `style`, each of its lines a span.
    pub fn styled<T: Into<Cow<'a, str>>, S: Into<Style>>(content: T, style: S) -> Self {
        Self::raw(content).style(style)
    }

    /// The line with its spans replaced.
    #[must_use]
    pub fn spans<I, T>(mut self, spans: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<Span<'a>>,
    {
        self.spans = spans.into_iter().map(Into::into).collect();
        self
    }

    /// The line with its style replaced.
    #[must_use]
    pub fn style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = style.into();
        self
    }

    /// The line with `style` patched over its own.
    #[must_use]
    pub fn patch_style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = self.style.patch(style.into());
        self
    }

    /// The line with no style of its own. The spans keep theirs.
    #[must_use]
    pub fn reset_style(self) -> Self {
        self.style(Style::new())
    }

    /// The line placed as `alignment` says.
    #[must_use]
    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = Some(alignment);
        self
    }

    /// The line placed at the left.
    #[must_use]
    pub fn left_aligned(self) -> Self {
        self.alignment(Alignment::Left)
    }

    /// The line placed in the middle.
    #[must_use]
    pub fn centered(self) -> Self {
        self.alignment(Alignment::Center)
    }

    /// The line placed at the right.
    #[must_use]
    pub fn right_aligned(self) -> Self {
        self.alignment(Alignment::Right)
    }

    /// The cells the spans take, as [`width`] measures them.
    pub fn width(&self) -> usize {
        let profile = width_profile();
        self.spans
            .iter()
            .map(|span| width_with(&span.content, profile))
            .sum()
    }

    /// The spans, from the left.
    pub fn iter(&self) -> core::slice::Iter<'_, Span<'a>> {
        self.spans.iter()
    }

    /// The spans, from the left, to change in place.
    pub fn iter_mut(&mut self) -> core::slice::IterMut<'_, Span<'a>> {
        self.spans.iter_mut()
    }

    /// Adds a span at the right.
    pub fn push_span<T: Into<Span<'a>>>(&mut self, span: T) {
        self.spans.push(span.into());
    }
}

fn cow_to_spans(content: Cow<'_, str>) -> Vec<Span<'_>> {
    match content {
        Cow::Borrowed(s) => s.lines().map(Span::raw).collect(),
        Cow::Owned(s) => s.lines().map(|line| Span::raw(line.to_owned())).collect(),
    }
}

impl From<String> for Line<'_> {
    fn from(content: String) -> Self {
        Self::raw(content)
    }
}

impl<'a> From<&'a str> for Line<'a> {
    fn from(content: &'a str) -> Self {
        Self::raw(content)
    }
}

impl<'a> From<Cow<'a, str>> for Line<'a> {
    fn from(content: Cow<'a, str>) -> Self {
        Self::raw(content)
    }
}

impl<'a> From<Vec<Span<'a>>> for Line<'a> {
    fn from(spans: Vec<Span<'a>>) -> Self {
        Self {
            spans,
            ..Self::default()
        }
    }
}

impl<'a> From<Span<'a>> for Line<'a> {
    fn from(span: Span<'a>) -> Self {
        Self::from(vec![span])
    }
}

impl<'a, T: Into<Span<'a>>> FromIterator<T> for Line<'a> {
    fn from_iter<I: IntoIterator<Item = T>>(spans: I) -> Self {
        Self::from(spans.into_iter().map(Into::into).collect::<Vec<_>>())
    }
}

impl<'a> IntoIterator for Line<'a> {
    type Item = Span<'a>;
    type IntoIter = std::vec::IntoIter<Span<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.spans.into_iter()
    }
}

impl<'a, 'b> IntoIterator for &'b Line<'a> {
    type Item = &'b Span<'a>;
    type IntoIter = core::slice::Iter<'b, Span<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.spans.iter()
    }
}

impl<'a, 'b> IntoIterator for &'b mut Line<'a> {
    type Item = &'b mut Span<'a>;
    type IntoIter = core::slice::IterMut<'b, Span<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.spans.iter_mut()
    }
}

impl fmt::Display for Line<'_> {
    /// Writes the spans' text without styles.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.spans
            .iter()
            .try_for_each(|span| fmt::Display::fmt(span, f))
    }
}

/// Lines of text in a style, top to bottom.
///
/// ```
/// use tuiframe::text::Text;
///
/// let text = Text::from("first\nsecond, longer");
/// assert_eq!(text.height(), 2);
/// assert_eq!(text.width(), 14);
/// ```
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Text<'a> {
    /// The lines, from the top.
    pub lines: Vec<Line<'a>>,
    /// The style under every line.
    pub style: Style,
    /// Where lines without an alignment of their own sit.
    pub alignment: Option<Alignment>,
}

impl<'a> Text<'a> {
    /// The text with no style, a line for each of its lines. Empty text is one empty line.
    pub fn raw<T: Into<Cow<'a, str>>>(content: T) -> Self {
        let lines = match content.into() {
            Cow::Borrowed("") => vec![Line::default()],
            Cow::Borrowed(s) => s.lines().map(Line::from).collect(),
            Cow::Owned(s) if s.is_empty() => vec![Line::default()],
            Cow::Owned(s) => s.lines().map(|line| Line::from(line.to_owned())).collect(),
        };
        Self::from(lines)
    }

    /// The text in `style`, a line for each of its lines.
    pub fn styled<T: Into<Cow<'a, str>>, S: Into<Style>>(content: T, style: S) -> Self {
        Self::raw(content).style(style)
    }

    /// The cells the widest line takes, as [`width`] measures it.
    pub fn width(&self) -> usize {
        self.lines.iter().map(Line::width).max().unwrap_or(0)
    }

    /// The number of lines.
    pub fn height(&self) -> usize {
        self.lines.len()
    }

    /// The text with its style replaced.
    #[must_use]
    pub fn style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = style.into();
        self
    }

    /// The text with `style` patched over its own.
    #[must_use]
    pub fn patch_style<S: Into<Style>>(mut self, style: S) -> Self {
        self.style = self.style.patch(style.into());
        self
    }

    /// The text with no style of its own. The lines keep theirs.
    #[must_use]
    pub fn reset_style(self) -> Self {
        self.style(Style::new())
    }

    /// The text with lines placed as `alignment` says, where they don't say themselves.
    #[must_use]
    pub fn alignment(mut self, alignment: Alignment) -> Self {
        self.alignment = Some(alignment);
        self
    }

    /// The text placed at the left.
    #[must_use]
    pub fn left_aligned(self) -> Self {
        self.alignment(Alignment::Left)
    }

    /// The text placed in the middle.
    #[must_use]
    pub fn centered(self) -> Self {
        self.alignment(Alignment::Center)
    }

    /// The text placed at the right.
    #[must_use]
    pub fn right_aligned(self) -> Self {
        self.alignment(Alignment::Right)
    }

    /// The lines, from the top.
    pub fn iter(&self) -> core::slice::Iter<'_, Line<'a>> {
        self.lines.iter()
    }

    /// The lines, from the top, to change in place.
    pub fn iter_mut(&mut self) -> core::slice::IterMut<'_, Line<'a>> {
        self.lines.iter_mut()
    }

    /// Adds a line at the bottom.
    pub fn push_line<T: Into<Line<'a>>>(&mut self, line: T) {
        self.lines.push(line.into());
    }

    /// Adds a span at the end of the last line, or as a new line when there's none.
    pub fn push_span<T: Into<Span<'a>>>(&mut self, span: T) {
        match self.lines.last_mut() {
            Some(line) => line.push_span(span),
            None => self.lines.push(Line::from(span.into())),
        }
    }
}

impl From<String> for Text<'_> {
    fn from(content: String) -> Self {
        Self::raw(content)
    }
}

impl<'a> From<&'a str> for Text<'a> {
    fn from(content: &'a str) -> Self {
        Self::raw(content)
    }
}

impl<'a> From<Cow<'a, str>> for Text<'a> {
    fn from(content: Cow<'a, str>) -> Self {
        Self::raw(content)
    }
}

impl<'a> From<Span<'a>> for Text<'a> {
    fn from(span: Span<'a>) -> Self {
        Self::from(Line::from(span))
    }
}

impl<'a> From<Line<'a>> for Text<'a> {
    fn from(line: Line<'a>) -> Self {
        Self::from(vec![line])
    }
}

impl<'a> From<Vec<Line<'a>>> for Text<'a> {
    fn from(lines: Vec<Line<'a>>) -> Self {
        Self {
            lines,
            ..Self::default()
        }
    }
}

impl<'a, T: Into<Line<'a>>> FromIterator<T> for Text<'a> {
    fn from_iter<I: IntoIterator<Item = T>>(lines: I) -> Self {
        Self::from(lines.into_iter().map(Into::into).collect::<Vec<_>>())
    }
}

impl<'a, T: Into<Line<'a>>> Extend<T> for Text<'a> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, lines: I) {
        self.lines.extend(lines.into_iter().map(Into::into));
    }
}

impl<'a> IntoIterator for Text<'a> {
    type Item = Line<'a>;
    type IntoIter = std::vec::IntoIter<Line<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.lines.into_iter()
    }
}

impl<'a, 'b> IntoIterator for &'b Text<'a> {
    type Item = &'b Line<'a>;
    type IntoIter = core::slice::Iter<'b, Line<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.lines.iter()
    }
}

impl<'a, 'b> IntoIterator for &'b mut Text<'a> {
    type Item = &'b mut Line<'a>;
    type IntoIter = core::slice::IterMut<'b, Line<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.lines.iter_mut()
    }
}

impl fmt::Display for Text<'_> {
    /// Writes the lines' text without styles, with a line break between lines.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, line) in self.lines.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            fmt::Display::fmt(line, f)?;
        }
        Ok(())
    }
}

/// The grapheme clusters of `text` with the cells each takes under `profile`, from [`graphemes`].
#[derive(Debug, Clone)]
pub struct Graphemes<'a> {
    clusters: unicode_segmentation::Graphemes<'a>,
    profile: WidthProfile,
}

impl<'a> Iterator for Graphemes<'a> {
    type Item = (&'a str, u16);

    fn next(&mut self) -> Option<Self::Item> {
        let cluster = self.clusters.next()?;
        Some((cluster, u16::from(measure(cluster, self.profile).width)))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.clusters.size_hint()
    }
}

impl DoubleEndedIterator for Graphemes<'_> {
    fn next_back(&mut self) -> Option<Self::Item> {
        let cluster = self.clusters.next_back()?;
        Some((cluster, u16::from(measure(cluster, self.profile).width)))
    }
}

impl core::iter::FusedIterator for Graphemes<'_> {}

/// The profile every thread measures with unless it set its own, three bits of a
/// [`WidthProfile`].
static PROFILE: AtomicU8 = AtomicU8::new(0);

thread_local! {
    /// The profile this thread measures with, inside [`with_width_profile`].
    static THREAD_PROFILE: StdCell<Option<WidthProfile>> = const { StdCell::new(None) };
}

const fn profile_bits(profile: WidthProfile) -> u8 {
    profile.joins_emoji as u8
        | (profile.vs16_widens as u8) << 1
        | (profile.ambiguous_wide as u8) << 2
}

const fn profile_from_bits(bits: u8) -> WidthProfile {
    WidthProfile {
        joins_emoji: bits & 1 != 0,
        vs16_widens: bits & 2 != 0,
        ambiguous_wide: bits & 4 != 0,
    }
}

/// The width profile text is measured with, which [`width`], [`graphemes`], the `width` methods
/// of the text types and [`Buffer::empty`](crate::Buffer::empty) use.
///
/// It's the terminal's, from the moment the terminal opens, so a program's wrapping, cursor columns
/// and alignment agree with the cells it draws. Before that, and with no terminal, it's the
/// default, and [`with_width_profile`] sets one for a thread.
pub fn width_profile() -> WidthProfile {
    THREAD_PROFILE
        .try_with(StdCell::get)
        .ok()
        .flatten()
        .unwrap_or_else(|| profile_from_bits(PROFILE.load(Ordering::Relaxed)))
}

/// Runs `f` with text on this thread measured by `profile`, and the profile before it back
/// afterwards, even when `f` panics. Tests that draw for a terminal of their own use it, since
/// tests run on threads side by side.
///
/// ```
/// use tuiframe::caps::WidthProfile;
/// use tuiframe::text::{self, Span};
///
/// let kitty = WidthProfile { joins_emoji: true, vs16_widens: true, ambiguous_wide: false };
/// let family = Span::raw("👨\u{200d}👩\u{200d}👧");
/// assert_eq!(text::with_width_profile(kitty, || family.width()), 2);
/// assert_eq!(text::with_width_profile(WidthProfile::default(), || family.width()), 6);
/// ```
pub fn with_width_profile<R>(profile: WidthProfile, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<WidthProfile>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let before = self.0;
            let _ = THREAD_PROFILE.try_with(|p| p.set(before));
        }
    }
    let before = THREAD_PROFILE
        .try_with(|p| p.replace(Some(profile)))
        .ok()
        .flatten();
    let _restore = Restore(before);
    f()
}

/// Makes `profile` the one every thread measures with, unless it set its own. The terminal sets
/// the one it found when it opens.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "`Terminal::open` sets it, and comes with the terminal API"
    )
)]
pub(crate) fn set_width_profile(profile: WidthProfile) {
    PROFILE.store(profile_bits(profile), Ordering::Relaxed);
}

/// The grapheme clusters of `text`, each with the cells it takes in a buffer that measures with
/// the current [`width_profile`].
///
/// A cluster that holds a control character or has no width takes 0 cells, and the buffer skips
/// it, so the widths add up to what writing the text takes.
///
/// ```
/// use tuiframe::text;
///
/// let clusters: Vec<_> = text::graphemes("e\u{301}界\t").collect();
/// assert_eq!(clusters, [("e\u{301}", 1), ("界", 2), ("\t", 0)]);
/// ```
pub fn graphemes(text: &str) -> Graphemes<'_> {
    graphemes_with(text, width_profile())
}

/// [`graphemes`] under `profile`.
pub fn graphemes_with(text: &str, profile: WidthProfile) -> Graphemes<'_> {
    Graphemes {
        clusters: text.graphemes(true),
        profile,
    }
}

/// The cells `text` takes in a buffer that measures with the current [`width_profile`], the
/// widths of [`graphemes`] added up.
///
/// ```
/// use tuiframe::text;
///
/// assert_eq!(text::width("a界b"), 4);
/// ```
pub fn width(text: &str) -> usize {
    width_with(text, width_profile())
}

/// [`width`] under `profile`. A family emoji is three wide emoji to a terminal that doesn't join
/// emoji sequences, and one to a terminal that does.
///
/// ```
/// use tuiframe::caps::WidthProfile;
/// use tuiframe::text;
///
/// let family = "👨\u{200d}👩\u{200d}👧";
/// assert_eq!(text::width_with(family, WidthProfile::default()), 6);
/// let joining = WidthProfile { joins_emoji: true, ..WidthProfile::default() };
/// assert_eq!(text::width_with(family, joining), 2);
/// ```
pub fn width_with(text: &str, profile: WidthProfile) -> usize {
    if text.is_ascii() {
        return text.bytes().filter(|b| !b.is_ascii_control()).count();
    }
    graphemes_with(text, profile)
        .map(|(_, w)| usize::from(w))
        .sum()
}

/// The widest cluster the buffer gives the cells it really takes, beyond which a cluster is cut
/// to this.
pub(crate) const MAX_CLUSTER_WIDTH: u8 = 7;

/// What the buffer keeps of a cluster's measure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Measure {
    /// The cells it takes, 0 for a cluster the buffer skips.
    pub(crate) width: u8,
    /// Whether terminals may disagree on that width.
    pub(crate) uncertain: bool,
}

const SKIPPED: Measure = Measure {
    width: 0,
    uncertain: false,
};

/// Measures one grapheme cluster under `profile`.
pub(crate) fn measure(cluster: &str, profile: WidthProfile) -> Measure {
    if let [byte] = cluster.as_bytes() {
        return Measure {
            width: u8::from(!byte.is_ascii_control()),
            uncertain: false,
        };
    }
    let mut chars = cluster.chars();
    let Some(first) = chars.next() else {
        return SKIPPED;
    };
    if chars.as_str().is_empty() {
        measure_char(first, profile)
    } else {
        measure_sequence(cluster, profile)
    }
}

/// The cells `c` takes on its own. That's unicode-width's width, except that an East Asian
/// ambiguous character takes two under `ambiguous_wide`, and the halfwidth katakana sound marks
/// take one, as terminals draw them as characters of their own.
fn char_width(c: char, profile: WidthProfile) -> usize {
    let width = match c {
        '\u{FF9E}' | '\u{FF9F}' => 1,
        _ => c.width().unwrap_or(0),
    };
    if width == 1 && profile.ambiguous_wide && is_ambiguous(c) {
        2
    } else {
        width
    }
}

/// The cells a whole cluster takes in a terminal that joins emoji sequences, from unicode-width's
/// width of the string, with the same exceptions as [`char_width`].
fn joined_width(cluster: &str, profile: WidthProfile) -> usize {
    let marks = cluster
        .chars()
        .filter(|c| matches!(c, '\u{FF9E}' | '\u{FF9F}'))
        .count();
    let width = cluster.width() + marks;
    match cluster.chars().next() {
        Some(base) if width == 1 && profile.ambiguous_wide && is_ambiguous(base) => 2,
        _ => width,
    }
}

fn clamp_width(width: usize, max: u8) -> u8 {
    u8::try_from(width).map_or(max, |w| w.min(max))
}

fn measure_char(c: char, profile: WidthProfile) -> Measure {
    if c.is_control() {
        return SKIPPED;
    }
    let width = clamp_width(char_width(c, profile), MAX_CLUSTER_WIDTH);
    Measure {
        width,
        uncertain: width > 0 && is_recent_emoji(c),
    }
}

/// Measures a cluster of more than one scalar value.
///
/// Without `joins_emoji`, the cluster takes the sum of its scalars' widths, as a terminal that
/// draws each scalar on its own does. With it, the cluster takes unicode-width's width of the whole
/// string, at most 2. `vs16_widens` makes a text symbol followed by U+FE0F wide in both. The width
/// is uncertain where the two measures differ, or the cluster is an emoji sequence.
fn measure_sequence(cluster: &str, profile: WidthProfile) -> Measure {
    let mut sum = 0;
    let mut uncertain = false;
    let mut joined = false;
    let mut selected = false;
    let mut previous: Option<(usize, usize)> = None;
    for (i, c) in cluster.char_indices() {
        if c.is_control() {
            return SKIPPED;
        }
        match c {
            '\u{200D}' | '\u{1F3FB}'..='\u{1F3FF}' => joined = true,
            '\u{FE0F}' => selected = true,
            _ => {}
        }
        uncertain |= is_sequence_mark(c) || is_recent_emoji(c);
        let own = char_width(c, profile);
        let widened = c == '\u{FE0F}'
            && profile.vs16_widens
            && previous.is_some_and(|(start, w)| w == 1 && cluster[start..i + 3].width() == 2);
        sum += if widened { 1 } else { own };
        previous = Some((i, own));
    }
    let summed = clamp_width(sum, MAX_CLUSTER_WIDTH);
    let whole = if joined || profile.vs16_widens || !selected {
        clamp_width(joined_width(cluster, profile), 2)
    } else {
        clamp_width(sum, 2)
    };
    let width = if profile.joins_emoji { whole } else { summed };
    // A terminal that joins clusters and one that doesn't would place what follows differently.
    uncertain |= summed != whole;
    Measure { width, uncertain }
}

/// Whether `c` is one of the marks that make an emoji sequence, whose width terminals disagree on.
fn is_sequence_mark(c: char) -> bool {
    matches!(
        c,
        '\u{200D}'
            | '\u{FE0E}'
            | '\u{FE0F}'
            | '\u{20E3}'
            | '\u{1F1E6}'..='\u{1F1FF}'
            | '\u{1F3FB}'..='\u{1F3FF}'
            | '\u{E0020}'..='\u{E007F}'
    )
}

/// Whether `c` is East Asian ambiguous, one cell in most terminals and two in those set for East
/// Asian text.
///
/// These don't count as uncertain widths. They include box drawing, the ellipsis and the Greek and
/// Cyrillic alphabets, which a resync after each cell would make several times longer to write,
/// and every measured terminal draws them narrow unless set otherwise, which the profile says.
fn is_ambiguous(c: char) -> bool {
    c >= '\u{A1}' && in_ranges(tables::AMBIGUOUS, c)
}

fn is_recent_emoji(c: char) -> bool {
    c >= '\u{26A7}' && in_ranges(tables::RECENT_EMOJI, c)
}

fn in_ranges(ranges: &[(char, char)], c: char) -> bool {
    ranges
        .binary_search_by(|&(first, last)| {
            if last < c {
                core::cmp::Ordering::Less
            } else if first > c {
                core::cmp::Ordering::Greater
            } else {
                core::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::{Color, Modifier};

    const PLAIN: WidthProfile = WidthProfile {
        joins_emoji: false,
        vs16_widens: false,
        ambiguous_wide: false,
    };

    fn profile(joins_emoji: bool, vs16_widens: bool, ambiguous_wide: bool) -> WidthProfile {
        WidthProfile {
            joins_emoji,
            vs16_widens,
            ambiguous_wide,
        }
    }

    #[test]
    fn spans_build_as_ratatui_does() {
        let owned = Span::raw(String::from("abc"));
        assert!(matches!(owned.content, Cow::Owned(_)));
        let borrowed = Span::from("abc");
        assert!(matches!(borrowed.content, Cow::Borrowed(_)));
        assert_eq!(owned, borrowed);

        let style = Style::new().fg(Color::Red);
        let span = Span::styled("x", style);
        assert_eq!(span.style, style);
        let span = span.patch_style(Style::new().add_modifier(Modifier::BOLD));
        assert_eq!(span.style, style.add_modifier(Modifier::BOLD));
        assert_eq!(span.clone().reset_style().style, Style::new());
        assert_eq!(span.content("yz").content, "yz");
        assert_eq!(Span::raw("ab界").to_string(), "ab界");
    }

    #[test]
    fn lines_split_strings_into_spans() {
        assert_eq!(Line::raw("a\nb").spans, [Span::raw("a"), Span::raw("b")]);
        assert_eq!(Line::from(String::from("a\r\nb")).spans.len(), 2);
        assert!(Line::raw("").spans.is_empty());
        let line = Line::styled("ab", Style::new().fg(Color::Blue));
        assert_eq!(line.style, Style::new().fg(Color::Blue));
        assert_eq!(line.spans, [Span::raw("ab")]);
    }

    #[test]
    fn lines_build_from_spans() {
        let line: Line<'_> = ["a", "bc"].into_iter().collect();
        assert_eq!(line.spans, [Span::raw("a"), Span::raw("bc")]);
        assert_eq!(line.width(), 3);
        assert_eq!(line.to_string(), "abc");
        assert_eq!(Line::from(Span::raw("x")).spans, [Span::raw("x")]);
        let mut line = Line::default().spans(["a"]);
        line.push_span("界");
        assert_eq!(line.width(), 3);
        assert_eq!(line.iter().count(), 2);
        for span in &mut line {
            span.style = Style::new().fg(Color::Green);
        }
        assert!(line.iter().all(|s| s.style.fg == Some(Color::Green)));
        assert_eq!(line.clone().centered().alignment, Some(Alignment::Center));
        assert_eq!(line.clone().left_aligned().alignment, Some(Alignment::Left));
        assert_eq!(line.right_aligned().alignment, Some(Alignment::Right));
    }

    #[test]
    fn text_splits_lines() {
        assert_eq!(Text::raw("").lines, [Line::default()]);
        assert_eq!(Text::raw(String::new()).lines, [Line::default()]);
        let text = Text::from("one\ntwo three");
        assert_eq!(text.height(), 2);
        assert_eq!(text.width(), 9);
        assert_eq!(text.to_string(), "one\ntwo three");
        let mut text = Text::default();
        text.push_span("a");
        text.push_span("b");
        text.push_line("c");
        assert_eq!(text.to_string(), "ab\nc");
        text.extend(["d"]);
        assert_eq!(text.height(), 3);
        let text: Text<'_> = ["x", "y"].into_iter().collect();
        assert_eq!(text.lines.len(), 2);
        assert_eq!(Text::from(Span::raw("s")).lines, [Line::from("s")]);
        assert_eq!(
            Text::styled("s", Style::new().fg(Color::Red)).style,
            Style::new().fg(Color::Red)
        );
    }

    #[test]
    fn controls_and_zero_width_clusters_take_no_cells() {
        assert_eq!(measure("\t", PLAIN).width, 0);
        assert_eq!(measure("\r\n", PLAIN).width, 0);
        assert_eq!(measure("\u{7f}", PLAIN).width, 0);
        assert_eq!(measure("\u{9b}", PLAIN).width, 0);
        assert_eq!(measure("\u{200b}", PLAIN).width, 0);
        assert_eq!(measure("\u{301}", PLAIN).width, 0);
        assert_eq!(measure("", PLAIN).width, 0);
        assert_eq!(width_with("a\tb\u{1b}c", PLAIN), 3);
    }

    #[test]
    fn combining_marks_join_their_base() {
        let m = measure("e\u{301}", PLAIN);
        assert_eq!((m.width, m.uncertain), (1, false));
        assert_eq!(measure("界\u{301}", PLAIN).width, 2);
        assert_eq!(measure("\u{1100}\u{1161}\u{11a8}", PLAIN).width, 2);
    }

    #[test]
    fn emoji_sequences_sum_unless_the_profile_joins_them() {
        let thumbs = "👍🏽";
        assert_eq!(measure(thumbs, PLAIN).width, 4);
        assert_eq!(measure(thumbs, profile(true, false, false)).width, 2);
        let hands = "🧑🏻\u{200d}🤝\u{200d}🧑🏼";
        assert_eq!(measure(hands, PLAIN).width, MAX_CLUSTER_WIDTH);
        assert_eq!(measure(hands, profile(true, true, false)).width, 2);
        assert!(measure(hands, PLAIN).uncertain);
        let flag = "🇳🇿";
        assert_eq!(measure(flag, PLAIN).width, 2);
        assert_eq!(measure(flag, profile(true, true, true)).width, 2);
        assert!(measure(flag, PLAIN).uncertain);
    }

    #[test]
    fn vs16_widens_text_symbols_only_where_the_profile_says() {
        let heart = "❤\u{fe0f}";
        assert_eq!(measure(heart, PLAIN).width, 1);
        assert_eq!(measure(heart, profile(false, true, false)).width, 2);
        assert_eq!(measure(heart, profile(true, false, false)).width, 1);
        assert_eq!(measure(heart, profile(true, true, false)).width, 2);
        assert!(measure(heart, PLAIN).uncertain);
        let not_emoji = "a\u{fe0f}";
        assert_eq!(measure(not_emoji, profile(false, true, false)).width, 1);
        assert_eq!(measure(not_emoji, profile(true, true, false)).width, 1);
        let keycap = "1\u{fe0f}\u{20e3}";
        assert_eq!(measure(keycap, PLAIN).width, 1);
        assert_eq!(measure(keycap, profile(false, true, false)).width, 2);
    }

    #[test]
    fn ambiguous_characters_follow_the_profile_and_are_not_uncertain() {
        let m = measure("±", PLAIN);
        assert_eq!((m.width, m.uncertain), (1, false));
        assert_eq!(measure("±", profile(false, false, true)).width, 2);
        assert_eq!(measure("é", profile(false, false, true)).width, 2);
        assert_eq!(measure("─", profile(true, true, true)).width, 2);
        assert_eq!(measure("é", PLAIN).width, 1);
        assert_eq!(measure("e\u{301}", profile(true, false, true)).width, 1);
        assert_eq!(
            measure("\u{e0}\u{301}", profile(true, false, true)).width,
            2
        );
        assert_eq!(
            measure("\u{e0}\u{301}", profile(false, false, true)).width,
            2
        );
        let m = measure("a", PLAIN);
        assert!(!m.uncertain);
        let m = measure("界", PLAIN);
        assert_eq!((m.width, m.uncertain), (2, false));
    }

    #[test]
    fn profiles_round_trip_through_their_bits() {
        for bits in 0..8 {
            let profile = profile_from_bits(bits);
            assert_eq!(profile_bits(profile), bits);
        }
    }

    #[test]
    fn a_thread_measures_with_its_own_profile_inside_with_width_profile() {
        let joining = profile(true, true, false);
        let family = "👨\u{200d}👩\u{200d}👧";
        let outside = width_profile();
        let inside = with_width_profile(joining, || {
            let nested = with_width_profile(PLAIN, || width(family));
            (
                width_profile(),
                width(family),
                Span::raw(family).width(),
                nested,
                width_profile(),
            )
        });
        assert_eq!(inside, (joining, 2, 2, 6, joining));
        assert_eq!(width_profile(), outside);
        let other_thread = std::thread::spawn(move || with_width_profile(joining, || 0)).join();
        assert!(other_thread.is_ok());
        let after_panic =
            std::panic::catch_unwind(|| with_width_profile(joining, || panic!("in f")));
        assert!(after_panic.is_err());
        assert_eq!(
            width_profile(),
            outside,
            "a panic in f gives the profile back"
        );
    }

    #[test]
    fn the_process_profile_reaches_threads_without_their_own() {
        // Setting it to what it already is keeps other tests, which run alongside, unaffected.
        let current = profile_from_bits(PROFILE.load(Ordering::Relaxed));
        set_width_profile(current);
        let seen = std::thread::spawn(width_profile).join().ok();
        assert_eq!(seen, Some(current));
        let joining = profile(true, false, false);
        let shadowed =
            with_width_profile(joining, || std::thread::spawn(width_profile).join().ok());
        assert_eq!(
            shadowed,
            Some(current),
            "an override belongs to its own thread"
        );
    }

    #[test]
    fn clusters_the_two_measures_disagree_on_are_uncertain() {
        let mark = measure("界\u{903}", PLAIN);
        assert_eq!((mark.width, mark.uncertain), (3, true));
        assert_eq!(measure("界\u{903}", profile(true, false, false)).width, 2);
        assert!(!measure("e\u{301}", PLAIN).uncertain);
        assert!(!measure("\u{1100}\u{1161}\u{11a8}", PLAIN).uncertain);
    }

    #[test]
    fn halfwidth_sound_marks_take_a_cell() {
        assert_eq!(measure("\u{ff9e}", PLAIN).width, 1);
        assert_eq!(measure("\u{ff76}\u{ff9e}", PLAIN).width, 2);
        assert_eq!(
            measure("\u{ff76}\u{ff9e}", profile(true, false, false)).width,
            2
        );
    }

    #[test]
    fn recent_emoji_are_uncertain() {
        assert!(measure("🫨", PLAIN).uncertain);
        assert!(measure("🥱", PLAIN).uncertain);
        assert!(!measure("😀", PLAIN).uncertain);
        for table in [tables::RECENT_EMOJI, tables::AMBIGUOUS] {
            assert!(table.windows(2).all(|w| w[0].1 < w[1].0));
            assert!(table.iter().all(|&(a, b)| a <= b));
        }
    }

    #[test]
    fn graphemes_and_width_agree() {
        let text = "a界👍🏽e\u{301}\u{1b}x";
        for p in [PLAIN, profile(true, true, true)] {
            let sum: usize = graphemes_with(text, p).map(|(_, w)| usize::from(w)).sum();
            assert_eq!(width_with(text, p), sum);
        }
        let back: Vec<_> = graphemes_with("ab界", PLAIN).rev().collect();
        assert_eq!(back, [("界", 2), ("b", 1), ("a", 1)]);
    }
}
