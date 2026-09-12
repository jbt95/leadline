---
name: leadline
description: Function-level complexity feedback on changed code. Use after substantial edits to check for newly introduced regressions.
---

# Leadline

`leadline` measures function-level complexity (cyclomatic, cognitive,
CRAP, nesting). Treat numbers as signals, not objectives.

## When to run

After implementing a substantial change, run:

```bash
leadline changed --format agent-json
```

For one function:

```bash
leadline function <file> <name> --json
```

## Policy

- Prioritize newly introduced complexity regressions.
- Do not refactor unrelated legacy code solely to improve metrics.
- Never sacrifice correctness to reduce a score.
- Investigate severe CRAP regressions; run tests after refactoring.
- Do not game metrics with meaningless function extraction.
- If coverage is unavailable, say so instead of guessing CRAP.

> Do not refactor solely to lower a numeric metric. Use analyzer output
> to identify maintainability risk, then apply normal engineering
> judgment and preserve behavior.
