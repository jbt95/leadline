# Changelog

All notable changes use this file. Version numbers follow Semantic Versioning.

## Unreleased

### Changed

- Dependency extraction and duplication tokenization run in parallel across
  files, and duplication interns token ids in a hash map. Output is unchanged.
- JSON output streams directly to stdout instead of materializing the full
  encoded document first, lowering peak memory on large reports.

## 0.13.2 - 2026-09-18

### Fixed

- Local metrics no longer drop samples under concurrent invocations: the
  metrics store lock now blocks briefly instead of giving up after 80ms,
  fixing the flaky `concurrent_invocations_are_not_lost` failure.

## 0.13.1 - 2026-09-18

### Fixed

- Update integration tests retry the test-binary exec when the kernel
  reports it busy (`ETXTBSY`) under parallel load on Linux, fixing the
  flaky `plain_update_with_a_new_release_does_not_probe_harnesses` failure.

## 0.13.0 - 2026-09-18

### Added

- OpenCode native plugin (`integrations/opencode/plugin-v2`) emits warn-mode
  post-edit feedback automatically after successful `edit`/`write` calls via
  the shared `postEditFeedback` adapter; silent on clean runs and analyzer
  failures, never blocks edits.
- The post-edit hook skips analysis when the edited file is definitely
  outside Leadline's scope (Java/JS/TS/TSX); unknown tool input shapes fall
  through to analysis.

## 0.12.0 - 2026-09-18

### Added

- `leadline update --integrations`: after the binary update, probe installed
  Pi, OMP, and Claude Code integrations and refresh each through its own CLI;
  OpenCode local plugin paths are reported with restart guidance and never
  modified. Failures do not stop later updates and exit `3`.

### Changed

- Duplication comparison groups candidates by exact token-id run instead of
  formatting a joined-text key per comparison and indexes occurrences per
  file; clone-heavy repositories report substantially faster.

- `debt` classifies findings from the before/after analyses it already built
  instead of loading and re-analyzing both sides a second time, so
  classification and Project views on each side come from the same snapshot.

### Fixed

- Duplication missed repeated token runs at different offsets because the
  rolling-hash slide removed the outgoing token with an off-by-one power.
  Detection now matches every aligned window, so clone reports include
  previously invisible groups.

## 0.11.0 - 2026-09-17

### Added

- Telemetry v2: invocation durations render as a Prometheus histogram with
  fixed buckets (p50/p90/p99 via `histogram_quantile`), duration rows carry
  `outcome`, scanner findings break down by `severity`
  (`leadline_security_findings_total`), parse errors break down by `language`
  (`leadline_parse_errors_total`), and MCP `check`/`debt` calls record
  findings like CLI ones. Store schema is now `2`; existing stores reset
  once on first write.

### Changed

- Agent-facing wording: the MCP `initialize` instructions are trigger-first
  (when to call each tool across edit, review, triage, and SQL/security
  workflows), and the native OpenCode and Pi/OMP tool descriptions state
  their call triggers. No tool, input, or output shape changes.

## 0.10.1 - 2026-09-17

### Fixed

- Release publishing: create the GitHub release before uploading assets and
  retry asset uploads with backoff, so a transient `uploads.github.com`
  500 no longer fails the whole release and re-runs resume with `--clobber`.

## 0.10.0 - 2026-09-17

### Added

- Opt-in local metrics: with `LEADLINE_METRICS_DIR` set, every CLI
  invocation and MCP tool call updates a bounded, schema-versioned store plus
  a Prometheus text file (`leadline.prom`) that Grafana Alloy's textfile
  collector or any text-format scraper can read. It records fixed-label
  counts only — invocations by surface/operation/outcome, per-operation
  durations, `check` findings by kind (function, parse error, and the
  scanner families), `debt` new/resolved/risk counts, and the standing
  `existing` function count — never paths, arguments, source, findings,
  commits, or identities. Recording is best-effort and never changes output
  or exit codes, and MCP writes go only to the configured metrics directory.
  See `docs/telemetry.md`.

