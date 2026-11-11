# Testing in real terminals

`tools/harness` starts a real terminal running `tools/probe`, waits until the probe's scene shows,
reads the screen back, and compares it cell by cell with the screen `tuiframe-vt` computes for the
probe's frame. Text and colours always count. Attributes and the cursor count where the terminal's
readback reports them.

The probe opens the terminal with the framework, which asks what it supports and measures its
widths, checks that it has the size the harness asked for, and draws the scene as a frame ending
with a marker on the last row. Once the screen is read back, the harness presses `q`, and the probe
saves every byte the framework wrote to the terminal, from its setup and queries to the frame, and
writes a report of the capabilities it found and the events it received.

```sh
cargo build -p harness -p probe
target/debug/harness smoke tmux
target/debug/harness run tmux --probe target/debug/probe --out harness-out
```

`smoke` starts the terminal with `echo` and reads it back, which tells a broken setup from a
failure of the program under test. `run` runs every scene the probe lists (or those named with
`--scene`) at 80 by 24 (or `--size`). For each scene it writes the probe's bytes and report, a JSON
report with each difference, both screens in the styled-text format, an HTML page with the two side
by side, and a screenshot where there's a display. It exits with 1 when a screen differs.

| Terminal | Needs | Read back with | Reports |
|---|---|---|---|
| tmux | tmux | `capture-pane -p -e -N` | text, colours, attributes, cursor |
| GNU screen | screen and tmux | `hardcopy` | Latin-1 text only |
| kitty | kitty, an X display, xdotool | `kitten @ get-text --ansi --add-cursor` | text, colours, attributes, cursor |
| xterm | xterm, the `9x15` font, an X display, xdotool | the `CSI 10 i` XHTML dump | text, RGB colours, bold, italic, underline, strike |
| WezTerm | wezterm, an X display, xdotool | `wezterm cli get-text --escapes` | text, colours, attributes, cursor |
| xterm.js | node, and `npm ci` in `tools/harness/xtermjs` | `@xterm/headless`'s buffer, through a node script | text, colours, attributes, links, cursor |
| VTE | Python's GObject bindings, VTE 2.91 (0.76 or later), a display | `get_text_format(HTML)` in `tools/harness/vte` | text, RGB colours, attributes |
| Alacritty | alacritty, an X display | `--ref-test`'s `grid.json`, written as the window closes | text, colours, attributes, links |
| Konsole | Konsole 24.02 or later, an X display, a D-Bus session, gdbus | D-Bus `getAllDisplayedText` | text only |
| foot | foot and wtype, under a Wayland compositor | `pipe-visible`, bound to a key | text only |
| Ghostty | Ghostty on macOS, with Accessibility for `osascript` | the `write_screen_file` action | text only |

Alacritty reports its screen only as its window closes, so a readback ends the program, and the
driver reads once after the program has had a moment to draw. xterm.js takes keys only as browser
events, so its driver sends bytes, and mouse events through xterm.js's own mouse service.

## Keys and mouse

`keys` runs the probe as `probe --keys MODE`, which turns the mode on, shows a marker and saves
every byte it reads. The harness presses a fixed list of keys and mouse events through the
terminal's own encoder, with a `|` after each, and holds what the terminal sent against what
`tuiframe-vt` sends under that terminal's profile. The modes are `legacy`, `disambiguate` (the
kitty keyboard protocol's flag 1) and `modify-other-keys` (xterm's level 2). Mouse events go in
the legacy mode alone, since the keyboard modes leave them as they are.

```sh
target/debug/harness keys tmux --probe target/debug/probe --out harness-out
```

| Terminal | Keys | Mouse |
|---|---|---|
| tmux | `send-keys`, all but ctrl+backspace, which tmux 3.6 has no name for | none |
| GNU screen | the tmux pane's `send-keys`, all but ctrl+backspace and ctrl+a, screen's command key | none |
| VTE | GTK key events | GTK button and scroll events |
| xterm.js | none, since it takes keys as browser events | its mouse service |

The other drivers press keys with the tools in the table above, and make no mouse events.

Each terminal, version and mode has a fixture in `crates/tuiframe/tests/fixtures/input`, with a
line per event and the bytes in hex. `keys` names every change since the fixture, and `--update`
writes it again. The decoder's tests decode each fixture, and the `decoded.txt` beside it lists
the events that decode as another, such as ctrl+1, which these terminals send as `1`.

Under X, start Xvfb first and set `DISPLAY`, and for foot a headless sway and `WAYLAND_DISPLAY`, as
`.github/workflows/terminals.yml` does. Screenshots
are taken with ImageMagick's `import` where it's installed. They're for a person to look at, and
never decide a result.

The harness's own tests run tmux and screen where they're installed, and check the drivers'
parsers on saved readbacks everywhere:

```sh
cargo test -p harness
```
