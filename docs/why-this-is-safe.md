# Why this is safe

The library denies `unsafe_code` everywhere except two modules, `sys/macos.rs` and
`sys/windows/console.rs`, and the harness's Windows driver, `tools/harness/src/windows.rs`. Each
unsafe block holds one unsafe operation. Every block, `unsafe impl`, `unsafe extern` block and
unsafe attribute has a comment directly above it that starts with `// SAFETY: [id]` and a reason,
and the id names its entry here, a `## id` heading such as `## win-read-input`. Ids are lowercase
letters, digits and hyphens. Blocks that make the same call for the same reason may share an entry.

The test `crates/termelon/tests/unsafe_inventory.rs` lists every `unsafe` in the workspace and fails
when one is in any other file, when a block has no `SAFETY` comment, when an id has no entry, or when
an entry has no code that names it. Every `##` heading in this file is an entry.

Each entry says what the call needs from its arguments, where the code meets each requirement, what
happens when the call fails, and what a change near it must keep true.

On Windows, `console.rs` only makes the calls and copies what they give into plain structs. Which
modes to set, the reader's loop and the decoding of records are safe code, tested on every system
against a console played in memory.

## macos-select

`crates/termelon/src/sys/macos.rs`, in `wait`, the reader thread's wait for input on macOS, whose
`poll` doesn't work on `/dev/tty`.

`rustix::event::select` is unsafe because it can't check that every descriptor in its sets is open.
A descriptor closed during the call, or closed and reused before it, would make it watch the wrong
file. Its other needs are checked or met by rustix's types. Each set must have room for `nfds`
descriptors, and the timeout must be one `select` accepts.

- Every descriptor in the one set is one of `fds`, a `BorrowedFd` each, borrowed for the whole call,
  so none can close until `select` returns.
- `nfds` is one more than the highest of them, and the set comes from `fd_set_num_elements` with that
  `nfds`, so it has room for each. rustix passes sets of any size, as `_DARWIN_UNLIMITED_SELECT`
  does, so a descriptor above 1024 is fine too.
- The timeout comes from `timespec`, which caps it at a day, well within what `select` accepts.

When `select` fails, `wait` returns the error. The reader tries again on `EINTR` and ends on any
other error, which closes the event stream.

A change here must keep every descriptor in the sets borrowed for the call and the sets sized from
the highest of them. Never put a raw descriptor in a set that nothing borrows.

## win-console-mode

`crates/termelon/src/sys/windows/console.rs`, in `mode` and `set_mode`, which read and set the
console's input and output modes.

`GetConsoleMode` needs an open console handle and a place for a `u32`, and `SetConsoleMode` an open
console handle.

- The handle comes from a `File` the caller borrows for the call, so it stays open until the call
  returns.
- The mode is written through `&raw mut` to a local `u32`, which lives through the call.

A failed call returns the system's error, and a mode the console doesn't take fails the call rather
than changing anything. A change here must keep the handle borrowed from a `File` that outlives the
call.

## win-screen-buffer-info

`crates/termelon/src/sys/windows/console.rs`, in `window`, which reads the window's rectangle.

`GetConsoleScreenBufferInfo` needs an open screen buffer handle and a `CONSOLE_SCREEN_BUFFER_INFO`
to write.

- The handle is borrowed from a `File` for the call.
- The structure is a local of the type the call writes, made with `Default`, and passed through
  `&raw mut`.

A failed call returns the system's error, and the reader keeps the rectangle it had. A change here
must keep the structure the call's own type, so its size is right.

## win-pending-input

`crates/termelon/src/sys/windows/console.rs`, in `pending`, which counts the records waiting.

`GetNumberOfConsoleInputEvents` needs an open input handle and a place for a `u32`. The handle is
borrowed from a `File` for the call, and the count is a local written through `&raw mut`. A failed
call returns the system's error, which ends the reader.

## win-read-input

`crates/termelon/src/sys/windows/console.rs`, in `read_records`, the reader's read of input
records.

`ReadConsoleInputW` writes up to `nLength` records into the buffer and the number it wrote into the
count. It waits when the input is empty.

- The buffer is a local array of `READ_RECORDS` records, and `nLength` is its length, so the call
  never writes past it.
- The count is a local `u32` written through `&raw mut`, and only that many records are read back,
  capped at the array's length.
