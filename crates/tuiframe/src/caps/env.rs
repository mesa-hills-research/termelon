//! What the environment says about the terminal, for what the probe didn't learn.
//!
//! Variables leak into terminals started from another one (kitty and Alacritty don't set
//! `TERM_PROGRAM`, so a parent's stays), so a variable only fills in what no reply said.

use super::{ColorDepth, Multiplexer, TerminalId};

/// Where the variables come from, the process's environment or a test's.
pub(crate) trait Environment {
    /// The variable `name`, when it's set and not empty.
    fn var(&self, name: &str) -> Option<String>;
}

/// The process's environment.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Process;

impl Environment for Process {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var_os(name)
            .map(|value| value.to_string_lossy().into_owned())
            .filter(|value| !value.is_empty())
    }
}

/// Variables given by name, for tests.
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct Vars<'a>(pub(crate) &'a [(&'a str, &'a str)]);

#[cfg(test)]
impl Environment for Vars<'_> {
    fn var(&self, name: &str) -> Option<String> {
        self.0
            .iter()
            .find(|(key, value)| *key == name && !value.is_empty())
            .map(|(_, value)| (*value).to_owned())
    }
}

/// What the variables say.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Hints {
    /// The terminal, as its own variables or `TERM` name it.
    pub(crate) terminal: Option<TerminalId>,
    /// tmux or GNU screen, from `TMUX`, `STY` or `TERM`.
    pub(crate) multiplexer: Option<Multiplexer>,
    /// The colours `COLORTERM` or `TERM` promise at least.
    pub(crate) colors: Option<ColorDepth>,
    /// `NO_COLOR` is set.
    pub(crate) no_color: bool,
    /// `TERM` is the Linux console's, outside any multiplexer.
    pub(crate) linux_console: bool,
}

/// What `env` says about the terminal.
pub(crate) fn hints(env: &dyn Environment) -> Hints {
    let term = env.var("TERM").unwrap_or_default();
    let multiplexer = if env.var("TMUX").is_some() || term.starts_with("tmux") {
        Some(Multiplexer::Tmux { version: None })
    } else if env.var("STY").is_some() || term.starts_with("screen") {
        Some(Multiplexer::Screen { version: None })
    } else {
        None
    };
    let colors = match env.var("COLORTERM").as_deref() {
        Some("truecolor" | "24bit") => Some(ColorDepth::TrueColor),
        _ if term.ends_with("-direct") => Some(ColorDepth::TrueColor),
        _ if term.contains("256color") => Some(ColorDepth::Ansi256),
        _ => None,
    };
    Hints {
        terminal: identify(env, &term),
        linux_console: multiplexer.is_none() && (term == "linux" || term.starts_with("linux-")),
        multiplexer,
        colors,
        no_color: env.var("NO_COLOR").is_some(),
    }
}

/// The terminal `env` names, from the variables terminals set for themselves, then `TERM`, then
/// `TERM_PROGRAM`, which leaks the most.
fn identify(env: &dyn Environment, term: &str) -> Option<TerminalId> {
    let named = |name: &str, version: Option<String>| {
        Some(TerminalId {
            name: Some(name.to_owned()),
            version,
        })
    };
    let program = env.var("TERM_PROGRAM");
    let program_version = || env.var("TERM_PROGRAM_VERSION");
    if env.var("KITTY_WINDOW_ID").is_some() || term == "xterm-kitty" {
        return named("kitty", None);
    }
    if env.var("ALACRITTY_WINDOW_ID").is_some() || term == "alacritty" {
        return named("alacritty", None);
    }
    if env.var("WEZTERM_EXECUTABLE").is_some() {
        let version = program
            .as_deref()
            .filter(|p| *p == "WezTerm")
            .and_then(|_| program_version());
        return named("WezTerm", version);
    }
    if env.var("WT_SESSION").is_some() {
        return named("Windows Terminal", None);
    }
    if let Some(version) = env.var("KONSOLE_VERSION") {
        return named("Konsole", konsole_version(&version));
    }
    if let Some(version) = env.var("VTE_VERSION") {
        return named("VTE", vte_version(&version));
    }
    if term.starts_with("foot") {
        return named("foot", None);
    }
    if let Some(text) = env.var("XTERM_VERSION") {
        return Some(super::probe::terminal_id(&text));
    }
    match program.as_deref() {
        None | Some("tmux" | "screen") => None,
        Some(program) => named(program, program_version()),
    }
}

