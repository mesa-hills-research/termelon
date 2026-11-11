# How it works

tuiframe owns the terminal from the moment a program opens it until it gives it back. The program
draws frames into a cell buffer and reads events, and two threads of the library's own do the
reading and writing.

## Crates

| Crate | Holds |
|---|---|
| `crates/tuiframe` | The library |
| `crates/tuiframe-vt` | A terminal emulator in a library, which tests use to see what a terminal would show. It doesn't depend on `tuiframe`, so it checks the library's output independently |
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
  returns everything already queued, and `next` is an `async fn` that works on any executor. It
  keeps the waker of every pending poll, so no event waits for something else to wake the task.
- The reader keeps the terminal's size in an atomic, so `draw` notices a resize without a system
  call per frame.
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
writes the cells that aren't blank. Nothing ever asks the terminal where its cursor is.

`Terminal::draw_regions(&[Rect], ...)` is the partial frame. The closure draws into the front buffer
with every write clipped to the regions, and only those regions are diffed and written, so an
animation costs the size of what moves.

`testing::TestTerminal` takes the same steps with the same buffers and encoder, and feeds the bytes
to `tuiframe-vt` instead of a writer, so a test sees what a terminal would show.

## Cells and widths

A `Cell` is 16 bytes and `Copy`. It holds one character's UTF-8 inline, or a tag and an index into
the buffer's cluster table for a longer grapheme cluster, so the encoder copies most symbols
straight to the output. Rare data (underline colour, hyperlinks) sits in a per-buffer extras table.
The trailing cells of a wide character carry a continuation tag, and the buffer keeps one invariant,
which is that a wide character always has all its continuation cells and none is ever without its
leader.

