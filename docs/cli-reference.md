# CLI reference

Binary: `leadline`. Every command prints terminal text by default, JSON with `--json`.

## Commands

```console
leadline analyze [PATH] [--json] [--format agent-json|sarif] [--top N] [--sort-by KEY] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE] [--cache-dir DIR]
leadline function FILE NAME [--json] [--format agent-json] [--explain] [--top N] [--sort-by KEY] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE]
leadline check [PATH] [--base REV | --baseline FILE] [--regressions] [--cognitive N] [--cyclomatic N] [--crap N] [--max-nesting N] [--json] [--format agent-json|sarif] [--top N] [--sort-by KEY] [--min-crap X] [--lcov FILE] [--jacoco FILE] [--coverage FILE] [--cache-dir DIR]
leadline changed [--base REV] [--staged | --target REV] [--renames] [--path PATH] [--json] [--format agent-json] [--explain] [--top N] [--sort-by KEY] [--min-crap X] [--min-delta D]
leadline diff [REV] [--staged | --target REV] [--renames] [--path PATH] [--json] [--format agent-json] [--explain] [--top N] [--sort-by KEY] [--min-crap X] [--min-delta D]
leadline test-targets [PATH] (--coverage FILE | --lcov FILE | --jacoco FILE) [--top N] [--format agent-json]
leadline baseline [PATH] --output FILE [--lcov FILE] [--jacoco FILE] [--coverage FILE]
leadline --version
leadline version
leadline doctor [PATH]
leadline mcp
leadline skill
```

- `analyze`: all supported functions under `PATH` (default `.`), or one file. Output sorted by path, functions in source order.
- `function`: functions named `NAME` in `FILE` only. Errors when the name is absent. `--explain` appends one terminal line per metric contribution (`<rule> line <line> nesting <nesting> +<cog> cog +<cyc> cyc`); both JSON shapes already carry `contributions`, and `--format agent-json` includes the array only with `--explain`.
- `check`: like `analyze`, but keeps only violations and parse errors. It requires at least one threshold flag unless `--regressions` is selected. With `--base REV`, gates changed functions between the revision and the working tree. With `--baseline FILE`, gates current functions against a saved snapshot instead (exclusive with `--base`): paired functions fail on absolute or (with `--regressions`) delta violations, new functions fail only on absolute thresholds, and deleted functions are ignored. `--regressions` applies configured positive-delta limits to paired functions; added and removed functions are excluded. Absolute and delta gates combine, and either failure exits `1`.
- `changed` / `diff`: functions changed between a base revision and a comparison target (default the working tree). `changed` takes `--base REV` (default `HEAD~1`); `diff` takes the base revision positionally. `--staged` compares against the index instead of the working tree; `--target REV` compares against another revision instead; the two are mutually exclusive (exit `2`). `--renames` enables Git file rename detection (`-M`), pairing old-path content with new content under the new path. `--explain` adds deterministic multiset-added contribution causes, with after-side lines, to regression rows in agent JSON. Both commands accept `--path` to scope to a file or directory.
- `test-targets`: rank functions holding decision lines with known zero line-coverage hits, sorted by CRAP descending, then path/function/line. Coverage is required (exit `2` without it). Rows carry `uncovered` (known-zero-hit) and `unknown` (absent from the coverage record) contribution lines; unknown is never called uncovered. Output is capped at `--top N` (default 200). Coverage is line coverage, not branch coverage.
- `baseline`: snapshot current function metrics to `FILE` (writes atomically via a sibling temp file, then rename). Review the file, commit it, and gate later edits with `check --baseline FILE --regressions`. Snapshot rows pair with current functions by path and name in same-name source order; parsing rejects unknown schemas, unknown metric profiles, and duplicate identities (exit `3`).
- `--version` / `-V` / `version`: print `leadline <version>`.
- `doctor`: self-check parsers, coverage readers, `git`, and `leadline.toml`.
- `mcp`: serve the read-only MCP tool API over stdio.
- `skill`: print the canonical agent skill (`integrations/common/leadline-skill/SKILL.md`, baked into the binary).

## Flags

| Flag | Commands | Meaning |
| --- | --- | --- |
| `--json` | all analysis | Emit JSON report instead of terminal text. Exclusive with any `--format`. |
| `--format agent-json` | analyze, function, check, changed, diff, test-targets | Emit the compact agent-oriented JSON shape. Carries a `truncated` bool when budget flags drop entries. |
| `--format sarif` | analyze, check | Emit SARIF 2.1.0 (`version` / `runs` / `results`), violations only. Exclusive with `--json`. |
| `--top N` | analyze, function, check, changed, diff, test-targets | Keep at most `N` entries per list (`test-targets` default cap is 200). Only with `--format agent-json` (`N >= 1`), except `test-targets` where it also caps terminal output. |
| `--sort-by KEY` | analyze, function, check, changed, diff | Sort budget entries by `crap`, `cognitive`, or `cyclomatic`. Only with `--format agent-json`. |
| `--min-crap X` | analyze, function, check, changed, diff | Drop entries below CRAP `X`. Only with `--format agent-json`. |
| `--min-delta D` | changed, diff | Drop entries whose max before/after delta is below `D`. Only with `--format agent-json`. |
| `--explain` | function, changed, diff | Add contribution details to function agent JSON, or multiset-added causes to changed regression rows. |
| `--cache-dir DIR` | analyze, check | Reuse per-file analysis from `DIR/file-cache.json` across runs; saved at end. Bypassed when any coverage flag is passed (cached functions are pre-coverage, so CRAP would go stale). Cache I/O failures warn on stderr, never fail. |
| `--lcov FILE` | analyze, function, check, test-targets | Merge LCOV line coverage. Repeatable. Required on `test-targets` (one coverage flag at minimum). |
| `--jacoco FILE` | analyze, function, check, test-targets | Merge JaCoCo XML line coverage. Repeatable. Required on `test-targets` (one coverage flag at minimum). |
| `--coverage FILE` | analyze, function, check, test-targets | Merge coverage with format detected from the extension (`.info` LCOV, `.xml` JaCoCo). Repeatable. Required on `test-targets` (one coverage flag at minimum). |
| `--regressions` | check | Enable regression-only gates. Limits come from `[regressions]` and default to zero. It removes the absolute-threshold requirement; combine with `--base REV` or `--baseline FILE` to compare changed functions. |
| `--base REV` | check, changed | Base revision (`changed` default `HEAD~1`). Exclusive with `--baseline` on `check`. |
| `--baseline FILE` | check | Saved snapshot for regression gates without Git. Exclusive with `--base`. |
| `--output FILE` | baseline | Snapshot destination (required). |
| `--staged` | changed, diff | Compare `--base`/`REV` against the index (staged blobs) instead of the working tree. Exclusive with `--target`. |
| `--target REV` | changed, diff | Compare `--base`/`REV` against another revision instead of the working tree. Exclusive with `--staged`. |
| `--renames` | changed, diff | Detect Git file renames (`-M`) and pair a renamed file's old content with its new content under the new path. |
| `--path PATH` | changed, diff | Scope changed analysis to a file or directory. |

Budget flags with `--json` or default terminal output are a usage error (exit `2`), never silently ignored. `--top 0` and unparsable budget values are usage errors.

`check` thresholds fail only when a value exceeds its limit. A `--crap` threshold also fails when coverage (and therefore CRAP) is unavailable for a function.

`--format sarif` on `check` uses the command thresholds; on `analyze` it uses `leadline.toml` thresholds when present, else empty thresholds (rules listed, empty results).
