# CLI reference

Binary: `leadline`. Every command prints terminal text by default, JSON with `--json`.

## Commands

```console
leadline analyze [PATH] [--json] [--format agent-json|sarif] [--top N] [--sort-by KEY] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE] [--cache-dir DIR]
leadline function FILE NAME [--json] [--format agent-json] [--explain] [--top N] [--sort-by KEY] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE]
leadline check [PATH] [--base REV | --baseline FILE] [--regressions] [--cognitive N] [--cyclomatic N] [--crap N] [--max-nesting N] [--json] [--format agent-json|sarif] [--top N] [--sort-by KEY] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE] [--cache-dir DIR]
leadline changed [--base REV] [--staged | --target REV] [--renames] [--path PATH] [--json] [--format agent-json] [--explain] [--top N] [--sort-by KEY] [--min-crap X] [--min-delta D]
leadline diff [REV] [--staged | --target REV] [--renames] [--path PATH] [--json] [--format agent-json] [--explain] [--top N] [--sort-by KEY] [--min-crap X] [--min-delta D]
leadline hotspots [PATH] [--limit N] [--since 30d|90d|365d] [--json] [--format agent-json] [--lcov FILE] [--jacoco FILE] [--coverage FILE]
leadline risk [PATH] [--limit N] [--since 30d|90d|365d] [--json] [--format agent-json] [--lcov FILE] [--jacoco FILE] [--coverage FILE]
leadline project [PATH] [--target REV] [--since 30d|90d|365d] [--pit FILE]... [--stryker FILE]... [--test-map FILE]... [--snapshots FILE] [--include-authors | --anonymize-authors | --exclude-git-identities] [--json] [--format agent-json]
leadline debt [--base REV] [--staged | --target REV] [--renames] [--path PATH] [--since 30d|90d|365d] [--fail-on-regression] [--json] [--format agent-json]
leadline snapshot [PATH] --output FILE [--since 30d|90d|365d] [--pit FILE]... [--stryker FILE]... [--replace]
leadline mutation [PATH] (--pit FILE | --stryker FILE)... [--test-map FILE]... [--json]
leadline duplication [PATH] [--base REV] [--json]
leadline policy [PATH] [--base REV] [--fail-on-violation] [--json]
leadline coupling TARGET [--path ROOT] [--top N] [--min-cochanges N] [--json] [--format agent-json]
leadline dependencies [PATH] [--json] [--format agent-json]
leadline impact TARGET [--path ROOT] [--top N] [--json] [--format agent-json]
leadline test-targets [PATH] (--coverage FILE | --lcov FILE | --jacoco FILE) [--top N] [--format agent-json]
leadline baseline [PATH] --output FILE [--lcov FILE] [--jacoco FILE] [--coverage FILE]
leadline --version
leadline version
leadline doctor [PATH]
leadline mcp
leadline skill
```

