# leadline policy (append to AGENTS.md or include as project instructions)

After implementing a substantial change, run changed-code analysis:

```bash
leadline changed --format agent-json
```

- Prioritize newly introduced complexity regressions over pre-existing debt.
- Never refactor legacy code solely for metrics.
- Never sacrifice correctness to reduce a score.
- Investigate severe CRAP regressions; run tests after refactoring.
- Keep tool output small: prefer `agent-json`, never paste full
  repository reports into context.