- The handle is borrowed from a `File` for the call.
- It's called only after `GetNumberOfConsoleInputEvents` says records are waiting, so it doesn't
  wait.

A failed call returns the system's error, which ends the reader and closes the event stream. A
change here must keep `nLength` the buffer's own length.

## win-record-union

`crates/termelon/src/sys/windows/console.rs`, in `copy_record`, which reads an `INPUT_RECORD`'s
union.

Reading a union field is defined only for the field the union holds. `EventType` says which, and
each read is in the match arm of the event type that names its field, `KeyEvent` for `KEY_EVENT`,
`MouseEvent` for `MOUSE_EVENT` and `FocusEvent` for `FOCUS_EVENT`. Window size and menu records
read no field. A key record's `uChar` is a union of two integers in the same place, so either read
is defined, and `ReadConsoleInputW` fills `UnicodeChar`.

A change here must keep each read inside the arm of its event type.

## win-create-event

`crates/termelon/src/sys/windows/console.rs`, in `WakeEvent::new`, the event that wakes the reader.

`CreateEventW` takes optional security attributes and an optional name, and returns a new handle or
null. Both are null, which the call allows, and a null result returns the system's error.

## win-own-event

`crates/termelon/src/sys/windows/console.rs`, in `WakeEvent::new`.

`OwnedHandle::from_raw_handle` needs a handle that nothing else owns, since it closes the handle
when dropped. The handle is the one `CreateEventW` just returned, checked not to be null, and
nothing else has seen it.

## win-set-event

`crates/termelon/src/sys/windows/console.rs`, in `WakeEvent::wake`.

`SetEvent` needs an open event handle. The handle is the event's own, owned by the `WakeEvent`
that `self` borrows, so it's open for the call. A failure is ignored, since the reader also ends on
its own once the program stops it.

## win-wait

`crates/termelon/src/sys/windows/console.rs`, in `wait`, the reader's wait on the console's input
and the wake event.

`WaitForMultipleObjects` reads `nCount` handles from the array and needs each to be open, and
`nCount` to be at most 64.

- The array holds the `N` handles given, each a `BorrowedHandle` for the call, so none can close
  until it returns. `N` is 2.
- The timeout is capped below `INFINITE`, or is `INFINITE` for no timeout.

`WAIT_FAILED` returns the system's error, and an abandoned wait (which only mutexes can give) is an
error too. Either ends the reader. A change here must keep every handle in the array borrowed for
the call.

## win-write-console

`crates/termelon/src/sys/windows/console.rs`, in `write_units`, the writer's write to the screen.

`WriteConsoleW` reads `nNumberOfCharsToWrite` UTF-16 units from the buffer, writes the count it
wrote, and needs its reserved pointer null.

- The count is the slice's length, so the call reads only the slice.
- The written count is a local `u32` written through `&raw mut`.
- The reserved pointer is null, and the handle is borrowed from a `File` for the call.

A failed call returns the system's error, which fails the writer. A change here must keep the count
the slice's own length.

## win-ctrl-handler

`crates/termelon/src/sys/windows/console.rs`, in `set_ctrl_handler`, which a program's
`restore_on_console_events` installs once.

