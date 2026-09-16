#!/bin/sh
# Shared secret gate: redacted gitleaks scan handed to `leadline security`.
#
# Mode comes from LEADLINE_SECRET_MODE (`staged` for pre-commit hooks,
# `worktree` for agent adapters); the leadline binary from LEADLINE_BIN.
# Scanner output is suppressed and the SARIF report never prints: only fixed
# diagnostics reach stdout/stderr. Exits: 0 clean, 1 findings, 2 bad mode,
# 3 no git comparison target, 4 scanner/report failure, 127 missing
# executable.
set -eu
umask 077

mode="${LEADLINE_SECRET_MODE:-staged}"
if [ "$mode" != staged ] && [ "$mode" != worktree ]; then
  echo "leadline-secret-check: bad LEADLINE_SECRET_MODE '$mode': expected 'staged' or 'worktree'" >&2
  exit 2
fi
# Worktree findings only gate when they sit in changed paths, which needs a
# git HEAD to diff against. Preflight it so a directory with no comparison
# target skips the gate before a full-tree gitleaks scan can run.
if [ "$mode" = worktree ]; then
  if ! command -v git >/dev/null 2>&1; then
    echo "leadline-secret-check: git not found; worktree secret gate skipped" >&2
    exit 127
  fi
  if ! git rev-parse --show-toplevel >/dev/null 2>&1; then
    echo "leadline-secret-check: not a git repository; worktree secret gate skipped" >&2
    exit 3
  fi
  if ! git rev-parse --verify --quiet HEAD >/dev/null 2>&1; then
    echo "leadline-secret-check: repository has no commits; worktree secret gate skipped" >&2
    exit 3
  fi
fi
if ! command -v gitleaks >/dev/null 2>&1; then
  echo "leadline-secret-check: gitleaks not found; install gitleaks to enable secret gating" >&2
  exit 127
fi
leadline_bin="${LEADLINE_BIN:-leadline}"
case "$leadline_bin" in
  */*)
    if [ ! -x "$leadline_bin" ]; then
      echo "leadline-secret-check: leadline executable not found: $leadline_bin" >&2
      exit 127
    fi
    ;;
  *)
    if ! leadline_bin="$(command -v "$leadline_bin")"; then
      echo "leadline-secret-check: leadline executable not found: $leadline_bin" >&2
      exit 127
    fi
    ;;
esac
tmp="${TMPDIR:-/tmp}/leadline-secrets.XXXXXX"
tmp="$(mktemp -d "$tmp")"
trap 'rm -rf "$tmp"' EXIT HUP INT TERM
report="$tmp/report.sarif"
if [ "$mode" = staged ]; then
  set +e
  gitleaks git --staged --no-banner --redact --report-format sarif --report-path "$report" >/dev/null 2>&1
  scanner=$?
  set -e
else
  set +e
  gitleaks dir . --no-banner --redact --report-format sarif --report-path "$report" >/dev/null 2>&1
  scanner=$?
  set -e
fi
if [ "$scanner" -gt 1 ]; then
  echo "leadline-secret-check: gitleaks scan failed" >&2
  exit 4
fi
if [ "$mode" = staged ]; then
  "$leadline_bin" security . --sarif "$report" --staged --fail-on-severity low --changed-only
else
  "$leadline_bin" security . --sarif "$report" --base HEAD --fail-on-severity low --changed-only
fi
