---
name: leadline
description: Function-level complexity feedback on changed code. Use after substantial edits to check for newly introduced regressions.
---

# Leadline

`leadline` measures function-level complexity (cyclomatic, cognitive,
CRAP, nesting). Treat numbers as signals, not objectives.

## When to run

Run changed-code analysis after a substantial edit batch:

```bash
leadline changed --format agent-json
```

For a single function:

```bash
leadline function src/payment.ts processPayment --json
```

For quality gates over changed code:

```bash
leadline check . --format agent-json
```

## Rules

1. Run analysis after substantial edits, not after every keystroke.
2. Treat metrics as signals, not objectives.
3. Never sacrifice correctness to reduce a score.
4. Prefer fixing newly introduced complexity over unrelated legacy code.
5. Investigate severe CRAP regressions (uncovered complex code).
6. Run tests after complexity-driven refactoring.
7. Do not game metrics with meaningless function extraction.
8. If coverage is unavailable, say so instead of guessing CRAP.

> Do not refactor solely to lower a numeric metric. Use analyzer output
> to identify maintainability risk, then apply normal engineering
> judgment and preserve behavior.
