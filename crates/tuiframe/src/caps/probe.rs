//! The startup probe, one batch of queries ending with DA1, and what the replies say.
//!
//! Terminals answer in order, so a query whose answer hasn't come when DA1's does is unsupported,
//! and the probe takes one round trip.

use super::{KeyboardSupport, Multiplexer, Rgb, Support, TerminalId, WidthProfile};
use crate::event::decode::{ColorSlot, ModeValue, Reply};

/// The private modes whose support the probe asks about with DECRQM.
pub(crate) const MODES: [u16; 5] = [2026, 2027, 1004, 2004, 1006];

/// The kitty keyboard flags a terminal that answers `CSI ? u` is taken to know. Every terminal
/// checked that answers it implements all five.
const KITTY_FLAGS: u8 = 0b1_1111;

/// The queries, ending with DA1. The Linux console gets DA1 alone, since it takes `ESC P` as a
/// palette command and prints what follows an `ESC ]` it doesn't know.
pub(crate) fn queries(linux_console: bool) -> Vec<u8> {
    let mut out = Vec::new();
    if !linux_console {
        // The kitty keyboard flags.
        out.extend_from_slice(b"\x1b[?u");
        // DECRQM for each mode.
        for mode in MODES {
            out.extend_from_slice(format!("\x1b[?{mode}$p").as_bytes());
        }
        // XTVERSION, then DA2.
        out.extend_from_slice(b"\x1b[>q\x1b[>c");
        // The background colour.
        out.extend_from_slice(b"\x1b]11;?\x1b\\");
        // XTGETTCAP for `Tc`, true colour.
        out.extend_from_slice(b"\x1bP+q5463\x1b\\");
    }
    out.extend_from_slice(b"\x1b[c");
    out
}

/// Whether `reply` ends the probe's replies.
pub(crate) fn is_end(reply: &Reply) -> bool {
    matches!(reply, Reply::Da1 { .. })
}

/// What the replies say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Answers {
    /// DA1 came, so every reply that was coming came.
    pub(crate) complete: bool,
    /// The terminal, from XTVERSION or DA2.
    pub(crate) terminal: Option<TerminalId>,
    /// The multiplexer, which answers in the terminal's place.
    pub(crate) multiplexer: Option<Multiplexer>,
    /// The modes DECRQM reported on.
    pub(crate) modes: Vec<(u16, Support)>,
    /// The kitty keyboard protocol, from `CSI ? u`.
    pub(crate) keyboard: Option<KeyboardSupport>,
    /// Whether XTGETTCAP knows `Tc`, where it answered.
    pub(crate) truecolor: Option<bool>,
    /// The background colour, from OSC 11.
    pub(crate) background: Option<Rgb>,
}

#[cfg(test)]
impl Answers {
    /// What DECRQM said about `mode`, if it said anything.
    pub(crate) fn mode(&self, mode: u16) -> Option<Support> {
        self.modes
            .iter()
            .find(|(reported, _)| *reported == mode)
            .map(|(_, support)| *support)
    }
}

/// What `replies` say.
pub(crate) fn answers(replies: &[Reply]) -> Answers {
    let mut answers = Answers::default();
    let mut da2 = None;
    for reply in replies {
        match reply {
            Reply::Da1 { .. } => answers.complete = true,
            Reply::Da2 { kind, version } => da2 = Some((*kind, *version)),
            Reply::ModeReport {
                mode,
                private: true,
                value,
            } if MODES.contains(mode) => {
                // (A mode reported reset can be set, which is how Alacritty answers for 2026.)
                let support = match value {
                    ModeValue::Set | ModeValue::Reset | ModeValue::PermanentlySet => Support::Yes,
                    ModeValue::NotRecognized | ModeValue::PermanentlyReset => Support::No,
                };
                answers.modes.push((*mode, support));
            }
            Reply::KittyKeyboard(_) => {
                answers.keyboard = Some(KeyboardSupport::Kitty { flags: KITTY_FLAGS });
            }
            Reply::XtVersion(text) => {
                let id = terminal_id(text);
                match id.name.as_deref().map(str::to_ascii_lowercase).as_deref() {
                    Some("tmux") => {
                        answers.multiplexer = Some(Multiplexer::Tmux {
                            version: id.version,
                        });
                    }
                    _ => answers.terminal = Some(id),
                }
            }
            Reply::XtGetTcap { name, value } if name == "Tc" => {
                answers.truecolor = Some(value.is_some());
            }
            Reply::Color {
                which: ColorSlot::Background,
                rgb,
            } => answers.background = Some(*rgb),
            _ => {}
        }
    }
    if answers.terminal.is_none() && answers.multiplexer.is_none() {
        match da2 {
            Some((84, _)) => answers.multiplexer = Some(Multiplexer::Tmux { version: None }),
            Some((83, version)) => {
                answers.multiplexer = Some(Multiplexer::Screen {
                    version: Some(dotted(version)),
                });
            }
            Some((41, version)) => answers.terminal = Some(named("XTerm", version.to_string())),
            Some((65, version)) => answers.terminal = Some(named("VTE", dotted(version))),
            _ => {}
        }
    }
    answers
}

