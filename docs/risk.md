# Change risk

`leadline risk` answers: which files combine hard-to-understand code, weak
test backing, frequent editing, wide blast radius, and concentrated
knowledge? It joins static source metrics, Git history facts, and the static
dependency graph into one per-file score with explicit components
(model `change-risk-v1`).

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
null-renormalization below). Weights sum to 100 across the five measured
components; `policy` carries weight 0 and does not redistribute.

| Component | Formula | Weight | `null` when |
| --- | --- | --- | --- |
| `complexity` | `100 * max(min(max_cognitive / 30, 1), min(max_cyclomatic / 20, 1))` | 25 | never (`Some` always) |
| `crap` | `100 * min(max_crap / 30, 1)` | 20 | no coverage for any function in the file |
| `churn` | `100 * min(changes_in_window / 20, 1)` | 20 | file has no history row (new/untracked, or no Git) |
| `impact` | `min(blast_radius_percent, 100)` | 20 | never (`Some` always; `0.0` when the file has no graph node) |
| `ownership` | `100.0 / contributors` | 15 | file has no history row, or `contributors == 0` |
| `policy` | always `null` in v1 | 0 | always |

- `max_cognitive` / `max_cyclomatic` are the worst per-metric function values
  in the file (they may come from different functions). A file with no
  functions scores `complexity = 0.0`.
- `max_crap` is the worst (highest) function CRAP in the file.
- `changes_in_window` is the file's `changes_30d`, `changes_90d`, or
  `changes_365d` matching `--since`.
- `blast_radius_percent` is the `impact-v1` value over the graph scope:
  `blast_radius / (scope_files - 1) * 100` (`0.0` when
  `scope_files <= 1`). `scope_files` equals `files_analyzed` except when
  analysis covers a sub-scope such as a single file. Caps: complexity ratios saturate at cognitive 30 /
  cyclomatic 20, CRAP at 30, churn at 20 changes, impact at 100.
- The fixed caps are heuristics, not thresholds: they stop one extreme
  dimension from dominating, and they saturate rather than fail.

## Score and null-renormalization

`score = sum(weight_i * component_i) / sum(weight_i)` over the known
(`Some`) components only. Unknown components are skipped, not zeroed, so a
file without coverage or history is scored on what is known. A later model
version carrying `policy` data will be `change-risk-v2`; v1 weights do not
shift to absorb the null.

Worked example (from `tests/risk.rs`): a file with `complexity = 100`,
`crap = 100`, `churn = 100`, `impact = 50`, `ownership = 50`,
`policy = null` scores
`(25*100 + 20*100 + 20*100 + 20*50 + 15*50) / 100 = 82.5`.
The same file with no coverage and no history row keeps only complexity and
impact: `(25*100 + 20*50) / 45 = 77.78`.

## Ownership proxy and guardrail

`ownership` is a contributor-count concentration proxy: `100.0 / n`, so a
solo-author file scores 100 and a four-contributor file scores 25. The
reading is knowledge concentration ("how few people have touched this
file?"), not authorship quality, and the tool reports counts only — never
named contributor rankings. The proxy misranks solo-author files that are
well understood; weight 15 bounds that damage. Do not use commit count,
churn, or ownership metrics to rank developers.

## Policy

`policy` is an explicit always-`null` v1 component with weight 0.0. It
reserves the slot (and the weight budget) for architecture-rule input in a
later model version without rescoring v1 reports.

## Windows

`--since 30d|90d|365d` (default `90d`) selects the churn window and the
report's `window` field. Windows mirror `hotspots`: they are relative to the
HEAD commit time, never the wall clock, so the same snapshot produces the
same report. Merge commits are excluded and renames resolve newest to oldest
via the shared history walk. A directory outside a Git repository, an unborn
HEAD, or a machine without `git` reports `git_available: false` with `null`
churn/ownership and still ranks by the static dimensions.

## Ranking and truncation

Rows sort by `score` descending, ties by `path` ascending.
`--limit N` (default 10, `N >= 1`) caps the shown rows across terminal,
JSON, and agent JSON; `truncated` is true when more files were analyzed than
shown. `--json` and `--format agent-json` are exclusive; `--format` accepts
only `agent-json`.

## Exit status

`risk` is informational and exits `0` on success. It never gates: quality
gating on risk regressions is deferred to Milestone E (diff intelligence),
which will keep these exit codes backward compatible. Usage errors exit `2`
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
