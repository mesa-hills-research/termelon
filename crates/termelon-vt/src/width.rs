//! How many cells a grapheme cluster takes.

use std::borrow::Cow;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::ambiguous::is_ambiguous;

/// The widest cluster the emulator places when it adds up its scalars' widths.
const MAX_SUMMED: usize = 7;

/// The rules a terminal measures grapheme clusters by. Terminals disagree in more than one way, so
/// these are separate switches rather than a terminal's name.
///
/// A scalar's width is unicode-width's, with two changes terminals make. U+FF9E and U+FF9F, the
/// halfwidth katakana sound marks, take one cell. With `ambiguous_wide`, every scalar whose
/// `East_Asian_Width` is `A` (letters such as é, ß, Greek and Cyrillic included) takes two cells
/// unless it has no width at all, as xterm's `-cjk_width` and VTE's ambiguous width setting do.
///
/// A cluster made only of zero-width scalars (a lone combining mark) has width 0, and the emulator
/// joins it to the cell before the cursor. Any other cluster is measured as follows.
///
/// - Without `joins_emoji`, the width is the sum of its scalars' widths, at most 7. A family emoji
///   in a terminal that doesn't join it then takes the cells the terminal really gives it.
/// - With `joins_emoji`, it's unicode-width's width of the whole cluster, with the sound marks as
///   one cell each, at least 2 when `ambiguous_wide` widens one of its scalars, and from 1 to 2.
/// - `vs16_widens` decides whether U+FE0F after a text symbol makes it two cells. Without it, the
///   selector counts for nothing. With it, a scalar followed by U+FE0F takes the width unicode-width
///   gives the pair, if that is more.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WidthRules {
    /// An emoji ZWJ or skin-tone sequence is one cluster of two cells, not its emoji added up.
    pub joins_emoji: bool,
    /// A text symbol followed by U+FE0F takes two cells.
    pub vs16_widens: bool,
    /// East Asian ambiguous characters take two cells.
    pub ambiguous_wide: bool,
}

/// The halfwidth katakana voiced and semi-voiced sound marks, one cell in terminals.
fn is_sound_mark(c: char) -> bool {
    matches!(c, '\u{ff9e}' | '\u{ff9f}')
}

impl WidthRules {
    /// The cells a grapheme cluster takes under these rules.
    pub fn width(self, cluster: &str) -> u8 {
        if cluster.chars().all(|c| self.char_width(c) == 0) {
            return 0;
        }
        let width = if self.joins_emoji {
            let text: Cow<'_, str> = if self.vs16_widens || !cluster.contains('\u{fe0f}') {
                Cow::Borrowed(cluster)
            } else {
                Cow::Owned(cluster.replace('\u{fe0f}', ""))
            };
            let marks = text.chars().filter(|&c| is_sound_mark(c)).count();
            let mut width = text.width() + marks;
            if self.ambiguous_wide && text.chars().any(|c| self.widens(c)) {
                width = width.max(2);
            }
            width.clamp(1, 2)
        } else {
            let mut sum = 0;
            let mut chars = cluster.chars().peekable();
            while let Some(c) = chars.next() {
                let mut width = self.char_width(c);
                if self.vs16_widens && chars.peek() == Some(&'\u{fe0f}') {
                    let mut pair = String::with_capacity(8);
                    pair.push(c);
                    pair.push('\u{fe0f}');
                    width = width.max(pair.width());
                }
                sum += width;
            }
            sum.min(MAX_SUMMED)
        };
        u8::try_from(width).unwrap_or(u8::MAX)
    }

    /// The width of one scalar.
    fn char_width(self, c: char) -> usize {
        if is_sound_mark(c) {
            return 1;
        }
        if self.widens(c) {
            return 2;
        }
        c.width().unwrap_or(0)
    }

    /// Whether `ambiguous_wide` makes a scalar two cells.
    fn widens(self, c: char) -> bool {
        self.ambiguous_wide && c.width() == Some(1) && is_ambiguous(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLAIN: WidthRules = WidthRules {
        joins_emoji: false,
        vs16_widens: false,
        ambiguous_wide: false,
    };
    const JOINS: WidthRules = WidthRules {
        joins_emoji: true,
        vs16_widens: false,
        ambiguous_wide: false,
    };
    const KITTY: WidthRules = WidthRules {
        joins_emoji: true,
        vs16_widens: true,
        ambiguous_wide: false,
    };
    const FAMILY: &str = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
    const HEART: &str = "\u{2764}\u{fe0f}";

    #[test]
    fn plain_clusters() {
        for rules in [PLAIN, JOINS, KITTY] {
            assert_eq!(rules.width("a"), 1);
            assert_eq!(rules.width("字"), 2);
            assert_eq!(rules.width("e\u{301}"), 1);
            assert_eq!(rules.width("\u{1f600}"), 2);
            assert_eq!(rules.width("\u{1f1fa}\u{1f1f8}"), 2);
            assert_eq!(rules.width("\u{301}"), 0);
        }
    }

    #[test]
    fn zwj_sequences_add_up_unless_joined() {
        assert_eq!(PLAIN.width(FAMILY), 6);
        assert_eq!(JOINS.width(FAMILY), 2);
        assert_eq!(PLAIN.width("\u{1f44d}\u{1f3fd}"), 4);
        assert_eq!(JOINS.width("\u{1f44d}\u{1f3fd}"), 2);
    }

    #[test]
    fn summed_widths_stop_at_seven() {
        let long = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}";
        assert_eq!(PLAIN.width(long), 7);
    }

    #[test]
    fn vs16_widens_only_by_its_rule() {
        assert_eq!(PLAIN.width(HEART), 1);
        assert_eq!(JOINS.width(HEART), 1);
        assert_eq!(KITTY.width(HEART), 2);
        let summed = WidthRules {
            vs16_widens: true,
            ..PLAIN
        };
        assert_eq!(summed.width(HEART), 2);
    }

    #[test]
    fn ambiguous_characters_widen_by_their_rule() {
        let wide = WidthRules {
            ambiguous_wide: true,
            ..PLAIN
        };
        let wide_joined = WidthRules {
            ambiguous_wide: true,
            ..JOINS
        };
        for text in ["é", "ß", "α", "д", "\u{2460}", "±", "─", "é\u{301}"] {
            assert_eq!(PLAIN.width(text), 1, "{text:?}");
            assert_eq!(wide.width(text), 2, "{text:?}");
            assert_eq!(wide_joined.width(text), 2, "{text:?}");
        }
        for text in ["a", "e\u{301}", "ä", "字", "\u{1f600}"] {
            assert_eq!(wide.width(text), PLAIN.width(text), "{text:?}");
        }
        // A combining mark stays without width.
        assert_eq!(wide.width("\u{300}"), 0);
    }

    #[test]
    fn halfwidth_sound_marks_take_a_cell() {
        for rules in [PLAIN, JOINS, KITTY] {
            assert_eq!(rules.width("\u{ff9e}"), 1);
            assert_eq!(rules.width("\u{ff76}\u{ff9e}"), 2);
            assert_eq!(rules.width("\u{ff8a}\u{ff9f}"), 2);
        }
    }
}
