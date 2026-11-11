//! One driver per terminal.

pub mod screen;
pub mod tmux;

use tuiframe_vt::Profile;

use crate::driver::{Driver, Error};

/// The terminals the harness has drivers for.
pub const TERMINALS: [&str; 2] = ["tmux", "screen"];

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
        _ => None,
    }
}
