# Security model

- Parse-only: `leadline` reads source and coverage files and walks syntax trees. It never executes project code, test scripts, or build plugins.
- No network: analysis, coverage merge, and diffing are fully offline. No telemetry, no fetching.
- `git` is the only subprocess, invoked as `git` with fixed argument lists (`rev-parse`, `diff --no-renames --name-only -z`, `ls-files`, `show <rev>:<path>`). Revisions starting with `-`, containing `:`, or containing control characters are rejected before execution.
- MCP server is read-only: the five tools return analysis data only. No file writes, no shell access, no hook installation.
- Coverage inputs (LCOV, JaCoCo XML) are data files parsed with a hardened XML reader; entity expansion and external references are not resolved.
- Reports contain function names, metrics, and line spans only. Source text is never embedded.
