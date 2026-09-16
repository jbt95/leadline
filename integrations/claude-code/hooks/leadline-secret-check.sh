#!/bin/sh
# Secret-gate wrapper for host hooks where exit 2 blocks the action or turn
# (Claude Code, Gemini CLI, Cline). Only findings block: the shared runner's
# exit 1 becomes exit 2. An unavailable runner or scanner, a scan failure, or
# a bad mode exits 1 with the runner's redacted diagnostics on stderr, so an
# environment miss warns visibly without trapping the turn.
set -eu
here="$(CDPATH='' cd -- "$(dirname "$0")" && pwd)"
runner="${LEADLINE_SECRET_RUNNER:-}"
if [ -z "$runner" ]; then
  # A vendored runner beside the hook (`common/`) keeps packaged extensions
  # self-contained; the repository layout carries it one directory higher.
  for candidate in \
    "$here/../common/leadline-secret-check.sh" \
    "$here/../../common/leadline-secret-check.sh"
  do
    if [ -f "$candidate" ]; then
      runner="$candidate"
      break
    fi
  done
fi
if [ -z "$runner" ]; then
  # A copied extension without a vendored runner still resolves from the host
  # project's checkout.
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
  exit 1
fi
set +e
diagnostics="$(LEADLINE_SECRET_MODE=worktree "$runner" 2>&1)"
status=$?
set -e
if [ "$status" -eq 0 ]; then
  exit 0
fi
if [ -n "$diagnostics" ]; then
  printf '%s\n' "$diagnostics" >&2
fi
if [ "$status" -eq 1 ]; then
  # Findings are the only blocking status in the host hook protocol.
  exit 2
fi
# Scanner/runner unavailable (127), scan failure (4), bad mode (2): visible
# but non-blocking.
exit 1
