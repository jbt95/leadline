#!/bin/sh
# Stop hook (warn mode): final changed-code check before completion.
# Non-blocking by design: always exits 0, prints ONLY on gate failure.
set -u
if ! command -v leadline >/dev/null 2>&1; then exit 0; fi
# Prefer project thresholds from leadline.toml; fall back to the documented
# defaults only when the project defines no threshold at all (exit 2).
out=$(leadline check . --format agent-json 2>/dev/null)
rc=$?
if [ "$rc" -eq 2 ]; then
  out=$(leadline check . --format agent-json --cognitive 15 --cyclomatic 10 --max-nesting 4 2>/dev/null)
  rc=$?
fi
# Exit 1 means violations or parse errors and carries the report on stdout.
[ "$rc" -eq 1 ] || exit 0
[ -n "$out" ] || exit 0
printf '%s\n' "$out" | head -c 4000