A cluster's width is decided once, when it's written, by the buffer's `WidthProfile`. Programs
measure with the same one, since `Terminal::open` makes the terminal's the process's profile
(`text::set_width_profile`), which `text::width`, the text types' `width` and `Buffer::empty`
read. Tests set one for their own thread with `text::with_width_profile`. Cells whose
width terminals disagree on (emoji sequences, and emoji newer than many terminals' width tables) are
marked. The encoder writes spaces over such a cell's cells first, then the cluster, then again the
cells that the widest measure of it would cover, placing each absolutely, so a terminal that
measures differently shows blanks in that cluster's cells and nothing wrong around it. One that
the widest measure wouldn't fit before the right edge stays blank, since a terminal pushes a
cluster too wide for the room left over the cells before it. East Asian ambiguous characters follow the profile
without a mark. They include box drawing and the Greek and Cyrillic alphabets, which every measured
terminal draws narrow unless it's set for East Asian text, and a resync after each would make them
several times longer to write.

## Output

The diff compares rows as slices, finds the changed runs, treats a wide character as one unit, and
merges two runs when resending the cells between them is cheaper than moving the cursor. The encoder
writes the runs with the shortest cursor moves, the fewest SGR parameters from the terminal's current
pen, and colours lowered to the terminal's depth. It never writes an empty SGR, which would reset
everything. Autowrap is off while the program has the screen, so the last column never scrolls.

A frame's packet is one write.

```
CSI ? 2026 h                      begin the synchronized update
CSI ? 25 l                        hide the cursor, if it was visible
<runs: moves, SGR changes, OSC 8, text>
SGR 0, and OSC 8 ; ; ST if a link is open
CSI row ; col H                   the cursor's place
CSI n SP q                        its shape, if it changed
CSI ? 25 h                        show it, if the frame has one
CSI ? 2026 l                      end the synchronized update
```

A frame that changes no cell writes only a cursor move when the cursor moved, and nothing otherwise.

Closing gives the terminal back in every way a program can end. The reset packet is built at open
from what was enabled. It ends any synchronized update, pops the kitty keyboard flags, turns off the
reports that were on, resets the pen, turns autowrap back on, resets the scroll region, leaves the
alternate screen, shows the cursor and restores the title. When the writer is stuck mid-write, `RestoreHandle::restore_now`
writes CAN, ST and the reset from a thread of its own, once, and the writer stops within the 4 KiB
chunk it was writing.

## Input

A tokenizer in the shape of the VT500 parser turns bytes into tokens (text, controls, escape
sequences, strings, pastes) within fixed bounds, and the decoders turn tokens into events. A lone ESC
at the end of what's available is the Esc key at once, unless `Options::escape_timeout` says to wait
for a following byte. On Windows the reader reads console input records, which the decoder turns
into the same events. Replies to the library's queries go to the probe's reply slot, never to the
event queue, and replies nobody waits for are dropped.

## Capabilities

At open, before any event reaches the program, the probe writes one batch of queries ending with DA1
(`CSI c`). Every terminal tested answers DA1, and terminals answer in order, so a query whose answer
hasn't come by the time DA1's does is unsupported, and the probe takes one round trip. A terminal
that doesn't answer at all is waited for up to `Options::probe_timeout`. Environment variables
and a table of known terminals and versions fill in what can't be asked, and a reply always beats a
variable. `TUIFRAME_CAPS` overrides any field.

Where support is unknown, private modes are sent, since terminals ignore modes they don't know, and
strings (OSC and DCS) aren't, since a terminal that doesn't parse them may print them.

## Internal interfaces

These are the crate-internal contracts the parts of the library meet at. Each part is built and
tested against them on its own.

### The decoder

In `event::decode`, a pure state machine over bytes and Windows records, with no I/O. The reader
thread owns one.

```rust
pub(crate) struct DecoderOptions {
    pub(crate) erase: u8,        // termios VERASE, 0x7f or 0x08
    pub(crate) max_paste: usize, // 64 MiB by default
    pub(crate) all_keys: bool,   // Windows keys alone too, as kitty's flag 8 reports them
}

pub(crate) struct Decoder { /* the tokenizer's state and the outstanding queries */ }

impl Decoder {
    pub(crate) fn new(options: DecoderOptions) -> Self;
    pub(crate) fn feed(&mut self, bytes: &[u8], out: &mut Vec<Decoded>);
    pub(crate) fn has_pending(&self) -> bool;
    pub(crate) fn flush_pending(&mut self, out: &mut Vec<Decoded>);
    pub(crate) fn has_held(&self) -> bool;
    pub(crate) fn flush_held(&mut self, out: &mut Vec<Decoded>);
    pub(crate) fn expect(&mut self, query: Query);
    pub(crate) fn cancel_expected(&mut self);
    pub(crate) fn decode_records(&mut self, records: &[InputRecord], out: &mut Vec<Decoded>);
}
```

- `new` takes the byte the terminal sends for Backspace. `0x7f` is always Backspace, and `0x08` is
  Backspace where `VERASE` says so and Ctrl+H otherwise, as crossterm has it. Ctrl+Backspace is told
  apart only under the kitty keyboard protocol and on Windows.
- `feed` decodes what it can and keeps an incomplete sequence for the next call, so a sequence split
  across reads decodes as if it came whole.
- `has_pending` says whether what it holds ends in an ESC or a sequence's introducer (`ESC [`,
  `ESC O`, `ESC ]`, `ESC P`, `ESC _`, `ESC [ M` and its bytes), which could be keys typed with Alt or
  the start of a sequence. It tells the reader whether to wait for the escape timeout.
