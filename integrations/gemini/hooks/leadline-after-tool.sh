#!/bin/sh
# Gemini hook: AfterTool / AfterAgent changed-code feedback.
# Protocol: reads hook JSON on stdin, writes hook JSON on stdout,
# logs ONLY to stderr. Non-blocking: always exits 0, never blocks.
# ponytail: single script serves both events; python3 used when present.
set -u
input=$(cat)
if ! command -v leadline >/dev/null 2>&1; then
  printf '{"decision":"continue"}\n'
  exit 0
fi
echo "$input" >&2
out=$(leadline changed --format agent-json 2>/dev/null) || out=""
if [ -z "$out" ]; then
  printf '{"decision":"continue"}\n'
  exit 0
fi
if command -v python3 >/dev/null 2>&1; then
  extra=$(printf '%s' "$out" | python3 -c 'import json,sys
try: d=json.load(sys.stdin)
except Exception: raise SystemExit(1)
regs=d.get("regressions",[]) or []
summ=d.get("summary",{}) or {}
if not (regs or summ.get("regressions",0)): raise SystemExit(1)
print(json.dumps({"regressions":regs[:5],"summary":summ})[:4000])') || {
    printf '{"decision":"continue"}\n'
    exit 0
  }
  printf '{"decision":"continue","additionalContext":%s}\n' "$(printf '%s' "$extra" | python3 -c 'import json,sys;print(json.dumps(sys.stdin.read()))')"
else
  printf '{"decision":"continue"}\n'
fi
