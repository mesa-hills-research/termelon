//! Each profile's answers to the queries a startup probe sends, and what each terminal ignores.

use tuiframe_vt::{Emulator, Event, Profile};

/// The queries, in the order of the startup probe's batch, which ends with DA1.
const QUERIES: [(&str, &str); 11] = [
    ("kitty flags", "\x1b[?u"),
    ("DECRQM 2026", "\x1b[?2026$p"),
    ("DECRQM 2027", "\x1b[?2027$p"),
    ("DECRQM 1004", "\x1b[?1004$p"),
    ("DECRQM 2004", "\x1b[?2004$p"),
    ("XTVERSION", "\x1b[>q"),
    ("DA2", "\x1b[>c"),
    ("OSC 10", "\x1b]10;?\x1b\\"),
    ("OSC 11", "\x1b]11;?\x1b\\"),
    ("XTGETTCAP Tc", "\x1bP+q5463\x1b\\"),
    ("DA1", "\x1b[c"),
];

/// Two more that the probe and the harness send apart from the batch.
const OTHERS: [(&str, &str); 2] = [("cursor report", "\x1b[3;5H\x1b[6n"), ("size", "\x1b[18t")];

struct Expected {
    profile: Profile,
    /// The answers to `QUERIES`, empty where the terminal doesn't answer.
    batch: [&'static str; 11],
    /// The answers to `OTHERS`.
    others: [&'static str; 2],
}

fn expected() -> Vec<Expected> {
    vec![
        Expected {
            profile: Profile::xterm(),
            batch: [
                "",
                "\x1b[?2026;0$y",
                "\x1b[?2027;0$y",
                "\x1b[?1004;2$y",
                "\x1b[?2004;2$y",
                "\x1bP>|XTerm(390)\x1b\\",
                "\x1b[>41;390;0c",
                "\x1b]10;rgb:0000/0000/0000\x1b\\",
                "\x1b]11;rgb:ffff/ffff/ffff\x1b\\",
                "\x1bP0+r5463\x1b\\",
                "\x1b[?64;1;2;6;9;15;16;17;18;21;22;28c",
            ],
            others: ["\x1b[3;5R", "\x1b[8;24;80t"],
        },
        Expected {
            profile: Profile::kitty(),
            batch: [
                "\x1b[?0u",
                "\x1b[?2026;2$y",
                "\x1b[?2027;0$y",
                "\x1b[?1004;2$y",
                "\x1b[?2004;2$y",
                "\x1bP>|kitty(0.32.2)\x1b\\",
                "\x1b[>1;4000;32c",
                "\x1b]10;rgb:dddd/dddd/dddd\x1b\\",
                "\x1b]11;rgb:0000/0000/0000\x1b\\",
                "\x1bP1+r5463\x1b\\",
                "\x1b[?62;c",
            ],
            others: ["\x1b[3;5R", "\x1b[8;24;80t"],
        },
        Expected {
            profile: Profile::konsole(),
            batch: [
                "",
                "",
                "",
                "",
                "",
                "",
                "\x1b[>1;115;0c",
                "\x1b]10;rgb:fcfc/fcfc/fcfc\x1b\\",
                "\x1b]11;rgb:2323/2626/2727\x1b\\",
                "",
                "\x1b[?62;1;4c",
            ],
            others: ["\x1b[3;5R", "\x1b[8;24;80t"],
        },
        Expected {
            profile: Profile::windows_terminal(),
            batch: [
                "\x1b[?0u",
                "\x1b[?2026;2$y",
                "\x1b[?2027;2$y",
                "\x1b[?1004;2$y",
                "\x1b[?2004;2$y",
                "",
                "\x1b[>0;10;1c",
                "\x1b]10;rgb:cccc/cccc/cccc\x1b\\",
                "\x1b]11;rgb:0c0c/0c0c/0c0c\x1b\\",
                "",
                "\x1b[?61;4;6;7;14;21;22;23;24;28;32;42c",
            ],
            others: ["\x1b[3;5R", "\x1b[8;24;80t"],
        },
        Expected {
            profile: Profile::tmux_3_4(),
            batch: [
                "",
                "",
                "",
                "",
                "",
                "\x1bP>|tmux 3.4\x1b\\",
                "\x1b[>84;0;0c",
                "",
                "",
                "",
                "\x1b[?1;2c",
            ],
            others: ["\x1b[3;5R", "\x1b[8;24;80t"],
        },
        Expected {
            profile: Profile::silent(),
            batch: [""; 11],
            others: ["", ""],
        },
    ]
}

#[test]
fn each_profile_answers_each_query() {
    for expected in expected() {
        let name = expected.profile.name.clone();
        for ((query, bytes), answer) in QUERIES
            .iter()
            .chain(&OTHERS)
            .zip(expected.batch.iter().chain(&expected.others))
        {
            let mut term = Emulator::with_profile(expected.profile.clone(), 80, 24);
            term.feed(bytes.as_bytes()).unwrap();
            assert_eq!(
                String::from_utf8(term.take_output()).unwrap(),
                *answer,
                "{name}, {query}"
            );
        }
    }
}

#[test]
fn the_probe_batch_is_answered_in_order() {
    for expected in expected() {
        let mut term = Emulator::with_profile(expected.profile.clone(), 80, 24);
        let batch: String = QUERIES.iter().map(|(_, bytes)| *bytes).collect();
        term.feed(batch.as_bytes()).unwrap();
        assert_eq!(
            String::from_utf8(term.take_output()).unwrap(),
            expected.batch.concat(),
            "{}",
            expected.profile.name
        );
    }
}

#[test]
fn the_linux_console_answers_only_da1_and_the_cursor_report() {
    let mut term = Emulator::with_profile(Profile::linux_console(), 80, 24);
    term.feed(b"\x1b[?u\x1b[?2026$p\x1b[>q\x1b[>c\x1b[18t\x1b[c\x1b[3;5H\x1b[6n")
        .unwrap();
    assert_eq!(term.take_output(), b"\x1b[?6c\x1b[3;5R");
}

#[test]
fn the_linux_console_prints_osc_and_dcs_queries() {
    let mut term = Emulator::with_profile(Profile::linux_console(), 80, 2);
    term.set_strict(false);
    term.feed(b"\x1b]11;?\x1b\\\x1bP+q5463\x1b\\").unwrap();
    assert!(term.take_output().is_empty());
    assert_eq!(term.row_text(0).as_deref(), Some("1;?q5463"));
    assert_eq!(term.unknown_sequences().len(), 2);
}

#[test]
fn decrqm_reports_a_mode_once_set() {
    let mut term = Emulator::with_profile(Profile::kitty(), 80, 24);
    term.feed(b"\x1b[?2026h\x1b[?2004h\x1b[?2026$p\x1b[?2004$p")
        .unwrap();
    assert_eq!(term.take_output(), b"\x1b[?2026;1$y\x1b[?2004;1$y");
}

#[test]
fn kitty_reports_the_flags_pushed() {
    let mut term = Emulator::with_profile(Profile::kitty(), 80, 24);
    term.feed(b"\x1b[>1u\x1b[?u\x1b[>31u\x1b[?u\x1b[<u\x1b[?u")
        .unwrap();
    assert_eq!(term.take_output(), b"\x1b[?1u\x1b[?31u\x1b[?1u");
}

#[test]
fn xtgettcap_answers_known_names_with_values() {
    let mut term = Emulator::with_profile(Profile::kitty(), 80, 24);
    term.feed(b"\x1bP+q544e;436f;5847\x1b\\").unwrap();
    assert_eq!(
        String::from_utf8(term.take_output()).unwrap(),
        "\x1bP1+r544e=787465726D2D6B69747479\x1b\\\x1bP1+r436f=323536\x1b\\\x1bP0+r5847\x1b\\"
    );
}

#[test]
fn tmux_3_4_ignores_synchronized_update() {
    let mut term = Emulator::with_profile(Profile::tmux_3_4(), 10, 1);
    term.feed(b"\x1b[?2026hx\x1b[?2026l").unwrap();
    assert!(
        !term
            .events()
            .iter()
            .any(|e| matches!(e, Event::SyncBegin | Event::SyncEnd))
    );
}

#[test]
fn each_profile_knows_its_synchronized_update() {
    let sync = |profile: Profile| {
        let mut term = Emulator::with_profile(profile, 10, 1);
        term.feed(b"\x1b[?2026h").unwrap();
        term.mode(2026)
    };
    assert!(!sync(Profile::xterm()));
    assert!(sync(Profile::kitty()));
    assert!(!sync(Profile::konsole()));
    assert!(sync(Profile::windows_terminal()));
    assert!(!sync(Profile::tmux_3_4()));
    assert!(!sync(Profile::linux_console()));
}

#[test]
fn each_profile_has_its_width_rules() {
    let width = |profile: Profile, text: &str| {
        let mut term = Emulator::with_profile(profile, 10, 1);
        term.feed(text.as_bytes()).unwrap();
        term.cursor().x
    };
    let family = "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}";
    assert_eq!(width(Profile::xterm(), family), 6);
    assert_eq!(width(Profile::kitty(), family), 2);
    assert_eq!(width(Profile::kitty(), "\u{2764}\u{fe0f}"), 2);
    assert_eq!(width(Profile::windows_terminal(), family), 6);
    assert_eq!(width(Profile::tmux_3_4(), family), 6);
}

#[test]
fn only_some_terminals_keep_hyperlinks() {
    let linked = |profile: Profile| {
        let mut term = Emulator::with_profile(profile, 10, 1);
        term.feed(b"\x1b]8;;https://example.com\x1b\\a").unwrap();
        term.cell(0, 0).unwrap().style().link.is_some()
    };
    assert!(!linked(Profile::xterm()));
    assert!(linked(Profile::kitty()));
    assert!(linked(Profile::konsole()));
    assert!(linked(Profile::windows_terminal()));
    assert!(linked(Profile::tmux_3_4()));
}

#[test]
fn the_silent_terminal_still_draws() {
    let mut term = Emulator::with_profile(Profile::silent(), 10, 1);
    term.feed(b"\x1b[1mhi\x1b[c\x1b[6n").unwrap();
    assert_eq!(term.row_text(0).as_deref(), Some("hi"));
    assert!(term.output().is_empty());
}
