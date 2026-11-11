//! What the terminal supports, as [`Capabilities`].
//!
//! The terminal's answers to the startup probe come first, then environment variables, then a
//! table of known terminals and versions for what can't be asked. `Capabilities::default()` is what
//! the framework assumes when it knows nothing, which is the safe choice for every field.

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

/// How the terminal can report keys.
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
    /// The legacy encodings only, where Shift+Enter arrives as Enter.
    #[default]
    Legacy,
}

/// Whether the terminal's font has box drawing and braille.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GlyphSupport {
    /// It has them.
    #[default]
    Full,
    /// It may lack them, as the Linux console's fonts do, so a program can choose ASCII borders and
    /// a spinner the font has.
    Limited,
}

/// The terminal's name and version, from its XTVERSION or DA2 answer or its environment.
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct TerminalId {
    /// The name, such as `kitty` or `WezTerm`.
    pub name: Option<String>,
    /// The version, as the terminal writes it.
    pub version: Option<String>,
}

/// A terminal multiplexer between the program and the terminal. It answers the probe's queries
/// itself, since it's the one drawing.
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
#[derive(Debug, Default, Clone, PartialEq, Eq, Hash)]
pub struct Capabilities {
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
    /// Copying to the clipboard with OSC 52.
    pub clipboard_write: Support,
    /// Cursor shapes (`CSI n SP q`).
    pub cursor_shape: Support,
    /// Setting the window title with OSC 0 and 2.
    pub title: Support,
    /// Saving and restoring the title with xterm's title stack.
    pub title_stack: Support,
    /// The background colour, for choosing a dark or light theme.
    pub background: Option<Rgb>,
    /// Whether its font has box drawing and braille.
    pub glyphs: GlyphSupport,
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
            caps.clipboard_write,
            caps.cursor_shape,
            caps.title,
            caps.title_stack,
        ];
        assert!(supports.iter().all(|&s| s == Support::Unknown));
    }

    #[test]
    fn default_width_profile_sets_no_rule() {
        let profile = WidthProfile::default();
        assert!(!profile.joins_emoji && !profile.vs16_widens && !profile.ambiguous_wide);
    }
}
