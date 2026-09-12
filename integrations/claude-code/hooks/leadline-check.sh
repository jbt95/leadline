#!/bin/sh
# Stop hook (warn mode): final changed-code check before completion.
# Non-blocking by design: always exits 0, prints ONLY on gate failure.
set -u
if ! command -v leadline >/dev/null 2>&1; then exit 0; fi
out=$(leadline check . --format agent-json 2>/dev/null) || exit 0
[ -n "$out" ] || exit 0
printf '%s\n' "$out" | head -c 4000
