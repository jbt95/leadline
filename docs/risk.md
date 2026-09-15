# Change risk

`leadline risk` answers: which files combine hard-to-understand code, weak
test backing, frequent editing, wide blast radius, concentrated knowledge,
and architecture violations? It joins static source metrics, Git history
facts, ownership concentration, the static dependency graph, and
architecture policy into one per-file score with explicit components
(model `change-risk`).

```console
leadline risk
leadline risk --limit 20 --since 30d
leadline risk --json
leadline risk --format agent-json
leadline risk src/payment --lcov coverage/lcov.info
```

The terminal report lists the top files with their score, components, and
raw dimensions. JSON keeps every dimension, the selected window, the model
name, and the Git signal. The score exists to order a list, not to be an
objective: every factor is in the same row next to it.

## Components

All components are on a 0-100 scale. `null` means unknown, never zero (see
null-renormalization below).

| Component | Formula | Weight | `null` when |
| --- | --- | --- | --- |
| `complexity` | `100 * max(min(max_cognitive / 30, 1), min(max_cyclomatic / 20, 1))` | 20 | never (`Some` always) |
| `crap` | `100 * min(max_crap / 30, 1)` | 15 | no coverage for any function in the file |
| `churn` | `100 * min(changes_in_window / 20, 1)` | 20 | file has no history row (new/untracked, or no Git) |
| `impact` | `min(blast_radius_percent, 100)` | 20 | never (`Some` always; `0.0` when the file has no graph node) |
| `ownership` | touch concentration percent | 10 | file has no touches (no Git, or file outside the analyzed paths) |
| `policy` | highest unresolved source severity: error 100, warning 60, info 30 | 15 | never (`Some(0.0)` when no rule fires) |

- `max_cognitive` / `max_cyclomatic` are the worst per-metric function values
  in the file (they may come from different functions). A file with no
  functions scores `complexity = 0.0`.
- `max_crap` is the worst (highest) function CRAP in the file.
- `changes_in_window` is the file's `changes_30d`, `changes_90d`, or
  `changes_365d` matching `--since`.
- `blast_radius_percent` is the `impact` value over the graph scope:
  `blast_radius / (scope_files - 1) * 100` (`0.0` when
  `scope_files <= 1`). `scope_files` equals `files_analyzed` except when
  analysis covers a sub-scope such as a single file. Caps: complexity ratios saturate at cognitive 30 /
  cyclomatic 20, CRAP at 30, churn at 20 changes, impact at 100.
- The fixed caps are heuristics, not thresholds: they stop one extreme
  dimension from dominating, and they saturate rather than fail.

## Score and null-renormalization

`score = sum(weight_i * component_i) / sum(weight_i)` over the known
(`Some`) components only. Unknown components are skipped, not zeroed, so a
file without coverage or history is scored on what is known.

Worked example (from `tests/risk.rs`): a file with `complexity = 100`,
`crap = 100`, `churn = 100`, `impact = 50`, `ownership = 80`,
`policy = 0.0` scores
`(20*100 + 15*100 + 20*100 + 20*50 + 10*80 + 15*0) / 100 = 73.0`.
The same file with no coverage, no history row, and no touches keeps only
complexity, impact, and policy: `(20*100 + 20*50 + 15*0) / 55 = 54.55`.

## Ownership concentration and guardrail

`ownership` is the top identity's share of file touches (0-100) from the
Git walk with the target `.mailmap` applied: a file touched only by one
identity scores 100, an evenly split two-identity file scores 50. The
reading is knowledge concentration ("how few people have touched this
file?"), not authorship quality, and the tool reports the percent only —
never named contributor rankings. The measure misranks solo-author files
that are well understood; weight 10 bounds that damage. Do not use commit
count, churn, or ownership metrics to rank developers.

## Policy

`policy` is the highest severity among unresolved violations whose source
is the file: error 100, warning 60, info 30. Violations with status
`resolved` are ignored. With no architecture rules configured (or no rule
firing), the component is `Some(0.0)`: the absence of violations is known,
not unknown. The raw row also carries `policy_severity` (`"info"` /
`"warning"` / `"error"`, absent when nothing fires).

## Windows

`--since 30d|90d|365d` (default `90d`) selects the churn window and the
report's `window` field. Windows mirror `hotspots`: they are relative to the
HEAD commit time, never the wall clock, so the same snapshot produces the
same report. Merge commits are excluded and renames resolve newest to oldest
via the shared history walk. A directory outside a Git repository, an unborn
HEAD, or a machine without `git` reports `git_available: false` with `null`
churn/ownership and still ranks by the static dimensions.

## Ranking and output caps

Rows sort by `score` descending, ties by `path` ascending. The report keeps
the full ranking: `--json` always emits every file. `--limit N` (default
10, `N >= 1`) caps rows only for the terminal report and agent JSON, which
print a `raise --limit` hint when rows are hidden. `--json` and
`--format agent-json` are exclusive; `--format` accepts only `agent-json`.

## Exit status

`risk` is informational and exits `0` on success. It never gates: score
regressions are compared by `debt --fail-on-regression` (diff intelligence),
which exits `1` when risk increases. Usage errors exit `2`
(unknown flag, bad `--since`, `--limit 0`, `--json` with `--format`),
an empty scope exits `3`, and unreadable coverage exits `4`.

## Limitations

- **File level only.** Churn, ownership, and impact are per-file facts; the
  worst function in the file is shown, but recent changes may not touch that
  function.
- **Fixed caps are heuristics.** Cognitive 30, cyclomatic 20, CRAP 30, and
  20 changes saturate by design; two saturated files can hide very different
  extremes.
- **Impact cost on huge scopes.** Risk runs one reverse-BFS per file over a
  single shared reverse map (`O(files x (V+E))` reachability work, with map
  lookups hoisted); prefer a scoped path on very large repositories.
- **No coupling input.** Co-change evidence (`coupling`) is not a component;
  inspect it separately before editing.
- **Static graph limits apply.** Relative JS/TS imports and exact Java type
  imports only; aliases, package graphs, reflection, and runtime-built
  specifiers are invisible to `impact` (see `dependencies.md`).
- **Contributor identity is approximate.** Shared emails, bots, and mailmap
  drift distort `ownership`, as with `hotspots`.
