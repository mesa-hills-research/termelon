//! What the terminal supports, as [`Capabilities`].
//!
//! The terminal's answers to the startup probe come first, then environment variables, then a
//! table of known terminals and versions for what can't be asked. `Capabilities::default()` is what
//! the framework assumes when it knows nothing, which is the safe choice for every field.
//!
//! `TERMELON_CAPS` sets any field by hand, for tests and for setups the probe gets wrong. It holds
//! `field=value` pairs separated by commas, and skips a field or value it doesn't know.
//!
//! ```text
//! TERMELON_CAPS="colors=256,hyperlinks=no,keyboard=kitty:1,terminal=kitty 0.32.2"
//! ```
//!
//! | Field | Values |
//! |---|---|
//! | Every [`Support`] field, such as `hyperlinks` or `synchronized_output` | `yes`, `no`, `unknown` |
//! | `colors` | `truecolor`, `256`, `16`, `none` |
//! | `keyboard` | `legacy`, `modify-other-keys`, `kitty`, `kitty:<flags>` |
//! | `terminal` | a name, then a space and a version if known |
//! | `multiplexer` | `tmux` or `screen`, then a version if known, or `none` |
//! | `joins_emoji`, `vs16_widens`, `ambiguous_wide` | `yes`, `no` |
//! | `background` | `#rrggbb`, `none` |
//! | `glyphs` | `full`, `limited` |

pub(crate) mod env;
mod overrides;
pub(crate) mod probe;
mod table;

/// Whether the terminal supports a feature.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Support {
    /// It does.
    Yes,
    /// It doesn't.
    No,
    /// Neither its answers, its environment nor the table of known terminals says.
    #[default]
    Unknown,
}

/// How many colours the terminal shows. Colours a frame uses are lowered to this depth.
#[non_exhaustive]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorDepth {
    /// 24-bit colour.
    TrueColor,
    /// The 256-colour palette.
    Ansi256,
    /// The 16 named colours.
    #[default]
    Ansi16,
    /// No colour, as under `NO_COLOR`. Attributes such as bold and reverse still go out.
    None,
}

/// How the terminal measures the width of a grapheme cluster, as rules rather than a name, since
/// terminals disagree in more than one way.
///
/// With no rule set, a cluster takes the sum of its characters' widths (at most 7 cells), so a
/// sequence the terminal doesn't join keeps the row aligned. The default sets none, which is what
/// most terminals do.
#[non_exhaustive]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WidthProfile {
    /// An emoji ZWJ or skin-tone sequence is one wide cluster rather than its emoji summed, as
    /// terminals with grapheme clustering (mode 2027) measure it.
    pub joins_emoji: bool,
    /// A text symbol followed by U+FE0F (VS16) takes two cells.
    pub vs16_widens: bool,
    /// East Asian ambiguous characters take two cells.
    pub ambiguous_wide: bool,
}

impl WidthProfile {
    /// The profile with these rules.
    #[must_use]
    pub const fn new(joins_emoji: bool, vs16_widens: bool, ambiguous_wide: bool) -> Self {
        Self {
            joins_emoji,
            vs16_widens,
            ambiguous_wide,
        }
    }
}

/// How the terminal can report keys.
#[non_exhaustive]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyboardSupport {
    /// The kitty keyboard protocol, which tells Shift+Enter from Enter and can report releases.
    Kitty {
        /// The progressive enhancement flags the terminal understands, as in kitty's `CSI > flags u`
        /// (1 disambiguate, 2 event types, 4 alternate keys, 8 every key as an escape, 16 associated
        /// text).
        flags: u8,
    },
    /// xterm's modifyOtherKeys, which tells modified keys apart but has no releases.
    ModifyOtherKeys,
    /// The Windows console's key records, which tell every key apart, Shift+Enter included, and
    /// report releases.
    ConsoleRecords,
    /// The legacy encodings only, where Shift+Enter arrives as Enter.
    #[default]
    Legacy,
}

/// Whether the terminal's font has box drawing and braille.
#[non_exhaustive]
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphSupport {
    /// It has them.
    #[default]
    Full,
    /// It may lack them, as the Linux console's fonts do, so a program can choose ASCII borders and
    /// a spinner the font has.
    Limited,
}

/// How the startup probe went.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Probe {
    /// It didn't run, as the program gave the capabilities.
    #[default]
    NotRun,
    /// The terminal answered DA1, so every answer that was coming came.
    Complete,
    /// The terminal didn't answer DA1 in time, so what it supports comes from the environment and
    /// the table alone.
    TimedOut,
}

