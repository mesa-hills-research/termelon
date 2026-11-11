//! What a coding agent's transcript holds: the user's messages, thinking, replies with bold and code
//! spans, highlighted code, tool calls and their results, diffs and notices, with some Japanese,
//! Chinese and emoji among them.

use crate::model::Color;
use crate::scene::{Rng, Row, Span, accent, dim, on_background, palette, text, wrap};

/// One block of the transcript, before it is laid out for a width.
#[derive(Clone, Debug)]
pub(crate) enum Item {
    /// The user's message, in a band with a bar in the margin.
    User(String),
    /// The agent's thinking, in dim italics.
    Thinking(String),
    /// A reply, as styled words.
    Reply(Vec<Span>),
    /// Highlighted code, a line to a row, behind a gutter.
    Code(Vec<Vec<Span>>),
    /// A tool call with its outcome.
    Call { command: String, outcome: String },
    /// Output of a call, dim.
    Output(Vec<String>),
    /// A diff, each line a kind and its text.
    Diff(Vec<(char, String)>),
    /// A warning.
    Notice(String),
    /// The line that ends a turn.
    Turn(String),
}

/// Lays `items` out for `width` columns, with a blank row after each.
pub(crate) fn layout(items: &[Item], width: u16) -> Vec<Row> {
    let width = usize::from(width);
    let mut rows = Vec::new();
    for item in items {
        item_rows(item, width, &mut rows);
        rows.push(Row::blank());
    }
    rows
}

/// The rows of one item, added to `rows`.
fn item_rows(item: &Item, width: usize, rows: &mut Vec<Row>) {
    let margin = || Span::new("  ", text());
    let inner = width.saturating_sub(4);
    match item {
        Item::User(message) => {
            let bar = accent();
            for line in wrap(&[Span::new(message.clone(), text())], inner) {
                let mut spans = vec![Span::new("▎ ", bar)];
                spans.extend(line);
                rows.push(Row {
                    spans,
                    band: Some(palette::USER),
                });
            }
        }
        Item::Thinking(thought) => {
            let style = dim().italic();
            for line in wrap(&inline(thought, style), inner) {
                rows.push(indented(line));
            }
        }
        Item::Reply(words) => {
            for line in wrap(words, inner) {
                rows.push(indented(line));
            }
        }
        Item::Code(lines) => {
            for line in lines {
                let mut spans = vec![margin(), Span::new("│ ", dim())];
                spans.extend(line.iter().cloned());
                rows.push(Row::of(spans));
            }
        }
        Item::Call { command, outcome } => {
            let mut spans = vec![
                margin(),
                Span::new("●", on_background(palette::SUCCESS)),
                Span::new(format!(" {command}"), text()),
            ];
            if !outcome.is_empty() {
                spans.push(Span::new(format!("  {outcome}"), dim()));
            }
            rows.push(Row::of(spans));
        }
        Item::Output(lines) => {
            for line in lines {
                rows.push(Row::of(vec![
                    margin(),
                    Span::new("│ ", dim()),
                    Span::new(line.clone(), dim()),
                ]));
            }
        }
        Item::Diff(lines) => {
            for (kind, line) in lines {
                let (band, style) = match kind {
                    '-' => (Some(palette::REMOVED), text()),
                    '+' => (Some(palette::ADDED), text()),
                    _ => (None, dim()),
                };
                rows.push(Row {
                    spans: vec![
                        margin(),
                        Span::new(format!("{kind} "), dim()),
                        Span::new(line.clone(), style),
                    ],
                    band,
                });
            }
        }
        Item::Notice(notice) => rows.push(Row::of(vec![
            margin(),
            Span::new(notice.clone(), on_background(palette::WARNING)),
        ])),
        Item::Turn(summary) => {
            rows.push(Row::of(vec![margin(), Span::new(summary.clone(), dim())]));
        }
    }
}

fn indented(line: Vec<Span>) -> Row {
    let mut spans = vec![Span::new("  ", text())];
    spans.extend(line);
    Row::of(spans)
}

