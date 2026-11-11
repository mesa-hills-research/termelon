//! Breaking lines into rows by words, for [`wrap()`] and the paragraph widget.

use std::borrow::Cow;

use unicode_segmentation::UnicodeSegmentation;

use super::{Line, Span, measure, width_profile};
use crate::caps::WidthProfile;

/// A grapheme cluster of a line, as the wrapper reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Piece<'a> {
    /// The cluster's text.
    pub(crate) symbol: &'a str,
    /// The cells it takes.
    pub(crate) width: u16,
    /// Whether it separates words.
    pub(crate) space: bool,
    /// The index of its span in the line.
    pub(crate) span: usize,
    /// Its byte offset in the span's content.
    pub(crate) offset: usize,
}

/// One row of a line, the pieces `start..end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) start: usize,
    pub(crate) end: usize,
    /// The cells its pieces take.
    pub(crate) width: u16,
}

impl Row {
    const EMPTY: Self = Self {
        start: 0,
        end: 0,
        width: 0,
    };
}

/// Adds the clusters of `line` that a row of `max` cells can show to `out`, measured with
/// `profile`.
pub(crate) fn pieces<'a>(
    line: &'a Line<'_>,
    max: u16,
    profile: WidthProfile,
    out: &mut Vec<Piece<'a>>,
) {
    for (span, s) in line.spans.iter().enumerate() {
        for (offset, symbol) in s.content.grapheme_indices(true) {
            if symbol.chars().any(char::is_control) {
                continue;
            }
            let width = u16::from(measure(symbol, profile).width);
            if width > max {
                continue;
            }
            out.push(Piece {
                symbol,
                width,
                space: is_space(symbol),
                span,
                offset,
            });
        }
    }
}

/// Whether `cluster` separates words. A zero-width space does and a no-break space doesn't.
fn is_space(cluster: &str) -> bool {
    cluster == "\u{200b}" || (cluster != "\u{a0}" && cluster.chars().all(char::is_whitespace))
}

/// Consecutive pieces, and the cells they take.
#[derive(Debug, Clone, Copy)]
struct Run {
    start: usize,
    end: usize,
    width: usize,
}

impl Run {
    const fn at(i: usize) -> Self {
        Self {
            start: i,
            end: i,
            width: 0,
        }
    }

    const fn is_empty(self) -> bool {
        self.start == self.end
    }

    /// Adds piece `i`, which follows the run's last.
    fn push(&mut self, i: usize, width: usize) {
        if self.is_empty() {
            self.start = i;
        }
        self.end = i + 1;
        self.width += width;
    }

    /// Adds `other`, which follows this run.
    fn append(&mut self, other: Self) {
        if other.is_empty() {
            return;
        }
        if self.is_empty() {
            *self = other;
        } else {
            self.end = other.end;
            self.width += other.width;
        }
    }

    fn row(self) -> Row {
        Row {
            start: self.start,
            end: self.end,
            width: u16::try_from(self.width).unwrap_or(u16::MAX),
        }
    }
}

/// Adds the rows of at most `max` cells that `pieces`, a line's, wrap into to `rows`. An empty
/// line, and one that is only spaces when `trim` drops them, gives one empty row. A `max` of 0
/// gives none.
///
/// The row being built, the spaces after it and the word after those grow a piece at a time. A
/// word joins the row with the spaces before it once a space ends it, or once it can't fit on a
/// row of its own. The row is finished when it's full, when what waits to join it leaves no cell
/// for the next piece (ratatui's rule), or when the next piece, a wide one, would take the row
/// past `max` with what it keeps of that. The last rule is the one ratatui lacks, where a wide
/// character can overflow its row by a cell.
pub(crate) fn word_rows(pieces: &[Piece<'_>], max: u16, trim: bool, rows: &mut Vec<Row>) {
    let max = usize::from(max);
    if max == 0 {
        return;
    }
    let first = rows.len();
    let mut line = Run::at(0);
    let mut spaces = Run::at(0);
    let mut word = Run::at(0);
    let mut after_word = false;
    'pieces: for (i, piece) in pieces.iter().enumerate() {
        let width = usize::from(piece.width);
        if width > max {
            continue;
        }
        let mut ends_word = after_word && piece.space;
        loop {
            let alone = line.is_empty();
            let too_long = alone
                && if trim {
                    word.width + width > max || spaces.width + width > max
                } else {
                    word.width + spaces.width + width > max
                };
            if ends_word || too_long {
                if !(alone && trim) {
                    line.append(spaces);
                }
                line.append(word);
                spaces = Run::at(i);
                word = Run::at(i);
            }
            ends_word = false;
            let full = line.width >= max;
            let pending = line.width + spaces.width + word.width;
            let kept = if line.is_empty() && trim {
                word.width
            } else {
                pending
            };
            let overflows = width > 0 && (pending >= max || kept + width > max);
            if !full && !overflows {
                break;
            }
            let mut room = max.saturating_sub(line.width);
            rows.push(line.row());
            line = Run::at(i);
            while let Some(next) = pieces.get(spaces.start).filter(|_| !spaces.is_empty()) {
                let w = usize::from(next.width);
                if w > room {
                    break;
                }
                room -= w;
                spaces.width -= w;
                spaces.start += 1;
            }
            if piece.space && spaces.is_empty() {
                continue 'pieces;
            }
        }
        if piece.space {
            spaces.push(i, width);
        } else {
            word.push(i, width);
        }
        after_word = !piece.space;
    }
    if line.is_empty() && word.is_empty() && !spaces.is_empty() && trim {
        rows.push(Row::EMPTY);
    }
    if !line.is_empty() || !trim {
        line.append(spaces);
    }
    line.append(word);
    if !line.is_empty() {
        rows.push(line.row());
    }
    if rows.len() == first {
        rows.push(Row::EMPTY);
    }
}