/// The terminal's name and version, from its XTVERSION or DA2 answer or its environment.
#[non_exhaustive]
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct TerminalId {
    /// The name, such as `kitty` or `WezTerm`.
    pub name: Option<String>,
    /// The version, as the terminal writes it.
    pub version: Option<String>,
}

/// A terminal multiplexer between the program and the terminal. It answers the probe's queries
/// itself, since it's the one drawing.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Multiplexer {
    /// tmux.
    Tmux {
        /// Its version, such as `3.4`.
        version: Option<String>,
    },
    /// GNU screen.
    Screen {
        /// Its version.
        version: Option<String>,
    },
}

/// A colour the terminal reported, with 8 bits a channel.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Rgb {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

impl Rgb {
    /// The colour with these channels.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }
}

/// What the terminal supports, found once at open.
///
/// The framework falls back where a feature is missing. Private modes are sent even where support
/// is unknown, since terminals ignore modes they don't know, and strings (OSC and DCS sequences) are
/// sent only where support is known. A program reads these to adapt what it shows, such as
/// offering another chord where Shift+Enter can't be told from Enter.
#[non_exhaustive]
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Capabilities {
    /// How the startup probe went.
    pub probe: Probe,
    /// The terminal's name and version.
    pub terminal: TerminalId,
    /// The multiplexer the program runs in, if any.
    pub multiplexer: Option<Multiplexer>,
    /// The colours it shows.
    pub colors: ColorDepth,
    /// How it measures grapheme clusters.
    pub width_profile: WidthProfile,
    /// The alternate screen, which the Linux console lacks.
    pub alternate_screen: Support,
    /// How it reports keys.
    pub keyboard: KeyboardSupport,
    /// Synchronized updates (mode 2026), which keep a frame from showing half drawn.
    pub synchronized_output: Support,
    /// Grapheme clustering (mode 2027).
    pub grapheme_clustering: Support,
    /// OSC 8 hyperlinks.
    pub hyperlinks: Support,
    /// Underline shapes (`SGR 4:n`).
    pub underline_styles: Support,
    /// Underline colours (`SGR 58`).
    pub underline_color: Support,
    /// Focus reports (mode 1004).
    pub focus_events: Support,
    /// Bracketed paste (mode 2004).
    pub bracketed_paste: Support,
    /// Mouse reports in SGR coordinates (mode 1006), which the Linux console lacks.
    pub mouse: Support,
    /// Copying to the clipboard with OSC 52.
    pub clipboard_write: Support,
    /// Cursor shapes (`CSI n SP q`).
    pub cursor_shape: Support,
    /// Setting the window title with OSC 0 and 2.
    pub title: Support,
    /// Saving and restoring the title with xterm's title stack.
    pub title_stack: Support,
    /// Background colour erase (BCE). Erasing fills cells with the pen's background colour, so
    /// the encoder may erase runs of spaces in a colour rather than write them.
    pub bce: Support,
    /// Scrolling a region of rows, with DECSTBM and then insert or delete line. Where it's `Yes`,
    /// rows a widget says moved ([`Buffer::note_scroll`](crate::Buffer::note_scroll)) are
    /// scrolled in the terminal rather than written again. The table says `Yes` for the terminals
    /// whose runs in the project's terminal matrix showed scrolled regions right.
    pub scroll_regions: Support,
    /// The background colour, for choosing a dark or light theme.
    pub background: Option<Rgb>,
    /// Whether its font has box drawing and braille.
    pub glyphs: GlyphSupport,
}

impl ColorDepth {
    /// How many colours it shows, for comparing depths.
    fn rank(self) -> u8 {
        match self {
            ColorDepth::None => 0,
            ColorDepth::Ansi16 => 1,
            ColorDepth::Ansi256 => 2,
            ColorDepth::TrueColor => 3,
        }
    }
}

