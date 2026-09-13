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

`leadline impact <file> --format agent-json` returns the transitive dependents
of one file so an agent can gauge blast radius before editing:

```json
{
  "schema_version": 1,
  "model": "impact-v1",
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
  "model": "change-risk-v1",
  "window": "90d",
  "git_available": true,
  "summary": { "files_analyzed": 128, "scope_files": 128, "risks": 10 },
  "risks": [
    { "path": "src/payment.ts", "score": 82.5,
      "components": { "complexity": 100.0, "crap": 100.0, "churn": 100.0, "impact": 50.0, "ownership": 50.0, "policy": null } }
  ],
  "truncated": false
}
```

`score` is the weight-renormalized mean of the known components on a 0-100
scale; `null` components mean unknown, never zero. Rows sort by `score`
descending; `truncated` means `--limit` hid rows. See `docs/risk.md` for the
formulas, weights, and caps.

## Pre-edit workflow

Before editing a file, check its change risk, what depends on it, and what
usually changes with it:

```console
leadline risk <path> --format agent-json
leadline impact <file> --format agent-json
leadline coupling <file> --format agent-json
```

`risk` answers "which files are riskiest to touch?" (explainable
`change-risk-v1` components, informational only);

`impact` answers "what imports this file?" (static evidence, incomplete
where language or runtime configuration decides the real target);
`coupling` answers "what usually changes with this file?" (process
evidence, not a dependency). Inspect both lists, then make the smallest
diff that covers them.

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
