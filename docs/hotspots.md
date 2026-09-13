# Hotspots and Git history

`leadline hotspots` combines static source metrics with Git history to answer:
which files are both complex and frequently changed? A file that is complex but
untouched for years is usually less urgent than moderately unhealthy code that
is edited every week.

```console
leadline hotspots
leadline hotspots --limit 20
leadline hotspots --since 30d
leadline hotspots --json
leadline hotspots --format agent-json
leadline hotspots src/payment --lcov coverage/lcov.info
```

The terminal report lists the ranked files and their dimensions. JSON keeps
every dimension, the selected window, the model name, and the Git reference.

## Dimensions

| Field | Meaning |
| --- | --- |
| `loc` | Sum of function LOC in the file (not raw file lines). |
| `functions` | Number of analyzed functions. |
| `max_cognitive`, `max_cyclomatic` | Worst function in the file. |
| `max_crap` | Worst CRAP when coverage was supplied; `null` otherwise. |
| `coverage`, `functions_with_coverage` | LOC-weighted mean over functions that have coverage data; the count keeps partial coverage honest. |
| `commits` | Commits that touched the file, after rename resolution. |
| `changes`, `changes_30d/90d/365d` | Commits in the selected window and in each fixed window. |
| `lines_added`, `lines_deleted` | Total added/deleted lines across history (binary files count zero lines). |
| `days_since_last_change` | Days between the file's last change and the HEAD commit. |
| `contributors`, `recent_contributors` | Distinct author identities overall and within 90 days. |
| `score` | `max_cognitive x changes`; `null` without Git history. |

## How the Git facts are calculated

- One `git log --relative --no-merges --numstat -z -M30%` walk per run,
  parsed as a stream. `--relative` scopes the walk to the requested directory
  and prints paths relative to it, matching the analyzer's paths.
- Merge commits are excluded. They do not represent authored changes, and
  `--no-merges` keeps contributor and churn counts from inflating.
- Recency windows are relative to the **HEAD commit time**, not the wall
  clock. The same repository snapshot produces the same report on any day.
- Renames are resolved newest to oldest, so a moved file keeps its history
  under its current path. The 30% similarity threshold is below git's 50%
  default because small moved files otherwise read as delete plus add.
- Contributors are identities, not people: the mailmap-applied author email
  (lowercased) is the key, with the author name as fallback.
- A directory outside a Git repository, an unborn HEAD, or a machine without
  `git` produces `"git_available": false` and `null` churn fields instead of
  an error. Complexity dimensions still work.

## The hotspot score

`score = max_cognitive x changes in the selected window`
(model name `complexity-x-churn-v1`).

This is the classic complexity-times-change-frequency heuristic. It is
deliberately transparent: every factor is in the same row, and the score adds
no information the dimensions do not have. It exists to order a list, not to
be an objective. Composite risk models (Milestone D) are separate, versioned,
and explainable.

## What to use it for

- "What should we refactor first?" - highest score with weak coverage.
- "Which code is complex and changing?" - sort by score, inspect dimensions.
- "Where is knowledge concentrated?" - high `max_cognitive` with
  `recent_contributors` of one.
- "What risks did this change introduce?" - run `hotspots` before and after,
  or use `diff` for changed functions.

## Limitations

- **File level only.** Churn is not attributed to individual functions in this
  milestone. A file's worst function is shown, but recent changes may not
  touch that function.
- **Rename heuristics.** At a 30% similarity threshold, two unrelated small
  files with common boilerplate can be paired; git's default 50% threshold
  misses small moves instead. Cross-directory-scope renames are an add or a
  delete because only one side is in scope. Commits wider than git's rename
  limit (default 1000 files) fall back to add/delete for renamed files.
- **Unknown churn is `null`.** A `null` churn field means either that Git
  history is unavailable (`git_available: false`) or that the file has no
  history record (new or untracked file); the report-level flag distinguishes
  the two.
- **Contributor identity is approximate.** Shared emails, bots, salary
  changes, and inconsistent mailmap entries distort counts.
- **Recency is relative to HEAD**, so an old checkout reports old windows.
  This is intentional for reproducibility.
- **Score is a proxy.** Two files with the same score are not equally risky.
  Use it to prioritize inspection, never as a quality gate by itself.

## Ethical guardrail

Do not use commit count, churn, or ownership metrics to rank developers.
They exist to understand code risk, knowledge concentration, and maintenance
patterns. Contributor counts are organizational risk signals, not employee
performance measurements, and the tool never reports named contributor
rankings.