/// The name and version in a terminal's XTVERSION answer, which writes them as `kitty(0.32.2)` or
/// `tmux 3.4`.
pub(crate) fn terminal_id(text: &str) -> TerminalId {
    let text = text.trim();
    let (name, version) = match text.split_once('(') {
        Some((name, rest)) if rest.ends_with(')') => (name, Some(&rest[..rest.len() - 1])),
        _ => match text.split_once(' ') {
            Some((name, version)) => (name, Some(version)),
            None => (text, None),
        },
    };
    TerminalId {
        name: Some(name.trim().to_owned()).filter(|name| !name.is_empty()),
        version: version
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .map(str::to_owned),
    }
}

fn named(name: &str, version: String) -> TerminalId {
    TerminalId {
        name: Some(name.to_owned()),
        version: Some(version),
    }
}

/// A version DA2 writes as one number with two digits a part, `40901` for 4.9.1.
fn dotted(version: u32) -> String {
    format!(
        "{}.{}.{}",
        version / 10_000,
        version / 100 % 100,
        version % 100
    )
}

/// What the width probes write, each measured by the cursor's advance: an emoji ZWJ sequence, a
/// text symbol made wide by VS16, and an East Asian ambiguous character.
pub(crate) const WIDTH_SAMPLES: [&str; 3] =
    ["\u{1f469}\u{200d}\u{1f4bb}", "\u{2764}\u{fe0f}", "\u{a7}"];

/// The width probes, inside a synchronized update. Each sample goes at the start of the second
/// row, followed by a cursor report, and the screen is cleared after, so nothing shows. DA1 ends
/// them. (A report from the first row, `CSI 1 ; n R`, would read as F3 if it came late.)
pub(crate) fn width_queries() -> Vec<u8> {
    let mut out = b"\x1b[?2026h".to_vec();
    for sample in WIDTH_SAMPLES {
        out.extend_from_slice(b"\x1b[2H");
        out.extend_from_slice(sample.as_bytes());
        out.extend_from_slice(b"\x1b[6n");
    }
    out.extend_from_slice(b"\x1b[H\x1b[2J\x1b[?2026l\x1b[c");
    out
}

