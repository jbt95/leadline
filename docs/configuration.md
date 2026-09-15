# Configuration

Optional file: `leadline.toml` in the analysis root. CLI flags override file values, and MCP tools read the same file from their `path` argument's analysis root, so `[analysis]`, `[sql]`, and `[vulnerabilities]` behave identically on both surfaces. Unknown keys are a config error (exit `2`).

```toml
[analysis]
exclude = ["docs/**", "fixtures/**"]

[metrics]
cyclomatic_profile = "default"
cognitive_profile = "default"

[thresholds.function]
cognitive = 15
cyclomatic = 10
max_nesting = 4
crap = 30.0

[regressions]
cognitive = 1
cyclomatic = 0
max_nesting = 0
crap = 0.0

[duplication]
min_tokens = 100
min_lines = 10
exclude = ["generated/**"]

[[architecture.rules]]
name = "domain-no-ui"
source = "src/domain/**"
deny = ["src/ui/**"]
severity = "error"
```

## `[analysis]`

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `exclude` | list of glob strings | built-in generated/vendor list | Extra paths to skip during discovery. Explicit CLI file paths still analyze. |

Built-in skips (fixed directory names) always apply: `.git`, `node_modules`, `target`, `dist`, `build`, `coverage`, `.next`, `.gradle`, `vendor`, `generated`.

## `[metrics]`

Metric profiles. Both profile keys must be `"default"`.

## `[thresholds.function]`

Absolute limits for `check`. Any subset of `cognitive`, `cyclomatic`, `max_nesting` (integers), and `crap` (float) is allowed. A value fails when it exceeds the limit. Unknown CRAP also fails a CRAP gate.

## `[vulnerabilities]`

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `minimum_severity` | `low` \| `medium` \| `high` \| `critical` | none (informational) | Gate floor for `vulnerabilities` and `check --osv/--trivy`. `--fail-on-severity` overrides it for one invocation. `unknown` is rejected. |

## `[sql]`

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `large_offset` | integer >= 1 | `1000` | Numeric top-level `OFFSET` above this fails `sql/large-offset`. `--large-offset` overrides it for one invocation. Zero is rejected. |
| `migration_roots` | list of relative paths | empty (no schema evidence) | Migration directories declaring tables for `sql/unknown-table`. Roots must be relative with `/` separators; duplicates are rejected. `--migration-root` overrides the list for one invocation. |

## `[regressions]`

Allowed positive deltas for `check --base REV --regressions` and `check --baseline FILE --regressions`. Values are non-negative. Missing values default to zero. Only paired functions are checked; added and removed functions are ignored. Absolute thresholds and regression limits can run together. A failure in either gate exits `1`.

## `[duplication]`

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `min_tokens` | integer >= 1 | `100` | Minimum normalized token sequence for a clone. |
| `min_lines` | integer >= 1 | `10` | Minimum line span of every occurrence. |
| `exclude` | list of glob strings | none | Extra paths skipped in addition to `[analysis].exclude`. |

Defaults match SonarQube's non-Java clone gate (100 tokens, 10 lines); lower them to catch smaller clones. Java keeps token-based detection (Sonar Java counts 10 statements instead — a different unit, intentionally not matched).

## `[[architecture.rules]]`

Ordered deny rules over resolved dependency edges. Each rule needs `name` (unique), `source` (one glob), `deny` (at least one glob), and `severity` (`info`, `warning`, or `error`). Globs are analysis-root-relative gitignore-style patterns; negation, absolute/drive paths, trailing `/`, and escaping `..` are rejected. Violations classify `new`, `existing`, or `resolved` against `--base`.

## Baselines

Snapshots gate refactors that Git history cannot see (vendored drops, rewrites, generated code). The workflow is explicit:

```console
leadline baseline . --output .leadline-baseline.json
git diff .leadline-baseline.json   # review what you pin
git add .leadline-baseline.json    # commit the reviewed snapshot
leadline check . --baseline .leadline-baseline.json --regressions
```

`baseline` writes `schema_version`, `metric_profile`, and one row per function (path, stable id, name, line, gate metrics), sorted for deterministic diffs. Writes are atomic. `check --baseline` reuses the `[regressions]` limits above; new functions fail only on absolute thresholds.
