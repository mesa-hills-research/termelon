//! Lists every `unsafe` in the workspace's sources and checks it against the rules in
//! `docs/why-this-is-safe.md`. Unsafe code lives only in the allowed files, every place that
//! asserts soundness (a block, an `unsafe impl`, an `unsafe extern` block or an unsafe attribute)
//! has a `SAFETY` comment directly above it with an id in brackets and a reason, every id has a
//! `## id` entry in the doc, and every entry has code that names it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

/// The only files that may hold unsafe code, relative to the workspace's root.
const ALLOWED: &[&str] = &[
    "crates/termelon/src/sys/macos.rs",
    "crates/termelon/src/sys/windows/console.rs",
    "tools/harness/src/windows.rs",
];

/// The doc that explains each unsafe block, relative to the workspace's root.
const DOC: &str = "docs/why-this-is-safe.md";

/// How a safety comment starts, split so that clippy doesn't take this file's own text for one.
const SAFETY_PREFIX: &str = concat!("// SAFETY", ": [");

/// The form of a safety comment, for messages.
const SAFETY_FORM: &str = concat!("// SAFETY", ": [id] reason");

#[test]
fn every_unsafe_is_allowed_tagged_and_explained() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let files = workspace_sources(&root).expect("read the workspace's sources");
    assert!(
        files
            .iter()
            .any(|(path, _)| path == "crates/termelon/tests/unsafe_inventory.rs"),
        "the walk missed this test's own file, so it isn't reading the workspace"
    );
    let doc = fs::read_to_string(root.join(DOC)).expect("read the doc");
    let errors = check(&files, &doc);
    assert!(errors.is_empty(), "\n{}", errors.join("\n"));
}

/// What follows an `unsafe` keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Block,
    Fn,
    Impl,
    Trait,
    Extern,
    Attribute,
    Other,
}

impl Kind {
    /// Whether the code asserts soundness here, so a `SAFETY` comment must say why it holds. An
    /// `unsafe fn` or `unsafe trait` only states an obligation, which its `# Safety` docs describe.
    fn needs_reason(self) -> bool {
        matches!(
            self,
            Kind::Block | Kind::Impl | Kind::Extern | Kind::Attribute
        )
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Kind::Block => "block",
            Kind::Fn => "fn",
            Kind::Impl => "impl",
            Kind::Trait => "trait",
            Kind::Extern => "extern block",
            Kind::Attribute => "attribute",
            Kind::Other => "keyword",
        })
    }
}

/// An `unsafe` keyword in code, with its 1-based line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Unsafe {
    line: usize,
    kind: Kind,
}

/// Checks the sources against the doc and returns one message per problem, naming the file and
/// line.
fn check(files: &[(String, String)], doc: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let mut entries = BTreeMap::new();
    for (line, id) in doc_entries(doc) {
        match entries.get(id) {
            Some(first) => {
                errors.push(format!(
                    "{DOC}:{line}: entry `{id}` repeats the one at line {first}"
                ));
            }
            None => {
                entries.insert(id, line);
            }
        }
    }
    let mut named = BTreeSet::new();
    for (path, source) in files {
        let lines: Vec<&str> = source.lines().collect();
        let allowed = ALLOWED.contains(&path.as_str());
        for Unsafe { line, kind } in find_unsafe(source) {
            if !allowed {
                errors.push(format!(
                    "{path}:{line}: unsafe {kind} outside the files allowed to hold unsafe code"
                ));
            } else if kind.needs_reason() {
                match safety_id(&lines, line) {
                    None => errors.push(format!(
                        "{path}:{line}: unsafe {kind} without a `{SAFETY_FORM}` comment directly above it"
                    )),
                    Some(id) => {
                        if !entries.contains_key(id) {
                            errors.push(format!(
                                "{path}:{line}: SAFETY id `{id}` has no `## {id}` entry in {DOC}"
                            ));
                        }
                        named.insert(id);
                    }
                }
            }
        }
    }
    for (id, line) in &entries {
        if !named.contains(id) {
            errors.push(format!(
                "{DOC}:{line}: entry `{id}` has no unsafe code that names it"
            ));
        }
    }
    errors
}

/// The `## id` headings of the doc, with their 1-based lines.
fn doc_entries(doc: &str) -> Vec<(usize, &str)> {
    doc.lines()
        .enumerate()
        .filter_map(|(i, text)| Some((i + 1, text.strip_prefix("## ")?.trim())))
        .collect()
}

/// The id of the safety comment in the run of `//` lines directly above line
/// `line` (1-based), if there is one.
fn safety_id<'a>(lines: &[&'a str], line: usize) -> Option<&'a str> {
    lines[..line.saturating_sub(1).min(lines.len())]
        .iter()
        .rev()
        .copied()
        .map(str::trim)
        .take_while(|text| text.starts_with("//"))
        .find_map(|text| {
            let (id, reason) = text.strip_prefix(SAFETY_PREFIX)?.split_once(']')?;
            let well_formed = !id.is_empty()
                && id
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
            (well_formed && reason.starts_with(' ') && !reason.trim().is_empty()).then_some(id)
        })
}