- Optional TypeSafe triage companion (`integrations/typesafe-triage/`): six
  advisory features over leadline JSON reports — changed/regression triage,
  repo-wide `check` backlog triage, security/vulnerability triage, debt
  acceptance review, duplication intent triage, and workflow routing. It is a
  separate TypeScript tool with no runtime dependencies; leadline itself stays
  offline and MCP stays network-free. Judgments are composed with
  deterministic weights and confidence gates, and the output is advisory: it
  never gates, changes an exit code, or suppresses a finding. The package
  vendors the MIT anti-slop Oxlint rules and enforces them in CI.

### Changed

- Agent guidance for cheaper sessions: the skill and the MCP server
  instructions now tell agents to batch independent calls in one block, read
  each file once per task, and start from a broad call (`analyze --top`,
  `hotspots`, `risk`, `repo_summary`, `project`) before per-function drills.

## 0.9.0 - 2026-09-16

### Added

- MCP exposes nine more read-only tools mirroring the CLI analytics commands:
  `hotspots`, `risk`, `dependencies`, `impact`, `coupling`, `duplication`,
  `policy`, `debt`, and `project`. Arguments are validated as strictly as the
  existing tools; `risk`'s pipeline and scoped-target resolution for
  `coupling`/`impact` are shared with the CLI.

### Fixed

- MCP tool arguments are validated strictly: `analyze_changed.explain`,
  `analyze_changed.renames`, and `analyze_function.explain` reject a
  non-boolean value with `-32602` instead of silently treating it as `false`,
  and `repo_summary` rejects `top` above 50 instead of clamping it.
- The worktree secret gate preflights the git HEAD it needs to diff against
  before scanning: a directory that is not a repository, or a repository with
  no commits, skips the gate with a visible non-blocking diagnostic instead
  of running a full-tree `gitleaks` scan and then failing with a raw
  `fatal: not a git repository` from the changed-path comparison. The TS
  adapters report the runner's exit `3` as `unavailable` instead of throwing.

## 0.8.2 - 2026-09-16

### Fixed

- The secret-gate hook-wrapper test retries Linux's transient `ETXTBSY`
  fork race, so the Ubuntu CI lane no longer flakes when a sibling test
  thread holds a just-copied script's write descriptor.

## 0.8.1 - 2026-09-16

### Fixed

- The Claude Code plugin now vendors the shared secret-gate runner
  (`integrations/claude-code/common/`), so a marketplace install resolves it
  without `LEADLINE_SECRET_RUNNER`; a drift test keeps the copy identical to
  `integrations/common/leadline-secret-check.sh`.
- The Claude/Gemini/Cline secret-gate wrapper only blocks on findings
  (runner exit 1 → hook exit 2). An unavailable runner or scanner, a scan
  failure, or a bad mode now exits 1 with the redacted diagnostics on
  stderr, so a missing `gitleaks` or a packaging miss warns visibly instead
  of trapping a Stop hook in a blocking loop.

## 0.8.0 - 2026-09-16

### Fixed

- `sql_risks` no longer reads a foreign-key `ON DELETE` clause as an unbounded
  `DELETE` statement, and `ALTER TABLE ... RENAME TO` targets count as declared
  tables, so references after a rename stop reporting `sql/unknown-table`.
- `check` violation rows now carry a `reason` array naming the failed
  thresholds; a CRAP gate with no coverage record reports `crap_unavailable`
  instead of an unexplained `crap: null`. `repo_summary` accepts `coverage` and
  omits the CRAP list without it, and every tool rejects unknown arguments and
  unknown `thresholds`/`regressions` keys instead of silently ignoring them.
- `leadline check` validates thresholds after loading `leadline.toml`, so a
  `[thresholds.function]` section alone satisfies the gate instead of erroring
  and (in the native adapters) falling back to hardcoded defaults. A
  config-only `check` invocation therefore exits `0`/`1` instead of `2` (CLI
  schema 1.5).

### Changed (breaking)

- The native OpenCode/Pi/OMP wrapper tool `leadline_check` is renamed
  `leadline_gate`: harnesses that expose the `leadline` MCP server namespace a
  `check` tool as `leadline_check`, which silently shadowed the native tool in
  OpenCode V2. Integration docs and READMEs use the new name.

## 0.7.1 - 2026-09-16

### Fixed

- Claude Code plugin manifest version now tracks the release version, so
  `claude plugin update` detects new releases instead of reporting an
  unchanged 0.1.0.

## 0.7.0 - 2026-09-16

### Added

