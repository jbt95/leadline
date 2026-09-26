# CLI reference

Binary: `leadline`. Every command prints terminal text by default, JSON with `--json`.

## Commands

```console
leadline analyze [PATH] [--json] [--format agent-json|sarif] [--top N] [--sort-by KEY] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE] [--index DIR]
leadline function FILE NAME [--json] [--format agent-json] [--explain] [--top N] [--sort-by KEY] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE]
leadline check [PATH] [--base REV | --baseline FILE] [--regressions] [--cognitive N] [--cyclomatic N] [--crap N] [--max-nesting N] [--json] [--format agent-json|sarif|<ci-format>] [--top N] [--sort-by KEY] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE] [--index DIR] [--sarif FILE] [--baseline-sarif FILE] [--fail-on-severity LEVEL] [--new-only] [--changed-only] [--osv FILE] [--trivy FILE] [--baseline-osv FILE] [--baseline-trivy FILE] [--sql] [--sql-fail-on-severity LEVEL]
leadline index [PATH] [--output DIR] [--verify] [--json]
leadline security [PATH] --sarif FILE [--baseline-sarif FILE] [--base REV | --staged | --target REV] [--fail-on-severity low|medium|high|critical] [--new-only] [--changed-only] [--json] [--format agent-json|sarif|<ci-format>] [--top N]
leadline changed [--base REV] [--staged | --target REV] [--renames] [--path PATH] [--json] [--format agent-json] [--explain] [--top N] [--sort-by KEY] [--min-crap X] [--min-delta D]
leadline diff [REV] [--staged | --target REV] [--renames] [--path PATH] [--json] [--format agent-json] [--explain] [--top N] [--sort-by KEY] [--min-crap X] [--min-delta D]
leadline hotspots [PATH] [--limit N] [--since 30d|90d|365d] [--json] [--format agent-json] [--lcov FILE] [--jacoco FILE] [--coverage FILE]
leadline risk [PATH] [--limit N] [--since 30d|90d|365d] [--json] [--format agent-json] [--lcov FILE] [--jacoco FILE] [--coverage FILE]
leadline project [PATH] [--target REV] [--since 30d|90d|365d] [--pit FILE]... [--stryker FILE]... [--test-map FILE]... [--snapshots FILE] [--include-authors | --anonymize-authors | --exclude-git-identities] [--lcov FILE] [--jacoco FILE] [--coverage FILE] [--json] [--format agent-json]
leadline debt [--base REV] [--staged | --target REV] [--renames] [--path PATH] [--since 30d|90d|365d] [--fail-on-regression] [--json] [--format agent-json]
leadline snapshot [PATH] --output FILE [--since 30d|90d|365d] [--pit FILE]... [--stryker FILE]... [--replace]
leadline mutation [PATH] (--pit FILE | --stryker FILE)... [--test-map FILE]... [--json]
leadline duplication [PATH] [--base REV] [--json]
leadline policy [PATH] [--base REV] [--fail-on-violation] [--json]
leadline vulnerabilities [PATH] --osv FILE [--trivy FILE] [--baseline-osv FILE] [--baseline-trivy FILE] [--base REV | --staged | --target REV] [--fail-on-severity low|medium|high|critical] [--json] [--format agent-json|sarif|<ci-format>] [--top N]
leadline sql [PATH] [--large-offset N] [--migration-root DIR] [--fail-on-severity low|medium|high|critical] [--json] [--format agent-json|sarif|<ci-format>] [--top N]
leadline sql-plan --current DIR --baseline DIR [--max-cost-increase-percent N] [--max-plan-rows-ratio N] [--max-estimate-error-ratio N] [--json] [--format agent-json|sarif|<ci-format>] [--top N]
leadline coupling TARGET [--path ROOT] [--top N] [--min-cochanges N] [--json] [--format agent-json]
leadline dependencies [PATH] [--json] [--format agent-json]
leadline unused [PATH] [--json] [--format agent-json] [--entry PATTERN]... [--include-tests]
leadline report --from FILE --format NAME [--cognitive N] [--cyclomatic N] [--crap N] [--max-nesting N]
leadline impact TARGET [--path ROOT] [--top N] [--json] [--format agent-json]
leadline test-targets [PATH] (--coverage FILE | --lcov FILE | --jacoco FILE) [--top N] [--format agent-json]
leadline baseline [PATH] --output FILE [--lcov FILE] [--jacoco FILE] [--coverage FILE]
leadline --version
leadline version
leadline doctor [PATH]
leadline update [--integrations]
leadline mcp [--port [N] [--host ADDR]]
leadline stats [PATH] [--port [N]] [--host ADDR] [--open] [--lcov FILE | --jacoco FILE | --coverage FILE] [--since 30d|90d|365d] [--target REV] [--pit FILE | --stryker FILE | --test-map FILE] [--snapshots FILE] [--include-authors | --anonymize-authors | --exclude-git-identities]
leadline skill
```

