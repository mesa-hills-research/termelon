//! Terminals the emulator can play, with what each answers and how it draws.
//!
//! The facts come from each terminal's own code and docs where they were checked, and are believed
//! where not. The constructors' docs say which.

use crate::input::{Key, Mods};
use crate::width::WidthRules;

/// How a terminal encodes keys without the kitty keyboard protocol.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum KeyEncoding {
    /// xterm's encodings, which most terminals share.
    #[default]
    Xterm,
    /// tmux's, which are xterm's but for Home and End without modifiers, `CSI 1 ~` and `CSI 4 ~`.
    Tmux,
    /// tmux 3.4's. It sends nothing for a key with Ctrl that has no control character, such as
    /// Ctrl+1 or Ctrl+Enter, or for Shift with Enter, Escape or Backspace. Once a program asks for
    /// modifyOtherKeys, at either level, those keys come as `CSI code ; mods u`, and Ctrl+Tab as
    /// `CSI 9 ; 5 u`. Its other keys are [`KeyEncoding::Tmux`]'s.
    Tmux34,
    /// Konsole's, from its `default.keytab`, which are xterm's but for Shift+Enter, `ESC O M`, and
    /// F1 to F4 with modifiers, `ESC O mods P` and on. Shift with Up, Down, Page Up, Page Down,
    /// Home and End scrolls the main screen and sends nothing there.
    Konsole,
    /// The Linux console's. F1 to F5 are `CSI [ A` to `CSI [ E`, Home and End are `CSI 1 ~` and
    /// `CSI 4 ~`, and modifiers on the other special keys are lost.
    LinuxConsole,
}

/// Behaviours some terminals have that a program shouldn't depend on either way. Each is off in
/// the built-in profiles, so a test turns them on to check that output still shows right.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Quirks {
    /// `CSI ? 7 l` does nothing, so autowrap stays on.
    pub ignores_autowrap_off: bool,
    /// Writing over half of a wide character leaves the other half on the screen. The emulator
    /// keeps the character's text there, in a narrow cell.
    pub keeps_wide_halves: bool,
    /// `CSI n S` and `CSI n T` do nothing.
    pub no_scroll_up_down: bool,
    /// `CSI n L` and `CSI n M` do nothing.
    pub no_insert_delete_lines: bool,
}

/// One way a terminal differs, for turning it on in a profile with [`Profile::with_quirk`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Quirk {
    /// [`Quirks::ignores_autowrap_off`].
    IgnoresAutowrapOff,
    /// [`Quirks::keeps_wide_halves`].
    KeepsWideHalves,
    /// [`Quirks::no_scroll_up_down`].
    NoScrollUpDown,
    /// [`Quirks::no_insert_delete_lines`].
    NoInsertDeleteLines,
    /// No mode 2026, so synchronized updates are ignored.
    NoSync,
    /// East Asian ambiguous characters take two cells ([`WidthRules::ambiguous_wide`]).
    AmbiguousWide,
    /// Erasing and scrolling leave the default background ([`Profile::bce`] off).
    NoBce,
}

impl Quirk {
    /// Every quirk.
    pub const ALL: [Quirk; 7] = [
        Quirk::IgnoresAutowrapOff,
        Quirk::KeepsWideHalves,
        Quirk::NoScrollUpDown,
        Quirk::NoInsertDeleteLines,
        Quirk::NoSync,
        Quirk::AmbiguousWide,
        Quirk::NoBce,
    ];
}