/// Text with `**bold**` and `` `code` `` spans, over `base`.
pub(crate) fn inline(source: &str, base: crate::model::Style) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut rest = source;
    let mut bold = false;
    let mut code = false;
    while !rest.is_empty() {
        let next = [("**", rest.find("**")), ("`", rest.find('`'))]
            .into_iter()
            .filter_map(|(marker, at)| at.map(|at| (at, marker)))
            .min();
        let (chunk, marker) = match next {
            Some((at, marker)) => (&rest[..at], marker),
            None => (rest, ""),
        };
        if !chunk.is_empty() {
            let style = if code {
                on_background(palette::STRING)
            } else if bold {
                base.bold().fg(palette::TEXT)
            } else {
                base
            };
            spans.push(Span::new(chunk, style));
        }
        rest = &rest[chunk.len() + marker.len()..];
        match marker {
            "**" => bold = !bold,
            "`" => code = !code,
            _ => {}
        }
    }
    spans
}

/// One word to a span, each with the space after it, in the style it had.
pub(crate) fn words(spans: &[Span]) -> Vec<Span> {
    let mut words: Vec<Span> = Vec::new();
    for span in spans {
        let mut start = 0;
        for (at, _) in span.text.match_indices(' ') {
            match words.last_mut() {
                // A space that starts a span goes to the word before it, so no word is blank.
                Some(previous) if start == at => previous.text.push(' '),
                _ => words.push(Span::new(&span.text[start..=at], span.style)),
            }
            start = at + 1;
        }
        if start < span.text.len() {
            words.push(Span::new(&span.text[start..], span.style));
        }
    }
    words
}

/// `spans` with each blank one joined to the span after it, so every token shows something when it
/// arrives.
fn join_blanks(spans: Vec<Span>) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::new();
    let mut blank = String::new();
    for span in spans {
        if span.text.trim().is_empty() {
            blank.push_str(&span.text);
        } else {
            out.push(Span::new(format!("{blank}{}", span.text), span.style));
            blank.clear();
        }
    }
    out
}

const KEYWORDS: [&str; 18] = [
    "fn", "let", "mut", "pub", "use", "if", "else", "match", "for", "in", "return", "impl",
    "struct", "enum", "while", "loop", "const", "self",
];

/// A line of Rust in the theme's syntax colours.
pub(crate) fn highlight(line: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let chars: Vec<char> = line.chars().collect();
    let mut at = 0;
    let push = |spans: &mut Vec<Span>, from: usize, to: usize, color: Color| {
        let piece: String = chars[from..to].iter().collect();
        spans.push(Span::new(piece, on_background(color)));
    };
    while at < chars.len() {
        let c = chars[at];
        let start = at;
        if c == '/' && chars.get(at + 1) == Some(&'/') {
            push(&mut spans, at, chars.len(), palette::DIM);
            break;
        } else if c == '"' {
            at += 1;
            while at < chars.len() && chars[at] != '"' {
                at += 1;
            }
            at = (at + 1).min(chars.len());
            push(&mut spans, start, at, palette::STRING);
        } else if c.is_ascii_digit() {
            while at < chars.len() && (chars[at].is_ascii_digit() || chars[at] == '_') {
                at += 1;
            }
            push(&mut spans, start, at, palette::NUMBER);
        } else if c.is_alphabetic() || c == '_' {
            while at < chars.len() && (chars[at].is_alphanumeric() || chars[at] == '_') {
                at += 1;
            }
            let word: String = chars[start..at].iter().collect();
            let color = if KEYWORDS.contains(&word.as_str()) {
                palette::KEYWORD
            } else if c.is_uppercase() {
                palette::TYPE
            } else if chars.get(at) == Some(&'(') {
                palette::FUNCTION
            } else if start > 0 && chars[start - 1] == '.' {
                palette::PROPERTY
            } else {
                palette::TEXT
            };
            push(&mut spans, start, at, color);
        } else {
            while at < chars.len()
                && !(chars[at].is_alphanumeric() || matches!(chars[at], '_' | '"' | '/'))
            {
                at += 1;
            }
            if at == start {
                at += 1;
            }
            push(&mut spans, start, at, palette::TEXT);
        }
    }
    spans
}

const FILES: [&str; 8] = [
    "src/parser.rs",
    "src/lexer.rs",
    "src/render/mod.rs",
    "crates/core/src/session.rs",
    "tests/golden.rs",
    "docs/design.md",
    "src/config/loader.rs",
    "src/net/retry.rs",
];