/// The capabilities `answers` and `hints` give, with the table filling in what neither says, and
/// `overrides` (`TERMELON_CAPS`) last.
pub(crate) fn detect(
    answers: &probe::Answers,
    hints: &env::Hints,
    overrides: Option<&str>,
) -> Capabilities {
    let mut caps = Capabilities {
        probe: if answers.complete {
            Probe::Complete
        } else {
            Probe::TimedOut
        },
        ..Capabilities::default()
    };
    // (A reply names what answered, which a leaked variable can't.)
    let replied = answers.terminal.is_some() || answers.multiplexer.is_some();
    caps.multiplexer = if replied {
        answers.multiplexer.clone()
    } else {
        hints.multiplexer.clone()
    };
    caps.terminal = answers
        .terminal
        .clone()
        .or_else(|| hints.terminal.clone())
        .unwrap_or_default();
    // (Inside a multiplexer, it parses what the program writes.)
    let row = if hints.linux_console && !replied {
        table::find("linux", None)
    } else {
        match &caps.multiplexer {
            Some(Multiplexer::Tmux { version }) => table::find("tmux", version.as_deref()),
            Some(Multiplexer::Screen { version }) => table::find("screen", version.as_deref()),
            None => caps
                .terminal
                .name
                .as_deref()
                .and_then(|name| table::find(name, caps.terminal.version.as_deref())),
        }
    };
    if let Some(row) = row {
        apply_row(&mut caps, row);
        if row.modify_other_keys == Support::Yes {
            caps.keyboard = KeyboardSupport::ModifyOtherKeys;
        }
    }
    if let Some(colors) = hints.colors
        && colors.rank() > caps.colors.rank()
    {
        caps.colors = colors;
    }
    for &(mode, support) in &answers.modes {
        let field = match mode {
            2026 => &mut caps.synchronized_output,
            2027 => &mut caps.grapheme_clustering,
            1004 => &mut caps.focus_events,
            2004 => &mut caps.bracketed_paste,
            1006 => &mut caps.mouse,
            _ => continue,
        };
        *field = support;
    }
    if let Some(keyboard) = answers.keyboard {
        caps.keyboard = keyboard;
    }
    if answers.truecolor == Some(true) {
        caps.colors = ColorDepth::TrueColor;
    }
    caps.background = answers.background;
    if hints.no_color {
        caps.colors = ColorDepth::None;
    }
    // (The library turns grapheme clustering on where the terminal has it, and the terminal then
    // measures an emoji sequence as one cluster.)
    if caps.grapheme_clustering == Support::Yes {
        caps.width_profile.joins_emoji = true;
    }
    if let Some(overrides) = overrides {
        apply_overrides(&mut caps, overrides);
    }
    caps
}

/// Sets the fields `TERMELON_CAPS` names in `overrides`.
pub(crate) fn apply_overrides(caps: &mut Capabilities, overrides: &str) {
    overrides::apply(caps, overrides);
}

