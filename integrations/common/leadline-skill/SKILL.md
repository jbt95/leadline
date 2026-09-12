---
name: leadline
description: Function-level complexity feedback via the leadline analyzer. Use after substantial edits to check changed code for maintainability risk.
---

# Leadline skill

`leadline` is a fast, deterministic function-level complexity analyzer
(Java, JavaScript, TypeScript, TSX). It reports signals about
maintainability risk. It never judges correctness.

## When to run

Run changed-code analysis after substantial edits (new features,
refactors, logic bug fixes). Scope follow-ups to flagged functions:

```bash
leadline changed --base <rev> --format agent-json
```

## Rules

1. Metrics are signals, not objectives.

   > Do not refactor solely to lower a numeric metric. Use analyzer output to identify maintainability risk, then apply normal engineering judgment and preserve behavior.

2. Prefer new complexity over legacy complexity. Fix regressions your
   change introduced. Leave unrelated legacy code alone unless the task
   already owns it.
3. Investigate CRAP regressions. A rising CRAP score on a changed
   function means complex code with weak coverage: add tests, simplify,
   then re-run `leadline` on the function.
4. Run the project's tests after any complexity-driven refactoring.
5. No metric gaming. Never split a function purely to move numbers.
   Extract a function only when it has a coherent responsibility.
6. Report unavailable coverage explicitly. When the analyzer reports no
   coverage for a function, say so (for example, "coverage unavailable,
   CRAP not computed") instead of treating the score as zero or risk-free.
