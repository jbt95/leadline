# Changelog

All notable changes use this file. Version numbers follow Semantic Versioning.

## Unreleased

### Added

- Renamed `code-health` to `leadline`.
- Compact `--format agent-json` output on `analyze`, `function`, `check`, `changed`, and `diff`.
- Local MCP server module (`leadline::mcp`) for agent tool calls.
- Optional `leadline.toml` project configuration with discovery excludes and check thresholds.
- `leadline doctor` self-check and `leadline version` subcommand.
- `check --base REV` quality gate scoped to changed functions.
- `function --explain` per-decision metric contributions.
- `--format sarif` output on `analyze` and `check`.
- Output budgets (`--top`, `--sort-by`, `--min-crap`, `--min-delta`) with `truncated` signaling on agent JSON.
- `--cache-dir` content-hash incremental file cache for `analyze` and `check`.

## 0.1.0 - 2026-09-12

### Added

- Function analysis for Java, JavaScript, TypeScript, and TSX.
- `default-v1` cyclomatic and cognitive complexity rules.
- Physical LOC, logical LOC, parameter, nesting, Halstead, and maintainability metrics.
- LCOV and JaCoCo coverage with function-level CRAP scores.
- Deterministic JSON, terminal reports, function lookup, changed-function analysis, and quality gates.
- Gitignore-aware parallel discovery and five-platform release builds.
- Fixture tests, parser diagnostics, and 10K, 100K, and 1M line benchmarks.