/// The width profile the cursor reports to [`width_queries`] give, when all three came. Each
/// sample is wide where the cursor moved two cells.
pub(crate) fn width_profile(replies: &[Reply]) -> Option<WidthProfile> {
    let wide: Vec<bool> = replies
        .iter()
        .filter_map(|reply| match reply {
            Reply::CursorPosition { column, .. } => Some(*column == 2),
            _ => None,
        })
        .collect();
    match wide[..] {
        [joins_emoji, vs16_widens, ambiguous_wide] => Some(WidthProfile {
            joins_emoji,
            vs16_widens,
            ambiguous_wide,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::decode::{Decoded, Decoder, DecoderOptions};
    use tuiframe_vt::{Emulator, Profile};

    /// What `profile`'s terminal answers to the probe, decoded.
    fn replies(profile: Profile, queries: &[u8]) -> Vec<Reply> {
        let mut terminal = Emulator::with_profile(profile, 80, 24);
        terminal.feed(queries).unwrap();
        let mut decoder = Decoder::new(DecoderOptions::default());
        let mut decoded = Vec::new();
        decoder.feed(&terminal.take_output(), &mut decoded);
        decoded
            .into_iter()
            .map(|decoded| match decoded {
                Decoded::Reply(reply) => reply,
                Decoded::Event(event) => panic!("{event:?} came as a key"),
            })
            .collect()
    }

    fn id(name: &str, version: Option<&str>) -> TerminalId {
        TerminalId {
            name: Some(name.to_owned()),
            version: version.map(str::to_owned),
        }
    }

    #[test]
    fn xtversion_gives_the_name_and_version() {
        assert_eq!(terminal_id("kitty(0.32.2)"), id("kitty", Some("0.32.2")));
        assert_eq!(terminal_id("XTerm(390)"), id("XTerm", Some("390")));
        assert_eq!(
            terminal_id("WezTerm 20240203-110809-5046fc22"),
            id("WezTerm", Some("20240203-110809-5046fc22"))
        );
        assert_eq!(terminal_id("foot"), id("foot", None));
    }

    /// kitty answers everything, and the answers say what it has.
    #[test]
    fn kitty_answers_every_query() {
        let answers = answers(&replies(Profile::kitty(), &queries(false)));
        assert!(answers.complete);
        assert_eq!(answers.terminal, Some(id("kitty", Some("0.32.2"))));
        assert_eq!(answers.multiplexer, None);
        assert_eq!(
            answers.keyboard,
            Some(KeyboardSupport::Kitty { flags: KITTY_FLAGS })
        );
        assert_eq!(answers.mode(2026), Some(Support::Yes));
        assert_eq!(answers.mode(2027), Some(Support::No));
        assert_eq!(answers.mode(1004), Some(Support::Yes));
        assert_eq!(answers.truecolor, Some(true));
        assert_eq!(answers.background, Some(Rgb::new(0, 0, 0)));
    }

    /// Konsole answers no DECRQM and no XTVERSION, tmux 3.4 answers XTVERSION for itself, and the
    /// Linux console gets DA1 alone, which it answers.
    #[test]
    fn what_each_terminal_leaves_unanswered() {
        let konsole = answers(&replies(Profile::konsole(), &queries(false)));
        assert!(konsole.complete);
        assert!(konsole.modes.is_empty());
        assert_eq!(konsole.terminal, None);
        assert_eq!(konsole.keyboard, None);
        assert_eq!(konsole.truecolor, None);

        let tmux = answers(&replies(Profile::tmux_3_4(), &queries(false)));
        assert_eq!(
            tmux.multiplexer,
            Some(Multiplexer::Tmux {
                version: Some("3.4".to_owned())
            })
        );
        assert_eq!(tmux.terminal, None);

        let xterm = answers(&replies(Profile::xterm(), &queries(false)));
        assert_eq!(xterm.terminal, Some(id("XTerm", Some("390"))));
        assert_eq!(xterm.mode(2026), Some(Support::No));
        assert_eq!(xterm.truecolor, Some(false));

        assert_eq!(queries(true), b"\x1b[c");
        let console = answers(&replies(Profile::linux_console(), &queries(true)));
        assert!(console.complete);
        assert_eq!(
            console,
            Answers {
                complete: true,
                ..Answers::default()
            }
        );

        assert_eq!(
            answers(&replies(Profile::silent(), &queries(false))),
            Answers::default()
        );
    }

    /// DA2 names tmux, screen, xterm and VTE where XTVERSION doesn't come.
    #[test]
    fn da2_names_what_xtversion_doesnt() {
        let screen = answers(&[Reply::Da2 {
            kind: 83,
            version: 40901,
        }]);
        assert_eq!(
            screen.multiplexer,
            Some(Multiplexer::Screen {
                version: Some("4.9.1".to_owned())
            })
        );
        let vte = answers(&[Reply::Da2 {
            kind: 65,
            version: 7600,
        }]);
        assert_eq!(vte.terminal, Some(id("VTE", Some("0.76.0"))));
        let windows = answers(&[Reply::Da2 {
            kind: 0,
            version: 10,
        }]);
        assert_eq!(windows.terminal, None);
    }

    /// The width probes measure each sample as the terminal does.
    #[test]
    fn width_probes_measure_the_terminal() {
        let mut decoder = Decoder::new(DecoderOptions::default());
        for (profile, expected) in [
            (Profile::xterm(), WidthProfile::default()),
            (
                Profile::kitty(),
                WidthProfile {
                    joins_emoji: true,
                    vs16_widens: true,
                    ambiguous_wide: false,
                },
            ),
        ] {
            let mut terminal = Emulator::with_profile(profile, 80, 24);
            terminal.feed(&width_queries()).unwrap();
            assert_eq!(terminal.row_text(1).as_deref(), Some(""));
            for _ in WIDTH_SAMPLES {
                decoder.expect(crate::event::decode::Query::CursorPosition);
            }
            let mut decoded = Vec::new();
            decoder.feed(&terminal.take_output(), &mut decoded);
            let replies: Vec<Reply> = decoded
                .into_iter()
                .filter_map(|decoded| match decoded {
                    Decoded::Reply(reply) => Some(reply),
                    Decoded::Event(_) => None,
                })
                .collect();
            assert_eq!(width_profile(&replies), Some(expected));
        }
        assert_eq!(width_profile(&[]), None);
    }
}
