//! What known terminals support that the probe can't ask, by terminal and version.
//!
//! Each row says where its facts come from. The table starts from the project's survey of
//! terminals, their docs and code, and measured runs of real programs in them. Where a row says
//! `Unknown`, nothing checked it, and the library treats the feature as it treats any unknown one.

use super::{ColorDepth, GlyphSupport, Support, WidthProfile};

use Support::{No, Unknown, Yes};

/// What one terminal supports from one version on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Row {
    /// The terminal's name, as [`key`] gives it.
    pub(crate) name: &'static str,
    /// The first version the row describes. The row with the highest one at or below the
    /// terminal's version applies, and a row without one applies to every version.
    pub(crate) since: Option<&'static str>,
    /// Where its facts come from.
    pub(crate) source: &'static str,
    pub(crate) colors: ColorDepth,
    pub(crate) alternate_screen: Support,
    pub(crate) synchronized_output: Support,
    pub(crate) grapheme_clustering: Support,
    pub(crate) focus_events: Support,
    pub(crate) bracketed_paste: Support,
    pub(crate) mouse: Support,
    pub(crate) hyperlinks: Support,
    pub(crate) clipboard_write: Support,
    pub(crate) underline_styles: Support,
    pub(crate) underline_color: Support,
    pub(crate) cursor_shape: Support,
    pub(crate) title: Support,
    pub(crate) title_stack: Support,
    /// Background colour erase, from the terminal's terminfo entry in ncurses' database where it
    /// has one.
    pub(crate) bce: Support,
    /// xterm's modifyOtherKeys, which the library turns on where the kitty protocol isn't.
    pub(crate) modify_other_keys: Support,
    pub(crate) width_profile: WidthProfile,
    pub(crate) glyphs: GlyphSupport,
}

/// A terminal with what most current ones have, for rows to start from.
const CURRENT: Row = Row {
    name: "",
    since: None,
    source: "",
    colors: ColorDepth::TrueColor,
    alternate_screen: Yes,
    synchronized_output: Yes,
    grapheme_clustering: No,
    focus_events: Yes,
    bracketed_paste: Yes,
    mouse: Yes,
    hyperlinks: Yes,
    clipboard_write: Yes,
    underline_styles: Yes,
    underline_color: Yes,
    cursor_shape: Yes,
    title: Yes,
    title_stack: Unknown,
    bce: Unknown,
    modify_other_keys: No,
    width_profile: WidthProfile {
        joins_emoji: false,
        vs16_widens: false,
        ambiguous_wide: false,
    },
    glyphs: GlyphSupport::Full,
};