/// A terminal as the emulator plays it: how it measures and erases, which modes it has, what it
/// answers, and how it encodes input.
///
/// The fields are public, so a test can start from a constructor and change what it needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    /// A name for messages.
    pub name: String,
    /// How it measures grapheme clusters.
    pub width: WidthRules,
    /// Whether erasing and scrolling fill cells with the pen's background colour (BCE).
    pub bce: bool,
    /// The DEC private modes it has. It ignores the others, and DECRQM answers 0 for them.
    pub modes: Vec<u16>,
    /// Whether it answers DECRQM (`CSI ? n $ p`).
    pub decrqm: bool,
    /// Whether it keeps OSC 8 hyperlinks. Without them it ignores OSC 8.
    pub hyperlinks: bool,
    /// The SGR attributes it takes and doesn't draw, such as 53 (overline) for kitty.
    pub ignored_sgr: Vec<u16>,
    /// Whether it has the title stack (`CSI 22 t` and `CSI 23 t`).
    pub title_stack: bool,
    /// Whether it sets the cursor's shape with DECSCUSR (`CSI n SP q`).
    pub cursor_shape: bool,
    /// Whether it has the kitty keyboard protocol and answers `CSI ? u`.
    pub kitty_keyboard: bool,
    /// Whether it honours modifyOtherKeys (`CSI > 4 ; n m`).
    pub modify_other_keys: bool,
    /// Whether it parses `ESC ]` and `ESC P` as the Linux console does. A palette command is then the
    /// only string it knows, and the text of any other prints on the screen.
    pub linux_console: bool,
    /// How it encodes keys without the kitty keyboard protocol.
    pub keys: KeyEncoding,
    /// The keys it keeps for itself and sends nothing for, such as xterm's Alt+Enter, which turns
    /// full screen on and off.
    pub bound_keys: Vec<(Key, Mods)>,
    /// Its answer to DA1 (`CSI c`).
    pub da1: Option<String>,
    /// Its answer to DA2 (`CSI > c`).
    pub da2: Option<String>,
    /// The text of its XTVERSION answer (`CSI > q`), which it sends as `DCS > | text ST`.
    pub xtversion: Option<String>,
    /// Whether it answers the cursor position report (`CSI 6 n`) and the status report (`CSI 5 n`).
    pub cursor_report: bool,
    /// Whether it answers `CSI 18 t` with its size in cells.
    pub size_report: bool,
    /// The colour it answers OSC 10 `?` with.
    pub foreground: Option<(u8, u8, u8)>,
    /// The colour it answers OSC 11 `?` with.
    pub background: Option<(u8, u8, u8)>,
    /// The capabilities it answers XTGETTCAP (`DCS + q name ST`) with, each a name and a value
    /// (`None` for a boolean). `None` means it doesn't answer XTGETTCAP at all.
    pub termcap: Option<Vec<(String, Option<String>)>>,
    /// Behaviours a program shouldn't depend on.
    pub quirks: Quirks,
}

const XTERM_MODES: [u16; 16] = [
    1, 6, 7, 9, 12, 25, 47, 1000, 1002, 1003, 1004, 1006, 1047, 1048, 1049, 2004,
];

fn caps(list: &[(&str, Option<&str>)]) -> Vec<(String, Option<String>)> {
    list.iter()
        .map(|(name, value)| ((*name).to_owned(), value.map(str::to_owned)))
        .collect()
}

impl Profile {
    /// xterm 390, as Ubuntu 24.04 ships it, with `allowWindowOps` and `metaSendsEscape` on as a
    /// test harness sets them. It has no synchronized output, no grapheme mode, no hyperlinks and
    /// no kitty keyboard protocol, and it answers everything else the startup probe asks. Alt+Enter
    /// turns full screen on and off (`Alt <Key>Return:fullscreen()` in its default translations).
    pub fn xterm() -> Profile {
        Profile {
            name: "xterm".to_owned(),
            width: WidthRules::default(),
            bce: true,
            modes: XTERM_MODES.to_vec(),
            decrqm: true,
            hyperlinks: false,
            ignored_sgr: Vec::new(),
            title_stack: true,
            cursor_shape: true,
            kitty_keyboard: false,
            modify_other_keys: true,
            linux_console: false,
            keys: KeyEncoding::Xterm,
            bound_keys: vec![(Key::Enter, Mods::ALT)],
            da1: Some("\x1b[?64;1;2;6;9;15;16;17;18;21;22;28c".to_owned()),
            da2: Some("\x1b[>41;390;0c".to_owned()),
            xtversion: Some("XTerm(390)".to_owned()),
            cursor_report: true,
            size_report: true,
            foreground: Some((0, 0, 0)),
            background: Some((0xff, 0xff, 0xff)),
            termcap: Some(caps(&[
                ("TN", Some("xterm")),
                ("Co", Some("256")),
                ("colors", Some("256")),
            ])),
            quirks: Quirks::default(),
        }
    }

