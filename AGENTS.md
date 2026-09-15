# AGENTS.md — working rules for leadline

## Backwards compatibility

Do not add backwards-compatibility machinery unless the user explicitly
asks for it: no versioned name suffixes (`-v1`/`-v2`), no parallel old/new
modules, no deprecated-flag aliases, no migration paths, no "keep the old
field for compatibility". When behavior must change, change it outright
and update the code, the tests, the docs, and the CHANGELOG in the same
commit. If compatibility ever becomes necessary, that decision gets its
own explicit request — it is never the default.

## Definitions that stay versioned

These are live identifiers, not compat: integer `schema_version`
fields, `analyzer_version`, and the external SARIF `v2.1.0` schema URL.
Historical CHANGELOG entries and `docs/superpowers/plans/` keep their
original names as history; do not rewrite them.

## Engineering rules

- Deletion over addition. Shortest working diff wins.
- No unrequested abstractions (one-implementation interfaces, factories
  for one product, config for values that never change).
- Non-trivial logic leaves one runnable check behind; trivial
  one-liners need no test.
- Verify before claiming: `cargo fmt --check`,
  `cargo clippy --offline --all-targets`, `cargo test --offline`.
- Rust toolchain 1.90:
  `export PATH="$HOME/.rustup/toolchains/1.90-aarch64-apple-darwin/bin:$PATH"`.
- `cargo test` stays offline and network-free; network fixtures live
  behind `scripts/realworld.sh` + `cargo bench --bench realworld` only.
- Implementation plans in `docs/superpowers/plans/` execute
  subagent-driven: one fresh implementer per task, reviewed between
  tasks, never inline in the primary session.
