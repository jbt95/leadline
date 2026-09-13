# AGENTS.md — working rules for leadline

## Product status

leadline has **no public release and no active users**. There is nothing
to stay compatible with: no backwards-compatibility shims, no versioned
model-name suffixes (`-v1`/`-v2`), no deprecated-flag aliases, no
migration paths. Model names are plain words (`change-risk`, `impact`,
`complexity-x-churn`, `mutation`, `tokens`, `duplication-drift`,
`change-risk-diff`) and the metric profile is `default`.

When behavior must change, change it outright: update the code, the
tests, the docs, and the CHANGELOG in the same commit. Never add a
`v2` module next to a `v1` module, never keep an old JSON field
"for compatibility", never branch on a version string to preserve
past output. If a compat layer ever becomes necessary (first public
release), that decision gets its own proposal — it is not the default.

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
