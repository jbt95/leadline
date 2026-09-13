# JSON schema

Reports are UTF-8 JSON. Paths use `/` separators. Floats use stable decimal formatting. Keys are sorted; file arrays are sorted by path; functions use source order.

## Analyze / function / check envelope

```json
{
  "schema_version": 1,
  "analyzer_version": "0.1.0",
  "metric_profile": "default-v1",
  "metric_specs": {
    "cyclomatic": "default-v1",
    "cognitive": "default-v1",
    "halstead": "default-v1",
    "maintainability": "default-v1",
    "crap": "default-v1"
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
- `metric_profile`: always `default-v1`; see `metrics.md` for rules.
- `metric_specs`: per-family rule versions, each `default-v1` in 1.0.
- Function `id` is `<path>:<kind>:<start_byte>:<end_byte>`. `kind` is `function` (or `method` / `arrow` where the grammar distinguishes). Byte offsets are source bytes.
- `coverage` and `crap` are numbers or `null`. `null` means unknown (no overlapping coverage lines); it never means zero.
- `parse_errors` entries carry `kind`, `start_line`, `start_column`, `end_line`, `end_column`. They are inline per file, never fatal by themselves.

## Changed envelope

```json
{
  "schema_version": 1,
  "analyzer_version": "0.1.0",
  "metric_profile": "default-v1",
  "metric_specs": { "cyclomatic": "default-v1", "cognitive": "default-v1", "halstead": "default-v1", "maintainability": "default-v1", "crap": "default-v1" },
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
  "metric_profile": "default-v1",
  "model": "complexity-x-churn-v1",
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

- `model`: the documented ordering rule (`complexity-x-churn-v1`); the
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