/// The one row of at most `max` cells that `pieces`, a line's, show without wrapping, after the
/// first `skip` cells. A wide character that `skip` would cut is shown whole.
pub(crate) fn truncated_row(pieces: &[Piece<'_>], max: u16, skip: u16) -> Row {
    let max = usize::from(max);
    let mut skip = usize::from(skip);
    let mut run = Run::at(0);
    for (i, piece) in pieces.iter().enumerate() {
        let width = usize::from(piece.width);
        if run.width + width > max {
            break;
        }
        if skip > 0 && width <= skip {
            skip -= width;
            run = Run::at(i + 1);
            continue;
        }
        skip = 0;
        run.push(i, width);
    }
    run.row()
}

/// The spans of `line` that `row`, pieces of it, covers, joined where pieces are next to each
/// other in one span.
fn row_spans<'a>(line: &'a Line<'_>, row: &[Piece<'a>]) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    let mut flush = |run: Option<(usize, usize, usize)>| {
        if let Some((i, start, end)) = run
            && let Some(span) = line.spans.get(i)
            && let Some(content) = span.content.get(start..end)
        {
            spans.push(Span {
                content: Cow::Borrowed(content),
                style: span.style,
            });
        }
    };
    let mut run = None;
    for piece in row {
        let end = piece.offset + piece.symbol.len();
        match &mut run {
            Some((i, _, run_end)) if *i == piece.span && *run_end == piece.offset => *run_end = end,
            _ => {
                flush(run);
                run = Some((piece.span, piece.offset, end));
            }
        }
    }
    flush(run);
    spans
}

/// Breaks `line` into rows of at most `width` cells between words, measuring clusters with the
/// current [`width_profile`](super::width_profile), as a paragraph with
/// [`Wrap`](crate::widgets::Wrap) does.
///
/// Each row is a line with the input's style and alignment. Its spans borrow the input's text and
/// keep their styles, so a span that crosses a break goes on in its style on the next row.
/// Whitespace separates words, a zero-width space included and a no-break space not. A row breaks
/// inside a word only when the word doesn't fit on a row of its own, counting the spaces before
/// it unless `trim` drops them, and never inside a grapheme cluster. Spaces at a break are dropped
/// as far as the row had room for them, and the rest start the next row. `trim` drops the spaces
/// that start a row as well, but the last row keeps the spaces after its last word. Clusters that
/// hold a control character, or that are wider than `width`, are left out. An empty line gives one
/// empty row, and a `width` of 0 gives none.
///
/// The rules are ratatui's word wrapper's, except that a wide character that would cross the
/// row's edge starts the next row.
///
/// ```
/// use termelon::style::{Color, Style};
/// use termelon::text::{self, Line, Span};
///
/// let yellow = Style::new().fg(Color::Yellow);
/// let line = Line::from(vec![
///     Span::raw("the "),
///     Span::styled("quick brown", yellow),
///     Span::raw(" fox"),
/// ]);
/// let rows = text::wrap(&line, 10, true);
/// assert_eq!(
///     rows,
///     [
///         Line::from(vec![Span::raw("the "), Span::styled("quick", yellow)]),
///         Line::from(vec![Span::styled("brown", yellow), Span::raw(" fox")]),
///     ]
/// );
/// ```
pub fn wrap<'a>(line: &'a Line<'_>, width: u16, trim: bool) -> Vec<Line<'a>> {
    wrap_with(line, width, trim, width_profile())
}