- Persistent analysis index: `leadline index [PATH] [--output DIR] [--verify] [--json]`
  builds content-keyed file metrics and HEAD-keyed Git history facts under
  `.leadline/` (configurable with `[index] path`). Warm `analyze` and `check`
  output is byte-identical to a cold run; `--verify` re-derives the index and
  reports whether the stored one was reproduced exactly.
- Warm `analyze` and `check` through `--index DIR` or the configured
  `[index].path`, and read-only index reuse in the MCP `analyze` and `check`
  tools, which never create or modify an index.
- `cargo bench --offline --bench index` measuring cold analysis against warm
  refresh on a synthetic repository.

### Fixed

- MCP guidance: `check` errors name the correct `thresholds`/`regressions` form, `security_findings` rejects an empty `sarif` list, and tool descriptions state coverage limits, `min_delta` churn filtering, pre-generated scanner files, and native `leadline_secret_check` for secrets.

### Changed (breaking)

- `--cache-dir DIR` on `analyze` and `check` is replaced by `--index DIR`. The
  index stores content-keyed file metrics plus HEAD-keyed Git history facts and
  is enabled per repository with a new `[index] path` section. Warm `analyze`
  and `check` output is byte-identical to a cold run; any version,
  configuration, or scope mismatch falls back to a full analysis.

## 0.6.1 - 2026-09-16

### Fixed

- Windows checksum verification accepts GNU `sha256sum`'s escaped output marker for paths containing backslashes.
- MCP artifact arguments accept Windows path separators while still rejecting absolute paths and parent-directory escapes.

## 0.6.0 - 2026-09-15

### Added

- Self-update: `leadline update` reads the latest release `VERSION`, downloads the platform archive, verifies it against the release `SHA256SUMS`, and replaces the running binary (atomic rename on Unix, rename-swap on Windows) with no automatic checks and no new dependencies. See `docs/installation.md`.
- Secret detection gates: a hardened `integrations/common/leadline-secret-check.sh` runner delegates to installed `gitleaks` (the only detector, always `--redact`) with a versioned pre-commit hook, Claude/Gemini/Cline worktree wrappers, and `runSecretGate` TypeScript adapters with Pi/OpenCode `leadline_secret_check` registration. See `docs/agent-integration-guide.md` and `integrations/COMPATIBILITY.md`.
- Static PostgreSQL risk analysis: `leadline sql [PATH]` flags seven fixed rules (unbounded updates, leading wildcards, nonsargable predicates, large offsets, unknown tables, dynamic concatenation, queries in loops) with config, gates, `check` parity, and a read-only `sql_risks` MCP tool. See `docs/postgresql-risks.md`.
- Vulnerable dependency prioritization: `leadline vulnerabilities [PATH] --osv/--trivy FILE` normalizes OSV-Scanner and Trivy advisories, marks changed direct-import evidence (Node builtins such as `node:fs` stay out of the npm mapping), and gates new findings with `reachable_from_changed: false` suppression, plus `check` parity and a read-only `vulnerabilities` MCP tool. See `docs/vulnerabilities.md`.
- Security finding enrichment: `leadline security [PATH] --sarif FILE` ingests SARIF 2.1.0 results and enriches them with innermost-function, risk, and changed-code context, with `--fail-on-severity`/`--new-only`/`--changed-only` gates, terminal/JSON/agent-JSON/SARIF output, `check --sarif` parity, and a read-only `security_findings` MCP tool. See `docs/security-findings.md`.
- PostgreSQL plan regression checks: `leadline sql-plan --current DIR --baseline DIR` compares checked-in `EXPLAIN (FORMAT JSON)` artifacts per query ID, with cost/row/estimate gates, index-to-sequential detection, terminal/JSON/agent-JSON/SARIF output, and a read-only `sql_plan` MCP tool. See `docs/postgresql-plans.md`.
- MCP HTTP transport: `leadline mcp --port [N] [--host ADDR]` serves the same eleven read-only tools over HTTP (`POST /mcp`, `GET /health`) with no new dependencies; stdio stays the default. A bare `--port` means 3000, `0` asks the OS for a free port, and a taken port falls back to a free one with the actual address printed to stderr. See `docs/cli-reference.md`.
- Smoke suite (`scripts/smoke.sh`, `docs/smoke.md`): end-to-end CLI proof on full clones of babel, elasticsearch, and timescaledb — disjoint from the bench fixtures — covering scale, gates, history, joins, sql, sql-plan, scanners (optional), and MCP transports. Local-only.
- Git history analytics (`leadline::history`): per-file commit counts, 30/90/365-day change windows, lines added/deleted, code age, days since last change, and contributor counts. One streamed `git log --relative` walk with rename resolution; recency windows are relative to the HEAD commit time for deterministic reports; snapshots without Git degrade to `git_available: false` instead of failing.
- `leadline hotspots` with `--limit N`, `--since 30d|90d|365d`, `--json`, `--format agent-json`, and LCOV/JaCoCo coverage flags. Ranks files by `max cognitive complexity x changes in window` (`complexity-x-churn`) and exposes every dimension: complexity, CRAP, coverage, churn, contributors, and age.
- `docs/analytics-roadmap.md`: architecture proposal, normalized Project analytics schema, static report data contract, Git ingestion trade-offs, and milestones B-I. `docs/hotspots.md`: formulas, calculation rules, limitations, and the no-developer-ranking guardrail.
- Git history benchmark target (`cargo bench --bench history`) measuring the raw log walk, end-to-end history analysis, and hotspot scoring separately; CI compiles it alongside the analyzer bench.
- Temporal (change) coupling: `leadline coupling TARGET` lists files that repeatedly change in the same commits, with co-change counts, directional coupling, and Jaccard similarity. One shared streamed history walk; commits wider than 50 files do not create pairs; `--min-cochanges`, `--top`, `--json`, and `--format agent-json` supported. See `docs/coupling.md`.
- Static dependency intelligence (`leadline::graph`, `leadline::impact`): `leadline dependencies [PATH]` reports file-level edges (source imports target), per-file fan-in/fan-out, unresolved imports with reasons, and import cycles; `leadline impact TARGET` reports transitive dependents with shortest distances, direct dependents, blast radius (`blast_radius_percent` on a 0-100 scale), and the cycles containing the target. Resolution covers relative JS/TS imports (including `require()` / `import()` forms, extensionless and emitted-JS mapping) and exact Java type imports (including static-member stripping); bare package imports are ignored and ambiguity stays unresolved with a reason. Both commands support `--json` and `--format agent-json` (with `--top` truncation on `impact`); reports are deterministically ordered and byte-identical across runs. See `docs/dependencies.md`.

