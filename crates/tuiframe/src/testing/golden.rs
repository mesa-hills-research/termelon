//! Golden files, which hold what a test expects to see, such as a screen's text or styled text.
//!
//! An assertion compares what the test got with its golden file and fails with the lines that
//! differ. With `UPDATE_GOLDENS=1` set, it writes what the test got to the file instead, creating
//! it and its directory where needed, so a change made on purpose is accepted by running the tests
//! once and reviewing the files' diff. A relative path starts at the package's directory, where
//! `cargo test` runs tests.
//!
//! ```no_run
//! use tuiframe::testing::{TestTerminal, golden};
//!
//! let mut term = TestTerminal::new(80, 24);
//! term.draw(|frame| frame.render_widget("Ready", frame.area())).unwrap();
//! golden::assert_text("tests/goldens/ready.txt", &term.screen().text());
//! golden::assert_styled("tests/goldens/ready.styled", &term.screen().styled());
//! ```

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

/// The lines of context shown around each difference.
const CONTEXT: usize = 2;

/// Checks `actual`, a screen's text, against the golden file at `path`, or writes it there under
/// `UPDATE_GOLDENS=1`. The file ends with a line break, which the comparison leaves out.
///
/// # Panics
///
/// When the file differs from `actual`, with the lines that differ, and when it can't be read, or
/// written under `UPDATE_GOLDENS=1`.
#[track_caller]
pub fn assert_text<P: AsRef<Path>>(path: P, actual: &str) {
    assert_golden(path.as_ref(), actual, Kind::Text);
}

/// Checks `actual`, a screen in the styled-text format, against the golden file at `path`, or
/// writes it there under `UPDATE_GOLDENS=1`.
///
/// # Panics
///
/// When the file differs from `actual`, with the lines that differ, and when it can't be read, or
/// written under `UPDATE_GOLDENS=1`.
#[track_caller]
pub fn assert_styled<P: AsRef<Path>>(path: P, actual: &str) {
    assert_golden(path.as_ref(), actual, Kind::Styled);
}

/// What a golden file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Styled,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Text => "text",
            Kind::Styled => "styled text",
        }
    }
}

#[track_caller]
#[expect(clippy::panic, reason = "an assertion fails its test by panicking")]
fn assert_golden(path: &Path, actual: &str, kind: Kind) {
    let update = std::env::var_os("UPDATE_GOLDENS").is_some_and(|v| !v.is_empty() && v != "0");
    if let Err(message) = check(path, actual, kind, update) {
        panic!("{message}");
    }
}

/// Compares `actual` with the golden file at `path`, or writes it there when `update`, and says
/// what went wrong.
fn check(path: &Path, actual: &str, kind: Kind, update: bool) -> Result<(), String> {
    let mut contents = actual.to_owned();
    if !contents.ends_with('\n') {
        contents.push('\n');
    }
    if update {
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            fs::create_dir_all(dir)
                .map_err(|e| format!("couldn't create {}: {e}", dir.display()))?;
        }
        return fs::write(path, &contents)
            .map_err(|e| format!("couldn't write the golden {}: {e}", path.display()));
    }
    let expected = match fs::read_to_string(path) {
        Ok(expected) => expected,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(format!(
                "the golden {} doesn't exist. Run the test with UPDATE_GOLDENS=1 to write it.",
                path.display()
            ));
        }
        Err(e) => return Err(format!("couldn't read the golden {}: {e}", path.display())),
    };
    // (A file edited on Windows may end its lines with CR LF.)
    let expected = expected.replace("\r\n", "\n");
    if expected == contents {
        return Ok(());
    }
    Err(format!(
        "the {} differs from the golden {}, whose lines are marked - and the test's +. Run the \
         test with UPDATE_GOLDENS=1 to accept it.\n{}",
        kind.name(),
        path.display(),
        diff(&expected, &contents)
    ))
}

/// The lines of `expected` and `actual` that differ, with a little context, in the style of a
/// unified diff.
fn diff(expected: &str, actual: &str) -> String {
    let old: Vec<&str> = expected.lines().collect();
    let new: Vec<&str> = actual.lines().collect();
    let ops = edits(&old, &new);
    let mut out = String::new();
    let mut i = 0;
    while i < ops.len() {
        if ops[i].0 == ' ' {
            i += 1;
            continue;
        }
        // (A hunk runs from the first change to the last one with no more than twice the context
        // between them.)
        let start = i.saturating_sub(CONTEXT);
        let mut end = i;
        let mut quiet = 0;
        while end < ops.len() && quiet <= 2 * CONTEXT {
            quiet = if ops[end].0 == ' ' { quiet + 1 } else { 0 };
            end += 1;
        }
        let end = (end - quiet + CONTEXT).min(ops.len());
        let line = ops[start].1 + 1;
        let _ = writeln!(out, "@@ line {line} @@");
        for &(mark, _, text) in &ops[start..end] {
            let _ = writeln!(out, "{mark} {text}");
        }
        i = end;
    }
    out
}

