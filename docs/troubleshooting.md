# Troubleshooting

## Parse errors appear inline

A syntax error never aborts analysis. The file keeps its valid functions and lists diagnostics under `parse_errors` (`kind`, start/end line and column). Under `check`, files with parse errors count as findings (gate fails). Under `changed`, diagnostics appear on the `before`/`after` side where they occur.

## Exit codes

| Code | Cause | Fix |
| --- | --- | --- |
| `1` | Gate failed: violations or parse errors. | Fix the flagged functions or raise the threshold. |
| `2` | Usage/config: unknown flag, `check` without a threshold, bad `leadline.toml`, missing `--base`/`--path` value. | Read stderr; it names the option. |
| `3` | Analysis incomplete: missing path, unsupported extension, `git` failure, base revision rejected. | Check the path; `changed` needs a git repo and a safe revision. |
| `4` | Coverage input unreadable or unparsable. | Validate the LCOV / JaCoCo path and format. |
| `5` | Internal error. | Report with version and input. |

Base revisions starting with `-`, containing `:`, or containing control characters are rejected (exit `3`).

## Paths: Windows vs Unix

All report paths use `/`. Coverage paths are normalized the same way (`\` becomes `/`, `.`/`..` resolved lexically). `SF:C:\repo\src\a.ts` matches report path `src/a.ts` when the suffix aligns.

## Coverage stays `null`

`coverage` and `crap` are `null` (not zero) when no known line or branch record overlaps the function range. Two common causes:

- The coverage file uses paths that do not suffix-match the report path. Fix the `SF:` / package path or run from the same root.
- Ambiguous suffix matches: two coverage entries match one file (or vice versa). `leadline` attaches nothing rather than wrong data. Disambiguate by running from the repository root with full relative paths.
