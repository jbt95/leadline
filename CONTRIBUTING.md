# Contributing

Open an issue before large interface or metric-rule changes. Bug fixes can start with a pull request.

Every metric change must include a small fixture with explicit expected values. Language-specific behavior must also update `docs/metrics.md`.

Before submission, run:

```console
cargo fmt --check
cargo clippy --offline --all-targets --locked -- -D warnings
cargo test --offline --locked
```

Performance changes must include Criterion results from the same machine before and after the change.

Do not copy source from other analyzers. Describe external research and check its license first.

By contributing, you license your work under this repository's MIT license.