/// Konsole's version from `KONSOLE_VERSION`, which writes 23.08.5 as `230805`.
fn konsole_version(digits: &str) -> Option<String> {
    let number: u32 = digits.parse().ok()?;
    Some(format!(
        "{}.{:02}.{}",
        number / 10_000,
        number / 100 % 100,
        number % 100
    ))
}

/// VTE's version from `VTE_VERSION`, which writes 0.76.0 as `7600`.
fn vte_version(digits: &str) -> Option<String> {
    let number: u32 = digits.parse().ok()?;
    Some(format!(
        "{}.{}.{}",
        number / 10_000,
        number / 100 % 100,
        number % 100
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str, version: Option<&str>) -> TerminalId {
        TerminalId {
            name: Some(name.to_owned()),
            version: version.map(str::to_owned),
        }
    }

    fn hints_of(vars: &[(&str, &str)]) -> Hints {
        hints(&Vars(vars))
    }

    #[test]
    fn terminals_are_named_by_their_own_variables_first() {
        let kitty = hints_of(&[
            ("TERM", "xterm-kitty"),
            ("TERM_PROGRAM", "WezTerm"),
            ("KITTY_WINDOW_ID", "1"),
        ]);
        assert_eq!(kitty.terminal, Some(named("kitty", None)));
        let konsole = hints_of(&[("TERM", "xterm-256color"), ("KONSOLE_VERSION", "230805")]);
        assert_eq!(konsole.terminal, Some(named("Konsole", Some("23.08.5"))));
        assert_eq!(konsole.colors, Some(ColorDepth::Ansi256));
        let vte = hints_of(&[("VTE_VERSION", "7600"), ("COLORTERM", "truecolor")]);
        assert_eq!(vte.terminal, Some(named("VTE", Some("0.76.0"))));
        assert_eq!(vte.colors, Some(ColorDepth::TrueColor));
        let iterm = hints_of(&[
            ("TERM_PROGRAM", "iTerm.app"),
            ("TERM_PROGRAM_VERSION", "3.6.11"),
        ]);
        assert_eq!(iterm.terminal, Some(named("iTerm.app", Some("3.6.11"))));
        let xterm = hints_of(&[("XTERM_VERSION", "XTerm(390)")]);
        assert_eq!(xterm.terminal, Some(named("XTerm", Some("390"))));
        assert_eq!(
            hints_of(&[("TERM", "xterm"), ("TERM_PROGRAM", "")]),
            Hints::default()
        );
    }

    #[test]
    fn multiplexers_and_the_linux_console_are_told_apart() {
        assert!(hints_of(&[("TERM", "linux")]).linux_console);
        let tmux = hints_of(&[("TERM", "screen"), ("TMUX", "/tmp/tmux-1/0")]);
        assert!(!tmux.linux_console);
        assert_eq!(tmux.multiplexer, Some(Multiplexer::Tmux { version: None }));
        let screen = hints_of(&[("TERM", "screen.xterm-256color"), ("STY", "1.tty")]);
        assert_eq!(
            screen.multiplexer,
            Some(Multiplexer::Screen { version: None })
        );
        let no_color = hints_of(&[("NO_COLOR", "1"), ("COLORTERM", "24bit")]);
        assert!(no_color.no_color);
        assert_eq!(no_color.colors, Some(ColorDepth::TrueColor));
        assert!(!hints_of(&[("NO_COLOR", "")]).no_color);
    }
}
