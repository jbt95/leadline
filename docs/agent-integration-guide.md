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

Only the fields an agent gates on are included. Full detail remains in `--json`. `coverage`/`crap` are `null` when unknown. `parse_errors` carries the syntax-error spans per file (empty when the file parsed), and `changed --format agent-json` reports `parse_errors` per file with `before`/`after` spans; consumers must surface them instead of reporting an empty function list as clean. Leadline supports C, C++, Go, Java, JavaScript, Python, Rust, TypeScript, TSX, and Zig sources.

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
relative JS/TS imports, exact Java type imports, Rust `mod` declarations
resolved against the declaring module's directory, quoted C/C++ `#include`
lines resolved relative to the including file (angle includes are ignored),
and Python relative imports (absolute dotted imports are recorded
`unresolved` with reason `unsupported`); Go imports produce no edge. Zig
quoted `@import("...")` specifiers resolve only when an exact local `.zig` path
relative to the importing file matches a discovered file. Package resolution,
build graphs/options, generated-file provenance, and runtime-computed
specifiers are not interpreted. No aliases, package graph, reflection, or
runtime-built specifiers — inspect the listed dependents, but do not treat an
empty list as proof that nothing else loads the file.

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

## MCP: twenty direct analyzer tools

The MCP server advertises the twenty analyzer tools directly. `tools/list`
returns each name with its own description, read-only annotations, and JSON
Schema, and `tools/call` routes a name straight to that analyzer, so a host
discovers the whole surface with a `tools/list` request, independently of
`initialize`, and calls one tool per request.
The server never writes files in the analyzed repository, runs hooks, or
executes project code. When `LEADLINE_METRICS_DIR` is set, every tool call also
updates the opt-in local metrics store in that directory
([telemetry.md](telemetry.md)); keep that directory outside the analyzed
repository so the repository itself stays write-free. Each call is recorded in
telemetry under its own tool name, so `analyze` and `check` are counted and
timed separately.

The
default transport is stdio (`leadline mcp`), matching every `command: leadline, args: [mcp]` harness config; `leadline mcp --port [N] [--host ADDR]` serves the same tools over HTTP (`POST /mcp`, `GET /health`). A bare `--port` means 3000, `0` asks the OS, and a taken port falls back to a free one with the actual address on stderr.

HTTP limits apply to every request: 32 MiB body, 64 KiB of headers, an 8 KiB line cap, a five-second whole-request read deadline, a ten-second whole-response write deadline, and 64 concurrent connections (excess connections get 503). At most 64 MiB of request bodies may be buffered across all connections; further declared bodies get 503 instead of multiplying memory. Only HTTP/1.1 is spoken (505 otherwise), malformed request lines and header lines are 400, `Transfer-Encoding` is 501, empty POST bodies are 400, and a request must carry exactly one `Host` and one `Content-Length`. Loopback-bound listeners only accept loopback authorities; browser `Origin`s must be loopback `http(s)` origins (scheme-less origins are rejected). JSON-RPC batches are capped at 64 …

### Calling a tool

`tools/call` takes `name` and an optional `arguments` object; omitting
`arguments` is the same as passing none. The result is the MCP
`CallToolResult` envelope: a `content` array holding one `text` block with the
report serialized as JSON, and the same report object under `structuredContent`
for clients that consume native JSON. Every report also carries
`schema_version`, `analyzer_version`, and `metric_specs`.

Pre-edit triage is three calls, each with its own arguments:

```console
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"risk","arguments":{"path":"src/payment.ts"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"impact","arguments":{"target":"src/payment.ts"}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"coupling","arguments":{"target":"src/payment.ts"}}}
```

A JSON-RPC batch array is accepted on one line, so several independent tools
can share a round trip; a batch of notifications (requests without an `id`)
draws no response.

Gating after an edit is one call per path, and the report names only what
failed:

```console
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"check","arguments":{"path":"src/payment.ts","thresholds":{"cognitive":15}}}}
```

`check` requires at least one threshold, or `regressions` together with
`base` or `baseline`; its `passed` field is the verdict, and each violation row
carries a `reason` array holding the threshold names that failed, `regression`
for a delta gate, or `crap_unavailable` when a CRAP gate found no coverage
record.

