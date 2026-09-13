# JSON schema

Reports are UTF-8 JSON. Paths use `/` separators. Floats use stable decimal formatting. Keys are sorted; file arrays are sorted by path; functions use source order.

## Analyze / function / check envelope

```json
{
  "schema_version": 1,
  "analyzer_version": "0.1.0",
  "metric_profile": "default",
  "metric_specs": {
    "cyclomatic": "default",
    "cognitive": "default",
    "halstead": "default",
    "maintainability": "default",
    "crap": "default"
  },
  "files": [
    {
      "path": "src/payment.ts",
      "language": "typescript",
      "functions": [
        {
          "id": "src/payment.ts:function:0:96",
          "name": "processPayment",
          "kind": "function",
          "start_line": 1,
          "end_line": 6,
          "start_byte": 0,
          "end_byte": 96,
          "metrics": {
            "loc": 6,
            "logical_loc": 4,
            "function_length": 6,
            "parameters": 1,
            "max_nesting": 1,
            "cyclomatic": 2,
            "cognitive": 1,
            "halstead_n1": 8,
            "halstead_n2": 6,
            "halstead_N1": 10,
            "halstead_N2": 7,
            "halstead_vocabulary": 14,
            "halstead_length": 17,
            "halstead_volume": 64.7,
            "halstead_difficulty": 2.3,
            "halstead_effort": 150.0,
            "maintainability_index": 78.4,
            "coverage": 0.5,
            "crap": 2.5
          }
        }
      ],
      "parse_errors": []
    }
  ]
}
```

- `schema_version`: output compatibility marker (`1`).
- `analyzer_version`: the `leadline` crate version that produced the report.
- `metric_profile`: always `default`; see `metrics.md` for rules.
- `metric_specs`: per-family rule versions, each `default` in 1.0.
- Function `id` is `<path>:<kind>:<start_byte>:<end_byte>`. `kind` is `function` (or `method` / `arrow` where the grammar distinguishes). Byte offsets are source bytes.
- `coverage` and `crap` are numbers or `null`. `null` means unknown (no overlapping coverage lines); it never means zero.
- `parse_errors` entries carry `kind`, `start_line`, `start_column`, `end_line`, `end_column`. They are inline per file, never fatal by themselves.

## Changed envelope

```json
{
  "schema_version": 1,
  "analyzer_version": "0.1.0",
  "metric_profile": "default",
  "metric_specs": { "cyclomatic": "default", "cognitive": "default", "halstead": "default", "maintainability": "default", "crap": "default" },
  "base": "HEAD~1",
  "functions": [
    { "path": "src/payment.ts", "name": "processPayment", "before": {}, "after": {} }
  ],
  "parse_errors": [
    { "path": "src/payment.ts", "before": [], "after": [] }
  ]
}
```

Each `functions` entry pairs one before/after version. `before: null` means added, `after: null` means removed. Unchanged pairs are omitted. See `changed-code.md`.

## Hotspots envelope

```json
{
  "schema_version": 1,
  "analyzer_version": "0.2.0",
  "metric_profile": "default",
  "model": "complexity-x-churn",
  "window": "90d",
  "git_available": true,
  "head_commit": "0c2d7309cd3c84d33e4ea8fec01a581cf5246b37",
  "files_analyzed": 128,
  "hotspots": [
    {
      "path": "src/payment.ts",
      "language": "typescript",
      "loc": 420,
      "functions": 12,
      "max_cognitive": 31,
      "max_cyclomatic": 18,
      "max_crap": 62.0,
      "coverage": 0.47,
      "functions_with_coverage": 12,
      "commits": 96,
      "changes": 28,
      "changes_30d": 9,
      "changes_90d": 28,
      "changes_365d": 71,
      "lines_added": 1204,
      "lines_deleted": 486,
      "days_since_last_change": 3,
      "contributors": 7,
      "recent_contributors": 4,
      "score": 868
    }
  ],
  "truncated": false
}
```

- `model`: the documented ordering rule (`complexity-x-churn`); the
  dimensions next to it are the actual evidence. See `hotspots.md`.
