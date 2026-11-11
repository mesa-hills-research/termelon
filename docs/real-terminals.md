# Testing in real terminals

`tools/harness` starts a real terminal running `tools/probe`, waits until the probe's scene shows,
reads the screen back, and compares it cell by cell with the screen `tuiframe-vt` computes for the
bytes the probe wrote. Text and colours always count. Attributes and the cursor count where the
terminal's readback reports them.

```sh
cargo build -p harness -p probe
target/debug/harness smoke tmux
target/debug/harness run tmux --probe target/debug/probe --out harness-out
```

`smoke` starts the terminal with `echo` and reads it back, which tells a broken setup from a
failure of the program under test. `run` runs every scene the probe lists (or those named with
`--scene`) at 80 by 24 (or `--size`). For each scene it writes the bytes the probe wrote, a JSON
report with each difference, both screens in the styled-text format, an HTML page with the two side
by side, and a screenshot where there's a display. It exits with 1 when a screen differs.

| Terminal | Needs | Read back with | Reports |
|---|---|---|---|
| tmux | tmux | `capture-pane -p -e -N` | text, colours, attributes, cursor |
| GNU screen | screen and tmux | `hardcopy` | Latin-1 text only |
| kitty | kitty, an X display, xdotool | `kitten @ get-text --ansi --add-cursor` | text, colours, attributes, cursor |
| xterm | xterm, the `9x15` font, an X display, xdotool | the `CSI 10 i` XHTML dump | text, RGB colours, bold, italic, underline, strike |
| WezTerm | wezterm, an X display, xdotool | `wezterm cli get-text --escapes` | text, colours, attributes, cursor |

Under X, start Xvfb first and set `DISPLAY`, as `.github/workflows/terminals.yml` does. Screenshots
are taken with ImageMagick's `import` where it's installed. They're for a person to look at, and
never decide a result.

The harness's own tests run tmux and screen where they're installed, and check the drivers'
parsers on saved readbacks everywhere:

```sh
cargo test -p harness
```