/// Every `unsafe` keyword in code, skipping comments, strings, character literals and raw
/// identifiers.
fn find_unsafe(source: &str) -> Vec<Unsafe> {
    let tokens = tokens(source);
    tokens
        .iter()
        .enumerate()
        .filter(|(_, (_, word))| word == "unsafe")
        .map(|(i, &(line, _))| {
            let next = |n: usize| tokens.get(i + n).map_or("", |(_, word)| word.as_str());
            let kind = match next(1) {
                "{" => Kind::Block,
                "fn" => Kind::Fn,
                "impl" => Kind::Impl,
                "trait" => Kind::Trait,
                "extern" if next(2) == "fn" => Kind::Fn,
                "extern" => Kind::Extern,
                "(" => Kind::Attribute,
                _ => Kind::Other,
            };
            Unsafe { line, kind }
        })
        .collect()
}

/// The identifiers and punctuation of `source`, each with its 1-based line. Literals and comments
/// are left out, which is all this test needs of Rust's grammar.
fn tokens(source: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = source.chars().collect();
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    let mut out = Vec::new();
    let mut line = 1;
    let mut i = 0;
    // Moves past `chars[from..to]`, counting the line breaks in it.
    let skip = |line: &mut usize, from: usize, to: usize| {
        *line += chars[from..to.min(chars.len())]
            .iter()
            .filter(|&&c| c == '\n')
            .count();
        to
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '/' && at(i + 1) == '/' {
            let end = (i..chars.len())
                .find(|&j| chars[j] == '\n')
                .unwrap_or(chars.len());
            i = end;
        } else if c == '/' && at(i + 1) == '*' {
            let mut depth = 0usize;
            let mut j = i;
            while j < chars.len() {
                if at(j) == '/' && at(j + 1) == '*' {
                    depth += 1;
                    j += 2;
                } else if at(j) == '*' && at(j + 1) == '/' {
                    depth -= 1;
                    j += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    j += 1;
                }
            }
            i = skip(&mut line, i, j);
        } else if c == '"' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != '"' {
                j += if chars[j] == '\\' { 2 } else { 1 };
            }
            i = skip(&mut line, i, j + 1);
        } else if c == '\'' {
            if at(i + 1) == '\\' {
                let end = (i + 3..chars.len())
                    .find(|&j| chars[j] == '\'')
                    .unwrap_or(chars.len());
                i = end + 1;
            } else if at(i + 2) == '\'' {
                i += 3;
            } else {
                // A lifetime or a label, whose name follows as an identifier.
                i += 1;
            }
        } else if c.is_alphabetic() || c == '_' {
            let end = (i..chars.len())
                .find(|&j| !(chars[j].is_alphanumeric() || chars[j] == '_'))
                .unwrap_or(chars.len());
            let word: String = chars[i..end].iter().collect();
            i = end;
            if matches!(word.as_str(), "r" | "br" | "cr") && matches!(at(i), '"' | '#') {
                let hashes = (i..chars.len()).take_while(|&j| chars[j] == '#').count();
                if at(i + hashes) == '"' {
                    let close: Vec<char> = std::iter::once('"')
                        .chain(std::iter::repeat_n('#', hashes))
                        .collect();
                    let body = i + hashes + 1;
                    let end = (body..chars.len())
                        .find(|&j| chars[j..].starts_with(&close))
                        .map_or(chars.len(), |j| j + close.len());
                    i = skip(&mut line, i, end);
                    continue;
                }
                if word == "r" && hashes == 1 {
                    // A raw identifier such as `r#unsafe`, which names something and is no keyword.
                    i = (i + 1..chars.len())
                        .find(|&j| !(chars[j].is_alphanumeric() || chars[j] == '_'))
                        .unwrap_or(chars.len());
                    continue;
                }
            }
            out.push((line, word));
        } else {
            if c == '\n' {
                line += 1;
            } else if !c.is_whitespace() {
                out.push((line, c.to_string()));
            }
            i += 1;
        }
    }
    out
}