- `analyze`: all supported functions under `PATH` (default `.`), or one file. Output sorted by path, functions in source order.
- `function`: functions named `NAME` in `FILE` only. Errors when the name is absent. `--explain` appends one terminal line per metric contribution (`<rule> line <line> nesting <nesting> +<cog> cog +<cyc> cyc`); both JSON shapes already carry `contributions`, and `--format agent-json` includes the array only with `--explain`.
- `check`: like `analyze`, but keeps only violations and parse errors. Accepts `--sarif FILE`/`--baseline-sarif FILE` (repeatable) with `--fail-on-severity low|medium|high|critical`, `--new-only`, and `--changed-only`; `--osv`/`--trivy` with `--baseline-osv`/`--baseline-trivy`; and `--sql` with `--sql-fail-on-severity`. With `--sarif` input, check JSON gains `security_violations`; with `--osv`/`--trivy` it gains `vulnerability_violations`; with `--sql` it gains `sql_violations` — failing when any family fails. It requires at least one threshold flag, `--regressions`, or a scanner/SQL input. With `--base REV`, gates changed functions between the revision and the working tree. With `--baseline FILE`, gates current functions against a saved snapshot instead (exclusive with `--base`): paired functions fail on absolute or (with `--regressions`) delta violations, new functions fail only on absolute thresholds, and deleted functions are ignored. `--regressions` applies configured positive-delta limits to paired functions; added and removed functions are excluded. Absolute and delta gates combine, and either failure exits `1`. `check --base REV` (the `--changed` comparison path) does not use the index; only the plain and `--baseline` paths are warm.
- `index`: build or refresh the analysis index at `--output DIR` (default: the configured `[index].path`, else `.leadline`), printing file counts, analyzed/reused counts, history availability, and — with `--verify` — whether the stored index was reproduced exactly. `--verify` re-derives everything instead of reusing. `PATH` must be a directory.
- `changed` / `diff`: functions changed between a base revision and a comparison target (default the working tree). `changed` takes `--base REV` (default `HEAD~1`); `diff` takes the base revision positionally. `--staged` compares against the index instead of the working tree; `--target REV` compares against another revision instead; the two are mutually exclusive (exit `2`). `--renames` enables Git file rename detection (`-M`), pairing old-path content with new content under the new path. `--explain` adds deterministic multiset-added contribution causes, with after-side lines, to regression rows in agent JSON. Both commands accept `--path` to scope to a file or directory.
- `hotspots`: rank files by `max cognitive complexity x changes in the selected window` and expose every dimension (complexity, CRAP, coverage, churn, contributors, age). Git history is read with one streamed `git log --relative` walk; recency windows are relative to the HEAD commit time, not the wall clock (deterministic). A directory without Git still ranks by complexity and reports `git_available: false`. See `docs/hotspots.md` for formulas and limitations. `--limit N` (default 10) caps rows; `--since 30d|90d|365d` selects the window (default `90d`); coverage flags merge LCOV/JaCoCo before the join.
- `risk`: rank files by the explainable `change-risk` score with exposed components (complexity 20, CRAP 15, churn 20, impact 20, ownership concentration 10, policy 15). Unknown components stay `null` and the score renormalizes over the known weights; rows sort by score desc, path asc. Windows are HEAD-relative like `hotspots`; without Git, churn and ownership are `null` with `git_available: false`. Informational only (exit `0`); regression gating lives in `debt --fail-on-regression`. See `docs/risk.md` for formulas and limitations. `--limit N` (default 10, `N >= 1`) caps terminal and agent-JSON rows (`--json` is always the full ranking); `--since 30d|90d|365d` selects the window (default `90d`); coverage flags merge LCOV/JaCoCo before the join.
- `project`: build the canonical Project across every analytics section (files, functions, dependencies, Git activity, coupling, ownership, duplication, policy, risk, optional mutation/test/trend sections) and print a summary, `--json`, or compact `--format agent-json`. `--target REV` analyzes a revision, `--pit`/`--stryker`/`--test-map` merge external reports, `--snapshots FILE` attaches trend points, `--lcov`/`--jacoco`/`--coverage` merge coverage into the function rows (and therefore CRAP), and the ownership flags are mutually exclusive.
- `debt`: compare complete before/after states. Function debt classifies per configured threshold as `new`, `existing`, or `resolved`; unknown metrics are counted, never guessed. Risk rows pair by path (with `--renames`) and expose score/component deltas. `--staged` and `--target` are exclusive. Informational (exit `0`); `--fail-on-regression` exits `1` on new debt or increased risk.
- `snapshot`: capture one HEAD-tree trend point (never dirty worktree state) to an append-only store. Identical keys and fingerprints are idempotent; changed inputs for an existing key require `--replace`; a sibling lock guards concurrent writers, and writes are atomic. Missing HEAD exits `3`.
- `mutation`: normalize PIT/Stryker reports (and optional explicit test maps) against the analyzed sources. At least one input is required. Rows carry provider provenance, strict paths, 1-based half-open spans, optional unique innermost function IDs, and reasons for unresolved rows. Unsupported schemas and unreadable inputs exit `4`.
- `duplication`: detect `tokens` clones for the current state, or group-level `new`/`existing`/`resolved` drift with `--base REV` (rename-mapped) plus added/removed occurrence counts. Exceeding the token or comparison ceiling exits `3` with `complete: false`.
- `sql-plan`: compare checked-in PostgreSQL `EXPLAIN (FORMAT JSON)` directories query by query (filename stem is the query ID). Requires `--current DIR` and `--baseline DIR`. Limits must be finite and non-negative (exit `2` otherwise); malformed plans exit `4`. Index-to-sequential regressions always fail; numeric dimensions fail past their limit; sort/join changes fail only beside a numeric failure in the same query. New/removed queries never fail. `--json` is complete, `--format agent-json --top N` (default 50) bounds changed queries with `truncated`, `--format sarif` emits one `postgresql-plan/{kind}` result per violation. Any violation exits `1` with the report retained. See `docs/postgresql-plans.md` for artifact rules and formulas.
- `unused`: report files no entry point reaches, `dependencies` entries no source file of their manifest imports, and JS/TS exports no import or re-export covers. Entry points come from `--entry` patterns, `[unused] entries`, the source-naming fields of every discovered `package.json`, and the conventions (`index.*`/`main.*` at the root, under `src/`, and inside any directory that declares a `package.json`, `*.config.*` files, which tooling loads by name, the Rust crate roots `lib.rs`, `main.rs`, `build.rs`, and every `.rs` file under a `bin`, `benches`, `examples`, or `tests` directory, and every C or C++ translation unit `.c`, `.cpp`, `.cc`, or `.cxx`, which a build compiles rather than includes, and the Python modules a host runs or loads by name: `__main__.py`, `manage.py`, `conftest.py`, `setup.py`, `wsgi.py`, and `asgi.py` anywhere, plus any module whose top level holds an `if __name__ == "__main__":` guard, which `python mod.py` and `python -m pkg.mod` execute directly; a package's `__init__.py` is *not* a convention entry point, so a package whose submodules are imported by path can show its `__init__.py` as unused, because CPython loads it implicitly and the graph holds no edge for that load, and every Go file declaring `package main`, which `go build ./...` and `go run ./cmd/x` name directly). Test files are excluded from candidates unless `--include-tests` is passed. Informational (exit `0`): dynamic access, computed property names, and framework conventions can hide a use, so rows are candidates, not proof — the report says so and sets `complete: false` with `reason: "unresolved_references"` whenever the graph could not resolve a reference.
- Zig `unused` conventions are `build.zig` at any directory depth and exact
  `src/main.zig`, `src/lib.zig`, and `src/root.zig` paths at the analysis root
  or under an exact `/src/...` suffix. Arbitrary `.zig` files are not convention
  entries. Zig graph support resolves only exact local `@import` `.zig` paths;
  package imports and build execution are outside the graph.
- `report --from FILE --format NAME`: re-render a saved `check --json` document through any CI format without re-analyzing, so one analysis serves every surface. The saved document carries metrics but not the reasons a row failed, so pass the same threshold flags the gate used; the output is then byte-identical to a live run with those thresholds. A document that is not a check report exits `2`.
- `policy`: evaluate `[[architecture.rules]]` over high-confidence resolved edges; `--base REV` adds `new`/`existing`/`resolved` drift with rename-mapped endpoints. Informational; `--fail-on-violation` exits `1` when current error-severity violations exist.
- `security`: triage scanner SARIF with function, risk, and changed-code context (see `docs/security-findings.md`). Requires at least one `--sarif`; `--baseline-sarif` sets new/existing state, `--base`/`--staged`/`--target` (exclusive) set `changed`, `--fail-on-severity` gates with `--new-only`/`--changed-only` narrowing the gate only. Malformed SARIF exits `4`.
- `vulnerabilities`: prioritize vulnerable dependencies from OSV-Scanner/Trivy reports with changed-import evidence (see `docs/vulnerabilities.md`). Requires at least one `--osv`/`--trivy`; `--baseline-osv`/`--baseline-trivy` set new/existing state, `--base`/`--staged`/`--target` (exclusive) feed direct-import evidence, `--fail-on-severity` gates (or `[vulnerabilities] minimum_severity`). Malformed reports exit `4`.
- `sql`: flag static PostgreSQL query risks in `.sql` files and host-language call sites (see `docs/postgresql-risks.md`). Malformed SQL exits `4`; `--fail-on-severity` gates, otherwise informational.
- `coupling`: list files that repeatedly change in the same commits as `TARGET` (a file under the scope), ranked by directional coupling. Exposes `co_changes`, `commits`, directional, reverse-directional, and Jaccard values; commits wider than 50 files do not create pairs, and `--min-cochanges N` (default 2) suppresses one-off coincidences. `TARGET` must exist; use `--path ROOT` to set the analysis scope (default `.`). A directory without Git reports `git_available: false`. See `docs/coupling.md` for formulas and limitations.
- `dependencies`: report the static file-level dependency graph under `PATH` (default `.`): edges (`source` imports `target`), per-file fan-in/fan-out, unresolved imports with reasons, and import cycles. Honors `leadline.toml` `[analysis] exclude`. See `docs/dependencies.md` for the supported import forms and limitations.
- `impact`: report the transitive dependents of `TARGET` (a file under the scope) by reverse-BFS over the dependency graph: distances, direct dependents, blast radius, and the cycles containing the target. `TARGET` must exist under `--path ROOT` (default `.`); a missing file, a path outside the scope, or a file with no graph node is a usage error (exit `2`). `--top N` (default 20, `N >= 1`) caps the shown dependents; `blast_radius` still counts every dependent and `truncated` signals the cap. See `docs/dependencies.md` for definitions.
- `test-targets`: rank functions holding decision lines with known zero line-coverage hits, sorted by CRAP descending, then path/function/line. Coverage is required (exit `2` without it). Rows carry `uncovered` (known-zero-hit) and `unknown` (absent from the coverage record) contribution lines; unknown is never called uncovered. Output is capped at `--top N` (default 200). Test-targets ranks line-coverage gaps only; branch data feeds function coverage/CRAP (see README Coverage limits).
- `baseline`: snapshot current function metrics to `FILE` (writes atomically via a sibling temp file, then rename). Review the file, commit it, and gate later edits with `check --baseline FILE --regressions`. Snapshot rows pair with current functions by path and name in same-name source order; parsing rejects unknown schemas, unknown metric profiles, and duplicate identities (exit `3`).
- `--version` / `-V` / `version`: print `leadline <version>`.
- `--help` / `-h` / `help`: print the usage text. `--help` is also accepted after any command.
- `doctor`: self-check parsers, coverage readers, `git`, and `leadline.toml`.
- `update`: replace the running binary with the latest GitHub release. Reads the release `VERSION`, downloads the platform archive, verifies it against the release `SHA256SUMS`, and replaces the running executable (atomic rename on Unix, rename-swap on Windows). `LEADLINE_BASE_URL` points at a mirror. Never automatic, no `--json`/`--format`, never exposed over MCP; download, verification, or extraction failure exits `3` and leaves the installed binary untouched. With `--integrations`, it then refreshes detected Pi, OMP, and Claude Code integrations through each harness's own CLI (`pi update git:github.com/jbt95/leadline`, `omp plugin install git:github.com/jbt95/leadline --force`, `claude plugin update leadline@leadline`); OpenCode local plugin paths are reported with `opencode2 service restart` guidance and never modified. Integration failures do not stop later updates and exit `3`.
- `mcp`: serve the read-only MCP tool API over stdio (default; request lines are bounded at 32 MiB); the only writes are the opt-in local metrics store, confined to `LEADLINE_METRICS_DIR` ([telemetry.md](telemetry.md)). `--port [N]` serves the same tools over HTTP instead (`POST /mcp`, plus `GET /health` for status): a bare `--port` means 3000, `0` asks the OS for a free port, and a taken port falls back to a free one with the actual address printed to stderr. `--host ADDR` sets the bind address (default `127.0.0.1`) and requires `--port`, because it configures only the HTTP transport. HTTP mode bounds every request (bounded header lines, whole-request read and write deadlines, a 64 MiB in-flight body budget, a fixed worker ceiling that answers 503 when saturated) and rejects non-loopback browser `Origin` headers with 403 per the MCP Streamable HTTP spec; clients that send no `Origin` are unaffected. MCP tools read `leadline.toml` from the analysis root, so `[analysis].exclude`, `[sql]`, and `[vulnerabilities]` behave exactly as they do on the CLI.
- `stats`: analyze once and serve the canonical `Project` over loopback with an embedded, read-only page. `PATH` defaults to `.`, and every analysis flag is the one `project` parses (`--json` and `--format` are rejected with a usage error pointing at `/api/project`). `--port` defaults to 3000 (a bare `--port` means 3000, `0` asks the OS for a free port, and a taken port falls back to a free one with a note on stderr); `--host ADDR` defaults to `127.0.0.1` and does not require `--port`; `--open` launches the default browser at the served URL, best effort. The URL line goes to **stderr** (`leadline: stats on http://127.0.0.1:3000`), so stdout stays clean for pipelines. Exit codes follow the CLI contract: `2` usage, `3` incomplete analysis, `4` input error. See the endpoint table below.
- `skill` / `--skill`: print the canonical agent skill (`integrations/common/leadline-skill/SKILL.md`, baked into the binary).

`stats` endpoints (loopback-bound, read-only; unknown paths are `404`, unsupported methods on a known path are `405`, and a failed refresh answers `500` with the previous snapshot still served):

| Method | Path | Response |
| --- | --- | --- |
| GET | `/` | the embedded page (React dashboard built from `web/`, checked in under `web/dist`) |
| GET | `/assets/app.js`, `/assets/index.css`, `/logo.svg` | embedded assets (`/logo.svg` is the shipped `assets/logo.svg`, included at build time) |
| GET | `/api/project` | canonical `Project` JSON, byte-identical to `project --json` |
| GET | `/api/telemetry` | local telemetry store snapshot (`ok` with counters/summaries/gauges, `disabled` when `LEADLINE_METRICS_DIR` is unset, or `error` when the store exceeds 1 MiB); see [telemetry.md](telemetry.md) |
| POST | `/api/refresh` | `200` with fresh `meta` once the re-analysis finishes on that connection; `409` when one is already running |
| GET | `/health` | `{"status":"ok","analyzed_at":…,"head_commit":…,"analyzer_version":…}` |

## Flags

| Flag | Commands | Meaning |
| --- | --- | --- |
| `--json` | all analysis | Emit JSON report instead of terminal text. Exclusive with `--format agent-json` and `--format sarif`; a CI format renders instead of JSON. |
| `--format agent-json` | analyze, function, check, changed, diff, hotspots, risk, coupling, dependencies, unused, impact, test-targets, project, debt, sql-plan, security, vulnerabilities, sql | Emit the compact agent-oriented JSON shape. Function-shaped views (`analyze`, `function`, `check`) carry per-file `parse_errors`, and `changed`/`diff` carry per-file before/after `parse_errors`, so consumers can tell an unparsed file from a clean empty result. Carries a `truncated` bool when budget flags drop entries — except `risk`, whose agent JSON has no `truncated` field (`--limit` caps its rows silently). |
| `--format sarif` | analyze, check, security, vulnerabilities, sql, sql-plan | Emit SARIF 2.1.0 (`version` / `runs` / `results`). `check` emits its gate violations only, so results match its exit code; the scanner commands emit their full report. Exclusive with `--json`. |
| `--format <ci-format>` | check, security, vulnerabilities, sql, sql-plan | Render findings for CI platforms: `codeclimate` (alias `gitlab-codequality`) for GitLab Code Quality, `github-annotations` for workflow-command annotations, `github-summary` and `markdown` for job summaries and pull-request bodies, `badge` for a shields.io-compatible SVG showing the gate verdict and the violation count, and `compact` for one grep-friendly line per finding. Takes precedence over `--json` and `--format agent-json|sarif`; `check` renders its gate violations, the scanner commands their full report. `badge` carries no composite score: a single number invites refactoring to move it. |
| `--top N` | analyze, function, check, changed, diff, coupling, impact, test-targets, security, vulnerabilities, sql, sql-plan | Keep at most `N` entries per list (`coupling` and `impact` default 20, `test-targets` defaults 200, `sql-plan` defaults 50). Only with `--format agent-json` (`N >= 1`), except `coupling`, `impact`, and `test-targets` where it also caps terminal/JSON output. |
| `--sort-by KEY` | analyze, function, check, changed, diff | Sort budget entries by `crap`, `cognitive`, or `cyclomatic`. Only with `--format agent-json`. |
| `--min-crap X` | analyze, function, check, changed, diff | Drop entries below CRAP `X`. Only with `--format agent-json`. |
| `--min-delta D` | changed, diff | Drop entries whose max before/after delta is below `D`. Only with `--format agent-json`. |
| `--explain` | function, changed, diff | Add contribution details to function agent JSON, or multiset-added causes to changed regression rows. |
| `--index DIR` | analyze, check | Reuse content-keyed per-file analysis and HEAD-keyed Git history facts from `DIR/index.json` across runs; also settable per repository with `[index] path`. Bypassed when any coverage flag is passed (coverage merges after analysis, so indexed CRAP would go stale). Any version, configuration, or scope mismatch falls back to a full analysis; index write failures warn on stderr, never fail. |
| `--limit N` | hotspots, risk | Keep at most `N` ranked files (`N >= 1`, default 10). On `hotspots` caps terminal, JSON, and agent JSON (`truncated` signals the cap); on `risk` caps terminal and agent JSON only — `--json` is always the full ranking with no `truncated` field. |
| `--since WINDOW` | hotspots, risk, project, debt, snapshot | Rank changes over `30d`, `90d`, or `365d` (default `90d`). |
| `--lcov FILE` | analyze, function, check, hotspots, risk, project, test-targets, baseline | Merge LCOV line coverage. Repeatable. Required on `test-targets` (one coverage flag at minimum). |
| `--jacoco FILE` | analyze, function, check, hotspots, risk, project, test-targets, baseline | Merge JaCoCo XML line coverage. Repeatable. Required on `test-targets` (one coverage flag at minimum). |
| `--coverage FILE` | analyze, function, check, hotspots, risk, project, test-targets, baseline | Merge coverage with format detected from the extension (`.info` LCOV, `.xml` JaCoCo). Repeatable. Required on `test-targets` (one coverage flag at minimum). |
| `--regressions` | check | Enable regression-only gates. Limits come from `[regressions]` and default to zero. It removes the absolute-threshold requirement; combine with `--base REV` or `--baseline FILE` to compare changed functions. |
| `--base REV` | check, changed, debt, security, vulnerabilities | Base revision (`changed` default `HEAD~1`). Exclusive with `--baseline` on `check`. |
| `--baseline FILE` | check | Saved snapshot for regression gates without Git. Exclusive with `--base`. |
| `--output FILE` | baseline, snapshot, index | Destination (required): the snapshot file on `baseline`/`snapshot`, the index directory on `index` (default the configured `[index].path`, else `.leadline`). |
| `--staged` | changed, diff, debt, security, vulnerabilities | Compare `--base`/`REV` against the index (staged blobs) instead of the working tree. Exclusive with `--target`. |
| `--target REV` | changed, diff, debt, security, vulnerabilities | Compare `--base`/`REV` against another revision instead of the working tree. Exclusive with `--staged`. |
| `--renames` | changed, diff, debt | Detect Git file renames (`-M`) and pair a renamed file's old content with its new content under the new path. |
| `--path PATH` | changed, diff, debt, coupling, impact | Scope changed analysis to a file or directory; on `coupling` and `impact` it sets the history/graph scope (default `.`). |
| `--min-cochanges N` | coupling | Drop related files with fewer than `N` shared commits (default 2, minimum 1). |
| `--cognitive N` | check, report | Fail functions whose cognitive complexity exceeds `N`; on `report` it re-applies the gate limit to a saved document. |
| `--cyclomatic N` | check, report | Fail functions whose cyclomatic complexity exceeds `N`; on `report` it re-applies the gate limit to a saved document. |
| `--crap N` | check, report | Fail functions whose CRAP score exceeds `N`; functions without coverage also fail. On `report` it re-applies the gate limit to a saved document. |
| `--max-nesting N` | check, report | Fail functions nested deeper than `N`; on `report` it re-applies the gate limit to a saved document. |
| `--sarif FILE` | security, check | Scanner findings input (at least one required on `security`). Repeatable. |
| `--baseline-sarif FILE` | security, check | Previous findings for new/existing state with `--new-only`. |
| `--fail-on-severity LEVEL` | security, vulnerabilities, sql, check | Gate floor: `low`, `medium`, `high`, or `critical`. |
| `--new-only` | security, check | Gate only findings absent from `--baseline-sarif`. |
| `--changed-only` | security, check | Gate only findings in changed code. Requires a comparison: `security` takes `--base`, `--staged`, or `--target`; `check` takes `--base`. Without one every finding is unchanged, so the combination is a usage error (exit `2`) instead of a silent pass. Changed paths include non-source files (`.env`, YAML, JSON, shell scripts). |
| `--osv FILE` | vulnerabilities | OSV-Scanner report input (at least one of `--osv`/`--trivy` required). Repeatable. |
| `--trivy FILE` | vulnerabilities | Trivy report input. Repeatable. |
| `--baseline-osv FILE` | vulnerabilities | Previous OSV report for new/existing state. |
| `--baseline-trivy FILE` | vulnerabilities | Previous Trivy report for new/existing state. |
| `--sql` | check | Merge `sql` static risks into the gate (gated by `--sql-fail-on-severity`). |
| `--sql-fail-on-severity LEVEL` | check | Gate floor for the `--sql` family. |
| `--large-offset N` | sql | Numeric top-level `OFFSET` above `N` fails (default 1000). |
| `--migration-root DIR` | sql | Migration directory declaring tables (overrides config). Repeatable. |
| `--current DIR` | sql-plan | Current EXPLAIN artifact directory (required). |
| `--baseline DIR` | sql-plan | Baseline EXPLAIN artifact directory (required). |
| `--max-cost-increase-percent N` | sql-plan | Fail queries whose total cost grows past `N` percent. |
| `--max-plan-rows-ratio N` | sql-plan | Fail queries whose plan rows grow past ratio `N`. |
| `--max-estimate-error-ratio N` | sql-plan | Fail queries whose estimate error grows past ratio `N`. |
| `--pit FILE` | project, mutation | PIT report input. Repeatable. |
| `--stryker FILE` | project, mutation | Stryker report input. Repeatable. |
| `--test-map FILE` | project, mutation | Explicit test-to-code map. Repeatable. |
| `--snapshots FILE` | project | Trend points to attach. |
| `--include-authors` / `--anonymize-authors` / `--exclude-git-identities` | project | Identity handling; mutually exclusive. |
| `--fail-on-violation` | policy | Exit `1` when error-severity violations exist. |
| `--fail-on-regression` | debt | Exit `1` on new debt or increased risk. |
| `--replace` | snapshot | Overwrite the trend point when inputs changed for an existing key. |
| `--entry PATTERN` | unused | Entry-point pattern, gitignore-style and analysis-root-relative. Repeatable; adds to `[unused] entries`. |
| `--include-tests` | unused | Analyze test files as ordinary candidates instead of leaving them out. |
| `--verify` | index | Re-derive every file instead of reusing stored analysis, and report whether the stored index was reproduced exactly. |
| `--integrations` | update | After replacing the binary, refresh detected Pi, OMP, and Claude Code integrations through each harness's own CLI. |
| `--port [N]` | mcp, stats | Serve HTTP instead of stdio on `mcp`; on `stats` the flag only sets the port. Bare means 3000, `0` asks the OS, taken ports fall back free with a note on stderr. |
| `--host ADDR` | mcp, stats | Bind address (default `127.0.0.1`). `mcp` requires `--port` with it, because it configures only the HTTP transport; `stats` does not. |
| `--open` | stats | Launch the default browser at the served URL, best effort; the browser never opens without the flag. |

Budget flags with `--json` or default terminal output are a usage error (exit `2`), never silently ignored. `--top 0` and unparsable budget values are usage errors.

`check` thresholds fail only when a value exceeds its limit. A `--crap` threshold also fails when coverage (and therefore CRAP) is unavailable for a function.

`--format sarif` on `check` uses the command thresholds; on `analyze` it uses `leadline.toml` thresholds when present, else empty thresholds (rules listed, empty results).

## Local metrics

`LEADLINE_METRICS_DIR` enables opt-in local metrics: every CLI invocation and
MCP tool call updates a bounded store and a Prometheus text file
(`leadline.prom`) in that directory for Grafana Alloy or any text-format
scraper. Counters, durations, and fixed labels only; nothing is sent over the
network and nothing identifies a repository, path, argument, or person. Unset
or empty disables recording entirely. See [telemetry.md](telemetry.md) for the
metric reference and the Grafana setup. Metrics are deliberately not
configurable through `leadline.toml`: repository configuration is strict,
fingerprinted for index reuse, and parsed from historical revisions, so
runtime preferences live in the environment.
