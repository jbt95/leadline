---
name: leadline
description: Function-level complexity feedback on changed code. Use after substantial edits.
---

# Leadline

`leadline` measures function-level complexity (cyclomatic, cognitive,
CRAP, nesting). Treat numbers as signals, not objectives.

Prefer the native `leadline_changed` tool when available, otherwise run:

```bash
leadline changed --format agent-json
```

## Rules

1. Run analysis after substantial edits, not after every keystroke.
2. Never sacrifice correctness to reduce a score.
3. Prefer fixing newly introduced complexity over unrelated legacy code.
4. Investigate severe CRAP regressions; run tests after refactoring.
5. Do not game metrics with meaningless function extraction.
6. If coverage is unavailable, say so instead of guessing CRAP.

> Do not refactor solely to lower a numeric metric. Use analyzer output
> to identify maintainability risk, then apply normal engineering
> judgment and preserve behavior.
