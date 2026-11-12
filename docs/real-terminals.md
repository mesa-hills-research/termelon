# Testing in real terminals

`tools/harness` starts a real terminal running `tools/probe`, waits until the probe's scene shows,
reads the screen back, and compares it cell by cell with the screen `termelon-vt` computes for the
probe's frame under that terminal's profile. Text, in Unicode's composed form, and colours always
count. Attributes and the cursor count where the terminal's readback reports them.

The probe opens the terminal with the framework, which asks what it supports and measures its
widths, waits for the size the harness asked for, and draws the scene as a frame ending with a
marker on the last row. It saves every byte the framework wrote to the terminal, from its setup and
queries to the frame, and a report of the capabilities it found, once the frame is drawn and again
when the harness presses `q` to end it.

```sh
cargo build -p harness -p probe
target/debug/harness smoke tmux
target/debug/harness run tmux --probe target/debug/probe --out harness-out
```

`smoke` starts the terminal with `echo` and reads it back, which tells a broken setup from a
failure of the program under test. `run` runs every scene the probe lists (or those named with
`--scene`) at 80 by 24 (or `--size`). For each scene it writes the probe's bytes and report, the
terminal's readback as the terminal gave it, a JSON report with each difference, both screens in
the styled-text format, an HTML page with the two side by side, and a screenshot where there's a
display. Then it holds what the terminal answered the probe (its name and version, and the
capabilities the framework found, widths among them) against the expected report for that version
in `tools/harness/expected`, and names every change. It exits with 1 when a screen differs or an
answer changed.

## The terminals

`.github/workflows/terminals.yml` runs a job per terminal and runner on GitHub's hosted runners.

| Terminal | Runner and version | Read back with | Compares | Keys |
|---|---|---|---|---|
| tmux | ubuntu-24.04 (3.4), ubuntu-26.04 (3.6), macos-26 (3.8) | `capture-pane -p -e -N` | text, colours, attributes, cursor | `send-keys` |
| GNU screen | ubuntu-26.04 (4.9.1), macos-26 (5.0.2), in a tmux pane | `hardcopy` | Latin-1 text | the pane's `send-keys` |
| kitty | ubuntu-24.04 (0.32.2), ubuntu-26.04 (0.45.0) | `kitten @ get-text --ansi --add-cursor` | text, colours, attributes, cursor | `kitten @ send-key` |
| xterm | ubuntu-24.04 (390) | the `CSI 10 i` XHTML dump | text, RGB colours, bold, italic, underline, strike | xdotool |
| WezTerm | ubuntu-24.04 (20240203) | `wezterm cli get-text --escapes` | text, colours, attributes, cursor | xdotool |
| xterm.js | ubuntu-24.04 (`@xterm/headless` 6.0.0) | its buffer, through a node script | text, colours, attributes, links, cursor | none, mouse through its mouse service |
| VTE | ubuntu-24.04 (0.76.0) | `get_text_format(HTML)` in `tools/harness/vte` | text, RGB colours, attributes | GTK key, button and scroll events |
| Alacritty | ubuntu-24.04 (0.13.2) | `--ref-test`'s `grid.json`, written as the window closes | text, colours, attributes, links | xdotool |
| Konsole | ubuntu-26.04 (25.12.3) | D-Bus `getAllDisplayedText` | text | xdotool |
| foot | ubuntu-26.04 (1.25.0), under headless sway | `pipe-visible`, bound to a key | text | none |
| Ghostty | macos-26 (1.3.1) | the `write_screen_file` action | text | none |
| Linux console | ubuntu-24.04 (kernel 7.0), console 12 | `/dev/vcsu12`, and the cursor from `/dev/vcsa12` | text, cursor | none |
| Windows Terminal | windows-2025 (1.23), windows-2022 (the portable build of 1.25) | the `exportBuffer` action, on a key the driver's settings bind | text | none |
| conhost | windows-2022, windows-2025 | `ReadConsoleOutputW`, through a helper attached to the program's console | text | none |

Some readbacks can't say everything the terminal draws, so those parts aren't compared. Each
driver's `parse` says what its readback leaves out.

- kitty writes dotted and dashed underlines as none, and the palette's first sixteen colours as
  the named ones.
- WezTerm's `get-text` writes no overline, no curly, dotted or dashed underline and no underline
  colour, and ends a row's blanks with `CSI K` and the last background still set.
- xterm's dump gives every space as U+00A0 and a blinking cell as white on red.
- The Linux console keeps a code point per cell, and U+200B in the second cell of a wide character.

On Windows the program runs under `harness-console`, a helper the harness builds beside itself,
which starts it in the new console at the size asked for. A second one, attached to that console,
types with `WriteConsoleInputW` and, for conhost, reads the window. Windows Terminal is read back
by bringing its window to the front and pressing the key its `exportBuffer` action is bound to,
with `SendInput`, and its keys go the same way. The driver writes Windows Terminal's settings and
puts back the ones it found when it's done. Screenshots on Windows come from .NET's
`CopyFromScreen`.

