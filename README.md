# leadline

A native Rust CLI for deterministic function-level complexity analysis of Java, JavaScript, TypeScript, and TSX.

`leadline` reports physical and logical LOC, parameters, nesting, cyclomatic complexity, cognitive complexity, Halstead metrics, maintainability, coverage, and CRAP.

## Install

Download a binary from GitHub Releases, or build with Rust 1.88 or later:

```console
cargo install --path .
```

Release artifacts support macOS ARM64, macOS x86-64, Linux ARM64, Linux x86-64, and Windows x86-64.

## Analyze

```console
leadline analyze .
leadline analyze . --json
leadline analyze src/payment.ts
leadline analyze . --lcov coverage/lcov.info --json
leadline analyze . --jacoco build/reports/jacoco/test/jacocoTestReport.xml
leadline function src/payment.ts processPayment --json
```

The analyzer reads and parses each file once. Rayon workers process files independently. Repository discovery follows `.gitignore`.

Generated and vendor directories are skipped by default. Explicit file paths remain analyzable.

JSON files are sorted by path. Functions use source order. `schema_version` identifies output compatibility. Each file includes deterministic `parse_errors`.

## Quality gates

```console
leadline check . --cognitive 15 --cyclomatic 10 --max-nesting 4
leadline check . --crap 30 --lcov coverage/lcov.info --json
```

Thresholds fail only when a value exceeds its limit. A CRAP threshold also fails unavailable coverage.

Exit codes are stable:

- `0`: analysis succeeded, or a quality gate passed.
- `1`: a quality gate found metric violations or source parse errors.
- `2`: usage or config error (unknown flag, missing threshold, bad path, invalid `leadline.toml`).
- `3`: incomplete analysis (no supported files, Git failure, unreadable input).
- `4`: coverage input error (unreadable or unparsable LCOV / JaCoCo file).
- `5`: internal error.

See the [CLI reference](docs/cli-reference.md), [JSON schema](docs/json-schema.md),
[configuration](docs/configuration.md), and [agent integration guide](docs/agent-integration-guide.md).

## Changed functions

```console
leadline changed --base origin/main --json
leadline diff HEAD~1
```

Functions are paired by name and same-name source order. Renames appear as one removal and one addition.

`--format agent-json` emits the compact agent-oriented shape on `analyze`, `function`,
`check`, `changed`, and `diff`. `leadline doctor` self-checks the parsers, coverage
readers, `git`, and `leadline.toml`. `leadline version` prints the release version.

## Coverage limits

LCOV and JaCoCo line coverage are supported. Windows and Unix report paths are normalized.

Ambiguous suffix matches remain unavailable instead of attaching incorrect coverage. Nested functions can share covered lines.

Source maps and Cobertura are not supported.

## Development

```console
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo bench --bench analyzer
```

See [architecture](docs/architecture.md), [`default-v1` metric rules](docs/metrics.md), and [benchmark instructions](docs/benchmark.md).
