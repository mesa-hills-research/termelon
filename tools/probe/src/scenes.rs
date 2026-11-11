//! Fixed scenes, as the bytes a program would write. Each fits in 80 by 24 cells and ends with
//! its marker on the last row. Rows start with a cursor move rather than a line feed, since the
//! terminal's line discipline turns LF into CR LF.

/// A scene: its name, and its bytes, which [`bytes`] finishes with the marker.
pub(crate) struct Scene {
    /// The name the harness asks for.
    pub(crate) name: &'static str,
    body: &'static str,
    /// What comes after the marker, such as a final cursor move.
    after: &'static str,
}

/// Every scene.
pub(crate) const SCENES: [Scene; 4] = [
    Scene {
        name: "text",
        body: "\x1b[0m\x1b[H\x1b[2J\
               \x1b[1;1Hprobe stand-in: text\
               \x1b[3;1Hplain ASCII, punctuation !\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~\
               \x1b[4;1HLatin-1: café ñ ß ü ¿\
               \x1b[5;1HCJK: 字幕 한국어 カタカナ\
               \x1b[6;1HCombining: e\u{301} a\u{308} n\u{303}\
               \x1b[7;1HBox: ┌──┐ │ok│ └──┘ ═ ║\
               \x1b[8;1HSymbols: ⣷ … → ✓ • ± °\
               \x1b[10;40Hcolumn 40\
               \x1b[12;75Hedge!\
               \x1b[13;1Hoverwritten\x1b[13;1HOVER",
        after: "",
    },
    Scene {
        name: "styles",
        body: "\x1b[0m\x1b[H\x1b[2J\
               \x1b[1;1H\x1b[1mbold\x1b[0m \x1b[2mdim\x1b[0m \x1b[3mitalic\x1b[0m \x1b[4munderline\x1b[0m \
               \x1b[7mreverse\x1b[0m \x1b[9mstrike\x1b[0m \x1b[5mblink\x1b[0m \x1b[53moverline\x1b[0m\
               \x1b[2;1H\x1b[4:2mdouble\x1b[0m \x1b[4:3mcurly\x1b[0m \x1b[4:4mdotted\x1b[0m \x1b[4:5mdashed\x1b[0m \
               \x1b[4;58;2;255;0;0mred line\x1b[0m \x1b[4;58;5;33mindexed line\x1b[0m\
               \x1b[3;1H\x1b[30m30\x1b[31m31\x1b[32m32\x1b[33m33\x1b[34m34\x1b[35m35\x1b[36m36\x1b[37m37\
               \x1b[90m90\x1b[91m91\x1b[92m92\x1b[93m93\x1b[94m94\x1b[95m95\x1b[96m96\x1b[97m97\x1b[0m\
               \x1b[4;1H\x1b[41m41\x1b[42m42\x1b[43m43\x1b[44m44\x1b[45m45\x1b[46m46\x1b[47m47\
               \x1b[100m100\x1b[101m101\x1b[107m107\x1b[0m\
               \x1b[5;1H\x1b[38;5;1midx1\x1b[0m \x1b[38;5;196midx196\x1b[0m \x1b[48;5;236m bg236 \x1b[0m \
               \x1b[38;5;250midx250\x1b[0m\
               \x1b[6;1H\x1b[38;2;216;222;233mrgb fg\x1b[0m \x1b[48;2;30;40;50m rgb bg \x1b[0m \
               \x1b[38:2::127:142:163mcolon rgb\x1b[0m \x1b[38:5:208mcolon idx\x1b[0m\
               \x1b[7;1H\x1b[1;3;4;31mstacked\x1b[22;23;24mreset parts\x1b[0m \x1b[1;2mbold dim\x1b[22mnone\x1b[0m\
               \x1b[8;1H\x1b]8;id=probe;https://example.com/docs\x1b\\a link\x1b]8;;\x1b\\ after\
               \x1b[9;1H\x1b[44m  blue  \x1b[0m  \x1b[7;32m green reversed \x1b[0m",
        after: "",
    },
    Scene {
        name: "erase",
        body: "\x1b[0m\x1b[H\x1b[2J\
               \x1b[1;1Hline one is long and gets erased on the right\x1b[1;20H\x1b[K\
               \x1b[2;1Hline two is erased on the left\x1b[2;10H\x1b[1K\
               \x1b[3;1H\x1b[44m\x1b[K\x1b[0mblue background to the edge\
               \x1b[5;1Hregion top\x1b[6;1Hregion 2\x1b[7;1Hregion 3\x1b[8;1Hregion bottom\
               \x1b[5;8r\x1b[8;1H\x1bD\x1b[8;1Hscrolled in\x1b[5;1H\x1bMreverse\x1b[r\
               \x1b[10;1H\x1b[2Sscrolled up twice\
               \x1b[14;1Hgone\x1b[14;1H\x1b[2K\
               \x1b[15;5Hcursor moves\x1b[2A\x1b[3Cup\x1b[4B\x1b[10Ddown\x1b[17;1H\x1b[30Gcha\x1b[20dvpa",
        after: "",
    },
    Scene {
        name: "cursor",
        body: "\x1b[0m\x1b[H\x1b[2J\x1b[1;1Hthe cursor ends hidden at 20,10 as a bar\x1b[5 q",
        after: "\x1b[11;21H\x1b[?25l",
    },
];

/// The scene with a name.
pub(crate) fn find(name: &str) -> Option<&'static Scene> {
    SCENES.iter().find(|scene| scene.name == name)
}

/// A scene's bytes, its marker on the last row of 24 included.
pub(crate) fn bytes(scene: &Scene) -> Vec<u8> {
    format!(
        "{}\x1b[0m\x1b[24;1Hprobe-done:{}{}",
        scene.body, scene.name, scene.after
    )
    .into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_scene_is_one_tuiframe_vt_knows() {
        for scene in &SCENES {
            let mut term = tuiframe_vt::Emulator::new(80, 24);
            term.feed(&bytes(scene)).unwrap();
            assert!(
                term.text().contains(&format!("probe-done:{}", scene.name)),
                "{}",
                scene.name
            );
            assert!(!bytes(scene).contains(&b'\n'), "{}", scene.name);
        }
    }
}
