#!/bin/sh
# PostToolUse hook: report leadline regressions for changed code.
# Non-blocking by design: always exits 0, prints ONLY on material regression.
# ponytail: python3 used for JSON check when present, string fallback otherwise.
set -u
if ! command -v leadline >/dev/null 2>&1; then exit 0; fi
out=$(leadline changed --format agent-json 2>/dev/null) || exit 0
[ -n "$out" ] || exit 0
if command -v python3 >/dev/null 2>&1; then
  printf '%s' "$out" | python3 -c 'import json,sys
try: d=json.load(sys.stdin)
except Exception: raise SystemExit(0)
regs=d.get("regressions",[]) or []
summ=d.get("summary",{}) or {}
if regs or summ.get("regressions",0):
    print(json.dumps({"regressions":regs[:5],"summary":summ},indent=2)[:4000])' || exit 0
else
  case "$out" in *regression*) printf '%s' "$out" | head -c 4000;; *) exit 0;; esac
fi
