//! The text, styled-text and HTML renderings of a screen.

use std::fmt::Write as _;

use termelon_vt::{Emulator, Profile};

#[expect(
    clippy::unwrap_used,
    reason = "a helper of tests, which fail on an error"
)]
fn run(profile: Profile, cols: u16, rows: u16, bytes: &[u8]) -> Emulator {
    let mut term = Emulator::with_profile(profile, cols, rows);
    term.feed(bytes).unwrap();
    term
}

#[test]
fn text_leaves_out_continuation_cells_and_trailing_spaces() {
    let term = run(Profile::xterm(), 8, 2, "a字b  \r\n  x".as_bytes());
    assert_eq!(term.rows(), ["a字b", "  x"]);
    assert_eq!(term.text(), "a字b\n  x");
    assert_eq!(term.row_text(2), None);
}

#[test]
fn styled_text_writes_the_documented_example() {
    let bytes = "\x1b[1;3H\x1b[38;2;127;142;163m⣷\x1b[0m \x1b[38;2;216;222;233;1mWorking… \x1b[0m\x1b[24;3H\x1b[6 q";
    let term = run(Profile::xterm(), 80, 24, bytes.as_bytes());
    let mut expected = String::from(
        "size 80 24\n\
         cursor 2 23 visible bar\n\
         text 0 \"  ⣷ Working…\"\n\
         style 0 2 3 fg=#7f8ea3\n\
         style 0 4 13 fg=#d8dee9 bold\n",
    );
    for y in 1..24 {
        let _ = writeln!(expected, "text {y} \"\"");
    }
    assert_eq!(term.styled(), expected);
}

#[test]
fn styled_text_writes_every_attribute_in_order() {
    let bytes = b"\x1b]8;id=1;https://example.com/x\x1b\\\x1b[31;48;5;8;58;2;1;2;3;4:3;1;2;3;5;6;7;8;9;53mA";
    let term = run(
        Profile {
            hyperlinks: true,
            ..Profile::xterm()
        },
        2,
        1,
        bytes,
    );
    assert_eq!(
        term.styled(),
        "size 2 1\ncursor 1 0 visible default\ntext 0 \"A\"\nstyle 0 0 1 fg=red bg=idx:8 ul=#010203 \
         underline=curly bold dim italic blink rapid-blink reverse hidden strike overline \
         link=https://example.com/x\n"
    );
}

#[test]
fn styled_text_names_the_sixteen_colours() {
    let mut bytes = Vec::new();
    for i in 0..8 {
        bytes.extend_from_slice(format!("\x1b[{}mx", 30 + i).as_bytes());
    }
    for i in 0..8 {
        bytes.extend_from_slice(format!("\x1b[{}mx", 90 + i).as_bytes());
    }
    let term = run(Profile::xterm(), 16, 1, &bytes);
    let names: Vec<String> = term
        .styled()
        .lines()
        .filter_map(|line| line.strip_prefix("style 0 "))
        .map(|rest| rest.split_once("fg=").unwrap().1.to_owned())
        .collect();
    assert_eq!(
        names,
        [
            "black",
            "red",
            "green",
            "yellow",
            "blue",
            "magenta",
            "cyan",
            "gray",
            "dark-gray",
            "light-red",
            "light-green",
            "light-yellow",
            "light-blue",
            "light-magenta",
            "light-cyan",
            "white"
        ]
    );
}

#[test]
fn styled_text_underline_styles() {
    let term = run(
        Profile::xterm(),
        6,
        1,
        b"\x1b[4ma\x1b[4:2mb\x1b[4:4mc\x1b[4:5md\x1b[21me",
    );
    let styled = term.styled();
    let styles: Vec<&str> = styled
        .lines()
        .filter(|l| l.starts_with("style"))
        .map(|l| &l[12..])
        .collect();
    assert_eq!(
        styles,
        [
            "underline",
            "underline=double",
            "underline=dotted",
            "underline=dashed",
            "underline=double"
        ]
    );
}

#[test]
fn styled_text_hidden_cursor() {
    let term = run(Profile::xterm(), 3, 1, b"\x1b[2C\x1b[5 q\x1b[?25l");
    assert_eq!(term.styled(), "size 3 1\ncursor 2 0 hidden\ntext 0 \"\"\n");
}

#[test]
fn styled_text_counts_columns_in_cells() {
    let term = run(
        Profile::xterm(),
        6,
        1,
        "\x1b[1ma字\x1b[0mb\x1b[44m \x1b[0m".as_bytes(),
    );
    assert_eq!(
        term.styled(),
        "size 6 1\ncursor 5 0 visible default\ntext 0 \"a字b\"\nstyle 0 0 3 bold\nstyle 0 4 5 bg=blue\n"
    );
}

#[test]
fn styled_text_escapes_the_row_as_json() {
    let term = run(Profile::xterm(), 6, 1, b"a\"b\\c");
    assert!(term.styled().contains("text 0 \"a\\\"b\\\\c\"\n"));
}

#[test]
fn styled_text_joins_links_to_one_address() {
    let bytes = b"\x1b]8;id=1;https://example.com\x1b\\a\x1b]8;id=2;https://example.com\x1b\\b\x1b]8;;\x1b\\";
    let term = run(Profile::kitty(), 3, 1, bytes);
    assert!(
        term.styled()
            .contains("style 0 0 2 link=https://example.com\n")
    );
}

#[test]
fn html_has_colours_attributes_links_and_the_cursor() {
    let bytes = b"\x1b[31ma<\x1b[0m \x1b[1;4:3;58;5;196mb\x1b[0m\x1b]8;;https://example.com/?a=1&b\x1b\\l\x1b]8;;\x1b\\\x1b[7mr";
    let term = run(Profile::kitty(), 8, 2, bytes);
    let html = term.to_html();
    assert!(
        html.starts_with("<pre class=\"termelon-vt\" style=\"background:#000000;color:#e5e5e5\">")
    );
    assert!(html.ends_with("</pre>"));
    assert!(html.contains("<span style=\"color:#cd0000;\">a&lt;</span>"));
    assert!(html.contains(
        "<span style=\"font-weight:bold;text-decoration-line:underline;text-decoration-style:wavy;\
         text-decoration-color:#ff0000;\">b</span>"
    ));
    assert!(html.contains("<a href=\"https://example.com/?a=1&amp;b\">l</a>"));
    assert!(html.contains("<span style=\"color:#000000;background:#e5e5e5;\">r</span>"));
    assert!(
        html.contains("<span class=\"cursor\" style=\"outline:1px solid currentColor;\"> </span>")
    );
    assert_eq!(html.matches('\n').count(), 1);
}
