//! The names terminals' tools give keys: tmux's `send-keys`, kitty's `send-key` and xdotool's
//! `key`.

use tuiframe_vt::{Key, Mods};

/// The key as `tmux send-keys` names it, such as `C-a`, `M-Up` or `F5`. A plain character is
/// `None`, since it goes as literal text (`send-keys -l`), and so is a key tmux has no name for.
pub fn tmux(key: Key, mods: Mods) -> Option<String> {
    let mut prefix = String::new();
    if mods.contains(Mods::CTRL) {
        prefix.push_str("C-");
    }
    if mods.contains(Mods::ALT) {
        prefix.push_str("M-");
    }
    let shift = mods.contains(Mods::SHIFT);
    let name = match key {
        Key::Char(_) if prefix.is_empty() && !shift => return None,
        Key::Char(c) if shift && c.is_alphabetic() => c.to_uppercase().collect(),
        Key::Char(' ') => {
            if shift {
                prefix.push_str("S-");
            }
            "Space".to_owned()
        }
        Key::Char(c) => {
            if shift {
                prefix.push_str("S-");
            }
            c.to_string()
        }
        Key::Tab if shift => return Some(format!("{prefix}BTab")),
        // tmux 3.6 takes `C-BSpace` for literal text.
        Key::Backspace if mods.contains(Mods::CTRL) => return None,
        _ => {
            if shift {
                prefix.push_str("S-");
            }
            special(
                key,
                [
                    "Enter", "Tab", "BSpace", "Escape", "Up", "Down", "Left", "Right", "Home",
                    "End", "PPage", "NPage", "IC", "DC",
                ],
                "F",
            )?
        }
    };
    Some(format!("{prefix}{name}"))
}

/// The key as `kitten @ send-key` names it, such as `ctrl+a`, `shift+up` or `f5`.
pub fn kitty(key: Key, mods: Mods) -> Option<String> {
    let name = match key {
        Key::Char(' ') => "space".to_owned(),
        Key::Char(c) => c.to_lowercase().collect(),
        _ => special(
            key,
            [
                "enter",
                "tab",
                "backspace",
                "escape",
                "up",
                "down",
                "left",
                "right",
                "home",
                "end",
                "page_up",
                "page_down",
                "insert",
                "delete",
            ],
            "f",
        )?,
    };
    Some(format!(
        "{}{name}",
        modifiers(mods, ["ctrl+", "alt+", "shift+", "super+"])
    ))
}

/// The key as `xdotool key` names it, an X keysym such as `ctrl+a`, `shift+Up` or `F5`.
pub fn xdotool(key: Key, mods: Mods) -> Option<String> {
    let name = x_keysym(key)?;
    Some(format!(
        "{}{name}",
        modifiers(mods, ["ctrl+", "alt+", "shift+", "super+"])
    ))
}

/// The X keysym of a key, without modifiers, such as `a`, `Return` or `Prior`. GTK and wtype name
/// keys this way too.
pub fn x_keysym(key: Key) -> Option<String> {
    match key {
        Key::Char(c) => Some(keysym(c)),
        _ => special(
            key,
            [
                "Return",
                "Tab",
                "BackSpace",
                "Escape",
                "Up",
                "Down",
                "Left",
                "Right",
                "Home",
                "End",
                "Prior",
                "Next",
                "Insert",
                "Delete",
            ],
            "F",
        ),
    }
}

/// The modifiers held, as `ctrl`, `alt`, `shift` and `super`.
pub fn modifier_names(mods: Mods) -> Vec<&'static str> {
    [
        (Mods::CTRL, "ctrl"),
        (Mods::ALT, "alt"),
        (Mods::SHIFT, "shift"),
        (Mods::SUPER, "super"),
    ]
    .into_iter()
    .filter(|&(held, _)| mods.contains(held))
    .map(|(_, name)| name)
    .collect()
}

fn modifiers(mods: Mods, names: [&str; 4]) -> String {
    let mut out = String::new();
    for (held, name) in [Mods::CTRL, Mods::ALT, Mods::SHIFT, Mods::SUPER]
        .into_iter()
        .zip(names)
    {
        if mods.contains(held) {
            out.push_str(name);
        }
    }
    out
}

