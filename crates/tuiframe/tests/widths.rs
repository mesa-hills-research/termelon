//! Widths of every sequence in Unicode's `emoji-test.txt` and every range of `EastAsianWidth.txt`,
//! under each combination of the width profile's rules.
//!
//! The expected widths come from the files' own data (each sequence's status, each range's East
//! Asian width and general category), the properties in `DerivedCoreProperties.txt` and
//! `GraphemeBreakProperty.txt` that make a character zero-width, and the rules documented on
//! `WidthProfile`. Trimmed copies of the four files, from Unicode 17.0 as unicode-width 0.2.2 uses,
//! are in `tests/data`.

use std::collections::HashSet;

use tuiframe::caps::WidthProfile;
use tuiframe::text;

const EMOJI_TEST: &str = include_str!("data/emoji-test.txt");
const EAST_ASIAN_WIDTH: &str = include_str!("data/EastAsianWidth.txt");
const DERIVED_CORE_PROPERTIES: &str = include_str!("data/DerivedCoreProperties.txt");
const GRAPHEME_BREAK_PROPERTY: &str = include_str!("data/GraphemeBreakProperty.txt");

/// The widest a cluster gets without `joins_emoji`, by `WidthProfile`'s rule.
const MAX_WIDTH: usize = 7;

fn profiles() -> impl Iterator<Item = WidthProfile> {
    (0..8u8).map(|bits| WidthProfile {
        joins_emoji: bits & 1 != 0,
        vs16_widens: bits & 2 != 0,
        ambiguous_wide: bits & 4 != 0,
    })
}

fn data_lines(file: &str) -> impl Iterator<Item = &str> {
    file.lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
}

fn hex(s: &str) -> Option<u32> {
    u32::from_str_radix(s, 16).ok()
}

/// One line of `EastAsianWidth.txt`.
#[derive(Debug, Clone, Copy)]
struct Range {
    first: u32,
    last: u32,
    width: &'static str,
    category: &'static str,
}

fn east_asian_widths() -> Vec<Range> {
    data_lines(EAST_ASIAN_WIDTH)
        .filter_map(|line| {
            let mut fields = line.split(';');
            let (cps, width, category) = (fields.next()?, fields.next()?, fields.next()?);
            let (first, last) = cps.split_once("..").unwrap_or((cps, cps));
            Some(Range {
                first: hex(first)?,
                last: hex(last)?,
                width,
                category,
            })
        })
        .collect()
}

fn east_asian_width(table: &[Range], cp: u32) -> &'static str {
    table
        .iter()
        .find(|r| r.first <= cp && cp <= r.last)
        .map_or("N", |r| r.width)
}

/// The ranges of one property in a trimmed UCD file.
fn property(file: &str, name: &str) -> Vec<(u32, u32)> {
    data_lines(file)
        .filter_map(|line| {
            let (cps, value) = line.split_once(';')?;
            if value != name {
                return None;
            }
            let (first, last) = cps.split_once("..").unwrap_or((cps, cps));
            Some((hex(first)?, hex(last)?))
        })
        .collect()
}

/// The properties that make a character take no cells on its own.
struct ZeroWidth {
    extend: Vec<(u32, u32)>,
    ignorable: Vec<(u32, u32)>,
    prepend: Vec<(u32, u32)>,
}

impl ZeroWidth {
    fn load() -> Self {
        let zero = Self {
            extend: property(DERIVED_CORE_PROPERTIES, "Grapheme_Extend"),
            ignorable: property(DERIVED_CORE_PROPERTIES, "Default_Ignorable_Code_Point"),
            prepend: property(GRAPHEME_BREAK_PROPERTY, "Prepend"),
        };
        assert!(zero.extend.len() > 300 && zero.ignorable.len() > 10 && zero.prepend.len() > 10);
        zero
    }

    fn contains(&self, cp: u32) -> bool {
        let within = |ranges: &[(u32, u32)]| ranges.iter().any(|&(a, b)| a <= cp && cp <= b);
        // The prepended concatenation marks that unicode-width's rules give a cell.
        let visible_prepend = matches!(cp, 0x0600..=0x0604 | 0x06DD | 0x110BD | 0x110CD);
        // Hangul vowel and trailing jamo, which join the leading jamo before them.
        let jamo = (0x1160..=0x11FF).contains(&cp) || (0xD7B0..=0xD7FF).contains(&cp);
        within(&self.extend)
            || within(&self.ignorable)
            || (within(&self.prepend) && !visible_prepend)
            || jamo
    }
}

/// The cells one character takes on its own, from its East Asian width, its general category and
/// the properties above, with the few exceptions unicode-width's documentation lists, and the
/// halfwidth katakana sound marks, which terminals draw as characters of their own.
fn expected_char_width(cp: u32, range: &Range, zero: &ZeroWidth, profile: WidthProfile) -> usize {
    match cp {
        0xFF9E | 0xFF9F | 0x2D7F => return 1,
        0x115F | 0x17A4 => return 2,
        0x17D8 => return 3,
        0xA8FA => return 0,
        _ => {}
    }
    if range.category == "Cc" || zero.contains(cp) {
        return 0;
    }
    match range.width {
        "W" | "F" => 2,
        "A" if profile.ambiguous_wide => 2,
        _ => 1,
    }
}

