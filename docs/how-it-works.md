# How it works

Termelon owns the terminal from the moment a program opens it until it gives it back. The program
draws frames into a cell buffer and reads events, and two threads of the library's own do the
reading and writing.

## Crates

| Crate | Holds |
|---|---|
| `crates/termelon` | The library |
| `crates/termelon-vt` | A terminal emulator library, which tests use to see what a terminal would show. It doesn't depend on `termelon`, so it checks the library's output independently |
| `tools/probe` | A program run inside real terminals, which measures them and draws test scenes |
| `tools/harness` | Starts real terminals with the probe, drives them and reads back what they show |
| `tools/bench` | Workloads that compare the output layer with ratatui and crossterm |

The tools aren't published, so ratatui and crossterm never enter a user's dependency tree.

## Modules

| Module | Holds |
|---|---|
| `terminal` | `Terminal`, `Options`, `draw`, out-of-band commands, close and restore |
| `frame` | `Frame`, the cursor request, partial frames |
| `buffer` | `Cell`, `Buffer`, the cluster table and the extras table |
| `style` | `Color`, `Modifier`, `UnderlineStyle`, `Style` |
| `text` | `Span`, `Line`, `Text`, grapheme iteration, widths, wrapping |
| `layout` | `Position`, `Size`, `Rect`, `Layout`, `Constraint` |
| `widgets` | `Widget`, `Block`, `BorderSet`, `Clear`, `Paragraph` |
| `output` | The diff, the encoder, the writer thread |
| `event` | `Event` and its types, the tokenizer, the decoders, `Events` |
| `caps` | `Capabilities`, the startup probe, environment rules, the table of known terminals |
| `sys` | The private platform layer, in `unix`, `macos` and `windows` |
| `testing` | `TestTerminal`, screen text, styled text, goldens |

## Threads

The library never owns the program's loop. `Terminal::open` starts two threads and hands the
program a `Terminal` to draw with and an `Events` to read from.

```
 program thread                         reader thread                      writer thread
 Terminal::draw ── diff, encode ──► packet ──────────────────────────────► write_all(tty)
 Events::next ◄── queue ◄── decode ◄── read ◄── wait(tty, signal pipe, wake pipe)
 the probe ── queries ──► writer         replies ──► reply slot
```