const CASES: [&str; 6] = ["reads", "skips", "keeps", "counts", "wraps", "rejects"];

const COMMANDS: [&str; 6] = [
    "cargo test --lib parser",
    "cargo clippy --all-targets -- -D warnings",
    "cargo fmt --check",
    "rg -n \"Frame\" src",
    "git diff --stat",
    "cargo build --release",
];

const SNIPPETS: [&[&str]; 4] = [
    &[
        "fn blank_lines(input: &str) -> usize {",
        "    let mut count = 0;",
        "    for line in input.lines() {",
        "        if line.trim().is_empty() {",
        "            count += 1; // kept, not skipped",
        "        }",
        "    }",
        "    count",
        "}",
    ],
    &[
        "pub struct Retry {",
        "    attempts: u32,",
        "    delay: Duration,",
        "}",
        "",
        "impl Retry {",
        "    pub fn next(&mut self) -> Option<Duration> {",
        "        self.attempts = self.attempts.checked_sub(1)?;",
        "        Some(self.delay * 2)",
        "    }",
        "}",
    ],
    &[
        "match token {",
        "    Token::Word(text) => out.push_str(text),",
        "    Token::Space(n) => out.push_str(&\" \".repeat(n)),",
        "    Token::End => return Ok(out),",
        "}",
    ],
    &[
        "let config = Config::load(\"settings.toml\")?;",
        "let theme = config.theme.unwrap_or(DEFAULT);",
        "println!(\"{} files in {} ms\", files.len(), 120);",
    ],
];

const DIFFS: [&[(char, &str)]; 3] = [
    &[
        (' ', "    let mut rows = Vec::new();"),
        (
            '-',
            "    for line in text.lines().filter(|l| !l.is_empty()) {",
        ),
        ('+', "    for line in text.lines() {"),
        ('+', "        // an empty line is still a row"),
        (' ', "        rows.push(line);"),
        (' ', "    }"),
    ],
    &[
        ('-', "const MAX_TRIES: u32 = 3;"),
        ('+', "const MAX_TRIES: u32 = 5;"),
        (' ', "const BASE_DELAY_MS: u64 = 200;"),
    ],
    &[
        (' ', "fn width(text: &str) -> usize {"),
        ('-', "    text.chars().count()"),
        ('+', "    text.graphemes(true).map(cell_width).sum()"),
        (' ', "}"),
    ],
];

const ASKS: [&str; 6] = [
    "fix the handling of blank lines in {file}, then run the tests",
    "the retry in {file} gives up too early, can you look at it?",
    "add a test for {file} and update the docs",
    "why does `{cmd}` fail on my machine?",
    "rename the helper in {file} and keep the public API as it is",
    "summarize what changed in {file} since the last release",
];

const THOUGHTS: [&str; 3] = [
    "**Planning** Read {file} first, then the tests that cover it. Check how the end of the input is counted before changing the loop.",
    "The failing test is in {file}. It may be the same cause as the earlier one, so look at `{cmd}` output before editing anything.",
    "**Options** Change the parser, or change the caller. The caller is simpler, but the parser is where the bug is.",
];

const SUMMARIES: [&str; 6] = [
    "The parser skipped blank lines at the end, so the **last test** failed. It keeps them now, and all `12` tests pass.",
    "Fixed. The retry now waits `200 ms`, then doubles the delay up to **five** tries, and it stops on a permanent error.",
    "I renamed the helper in {file} and kept the public API. Nothing else in the workspace used the old name.",
    "パーサーの空行の扱いを直しました。テストもすべて通っています。",
    "已经修复了解析器对空行的处理，所有测试都通过了。 ✅ 12 passed 🎉",
    "Done ✅ The docs are updated too 👍🏽 and the release notes mention the change 🇯🇵",
];

/// Substitutes `{file}` and `{cmd}`.
fn fill(template: &str, file: &str, cmd: &str) -> String {
    template.replace("{file}", file).replace("{cmd}", cmd)
}

