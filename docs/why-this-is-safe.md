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

There are no unsafe blocks yet.