fn apply_row(caps: &mut Capabilities, row: &table::Row) {
    caps.colors = row.colors;
    caps.alternate_screen = row.alternate_screen;
    caps.synchronized_output = row.synchronized_output;
    caps.grapheme_clustering = row.grapheme_clustering;
    caps.focus_events = row.focus_events;
    caps.bracketed_paste = row.bracketed_paste;
    caps.mouse = row.mouse;
    caps.hyperlinks = row.hyperlinks;
    caps.clipboard_write = row.clipboard_write;
    caps.underline_styles = row.underline_styles;
    caps.underline_color = row.underline_color;
    caps.cursor_shape = row.cursor_shape;
    caps.title = row.title;
    caps.title_stack = row.title_stack;
    caps.bce = row.bce;
    caps.scroll_regions = row.scroll_regions;
    caps.width_profile = row.width_profile;
    caps.glyphs = row.glyphs;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_knows_nothing() {
        let caps = Capabilities::default();
        assert_eq!(
            caps.terminal,
            TerminalId {
                name: None,
                version: None
            }
        );
        assert_eq!(caps.multiplexer, None);
        assert_eq!(caps.colors, ColorDepth::Ansi16);
        assert_eq!(caps.width_profile, WidthProfile::default());
        assert_eq!(caps.keyboard, KeyboardSupport::Legacy);
        assert_eq!(caps.background, None);
        assert_eq!(caps.glyphs, GlyphSupport::Full);
        let supports = [
            caps.alternate_screen,
            caps.synchronized_output,
            caps.grapheme_clustering,
            caps.hyperlinks,
            caps.underline_styles,
            caps.underline_color,
            caps.focus_events,
            caps.bracketed_paste,
            caps.mouse,
            caps.clipboard_write,
            caps.cursor_shape,
            caps.title,
            caps.title_stack,
            caps.bce,
        ];
        assert!(supports.iter().all(|&s| s == Support::Unknown));
    }

    #[test]
    fn default_width_profile_sets_no_rule() {
        let profile = WidthProfile::default();
        assert!(!profile.joins_emoji && !profile.vs16_widens && !profile.ambiguous_wide);
    }

    fn detect_with(profile: termelon_vt::Profile, vars: &[(&str, &str)]) -> Capabilities {
        use crate::event::decode::{Decoded, Decoder, DecoderOptions};
        let hints = env::hints(&env::Vars(vars));
        let mut terminal = termelon_vt::Emulator::with_profile(profile, 80, 24);
        terminal.feed(&probe::queries(hints.linux_console)).unwrap();
        let mut decoded = Vec::new();
        Decoder::new(DecoderOptions::default()).feed(&terminal.take_output(), &mut decoded);
        let replies: Vec<_> = decoded
            .into_iter()
            .filter_map(|decoded| match decoded {
                Decoded::Reply(reply) => Some(reply),
                Decoded::Event(_) => None,
            })
            .collect();
        detect(&probe::answers(&replies), &hints, None)
    }

    /// Replies beat the table, the table fills in what can't be asked, and the environment only
    /// adds where neither says.
    #[test]
    fn replies_then_environment_then_table() {
        let kitty = detect_with(
            termelon_vt::Profile::kitty(),
            &[("TERM_PROGRAM", "WezTerm")],
        );
        assert_eq!(kitty.terminal.name.as_deref(), Some("kitty"));
        assert_eq!(kitty.keyboard, KeyboardSupport::Kitty { flags: 0b1_1111 });
        assert_eq!(kitty.synchronized_output, Support::Yes);
        assert_eq!(kitty.hyperlinks, Support::Yes);
        assert_eq!(kitty.colors, ColorDepth::TrueColor);
        assert!(kitty.width_profile.joins_emoji && kitty.width_profile.vs16_widens);

        let konsole = detect_with(
            termelon_vt::Profile::konsole(),
            &[("KONSOLE_VERSION", "230805"), ("TERM", "xterm-256color")],
        );
        assert_eq!(konsole.terminal.name.as_deref(), Some("Konsole"));
        assert_eq!(konsole.synchronized_output, Support::Unknown);
        assert_eq!(konsole.focus_events, Support::Yes);
        assert_eq!(konsole.keyboard, KeyboardSupport::Legacy);

        let xterm = detect_with(termelon_vt::Profile::xterm(), &[("COLORTERM", "truecolor")]);
        assert_eq!(xterm.keyboard, KeyboardSupport::ModifyOtherKeys);
        assert_eq!(xterm.synchronized_output, Support::No);
        assert_eq!(xterm.hyperlinks, Support::No);
        assert_eq!(xterm.colors, ColorDepth::TrueColor);

        let tmux = detect_with(
            termelon_vt::Profile::tmux_3_4(),
            &[("TERM", "tmux-256color"), ("KITTY_WINDOW_ID", "1")],
        );
        assert!(matches!(tmux.multiplexer, Some(Multiplexer::Tmux { .. })));
        assert_eq!(tmux.terminal.name.as_deref(), Some("kitty"));
        assert_eq!(tmux.synchronized_output, Support::No);
        assert_eq!(tmux.colors, ColorDepth::Ansi256);

        let windows = detect_with(
            termelon_vt::Profile::windows_terminal(),
            &[("WT_SESSION", "1")],
        );
        assert_eq!(windows.grapheme_clustering, Support::Yes);
        assert!(windows.width_profile.joins_emoji);
    }

    /// The Linux console gets its row, with no alternate screen and its fonts' limits, and
    /// `NO_COLOR` and `TERMELON_CAPS` have the last word.
    #[test]
    fn the_console_no_color_and_overrides() {
        let console = detect_with(termelon_vt::Profile::linux_console(), &[("TERM", "linux")]);
        assert_eq!(console.alternate_screen, Support::No);
        assert_eq!(console.glyphs, GlyphSupport::Limited);
        assert_eq!(console.mouse, Support::No);
        assert_eq!(console.colors, ColorDepth::Ansi16);

        let no_color = detect_with(
            termelon_vt::Profile::kitty(),
            &[("NO_COLOR", "1"), ("COLORTERM", "truecolor")],
        );
        assert_eq!(no_color.colors, ColorDepth::None);

        let silent = detect_with(termelon_vt::Profile::silent(), &[]);
        assert_eq!(
            silent,
            Capabilities {
                probe: Probe::TimedOut,
                ..Capabilities::default()
            }
        );
        assert_eq!(console.probe, Probe::Complete);

        let hints = env::hints(&env::Vars(&[("TERM", "xterm-kitty")]));
        let caps = detect(
            &probe::Answers::default(),
            &hints,
            Some("hyperlinks=no,colors=16"),
        );
        assert_eq!(caps.terminal.name.as_deref(), Some("kitty"));
        assert_eq!(caps.hyperlinks, Support::No);
        assert_eq!(caps.colors, ColorDepth::Ansi16);
    }
}
