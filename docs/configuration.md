# Configuration

Optional file: `leadline.toml` in the analysis root. CLI flags override file values. Unknown keys are a config error (exit `2`).

```toml
[analysis]
exclude = ["docs/**", "fixtures/**"]

[metrics]
cognitive = 15
cyclomatic = 10
max_nesting = 4
crap = 30.0

[thresholds.function]
"src/legacy/*" = { cyclomatic = 20 }
```

## `[analysis]`

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `exclude` | list of glob strings | built-in generated/vendor list | Extra paths to skip during discovery. Explicit CLI file paths still analyze. |

Built-in skips (fixed directory names) always apply: `.git`, `node_modules`, `target`, `dist`, `build`, `coverage`, `.next`, `.gradle`, `vendor`, `generated`.

## `[metrics]`

Default thresholds for `check`. Any subset of `cognitive`, `cyclomatic`, `max_nesting` (integers) and `crap` (float). A function violates when its value exceeds the limit. A `crap` limit also fails functions whose coverage is unknown.

## `[thresholds.function]`

Per-path overrides. Each key is a glob; each value is a table with the same keys as `[metrics]`. The most specific matching glob wins; ties resolve alphabetically. Values must be non-negative numbers.
