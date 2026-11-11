# Why this is safe

The library denies `unsafe_code` everywhere except two modules, `sys/macos.rs` and
`sys/windows/console.rs`, and the harness's Windows driver. Each unsafe block holds one unsafe
operation and a `// SAFETY: [id]` comment that names its entry here. A test in the library lists every
unsafe block in the workspace and fails when a block has no entry, an entry has no block, or `unsafe`
appears in any other file.

Each entry says what the call needs from its arguments, where the code meets each requirement, what
happens when the call fails, and what a change near it must keep true.

There are no unsafe blocks yet.