/// Every `.rs` file under `root`, as paths relative to it with `/` between components, leaving
/// out `target` and hidden directories.
fn workspace_sources(root: &Path) -> io::Result<Vec<(String, String)>> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if entry.file_type()?.is_dir() {
                if name != "target" && !name.starts_with('.') {
                    dirs.push(path);
                }
            } else if name.ends_with(".rs") {
                let relative = path
                    .strip_prefix(root)
                    .map_err(io::Error::other)?
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                files.push((relative, fs::read_to_string(&path)?));
            }
        }
    }
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod scanner {
    use super::*;

    fn kinds(source: &str) -> Vec<(usize, Kind)> {
        find_unsafe(source)
            .into_iter()
            .map(|u| (u.line, u.kind))
            .collect()
    }

    #[test]
    fn finds_each_kind_with_its_line() {
        let source = r#"
fn f() {
    let n = unsafe { g() };
}
unsafe fn g() -> u8 { 0 }
unsafe impl Send for S {}
pub unsafe trait T {}
unsafe extern "C" {
    fn h();
}
unsafe extern "C" fn i() {}
#[unsafe(no_mangle)]
fn j() {}
"#;
        assert_eq!(
            kinds(source),
            [
                (3, Kind::Block),
                (5, Kind::Fn),
                (6, Kind::Impl),
                (7, Kind::Trait),
                (8, Kind::Extern),
                (11, Kind::Fn),
                (12, Kind::Attribute),
            ]
        );
    }

    #[test]
    fn skips_comments_literals_and_other_words() {
        let source = r##"
#![deny(unsafe_code)]
// unsafe { a line comment }
/// unsafe { a doc comment }
//! unsafe { an inner doc comment }
/* unsafe { /* nested */ unsafe { } still a comment */
let s = "unsafe { \" still a string";
let r = r#"unsafe { " still a raw string "#;
let b = b"unsafe";
let c = br"unsafe";
let r#unsafe = 1;
let unsafe_count = 2;
fn f<'a>(x: &'a str) -> char { '"' }
let q = '\''; let n = unsafe { 3 };
let multi = "line one
unsafe {
"; let m = unsafe { 4 };
"##;
        assert_eq!(kinds(source), [(14, Kind::Block), (17, Kind::Block)]);
    }

    #[test]
    fn reads_the_safety_comment_directly_above() {
        let source = "\
fn f() {
    // SAFETY: [read-input] the buffer holds `len` records.
    let a = unsafe { g() };
    // SAFETY: [close-handle] we own the handle,
    // and nothing else closes it.
    unsafe { h() };
    // SAFETY: [spaced] a blank line separates this from the block.

    unsafe { i() };
    // SAFETY: no id
    unsafe { j() };
    // SAFETY: [no-reason]
    unsafe { k() };
    // SAFETY: [Upper-Case] ids are lowercase.
    unsafe { l() };
    let x = 1;
    unsafe { m() };
}
unsafe { n() };";
        let lines: Vec<&str> = source.lines().collect();
        let ids: Vec<Option<&str>> = find_unsafe(source)
            .iter()
            .map(|u| safety_id(&lines, u.line))
            .collect();
        assert_eq!(
            ids,
            [
                Some("read-input"),
                Some("close-handle"),
                None,
                None,
                None,
                None,
                None,
                None,
            ]
        );
    }

    fn files(sources: &[(&str, &str)]) -> Vec<(String, String)> {
        sources
            .iter()
            .map(|&(path, source)| (path.to_owned(), source.to_owned()))
            .collect()
    }

    const MACOS: &str = "crates/termelon/src/sys/macos.rs";

    #[test]
    fn passes_when_every_block_has_its_entry() {
        let source = "\
// SAFETY: [select] the descriptors stay open for the call.
let n = unsafe { select() };
// SAFETY: [select] the same, for the retry.
let n = unsafe { select() };
unsafe fn declared() {}";
        let doc = "# Why this is safe\n\nIntro.\n\n## select\n\nWhy.\n";
        assert_eq!(check(&files(&[(MACOS, source)]), doc), Vec::<String>::new());
        assert_eq!(check(&[], "# Why this is safe\n"), Vec::<String>::new());
    }

    #[test]
    fn names_each_problem_with_its_place() {
        let sources = files(&[
            (
                "crates/termelon/src/buffer.rs",
                "fn f() {\n    // SAFETY: [select] tagged, but in the wrong file.\n    unsafe { g() };\n}",
            ),
            (
                MACOS,
                "unsafe { a() };\n// SAFETY: [unlisted] no entry.\nunsafe { b() };\nunsafe impl Send for S {}",
            ),
        ]);
        let doc = "# Why this is safe\n## orphan\nWhy.\n## orphan\nAgain.\n";
        assert_eq!(
            check(&sources, doc),
            [
                "docs/why-this-is-safe.md:4: entry `orphan` repeats the one at line 2",
                "crates/termelon/src/buffer.rs:3: unsafe block outside the files allowed to hold unsafe code",
                "crates/termelon/src/sys/macos.rs:1: unsafe block without a `// SAFETY: [id] reason` comment directly above it",
                "crates/termelon/src/sys/macos.rs:3: SAFETY id `unlisted` has no `## unlisted` entry in docs/why-this-is-safe.md",
                "crates/termelon/src/sys/macos.rs:4: unsafe impl without a `// SAFETY: [id] reason` comment directly above it",
                "docs/why-this-is-safe.md:2: entry `orphan` has no unsafe code that names it",
            ]
        );
    }
}
