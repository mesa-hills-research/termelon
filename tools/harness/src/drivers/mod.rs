//! One driver per terminal.

pub mod alacritty;
pub mod conhost;
pub mod console;
pub mod foot;
pub mod ghostty;
pub mod kitty;
pub mod konsole;
pub mod lines;
pub mod linux;
pub mod screen;
pub mod tmux;
pub mod vte;
pub mod wezterm;
pub mod windows_terminal;
pub mod x11;
pub mod xterm;
pub mod xtermjs;

use termelon_vt::{Key, KeyEncoding, Mods, Profile, WidthRules};

use crate::driver::{Driver, Error};
use crate::input::version_slug;

/// The terminals the harness has drivers for.
pub const TERMINALS: [&str; 14] = [
    "tmux",
    "screen",
    "kitty",
    "xterm",
    "wezterm",
    "xtermjs",
    "vte",
    "alacritty",
    "konsole",
    "foot",
    "ghostty",
    "linux",
    "windows-terminal",
    "conhost",
];

/// A driver for a terminal by name.
///
/// # Errors
///
/// When the name isn't one of [`TERMINALS`], or the driver can't make its work directory.
pub fn driver(name: &str) -> Result<Box<dyn Driver>, Error> {
    Ok(match name {
        "tmux" => {
            let mut tmux = tmux::Tmux::new()?;
            tmux.set_width_rules(Profile::tmux_3_4().width);
            Box::new(tmux)
        }
        "screen" => Box::new(screen::Screen::new()?),
        "kitty" => Box::new(kitty::Kitty::new()?),
        "xterm" => Box::new(xterm::Xterm::new()?),
        "wezterm" => Box::new(wezterm::Wezterm::new()?),
        "xtermjs" => Box::new(xtermjs::XtermJs::new()?),
        "vte" => Box::new(vte::Vte::new()?),
        "alacritty" => Box::new(alacritty::Alacritty::new()?),
        "konsole" => Box::new(konsole::Konsole::new()?),
        "foot" => Box::new(foot::Foot::new()?),
        "ghostty" => Box::new(ghostty::Ghostty::new()?),
        "linux" => Box::new(linux::LinuxConsole::new()?),
        "windows-terminal" => Box::new(windows_terminal::WindowsTerminal::new()?),
        "conhost" => Box::new(conhost::Conhost::new()?),
        _ => return Err(Error::Missing(format!("a driver for {name}"))),
    })
}

/// The numbers in a terminal's version, such as `[0, 32, 2]` for `kitty 0.32.2 created by Kovid
/// Goyal`, so versions compare in order. An unknown version has none and comes first.
pub fn version_numbers(version: &str) -> Vec<u64> {
    version_slug(version)
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse().unwrap_or(u64::MAX))
        .collect()
}

