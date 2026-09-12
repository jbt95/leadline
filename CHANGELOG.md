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
- Comparison targets on `changed`/`diff`: `--staged`, `--target REV`, and `--renames` (Git file renames only, no fuzzy function matching).
- Changed-regression explanations: `--explain` adds multiset-added contribution causes to regression rows.
- Regression-only gates: `[regressions]` config with allowed deltas and `check --base REV --regressions`.
- Coverage-aware test targets: `leadline test-targets` CLI and read-only MCP `test_targets` tool (line coverage only).
- MCP `repo_summary` tool: repository totals plus top functions by CRAP, cognitive, and cyclomatic complexity.
- MCP budget parameters on `analyze` and `analyze_changed` (`top`, `sort_by`, `min_crap`, `min_delta`) mirroring the CLI agent-json flags.
- MCP `check` accepts a coverage file so CRAP thresholds and CRAP deltas gate on real data.
- MCP `analyze_function` `explain: true` returns per-decision contribution lines.
- Saved baselines: `leadline baseline --output FILE` snapshots and `check --baseline FILE` gates; MCP `check` accepts a read-only `baseline` path and never writes snapshots.

### Fixed

- MCP stdio responses flush after every line: live clients keep stdin open while waiting, and buffering until EOF made them time out (`-32001`).
- MCP `initialize` echoes the client's requested protocol version instead of a fixed stale one, so modern SDK clients no longer reject the handshake.
- MCP `tools/call` success results use the standard `CallToolResult` envelope (`content` text block plus `structuredContent`); without it, hosts surfaced `null` results.

## 0.1.0 - 2026-09-12

### Added

- Function analysis for Java, JavaScript, TypeScript, and TSX.
- `default-v1` cyclomatic and cognitive complexity rules.
- Physical LOC, logical LOC, parameter, nesting, Halstead, and maintainability metrics.
- LCOV and JaCoCo coverage with function-level CRAP scores.
- Deterministic JSON, terminal reports, function lookup, changed-function analysis, and quality gates.
- Gitignore-aware parallel discovery and five-platform release builds.
- Fixture tests, parser diagnostics, and 10K, 100K, and 1M line benchmarks.
