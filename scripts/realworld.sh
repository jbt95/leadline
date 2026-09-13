#!/usr/bin/env bash
# Fetch/update real-world benchmark fixtures (floating latest, shallow).
# Usage: bash scripts/realworld.sh
# Honors LEADLINE_REALWORLD_DIR (default: <repo>/target/realworld).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="${LEADLINE_REALWORLD_DIR:-$ROOT/target/realworld}"

command -v git >/dev/null || { echo "realworld: git not found" >&2; exit 1; }
mkdir -p "$DIR"

fetch() { # name url branch
  local name="$1" url="$2" branch="$3" path="$DIR/$1"
  if [ -d "$path/.git" ]; then
    echo "realworld: updating $name..."
    git -C "$path" fetch --depth 1 origin "$branch" && git -C "$path" reset --hard FETCH_HEAD
  else
    echo "realworld: cloning $name ($branch)..."
    git clone --depth 1 --branch "$branch" "$url" "$path"
  fi
}

fetch "tanstack-query" "https://github.com/TanStack/query" "main"
fetch "nest" "https://github.com/nestjs/nest" "master"
fetch "react" "https://github.com/facebook/react" "main"
fetch "spring-boot" "https://github.com/spring-projects/spring-boot" "main"

echo "realworld: fixtures ready in $DIR"
