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

## MCP: one `execute` tool

The MCP server advertises one read-only tool, `execute`. A script runs inside
the server and calls the twenty analyzer tools from a `tools` namespace, so a
multi-tool workflow costs one round trip instead of twenty and intermediate
reports never enter the model's context. The server never writes files in the
analyzed repository, runs hooks, or executes project code. When
`LEADLINE_METRICS_DIR` is set, tool calls additionally update the opt-in
local metrics store in that directory ([telemetry.md](telemetry.md)); keep
that directory outside the analyzed repository so the repository itself
stays write-free. Each tool called from a script records its own telemetry, and
the outer `execute` call is recorded too.

The
default transport is stdio (`leadline mcp`), matching every `command: leadline, args: [mcp]` harness config; `leadline mcp --port [N] [--host ADDR]` serves the same tool over HTTP (`POST /mcp`, `GET /health`). A bare `--port` means 3000, `0` asks the OS, and a taken port falls back to a free one with the actual address on stderr.

HTTP limits apply to every request: 32 MiB body, 64 KiB of headers, an 8 KiB line cap, a five-second whole-request read deadline, a ten-second whole-response write deadline, and 64 concurrent connections (excess connections get 503). At most 64 MiB of request bodies may be buffered across all connections; further declared bodies get 503 instead of multiplying memory. Only HTTP/1.1 is spoken (505 otherwise), malformed request lines and header lines are 400, `Transfer-Encoding` is 501, empty POST bodies are 400, and a request must carry exactly one `Host` and one `Content-Length`. Loopback-bound listeners only accept loopback authorities; browser `Origin`s must be loopback `http(s)` origins (scheme-less origins are rejected). JSON-RPC batches are capped at 64 …

### The `code` argument

`execute` takes one argument, `code`: a JavaScript **async function body**, not
an expression. Use `return` to produce the result. Top-level `await`, loops,
branching, and `Promise.all` all work, and the returned value must be
JSON-serializable. Inside a script, each tool is an async function taking that
tool's argument object and returning its report unchanged.

Pre-edit triage in one call:

```js
const [risk, blast, related] = await Promise.all([
  tools.risk({ path: "src/payment.ts" }),
  tools.impact({ target: "src/payment.ts" }),
  tools.coupling({ target: "src/payment.ts" }),
]);
return {
  score: risk.risks[0]?.score,
  dependents: blast.blast_radius,
  oftenChangedWith: related.related.map((row) => row.path),
};
```

Batch a gate over several paths, or fold a result into a decision:

```js
const targets = ["src/a.ts", "src/b.ts"];
const reports = [];
for (const path of targets) {
  const gate = await tools.check({ path, thresholds: { cognitive: 15 } });
  if (!gate.passed) reports.push({ path, violations: gate.violations });
}
return reports.length === 0 ? { ok: true } : { ok: false, reports };
```

### Limits and errors

A script has no filesystem, network, module, timer, or process access. One run
is capped at 100 tool calls, 30 seconds of wall clock, and 64 MB of script
memory. A tool that fails makes the script fail with
`script error: <message>`, carrying the tool's own message. A syntax error or
a thrown error reports the JavaScript message. Unknown `code` fields are
rejected as invalid parameters.

The twenty analyzer tools are reachable only from a script. A direct call to
one of their names is rejected with `unknown tool '<name>'; this server exposes
only \`execute\`, which calls leadline tools from a script`.

### Tools available inside a script

| Tool | Mirrors | Input | Output |
| --- | --- | --- | --- |
| `analyze` | `leadline analyze` | `path?`, `coverage?`, `top?`, `sort_by?`, `min_crap?`, `index?` | Ranked function rows. |
| `analyze_changed` | `leadline changed` | `base?`, `path?`, `target?`, `renames?`, `explain?`, budget flags | Before/after changed rows. |
| `analyze_function` | `leadline function` | `path`, `function`, `explain?` | One-function report, contributions with `explain`. |
| `check` | `leadline check` | `path?`, `base?`/`baseline?`, `coverage?`, `thresholds?`, `regressions?`, `index?` | Violations-only report. |
| `explain_metric` | `docs/metrics.md` | `metric` | Definition of one metric. |
| `repo_summary` | — | `path?`, `top?`, `coverage?` | Totals plus top functions per metric; the CRAP list needs `coverage`. |
| `test_targets` | `leadline test-targets` | `path?`, `coverage` (required), `top?` | Uncovered decision lines by CRAP. |
| `sql_plan` | `leadline sql-plan` | `current`, `baseline`, cost/row/estimate limits?, `top?` | Plan regressions in checked-in EXPLAIN artifacts. |
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
| `project` | `leadline project` | `path?`, `target?`, `since?`, `coverage?`, `ownership?`, mutation/test-map artifacts | Canonical project summary KPIs. |

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
