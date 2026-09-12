# Configuration

Optional file: `leadline.toml` in the analysis root. CLI flags override file values. Unknown keys are a config error (exit `2`).

```toml
[analysis]
exclude = ["docs/**", "fixtures/**"]

[metrics]
cyclomatic_profile = "default-v1"
cognitive_profile = "default-v1"

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
```

## `[analysis]`

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `exclude` | list of glob strings | built-in generated/vendor list | Extra paths to skip during discovery. Explicit CLI file paths still analyze. |

Built-in skips (fixed directory names) always apply: `.git`, `node_modules`, `target`, `dist`, `build`, `coverage`, `.next`, `.gradle`, `vendor`, `generated`.

## `[metrics]`

Metric profiles. Both profile keys must be `"default-v1"`.

## `[thresholds.function]`

Absolute limits for `check`. Any subset of `cognitive`, `cyclomatic`, `max_nesting` (integers), and `crap` (float) is allowed. A value fails when it exceeds the limit. Unknown CRAP also fails a CRAP gate.

## `[regressions]`

Allowed positive deltas for `check --base REV --regressions` and `check --baseline FILE --regressions`. Values are non-negative. Missing values default to zero. Only paired functions are checked; added and removed functions are ignored. Absolute thresholds and regression limits can run together. A failure in either gate exits `1`.

## Baselines

Snapshots gate refactors that Git history cannot see (vendored drops, rewrites, generated code). The workflow is explicit:

```console
leadline baseline . --output .leadline-baseline.json
git diff .leadline-baseline.json   # review what you pin
git add .leadline-baseline.json    # commit the reviewed snapshot
leadline check . --baseline .leadline-baseline.json --regressions
```

`baseline` writes `schema_version`, `metric_profile`, and one row per function (path, stable id, name, line, gate metrics), sorted for deterministic diffs. Writes are atomic. `check --baseline` reuses the `[regressions]` limits above; new functions fail only on absolute thresholds.