- `flush_pending` decodes those as keys, as if no more input will follow, so a lone ESC becomes Esc
  and `ESC [` becomes Alt+[, and afterwards nothing is pending. A sequence with more of it
  (`ESC [ 1 ;`), a string, a paste and part of a UTF-8 character stay held, since only a split read
  leaves them unfinished, and they don't count as pending. The reader calls it when no more input is
  ready.
- `has_held` says whether it holds anything at all, and `flush_held` gives up on it. What
  `flush_pending` takes becomes keys, a paste is delivered as it stands, and a sequence, a string or
  part of a character is dropped. The reader calls it once the input has been silent for about a
  second with something held, so a reply cut short or a stray `ESC ]` never swallows what's typed
  next.
- A paste longer than `max_paste` arrives as several `Event::Paste`s of about that size, in order,
  each of whole characters, so a paste never holds more memory than the bound.
- `expect` records one query whose answer is outstanding. Each matching reply uses one up, and a DA1
  reply clears them all, since terminals answer in order. Only the cursor report needs this, because
  `CSI r ; c R` is also a modified F3 in the legacy encoding. `cancel_expected` forgets them all,
  for a probe that stopped waiting.
- `decode_records` decodes Windows console records into the same events as `feed` does for bytes.

```rust
pub(crate) enum Decoded {
    Event(Event),
    Reply(Reply),
}

pub(crate) enum Query {
    CursorPosition, // CSI 6 n
}

pub(crate) enum Reply {
    Da1 { attributes: Vec<u16> },                              // CSI ? a ; b ... c
    Da2 { kind: u16, version: u32 },                           // CSI > kind ; version ; ... c
    ModeReport { mode: u16, private: bool, value: ModeValue }, // DECRPM, CSI ? mode ; value $ y
    CursorPosition { row: u16, column: u16 },                  // CSI row ; column R, while expected
    KittyKeyboard(u8),                                         // CSI ? flags u
    XtVersion(String),                                         // DCS > | text ST
    XtGetTcap { name: String, value: Option<String> },         // DCS 1 + r name = value ST
    Color { which: ColorSlot, rgb: Rgb },                      // OSC 10 or 11 ; rgb:r/g/b
    TerminalSize { rows: u16, columns: u16 },                  // CSI 8 ; rows ; columns t
}

pub(crate) enum ModeValue { NotRecognized, Set, Reset, PermanentlySet, PermanentlyReset }

pub(crate) enum ColorSlot { Foreground, Background } // OSC 10, OSC 11
```

`CursorPosition` is 0-based, as the library's positions are. `XtGetTcap` holds the name and value
decoded from hex, with no value when the terminal doesn't know the name. `Color` scales each channel
to 8 bits, whatever number of hex digits the terminal sent. The theme reports of mode 2031
(`CSI ? 997 ; 1 n` and `; 2 n`) come whether or not a query was sent, so they decode as
`Event::ColorScheme`.

### Windows input records

In `event::records`, compiled on every system so the decoder's tests run everywhere. The unsafe code
in `sys/windows/console.rs` only copies the fields of each `INPUT_RECORD` into these, and the reader
drops menu records.

```rust
pub(crate) enum InputRecord {
    Key(KeyRecord),
    Mouse(MouseRecord),
    WindowSize(WindowSizeRecord),
    Focus(FocusRecord),
}

pub(crate) struct KeyRecord {
    pub(crate) key_down: bool,
    pub(crate) repeat: u16,
    pub(crate) virtual_key: u16,
    pub(crate) scan_code: u16,
    pub(crate) unicode: u16, // one UTF-16 unit, so a surrogate pair takes two records
    pub(crate) control_state: u32,
}

pub(crate) struct MouseRecord {
    pub(crate) column: u16, // relative to the window, which the reader works out
    pub(crate) row: u16,
    pub(crate) button_state: u32,
    pub(crate) control_state: u32,
    pub(crate) event_flags: u32,
}

pub(crate) struct WindowSizeRecord {
    pub(crate) size: Size, // the window's size, which the reader reads, not the record's buffer size
}

pub(crate) struct FocusRecord {
    pub(crate) focused: bool,
}
```

A key record with a repeat count of n gives n events, AltGr arrives as Ctrl+Alt with a printable
character and decodes as that character, and a `WindowSizeRecord` gives `Event::Resize`. A modifier
or lock key pressed or released alone, and the keypad's middle key, are dropped as crossterm drops
them, and with `all_keys` they give their key codes. An Alt code (Alt held while digits are
typed on the keypad) gives its character when Alt comes up.

### The encoder

In `output::encode`. The diff and the encoder work on two buffers of the same area.

```rust
pub(crate) struct Encoder { /* the terminal's cursor place, shape and visibility, and link ids */ }

impl Encoder {
    pub(crate) fn new() -> Self;
    pub(crate) fn encode(
        &mut self,
        prev: &Buffer,
        next: &Buffer,
        cursor: Option<&Cursor>,
        caps: &Capabilities,
        out: &mut Vec<u8>,
    ) -> FrameStats;
    pub(crate) fn encode_regions(
        &mut self,
        before: &Buffer,
        after: &Buffer,
        regions: &[Rect],
        cursor: Option<&Cursor>,
        caps: &Capabilities,
        out: &mut Vec<u8>,
    ) -> FrameStats;
    pub(crate) fn redraw(&mut self);
    pub(crate) fn set_scroll_detection(&mut self, on: bool);
}

pub struct FrameStats {
    pub cells: usize, // cells written
    pub bytes: usize, // bytes appended to out
}
```

- `encode` appends to `out` the packet that takes the terminal from `prev` to `next` and puts the
  cursor where `cursor` says, or hides it when there's none. It appends nothing when nothing changed,
  and the caller then sends no packet. The encoder keeps what the terminal shows besides the cells
  (where its cursor is, its shape and visibility, and the ids of links it has seen), which two
  buffers alone can't say.
- `encode_regions` is the partial frame. It compares `before` and `after` inside `regions`, and
  `before` may hold anything outside them. `after` was drawn under a clip of the regions
  (`Buffer::set_clip`), which drops writes to cells outside them but writes a wide character that
  starts inside whole, and breaks one that starts outside when a write covers part of it. The
  buffer records the cells outside that this changed, and `encode_regions` writes them too, with
  any wide character, in either buffer, that crosses a region's edge.
- `redraw` makes the next `encode` a full redraw, which clears the screen, ignores `prev`, writes
  every cell that isn't blank and sends the cursor's state and the pen again.
- `set_scroll_detection` turns on looking for rows that moved when more than half of a frame's
  rows changed. When scrolling a region of rows saves bytes, the terminal scrolls them itself
  (`DECSTBM`, then delete or insert lines with `CSI n M` or `CSI n L` at the region's top, which
  every VT102 descendant has, the Linux console too) and only the rest is written. It's off until the terminal
  matrix passes it.

`Cursor` is the frame's cursor request (`position`, `shape`, `blinking`), and `FrameStats` reaches the
program in `CompletedFrame`.

### The writer

In `output::writer`. One thread owns the output handle (`/dev/tty` or standard output on Unix, the
console's output on Windows), and packets queue behind one mutex that is never held across a write.
Clones of a `Writer` share the queue and the thread, which ends when the last clone is dropped.

```rust
pub(crate) struct Writer { /* the queue, shared with the thread */ }

impl Writer {
    pub(crate) fn spawn(out: impl Write + Send + 'static, reset: Vec<u8>) -> io::Result<Self>;
    pub(crate) fn is_idle(&self) -> bool;
    pub(crate) fn send(&self, packet: Vec<u8>) -> Result<(), Failure>;
    pub(crate) fn spare_buffer(&self) -> Vec<u8>;
    pub(crate) fn written(&self) -> Written<'_>; // a Future<Output = ()>
    pub(crate) fn busy_since(&self) -> Option<Instant>;
    pub(crate) fn set_reset(&self, reset: Vec<u8>);
    pub(crate) fn ticket(&self) -> Ticket;
    pub(crate) fn close(&self, tail: &[u8]) -> (Ticket, bool);
    pub(crate) fn wait(&self, ticket: Ticket, timeout: Duration) -> Finish; // Written, Failed, TimedOut
    pub(crate) fn takeover(&self) -> Takeover;
    pub(crate) fn failure(&self) -> Option<Failure>; // Gone(io::ErrorKind) or Died
}
```

- `spawn` takes the output handle and the reset packet, and `set_reset` replaces the reset once the
  probe has decided what to turn on.
- `is_idle` is the check behind `Drawn::Held`, and `written` completes when it turns true. It wakes
  every task waiting on it, on any executor.
- `send` queues a packet. Frames, out-of-band commands and probe queries all go through it, in order.
  It fails once the writer failed, and drops the packet after `close`.
- `spare_buffer` returns a written packet's buffer, emptied, so a frame reuses it rather than
  allocating.
- `busy_since` is when the write in progress began, for a program's watchdog.
- `ticket` stands for everything sent so far, and `wait` says what became of it within the timeout.
- `close` drops the packets not yet started, then queues the reset and the program's `tail`, and says
  whether this call ended the output. Its ticket counts as written only once the reset is.
- `takeover` gives what writes CAN, ST and the reset from a thread of its own when the writer is
  stuck or died, with a record outside the writer's lock, so the reset is never written twice.
  `RestoreHandle` (`Clone + Send + Sync`) keeps one, and its `restore_now(timeout)` uses it.
- `failure` is set once a write fails or the thread ends. The terminal ends the event stream then.

### The reader

In `sys::unix`, `sys::macos` and `sys::windows`, behind one interface. It shares its results with
the program's thread through `Shared`.

```rust
pub(crate) struct Shared {
    pub(crate) events: EventQueue, // Mutex<VecDeque<Event>>, a Condvar, the pending Wakers, closed
    pub(crate) replies: ReplySlot, // Mutex and Condvar, open only while the probe waits
    size: AtomicU32,               // width << 16 | height, through size() and set_size()
    redraw: AtomicBool,            // through request_redraw() and take_redraw()
}

pub(crate) struct Reader { /* the thread's handle and its wake pipe or event */ }

impl Reader {
    pub(crate) fn spawn(
        input: OwnedFd,
        decoder: impl Decode, // feed, has_pending, flush_pending and expect, as Decoder has
        shared: Arc<Shared>,
        escape_timeout: Duration,
        signals: Signals,     // pipes for SIGWINCH, SIGCONT, SIGTSTP and the ending signals
        hooks: Hooks,         // what to do on SIGCONT, SIGTSTP and an ending signal
    ) -> io::Result<Self>;
    pub(crate) fn expect(&self, query: Query);
    pub(crate) fn cancel_expected(&self);
    pub(crate) fn set_escape_timeout(&self, timeout: Duration);
    pub(crate) fn park(&self) -> bool;
    pub(crate) fn unpark(&self);
    pub(crate) fn wait_for_stop(&self, since: u64, start: Duration) -> bool;
    pub(crate) fn stop(self);
}
```

On Unix, `restore::screen::Screen` puts these together. `Screen::open_with` takes the terminal
(raw mode, the writer, the reader) and `enable` sends the setup the probe's answers decided on. It
also closes the terminal, suspends it, and lends it to another program with `release` and
`reacquire`. A writer that fails ends the events.

Each turn of the reader's loop does these steps.

1. Wait on the input, the signal pipe and the wake pipe, with no timeout, or with the escape timeout
   while the decoder has a sequence pending.
2. Read what's ready (up to 4 KiB, or up to 64 records on Windows), hand the decoder any queries
   `expect` queued since the last turn, then decode.
3. Queue the events, notifying the condvar and waking every stored waker. Put replies in the reply
   slot when it's open, and drop them otherwise.
4. When no more input is ready, call `flush_pending` once the escape timeout has passed, and
   `flush_held` once the input has been silent for a second with something held.
5. On SIGWINCH or a window size record, store the new size, ask for a full redraw and queue
   `Event::Resize`. On SIGTSTP, call the `stop` hook, which gives the terminal back, stops the
   process with SIGSTOP and takes the terminal back once it runs again, then queue
   `Event::Resumed`. On a SIGCONT that came without it, call `resume`, which sends the reset and
   the setup again and applies raw mode, and queue `Event::Resumed`. Either asks for a full
   redraw. On an ending signal, call `end`, which gives the terminal back and lets the signal end
   the program.
6. When the input closes or fails, or the thread ends, mark the queue closed, so `Events` ends.

`expect` queues the query for the reader's next turn, so calling it before the query is written is
enough. `park` and `unpark` hold the reader still while another program has the terminal.
`wait_for_stop` is how a suspend, which sends SIGTSTP to the process group, waits for the reader to
handle it. `stop` ends the thread at close.

### The console on Windows

In `sys::windows`. The calls into the console API are in `console.rs`, the only unsafe code on
Windows. The rest works through two traits, `Console` (its modes, its window, a handle for the
writer, the reader's end) and `Input` (wait, read, whether more is ready), so it's tested on every
system against a console played in memory.

```rust
pub(crate) type Screen = ScreenOn<WinConsole>; // CONIN$ and CONOUT$, opened with OpenOptions

impl<C: Console> ScreenOn<C> {
    pub(crate) fn open<D: Decode>(console: C, decoder: impl FnOnce(u8) -> D, escape_timeout: Duration)
        -> io::Result<Self>;                 // Unsupported without virtual terminal processing
    pub(crate) fn enable(&self, enabled: &Enabled) -> io::Result<()>;
    pub(crate) fn set_enabled(&self, enabled: &Enabled);
    pub(crate) fn set_input(&self, mode: InputMode) -> io::Result<()>; // Records or Vt
    pub(crate) fn restore_on_console_events(&self) -> io::Result<()>;
    // writer, shared, window_size, restore_handle, draw_here, expect, cancel_expected,
    // set_escape_timeout, suspend (Unsupported) and close, as on Unix
}
```

- `open` saves the console's input and output modes and sets them for the program. Input has no line
  editing, echo or processed input, so keys arrive one at a time and Ctrl+C is a key. Output reads
  escape sequences, without the line feed that also returns the cursor. Closing puts back exactly
  what was found. The code pages never change, since the writer writes UTF-16 with `WriteConsoleW`,
  converting each packet from UTF-8.
- `set_enabled` takes mouse records, with the console's quick edit off, when the mouse is on, and
  leaves quick edit as found when it's off, so the user can still select text. Focus records become
  events only when focus reports are on, since the console sends them regardless.
- The reader waits on the console's input and a wake event with `WaitForMultipleObjects`, and reads
  up to 64 records with `ReadConsoleInputW`. Mouse coordinates are made the window's, and a window
  size record reads the window's size, not the buffer's.
- In `InputMode::Records` keys decode as key records. In `InputMode::Vt` each key-down record's
  character is a byte of what the terminal sent, so replies, the kitty protocol and bracketed paste
  decode as on Unix. `Terminal` probes in `Vt`, where the console's answers decode as replies, and
  then switches to `Records`. A probe left unanswered keeps `Vt` 200 ms longer first.
- In `Records`, records that carry what a terminal sent rather than a key typed go to the byte
  decoder for the whole session. Those are records with no virtual key, an ESC key-down that more
  character records follow in the same read, and what comes while the byte decoder holds a sequence.
  A late reply is then dropped rather than typed, and keys typed during the probe decode as the keys
  they were.
- The kitty keyboard flags are pushed only in `Vt`, since under a pseudoconsole the push reaches the
  terminal, which would then send keys a records client can't take. `caps.keyboard` is
  `ConsoleRecords` on Windows.
- `restore_on_console_events` installs a handler with `SetConsoleCtrlHandler`, once, so the console
  window closing, Ctrl+Break, logoff and shutdown give the console back before Windows ends the
  process.