- The reader thread waits on the terminal's input, a signal pipe and a wake pipe, with `poll` on
  Linux and the BSDs, `select` on macOS (whose `poll` doesn't work on `/dev/tty`) and
  `WaitForMultipleObjects` on Windows. It decodes what it reads and queues the events.
- `Events` is the program's end of the queue. `next_timeout` blocks, `try_next` doesn't, `batch`
  returns everything already queued, and `next` returns a future that works on any executor. It
  keeps the waker of every pending poll, so no event waits for something else to wake the task.
- The reader keeps the terminal's size in an atomic, which `Terminal::size` reads without a system
  call. `draw` asks the system for the size once a frame as well, since SIGWINCH reaches only the
  terminal's foreground process group.
- `draw` renders, diffs and encodes on the program's thread, which is CPU work only, then hands the
  bytes to the writer. Nothing on the program's thread can block on the terminal.
- The writer thread writes packets in order. A terminal that stops reading holds up that thread
  alone.

## Frames

`Terminal::draw(|frame| ...)` returns `Drawn::Held` without calling the closure while the writer is
still busy with an earlier frame, and `Terminal::written()` completes when it's idle again. The frame
drawn then carries every change since the last one the terminal took, so a slow terminal gets one
current frame rather than a backlog. When the writer is idle, `draw` resets the back buffer, runs the
closure, diffs the result against the front buffer (what the terminal shows once every packet so far
is written), encodes the changes into one packet, swaps the buffers and returns
`Drawn::Written`.

A resize, a resume and `Terminal::clear()` make the next frame a full redraw. The flag survives
`Held` and clears only when a full frame reaches the writer. A full redraw clears with `CSI 2 J` and
writes the cells that aren't blank. Where the capabilities say the terminal erases with the pen's
background (`bce`), it clears in the background most spaces have, and writes only what differs
from that. Nothing ever asks the terminal where its cursor is.

`Terminal::draw_regions(&[Rect], ...)` is the partial frame. The closure draws into the front buffer
with every write clipped to the regions, and only those regions are diffed and written, so an
animation costs the size of what moves.

`testing::TestTerminal` takes the same steps with the same buffers and encoder, and feeds the bytes
to `termelon-vt` instead of a writer, so a test sees what a terminal would show.

## Cells and widths

A `Cell` is 16 bytes and `Copy`. It holds one character's UTF-8 inline, or a tag and an index into
the buffer's cluster table for a longer grapheme cluster, so the encoder copies most symbols
straight to the output. Rare data (underline colour, hyperlinks) sits in a per-buffer extras table.
The trailing cells of a wide character carry a continuation tag, and the buffer keeps one invariant,
which is that a wide character always has all its continuation cells and none is ever without its
leader.

A cluster's width is decided once, when it's written, by the buffer's `WidthProfile`. Programs
measure with the same one, since `Terminal::open` makes the terminal's the process's profile, which
`text::width_profile` returns and `text::width`, the text types' `width` and `Buffer::empty` use.
Tests set one for their own thread with `text::with_width_profile`. Cells whose width terminals
disagree on (emoji sequences, and emoji newer than many terminals' width tables) are marked. The
encoder writes spaces over such a cell's cells first, then the cluster, then again the cells that
the widest measure of it would cover, placing each absolutely, so a terminal that measures
differently shows blanks in that cluster's cells and nothing wrong around it. One that the widest
measure wouldn't fit before the right edge stays blank, since a terminal pushes a cluster too wide
for the room left over the cells before it. East Asian ambiguous characters follow the profile
without a mark. They include box drawing and the Greek and Cyrillic alphabets, which every measured
terminal draws narrow unless it's set for East Asian text, and a resync after each would make them
several times longer to write.

## Output

The diff compares rows as slices, finds the changed runs, treats a wide character as one unit, and
merges two runs when resending the cells between them is cheaper than moving the cursor. The encoder
writes the runs with the shortest cursor moves, the fewest SGR parameters from the terminal's current
pen, and colours lowered to the terminal's depth. A row that ends in spaces of the default
background ends with `CSI K`, and so does one that ends in spaces of another background where the
terminal erases with the pen's background (`bce`). Elsewhere those are written as spaces, so
terminals with and without that erase agree. It never writes an empty SGR, which would reset
everything. Autowrap is off while the program has the screen, so the last column never scrolls.

A frame's packet is one write.

```
CSI ? 2026 h                      begin the synchronized update, unless the terminal lacks it
CSI ? 25 l                        hide the cursor, if it was visible
<runs: moves, SGR changes, OSC 8, text>
SGR 0, and OSC 8 ; ; ST if a link is open
CSI row ; col H                   the cursor's place
CSI n SP q                        its shape, if it changed
CSI ? 25 h                        show it, if the frame has one
CSI ? 2026 l                      end the synchronized update
```

A frame that changes no cell writes only a cursor move when the cursor moved, and nothing otherwise.

Closing gives the terminal back, and so does dropping the `Terminal`, a panic (through the panic
hook `Options::panic_hook` installs) and, where `Options::restore_on_signals` is on, a signal that
ends the program. The reset packet is built at open from what was enabled. It ends any synchronized
update, pops the kitty keyboard flags, turns off the reports that were on, resets the pen, turns
autowrap back on, resets the scroll region, leaves the alternate screen, shows the cursor and
restores the title. When the writer is stuck mid-write, `RestoreHandle::restore_now` writes CAN, ST
and the reset from a thread of its own, once, and the writer stops within the 4 KiB chunk it was
writing.

## Input

A tokenizer in the shape of the VT500 parser turns bytes into tokens (text, controls, escape
sequences, strings, pastes) within fixed bounds, and the decoders turn tokens into events. A lone
ESC at the end of what's available is the Esc key once `Options::escape_timeout` has passed without
a following byte, 20 ms by default and 100 ms under the kitty keyboard protocol. On Windows the
reader reads console input records, which the decoder turns into the same events. Replies to the
library's queries go to the probe's reply slot, never to the event queue, and replies nobody waits
for are dropped.

## Capabilities

At open, before any event reaches the program, the probe writes one batch of queries ending with DA1
(`CSI c`). Every terminal tested answers DA1, and terminals answer in order, so a query whose answer
hasn't come by the time DA1's does is unsupported, and the probe takes one round trip. On Windows
it waits up to 200 ms more, as [Windows consoles](#windows-consoles) explains. A terminal
that doesn't answer at all is waited for up to `Options::probe_timeout`. Environment variables
and a table of known terminals and versions fill in what can't be asked, and a reply always beats a
variable. `TERMELON_CAPS` overrides any field.

Where support is unknown, private modes are sent, since terminals ignore modes they don't know, and
strings (OSC and DCS) aren't, since a terminal that doesn't parse them may print them.

## Windows consoles

A program's console is conhost's window, or a pseudoconsole (ConPTY) with a terminal on its other
side, such as Windows Terminal. The pseudoconsole is the system's own conhost, or the
`OpenConsole.exe` a terminal ships. These facts come from the console tests on Windows Server 2022
(conhost 10.0.20348) and 2025 (10.0.26100), with Windows Terminal 1.23 and 1.25 and the
pseudoconsole of Windows Terminal 1.25.

A console puts each reply to a query into its input as key records, one for each UTF-16 unit, with
no virtual key, scan code or modifiers, the ESC too. Server 2022's conhost gives each character of
its own replies a press and a release, and the rest a press alone. The replies a console makes
itself arrive this way in records mode and in virtual terminal input mode alike. Who answers
differs.

| Query | conhost 2022 | conhost 2025 | system pseudoconsole 2022 | system pseudoconsole 2025 | Windows Terminal's pseudoconsole |
|---|---|---|---|---|---|
| DA1, DA2, cursor report | itself | itself | itself | itself | the terminal |
| DECRQM, `CSI 18 t` | nobody | itself | the terminal | itself | the terminal |
| kitty `CSI ? u` | nobody | nobody | the terminal | the terminal | the terminal |
| XTVERSION | nobody | nobody | the terminal, whose reply it drops | the terminal | the terminal |
| OSC 11 | nobody | nobody | the terminal, whose reply it drops | nobody | the terminal |

conhost answers DA1 with `CSI ? 1 ; 0 c` on Server 2022 and `CSI ? 61 ; 1 ; 6 ; 7 ; 21 ; 22 ; 23 ;
24 ; 28 ; 32 ; 42 c` on 2025, and DECRQM for 2026 with "not recognised". A pseudoconsole that answers
DA1 itself does so at once, so the terminal's answers to the queries it passes on come after DA1's.
That's why the probe waits for quiet after DA1 on Windows. Windows Terminal answers the kitty query
with flags 0 from 1.25, and 1.23 doesn't answer it. Neither answers XTVERSION.

In records mode, what a pseudoconsole makes of a terminal's bytes:

- Known key sequences become the key's records (Up, F5, Ctrl+C, Alt+x), and win32-input-mode's
  sequences the records they carry. Windows Terminal speaks win32-input-mode, so its keys arrive
  whole.
- Sequences the pseudoconsole doesn't know, a kitty `CSI 13 ; 2 u` or a late reply, pass as
  characters with no key through Windows Terminal's. The system's drops them, except that Server
  2025 turns the end of an XTVERSION reply into Alt+\.
- Under Windows Terminal, a cursor report arrives as F3, whose sequence `CSI 1 ; 1 R` also is.
- A paste loses its bracketed-paste markers and arrives as typed keys.
- A lone ESC arrives as Esc on Server 2022. Server 2025's pseudoconsole and Windows Terminal's hold
  it.
- A character its keyboard layout has no key for, such as an emoji, arrives from the system's
  pseudoconsole as an Alt code, Alt held while keypad digits go down, with the character on Alt's
  release, one per UTF-16 unit. It does so in virtual terminal input mode too. Windows Terminal's
  passes it as a character.
- SGR mouse reports become mouse records, and focus reports focus records, Server 2022's
  pseudoconsole dropping these.

In virtual terminal input mode the terminal's bytes pass as characters with no key, with three
exceptions. The system's pseudoconsoles drop OSC strings, and Server 2022's DCS strings too. Those
of Server 2025 and Windows Terminal drop focus reports. The system's sends Alt codes as above.
Windows Terminal's kitty keys and their releases under the protocol's flags weren't measured, so
records stay the input mode once the probe is done.

Key records written with `WriteConsoleInputW` come back from `ReadConsoleInputW` as written in
records mode. In virtual terminal input mode the console turns each key press into what the key
sends as characters, `ESC [ A` for Up and a carriage return for Shift+Enter, its modifiers lost.
conhost passes the releases as written, and Windows Terminal drops them. A conhost window and
Windows Terminal send focus records of their own as their windows gain focus.
`ReadConsoleOutputCharacterW` gives a wide character once for its two cells, and U+FFFD for one
outside the BMP.

Output passes through Windows Terminal's pseudoconsole byte for byte, synchronized output, RGB
colour, curly underlines, links and kitty pushes alike. Server 2025's passes it on with small
rewrites, `CSI 0 m` as `CSI m` and an `id` added to each link. Server 2022's draws its own copy of
the screen, so curly underlines are lost and a link's text comes apart from its sequence. Every
pseudoconsole asks its terminal for the cursor position as it starts, and those of Server 2025 and
Windows Terminal turn on focus reports and win32-input-mode.

The runner's own console on both images is conhost with no window. `conhost.exe PROGRAM` opens a
conhost window on Server 2025 too, and Windows Terminal's portable build runs on Server 2022.

Another process can drive a conhost window. Started with `CREATE_NEW_CONSOLE`, the console takes
key records from `WriteConsoleInputW` and gives its cells to `ReadConsoleOutputW`, in any process
that attaches to it with `AttachConsole`. `GetConsoleCursorInfo` says the cursor shows on the
alternate screen whatever the program set. Windows Terminal's `exportBuffer` action writes the text
of the screen it shows to a file, on a key that `SendInput` presses once the window is in front,
and `--size` gives its window exactly the cells asked for. Both work on both images, with Windows
Terminal 1.23 installed and 1.25 portable.

[Internals](internals.md) lists the private interfaces between these parts.