- `window`: `30d`, `90d`, or `365d`; `changes` and `score` use it.
- `git_available: false` (directory outside a repository, unborn HEAD, or no
  `git`) sets every churn field and `score` to `null`; complexity still ranks.
  A churn field is also `null` when Git is available but the file has no
  history record (new or untracked). `git_available` distinguishes the two.
- `score` is `max_cognitive x changes`, or `null` without Git history.
- `coverage` is the LOC-weighted mean over `functions_with_coverage`.
- All churn windows are relative to `head_commit`'s commit time, so the same
  snapshot yields identical JSON on any day.
- `--format agent-json` emits a compact shape: `schema_version`, `model`,
  `window`, `git_available`, `summary.{files_analyzed,hotspots}`, one row per
  hotspot (`path`, `score`, `cognitive`, `cyclomatic`, `crap`, `coverage`,
  `changes`, `contributors`), and `truncated`.

## Coupling envelope

```json
{
  "schema_version": 1,
  "analyzer_version": "0.2.0",
  "target": "src/payment/PaymentService.ts",
  "git_available": true,
  "target_commits": 20,
  "pair_commits": 19,
  "max_commit_files": 50,
  "related": [
    {
      "path": "src/payment/PaymentValidator.ts",
      "commits": 15,
      "co_changes": 12,
      "directional": 0.6,
      "reverse_directional": 0.8,
      "jaccard": 0.5217391304347826
    }
  ],
  "truncated": false
}
```

- `target_commits` counts every commit touching the target, including commits
  wider than `max_commit_files`; `pair_commits` counts the target commits small
  enough to contribute pairs.
- `directional` is `co_changes / target_commits`; `reverse_directional` is
  `co_changes / commits`; `jaccard` is
  `co_changes / (target_commits + commits - co_changes)`.
- `git_available: false` yields `target_commits: 0` and an empty `related`
  list. Rows are sorted by `directional`, then `co_changes`, then path.
- `--format agent-json` emits `schema_version`, `target`, `git_available`,
  `target_commits`, one row per related file (`path`, `commits`, `co_changes`,
  `directional`, `jaccard`), and `truncated`.

## Dependencies envelope

```json
{
  "schema_version": 1,
  "analyzer_version": "0.2.0",
  "metric_profile": "default",
  "files": [
    { "path": "src/a.ts", "fan_in": 1, "fan_out": 2 }
  ],
  "edges": [
    { "source": "src/a.ts", "target": "src/b.ts", "kind": "import", "confidence": "high" }
  ],
  "unresolved": [
    { "source": "src/a.ts", "specifier": "./missing", "line": 4, "reason": "not_found" }
  ],
  "cycles": [
    { "files": ["src/a.ts", "src/b.ts"] }
  ]
}
```

- `source` imports `target`. `kind` is `import` (static import or re-export)
  or `call` (`require()` / dynamic `import()` form); when both name the same
  pair, `import` wins. `confidence` is always `"high"` by construction.
- `fan_in` counts direct importers; `fan_out` counts resolved imported files.
- `unresolved` reasons: `not_found`, `ambiguous`, `outside_scope`,
  `unsupported`. Bare package imports are ignored, never listed.
- `cycles` are strongly connected components of at least two files; each
  `files` array is sorted, and the cycle list is sorted. A self-import is an
  edge, not a cycle.
- Ordering is deterministic: `files` by path, `edges` by
  `(source, target)`, `unresolved` by `(source, specifier, line, reason)`.
  The same repository, configuration, and analyzer version produce
  byte-identical JSON across runs.
- `--format agent-json` emits `schema_version`,
  `summary.{files,edges,cycles,unresolved}`, one row per file (`path`, `fan_in`,
  `fan_out`), one row per edge (`source`, `target`, `kind`; `confidence`
  dropped), one row per cycle (`files`), one row per unresolved reference
  (`source`, `specifier`, `line`, `reason`), and `truncated: false`.

## Impact envelope