/// The `termelon-vt` profile that computes what a terminal of a version, as its driver gives it,
/// should show and send.
pub fn profile(name: &str, version: &str) -> Option<Profile> {
    let numbers = version_numbers(version);
    match name {
        "tmux" if numbers.as_slice() < [3, 5].as_slice() => Some(Profile::tmux_3_4()),
        // From 3.5, tmux ignores a program's modifyOtherKeys while `extended-keys` is off, as it
        // is by default (its CHANGES, and runs of 3.6 and 3.8).
        "tmux" => Some(Profile {
            name: format!("tmux-{}", version_slug(version)),
            modify_other_keys: false,
            keys: KeyEncoding::Tmux,
            xtversion: Some(format!("tmux {}", version_slug(version))),
            ..Profile::tmux_3_4()
        }),
        // screen erases without the background colour and keeps no alternate screen or links
        // with its default options. It sends Home and End as tmux does, and ignores
        // modifyOtherKeys.
        "screen" => {
            let mut profile = Profile {
                name: "screen".to_owned(),
                bce: false,
                hyperlinks: false,
                modify_other_keys: false,
                keys: KeyEncoding::Tmux,
                bound_keys: Vec::new(),
                ..Profile::xterm()
            };
            profile
                .modes
                .retain(|mode| ![47, 1047, 1049].contains(mode));
            Some(profile)
        }
        "kitty" if numbers.as_slice() < [0, 43].as_slice() => Some(Profile::kitty()),
        // kitty 0.43 draws blink (its changelog), and still no rapid blink, hidden text or
        // overline (`cursor_from_sgr` in 0.45.0's `cursor.c`).
        "kitty" => Some(Profile {
            ignored_sgr: vec![6, 8, 53],
            xtversion: Some(format!("kitty({})", version_slug(version))),
            ..Profile::kitty()
        }),
        "xterm" => Some(Profile::xterm()),
        // WezTerm runs with its defaults, so without the kitty keyboard protocol, and keeps
        // Alt+Enter (full screen), Ctrl+Tab and Ctrl+Shift+Tab (the next and previous tab) for
        // itself, as its default key assignments have them. A cell keeps one intensity, so bold
        // and dim replace each other.
        "wezterm" => {
            let mut profile = Profile {
                name: "wezterm".to_owned(),
                width: WidthRules {
                    joins_emoji: true,
                    ..WidthRules::default()
                },
                hyperlinks: true,
                one_intensity: true,
                keys: KeyEncoding::Wezterm,
                bound_keys: vec![
                    (Key::Enter, Mods::ALT),
                    (Key::Tab, Mods::CTRL),
                    (Key::Tab, Mods::CTRL | Mods::SHIFT),
                ],
                ..Profile::xterm()
            };
            profile.modes.extend([2026, 2027]);
            Some(profile)
        }
        "xtermjs" => Some(Profile {
            name: "xtermjs".to_owned(),
            hyperlinks: true,
            bound_keys: Vec::new(),
            ..Profile::xterm()
        }),
        // VTE ignores modifyOtherKeys and sends its legacy keys.
        "vte" => Some(Profile {
            name: "vte".to_owned(),
            hyperlinks: true,
            modify_other_keys: false,
            bound_keys: Vec::new(),
            ..Profile::xterm()
        }),
        // Alacritty has the kitty keyboard protocol and ignores modifyOtherKeys, as a run of 0.13.2
        // showed. Its default bindings send the legacy bytes of Shift+Tab and Backspace with Shift
        // or Alt under the protocol too (`default_key_bindings` in its `bindings.rs`), and
        // Shift+Enter sends its text.
        "alacritty" => {
            let mut profile = Profile {
                name: "alacritty".to_owned(),
                hyperlinks: true,
                kitty_keyboard: true,
                modify_other_keys: false,
                bound_keys: Vec::new(),
                legacy_under_kitty: vec![
                    (Key::Tab, Mods::SHIFT),
                    (Key::Tab, Mods::SHIFT | Mods::ALT),
                    (Key::Backspace, Mods::NONE),
                    (Key::Backspace, Mods::SHIFT),
                    (Key::Backspace, Mods::ALT),
                    (Key::Enter, Mods::SHIFT),
                ],
                ..Profile::xterm()
            };
            profile.modes.push(2026);
            Some(profile)
        }
        "konsole" => Some(Profile::konsole()),
        "linux" => Some(Profile::linux_console()),
        // foot and Ghostty join clusters once a program sets mode 2027, and Ghostty has it on from
        // the start and widens VS16.
        "foot" | "ghostty" => {
            let mut profile = Profile {
                name: name.to_owned(),
                hyperlinks: true,
                kitty_keyboard: true,
                bound_keys: Vec::new(),
                ..Profile::xterm()
            };
            if name == "ghostty" {
                profile.width = WidthRules {
                    joins_emoji: true,
                    vs16_widens: true,
                    ambiguous_wide: false,
                };
            }
            profile.modes.extend([2026, 2027]);
            Some(profile)
        }
        "windows-terminal" => Some(Profile::windows_terminal()),
        // conhost drops links.
        "conhost" => Some(Profile {
            name: "conhost".to_owned(),
            hyperlinks: false,
            kitty_keyboard: false,
            ..Profile::windows_terminal()
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_follow_the_version() {
        assert_eq!(
            version_numbers("kitty 0.32.2 created by Kovid Goyal"),
            [0, 32, 2]
        );
        assert_eq!(version_numbers("tmux 3.6a"), [3, 6]);
        assert_eq!(version_numbers("xterm"), [] as [u64; 0]);
        let keys = |version: &str| profile("tmux", version).map(|p| p.keys);
        assert_eq!(keys("tmux 3.4"), Some(KeyEncoding::Tmux34));
        assert_eq!(keys("tmux 3.6a"), Some(KeyEncoding::Tmux));
        assert_eq!(keys("tmux 3.10"), Some(KeyEncoding::Tmux));
        let blinks = |version: &str| profile("kitty", version).map(|p| !p.ignored_sgr.contains(&5));
        assert_eq!(blinks("kitty 0.32.2 created by Kovid Goyal"), Some(false));
        assert_eq!(blinks("kitty 0.45.0 created by Kovid Goyal"), Some(true));
        assert!(profile("st", "st 0.9").is_none());
    }
}
