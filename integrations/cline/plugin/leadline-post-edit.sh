#!/bin/sh
# leadline Cline post-edit hook: changed-code analysis with quality-gate status.
#
# Contract: reads a hook event JSON object on stdin (optional {"base": "<rev>"}),
# writes a compact report on stdout, and always exits 0. Advisory only:
# it never blocks the agent.
#
# Quality gates run only when LEADLINE_CHECK_ARGS is set, e.g.
# LEADLINE_CHECK_ARGS="--cognitive 15 --cyclomatic 10". Otherwise the hook
# reports changed functions without judging them.
#
# ponytail: compact rendering is leadline's own terminal output; this script
# only frames it and must stay free of analysis logic.
set -u

INPUT=$(cat)
BASE=$(printf '%s' "$INPUT" | sed -n 's/.*"base"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)
if [ -z "$BASE" ]; then
  BASE="HEAD~1"
fi

if ! command -v leadline >/dev/null 2>&1; then
  echo "leadline: analyzer not on PATH; skipping changed analysis."
  exit 0
fi

# The base must resolve here: outside a repository, or before the base
# commit exists, the analyzer can only fail. Report the skip and continue.
if git rev-parse --verify --quiet "$BASE" >/dev/null 2>&1; then
  leadline changed --base "$BASE" || true
else
  echo "leadline: base '$BASE' is not available here; skipping changed analysis."
fi

if [ -n "${LEADLINE_CHECK_ARGS:-}" ]; then
  # shellcheck disable=SC2086
  if leadline check . $LEADLINE_CHECK_ARGS >/dev/null 2>&1; then
    echo "leadline gate: pass"
  else
    echo "leadline gate: findings present (advisory, not blocking)"
  fi
else
  echo "leadline gate: not configured (set LEADLINE_CHECK_ARGS to enable)"
fi
exit 0