```json
{
  "schema_version": 1,
  "analyzer_version": "0.2.0",
  "metric_profile": "default",
  "model": "impact",
  "target": "src/b.ts",
  "files_analyzed": 4,
  "fan_in": 1,
  "fan_out": 0,
  "direct_dependents": 1,
  "blast_radius": 3,
  "blast_radius_percent": 100.0,
  "dependents": [
    { "path": "src/a.ts", "distance": 1 },
    { "path": "src/c.ts", "distance": 2 },
    { "path": "src/d.ts", "distance": 3 }
  ],
  "cycles": [["src/a.ts", "src/b.ts"]],
  "truncated": false
}
```

- `dependents` are the unique direct and transitive importers of `target` by
  reverse-BFS, with shortest distances, sorted by `(distance, path)`. The
  target never re-appears, even through a cycle.
- `blast_radius` counts every dependent even when `--top` truncates the shown
  list; `truncated` signals the cap. `direct_dependents` counts distance-1
  rows. `fan_in` / `fan_out` repeat the target's graph row.
- `blast_radius_percent` is `blast_radius / (files_analyzed - 1) * 100` on a
  **0-100 scale** (`0.0` when `files_analyzed <= 1`).
- `cycles` keeps only the graph cycles containing the target.
- `--format agent-json` emits `schema_version`, `model`, `target`,
  `files_analyzed`, `fan_in`, `fan_out`, `direct_dependents`, `blast_radius`,
  `blast_radius_percent`, `dependents` (`path`, `distance`), `cycles`, and
  `truncated`; it drops `metric_profile` and `analyzer_version`.

## Risk envelope

```json
{
  "schema_version": 1,
  "analyzer_version": "0.2.0",
  "metric_profile": "default",
  "model": "change-risk",
  "window": "90d",
  "git_available": true,
  "head_commit": "0c2d7309cd3c84d33e4ea8fec01a581cf5246b37",
  "files_analyzed": 2,
  "scope_files": 2,
  "risks": [
    {
      "path": "src/risky.ts",
      "score": 73.0,
      "components": {
        "complexity": 100.0,
        "crap": 100.0,
        "churn": 100.0,
        "impact": 50.0,
        "ownership": 80.0,
        "policy": 0.0
      },
      "raw": {
        "max_cognitive": 30,
        "max_cyclomatic": 10,
        "max_crap": 30.0,
        "changes": 20,
        "contributors": 2,
        "concentration_percent": 80.0,
        "blast_radius": 1,
        "blast_radius_percent": 50.0,
        "fan_in": 1,
        "fan_out": 0,
        "policy_severity": null
      }
    }
  ]
}
```

- `model` is the versioned scoring rule (`change-risk`); `score` is the
  weight-renormalized component mean on a 0-100 scale. See `risk.md` for the
  per-component formulas, weights (complexity 20, CRAP 15, churn 20, impact
  20, ownership 10, policy 15), and caps.
- `window`: `30d`, `90d`, or `365d`; `changes` and the `churn` component use
  it. Windows are relative to the HEAD commit time, mirroring `hotspots`;
  `head_commit` pins which snapshot the window is relative to.
  `scope_files` is the graph size the blast percents range over; it equals
  `files_analyzed` except when analysis covers a sub-scope (e.g. a single
  file), and the terminal blast line pairs the percent with `scope_files`.
- `null` means unknown, never zero: `crap` is `null` without coverage,
  `churn`/`ownership` (and `raw.changes`/`raw.contributors`) are `null` when
  the file has no history row, and `policy` is `0.0` when no rule fires. The score
  renormalizes over the known components. `complexity` and `impact` are
  always present (`impact` is `0.0` for files with no graph node).
- `git_available: false` (directory outside a repository, unborn HEAD, or no
  `git`) still ranks by the static dimensions with `null` churn/ownership.
- Rows sort by `score` descending, ties by `path` ascending. `--limit N`
  (default 10, `N >= 1`) caps terminal and agent-JSON rows; `--json` always
  emits the full ranking. The command is informational and exits `0`.
- `--format agent-json` emits `schema_version`, `model`, `window`,
  `git_available`, `summary.{files_analyzed,scope_files,risks}`, and one row
  per file (`path`, `score`, `components` with the same six keys);
  it drops `metric_profile`, `analyzer_version`, `head_commit`, and `raw`.