    /// kitty 0.32, as Ubuntu 24.04 ships it. It joins emoji sequences, widens VS16, has the kitty
    /// keyboard protocol and synchronized output, and answers every query. It draws no blink,
    /// hidden text or overline (SGR 5, 6, 8 and 53, from `cursor_from_sgr` in its `cursor.c`), and
    /// blink comes in 0.43. Its DA2 version field is believed, not checked.
    pub fn kitty() -> Profile {
        let mut modes = XTERM_MODES.to_vec();
        modes.retain(|&m| m != 9);
        modes.push(2026);
        Profile {
            name: "kitty".to_owned(),
            width: WidthRules {
                joins_emoji: true,
                vs16_widens: true,
                ambiguous_wide: false,
            },
            bce: true,
            modes,
            decrqm: true,
            hyperlinks: true,
            ignored_sgr: vec![5, 6, 8, 53],
            title_stack: true,
            cursor_shape: true,
            kitty_keyboard: true,
            modify_other_keys: false,
            linux_console: false,
            keys: KeyEncoding::Xterm,
            bound_keys: Vec::new(),
            da1: Some("\x1b[?62;c".to_owned()),
            da2: Some("\x1b[>1;4000;32c".to_owned()),
            xtversion: Some("kitty(0.32.2)".to_owned()),
            cursor_report: true,
            size_report: true,
            foreground: Some((0xdd, 0xdd, 0xdd)),
            background: Some((0, 0, 0)),
            termcap: Some(caps(&[
                ("TN", Some("xterm-kitty")),
                ("Co", Some("256")),
                ("colors", Some("256")),
                ("Tc", None),
                ("RGB", None),
            ])),
            quirks: Quirks::default(),
        }
    }

    /// Konsole 23.08, as Ubuntu 24.04 ships it. It answers neither DECRQM, XTVERSION nor XTGETTCAP,
    /// and has no kitty keyboard protocol or synchronized output. Its keys are
    /// [`KeyEncoding::Konsole`], and Ctrl+Tab, a shortcut, sends nothing, from 25.12.3's keytab and a
    /// run of it on Ubuntu 26.04. Its title stack and `CSI 18 t` answers are believed.
    pub fn konsole() -> Profile {
        let mut modes = XTERM_MODES.to_vec();
        modes.retain(|&m| m != 9);
        Profile {
            name: "konsole".to_owned(),
            width: WidthRules::default(),
            bce: true,
            modes,
            decrqm: false,
            hyperlinks: true,
            ignored_sgr: Vec::new(),
            title_stack: false,
            cursor_shape: true,
            kitty_keyboard: false,
            modify_other_keys: false,
            linux_console: false,
            keys: KeyEncoding::Konsole,
            bound_keys: vec![(Key::Tab, Mods::CTRL)],
            da1: Some("\x1b[?62;1;4c".to_owned()),
            da2: Some("\x1b[>1;115;0c".to_owned()),
            xtversion: None,
            cursor_report: true,
            size_report: true,
            foreground: Some((0xfc, 0xfc, 0xfc)),
            background: Some((0x23, 0x26, 0x27)),
            termcap: None,
            quirks: Quirks::default(),
        }
    }

    /// Windows Terminal 1.22 or later. It answers DECRQM but not XTVERSION or XTGETTCAP, has the
    /// kitty keyboard protocol, synchronized output and grapheme mode 2027, and joins clusters once a
    /// program sets 2027. Its `CSI 18 t` answer is believed.
    pub fn windows_terminal() -> Profile {
        let mut modes = XTERM_MODES.to_vec();
        modes.extend([2026, 2027]);
        Profile {
            name: "windows-terminal".to_owned(),
            width: WidthRules::default(),
            bce: true,
            modes,
            decrqm: true,
            hyperlinks: true,
            ignored_sgr: Vec::new(),
            title_stack: false,
            cursor_shape: true,
            kitty_keyboard: true,
            modify_other_keys: false,
            linux_console: false,
            keys: KeyEncoding::Xterm,
            bound_keys: Vec::new(),
            da1: Some("\x1b[?61;4;6;7;14;21;22;23;24;28;32;42c".to_owned()),
            da2: Some("\x1b[>0;10;1c".to_owned()),
            xtversion: None,
            cursor_report: true,
            size_report: true,
            foreground: Some((0xcc, 0xcc, 0xcc)),
            background: Some((0x0c, 0x0c, 0x0c)),
            termcap: None,
            quirks: Quirks::default(),
        }
    }

