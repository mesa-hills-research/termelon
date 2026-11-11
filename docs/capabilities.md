# Capabilities

`Terminal::open` works out what the terminal supports before the program draws, and
`terminal.capabilities()` gives it as a `Capabilities`. The library uses it to choose what it sends,
and a program reads it to adapt what it shows, such as offering another chord where Shift+Enter
can't be told from Enter, or ASCII borders where the font has no box drawing.

## What it holds

Most fields are a `Support` of `Yes`, `No` or `Unknown`.

| Field | Says |
|---|---|
| `probe` | How the startup probe went, `Complete`, `TimedOut`, or `NotRun` when the program gave the capabilities |
| `terminal` | The terminal's name and version, such as `kitty` and `0.32.2` |
| `multiplexer` | tmux or GNU screen, with its version, when the program runs inside one |
| `colors` | `TrueColor`, `Ansi256`, `Ansi16` or `None` |
| `width_profile` | How the terminal measures emoji sequences, emoji with VS16 and East Asian ambiguous characters |
| `keyboard` | The kitty keyboard protocol with its flags, xterm's modifyOtherKeys, or the legacy encodings |
| `alternate_screen` | The alternate screen, which the Linux console lacks |
| `synchronized_output` | Synchronized updates (mode 2026) |
| `grapheme_clustering` | Grapheme clustering (mode 2027) |
| `hyperlinks` | OSC 8 hyperlinks |
| `underline_styles`, `underline_color` | Curly, dotted and other underlines (`SGR 4:n`), and their colour (`SGR 58`) |
| `focus_events`, `bracketed_paste`, `mouse` | Focus reports (1004), bracketed paste (2004), SGR mouse reports (1006) |
| `clipboard_write` | Copying with OSC 52 |
| `cursor_shape` | Cursor shapes (`CSI n SP q`) |
| `title`, `title_stack` | Setting the window title, and saving it on xterm's title stack |
| `background` | The background colour, for choosing a dark or a light theme |
| `glyphs` | `Full`, or `Limited` where the font may lack box drawing and braille |

## How it's found

Three sources fill it in, in this order, and a terminal's reply always beats a variable, since only
the program that answers can say what it is. What no source says stays `Unknown`.

### The startup probe

At open, one batch of queries asks for the kitty keyboard flags (`CSI ? u`), the support of modes
2026, 2027, 1004, 2004 and 1006 (DECRQM), the terminal's name and version (XTVERSION and DA2), the
background colour (OSC 11) and true colour (XTGETTCAP), and ends with DA1. Terminals answer in order
and every one answers DA1, so a query whose answer hasn't come when DA1's does is unsupported, and
the probe takes one round trip. A terminal that doesn't answer is waited for 500 ms, or 1.5 s over
ssh, which `Options::probe_timeout` changes. The Linux console gets DA1 alone, since it prints what
follows an `ESC ]` it doesn't know. Replies that come late are dropped, never taken as keys.

### The environment

`TERM`, `COLORTERM` (`truecolor` or `24bit`), `NO_COLOR`, `TMUX` and `STY`, and the variables
terminals set for themselves (`KITTY_WINDOW_ID`, `ALACRITTY_WINDOW_ID`, `WEZTERM_EXECUTABLE`,
`WT_SESSION`, `KONSOLE_VERSION`, `VTE_VERSION`, `XTERM_VERSION`), then `TERM_PROGRAM` and its
version. `COLORTERM` and `TERM` only ever raise the colour depth, and `NO_COLOR` takes colour away
whatever else says.

### The table of known terminals

For what a terminal can't be asked, a row for its name and version says what it supports. The table
has xterm, VTE (GNOME Terminal and the others built on it), Konsole, kitty, Alacritty, WezTerm,
foot, Windows Terminal, Terminal.app, iTerm2, tmux, GNU screen and the Linux console. Inside a
multiplexer the multiplexer's row applies, since it's what parses the program's output.

### Widths

Where grapheme clustering is supported, the library turns mode 2027 on, and the width profile then
joins emoji sequences.

`Options::width_probes` adds a second round trip that measures the width profile itself. It writes
an emoji ZWJ sequence, a heart with VS16 and an ambiguous character, each followed by a cursor
report, inside a synchronized update that clears them again.

## Setting them by hand

`Options::capabilities` skips the probe and uses what the program gives, for tests and for setups
the probe gets wrong.

`TUIFRAME_CAPS` sets fields over whatever was found, as `field=value` pairs separated by commas. A
field or value it doesn't know is skipped.

```sh
TUIFRAME_CAPS="colors=256,hyperlinks=no,keyboard=kitty:1,terminal=kitty 0.32.2" my-program
```

| Field | Values |
|---|---|
| Each `Support` field, such as `hyperlinks` or `synchronized_output` | `yes`, `no`, `unknown` |
| `colors` | `truecolor`, `256`, `16`, `none` |
| `keyboard` | `legacy`, `modify-other-keys`, `kitty`, `kitty:<flags>` |
| `terminal` | a name, then a space and a version if known |
| `multiplexer` | `tmux` or `screen`, then a version if known, or `none` |
| `joins_emoji`, `vs16_widens`, `ambiguous_wide` | `yes`, `no` |
| `background` | `#rrggbb`, `none` |
| `glyphs` | `full`, `limited` |

## Fallbacks

Two rules decide what's sent where support is unknown. Private modes (`CSI ? n h`) are sent, since
every terminal ignores a mode it doesn't know. Strings (OSC and DCS) are sent only where support is
known, since a terminal that doesn't parse them may print them.

| Missing | What happens |
|---|---|
| True colour | RGB becomes the nearest of xterm's 256 colours, then the nearest of the 16 named ones, by a perceptual distance |
| Colour (`NO_COLOR`) | No colour goes out. Bold, reverse and the other attributes still do |
| Underline styles | A single underline |
| Underline colour | The text's colour |
| Hyperlinks | The text without the link. The program can show the address itself |
| The window title, the title stack, OSC 52 | Nothing is sent |
| The kitty keyboard protocol | xterm's modifyOtherKeys where the terminal is known to take it, the legacy encodings elsewhere, where Shift+Enter arrives as Enter |
| Synchronized output | Each frame is still one write |
| The alternate screen | The screen is cleared at open and at close, so nothing is left behind |
| Focus reports, bracketed paste, mouse reports | They aren't turned on. A paste then arrives as keys |
| Cursor shapes | The terminal's own cursor. No shape is sent until a frame asks for one |
| Box drawing and braille | `glyphs` is `Limited`, and the program chooses `BorderSet::ASCII` and a spinner the font has |

## In tests

`testing::TestTerminal` probes the emulator's terminal the same way, so a test drawing for kitty
gets kitty's capabilities. `with_caps` gives others. [testing.md](testing.md) has the details.
