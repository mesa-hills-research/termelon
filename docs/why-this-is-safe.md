# Why this is safe

The library denies `unsafe_code` everywhere except two modules, `sys/macos.rs` and
`sys/windows/console.rs`, and the harness's Windows driver, `tools/harness/src/windows.rs`. Each
unsafe block holds one unsafe operation. Every block, `unsafe impl`, `unsafe extern` block and
unsafe attribute has a comment directly above it that starts with `// SAFETY: [id]` and a reason,
and the id names its entry here, a `## id` heading such as `## win-read-input`. Ids are lowercase
letters, digits and hyphens. Blocks that make the same call for the same reason may share an entry.

The test `crates/tuiframe/tests/unsafe_inventory.rs` lists every `unsafe` in the workspace and fails
when one is in any other file, when a block has no `SAFETY` comment, when an id has no entry, or when
an entry has no code that names it. Every `##` heading in this file is an entry.

Each entry says what the call needs from its arguments, where the code meets each requirement, what
happens when the call fails, and what a change near it must keep true.

## macos-select

`crates/tuiframe/src/sys/macos.rs`, in `wait`, the reader thread's wait for input on macOS, whose
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
