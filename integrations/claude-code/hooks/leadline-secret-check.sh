#!/bin/sh
# Shared secret-gate wrapper: worktree scan via the common runner, with the
# runner's status translated to the hook protocol shared by Claude Code and
# Gemini CLI, where only exit 2 blocks the action or turn. Findings and gate
# failures carry the runner's redacted diagnostics on stderr.
set -eu
here="$(CDPATH= cd -- "$(dirname "$0")" && pwd)"
runner="${LEADLINE_SECRET_RUNNER:-}"
if [ -z "$runner" ]; then
  # `|| common=""` keeps a missing shared directory from aborting under
  # `set -e` with exit 1, which hosts read as a warning, not a block.
  common="$(CDPATH= cd -- "$here/../../common" 2>/dev/null && pwd)" || common=""
  if [ -n "$common" ] && [ -f "$common/leadline-secret-check.sh" ]; then
    runner="$common/leadline-secret-check.sh"
  fi
fi
if [ -z "$runner" ]; then
  # A copied extension no longer sits beside integrations/common; the host
  # project's checkout still does.
  for project in "${GEMINI_PROJECT_DIR:-}" "${CLAUDE_PROJECT_DIR:-}"; do
    candidate="$project/integrations/common/leadline-secret-check.sh"
    if [ -n "$project" ] && [ -f "$candidate" ]; then
      runner="$candidate"
      break
    fi
  done
fi
if [ -z "$runner" ]; then
  echo "leadline-secret-check: shared runner not found; set LEADLINE_SECRET_RUNNER" >&2
  exit 2
fi
set +e
diagnostics="$(LEADLINE_SECRET_MODE=worktree "$runner" 2>&1)"
status=$?
set -e
if [ "$status" -ne 0 ]; then
  if [ -n "$diagnostics" ]; then
    printf '%s\n' "$diagnostics" >&2
  fi
  # Exit 1 (findings), 4 (scanner failure), and 127 (missing tool) would
  # otherwise read as warnings; only exit 2 blocks.
  exit 2
fi