### Errors and capabilities

A tool returns analysis data only and none accepts caller-supplied code, so
the interface has no script and a caller cannot compose several tools into a
new capability: each request names one analyzer and passes only that
analyzer's declared arguments. Filesystem access stays inside the reads the
analysis path already performs — source files, an optional coverage file,
`leadline.toml`, the index, and checked-in artifact files — and the only
subprocess is the fixed-argument `git` adapter described in
[security-model.md](security-model.md), which several tools invoke.

An unknown tool name is rejected with `unknown tool '<name>'`; an unknown
argument key is rejected with `unknown <tool> argument '<key>'`; and a wrong
type, a missing required argument, or an unusable artifact is rejected with
`-32602` and the tool's own message. A `top` above the maximum is rejected
rather than silently trimmed: 200 for the list tools, 50 for `repo_summary`.
A list that was cut to `top` reports `truncated` so it is never mistaken for
a complete one; the row tools that report a total also carry `total`. `risk` is
the exception: it truncates its list to `top` (default 10) and reports no
`truncated` flag, so nothing signals that rows were dropped — ask for a larger
`top` or narrow `path` if you need the full ranking.

### The twenty tools

| Tool | Mirrors | Input | Output |
| --- | --- | --- | --- |
| `analyze` | `leadline analyze` | `path?`, `coverage?`, `top?`, `sort_by?`, `min_crap?`, `index?` | Ranked function rows. |
| `analyze_changed` | `leadline changed` | `base?`, `path?`, `target?`, `renames?`, `explain?`, `top?`, `sort_by?`, `min_crap?`, `min_delta?` | Before/after changed rows. |
| `analyze_function` | `leadline function` | `path`, `function`, `explain?` | One-function report, contributions with `explain`. |
| `check` | `leadline check` | `path?`, `base?`/`baseline?`, `coverage?`, `thresholds?`, `regressions?`, `index?` | Violations-only report. |
| `explain_metric` | `docs/metrics.md` | `metric` | Definition of one metric. |
| `repo_summary` | — | `path?`, `top?`, `coverage?` | Totals plus top functions per metric; the CRAP list needs `coverage`. |
| `test_targets` | `leadline test-targets` | `path?`, `coverage` (required), `top?` | Uncovered decision lines by CRAP. |
| `sql_plan` | `leadline sql-plan` | `current`, `baseline`, `max_cost_increase_percent?`, `max_plan_rows_ratio?`, `max_estimate_error_ratio?`, `top?` | Plan regressions in checked-in EXPLAIN artifacts. |
| `security_findings` | `leadline security` | `sarif`, `baseline_sarif?`, `path?`, `base?`/`staged?`/`target?`, `minimum_severity?`, `new_only?`, `changed_only?`, `top?` | Scanner findings with code context. |
| `vulnerabilities` | `leadline vulnerabilities` | `osv?`/`trivy?`, `baseline_osv?`/`baseline_trivy?`, `path?`, `base?`/`staged?`/`target?`, `minimum_severity?`, `top?` | Vulnerable deps with changed-import evidence. |
| `sql_risks` | `leadline sql` | `path?`, `large_offset?`, `migration_roots?`, `minimum_severity?`, `top?` | Static PostgreSQL query risks. |
| `hotspots` | `leadline hotspots` | `path?`, `top?`, `since?`, `coverage?` | Churn/complexity/CRAP hotspot rows. |
| `risk` | `leadline risk` | `path?`, `top?`, `since?`, `coverage?` | Explainable risk scores with components. |
| `dependencies` | `leadline dependencies` | `path?` | Fan-in/fan-out, edges, cycles, unresolved imports. |
| `impact` | `leadline impact` | `target`, `path?`, `top?` | Transitive dependents and blast radius. |
| `coupling` | `leadline coupling` | `target`, `path?`, `top?`, `min_cochanges?` | Files that co-change with the target. |
| `duplication` | `leadline duplication` | `path?`, `base?` | Token clones, or new/existing/resolved drift. |
| `policy` | `leadline policy` | `path?`, `base?` | Architecture-rule violations or drift. |
| `debt` | `leadline debt` | `path?`, `base?`, `target?`, `renames?`, `since?` | Threshold and risk-score changes against a base. |
| `project` | `leadline project` | `path?`, `target?`, `since?`, `coverage?`, `ownership?`, `pit?`, `stryker?`, `test_map?`, `snapshots?` | Canonical project summary KPIs. |