/// [`wrap()`] under `profile`.
pub fn wrap_with<'a>(
    line: &'a Line<'_>,
    width: u16,
    trim: bool,
    profile: WidthProfile,
) -> Vec<Line<'a>> {
    let mut found = Vec::new();
    pieces(line, width, profile, &mut found);
    let mut rows = Vec::new();
    word_rows(&found, width, trim, &mut rows);
    rows.iter()
        .map(|row| Line {
            spans: row_spans(line, found.get(row.start..row.end).unwrap_or_default()),
            style: line.style,
            alignment: line.alignment,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::style::{Color, Style};

    const PIECES: &[&str] = &[
        "a", "bc", "def", " ", "  ", "界", "ｗｉ", "e\u{301}", "👍🏽", "🇳🇿", "\u{a0}", "\u{200b}",
        "\t", "x",
    ];

    /// The clusters of `text` that rows of `width` cells keep, without spaces.
    fn words(text: &str, width: u16, profile: WidthProfile) -> String {
        text.graphemes(true)
            .filter(|g| !is_space(g) && !g.chars().any(char::is_control))
            .filter(|g| u16::from(measure(g, profile).width) <= width)
            .collect()
    }

    proptest! {
        #[test]
        fn rows_fit_their_width_and_keep_every_word(
            parts in prop::collection::vec(prop::sample::select(PIECES), 0..30),
            width in 1u16..12,
            trim: bool,
            joins_emoji: bool,
        ) {
            let text = parts.concat();
            let profile = WidthProfile { joins_emoji, ..WidthProfile::default() };
            let line = Line::from(text.as_str());
            let rows = wrap_with(&line, width, trim, profile);
            prop_assert!(!rows.is_empty());
            let mut kept = String::new();
            for row in &rows {
                let row = row.to_string();
                prop_assert!(super::super::width_with(&row, profile) <= usize::from(width), "{row:?}");
                kept.push_str(&words(&row, width, profile));
            }
            prop_assert_eq!(kept, words(&text, width, profile));
        }
    }

    fn rows(text: &str, width: u16, trim: bool) -> Vec<String> {
        wrap_with(&Line::from(text), width, trim, WidthProfile::default())
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    fn both(text: &str, width: u16) -> [Vec<String>; 2] {
        [rows(text, width, false), rows(text, width, true)]
    }

    #[test]
    fn words_go_to_the_next_row_whole() {
        assert_eq!(
            rows("the quick brown fox", 10, true),
            ["the quick", "brown fox"]
        );
        assert_eq!(rows("aa bb cc", 5, false), ["aa bb", "cc"]);
        assert_eq!(rows("ab", 2, true), ["ab"]);
    }

    #[test]
    fn a_word_longer_than_the_row_fills_rows_of_its_own() {
        assert_eq!(both("abcdefghij", 4), [["abcd", "efgh", "ij"]; 2]);
        assert_eq!(rows("ab cdefghij", 4, true), ["ab", "cdef", "ghij"]);
    }

    #[test]
    fn clusters_stay_whole_at_a_break() {
        let e = "e\u{301}";
        assert_eq!(
            rows(&format!("a{e}{e}{e}"), 2, true),
            [format!("a{e}"), format!("{e}{e}")]
        );
        let flag = "🇳🇿";
        assert_eq!(rows(&format!("{flag}{flag}"), 3, true), [flag, flag]);
    }

    #[test]
    fn a_wide_character_that_would_cross_the_edge_starts_the_next_row() {
        assert_eq!(both("ab界", 3), [["ab", "界"], ["ab", "界"]]);
        assert_eq!(rows("abcd界 x", 5, true), ["abcd", "界 x"]);
        assert_eq!(rows("界界界", 5, true), ["界界", "界"]);
    }

    #[test]
    fn clusters_wider_than_the_row_are_left_out() {
        assert_eq!(rows("a界b", 1, true), ["a", "b"]);
        assert_eq!(rows("界界", 1, true), [""]);
    }

    #[test]
    fn spans_keep_their_styles_across_breaks() {
        let red = Style::new().fg(Color::Red);
        let line = Line::from(vec![
            Span::raw("ab "),
            Span::styled("cd ef", red),
            Span::styled("gh", red),
        ])
        .style(Style::new().bg(Color::Blue))
        .centered();
        let rows = wrap_with(&line, 5, true, WidthProfile::default());
        let expected = [
            Line::from(vec![Span::raw("ab "), Span::styled("cd", red)]),
            Line::from(vec![Span::styled("ef", red), Span::styled("gh", red)]),
        ];
        assert_eq!(rows.len(), 2);
        for (row, expected) in rows.iter().zip(expected) {
            assert_eq!(row.spans, expected.spans);
            assert_eq!(row.style, line.style);
            assert_eq!(row.alignment, line.alignment);
            assert!(
                row.spans
                    .iter()
                    .all(|s| matches!(s.content, Cow::Borrowed(_)))
            );
        }
    }

    #[test]
    fn spaces_at_a_break_go_as_far_as_the_row_has_room() {
        assert_eq!(
            both("abc    def", 5),
            [vec!["abc", " def"], vec!["abc", "def"]]
        );
        assert_eq!(both("abc def", 3), [["abc", "def"]; 2]);
        let indented = "               4 Indent";
        assert_eq!(rows(indented, 10, false), ["          ", "    4", "Indent"]);
        assert_eq!(rows(indented, 10, true), ["4 Indent"]);
    }

    #[test]
    fn the_last_row_keeps_its_trailing_spaces() {
        assert_eq!(both("abc   ", 10), [["abc   "]; 2]);
        assert_eq!(both("ab cd  ", 6), [["ab cd"]; 2]);
    }

    #[test]
    fn spaces_after_a_full_row_start_one_of_their_own() {
        assert_eq!(both("ab cd  ", 5), [vec!["ab cd", " "], vec!["ab cd", ""]]);
    }

    #[test]
    fn trim_drops_the_spaces_a_row_starts_with() {
        assert_eq!(both("  ab", 5), [["  ab"], ["ab"]]);
        assert_eq!(both(" ab cd", 3), [[" ab", "cd"], ["ab", "cd"]]);
        assert_eq!(both("  ab", 3), [vec!["  a", "b"], vec!["", "ab"]]);
    }

    #[test]
    fn spaces_alone_make_rows_only_without_trim() {
        assert_eq!(rows("     ", 3, true), [""]);
        assert_eq!(rows("     ", 3, false), ["   ", " "]);
        assert_eq!(rows("a                        ", 20, true), ["a", ""]);
    }

    #[test]
    fn empty_lines_give_one_empty_row() {
        assert_eq!(both("", 5), [[""]; 2]);
        let empty = Line::default();
        let rows_of_empty = wrap_with(&empty, 5, true, WidthProfile::default());
        assert_eq!(rows_of_empty.len(), 1);
        assert_eq!(rows_of_empty[0].spans, [] as [Span<'_>; 0]);
        assert_eq!(rows("abc", 0, true), Vec::<String>::new());
    }

    #[test]
    fn zero_width_spaces_separate_words_and_no_break_spaces_join_them() {
        assert_eq!(rows("foo\u{200b}bar", 3, true), ["foo", "bar"]);
        assert_eq!(rows("foo\u{200b}", 3, true), ["foo"]);
        assert_eq!(rows("aa\u{a0}bb cc", 5, true), ["aa\u{a0}bb", "cc"]);
        assert_eq!(rows("aa bb\u{a0}cc", 5, true), ["aa", "bb\u{a0}cc"]);
    }

    #[test]
    fn controls_are_left_out() {
        assert_eq!(rows("a\tb\u{1b}c", 5, true), ["abc"]);
        let line = Line::from(vec![Span::raw("a\tb")]);
        let rows = wrap_with(&line, 5, true, WidthProfile::default());
        assert_eq!(rows[0].spans, [Span::raw("a"), Span::raw("b")]);
    }

    #[test]
    fn widths_follow_the_profile() {
        let family = "👨\u{200d}👩\u{200d}👧";
        let text = format!("{family} {family}");
        let line = Line::from(text.as_str());
        let joining = WidthProfile {
            joins_emoji: true,
            ..WidthProfile::default()
        };
        assert_eq!(wrap_with(&line, 5, true, joining).len(), 1);
        assert_eq!(wrap_with(&line, 7, true, WidthProfile::default()).len(), 2);
    }

    #[test]
    fn truncated_rows_skip_whole_clusters() {
        let line = Line::from("a界bc");
        let mut found = Vec::new();
        pieces(&line, 4, WidthProfile::default(), &mut found);
        let row = |skip| truncated_row(&found, 3, skip);
        assert_eq!(
            row(0),
            Row {
                start: 0,
                end: 2,
                width: 3
            }
        );
        assert_eq!(
            row(1),
            Row {
                start: 1,
                end: 3,
                width: 3
            }
        );
        assert_eq!(
            row(2),
            Row {
                start: 1,
                end: 3,
                width: 3
            }
        );
        assert_eq!(
            row(3),
            Row {
                start: 2,
                end: 4,
                width: 2
            }
        );
        assert_eq!(
            row(9),
            Row {
                start: 4,
                end: 4,
                width: 0
            }
        );
    }
}
