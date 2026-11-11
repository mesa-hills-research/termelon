//! `TERMELON_CAPS`, which sets any field of [`Capabilities`] by hand.

use super::{
    Capabilities, ColorDepth, GlyphSupport, KeyboardSupport, Multiplexer, Rgb, Support, TerminalId,
};

/// Sets the fields `text` names, as `field=value` pairs separated by commas. A field or value it
/// doesn't know is skipped, since a typo in a variable must not stop a program.
pub(crate) fn apply(caps: &mut Capabilities, text: &str) {
    for pair in text.split(',') {
        let Some((field, value)) = pair.split_once('=') else {
            continue;
        };
        let (field, value) = (field.trim(), value.trim());
        if let Some(support) = support_field(caps, field) {
            if let Some(parsed) = parse_support(value) {
                *support = parsed;
            }
            continue;
        }
        let yes = parse_support(value).map(|support| support == Support::Yes);
        match field {
            "terminal" => caps.terminal = name_and_version(value),
            "multiplexer" => {
                let id = name_and_version(value);
                caps.multiplexer = match id.name.as_deref() {
                    Some("tmux") => Some(Multiplexer::Tmux {
                        version: id.version,
                    }),
                    Some("screen") => Some(Multiplexer::Screen {
                        version: id.version,
                    }),
                    Some("none") => None,
                    _ => caps.multiplexer.take(),
                };
            }
            "colors" => {
                if let Some(colors) = parse_colors(value) {
                    caps.colors = colors;
                }
            }
            "joins_emoji" => set(&mut caps.width_profile.joins_emoji, yes),
            "vs16_widens" => set(&mut caps.width_profile.vs16_widens, yes),
            "ambiguous_wide" => set(&mut caps.width_profile.ambiguous_wide, yes),
            "keyboard" => {
                if let Some(keyboard) = parse_keyboard(value) {
                    caps.keyboard = keyboard;
                }
            }
            "background" => {
                if value == "none" {
                    caps.background = None;
                } else if let Some(rgb) = parse_rgb(value) {
                    caps.background = Some(rgb);
                }
            }
            "glyphs" => match value {
                "full" => caps.glyphs = GlyphSupport::Full,
                "limited" => caps.glyphs = GlyphSupport::Limited,
                _ => {}
            },
            _ => {}
        }
    }
}

fn set(field: &mut bool, value: Option<bool>) {
    if let Some(value) = value {
        *field = value;
    }
}

/// The field named `name`, when it's a [`Support`].
fn support_field<'a>(caps: &'a mut Capabilities, name: &str) -> Option<&'a mut Support> {
    Some(match name {
        "alternate_screen" => &mut caps.alternate_screen,
        "synchronized_output" => &mut caps.synchronized_output,
        "grapheme_clustering" => &mut caps.grapheme_clustering,
        "hyperlinks" => &mut caps.hyperlinks,
        "underline_styles" => &mut caps.underline_styles,
        "underline_color" => &mut caps.underline_color,
        "focus_events" => &mut caps.focus_events,
        "bracketed_paste" => &mut caps.bracketed_paste,
        "mouse" => &mut caps.mouse,
        "clipboard_write" => &mut caps.clipboard_write,
        "cursor_shape" => &mut caps.cursor_shape,
        "title" => &mut caps.title,
        "title_stack" => &mut caps.title_stack,
        "bce" => &mut caps.bce,
        _ => return None,
    })
}

fn parse_support(value: &str) -> Option<Support> {
    match value {
        "yes" | "true" | "1" => Some(Support::Yes),
        "no" | "false" | "0" => Some(Support::No),
        "unknown" => Some(Support::Unknown),
        _ => None,
    }
}

fn parse_colors(value: &str) -> Option<ColorDepth> {
    match value {
        "truecolor" | "24bit" => Some(ColorDepth::TrueColor),
        "256" => Some(ColorDepth::Ansi256),
        "16" => Some(ColorDepth::Ansi16),
        "none" => Some(ColorDepth::None),
        _ => None,
    }
}

fn parse_keyboard(value: &str) -> Option<KeyboardSupport> {
    match value.split_once(':') {
        Some(("kitty", flags)) => Some(KeyboardSupport::Kitty {
            flags: flags.parse().ok()?,
        }),
        None if value == "kitty" => Some(KeyboardSupport::Kitty { flags: 0b1_1111 }),
        None if value == "modify-other-keys" => Some(KeyboardSupport::ModifyOtherKeys),
        None if value == "legacy" => Some(KeyboardSupport::Legacy),
        _ => None,
    }
}

fn parse_rgb(value: &str) -> Option<Rgb> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let channel = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
    Some(Rgb::new(channel(0)?, channel(2)?, channel(4)?))
}

/// `name version`, with the version optional.
fn name_and_version(value: &str) -> TerminalId {
    let (name, version) = match value.split_once(' ') {
        Some((name, version)) => (name, Some(version.trim().to_owned())),
        None => (value, None),
    };
    TerminalId {
        name: Some(name.to_owned()).filter(|name| !name.is_empty()),
        version: version.filter(|version| !version.is_empty()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_fields_are_set_and_the_rest_left() {
        let mut caps = Capabilities::default();
        apply(
            &mut caps,
            "colors=256, hyperlinks=yes,keyboard=kitty:1,terminal=kitty 0.32.2,\
             multiplexer=tmux 3.4,background=#1e1e2e,glyphs=limited,joins_emoji=yes,\
             nonsense=yes,title=perhaps,mouse=no",
        );
        assert_eq!(caps.colors, ColorDepth::Ansi256);
        assert_eq!(caps.hyperlinks, Support::Yes);
        assert_eq!(caps.keyboard, KeyboardSupport::Kitty { flags: 1 });
        assert_eq!(caps.terminal.name.as_deref(), Some("kitty"));
        assert_eq!(caps.terminal.version.as_deref(), Some("0.32.2"));
        assert_eq!(
            caps.multiplexer,
            Some(Multiplexer::Tmux {
                version: Some("3.4".to_owned())
            })
        );
        assert_eq!(caps.background, Some(Rgb::new(0x1e, 0x1e, 0x2e)));
        assert_eq!(caps.glyphs, GlyphSupport::Limited);
        assert!(caps.width_profile.joins_emoji);
        assert_eq!(caps.title, Support::Unknown);
        assert_eq!(caps.mouse, Support::No);
        apply(&mut caps, "multiplexer=none,background=none,colors=none");
        assert_eq!(caps.multiplexer, None);
        assert_eq!(caps.background, None);
        assert_eq!(caps.colors, ColorDepth::None);
    }
}
