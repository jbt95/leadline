# Changelog

All notable changes use this file. Version numbers follow Semantic Versioning.

## 0.3.0 - Unreleased

### Added

- Hardened read-only Git access (`leadline::git`): every subprocess sets `GIT_NO_LAZY_FETCH=1`, `GIT_OPTIONAL_LOCKS=0`, and a stable C locale; annotated tags peel to exactly one commit; repository corruption and missing objects propagate instead of degrading silently.
- Source snapshots (`leadline::source_snapshot`): deterministic worktree, index, and revision source states with one shared source filter, symlink/regular-file rules, streamed `cat-file` object reads, target config/mailmap blobs, and strict path validation. In-memory analysis (`analyze_sources`, `analyze_dependencies_from_sources`) matches filesystem analysis.
- Revision-bounded Git analytics (`analyze_git_at[_with_mailmap]`): one walk yields history, per-file identity touches, and whole-project coupling; raw `%an/%ae` with a deterministic target `.mailmap` adapter (git-faithful precedence, no ambient config).
- Ownership analytics (`leadline::ownership`): anonymous touch concentration, bus factor 50, module sums, and opt-in author rows with artifact-local anonymized labels. Aggregate reports never contain identities.
- Full-state diff intelligence (`leadline::debt`): threshold-transition debt (`new`/`existing`/`resolved` per dimension, unknown counted separately) plus complete-state risk deltas with component breakdown; `leadline debt [--base REV] [--staged|--target REV] [--renames] [--fail-on-regression]`.
- Mutation and test adapters (`leadline::mutation`, `leadline::test_relationships`): PIT `mutations.xml` and Stryker JSON normalize to one row shape with provenance IDs, git-mailmap-safe identities, strict external paths, and a documented score; explicit versioned test maps resolve against analyzed functions.
- Duplication (`leadline::duplication`, profile `tokens-v1`): type-2 token clones with language partitions, bounded candidate comparisons, stable content IDs, occurrence-level drift with rename mapping, and incomplete-report signaling at ceilings.
- Architecture policy (`leadline::policy`) and `change-risk-v2` (`leadline::risk_v2`): ordered deny rules over high-confidence edges, `new`/`existing`/`resolved` drift, and policy/concentration-aware scoring. `change-risk-v1` stays byte-for-byte unchanged.
- Canonical Project model (`leadline::project`) with meta, summary, modules, files, functions, dependencies, cycles, Git activity, coupling, ownership, coverage, mutation, test relationships, duplication, policy violations, risk, and optional trend points.
- Trend snapshots (`leadline::snapshots`): HEAD-tree capture, config/model-aware keys, idempotent append, `--replace` for changed inputs, and lock-protected atomic writes; `leadline snapshot --output FILE`.
- Orchestration (`leadline::analytics`) and new CLI commands: `leadline project`, `leadline debt`, and `leadline snapshot` with JSON and agent-JSON projections.
- Configuration: `[duplication]` settings and `[[architecture.rules]]` with strict validation; bounded external inputs (config 1 MiB/16 levels, XML/JSON per-file and aggregate limits, JSON/XML depth caps, DTD/entity/encoding rejection) and strict report-path parsing.

### Notes

- Remaining E-I work on this branch: static web report generation, focused `mutation`/`duplication`/`policy` CLI commands, MCP tool parity, and Project coverage/mutation CLI inputs.

## Unreleased

### Added