/// The items of turn number `turn`. Turns differ in what they hold, and the same turn is always the
/// same.
pub(crate) fn turn(turn: usize) -> Vec<Item> {
    let mut rng = Rng::new(turn as u64 + 1);
    let file = *rng.pick(&FILES);
    let command = *rng.pick(&COMMANDS);
    let module = file
        .rsplit('/')
        .next()
        .map_or(file, |name| name.trim_end_matches(".rs"));
    let mut items = vec![
        Item::User(format!(
            "Turn {turn}: {}",
            fill(rng.pick(&ASKS), file, command)
        )),
        Item::Thinking(fill(rng.pick(&THOUGHTS), file, command)),
        Item::Reply(words(&inline(
            &format!("Reading **{file}** first, then running `{command}`:"),
            text(),
        ))),
        Item::Call {
            command: format!("$ {command}"),
            outcome: format!("{} passed", 8 + rng.below(20)),
        },
        Item::Output(
            (0..2 + rng.below(4))
                .map(|n| {
                    format!(
                        "test {module}::tests::{}_{turn}_{n} ... ok",
                        rng.pick(&CASES)
                    )
                })
                .collect(),
        ),
        Item::Call {
            command: format!("read {file}"),
            outcome: String::new(),
        },
    ];
    if turn % 5 == 3 {
        items.push(Item::Notice(format!("retrying in {} s", 1 + rng.below(4))));
    }
    if turn.is_multiple_of(2) {
        let snippet = SNIPPETS[rng.below(SNIPPETS.len())];
        items.push(Item::Code(
            snippet.iter().map(|line| highlight(line)).collect(),
        ));
    }
    if turn % 3 == 1 {
        let diff = DIFFS[rng.below(DIFFS.len())];
        items.push(Item::Diff(
            diff.iter()
                .map(|(kind, line)| (*kind, (*line).to_string()))
                .collect(),
        ));
    }
    let summary = fill(rng.pick(&SUMMARIES), file, command);
    items.push(Item::Reply(words(&inline(&summary, text()))));
    items.push(Item::Turn(format!(
        "{} s · $0.0{:02}",
        2 + rng.below(30),
        4 + rng.below(40)
    )));
    items
}

/// A transcript of at least `min_rows` rows laid out for `width` columns, from turn 0 on.
pub(crate) fn history(width: u16, min_rows: usize) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut at = 0;
    while rows.len() < min_rows {
        rows.extend(layout(&turn(at), width));
        at += 1;
    }
    rows
}

const REPLY_PARAGRAPHS: [&str; 4] = [
    "I found the cause. The loop in **{file}** drops an empty line before it counts it, so the total is one short whenever the input ends in a blank line.",
    "The fix is small: count the line first, then decide whether to keep it. I also added a test that feeds the parser three blank lines in a row.",
    "Running `cargo test --lib parser` shows **12 passed** and nothing else changed. The release build needs one more check before this is done.",
    "Next I will update the docs. The page for the parser says blank lines are skipped, which was true before this change and is not now.",
];

/// A reply that streams in: its words and code lines, in order.
#[derive(Clone, Debug)]
pub(crate) enum Part {
    Words(Vec<Span>),
    Code(Vec<Vec<Span>>),
}

/// A reply with at least `tokens` words and code tokens.
pub(crate) fn reply(tokens: usize) -> Vec<Part> {
    let mut parts = Vec::new();
    let mut count = 0;
    let mut at = 0;
    while count < tokens {
        let file = FILES[at % FILES.len()];
        if at % 3 == 1 {
            let snippet = SNIPPETS[at % SNIPPETS.len()];
            let lines: Vec<Vec<Span>> = snippet
                .iter()
                .map(|line| join_blanks(highlight(line)))
                .collect();
            count += lines.iter().map(Vec::len).sum::<usize>();
            parts.push(Part::Code(lines));
        } else {
            let paragraph = fill(REPLY_PARAGRAPHS[at % REPLY_PARAGRAPHS.len()], file, "");
            let spans = words(&inline(&paragraph, text()));
            count += spans.len();
            parts.push(Part::Words(spans));
        }
        at += 1;
    }
    parts
}

