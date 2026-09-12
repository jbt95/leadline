---
name: leadline
description: Function-level complexity feedback via the leadline analyzer. Use after substantial edits to check changed code for maintainability risk.
---

# Leadline skill (Pi)

Policy source of truth: `../common/leadline-skill/SKILL.md`. This file
only adds Pi trigger notes; do not fork the policy here.

## Pi triggers

- After a substantial edit batch, run `leadline_changed`
  (`leadline changed --base <rev> --format agent-json` on the CLI).
- The extension also emits warn-mode post-edit feedback automatically.
  It never gates: treat it as advisory and keep working when it is silent.
- Metrics are signals, not objectives. Never refactor solely to lower a
  number, never game metrics via function extraction, and report
  unavailable coverage explicitly.
