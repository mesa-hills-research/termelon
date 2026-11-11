//! Reading the screen back as text.

use super::Emulator;
use crate::grid::Cell;

impl Emulator {
    /// A row's text, with continuation cells left out and trailing spaces trimmed, or `None` past
    /// the last row.
    pub fn row_text(&self, y: u16) -> Option<String> {
        let row = self.grid().row(y)?;
        let text: String = row.iter().map(Cell::text).collect();
        Some(text.trim_end_matches(' ').to_owned())
    }

    /// Every row's text, as [`row_text`](Emulator::row_text) gives it.
    pub fn rows(&self) -> Vec<String> {
        (0..self.rows).filter_map(|y| self.row_text(y)).collect()
    }

    /// The screen's text, its rows joined with `\n`.
    pub fn text(&self) -> String {
        self.rows().join("\n")
    }
}