- Git history analytics (`leadline::history`): per-file commit counts, 30/90/365-day change windows, lines added/deleted, code age, days since last change, and contributor counts. One streamed `git log --relative` walk with rename resolution; recency windows are relative to the HEAD commit time for deterministic reports; snapshots without Git degrade to `git_available: false` instead of failing.
- `leadline hotspots` with `--limit N`, `--since 30d|90d|365d`, `--json`, `--format agent-json`, and LCOV/JaCoCo coverage flags. Ranks files by `max cognitive complexity x changes in window` (`complexity-x-churn-v1`) and exposes every dimension: complexity, CRAP, coverage, churn, contributors, and age.
- `docs/analytics-roadmap.md`: architecture proposal, normalized Project analytics schema, static report data contract, Git ingestion trade-offs, and milestones B-I. `docs/hotspots.md`: formulas, calculation rules, limitations, and the no-developer-ranking guardrail.
- Git history benchmark target (`cargo bench --bench history`) measuring the raw log walk, end-to-end history analysis, and hotspot scoring separately; CI compiles it alongside the analyzer bench.
- Temporal (change) coupling: `leadline coupling TARGET` lists files that repeatedly change in the same commits, with co-change counts, directional coupling, and Jaccard similarity. One shared streamed history walk; commits wider than 50 files do not create pairs; `--min-cochanges`, `--top`, `--json`, and `--format agent-json` supported. See `docs/coupling.md`.
- Static dependency intelligence (`leadline::graph`, `leadline::impact`): `leadline dependencies [PATH]` reports file-level edges (source imports target), per-file fan-in/fan-out, unresolved imports with reasons, and import cycles; `leadline impact TARGET` reports transitive dependents with shortest distances, direct dependents, blast radius (`blast_radius_percent` on a 0-100 scale), and the cycles containing the target. Resolution covers relative JS/TS imports (including `require()` / `import()` forms, extensionless and emitted-JS mapping) and exact Java type imports (including static-member stripping); bare package imports are ignored and ambiguity stays unresolved with a reason. Both commands support `--json` and `--format agent-json` (with `--top` truncation on `impact`); reports are deterministically ordered and byte-identical across runs. See `docs/dependencies.md`.
- Explainable change risk (`leadline::risk`): `leadline risk [PATH]` ranks files with the `change-risk-v1` model — complexity (25), CRAP (20), churn (20), impact (20), and ownership (15) components on a 0-100 scale, plus an always-`null` weight-0 `policy` placeholder. Unknown components stay `null` and the score renormalizes over the known weights; rows sort by score desc, path asc. `--limit N` (default 10), `--since 30d|90d|365d` (default `90d`, HEAD-relative like hotspots), `--json`, `--format agent-json`, and LCOV/JaCoCo coverage flags supported. Informational only (exit `0`); gating is deferred to Milestone E. See `docs/risk.md`.

## 0.2.0 - 2026-09-13

### Fixed

- `install.sh` now verifies `SHA256SUMS` entries that carry a directory prefix (published releases list `dist/<archive>`); the release workflow now writes bare file names.

## 0.1.0 - 2026-09-13

### Added

- Function analysis for Java, JavaScript, TypeScript, and TSX.
- `default-v1` cyclomatic and cognitive complexity rules.
- Physical LOC, logical LOC, parameter, nesting, Halstead, and maintainability metrics.
- LCOV and JaCoCo coverage with function-level CRAP scores.
- Deterministic JSON, terminal reports, function lookup, changed-function analysis, and quality gates.
- Gitignore-aware parallel discovery and five-platform release builds.
- Fixture tests, parser diagnostics, and 10K, 100K, and 1M line benchmarks.
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
- MCP `initialize` returns usage instructions, and each tool carries a display title plus read-only, idempotent, and closed-world annotations; descriptions now state when to reach for the tool.
- `curl | sh` installer (`install.sh`): detects macOS/Linux and arm64/x86-64, verifies the release `SHA256SUMS`, and installs to `~/.local/bin` (`LEADLINE_INSTALL_DIR`, `LEADLINE_VERSION` overrides).
- Saved baselines: `leadline baseline --output FILE` snapshots and `check --baseline FILE` gates; MCP `check` accepts a read-only `baseline` path and never writes snapshots.

### Fixed

- MCP stdio responses flush after every line: live clients keep stdin open while waiting, and buffering until EOF made them time out (`-32001`).
- MCP `initialize` echoes the client's requested protocol version instead of a fixed stale one, so modern SDK clients no longer reject the handshake.
- MCP `tools/call` success results use the standard `CallToolResult` envelope (`content` text block plus `structuredContent`); without it, hosts surfaced `null` results.