`SetConsoleCtrlHandler` keeps the function to call it on a thread of the system's for each console
control event, so the function must stay valid for the life of the process and be safe to call from
any thread. It's `on_console_event`, a plain function of the library, which takes no lock it could
wait on forever (it tries the handle's lock for 50 ms) and returns false, so the system ends the
process as it does without a handler.

A failed call returns the system's error. A change here must keep the handler a function that
lives as long as the process.

## win-write-input

`crates/termelon/src/sys/windows/console.rs`, in `testing::write_input`, which tests that need a
real console use to type into it.

`WriteConsoleInputW` reads `nLength` records from the buffer and writes the count it wrote. The
records are a vector and `nLength` its length, the count is a local `u32`, and the handle is borrowed
from a `File`. The records are built whole, each union holding the field its event type names.

## win-read-output

`crates/termelon/src/sys/windows/console.rs`, in `testing::read_text`, which tests that need a real
console use to read back the screen.

`ReadConsoleOutputCharacterW` writes up to `nLength` UTF-16 units into the buffer and the count it
read. The buffer is a vector of `nLength` units, the count is a local `u32`, and only that many
units are read back. The handle is borrowed from a `File`.

## win-code-page

`crates/termelon/src/sys/windows/console.rs`, in `testing::code_pages`, which a test that needs a
real console uses to check that the code pages stay as the library found them.

`GetConsoleCP` and `GetConsoleOutputCP` take nothing and read the console of the process. Zero
means failure, which returns the system's error.

## harness-free-console

`tools/harness/src/windows.rs`, in `attach_console`, which the harness's console helper calls
before it attaches to a program's console.

`FreeConsole` takes nothing and detaches the process from its console. It fails, harmlessly, for a
process with none, which is how the helper starts, and the result is ignored.

## harness-attach-console

`tools/harness/src/windows.rs`, in `attach_console`.

`AttachConsole` takes a process id, which the call checks. A process that is gone, or has no
console, fails the call, which returns the system's error, and the helper tries again for ten
seconds before it gives up.

## harness-buffer-info

`tools/harness/src/windows.rs`, in `info`, which reads the window, the buffer's size and the
cursor of a screen buffer.

`GetConsoleScreenBufferInfo` needs an open screen buffer handle and a `CONSOLE_SCREEN_BUFFER_INFO`
to write. The handle is borrowed from a `File` for the call, and the structure is a local of the
call's own type, made with `Default` and passed through `&raw mut`. A failed call returns the
system's error.

## harness-console-size

`tools/harness/src/windows.rs`, in `set_console_size`, which makes a new console the size a
driver asked for.

`SetConsoleWindowInfo` reads a `SMALL_RECT` and `SetConsoleScreenBufferSize` takes a `COORD` by
value. Each needs an open screen buffer handle, borrowed from a `File` for the call, and the
rectangles are locals that live through it. A size the console refuses fails the call, which
returns the system's error and changes nothing.

## harness-read-output

`tools/harness/src/windows.rs`, in `read_window`, which reads the cells of a console's window.

`ReadConsoleOutputW` writes the cells of a region into a buffer whose size it's told, and writes
back the region it read.

- The buffer is a vector of `cols` times `rows` cells, and the size the call is given is `cols` by
  `rows`, so the call writes inside it.
- The region is the window, whose size is the same `cols` by `rows`, read from the console just
  before. A window that changed meanwhile is clipped to the buffer's size by the call.
- The region is a local written through `&raw mut`, and the handle is borrowed from a `File`.

A failed call returns the system's error. A change here must keep the buffer's length and the
size the call is given the same.

## harness-char-union

`tools/harness/src/windows.rs`, in `read_window`.

A `CHAR_INFO`'s `Char` is a union of two integers in the same place, so reading either is
defined, and `ReadConsoleOutputW` fills `UnicodeChar`.

## harness-cursor-info

`tools/harness/src/windows.rs`, in `read_window`.

`GetConsoleCursorInfo` needs an open screen buffer handle, borrowed from a `File` for the call, and
a `CONSOLE_CURSOR_INFO` to write, a local of the call's own type passed through `&raw mut`. A
failed call returns the system's error.

## harness-write-input

`tools/harness/src/windows.rs`, in `write_input`, which types key records into a console.

`WriteConsoleInputW` reads `nLength` records from the buffer and writes the count it wrote. The
records are a vector and `nLength` its length, the count is a local `u32`, and the handle is
borrowed from a `File`. Each record is built whole, its union holding the `KeyEvent` its event
type names.

## harness-send-input

`tools/harness/src/windows.rs`, in `send`, which presses keys in Windows Terminal's window.

`SendInput` reads `cInputs` structures of `cbSize` bytes each. The inputs are a slice whose length
is `cInputs`, `cbSize` is the size of `INPUT`, and each is a keyboard input built whole, its union
holding the `ki` its type names. Fewer inputs sent than given returns the system's error.

## harness-find-window

`tools/harness/src/windows.rs`, in `press_in_window`, which finds Windows Terminal's window.

`FindWindowW` reads a NUL-terminated UTF-16 class name and an optional window name. The class
name is a vector ending in 0 that lives through the call, and the window name is null, which
matches any window. No window found is a null handle, which the driver reports.

## harness-foreground

`tools/harness/src/windows.rs`, in `press_in_window`.

`GetForegroundWindow` takes nothing. `SetForegroundWindow` takes a window handle, here the one
`FindWindowW` gave just before. A window that closed since makes the call fail, which changes
nothing, and the driver then finds the window isn't in front and reports it.
