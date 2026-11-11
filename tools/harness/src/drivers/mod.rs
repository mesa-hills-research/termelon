//! One driver per terminal.

pub mod alacritty;
pub mod conhost;
pub mod console;
pub mod foot;
pub mod ghostty;
pub mod kitty;
pub mod konsole;
pub mod lines;
pub mod screen;
pub mod tmux;
pub mod vte;
pub mod wezterm;
pub mod windows_terminal;
pub mod x11;
pub mod xterm;
pub mod xtermjs;

use termelon_vt::{KeyEncoding, Profile, WidthRules};

use crate::driver::{Driver, Error};

/// The terminals the harness has drivers for.
pub const TERMINALS: [&str; 13] = [
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
            tmux.set_width_rules(profile(name).map(|p| p.width).unwrap_or_default());
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
        "windows-terminal" => Box::new(windows_terminal::WindowsTerminal::new()?),
        "conhost" => Box::new(conhost::Conhost::new()?),
        _ => return Err(Error::Missing(format!("a driver for {name}"))),
    })
}

/// The `termelon-vt` profile that computes what a terminal should show.
pub fn profile(name: &str) -> Option<Profile> {
    match name {
        "tmux" => Some(Profile::tmux_3_4()),
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
                ..Profile::xterm()
            };
            profile
                .modes
                .retain(|mode| ![47, 1047, 1049].contains(mode));
            Some(profile)
        }
        "kitty" => Some(Profile::kitty()),
        "xterm" => Some(Profile::xterm()),
        "wezterm" => {
            let mut profile = Profile {
                name: "wezterm".to_owned(),
                width: WidthRules {
                    joins_emoji: true,
                    ..WidthRules::default()
                },
                hyperlinks: true,
                kitty_keyboard: true,
                ..Profile::xterm()
            };
            profile.modes.extend([2026, 2027]);
            Some(profile)
        }
        "xtermjs" => Some(Profile {
            name: "xtermjs".to_owned(),
            hyperlinks: true,
            ..Profile::xterm()
        }),
        // VTE ignores modifyOtherKeys and sends its legacy keys.
        "vte" => Some(Profile {
            name: "vte".to_owned(),
            hyperlinks: true,
            modify_other_keys: false,
            ..Profile::xterm()
        }),
        "alacritty" => {
            let mut profile = Profile {
                name: "alacritty".to_owned(),
                hyperlinks: true,
                ..Profile::xterm()
            };
            profile.modes.push(2026);
            Some(profile)
        }
        "konsole" => Some(Profile::konsole()),
        // foot and Ghostty join clusters once a program sets mode 2027, and Ghostty has it on from
        // the start and widens VS16.
        "foot" | "ghostty" => {
            let mut profile = Profile {
                name: name.to_owned(),
                hyperlinks: true,
                kitty_keyboard: true,
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
