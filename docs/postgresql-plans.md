# PostgreSQL plan regression checks

`leadline sql-plan --current DIR --baseline DIR` compares checked-in
PostgreSQL `EXPLAIN` artifacts and fails fast on material plan regressions.
Leadline never connects to PostgreSQL, never executes SQL, and never shells
out to `psql`: it only reads raw JSON files you generate outside Leadline.

## Generating artifacts

Produce one file per query with `EXPLAIN (FORMAT JSON)`:

```sql
EXPLAIN (FORMAT JSON) SELECT * FROM users WHERE email = 'a@example.com';
```

Save the output (a one-element JSON array) as `DIR/<query-id>.json`, where
`<query-id>` is the UTF-8 filename without `.json`. Use `EXPLAIN (ANALYZE,
FORMAT JSON)` only when you accept that `ANALYZE` executes the query;
Leadline treats the extra `Actual Rows` / `Actual Loops` fields as estimate
evidence and still executes nothing itself.

## Directory rules

- Each directory is flat: nested directories, symlinks, and non-file entries
  are rejected. Non-`.json` regular files are ignored.
- At most 200 `.json` files per directory; 64 MiB per file, 256 MiB total,
  JSON nesting depth 128.
- Filenames must be UTF-8 with a non-empty stem free of control characters.
- Each file must hold exactly one statement: a one-element top-level array
  with a `Plan` object. SQL text, planning/execution times, filters, and
  other properties are ignored, never retained.
- Only these node fields are retained: `Node Type`, `Relation Name`,
  `Schema Name`, `Index Name`, `Total Cost`, `Plan Rows`, `Actual Rows`,
  `Actual Loops`, and child `Plans`. Negative or non-finite numbers are
  rejected. Malformed artifacts exit `4`.
- When `Schema Name` is present, scan identity is `schema.relation`, so
  same-named tables in different schemas never merge or mask each other.

## Compared facts

Each plan reduces to stable facts: root total cost and plan rows, sort-node
count (`Sort`, `Incremental Sort`), join counts (`Nested Loop`,
`Hash Join`, `Merge Join`), per-relation scan kinds (sequential, index,
index-only, bitmap), and the maximum planner estimate error.

Estimate error per node is
`max(actual / estimated, estimated / actual)` comparing per-execution values:
PostgreSQL reports `Actual Rows` averaged over `Actual Loops`, and `Plan Rows`
is the per-execution estimate, so the two are directly comparable and
`Actual Loops` never multiplies into the ratio. Nodes missing either side are
skipped; a zero estimate against non-zero actuals (or vice versa) is unbounded
and never stored as a non-finite float.

## Change kinds and gates

Each kind names the compared dimension; `detail` carries direction and
magnitude, and `violates_gate` marks a gate failure. Decreases are reported
under the same dimension kind with `detail` like `"decreased to 0.75x"` and
`violates_gate: false`.

| Kind | Fails when |
| --- | --- |
| `index_to_sequential_scan` | a relation served by an index/bitmap scan is now scanned sequentially only; always fails |
| `cost_increase` | total-cost increase exceeds `--max-cost-increase-percent` |
| `row_growth` | plan-row ratio exceeds `--max-plan-rows-ratio` |
| `estimate_error` | current absolute estimate error exceeds `--max-estimate-error-ratio` (baseline is not consulted) |
| `added_sort` | sort nodes grew; fails only beside a numeric failure in the same query |
| `join_strategy_change` | per-kind join counts changed; fails only beside a numeric failure in the same query |

Cost percent is `(current - baseline) / baseline * 100`; a zero baseline
with positive current is an unbounded increase. Row ratios use the same
zero rule. Threshold equality passes: only strictly greater values fail.
New and removed queries are informational and never fail. With no
violations the command exits `0`; any violation exits `1` with the full
report retained on stdout.

## Output shapes

- `--json`: the complete report (`queries` with per-query `changes`, plus
  flat `violations`), sorted by query ID, then kind rank, relation, detail.
- `--format agent-json --top N` (default 50): changed/new/removed queries
  capped at `N` with `truncated`, plus complete `violations`.
- `--format sarif`: one `postgresql-plan/{kind}` result per violation with
  artifact URI `plans/{query_id}.json` and metric-only messages.
- Terminal: one block per non-unchanged query (`!` marks gate failures).
- MCP tool `sql_plan` takes root-relative `current`/`baseline`, the three
optional limits, and `top`; absolute paths, unknown keys, negative limits,
and `top` above 200 are rejected.

## Limitations

Findings are planner evidence, not runtime guarantees: estimates reflect
the planner's statistics at generation time, and index-to-sequential
transitions deserve a look at the underlying schema and statistics before
blaming the query. Leadline parses plan shapes; it does not model costs,
buffers, or execution.