/// The name of a key that isn't a character, from a list in the order of [`Key`]'s variants, and
/// a function key's name with its prefix. F1 to F12 have names.
fn special(key: Key, names: [&str; 14], function: &str) -> Option<String> {
    let index = match key {
        Key::Enter => 0,
        Key::Tab => 1,
        Key::Backspace => 2,
        Key::Escape => 3,
        Key::Up => 4,
        Key::Down => 5,
        Key::Left => 6,
        Key::Right => 7,
        Key::Home => 8,
        Key::End => 9,
        Key::PageUp => 10,
        Key::PageDown => 11,
        Key::Insert => 12,
        Key::Delete => 13,
        Key::F(n @ 1..=12) => return Some(format!("{function}{n}")),
        Key::F(_) | Key::Char(_) => return None,
    };
    Some(names[index].to_owned())
}

/// The X keysym of a character: ASCII letters and digits are their own names, common punctuation
/// has a name, and anything else is its Unicode keysym.
fn keysym(c: char) -> String {
    let named = match c {
        ' ' => "space",
        '!' => "exclam",
        '"' => "quotedbl",
        '#' => "numbersign",
        '$' => "dollar",
        '%' => "percent",
        '&' => "ampersand",
        '\'' => "apostrophe",
        '(' => "parenleft",
        ')' => "parenright",
        '*' => "asterisk",
        '+' => "plus",
        ',' => "comma",
        '-' => "minus",
        '.' => "period",
        '/' => "slash",
        ':' => "colon",
        ';' => "semicolon",
        '<' => "less",
        '=' => "equal",
        '>' => "greater",
        '?' => "question",
        '@' => "at",
        '[' => "bracketleft",
        '\\' => "backslash",
        ']' => "bracketright",
        '^' => "asciicircum",
        '_' => "underscore",
        '`' => "grave",
        '{' => "braceleft",
        '|' => "bar",
        '}' => "braceright",
        '~' => "asciitilde",
        c if c.is_ascii_alphanumeric() => return c.to_string(),
        c => return format!("U{:04X}", u32::from(c)),
    };
    named.to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods::NONE;

    #[test]
    fn tmux_names() {
        assert_eq!(tmux(Key::Char('a'), NONE), None);
        assert_eq!(tmux(Key::Char('a'), Mods::CTRL).as_deref(), Some("C-a"));
        assert_eq!(
            tmux(Key::Char('a'), Mods::CTRL | Mods::ALT).as_deref(),
            Some("C-M-a")
        );
        assert_eq!(tmux(Key::Char('a'), Mods::SHIFT).as_deref(), Some("A"));
        assert_eq!(tmux(Key::Char(' '), Mods::CTRL).as_deref(), Some("C-Space"));
        assert_eq!(tmux(Key::Up, Mods::SHIFT).as_deref(), Some("S-Up"));
        assert_eq!(tmux(Key::Tab, Mods::SHIFT).as_deref(), Some("BTab"));
        assert_eq!(tmux(Key::Backspace, NONE).as_deref(), Some("BSpace"));
        assert_eq!(tmux(Key::Backspace, Mods::CTRL), None);
        assert_eq!(tmux(Key::PageDown, Mods::CTRL).as_deref(), Some("C-NPage"));
        assert_eq!(tmux(Key::F(5), NONE).as_deref(), Some("F5"));
        assert_eq!(tmux(Key::F(13), NONE), None);
    }

    #[test]
    fn kitty_names() {
        assert_eq!(
            kitty(Key::Char('a'), Mods::CTRL | Mods::SHIFT).as_deref(),
            Some("ctrl+shift+a")
        );
        assert_eq!(
            kitty(Key::Enter, Mods::SHIFT).as_deref(),
            Some("shift+enter")
        );
        assert_eq!(kitty(Key::PageUp, NONE).as_deref(), Some("page_up"));
        assert_eq!(
            kitty(Key::Char(' '), Mods::CTRL).as_deref(),
            Some("ctrl+space")
        );
        assert_eq!(kitty(Key::F(1), Mods::ALT).as_deref(), Some("alt+f1"));
    }

    #[test]
    fn xdotool_names() {
        assert_eq!(
            xdotool(Key::Char('a'), Mods::CTRL).as_deref(),
            Some("ctrl+a")
        );
        assert_eq!(xdotool(Key::Char(';'), NONE).as_deref(), Some("semicolon"));
        assert_eq!(xdotool(Key::Char('é'), NONE).as_deref(), Some("U00E9"));
        assert_eq!(
            xdotool(Key::Enter, Mods::SHIFT).as_deref(),
            Some("shift+Return")
        );
        assert_eq!(xdotool(Key::PageDown, NONE).as_deref(), Some("Next"));
    }
}
