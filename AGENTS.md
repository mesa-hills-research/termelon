# Notes for coding agents

Termelon is a terminal UI framework in pure Rust. README.md says what it does and `docs/` how it works.

- Pure Rust. A new dependency needs a reason in its `Cargo.toml` comment, and nothing may need a C or
  C++ toolchain or system headers to build.
- `unsafe` lives only in `crates/termelon/src/sys/macos.rs`, `crates/termelon/src/sys/windows/console.rs`
  and the harness's Windows driver. Each block holds one unsafe operation, has a `// SAFETY: [id]`
  comment, and has an entry with that id in `docs/why-this-is-safe.md`. The inventory test enforces it.
- The lints in the workspace `Cargo.toml` are deliberate, each with its reason. Fix the code rather than
  allowing a lint, and when an allow is right, use `#[expect(lint, reason = "...")]` at the narrowest
  scope.
- The library never writes to the terminal except through its writer thread, never prints, never
  panics on input or on the system's errors, and never ends the process.
- A change in behaviour comes with a test. Output changes are checked against `termelon-vt`, the
  emulator crate, which fails on any sequence it doesn't know.
- `termelon-vt` doesn't depend on `termelon`, so it stays an independent check of what the library
  writes.
- Tests, fixtures, examples and docs use made-up paths and names, never a real machine's or person's.
- Commit subjects are plain sentences saying the behaviour. Messages are public, so they say what changed
  and why for any user of the project.
- Everything a user reads (docs, comments, error messages) uses the fewest plain words that keep the
  meaning. Join clauses with a period or a comma, not a colon or a semicolon. Error messages are
  lowercase phrases that callers put into sentences of their own.