    /// tmux 3.4, as Ubuntu 24.04 ships it, with its default options. It doesn't know mode 2026 and
    /// doesn't answer DECRQM (3.6 answers it for some modes, 3.7 knows 2026), answers XTVERSION,
    /// and keeps hyperlinks. Its keys are [`KeyEncoding::Tmux34`], from `input_key` in its
    /// `input-keys.c` and a run on Ubuntu 24.04: with `extended-keys` off, as it is by default, it
    /// still sends extended keys once a program asks for modifyOtherKeys. Its `CSI 18 t` answer is
    /// believed.
    pub fn tmux_3_4() -> Profile {
        Profile {
            name: "tmux-3.4".to_owned(),
            width: WidthRules::default(),
            bce: true,
            modes: vec![
                1, 6, 7, 12, 25, 47, 1000, 1002, 1003, 1004, 1006, 1047, 1049, 2004,
            ],
            decrqm: false,
            hyperlinks: true,
            ignored_sgr: Vec::new(),
            title_stack: true,
            cursor_shape: true,
            kitty_keyboard: false,
            modify_other_keys: true,
            linux_console: false,
            keys: KeyEncoding::Tmux34,
            bound_keys: Vec::new(),
            da1: Some("\x1b[?1;2c".to_owned()),
            da2: Some("\x1b[>84;0;0c".to_owned()),
            xtversion: Some("tmux 3.4".to_owned()),
            cursor_report: true,
            size_report: true,
            foreground: None,
            background: None,
            termcap: None,
            quirks: Quirks::default(),
        }
    }

    /// The Linux console. It has no alternate screen, few modes and no cursor shapes, answers only
    /// DA1 and the cursor report, and parses `ESC P` and `ESC ] P` as palette commands, so the text
    /// of any other OSC or DCS prints on the screen.
    pub fn linux_console() -> Profile {
        Profile {
            name: "linux".to_owned(),
            width: WidthRules::default(),
            bce: true,
            modes: vec![1, 6, 7, 9, 25, 1000],
            decrqm: false,
            hyperlinks: false,
            ignored_sgr: Vec::new(),
            title_stack: false,
            cursor_shape: false,
            kitty_keyboard: false,
            modify_other_keys: false,
            linux_console: true,
            keys: KeyEncoding::LinuxConsole,
            bound_keys: Vec::new(),
            da1: Some("\x1b[?6c".to_owned()),
            da2: None,
            xtversion: None,
            cursor_report: true,
            size_report: false,
            foreground: None,
            background: None,
            termcap: None,
            quirks: Quirks::default(),
        }
    }

    /// A terminal that draws as xterm does and answers nothing, not even DA1.
    pub fn silent() -> Profile {
        Profile {
            name: "silent".to_owned(),
            decrqm: false,
            da1: None,
            da2: None,
            xtversion: None,
            cursor_report: false,
            size_report: false,
            foreground: None,
            background: None,
            termcap: None,
            ..Profile::xterm()
        }
    }

    /// The profile with a quirk turned on.
    #[must_use]
    pub fn with_quirk(mut self, quirk: Quirk) -> Profile {
        match quirk {
            Quirk::IgnoresAutowrapOff => self.quirks.ignores_autowrap_off = true,
            Quirk::KeepsWideHalves => self.quirks.keeps_wide_halves = true,
            Quirk::NoScrollUpDown => self.quirks.no_scroll_up_down = true,
            Quirk::NoInsertDeleteLines => self.quirks.no_insert_delete_lines = true,
            Quirk::NoSync => self.modes.retain(|&mode| mode != 2026),
            Quirk::AmbiguousWide => self.width.ambiguous_wide = true,
            Quirk::NoBce => self.bce = false,
        }
        self
    }

    /// Whether it has a DEC private mode.
    pub fn has_mode(&self, mode: u16) -> bool {
        self.modes.contains(&mode)
    }
}

impl Default for Profile {
    fn default() -> Profile {
        Profile::xterm()
    }
}