Alacritty reports its screen only as its window closes, so a readback ends the program, and the
driver reads once after the program has had a moment to draw. The probe has saved its bytes and
report by then. GNU screen runs in a tmux pane of the size wanted, since a detached screen keeps
80 by 24, and its Linux job takes Ubuntu 26.04, where tmux 3.6 presses keys as a terminal sends
them. tmux 3.4's `send-keys` drops Ctrl+Enter and Ctrl+1 and types Shift+Enter's name.

## Keys and mouse

`keys` runs the probe as `probe --keys MODE`, which turns the mode on, shows a marker and saves
every byte it reads. The harness presses a fixed list of keys and mouse events through the
terminal's own encoder, with a `/` after each, and holds what the terminal sent against what
`termelon-vt` sends under that terminal's profile. The modes are `legacy`, `disambiguate` (the
kitty keyboard protocol's flag 1) and `modify-other-keys` (xterm's level 2). Mouse events go in the
legacy mode alone, since the keyboard modes leave them as they are.

```sh
target/debug/harness keys tmux --probe target/debug/probe --out harness-out
```

Where a driver's tool can't press a key as a keyboard does, it leaves that key out.

- tmux has no name for Ctrl+Backspace, and before 3.5 types Shift+Enter's name where it can't
  encode it.
- Ctrl+A is GNU screen's command key.
- kitty's `send-key` gives a key no text, so Shift with a character and é come out wrong.
- Under Xvfb, Konsole reads xdotool's function keys as if Alt were held.
- foot's `wtype` has one level per key and sometimes loses the first key after a new keymap, and
  on macOS System Events types é as another key, and the system keeps Ctrl+Left and Ctrl+F5. foot
  and Ghostty run no key checks.
- Nothing can type into the Linux console, since the kernel's `dev.tty.legacy_tiocsti` is off.

Each terminal, version and mode has a fixture in `crates/termelon/tests/fixtures/input`, with a
line per event and the bytes in hex. `keys` names every change since the fixture. The decoder's
tests decode each fixture, and the `decoded.txt` beside it lists the events that decode as
another, such as Ctrl+1, which most of these terminals send as `1`.

## A new terminal version

A version with no expected report or fixture yet is only noted. Every `run` and `keys` writes what
it found into the output directory's `expected` and `fixtures`, so a runner's new version gets its
files from the run's artifact. Copy them into `tools/harness/expected` and
`crates/termelon/tests/fixtures/input`, then write the `decoded.txt` files.

```sh
UPDATE_GOLDENS=1 cargo test -p termelon --lib real_terminals_bytes_decode
```

`--update` writes the files in place instead, where the terminal runs locally. The parsers' tests
in `tools/harness/tests/samples.rs` read readbacks recorded on the runners, in
`tools/harness/tests/samples/recorded`, each beside the bytes the probe wrote there, so a parser
change is tested without the terminal.

## Which jobs are required

Jobs run on every push to main (tmux, xterm.js, and kitty's smoke test), nightly, and on demand. A
job is required, without `continue-on-error`, once it passed three runs in a row with its expected
reports and fixtures in place. Every Linux and macOS job but the Linux console's did so, so a
failure there fails the run.

| Job | Required |
|---|---|
| tmux, on each runner | yes |
| GNU screen, on each runner | yes |
| kitty, on each runner | yes |
| xterm | yes |
| xterm.js | yes |
| VTE | yes |
| WezTerm | yes |
| Alacritty | yes |
| Konsole | yes |
| foot | yes |
| Ghostty | yes |
| Windows Terminal, on each runner | no, it has no expected reports yet |
| conhost, on each runner | no, it has no expected reports yet |
| Linux console | no, it's an experiment |

## Setting up a runner

Under X, start Xvfb first and set `DISPLAY`. foot needs a headless sway whose windows float, so
they keep the size foot asks for, and `WAYLAND_DISPLAY`. Konsole needs a D-Bus session
(`dbus-run-session`). Alacritty needs `libxkbcommon-x11-0`, and WezTerm `libegl1`, which their
packages don't pull in. Ghostty runs on macOS with Accessibility for `osascript`, which GitHub's
macOS runners grant. The Linux console needs `sudo` without a password, for `openvt` and the
`/dev/vcs*` devices. `.github/workflows/terminals.yml` does all of this. Screenshots are taken
with ImageMagick's `import` where it's installed, and `screencapture` on macOS. They're for a person
to look at, and never decide a result.

The harness's own tests run tmux and screen where they're installed, and check the drivers'
parsers on recorded readbacks everywhere:

```sh
cargo test -p harness
```