/// Every code point of a range, or for a long one its first and last 64 and every 97th between.
fn sample(range: &Range) -> impl Iterator<Item = u32> + '_ {
    let long = range.last - range.first > 1024;
    (range.first..=range.last)
        .filter(move |&cp| !long || cp - range.first < 64 || range.last - cp < 64 || cp % 97 == 0)
}

#[test]
fn every_east_asian_width_range() {
    let table = east_asian_widths();
    let zero = ZeroWidth::load();
    assert_eq!(
        table.len(),
        data_lines(EAST_ASIAN_WIDTH).count(),
        "a line didn't parse"
    );
    let mut failures = Vec::new();
    let mut checked = 0;
    for profile in profiles() {
        for range in &table {
            for cp in sample(range) {
                let Some(c) = char::from_u32(cp) else {
                    continue;
                };
                let s = c.to_string();
                let expected = expected_char_width(cp, range, &zero, profile);
                let got = text::width(&s, profile);
                checked += 1;
                if got != expected {
                    failures.push(format!(
                        "U+{cp:04X} {};{} under {profile:?}: {got}, expected {expected}",
                        range.width, range.category
                    ));
                }
            }
        }
    }
    assert!(checked > 300_000, "checked only {checked}");
    assert!(
        failures.is_empty(),
        "{} of {checked} widths differ:\n{}",
        failures.len(),
        failures[..failures.len().min(60)].join("\n")
    );
}

/// One line of `emoji-test.txt`.
struct Sequence {
    chars: Vec<char>,
    status: &'static str,
}

fn emoji_sequences() -> Vec<Sequence> {
    data_lines(EMOJI_TEST)
        .filter_map(|line| {
            let (cps, status) = line.split_once(';')?;
            let chars = cps
                .split(' ')
                .map(|cp| char::from_u32(hex(cp)?))
                .collect::<Option<_>>()?;
            Some(Sequence { chars, status })
        })
        .collect()
}

const ZWJ: char = '\u{200D}';
const VS16: char = '\u{FE0F}';

fn is_modifier(c: char) -> bool {
    ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)
}

/// What the file says of single code points: those with emoji presentation (fully-qualified on
/// their own, or a component) and those with text presentation (unqualified on their own, or a
/// keycap base).
struct Presentation {
    emoji: HashSet<char>,
    text: HashSet<char>,
}

fn presentation(sequences: &[Sequence]) -> Presentation {
    let mut emoji = HashSet::new();
    let mut text: HashSet<char> = "#*0123456789".chars().collect();
    for s in sequences {
        match (s.chars.as_slice(), s.status) {
            ([c], "fully-qualified" | "component") => {
                emoji.insert(*c);
            }
            ([c], "unqualified") | ([c, VS16], "fully-qualified") => {
                text.insert(*c);
            }
            _ => {}
        }
    }
    Presentation { emoji, text }
}

/// The cells one code point of an emoji sequence takes when drawn on its own.
fn emoji_char_width(c: char, p: &Presentation, eaw: &[Range], profile: WidthProfile) -> usize {
    let mark = matches!(
        c,
        ZWJ | VS16 | '\u{FE0E}' | '\u{20E3}' | '\u{E0020}'..='\u{E007F}'
    );
    if mark {
        return 0;
    }
    if p.emoji.contains(&c) {
        return 2;
    }
    match east_asian_width(eaw, u32::from(c)) {
        "W" | "F" => 2,
        "A" if profile.ambiguous_wide => 2,
        _ => 1,
    }
}

/// The cells a sequence takes under `profile`, by the rules on `WidthProfile`.
fn expected_sequence_width(
    s: &[char],
    p: &Presentation,
    eaw: &[Range],
    profile: WidthProfile,
) -> usize {
    let widths: Vec<usize> = s
        .iter()
        .map(|&c| emoji_char_width(c, p, eaw, profile))
        .collect();
    let sum: usize = widths.iter().sum();
    // A text symbol followed by VS16 takes two cells where the profile says so.
    let widened = s
        .windows(2)
        .zip(&widths)
        .filter(|(pair, w)| pair[1] == VS16 && **w == 1 && p.text.contains(&pair[0]))
        .count();
    if !profile.joins_emoji {
        let sum = if profile.vs16_widens {
            sum + widened
        } else {
            sum
        };
        return sum.min(MAX_WIDTH);
    }
    if s.len() == 1 {
        return sum;
    }
    if s.iter().any(|&c| c == ZWJ || is_modifier(c)) {
        return 2;
    }
    if s.contains(&VS16) && profile.vs16_widens {
        return 2;
    }
    sum.min(2)
}

#[test]
fn every_emoji_test_sequence() {
    let sequences = emoji_sequences();
    assert_eq!(
        sequences.len(),
        data_lines(EMOJI_TEST).count(),
        "a line didn't parse"
    );
    let p = presentation(&sequences);
    let eaw = east_asian_widths();
    let mut failures = Vec::new();
    for s in &sequences {
        let string: String = s.chars.iter().collect();
        let clusters = text::graphemes(&string, WidthProfile::default()).count();
        if clusters != 1 {
            failures.push(format!("{:X?} is {clusters} clusters", s.chars));
        }
        for profile in profiles() {
            let expected = expected_sequence_width(&s.chars, &p, &eaw, profile);
            let got = text::width(&string, profile);
            if got != expected {
                failures.push(format!(
                    "{:X?} {} under {profile:?}: {got}, expected {expected}",
                    s.chars, s.status
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} differ:\n{}",
        failures.len(),
        failures[..failures.len().min(60)].join("\n")
    );
}
