# leadline snippet for `.github/copilot-instructions.md`

Use the leadline analyzer for complexity feedback. It is read-only and never
edits source code.

- After a substantial change, run `leadline changed --base origin/main --json`
  and prioritize newly introduced complexity regressions.
- Do not refactor unrelated legacy code solely to improve metrics.
- Never game metrics or sacrifice correctness to lower a score.
- After complexity-driven refactoring, run the project's tests.
