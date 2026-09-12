# CLI reference

Binary: `leadline`. Every command prints terminal text by default, JSON with `--json`.

## Commands

```console
leadline analyze [PATH] [--json] [--format agent-json|sarif] [--top N] [--sort-by KEY] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE] [--cache-dir DIR]
leadline function FILE NAME [--json] [--format agent-json] [--explain] [--top N] [--sort-by KEY] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE]
leadline check [PATH] [--base REV] [--cognitive N] [--cyclomatic N] [--crap N] [--max-nesting N] [--json] [--format agent-json|sarif] [--top N] [--sort-by KEY] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE] [--cache-dir DIR]
leadline changed [--base REV] [--path PATH] [--json] [--format agent-json] [--top N] [--sort-by KEY] [--min-crap X] [--min-delta D]
leadline diff [REV] [--path PATH] [--json] [--format agent-json] [--top N] [--sort-by KEY] [--min-crap X] [--min-delta D]
leadline --version
leadline version
leadline doctor [PATH]
leadline mcp
leadline skill
```

- `analyze`: all supported functions under `PATH` (default `.`), or one file. Output sorted by path, functions in source order.
- `function`: functions named `NAME` in `FILE` only. Errors when the name is absent. `--explain` appends one terminal line per metric contribution (`<rule> line <line> nesting <nesting> +<cog> cog +<cyc> cyc`); both JSON shapes already carry `contributions`, and `--format agent-json` includes the array only with `--explain`.
- `check`: like `analyze`, but keeps only violations and parse errors. Requires at least one threshold flag. With `--base REV`, gates only changed functions between the revision and the working tree (after-side violations in the same row format); exit `1` when non-empty, else `0`.
- `changed` / `diff`: functions changed between base revision and working tree. `changed` takes `--base REV` (default `HEAD~1`); `diff` takes the revision positionally. Both accept `--path` to scope to a file or directory.
- `--version` / `-V` / `version`: print `leadline <version>`.
- `doctor`: self-check parsers, coverage readers, `git`, and `leadline.toml`.
- `mcp`: serve the read-only MCP tool API over stdio.
- `skill`: print the canonical agent skill (`integrations/common/leadline-skill/SKILL.md`, baked into the binary).

## Flags

| Flag | Commands | Meaning |
| --- | --- | --- |
| `--json` | all analysis | Emit JSON report instead of terminal text. Exclusive with any `--format`. |
| `--format agent-json` | analyze, function, check, changed, diff | Emit the compact agent-oriented JSON shape. Carries a `truncated` bool when budget flags drop entries. |
| `--format sarif` | analyze, check | Emit SARIF 2.1.0 (`version` / `runs` / `results`), violations only. Exclusive with `--json`. |
| `--top N` | analyze, function, check, changed, diff | Keep at most `N` entries per list. Only with `--format agent-json` (`N >= 1`). |
| `--sort-by KEY` | analyze, function, check, changed, diff | Sort budget entries by `crap`, `cognitive`, or `cyclomatic`. Only with `--format agent-json`. |
| `--min-crap X` | analyze, function, check, changed, diff | Drop entries below CRAP `X`. Only with `--format agent-json`. |
| `--min-delta D` | changed, diff | Drop entries whose max before/after delta is below `D`. Only with `--format agent-json`. |
| `--explain` | function | Append per-contribution terminal lines; add `contributions` to agent-json entries. |
| `--cache-dir DIR` | analyze, check | Reuse per-file analysis from `DIR/file-cache.json` across runs; saved at end. Bypassed when any coverage flag is passed (cached functions are pre-coverage, so CRAP would go stale). Cache I/O failures warn on stderr, never fail. |
| `--lcov FILE` | analyze, function, check | Merge LCOV line coverage. Repeatable. |
| `--jacoco FILE` | analyze, function, check | Merge JaCoCo XML line coverage. Repeatable. |
| `--coverage FILE` | analyze, function, check | Merge coverage with format detected from the extension (`.info` LCOV, `.xml` JaCoCo). Repeatable. |
| `--base REV` | check, changed | Base revision (`changed` default `HEAD~1`). |
| `--path PATH` | changed, diff | Scope changed analysis to a file or directory. |

Budget flags with `--json` or default terminal output are a usage error (exit `2`), never silently ignored. `--top 0` and unparsable budget values are usage errors.

`check` thresholds fail only when a value exceeds its limit. A `--crap` threshold also fails when coverage (and therefore CRAP) is unavailable for a function.

`--format sarif` on `check` uses the command thresholds; on `analyze` it uses `leadline.toml` thresholds when present, else empty thresholds (rules listed, empty results).
