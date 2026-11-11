//! The console's modes, saved at open, set for a program that owns the console, and put back
//! exactly at close. The code pages stay as they are, since the library reads and writes the
//! console only in UTF-16.

use std::io;
use std::sync::{Arc, Mutex};

use super::{Console, InputMode};
use crate::output::writer::lock;

/// The bits of the console's modes, from `wincon.h`.
pub(crate) mod bits {
    #[cfg(test)]
    pub(crate) const ENABLE_PROCESSED_INPUT: u32 = 0x0001;
    #[cfg(test)]
    pub(crate) const ENABLE_LINE_INPUT: u32 = 0x0002;
    #[cfg(test)]
    pub(crate) const ENABLE_ECHO_INPUT: u32 = 0x0004;
    pub(crate) const ENABLE_WINDOW_INPUT: u32 = 0x0008;
    pub(crate) const ENABLE_MOUSE_INPUT: u32 = 0x0010;
    pub(crate) const ENABLE_QUICK_EDIT_MODE: u32 = 0x0040;
    pub(crate) const ENABLE_EXTENDED_FLAGS: u32 = 0x0080;
    pub(crate) const ENABLE_VIRTUAL_TERMINAL_INPUT: u32 = 0x0200;
    pub(crate) const ENABLE_PROCESSED_OUTPUT: u32 = 0x0001;
    pub(crate) const ENABLE_VIRTUAL_TERMINAL_PROCESSING: u32 = 0x0004;
    pub(crate) const DISABLE_NEWLINE_AUTO_RETURN: u32 = 0x0008;
}

use bits::{
    DISABLE_NEWLINE_AUTO_RETURN, ENABLE_EXTENDED_FLAGS, ENABLE_MOUSE_INPUT,
    ENABLE_PROCESSED_OUTPUT, ENABLE_QUICK_EDIT_MODE, ENABLE_VIRTUAL_TERMINAL_INPUT,
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, ENABLE_WINDOW_INPUT,
};

/// The modes as the program found them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Saved {
    pub(crate) input: u32,
    pub(crate) output: u32,
}

/// What the program wants from the console's input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct InputSettings {
    pub(crate) mode: InputMode,
    /// Mouse records, which take the mouse from the console's own selection.
    pub(crate) mouse: bool,
}

/// The input mode a program that owns the console runs in. Keys arrive one at a time with no echo
/// and no line editing, Ctrl+C arrives as a key, and window size changes arrive as records. With
/// the mouse on, the console's quick edit is off, so clicks reach the program. With it off, quick
/// edit is as the program found it, so the user can still select text.
pub(crate) fn input_mode(saved: u32, settings: InputSettings) -> u32 {
    let mut mode = ENABLE_WINDOW_INPUT | ENABLE_EXTENDED_FLAGS;
    if settings.mouse {
        mode |= ENABLE_MOUSE_INPUT;
    } else {
        mode |= saved & ENABLE_QUICK_EDIT_MODE;
    }
    if settings.mode == InputMode::Vt {
        mode |= ENABLE_VIRTUAL_TERMINAL_INPUT;
    }
    mode
}

/// The output mode, with the console reading escape sequences, as frames are written in them, and
/// without the line feed that also returns the cursor.
pub(crate) const OUTPUT_MODE: u32 =
    ENABLE_PROCESSED_OUTPUT | ENABLE_VIRTUAL_TERMINAL_PROCESSING | DISABLE_NEWLINE_AUTO_RETURN;

/// The console's modes while a program owns it.
#[derive(Debug)]
pub(crate) struct Modes<C> {
    console: Arc<C>,
    saved: Saved,
    settings: Mutex<InputSettings>,
}

impl<C: Console> Modes<C> {
    /// Saves the modes, then sets the output mode and the input mode for `settings`. A console
    /// without virtual terminal processing fails with [`io::ErrorKind::Unsupported`]. When it
    /// fails, the console is as it was.
    pub(crate) fn enter(console: Arc<C>, settings: InputSettings) -> io::Result<Self> {
        let saved = Saved {
            input: console.input_mode()?,
            output: console.output_mode()?,
        };
        let modes = Self {
            console,
            saved,
            settings: Mutex::new(settings),
        };
        if let Err(error) = modes.console.set_output_mode(OUTPUT_MODE) {
            let _ = modes.restore();
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                format!("the console has no virtual terminal processing ({error})"),
            ));
        }
        if let Err(error) = modes.apply() {
            let _ = modes.restore();
            return Err(error);
        }
        Ok(modes)
    }

    /// The modes as the program found them.
    #[cfg(test)]
    pub(crate) fn saved(&self) -> Saved {
        self.saved
    }

    /// Sets the modes a program owns the console with, again.
    pub(crate) fn apply(&self) -> io::Result<()> {
        let settings = *lock(&self.settings);
        self.console.set_output_mode(OUTPUT_MODE)?;
        self.console
            .set_input_mode(input_mode(self.saved.input, settings))
    }

    /// Changes what the program wants from the input.
    pub(crate) fn set_input(&self, settings: InputSettings) -> io::Result<()> {
        *lock(&self.settings) = settings;
        self.console
            .set_input_mode(input_mode(self.saved.input, settings))
    }

    /// What the program wants from the input now.
    pub(crate) fn input(&self) -> InputSettings {
        *lock(&self.settings)
    }

    /// Puts the modes back as the program found them. It tries both, and returns the first
    /// failure.
    pub(crate) fn restore(&self) -> io::Result<()> {
        let input = self.console.set_input_mode(self.saved.input);
        let output = self.console.set_output_mode(self.saved.output);
        input.and(output)
    }
}
