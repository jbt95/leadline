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
