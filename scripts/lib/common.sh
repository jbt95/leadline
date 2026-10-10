# Shared bootstrap for the fixture-fetching scripts.
#
# `scripts/smoke.sh` needs full clones because its history sections read git
# history; `scripts/realworld.sh` wants shallow floating clones. Both scripts
# otherwise repeat the same headers, the same clone-or-update logic, and the
# same output prefix, so that lives here once.

set -euo pipefail

# Resolved from this file, not the caller, so every sourcing script agrees on
# the repository root.
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

say() { echo "${SAY_PREFIX:-leadline}: $1"; }
die() { echo "${SAY_PREFIX:-leadline}: FAIL: $1" >&2; exit 1; }

have() { command -v "$1" >/dev/null || die "$1 not found"; }

# fetch NAME URL BRANCH DIRECTORY [DEPTH] — clone or update one fixture.
# DEPTH is optional; omitting it takes the full history.
fetch() {
  local name="$1" url="$2" branch="$3" dir="$4" depth="${5:-}"
  local path="$dir/$name"
  if [ -d "$path/.git" ]; then
    say "updating $name..."
    if [ -n "$depth" ]; then
      git -C "$path" fetch --depth "$depth" origin "$branch" \
        && git -C "$path" reset --hard FETCH_HEAD
    else
      git -C "$path" fetch origin "$branch" && git -C "$path" reset --hard FETCH_HEAD
    fi
  else
    say "cloning $name ($branch)..."
    if [ -n "$depth" ]; then
      git clone --depth "$depth" --branch "$branch" "$url" "$path"
    else
      git clone --branch "$branch" "$url" "$path"
    fi
  fi
}
