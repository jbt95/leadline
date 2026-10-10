#!/usr/bin/env bash
# Fetch/update real-world benchmark fixtures (floating latest, shallow).
# Usage: bash scripts/realworld.sh
# Honors LEADLINE_REALWORLD_DIR (default: <repo>/target/realworld).
SAY_PREFIX=realworld
# shellcheck source=lib/common.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib/common.sh"

DIR="${LEADLINE_REALWORLD_DIR:-$ROOT/target/realworld}"
have git
mkdir -p "$DIR"

# Depth 1: these fixtures only need the floating tip, not history.
fetch "tanstack-query" "https://github.com/TanStack/query" "main" "$DIR" 1
fetch "nest" "https://github.com/nestjs/nest" "master" "$DIR" 1
fetch "react" "https://github.com/facebook/react" "main" "$DIR" 1
fetch "spring-boot" "https://github.com/spring-projects/spring-boot" "main" "$DIR" 1

say "fixtures ready in $DIR"
