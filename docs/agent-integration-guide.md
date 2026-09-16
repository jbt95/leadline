# Agent integration guide

## agent-json shape

`--format agent-json` returns a compact per-file function list for machine consumers:

```json
{
  "schema_version": 2,
  "metric_profile": "default",
  "summary": {"files": 1, "functions": 1},
  "files": [
    {"path": "src/a.ts", "functions": [
      {"name": "pay", "line": 1, "cyclomatic": 2, "cognitive": 1, "crap": 2.5, "coverage": 0.5}
    ], "parse_errors": []}
  ],
  "truncated": false
}
```

Only the fields an agent gates on are included. Full detail remains in `--json`. `coverage`/`crap` are `null` when unknown. `parse_errors` carries the syntax-error spans per file (empty when the file parsed), and `changed --format agent-json` reports `parse_errors` per file with `before`/`after` spans; consumers must surface them instead of reporting an empty function list as clean.

`leadline hotspots --format agent-json` returns the ranked-file shape instead, so an agent can pick what to inspect before editing:

```json
{
  "schema_version": 1,
  "model": "complexity-x-churn",
  "window": "90d",
  "git_available": true,
  "summary": { "files_analyzed": 128, "hotspots": 10 },
  "hotspots": [
    { "path": "src/payment.ts", "score": 868, "cognitive": 31, "cyclomatic": 18, "crap": 62.0, "coverage": 0.47, "changes": 28, "contributors": 7 }
  ],
  "truncated": false
}
```

`leadline coupling <file> --format agent-json` returns historically related files so an agent can inspect hidden contracts before editing:

```json
{
  "schema_version": 1,
  "target": "src/payment.ts",
  "git_available": true,
  "target_commits": 20,
  "related": [
    { "path": "src/payment-validator.ts", "commits": 15, "co_changes": 12, "directional": 0.6, "jaccard": 0.52 }
  ],
  "truncated": false
}
```

`leadline impact <file> --format agent-json` returns the transitive dependents
of one file so an agent can gauge blast radius before editing:

```json
{
  "schema_version": 1,
  "model": "impact",
  "target": "src/payment.ts",
  "files_analyzed": 128,
  "fan_in": 3,
  "fan_out": 1,
  "direct_dependents": 3,
  "blast_radius": 9,
  "blast_radius_percent": 7.09,
  "dependents": [
    { "path": "src/checkout.ts", "distance": 1 }
  ],
  "cycles": [],
  "truncated": false
}
```

`blast_radius_percent` is on a 0-100 scale. `dependents` are sorted by
`(distance, path)`; `truncated` means `--top` hid rows while `blast_radius`
still counts every dependent. Resolved imports are static evidence:
relative JS/TS imports and exact Java type imports only, with no aliases,
package graph, reflection, or runtime-built specifiers — inspect the listed
dependents, but do not treat an empty list as proof that nothing else loads
the file.

`leadline risk --format agent-json` returns the ranked change-risk list so
an agent can pick the riskiest files to inspect before editing:

```json
{
  "schema_version": 1,
  "model": "change-risk",
  "window": "90d",
  "git_available": true,
  "summary": { "files_analyzed": 128, "scope_files": 128, "risks": 10 },
  "risks": [
    { "path": "src/payment.ts", "score": 73.0,
      "components": { "complexity": 100.0, "crap": 100.0, "churn": 100.0, "impact": 50.0, "ownership": 80.0, "policy": 0.0 } }
  ]
}
```

`score` is the weight-renormalized mean of the known components on a 0-100
scale; `null` components mean unknown, never zero. Rows sort by `score`
descending; `--limit` caps terminal and agent-JSON rows while `--json` stays
full. See `docs/risk.md` for the formulas, weights, and caps.

## Pre-edit workflow

Before editing a file, check its change risk, what depends on it, and what
usually changes with it:

```console
leadline risk <path> --format agent-json
leadline impact <file> --format agent-json
leadline coupling <file> --format agent-json
```

`risk` answers "which files are riskiest to touch?" (explainable
`change-risk` components, informational only);

`impact` answers "what imports this file?" (static evidence, incomplete
where language or runtime configuration decides the real target);
`coupling` answers "what usually changes with this file?" (process
evidence, not a dependency). Inspect both lists, then make the smallest
diff that covers them.

## MCP tools (read-only)

The MCP server exposes eleven read-only tools. It never writes files, runs hooks, or executes project code. The default transport is stdio (`leadline mcp`), matching every `command: leadline, args: [mcp]` harness config; `leadline mcp --port [N] [--host ADDR]` serves the same tools over HTTP (`POST /mcp`, `GET /health`). A bare `--port` means 3000, `0` asks the OS, and a taken port falls back to a free one with the actual address on stderr.

