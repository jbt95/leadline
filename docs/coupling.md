# Temporal (change) coupling

`leadline coupling` finds files that repeatedly change in the same commits.
Static imports show intended dependencies; co-change shows what actually moves
together in practice: a service and its validator, a component and its
snapshot test, two parallel implementations of one concept, or a config file
and the parser that reads it.

```console
leadline coupling src/payment/PaymentService.ts
leadline coupling src/payment/PaymentService.ts --min-cochanges 1 --top 50
leadline coupling src/payment/PaymentService.ts --json
leadline coupling src/payment/PaymentService.ts --format agent-json
```

Example:

```text
Historically related files (target: src/payment/PaymentService.ts)

src/payment/PaymentValidator.ts      83%   co-changes 15 of 18   jaccard 71%
src/payment/PaymentTypes.ts          72%   co-changes 13 of 18   jaccard 62%
src/checkout/Checkout.ts             61%   co-changes 11 of 18   jaccard 42%
```

## Measures

For target `A` and related file `B`:

| Field | Formula | Question it answers |
| --- | --- | --- |
| `commits` | commits touching `B` | How active is `B`? |
| `co_changes` | commits touching both | Raw evidence |
| `directional` | `co_changes / commits(A)` | When I change `A`, how likely is `B`? |
| `reverse_directional` | `co_changes / commits(B)` | When `B` changes, does `A` usually follow? |
| `jaccard` | `co_changes / (commits(A) + commits(B) - co_changes)` | Symmetric overlap |

Example: `A` changed in 20 commits, `B` in 15, together 12. Directional
`A -> B` is `12/20 = 60%`, directional `B -> A` is `12/15 = 80%`, and Jaccard
is `12 / (20 + 15 - 12) = 52%`.

Rows are ranked by `directional`, then `co_changes`, then path. The default
`--min-cochanges 2` suppresses one-off coincidences; pass
`--min-cochanges 1` to see everything. `--top N` caps the list (default 20)
and `truncated` marks a capped JSON response.

## How it is calculated

- One streamed `git log --relative` walk over the analysis scope, shared with
  `leadline hotspots`. Merge commits are excluded; recency is not applied, so
  coupling covers the full available history. Like `hotspots`, a run starts a
  HEAD probe and then the walk, so two `git` subprocesses run in total.
- Renames resolve to the file's current path, so a moved file keeps its
  co-change history.
- Commits touching more than 50 files (`max_commit_files` in JSON) are
  excluded from pair counting because formatting sweeps and dependency bumps
  would otherwise create thousands of spurious pairs. They still count toward
  each file's `commits` total.
- A directory outside a Git repository reports `git_available: false` with an
  empty list; it is never an error. The target must exist inside the analysis
  scope: a path outside it is a usage error, not an empty result.

## How to use it

- Before editing a file, ask what else usually moves with it and inspect those
  files for hidden contracts the type system cannot see.
- When a PR changes only one side of a strongly coupled pair, check whether
  the other side needs the same change.
- After a refactor, coupling between the old pair should weaken; if it does
  not, the dependency probably still exists somewhere.

## Limitations

- **Co-change is not dependency.** Shared commits can come from a release
  process, a test fixture edit, a formatting sweep, or an unrelated rename.
  Treat it as an inspection hint, never as build-time truth.
- **No time window yet.** Coupling reflects the full available history,
  including old regimes. A file pair that stopped changing together years ago
  can still rank high.
- **Generated files distort results.** A checked-in generated file moves with
  its generator by construction; exclude or ignore those paths when reading
  the report.
- **Thresholds are heuristics.** The 50-file cap and the minimum co-change
  count are pragmatic defaults, not statistical guarantees.
- **Contributors are not involved.** Coupling never reports who made the
  changes; it is a code signal, not a people signal.

## Ethical guardrail

Do not use co-change, commit, or churn data to rank developers. These signals
describe code risk, maintenance patterns, and architecture understanding.
They are not employee performance measurements.
