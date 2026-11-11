//! How many cells a grapheme cluster takes.

use std::borrow::Cow;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// The widest cluster the emulator places when it adds up its scalars' widths.
const MAX_SUMMED: usize = 7;

/// The rules a terminal measures grapheme clusters by. Terminals disagree in more than one way, so
/// these are separate switches rather than a terminal's name.
///
/// A cluster made only of zero-width scalars (a lone combining mark) has width 0, and the emulator
/// joins it to the cell before the cursor. Any other cluster is measured as follows.
///
/// - Without `joins_emoji`, the width is the sum of its scalars' widths from unicode-width, at most
///   7. A family emoji in a terminal that doesn't join it then takes the cells the terminal really
///   gives it.
/// - With `joins_emoji`, it's unicode-width's width of the whole cluster, from 1 to 2.
/// - `vs16_widens` decides whether U+FE0F after a text symbol makes it two cells. Without it, the
///   selector counts for nothing. With it, a scalar followed by U+FE0F takes the width unicode-width
///   gives the pair, if that is more.
/// - `ambiguous_wide` measures East Asian ambiguous characters as two cells.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct WidthRules {
    /// An emoji ZWJ or skin-tone sequence is one cluster of two cells, not its emoji added up.
    pub joins_emoji: bool,
    /// A text symbol followed by U+FE0F takes two cells.
    pub vs16_widens: bool,
    /// East Asian ambiguous characters take two cells.
    pub ambiguous_wide: bool,
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
            self.str_width(&text).clamp(1, 2)
        } else {
            let mut sum = 0;
            let mut chars = cluster.chars().peekable();
            while let Some(c) = chars.next() {
                let mut width = self.char_width(c);
                if self.vs16_widens && chars.peek() == Some(&'\u{fe0f}') {
                    let mut pair = String::with_capacity(8);
                    pair.push(c);
                    pair.push('\u{fe0f}');
                    width = width.max(self.str_width(&pair));
                }
                sum += width;
            }
            sum.min(MAX_SUMMED)
        };
        u8::try_from(width).unwrap_or(u8::MAX)
    }

    fn char_width(self, c: char) -> usize {
        let width = if self.ambiguous_wide {
            c.width_cjk()
        } else {
            c.width()
        };
        width.unwrap_or(0)
    }

    fn str_width(self, text: &str) -> usize {
        if self.ambiguous_wide {
            text.width_cjk()
        } else {
            text.width()
        }
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
    fn ambiguous_characters() {
        let wide = WidthRules {
            ambiguous_wide: true,
            ..PLAIN
        };
        assert_eq!(PLAIN.width("é"), 1);
        assert_eq!(PLAIN.width("\u{2460}"), 1);
        assert_eq!(wide.width("\u{2460}"), 2);
    }
}