- `analyze`: all supported functions under `PATH` (default `.`), or one file. Output sorted by path, functions in source order.
- `function`: functions named `NAME` in `FILE` only. Errors when the name is absent. `--explain` appends one terminal line per metric contribution (`<rule> line <line> nesting <nesting> +<cog> cog +<cyc> cyc`); both JSON shapes already carry `contributions`, and `--format agent-json` includes the array only with `--explain`.
- `check`: like `analyze`, but keeps only violations and parse errors. It requires at least one threshold flag unless `--regressions` is selected. With `--base REV`, gates changed functions between the revision and the working tree. With `--baseline FILE`, gates current functions against a saved snapshot instead (exclusive with `--base`): paired functions fail on absolute or (with `--regressions`) delta violations, new functions fail only on absolute thresholds, and deleted functions are ignored. `--regressions` applies configured positive-delta limits to paired functions; added and removed functions are excluded. Absolute and delta gates combine, and either failure exits `1`.
- `changed` / `diff`: functions changed between a base revision and a comparison target (default the working tree). `changed` takes `--base REV` (default `HEAD~1`); `diff` takes the base revision positionally. `--staged` compares against the index instead of the working tree; `--target REV` compares against another revision instead; the two are mutually exclusive (exit `2`). `--renames` enables Git file rename detection (`-M`), pairing old-path content with new content under the new path. `--explain` adds deterministic multiset-added contribution causes, with after-side lines, to regression rows in agent JSON. Both commands accept `--path` to scope to a file or directory.
- `hotspots`: rank files by `max cognitive complexity x changes in the selected window` and expose every dimension (complexity, CRAP, coverage, churn, contributors, age). Git history is read with one streamed `git log --relative` walk; recency windows are relative to the HEAD commit time, not the wall clock (deterministic). A directory without Git still ranks by complexity and reports `git_available: false`. See `docs/hotspots.md` for formulas and limitations. `--limit N` (default 10) caps rows; `--since 30d|90d|365d` selects the window (default `90d`); coverage flags merge LCOV/JaCoCo before the join.
- `risk`: rank files by the explainable `change-risk` score with exposed components (complexity 20, CRAP 15, churn 20, impact 20, ownership concentration 10, policy 15). Unknown components stay `null` and the score renormalizes over the known weights; rows sort by score desc, path asc. Windows are HEAD-relative like `hotspots`; without Git, churn and ownership are `null` with `git_available: false`. Informational only (exit `0`); risk gating is deferred to Milestone E. See `docs/risk.md` for formulas and limitations. `--limit N` (default 10, `N >= 1`) caps terminal and agent-JSON rows (`--json` is always the full ranking); `--since 30d|90d|365d` selects the window (default `90d`); coverage flags merge LCOV/JaCoCo before the join.
- `project`: build the canonical Project across every analytics section (files, functions, dependencies, Git activity, coupling, ownership, duplication, policy, risk, optional mutation/test/trend sections) and print a summary, `--json`, or compact `--format agent-json`. `--target REV` analyzes a revision, `--pit`/`--stryker`/`--test-map` merge external reports, `--snapshots FILE` attaches trend points, and the ownership flags are mutually exclusive. Coverage in Project output is not wired yet.
- `debt`: compare complete before/after states. Function debt classifies per configured threshold as `new`, `existing`, or `resolved`; unknown metrics are counted, never guessed. Risk rows pair by path (with `--renames`) and expose score/component deltas. `--staged` and `--target` are exclusive. Informational (exit `0`); `--fail-on-regression` exits `1` on new debt or increased risk.
- `snapshot`: capture one HEAD-tree trend point (never dirty worktree state) to an append-only store. Identical keys and fingerprints are idempotent; changed inputs for an existing key require `--replace`; a sibling lock guards concurrent writers, and writes are atomic. Missing HEAD exits `3`.
- `mutation`: normalize PIT/Stryker reports (and optional explicit test maps) against the analyzed sources. At least one input is required. Rows carry provider provenance, strict paths, 1-based half-open spans, optional unique innermost function IDs, and reasons for unresolved rows. Unsupported schemas and unreadable inputs exit `4`.
- `duplication`: detect `tokens` clones for the current state, or occurrence-level `new`/`existing`/`resolved` drift with `--base REV` (rename-mapped). Exceeding the token or comparison ceiling exits `3` with `complete: false`.
- `policy`: evaluate `[[architecture.rules]]` over high-confidence resolved edges; `--base REV` adds `new`/`existing`/`resolved` drift with rename-mapped endpoints. Informational; `--fail-on-violation` exits `1` when current error-severity violations exist.
- `coupling`: list files that repeatedly change in the same commits as `TARGET` (a file under the scope), ranked by directional coupling. Exposes `co_changes`, `commits`, directional, reverse-directional, and Jaccard values; commits wider than 50 files do not create pairs, and `--min-cochanges N` (default 2) suppresses one-off coincidences. `TARGET` must exist; use `--path ROOT` to set the analysis scope (default `.`). A directory without Git reports `git_available: false`. See `docs/coupling.md` for formulas and limitations.
- `dependencies`: report the static file-level dependency graph under `PATH` (default `.`): edges (`source` imports `target`), per-file fan-in/fan-out, unresolved imports with reasons, and import cycles. Honors `leadline.toml` `[analysis] exclude`. See `docs/dependencies.md` for the supported import forms and limitations.
- `impact`: report the transitive dependents of `TARGET` (a file under the scope) by reverse-BFS over the dependency graph: distances, direct dependents, blast radius, and the cycles containing the target. `TARGET` must exist under `--path ROOT` (default `.`); a missing file, a path outside the scope, or a file with no graph node is a usage error (exit `2`). `--top N` (default 20, `N >= 1`) caps the shown dependents; `blast_radius` still counts every dependent and `truncated` signals the cap. See `docs/dependencies.md` for definitions.
- `test-targets`: rank functions holding decision lines with known zero line-coverage hits, sorted by CRAP descending, then path/function/line. Coverage is required (exit `2` without it). Rows carry `uncovered` (known-zero-hit) and `unknown` (absent from the coverage record) contribution lines; unknown is never called uncovered. Output is capped at `--top N` (default 200). Coverage is line coverage, not branch coverage.
- `baseline`: snapshot current function metrics to `FILE` (writes atomically via a sibling temp file, then rename). Review the file, commit it, and gate later edits with `check --baseline FILE --regressions`. Snapshot rows pair with current functions by path and name in same-name source order; parsing rejects unknown schemas, unknown metric profiles, and duplicate identities (exit `3`).
- `--version` / `-V` / `version`: print `leadline <version>`.
- `doctor`: self-check parsers, coverage readers, `git`, and `leadline.toml`.
- `mcp`: serve the read-only MCP tool API over stdio.
- `skill`: print the canonical agent skill (`integrations/common/leadline-skill/SKILL.md`, baked into the binary).