/// The rows, by terminal and then by version.
pub(crate) const ROWS: &[Row] = &[
    Row {
        name: "xterm",
        source: "xterm's ctlseqs and a measured run of xterm 411. OSC 52 needs allowWindowOps, \
                 and xterm ignores it otherwise. Its BCE is from xterm-256color in ncurses' \
                 terminfo",
        synchronized_output: No,
        hyperlinks: No,
        underline_styles: No,
        underline_color: Unknown,
        title_stack: Yes,
        modify_other_keys: Yes,
        bce: Yes,
        ..CURRENT
    },
    Row {
        name: "vte",
        source: "VTE's code, for GNOME Terminal and the others built on it. Its BCE is from \
                 vte-256color in ncurses' terminfo",
        synchronized_output: No,
        clipboard_write: No,
        bce: Yes,
        ..CURRENT
    },
    Row {
        name: "konsole",
        source: "Konsole's code (Vt102Emulation.cpp). Mode 2026 is in its main branch and not in \
                 23.08 or 25.12. Its BCE is from konsole-256color in ncurses' terminfo",
        synchronized_output: Unknown,
        bce: Yes,
        ..CURRENT
    },
    Row {
        name: "kitty",
        source: "kitty's docs and a measured run of kitty 0.49.2, which joins emoji sequences and \
                 widens VS16. Its BCE is from termelon-vt's kitty profile, since ncurses has no \
                 xterm-kitty",
        title_stack: Yes,
        width_profile: WidthProfile {
            joins_emoji: true,
            vs16_widens: true,
            ambiguous_wide: false,
        },
        bce: Yes,
        ..CURRENT
    },
    Row {
        name: "alacritty",
        source: "Alacritty's code (alacritty_terminal) and a measured run of Alacritty 0.17.0. \
                 Its BCE is from alacritty in ncurses' terminfo",
        title_stack: Yes,
        bce: Yes,
        ..CURRENT
    },
    Row {
        name: "wezterm",
        source: "WezTerm's docs and a measured run of a 2026-10-10 nightly, which joins emoji \
                 sequences. Its BCE is from wezterm in ncurses' terminfo",
        grapheme_clustering: Yes,
        modify_other_keys: Yes,
        width_profile: WidthProfile {
            joins_emoji: true,
            vs16_widens: false,
            ambiguous_wide: false,
        },
        bce: Yes,
        ..CURRENT
    },
    Row {
        name: "foot",
        source: "foot's docs and changelog. Its BCE is from foot in ncurses' terminfo",
        grapheme_clustering: Yes,
        title_stack: Yes,
        bce: Yes,
        ..CURRENT
    },
    Row {
        name: "tmux",
        source: "tmux's CHANGES and man page. It passes true colour, styled underlines and \
                 underline colour on only where the outer terminal's features say so, OSC 52 only \
                 with set-clipboard on, and modified keys only with extended-keys on. \
                 tmux-256color in ncurses' terminfo has no BCE",
        modify_other_keys: Unknown,
        colors: ColorDepth::Ansi256,
        synchronized_output: No,
        underline_styles: Unknown,
        underline_color: Unknown,
        title_stack: Yes,
        bce: No,
        ..CURRENT
    },
    Row {
        name: "tmux",
        since: Some("3.7"),
        source: "tmux's CHANGES, which add mode 2026 in 3.7. tmux-256color in ncurses' terminfo \
                 has no BCE",
        modify_other_keys: Unknown,
        colors: ColorDepth::Ansi256,
        underline_styles: Unknown,
        underline_color: Unknown,
        title_stack: Yes,
        bce: No,
        ..CURRENT
    },
    Row {
        name: "screen",
        source: "GNU screen's code (ansi.c, 5.0.2). screen-256color in ncurses' terminfo has no \
                 BCE",
        colors: ColorDepth::Ansi256,
        synchronized_output: No,
        focus_events: No,
        bracketed_paste: Unknown,
        mouse: Unknown,
        hyperlinks: No,
        clipboard_write: No,
        underline_styles: No,
        underline_color: No,
        cursor_shape: No,
        title_stack: No,
        bce: No,
        ..CURRENT
    },
    Row {
        name: "linux",
        source: "the Linux console's code (drivers/tty/vt/vt.c). It maps every colour to the 16 \
                 basic ones, and its fonts lack most box drawing and braille. Its BCE is from \
                 linux in ncurses' terminfo",
        colors: ColorDepth::Ansi16,
        alternate_screen: No,
        synchronized_output: No,
        focus_events: No,
        bracketed_paste: No,
        mouse: No,
        hyperlinks: No,
        clipboard_write: No,
        underline_styles: No,
        underline_color: No,
        cursor_shape: No,
        title: No,
        title_stack: No,
        glyphs: GlyphSupport::Limited,
        bce: Yes,
        ..CURRENT
    },
    Row {
        name: "windows-terminal",
        source: "Windows Terminal's code (terminalInput.hpp, adaptDispatch.cpp) and docs. Its BCE \
                 is from ms-terminal in ncurses' terminfo",
        grapheme_clustering: Yes,
        bce: Yes,
        ..CURRENT
    },
    Row {
        name: "apple-terminal",
        source: "a measured run of Terminal.app 2.15 on macOS 26, the first with true colour, \
                 which this row leaves out for older versions. Its BCE is from nsterm-256color in \
                 ncurses' terminfo",
        colors: ColorDepth::Ansi256,
        synchronized_output: No,
        focus_events: Unknown,
        mouse: Unknown,
        hyperlinks: Unknown,
        clipboard_write: No,
        underline_color: No,
        cursor_shape: Unknown,
        bce: Yes,
        ..CURRENT
    },
    Row {
        name: "iterm2",
        source: "iTerm2's docs and a measured run of iTerm2 3.6.11, which joins emoji sequences \
                 and widens VS16. Its BCE is from iterm2 in ncurses' terminfo",
        modify_other_keys: Yes,
        width_profile: WidthProfile {
            joins_emoji: true,
            vs16_widens: true,
            ambiguous_wide: false,
        },
        bce: Yes,
        ..CURRENT
    },
];

