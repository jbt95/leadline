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

## Workflows

Shortest post-edit commands (all deterministic, JSON with `--format agent-json`):

- Explain a regression: `leadline changed --base <rev> --format agent-json --explain` (adds `causes` to regression rows only).
- Inspect a target before editing it: `leadline hotspots <path> --format agent-json` (files ranked by complexity x churn; the JSON carries the dimensions, not just a score).
- Check what usually changes with a file: `leadline coupling <file> --format agent-json` (historically related files to inspect; co-change is a hint, not a dependency).
- Check what depends on a file before editing it: `leadline impact <file> --format agent-json` (transitive dependents with distances and blast radius; static import evidence only).
- Rank change risk before editing: `leadline risk <path> --format agent-json` (files scored by `change-risk` components; informational only, never a developer ranking).
- Gate deltas: `leadline check . --base <rev> --regressions` (zero-tolerance unless `leadline.toml` sets `[regressions]` allowances).
- Target tests: `leadline test-targets . --coverage <file>` (line coverage only; unknown lines are reported, never called uncovered).
- Snapshot without Git history: `leadline baseline . --output <file>`, then `leadline check . --baseline <file> --regressions`.
- Full project report: `leadline analyze . --format agent-json --top 30 --sort-by cognitive` for the hotspot list, `leadline risk . --format agent-json` for file risk ranking, then `leadline impact <file> --format agent-json` on the riskiest files. `leadline check` is a gate, not a report: it needs at least one threshold flag (e.g. `--cognitive 15`) or `--regressions` with `--base`/`--baseline`, and errors without them.

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