## Flags

| Flag | Commands | Meaning |
| --- | --- | --- |
| `--json` | all analysis | Emit JSON report instead of terminal text. Exclusive with any `--format`. |
| `--format agent-json` | analyze, function, check, changed, diff, hotspots, risk, coupling, dependencies, impact, test-targets | Emit the compact agent-oriented JSON shape. Carries a `truncated` bool when budget flags drop entries — except `risk`, whose agent JSON has no `truncated` field (`--limit` caps its rows silently). |
| `--format sarif` | analyze, check | Emit SARIF 2.1.0 (`version` / `runs` / `results`), violations only. Exclusive with `--json`. |
| `--top N` | analyze, function, check, changed, diff, coupling, impact, test-targets | Keep at most `N` entries per list (`coupling` and `impact` default 20, `test-targets` default 200). Only with `--format agent-json` (`N >= 1`), except `coupling`, `impact`, and `test-targets` where it also caps terminal/JSON output. |
| `--sort-by KEY` | analyze, function, check, changed, diff | Sort budget entries by `crap`, `cognitive`, or `cyclomatic`. Only with `--format agent-json`. |
| `--min-crap X` | analyze, function, check, changed, diff | Drop entries below CRAP `X`. Only with `--format agent-json`. |
| `--min-delta D` | changed, diff | Drop entries whose max before/after delta is below `D`. Only with `--format agent-json`. |
| `--explain` | function, changed, diff | Add contribution details to function agent JSON, or multiset-added causes to changed regression rows. |
| `--cache-dir DIR` | analyze, check | Reuse per-file analysis from `DIR/file-cache.json` across runs; saved at end only when its contents change. Bypassed when any coverage flag is passed (cached functions are pre-coverage, so CRAP would go stale). Cache I/O failures warn on stderr, never fail. |
| `--limit N` | hotspots, risk | Keep at most `N` ranked files (`N >= 1`, default 10). On `hotspots` caps terminal, JSON, and agent JSON (`truncated` signals the cap); on `risk` caps terminal and agent JSON only — `--json` is always the full ranking with no `truncated` field. |
| `--since WINDOW` | hotspots, risk | Rank changes over `30d`, `90d`, or `365d` (default `90d`). |
| `--lcov FILE` | analyze, function, check, hotspots, risk, test-targets | Merge LCOV line coverage. Repeatable. Required on `test-targets` (one coverage flag at minimum). |
| `--jacoco FILE` | analyze, function, check, hotspots, risk, test-targets | Merge JaCoCo XML line coverage. Repeatable. Required on `test-targets` (one coverage flag at minimum). |
| `--coverage FILE` | analyze, function, check, hotspots, risk, test-targets | Merge coverage with format detected from the extension (`.info` LCOV, `.xml` JaCoCo). Repeatable. Required on `test-targets` (one coverage flag at minimum). |
| `--regressions` | check | Enable regression-only gates. Limits come from `[regressions]` and default to zero. It removes the absolute-threshold requirement; combine with `--base REV` or `--baseline FILE` to compare changed functions. |
| `--base REV` | check, changed | Base revision (`changed` default `HEAD~1`). Exclusive with `--baseline` on `check`. |
| `--baseline FILE` | check | Saved snapshot for regression gates without Git. Exclusive with `--base`. |
| `--output FILE` | baseline | Snapshot destination (required). |
| `--staged` | changed, diff | Compare `--base`/`REV` against the index (staged blobs) instead of the working tree. Exclusive with `--target`. |
| `--target REV` | changed, diff | Compare `--base`/`REV` against another revision instead of the working tree. Exclusive with `--staged`. |
| `--renames` | changed, diff | Detect Git file renames (`-M`) and pair a renamed file's old content with its new content under the new path. |
| `--path PATH` | changed, diff, coupling, impact | Scope changed analysis to a file or directory; on `coupling` and `impact` it sets the history/graph scope (default `.`). |
| `--min-cochanges N` | coupling | Drop related files with fewer than `N` shared commits (default 2, minimum 1). |

Budget flags with `--json` or default terminal output are a usage error (exit `2`), never silently ignored. `--top 0` and unparsable budget values are usage errors.

`check` thresholds fail only when a value exceeds its limit. A `--crap` threshold also fails when coverage (and therefore CRAP) is unavailable for a function.

`--format sarif` on `check` uses the command thresholds; on `analyze` it uses `leadline.toml` thresholds when present, else empty thresholds (rules listed, empty results).
