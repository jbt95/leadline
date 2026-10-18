# Quality gate, ratings, and effort

`leadline` can turn a `check` run into a pass/fail verdict in SonarQube's
vocabulary — A–E ratings and remediation minutes — without a rule catalog.
Everything is derived from the metrics the run already computes, stays
deterministic, and needs no network.

## Remediation effort: the `metric-overage` model

Every unit a function exceeds a configured threshold costs a fixed number of
minutes:

| Metric | Minutes per unit over the limit |
| --- | --- |
| `cognitive` | 8 |
| `cyclomatic` | 4 |
| `max_nesting` | 10 |
| `crap` | 12 (per point, rounded up) |

A function contributes nothing for a dimension it is under, and nothing for a
suppressed dimension (see [in-source suppression](#in-source-suppression)).
Function effort is the sum over dimensions; the report's effort is the sum over
the gated set.

A CRAP gate without a coverage record fails the function but contributes no
minutes: the overage is unknown, and `crap_unavailable` names it instead of a
guessed number.

The model name `metric-overage` is carried in every effort number.

## Ratings

`A` is best. `E` is worst. `null` always means unknown, never zero.

| Rating | Maintainability (effort / assumed cost) | Security (worst ingested severity) |
| --- | --- | --- |
| `A` | ratio <= 0.05 | no ingested findings |
| `B` | ratio <= 0.10 | at least one `low` |
| `C` | ratio <= 0.20 | at least one `medium` |
| `D` | ratio <= 0.50 | at least one `high` |
| `E` | ratio > 0.50 | at least one `critical` |

- **Assumed cost** is `loc * 30` minutes: one line of code is assumed to cost
  30 minutes to develop, so the ratio is the share of the codebase's assumed
  cost that is remediation debt. The constant is `MINUTES_PER_LINE` in
  `src/quality.rs`.
- **Maintainability** needs at least one configured threshold; with none there
  is no overage and the rating stays `null`.
- **Security** counts every ingested finding in the run's scanner families
  (`--sarif`, `--osv`, `--trivy`) — gated or not, because the rating describes
  what the scanners reported, not what the gate was asked to fail on. A run
  whose findings all carry unreadable severities stays `null`.
- **Reliability** is always `null`: `check` ingests no bug-severity source, and
  an unknown signal is never guessed into a letter. `mutation` reports carry
  mutation scores for the same reason, but `check` does not read them.

## `[quality_gate]`

```toml
[quality_gate]
name = "team-default"
maintainability_rating = "B"
security_rating = "A"
effort_minutes = 600
```

| Key | Type | Meaning |
| --- | --- | --- |
| `name` | string | Reported verbatim; defaults to `default`. Must not be empty. |
| `maintainability_rating` | `A`–`E` (case-insensitive) | Highest allowed rating; a worse rating fails. |
| `security_rating` | `A`–`E` (case-insensitive) | Highest allowed rating; a worse rating fails. |
| `effort_minutes` | integer >= 0 | Highest allowed total effort in the gated set. `0` means "no new debt". |

At least one condition is required. A condition with no input to evaluate is a
usage error (exit `2`), never a silently passing no-op: the maintainability
rating and effort need at least one threshold, and the security rating needs
`--sarif` or `--osv`/`--trivy`.

The gate adds `quality_gate` to `check --json`, `check --format agent-json`,
and every `--format <ci-format>` document, prints a verdict line in terminal
output, and flips the markdown/`github-summary` document's verdict when the
gate failed. `report --from` reads the block back, so a re-rendered saved run
agrees with the live run.

## Gate scope

| `check` invocation | Gated set | `scope` |
| --- | --- | --- |
| plain | every analyzed function | `overall` |
| `--base REV` | after-side functions of the changed set | `changed` |
| `--baseline FILE` | the whole current report (a function the snapshot lacks counts as new) | `overall` |

Effort, ratings, and the condition checks are computed over the gated set
before the report is filtered down to violations, so a clean run still rates
honestly.

## Output

```json
"quality_gate": {
  "name": "team-default",
  "status": "failed",
  "scope": "changed",
  "conditions": [
    {"metric": "maintainability_rating", "operator": "<=", "threshold": "B", "actual": "C", "status": "failed"},
    {"metric": "effort_minutes", "operator": "<=", "threshold": 600, "actual": 812, "status": "failed"}
  ],
  "ratings": {"maintainability": "C", "reliability": null, "security": "A"},
  "effort": {
    "model": "metric-overage",
    "minutes": 812,
    "functions": 12,
    "by_metric": {"cognitive": 640, "cyclomatic": 172}
  },
  "suppressed_violations": 3
}
```

- Conditions are reported in the order `maintainability_rating`,
  `security_rating`, `effort_minutes`, and only for the conditions the gate
  declares.
- A condition on an uncomputable value fails closed — the same rule a CRAP gate
  without coverage follows.
- `suppressed_violations` counts violation reasons silenced by suppression
  lines, per dimension.
- `by_metric` omits metrics that contributed nothing.

## Exit codes

`check` exits `1` when any metric violation, any scanner violation, or any
failed gate condition exists; `0` when all are clean. `quality_gate.status` is
the gate's own verdict, so it can read `passed` while the run exits `1` because
a threshold violation is still a finding. SARIF output carries findings only:
a gate verdict has no SARIF shape and is not emitted there.

## In-source suppression

| Marker | Effect |
| --- | --- |
| `NOSONAR` (case-insensitive) | Silences `cognitive`, `cyclomatic`, `max_nesting`, and `crap` for its function. |
| `leadline: ignore cognitive, cyclomatic` | Silences the named dimensions only. Unknown names are ignored. |

Comment syntax is the language's own, and the marker may appear anywhere inside
the comment:

```ts
function hairy(x: boolean) { if (x) { return 1; } return 0; } // NOSONAR
```

```python
# leadline: ignore cognitive, max_nesting
def hairy(x):
    ...
```

```c
/* leadline: ignore cyclomatic */
int hairy(int x) { ... }
```

A suppression comment applies to a function when:

- it lies inside the function's line span and no nested function's span
  contains it (a marker inside a nested function belongs to that function
  only), or
- it ends on the line directly above the function's first line.

Suppression is never silent: the function row carries `suppressed_metrics`
(omitted when empty), and the gate counts `suppressed_violations`. Parse errors
are not metric reasons, so no suppression line can hide one.

## Surfaces

| Surface | Behavior |
| --- | --- |
| CLI `check` | Everything above. |
| MCP `check` | Suppression is honored, and a configured gate is evaluated and returned as `quality_gate`; it can fail `passed` on its own. MCP `check` ingests no scanner reports, so a `security_rating` condition is a usage error (`-32602`) there rather than a permanently failing gate. |
| `report --from` | Reads the saved `quality_gate` block; a failed gate makes the re-rendered verdict fail, matching the live run. |
| `stats` / `project` | Not gated: they model the repository, they do not gate a change. |
