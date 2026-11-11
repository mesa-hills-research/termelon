//! One driver per terminal.

pub mod kitty;
pub mod lines;
pub mod screen;
pub mod tmux;
pub mod vte;
pub mod wezterm;
pub mod x11;
pub mod xterm;
pub mod xtermjs;

use tuiframe_vt::{Profile, WidthRules};

use crate::driver::{Driver, Error};

/// The terminals the harness has drivers for.
pub const TERMINALS: [&str; 7] = [
    "tmux", "screen", "kitty", "xterm", "wezterm", "xtermjs", "vte",
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
        _ => return Err(Error::Missing(format!("a driver for {name}"))),
    })
}

/// The `tuiframe-vt` profile that computes what a terminal should show.
pub fn profile(name: &str) -> Option<Profile> {
    match name {
        "tmux" => Some(Profile::tmux_3_4()),
        // screen erases without the background colour and keeps no alternate screen or links
        // with its default options.
        "screen" => {
            let mut profile = Profile {
                name: "screen".to_owned(),
                bce: false,
                hyperlinks: false,
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
        "vte" => Some(Profile {
            name: "vte".to_owned(),
            hyperlinks: true,
            ..Profile::xterm()
        }),
        _ => None,
    }
}
