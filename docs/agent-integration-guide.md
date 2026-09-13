# Agent integration guide

## agent-json shape

`--format agent-json` returns a compact per-function list for machine consumers:

```json
{
  "schema_version": 1,
  "analyzer_version": "0.1.0",
  "metric_profile": "default-v1",
  "functions": [
    { "id": "src/a.ts:function:0:48", "path": "src/a.ts", "name": "pay", "start_line": 1, "cyclomatic": 2, "cognitive": 1, "crap": 2.5, "coverage": 0.5 }
  ]
}
```

Only the fields an agent gates on are included. Full detail remains in `--json`. `coverage`/`crap` are `null` when unknown.

`leadline hotspots --format agent-json` returns the ranked-file shape instead, so an agent can pick what to inspect before editing:

```json
{
  "schema_version": 1,
  "model": "complexity-x-churn-v1",
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

## MCP tools (read-only)

The MCP server exposes five read-only tools. It never writes files, runs hooks, or executes project code.

| Tool | Mirrors | Input | Output |
| --- | --- | --- | --- |
| `analyze` | `leadline analyze` | `path`, `lcov?`, `jacoco?` | Full JSON report. |
| `function` | `leadline function` | `file`, `name` | One-function JSON report. |
| `changed` | `leadline changed` | `base?`, `path?` | Changed JSON report. |
| `check` | `leadline check` | `path`, thresholds | Violations-only JSON report. |
| `version` | `leadline --version` | — | Version and schema identifiers. |

## Skill principles

- Report numbers with file, line, and threshold context. Never bare scores.
- Prefer the smallest diff that fixes the flagged function.
- Treat `null` coverage as unknown: ask for a coverage run, do not assume zero.
- Changed-only gating by default: gate on `changed`, observe the full tree.

> "Never rewrite code to lower a number without improving the design."

## Hooks

CI / pre-commit hooks default to warn-not-gate: they post violations as warnings and exit `0`. Switch to gating by passing explicit thresholds with a fail-on-violation flag in your pipeline, not by default.