/// The table's name for a terminal named `name`, as its XTVERSION answer or its environment gives
/// it.
pub(crate) fn key(name: &str) -> String {
    let lower = name.trim().to_ascii_lowercase();
    match lower.as_str() {
        "gnome-terminal" | "vte" => "vte".to_owned(),
        "windows terminal" | "windowsterminal" | "windows-terminal" => {
            "windows-terminal".to_owned()
        }
        "apple_terminal" | "terminal.app" | "apple-terminal" => "apple-terminal".to_owned(),
        "iterm.app" | "iterm2" => "iterm2".to_owned(),
        _ => lower,
    }
}

/// The row for the terminal named `name` at `version`, if the table has one.
pub(crate) fn find(name: &str, version: Option<&str>) -> Option<&'static Row> {
    let name = key(name);
    let version = version.map(numbers);
    ROWS.iter()
        .filter(|row| row.name == name)
        .filter(|row| match (row.since, &version) {
            (None, _) => true,
            (Some(since), Some(version)) => numbers(since) <= *version,
            (Some(_), None) => false,
        })
        .max_by_key(|row| row.since.map(numbers))
}

/// The numbers in a version, in order, so `3.10` comes after `3.9`.
fn numbers(version: &str) -> Vec<u64> {
    version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse().unwrap_or(u64::MAX))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_names_its_source() {
        for row in ROWS {
            assert!(!row.name.is_empty());
            assert_eq!(key(row.name), row.name, "{} isn't its own key", row.name);
            assert!(row.source.len() > 10, "{} has no source", row.name);
        }
    }

    /// The row for a version is the newest one at or below it, and an unknown version gets the
    /// row that holds for every version.
    #[test]
    fn rows_are_found_by_name_and_version() {
        assert_eq!(find("tmux", Some("3.4")).unwrap().synchronized_output, No);
        assert_eq!(find("tmux", Some("3.7")).unwrap().synchronized_output, Yes);
        assert_eq!(find("tmux", Some("3.10")).unwrap().synchronized_output, Yes);
        assert_eq!(find("tmux", Some("3.6a")).unwrap().synchronized_output, No);
        assert_eq!(find("tmux", None).unwrap().synchronized_output, No);
        assert_eq!(find("Apple_Terminal", None).unwrap().name, "apple-terminal");
        assert_eq!(find("iTerm.app", Some("3.6.11")).unwrap().name, "iterm2");
        assert_eq!(find("XTerm", Some("390")).unwrap().name, "xterm");
        assert!(find("st", None).is_none());
    }

    #[test]
    fn every_row_says_whether_it_erases_with_the_background() {
        assert!(ROWS.iter().all(|row| row.bce != Unknown), "{ROWS:?}");
        assert_eq!(find("xterm", None).unwrap().bce, Yes);
        assert_eq!(find("tmux", Some("3.7")).unwrap().bce, No);
    }

    #[test]
    fn versions_compare_by_their_numbers() {
        assert!(numbers("3.10") > numbers("3.9"));
        assert!(numbers("0.32.2") < numbers("0.49"));
        assert_eq!(numbers("3.6a"), [3, 6]);
        assert_eq!(
            numbers("20240203-110809-5046fc22"),
            [20_240_203, 110_809, 5046, 22]
        );
    }
}
