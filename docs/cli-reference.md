# CLI reference

Binary: `leadline`. Every command prints terminal text by default, JSON with `--json`.

## Commands

```console
leadline analyze [PATH] [--json] [--format agent-json] [--lcov FILE] [--jacoco FILE]
leadline function FILE NAME [--json] [--format agent-json] [--lcov FILE] [--jacoco FILE]
leadline check [PATH] [--cognitive N] [--cyclomatic N] [--crap N] [--max-nesting N] [--json] [--lcov FILE] [--jacoco FILE]
leadline changed [--base REV] [--path PATH] [--json]
leadline diff [REV] [--path PATH] [--json]
leadline --version
```

- `analyze`: all supported functions under `PATH` (default `.`), or one file. Output sorted by path, functions in source order.
- `function`: functions named `NAME` in `FILE` only. Errors when the name is absent.
- `check`: like `analyze`, but keeps only violations and parse errors. Requires at least one threshold flag.
- `changed` / `diff`: functions changed between base revision and working tree. `changed` takes `--base REV` (default `HEAD~1`); `diff` takes the revision positionally. Both accept `--path` to scope to a file or directory.
- `--version` / `-V`: print `leadline <version>`.

`version`, `doctor`, `mcp`, and `integrate` subcommands are deferred and not available in 1.0. `--format agent-json` selects the agent-oriented JSON shape (see `agent-integration-guide.md`).

## Flags

| Flag | Commands | Meaning |
| --- | --- | --- |
| `--json` | all analysis | Emit JSON report instead of terminal text. |
| `--format agent-json` | analyze, function, check | Emit the compact agent-oriented JSON shape. |
| `--lcov FILE` | analyze, function, check | Merge LCOV line coverage. Repeatable. |
| `--jacoco FILE` | analyze, function, check | Merge JaCoCo XML line coverage. Repeatable. |
| `--base REV` | changed | Base revision (default `HEAD~1`). |
| `--path PATH` | changed, diff | Scope changed analysis to a file or directory. |

`check` thresholds fail only when a value exceeds its limit. A `--crap` threshold also fails when coverage (and therefore CRAP) is unavailable for a function.

## Exit codes

| Code | Meaning |
| --- | --- |
| `0` | Success, or quality gate passed. |
| `1` | Quality gate failed (violations or parse errors under `check`). |
| `2` | Usage or config error (unknown flag, missing threshold, bad path). |
| `3` | Analysis incomplete (Git failure, unreadable input, unsupported extension). |
| `4` | Coverage input error (unreadable or unparsable LCOV / JaCoCo file). |
| `5` | Internal error. |