A `?` marks an optional argument. `tools/list` carries the full per-tool
schema — types, defaults, enums, and required arguments — and that is the
authority; this table is the quick index.

The `analyze` and `check` tools accept an optional `index` directory pointing at a repository index built by `leadline index`. Repeated post-edit calls then reuse unchanged file metrics instead of recomputing them. The server only ever reads the index and will never create or modify one. When the argument is absent the configured `[index].path` is used, and an unusable or missing index silently degrades to a full analysis.

## Triage companion (optional, network)

`integrations/typesafe-triage/` is an optional companion that ranks leadline
reports with [TypeSafe](https://docs.typesafe.ai) judgments (role, inherent
complexity, attention) and prints a prioritized worklist for changed-code,
repo-wide `check`, scanner, debt, duplication, and workflow-routing triage. It
runs as a separate TypeScript tool with no runtime dependencies — dev
dependencies cover typechecking and the vendored MIT
[anti-slop](https://github.com/dmmulroy/anti-slop) Oxlint rules — and sends
only report metadata — paths, function names, metrics, scanner IDs, or the
free-form request text for `route` — never source text. The output is advisory: it cannot gate, change exit codes, or
suppress a finding, and the leadline binary and MCP server remain offline and
network-free. See `integrations/typesafe-triage/README.md`.

## Skill principles

- Report numbers with file, line, and threshold context. Never bare scores.
- Prefer the smallest diff that fixes the flagged function.
- Treat `null` coverage as unknown: ask for a coverage run, do not assume zero.
- Changed-only gating by default: gate on `changed`, observe the full tree.

> "Never rewrite code to lower a number without improving the design."

## Hooks

The agent lifecycle hooks that report complexity default to warn-not-gate:
they post findings as warnings and always exit `0`; the secret gates are the
exception (see below). Enable quality gating by passing explicit thresholds to
`leadline check` in your pipeline (violations exit `1`), not by default.

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
supported lifecycle event, scoped to files changed against HEAD (untracked
files count; an empty change set skips the scan). Warn/block capability by
host: the Git hook, the Gemini checkpoint, and Pi's explicit
`leadline_secret_check` tool block; the Claude Code `Stop` hook runs checks
only (the per-turn secret scan was removed: a whole-tree scan on every turn
while the gate only evaluates changed paths); Cline's `secretGate` runs the
same shared wrapper (whether Cline honors exit `2` as blocking is
unverified); Pi's and Cline's post-edit hooks only warn; and the OpenCode
`leadline_secret_check` tool runs on demand in warn mode. Claude and Gemini
only block on exit `2`, so the shared wrapper maps findings to exit `2` with
the runner's redacted diagnostics on stderr;
an unavailable scanner or runner, a scan failure, a bad mode, or a missing
git comparison target exits `1` (visible to the user, non-blocking) so an
environment miss cannot loop a Stop hook. Worktree mode checks the git HEAD
it needs to diff against before scanning: a directory that is not a
repository, or a repository with no commits, skips the gate without running
gitleaks and reports exit `3` through the same visible, non-blocking path.
The wrapper resolves
`integrations/common/leadline-secret-check.sh` from the packaged extension
(a vendored `common/` copy ships with the Claude Code plugin), from the
checkout, from the host project directory
(`GEMINI_PROJECT_DIR`/`CLAUDE_PROJECT_DIR`), or from
`LEADLINE_SECRET_RUNNER`. Every invocation passes
`--redact`, prints only fixed diagnostics (never SARIF, diffs, or secret
values), and cleans its private temporary files via trap. To bypass,
intentionally disable the installed hook or integration — there is no
pass-through flag.