HTTP limits apply to every request: 32 MiB body, 64 KiB of headers, an 8 KiB line cap, a five-second whole-request read deadline, a ten-second whole-response write deadline, and 64 concurrent connections (excess connections get 503). At most 64 MiB of request bodies may be buffered across all connections; further declared bodies get 503 instead of multiplying memory. Only HTTP/1.1 is spoken (505 otherwise), malformed request lines and header lines are 400, `Transfer-Encoding` is 501, empty POST bodies are 400, and a request must carry exactly one `Host` and one `Content-Length`. Loopback-bound listeners only accept loopback authorities; browser `Origin`s must be loopback `http(s)` origins (scheme-less origins are rejected). JSON-RPC batches are capped at 64 requests and every response (batch included) at 32 MiB. `top` accepts at most 200 entries on every tool, and each scanner tool caps both `findings` and `violations` at `top`, setting `truncated` when either was cut. Artifact arguments (`sarif`, `osv`, `trivy`, and `sql_plan`'s `current`/`baseline`) stay root-relative: absolute paths, parent-directory escapes, and symlinks that resolve outside the working directory are rejected; `migration_roots` are analysis-root-relative and get lexical validation only. `check` fills missing metrics from `leadline.toml` `[thresholds.function]` and uses `[regressions]` limits for `regressions: true`, exactly like the CLI.

| Tool | Mirrors | Input | Output |
| --- | --- | --- | --- |
| `analyze` | `leadline analyze` | `path?`, `coverage?`, `top?`, `sort_by?`, `min_crap?` | Ranked function rows. |
| `analyze_changed` | `leadline changed` | `base?`, `path?`, `target?`, `renames?`, `explain?`, budget flags | Before/after changed rows. |
| `analyze_function` | `leadline function` | `path`, `function`, `explain?` | One-function report, contributions with `explain`. |
| `check` | `leadline check` | `path?`, `base?`/`baseline?`, `coverage?`, `thresholds?`, `regressions?` | Violations-only report. |
| `explain_metric` | `docs/metrics.md` | `metric` | Definition of one metric. |
| `repo_summary` | — | `path?`, `top?` | Totals plus top functions per metric. |
| `test_targets` | `leadline test-targets` | `path?`, `coverage` (required), `top?` | Uncovered decision lines by CRAP. |
| `sql_plan` | `leadline sql-plan` | `current`, `baseline`, cost/row/estimate limits?, `top?` | Plan regressions in checked-in EXPLAIN artifacts. |
| `security_findings` | `leadline security` | `sarif`, `baseline_sarif?`, `path?`, `base?`/`staged?`/`target?`, `minimum_severity?`, `new_only?`, `changed_only?`, `top?` | Scanner findings with code context. |
| `vulnerabilities` | `leadline vulnerabilities` | `osv?`/`trivy?`, `baseline_osv?`/`baseline_trivy?`, `path?`, `base?`/`staged?`/`target?`, `minimum_severity?`, `top?` | Vulnerable deps with changed-import evidence. |
| `sql_risks` | `leadline sql` | `path?`, `large_offset?`, `migration_roots?`, `minimum_severity?`, `top?` | Static PostgreSQL query risks. |

The `analyze` and `check` tools accept an optional `index` directory pointing at a repository index built by `leadline index`. Repeated post-edit calls then reuse unchanged file metrics and Git facts instead of recomputing them. The server only ever reads the index and will never create or modify one. When the argument is absent the configured `[index].path` is used, and an unusable or missing index silently degrades to a full analysis.

## Skill principles

- Report numbers with file, line, and threshold context. Never bare scores.
- Prefer the smallest diff that fixes the flagged function.
- Treat `null` coverage as unknown: ask for a coverage run, do not assume zero.
- Changed-only gating by default: gate on `changed`, observe the full tree.

> "Never rewrite code to lower a number without improving the design."

## Hooks

CI / pre-commit hooks default to warn-not-gate: they post violations as warnings and exit `0`. Switch to gating by passing explicit thresholds with a fail-on-violation flag in your pipeline, not by default.

## Secret gating pre-commit hook

`integrations/git-hooks/pre-commit` delegates to the shared
`integrations/common/leadline-secret-check.sh` runner in staged mode and
blocks the commit when findings fail the gate. Install it manually (Leadline
does not install hooks automatically):

```sh
cp integrations/git-hooks/pre-commit .git/hooks/pre-commit
```

or symlink it:

```sh
ln -s ../../integrations/git-hooks/pre-commit .git/hooks/pre-commit
```

## Secret gating operational limits

Leadline itself never detects secrets: the shared runner delegates detection
to an installed `gitleaks` binary (a prerequisite; a missing binary fails
visibly, never silently). The pre-commit hook scans staged content and blocks
the commit on findings; agent adapters scan the worktree at the earliest
supported lifecycle event. Warn/block capability by host: the Git hook,
Claude/Gemini checkpoints, and Pi's explicit `leadline_secret_check` tool
block; Cline's `secretGate` runs the same shared wrapper (whether Cline
honors exit `2` as blocking is unverified); Pi's and Cline's post-edit hooks
only warn; and the OpenCode `leadline_secret_check` tool runs on demand in
warn mode. Claude and Gemini only block on exit `2`, so the shared wrapper
maps findings, scanner failures, and missing tools to exit `2` with the
runner's redacted diagnostics on stderr. The wrapper resolves
`integrations/common/leadline-secret-check.sh` from the checkout, from the
host project directory (`GEMINI_PROJECT_DIR`/`CLAUDE_PROJECT_DIR`), or from
`LEADLINE_SECRET_RUNNER`. Every invocation passes
`--redact`, prints only fixed diagnostics (never SARIF, diffs, or secret
values), and cleans its private temporary files via trap. To bypass,
intentionally disable the installed hook or integration — there is no
pass-through flag.