/// The items of `parts` once `tokens` of their words and code tokens have arrived.
pub(crate) fn reveal(parts: &[Part], tokens: usize) -> Vec<Item> {
    let mut left = tokens;
    let mut items = Vec::new();
    for part in parts {
        if left == 0 {
            break;
        }
        match part {
            Part::Words(words) => {
                let take = left.min(words.len());
                items.push(Item::Reply(words[..take].to_vec()));
                left -= take;
            }
            Part::Code(lines) => {
                let mut shown = Vec::new();
                for line in lines {
                    if left == 0 {
                        break;
                    }
                    let take = left.min(line.len());
                    shown.push(line[..take].to_vec());
                    left -= take;
                }
                items.push(Item::Code(shown));
            }
        }
    }
    items
}

/// The sentence typed into the input box, over and over.
const TYPED: &str = "Please fix the blank line handling in src/parser.rs, add a test with 日本語 text and an emoji 🚀, then update docs/design.md and run cargo test. ";

/// The first `count` grapheme clusters of what is typed, which repeats if it runs out.
pub(crate) fn typed(count: usize) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    TYPED.graphemes(true).cycle().take(count).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::spans_width;

    fn row_width(row: &Row) -> usize {
        spans_width(&row.spans)
    }

    fn text_of(row: &Row) -> String {
        row.spans.iter().map(|span| span.text.as_str()).collect()
    }

    #[test]
    fn inline_marks_bold_and_code() {
        let spans = inline("a **b** and `c` d", dim());
        let pieces: Vec<(&str, bool, bool)> = spans
            .iter()
            .map(|span| {
                (
                    span.text.as_str(),
                    span.style.mods == crate::model::Mods::BOLD,
                    span.style.fg == palette::STRING,
                )
            })
            .collect();
        assert_eq!(
            pieces,
            [
                ("a ", false, false),
                ("b", true, false),
                (" and ", false, false),
                ("c", false, true),
                (" d", false, false)
            ]
        );
    }

    #[test]
    fn inline_without_marks_is_one_span() {
        assert_eq!(
            inline("plain text", dim()),
            [Span::new("plain text", dim())]
        );
        assert!(inline("", dim()).is_empty());
    }

    #[test]
    fn words_keep_the_space_after_each() {
        let spans = words(&[Span::new("a bc ", dim()), Span::new("d", text())]);
        let texts: Vec<&str> = spans.iter().map(|span| span.text.as_str()).collect();
        assert_eq!(texts, ["a ", "bc ", "d"]);
        assert_eq!(spans[2].style, text());
        // A space at the start of a span joins the word before it.
        let joined = words(&[
            Span::new("in ", dim()),
            Span::new("x", text().bold()),
            Span::new(" y z", dim()),
        ]);
        let texts: Vec<&str> = joined.iter().map(|span| span.text.as_str()).collect();
        assert_eq!(texts, ["in ", "x ", "y ", "z"]);
    }

    #[test]
    fn highlight_colours_the_kinds_of_token() {
        let spans = highlight("    let x = foo(\"a\", 42) + Bar::new(); // note");
        let colour = |piece: &str| {
            spans
                .iter()
                .find(|span| span.text == piece)
                .unwrap_or_else(|| panic!("no span {piece:?} in {spans:?}"))
                .style
                .fg
        };
        assert_eq!(colour("let"), palette::KEYWORD);
        assert_eq!(colour("foo"), palette::FUNCTION);
        assert_eq!(colour("\"a\""), palette::STRING);
        assert_eq!(colour("42"), palette::NUMBER);
        assert_eq!(colour("Bar"), palette::TYPE);
        assert_eq!(colour("// note"), palette::DIM);
        let joined: String = spans.iter().map(|span| span.text.as_str()).collect();
        assert_eq!(joined, "    let x = foo(\"a\", 42) + Bar::new(); // note");
    }

    #[test]
    fn every_snippet_highlights_back_to_its_text() {
        for snippet in SNIPPETS {
            for line in snippet {
                let joined: String = highlight(line)
                    .iter()
                    .map(|span| span.text.as_str())
                    .collect();
                assert_eq!(&joined, line);
            }
        }
    }

    #[test]
    fn blanks_join_the_next_span() {
        let spans = join_blanks(highlight("    let x = 1;"));
        let texts: Vec<&str> = spans.iter().map(|span| span.text.as_str()).collect();
        assert_eq!(texts, ["    let", " x", " = ", "1", ";"]);
        assert!(join_blanks(highlight("   ")).is_empty());
    }

    #[test]
    fn user_messages_are_a_band_with_a_bar() {
        let rows = layout(&[Item::User("hello there general kenobi".into())], 20);
        assert_eq!(rows.len(), 3, "two rows of text and a blank one");
        assert_eq!(rows[0].band, Some(palette::USER));
        assert!(text_of(&rows[0]).starts_with("▎ hello there"));
        assert_eq!(rows[2], Row::blank());
        assert!(rows.iter().all(|row| row_width(row) <= 20));
    }

    #[test]
    fn diffs_tint_their_changed_lines() {
        let rows = layout(
            &[Item::Diff(vec![
                (' ', "a".into()),
                ('-', "b".into()),
                ('+', "c".into()),
            ])],
            40,
        );
        assert_eq!(rows[0].band, None);
        assert_eq!(rows[1].band, Some(palette::REMOVED));
        assert_eq!(rows[2].band, Some(palette::ADDED));
        assert_eq!(text_of(&rows[1]), "  - b");
    }

    #[test]
    fn a_turn_is_the_same_every_time_and_turns_differ() {
        let render = |n| {
            layout(&turn(n), 100)
                .iter()
                .map(text_of)
                .collect::<Vec<_>>()
        };
        assert_eq!(render(7), render(7));
        assert_ne!(render(7), render(8));
    }

    #[test]
    fn the_history_is_long_enough_and_fits_the_width() {
        for width in [80, 120, 300] {
            let rows = history(width, 500);
            assert!(rows.len() >= 500);
            assert!(rows.iter().all(|row| row_width(row) <= usize::from(width)));
        }
    }

    #[test]
    fn the_history_has_every_kind_of_content() {
        let rows = history(120, 600);
        let all: String = rows.iter().map(text_of).collect::<Vec<_>>().join("\n");
        for wanted in [
            "▎ Turn 0:",
            "Reading ",
            "● $ cargo",
            "│ test parser",
            "retrying in",
            "  - ",
            "  + ",
            "パーサー",
            "已经",
            "🎉",
            "👍🏽",
        ] {
            assert!(all.contains(wanted), "no {wanted:?}");
        }
        let styles: std::collections::HashSet<_> = rows
            .iter()
            .flat_map(|row| {
                row.spans
                    .iter()
                    .map(|span| (span.style.fg, span.style.mods))
            })
            .collect();
        assert!(styles.len() >= 12, "{} styles", styles.len());
        assert!(rows.iter().any(|row| row.band == Some(palette::USER)));
    }

    #[test]
    fn a_reply_reveals_one_word_at_a_time() {
        let parts = reply(300);
        let count = |tokens| -> usize {
            reveal(&parts, tokens)
                .iter()
                .map(|item| match item {
                    Item::Reply(words) => words.len(),
                    Item::Code(lines) => lines.iter().map(Vec::len).sum(),
                    _ => 0,
                })
                .sum()
        };
        assert_eq!(count(0), 0);
        for tokens in [1, 17, 150, 300] {
            assert_eq!(count(tokens), tokens);
        }
        assert!(
            reveal(&parts, 300)
                .iter()
                .any(|item| matches!(item, Item::Code(_)))
        );
    }

    #[test]
    fn a_reply_grows_by_whole_words() {
        let parts = reply(100);
        let rows = |tokens| layout(&reveal(&parts, tokens), 40);
        for tokens in 1..60 {
            let (before, after) = (rows(tokens), rows(tokens + 1));
            assert!(after.len() >= before.len());
            assert!(after.len() <= before.len() + 3);
        }
    }

    #[test]
    fn typing_repeats_when_it_runs_out() {
        assert_eq!(typed(0), "");
        assert_eq!(typed(6), "Please");
        let long = typed(500);
        assert_eq!(long.graphemes_count(), 500);
        assert!(typed(150).contains("日本語"));
    }

    trait Count {
        fn graphemes_count(&self) -> usize;
    }

    impl Count for String {
        fn graphemes_count(&self) -> usize {
            use unicode_segmentation::UnicodeSegmentation;
            self.graphemes(true).count()
        }
    }
}