/// The edits that turn `old` into `new`, each a mark (` `, `-` or `+`), the line's number in
/// `old` (where a `+` line would go), and the line, from a longest common subsequence.
fn edits<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(char, usize, &'a str)> {
    let (n, m) = (old.len(), new.len());
    // (lengths[i][j] is the longest common subsequence of old[i..] and new[j..].)
    let mut lengths = vec![vec![0_usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lengths[i][j] = if old[i] == new[j] {
                lengths[i + 1][j + 1] + 1
            } else {
                lengths[i + 1][j].max(lengths[i][j + 1])
            };
        }
    }
    let mut ops = Vec::with_capacity(n.max(m));
    let (mut i, mut j) = (0, 0);
    while i < n || j < m {
        if i < n && j < m && old[i] == new[j] {
            ops.push((' ', i, old[i]));
            i += 1;
            j += 1;
        } else if i < n && (j == m || lengths[i + 1][j] >= lengths[i][j + 1]) {
            ops.push(('-', i, old[i]));
            i += 1;
        } else {
            ops.push(('+', i, new[j]));
            j += 1;
        }
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A file in a directory of this test run's own.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("tuiframe-golden-{}", std::process::id()));
        dir.join(name)
    }

    #[test]
    fn update_writes_the_golden_and_then_it_passes() {
        let path = scratch("new/screen.txt");
        let _ = fs::remove_file(&path);
        let missing = check(&path, "a\nb", Kind::Text, false);
        assert!(missing.is_err_and(|e| e.contains("UPDATE_GOLDENS=1 to write it")));
        assert_eq!(check(&path, "a\nb", Kind::Text, true), Ok(()));
        assert_eq!(fs::read_to_string(&path).unwrap(), "a\nb\n");
        assert_eq!(check(&path, "a\nb", Kind::Text, false), Ok(()));
        assert_eq!(check(&path, "a\nb\n", Kind::Text, false), Ok(()));
        fs::write(&path, "a\r\nb\r\n").unwrap();
        assert_eq!(check(&path, "a\nb", Kind::Text, false), Ok(()));
        let _ = fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn a_difference_fails_with_the_lines_that_differ() {
        let path = scratch("styled");
        let golden = "size 4 3\ncursor 0 0 hidden\ntext 0 \"one\"\ntext 1 \"two\"\ntext 2 \"\"\n";
        check(&path, golden, Kind::Styled, true).unwrap();
        let changed = golden.replace("\"two\"", "\"too\"");
        let message = check(&path, &changed, Kind::Styled, false).unwrap_err();
        assert!(message.starts_with("the styled text differs from the golden "));
        let diff = message.split_once('\n').unwrap().1;
        assert_eq!(
            diff,
            "@@ line 2 @@\n  cursor 0 0 hidden\n  text 0 \"one\"\n- text 1 \"two\"\n+ text 1 \
             \"too\"\n  text 2 \"\"\n"
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn far_apart_differences_get_hunks_of_their_own() {
        let old: Vec<String> = (0..20).map(|i| format!("row {i}")).collect();
        let mut new = old.clone();
        new[1] = "changed".into();
        new[15] = "changed".into();
        new.push("added".into());
        let diff = diff(&old.join("\n"), &new.join("\n"));
        let hunks: Vec<&str> = diff.lines().filter(|l| l.starts_with("@@")).collect();
        assert_eq!(hunks, ["@@ line 1 @@", "@@ line 14 @@"]);
        assert!(diff.contains("- row 1\n+ changed\n"));
        assert!(diff.contains("  row 19\n+ added\n"));
        assert!(!diff.contains("row 8"));
    }

    #[test]
    fn lines_that_moved_show_as_removed_and_added() {
        assert_eq!(
            diff("a\nb\nc", "b\nc\na"),
            "@@ line 1 @@\n- a\n  b\n  c\n+ a\n"
        );
        assert_eq!(diff("", "x"), "@@ line 1 @@\n+ x\n");
    }
}