### Changed

- Output `schema_version` is now `2`: `check` JSON may carry `security_violations`, `vulnerability_violations`, or `sql_violations` when scanner inputs are passed.
- `--format agent-json` for function-shaped views (`analyze`, `function`, `check`) now carries per-file `parse_errors`, and `changed`/`diff` carry per-file before/after `parse_errors`; consumers must surface them rather than treat an empty function list as clean.
- Parallelized directory discovery and cache-enabled directory analysis. Cache misses reuse the bytes already read for lookup, and unchanged caches are no longer rewritten.
- The OpenCode V1 plugin now delegates to the shared TypeScript adapter (`runChanged`/`runFunction`/`runCheck`/`runSecretGate`), so V1 tools take the same `path` scope, threshold fallback, and output shape as V2/Pi.

### Fixed

- Secret gating saw only supported source files: changed-path enumeration for security attribution now returns every Git path, so a secret in `.env`, YAML, JSON, or a shell script can no longer pass `--changed-only`. Source filtering stays in the parsers. Attribution paths are analysis-root-relative, so a run scoped to `src/` matches `app.py` from a scanner rooted there instead of silently reporting `changed: false`.
- The explicit `leadline_secret_check` tools on Pi, OpenCode V1, and OpenCode V2 no longer report `unavailable` (missing runner or `gitleaks`) as `clean`; the shared formatter surfaces it.
- `--changed-only` without a comparison is a usage error (exit `2`) on `security`, `check`, and the `security_findings` MCP tool instead of a silent gate pass.
- Regression-only `check --format sarif` reports the real before/after deltas and allowed limits, and only for dimensions that regressed: the synthetic zero-threshold projection that fabricated unrelated results is gone.
- `check --format sarif` merges scanner gate violations only, so results never contradict the exit code; standalone scanner commands still emit their full report.
- PostgreSQL plan estimate-error gates compare per-execution `Actual Rows` against `Plan Rows`: multiplying by `Actual Loops` fabricated violations on nested-loop nodes.
- MCP tools read `leadline.toml` like the CLI: `[analysis].exclude` applies to `analyze`, `check`, `repo_summary`, `test_targets`, `sql_risks`, and the host-source walk, `[sql]` supplies `large_offset`/`migration_roots` defaults, and `[vulnerabilities] minimum_severity` gates `vulnerabilities` when the argument is absent.
- HTTP MCP: bounded request lines and header blocks (431 past the limit), socket read/write deadlines, a fixed worker ceiling that answers 503 when saturated, and `Origin` validation (403 for non-loopback browser origins) per the MCP Streamable HTTP transport spec; a dual-stack listener also treats IPv4-mapped loopback peers as loopback for the `Host` rule.
- HTTP MCP read deadlines now cover the whole request: every read is re-armed against one five-second deadline, so a client cannot trickle a declared body forever. Requests must also carry exactly one `Host` and one `Content-Length` (400 otherwise), and loopback-bound listeners only accept loopback `Host` authorities.
- JSON-RPC: empty batches, objects without a `method`, and object/array/boolean ids now answer `-32600` instead of passing silently; batches are capped at 64 requests and every response (batch included) at 32 MiB.
- MCP response bounds are complete: `top` is capped at 200 entries on `analyze`, `analyze_changed`, and `test_targets`, and `security_findings`, `vulnerabilities`, `sql_risks`, and `sql_plan` cap gate `violations` at `top` alongside findings, setting `truncated` when either was cut.
- MCP artifact arguments (`sarif`, `osv`, `trivy`, `sql_plan`'s `current`/`baseline`) reject parent-directory escapes and symlinks that resolve outside the working directory, including a symlinked parent of a missing artifact, not just absolute paths.
- MCP `check` fills missing metrics from `leadline.toml` `[thresholds.function]` and uses configured `[regressions]` limits when `regressions: true`, matching the CLI instead of gating on zero tolerances.
- Scanner inputs charge the aggregate row budget while parsing: SARIF and OSV/Trivy findings (baselines included) can no longer expand a compact report into millions of retained rows.
- SQL analysis rejects inputs past 1,000,000 tokens, supports nested block comments, analyzes explicit `.sql` files, recognizes `CREATE [GLOBAL|LOCAL] [TEMP|TEMPORARY|UNLOGGED] TABLE` declarations, and knows every CTE alias in a statement; E-string backslash escapes never end the literal (a leading `\%` still decodes to a wildcard), scalar-function `FROM` (`extract`, `trim`, `substring`) is not a table reference, and comma-separated `FROM` items and `DELETE ... USING` sources are all checked.
- PostgreSQL plan comparison keys scans by `schema.relation` when EXPLAIN carries `Schema Name`, so same-named tables in different schemas no longer fabricate or hide index-to-sequential regressions.
- PostgreSQL plan `violations` sort by the documented keys — query ID, kind, relation, then detail — so same-query changes have one deterministic order.
- `[sql] migration_roots` rejects Windows drive prefixes (`C:/migrations`) that would become absolute paths.
- Gemini hooks use the current manifest schema with `${extensionPath}`, millisecond timeouts, and a secret-gate wrapper that maps findings, scanner failures, and missing tools to exit `2` (the only blocking status) with redacted diagnostics on stderr, resolving the shared runner from the checkout or the host project directory so a copied extension still gates; the feedback hook no longer copies hook input (prompts, responses, tool payloads) to stderr.
- Agent adapters surface analyzer parse errors instead of an empty "No functions reported", `changed` reports before/after `parse_errors`, Pi/OpenCode analyzer failures all return tool text, Pi's explicit `leadline_secret_check` fails the call on findings or an unavailable scanner, and the secret-runner path survives percent-encoded installs (`fileURLToPath`).
- HTTP MCP buffers at most 64 MiB of request bodies across all connections (further declared bodies get 503) instead of letting 64 clients reserve 32 MiB each.
- HTTP MCP framing is strict: only HTTP/1.1 is accepted (505 otherwise), malformed request or header lines are 400, `Transfer-Encoding` is 501, and empty POST bodies are 400 rather than 202.
- MCP responses stay inside the 32 MiB cap in the single-response fallback and batch accounting, even when a giant request id would otherwise be echoed.
- MCP `migration_roots` keep the CLI's normalizer as the single lexical authority (including its interior-`..` rejection) and are no longer checked against the process working directory, so an unrelated CWD symlink cannot reject a safe analysis-root-relative root.
- MCP `Origin` validation matches its contract again: scheme-less and non-`http(s)` origins are rejected.
- JSON-RPC notifications are dispatched for their side effects without a response, scalar `params` are rejected with `-32602`, and an oversized batch of notifications stays silent instead of drawing an error response.
- HTTP MCP `Content-Length` accepts digits only, header names are validated as RFC 7230 tokens, and a connection closed before the terminating blank line is rejected as truncated instead of read as an empty header line.
- MCP tools reject mis-typed string arguments instead of treating them as absent, so a numeric `minimum_severity` (or `path`, `coverage`, `base`, `target`) is an invalid-params error rather than a silently disabled gate.
- An over-limit JSON-RPC batch now answers with a one-element error array, keeping the batch shape clients parse; a batch of notifications still draws no response at all.
- MCP stdio reads one bounded line per request (32 MiB, matching the HTTP body cap) instead of buffering an unbounded line, and direct tool methods reject array `params` instead of silently falling back to default paths.
- `leadline mcp --host ADDR` without `--port` is a usage error instead of silently starting the stdio server.
- HTTP MCP writes share one whole-response deadline, so a slow reader cannot hold a worker past it; response connections half-close and drain the rest of the request, so an early rejection (such as 431 on an oversized header) cannot reset the response away from a peer that is still writing.
- Security enrichment sets `changed` from the finding's path alone: file-level SARIF results without a region now count for `--changed-only` gating, matching `docs/security-findings.md`.
- Security SARIF reads object-shaped `fingerprints` (SARIF 2.1.0) as well as arrays, so scanner fingerprints classify moved findings against baselines instead of falling back to spans, and artifact URIs are percent-decoded before path validation, so encoded paths keep their attribution, encoded traversals are still rejected, and `file://` authorities are refused.
- The published crate drops the tracked `docs/superpowers/**` planning document, which the gitignore already excludes from the working tree.
- JavaScript/TypeScript logical-operator metrics ignore operators inside nested functions and inside string, template, and JSX text; Java hex/octal/binary integers and floating-point literals normalize to `<num>` for duplication tokens like every other number and count as Halstead operands the same way. Host SQL concatenation checks stop at nested functions, so a callback's arithmetic is never the call's query text.

### Changed (breaking)

- Cyclomatic complexity now follows SonarQube per-language rules: Java no longer counts `catch` and counts each `->` (lambda/switch arrow) in the enclosing function; JavaScript/TypeScript count `throw` and no longer count `??`. Fixture baselines in `tests/fixtures.rs` updated; see `docs/metrics.md`.
- Cognitive complexity counts direct self-recursion (+1 `recursion` contribution); mutual cycles stay unscored. Indirect recursion, `else`-body depth, and lambda-through nesting remain documented Sonar deltas; see `docs/metrics.md`.
- Function coverage mixes branch data when present (LCOV `BRDA`, JaCoCo `mb`/`cb`) as `(lines_covered + branches_covered) / (lines + branches)`; reports without branch records keep exact line-only ratios. Duplication defaults now match Sonar thresholds (`min_tokens` 100, `min_lines` 10). `README.md`, `docs/metrics.md`, `docs/cli-reference.md`, and `test-targets`/MCP wording updated; see Task 4.

## 0.5.3 - 2026-09-14

### Changed

- Reduced analyzer overhead with no output change: tree-sitter parsers are reused per thread, function/token walks no longer re-sort already-ordered traversals, Halstead distinct-operator/operand counting uses sort+dedup instead of hash sets, and the duplication tokenizer interns each distinct token once while moving (not cloning) token text into per-file tables.

## 0.5.1 - 2026-09-13

### Fixed

- Synchronized the Rust crate, Pi, OMP, and shared TypeScript adapter package versions for the release tag.
- Added the public repository URL to crate metadata and excluded dev-only agent/workflow files from the published crate package.

### Changed

- Strip release binaries to reduce published archive size.

## 0.5.0 - 2026-09-13

### Fixed

- Bare `leadline check` error now names the escape hatches (example threshold flags, or `--regressions` with `--base`/`--baseline`); the agent skill documents the full-project report recipe (`analyze` → `risk` → `impact`).

### Changed (breaking)

- Dropped every model/profile version suffix: `change-risk`, `change-risk-diff`, `impact`, `complexity-x-churn`, `mutation`, `tokens`, `duplication-drift`, and the `default` metric profile. No public release exists, so no compatibility is kept. Added `AGENTS.md` stating the no-backwards-compat rule.

## 0.4.0 - 2026-09-13

### Added

- Real-world smoke + benchmark suite (`scripts/realworld.sh`, `cargo bench --bench realworld`): shallow floating-main clones of TanStack Query, Nest, React, and Spring Boot with determinism smoke asserts, a `risk`/`project` join smoke, and per-fixture throughput floors in `benches/realworld.toml`. Local-only; see `docs/realworld-benchmarks.md`.

### Changed (breaking)

- Risk is a single model again: `leadline::risk_v2` is merged into `leadline::risk` as `change-risk-v2` (weights complexity 20, CRAP 15, churn 20, impact 20, ownership concentration 10, policy 15) and the `change-risk-v1` fallback is removed. `ownership` is now touch concentration instead of `100 / contributors`, and `policy` carries the highest unresolved source severity (error 100, warning 60, info 30; `0.0` when no rule fires) instead of always-`null`.
- `leadline risk --json` always emits the full ranking (no `truncated` field); `--limit` caps terminal and agent-JSON rows only. The `truncated` field is also gone from risk agent-JSON.
- `Project.risk` and `DebtReport` risk entries always use the single model; `ProjectInputs` takes `risk: &RiskReport` and `policy: &PolicyReport` instead of the v1/v2 option pair.

## 0.3.2 - 2026-09-13

### Fixed

- Agent integrations: the shared TypeScript adapter now decodes the analyzer's real `agent-json` shapes — `files[].functions[]` for analyze/function/check and `summary`/`regressions`/`improvements` for changed — instead of a schema the binary never emitted, and it keeps `leadline check` stdout on exit 1 (threshold violations) instead of discarding it.
- Pi and OMP extensions now register on the real extension API (`export default`, `pi.registerTool` with TypeBox parameter schemas) and attach warn-mode post-edit feedback to tool results; the previous fictional `ExtensionHost`/`activate()` shim was never called by any harness.
- OpenCode V2: the plugin now uses the shared adapter core, and the install docs say to link `plugin-v2/` under `~/.config/opencode/plugins/` only. A second `plugins` entry for the same directory fails the whole reload with `Duplicate plugin ID: leadline`.
- Claude Code: added the repository-root `.claude-plugin/marketplace.json`, hooks resolve through `${CLAUDE_PLUGIN_ROOT}` instead of `$CLAUDE_PROJECT_DIR`, and the Stop hook no longer discards violation output before printing it.
- `leadline check` from an integration now falls back to the documented default thresholds only when neither CLI flags nor `leadline.toml` define one.

### Added

- Root `package.json` with a `pi` manifest, so `pi install git:github.com/jbt95/leadline` and `omp plugin install` load the extension and skill directly from the repository.

## 0.3.1 - 2026-09-13

### Fixed

- Windows: `source_snapshot` strips the `\\?\` verbatim prefix from the canonicalized analysis root, so scope resolution no longer fails with "analysis path is outside the Git repository" on Windows CI.
- Windows: the snapshot test suite compares plain (non-verbatim) repository paths, and Unix-only import/property tests are gated to Unix targets.

## 0.3.0 - 2026-09-13

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
- Orchestration (`leadline::analytics`) and new CLI commands: `leadline project`, `leadline debt`, `leadline snapshot`, `leadline mutation`, `leadline duplication`, and `leadline policy` with JSON/agent-JSON projections and gates.
- Configuration: `[duplication]` settings and `[[architecture.rules]]` with strict validation; bounded external inputs (config 1 MiB/16 levels, XML/JSON per-file and aggregate limits, JSON/XML depth caps, DTD/entity/encoding rejection) and strict report-path parsing.

### Notes

- Remaining E-I work: static web report generation (Milestone F UI) and MCP tool parity for the new analytics surfaces.

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
