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
