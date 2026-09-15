#!/usr/bin/env bash
# End-to-end smoke test on large third-party repos (full clones).
#
# Usage:
#   bash scripts/smoke.sh                    # everything (network + GBs of clones)
#   SMOKE_ONLY=self bash scripts/smoke.sh    # offline self-check, no clones
#   SMOKE_ONLY=sql bash scripts/smoke.sh     # one section (after fetching)
#
# Environment:
#   LEADLINE_SMOKE_DIR   fixture directory (default: <repo>/target/smoke)
#   LEADLINE_BIN         leadline binary (default: build it with cargo)
#   SMOKE_ONLY           one section or "all" (default: all)
#   SMOKE_SECURITY_SARIF existing SARIF file for the security gate (else skipped)
#
# Sections: fetch scale gates history joins sql plan coverage scanners mcp self
#
# Fixtures float on latest main; nothing here runs in CI and `cargo test`
# never touches the network. Full (not shallow) clones: the history
# surfaces degrade to git_available:false without git history.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIR="${LEADLINE_SMOKE_DIR:-$ROOT/target/smoke}"
BIN="${LEADLINE_BIN:-$ROOT/target/debug/leadline}"
ONLY="${SMOKE_ONLY:-all}"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/leadline-smoke.XXXXXX")"
SRV=""

cleanup() { rm -rf "$TMP"; if [ -n "$SRV" ]; then kill "$SRV" 2>/dev/null || true; wait "$SRV" 2>/dev/null || true; fi; }
trap cleanup EXIT

die() { echo "smoke: FAIL: $1" >&2; exit 1; }
say() { echo "smoke: $1"; }
wanted() { [ "$ONLY" = "all" ] || [ "$ONLY" = "$1" ]; }

command -v git >/dev/null || die "git not found"
command -v python3 >/dev/null || die "python3 not found"
command -v curl >/dev/null || die "curl not found"

if [ ! -x "$BIN" ]; then
  say "building leadline..."
  (cd "$ROOT" && cargo build --offline) || die "cargo build failed"
fi

# assert_json FILE [KEY] — parses as JSON and carries KEY (default schema_version).
# Scanner agent-json shapes (sql-plan, security, vulnerabilities, sql) carry no
# schema_version; assert their payload key instead.
assert_json() {
  local key="${2:-schema_version}"
  python3 -c 'import json,sys; d=json.load(open(sys.argv[1])); assert sys.argv[2] in d, "missing "+sys.argv[2]' "$1" "$key" \
    || die "bad JSON: $1"
}

# allow_01 CMD... — quality gates exit 0 (clean) or 1 (violations).
allow_01() { "$@" || { local code=$?; [ "$code" -eq 1 ] || die "exit $code: $*"; }; }

# fetch NAME URL BRANCH — full clone (history needs it), update in place.
fetch() {
  local name="$1" url="$2" branch="$3" path="$DIR/$1"
  if [ -d "$path/.git" ]; then
    say "updating $name..."
    git -C "$path" fetch origin "$branch" && git -C "$path" reset --hard FETCH_HEAD
  else
    say "cloning $name ($branch)..."
    git clone --branch "$branch" "$url" "$path"
  fi
}

need_repo() { [ -d "$1/.git" ] || die "missing $1 (run the fetch section first)"; }

if wanted fetch; then
  mkdir -p "$DIR"
  fetch "babel" "https://github.com/babel/babel.git" "main"
  fetch "elasticsearch" "https://github.com/elastic/elasticsearch.git" "main"
  fetch "timescaledb" "https://github.com/timescale/timescaledb.git" "main"
fi

BABEL="$DIR/babel"
ES="$DIR/elasticsearch"
TSDB="$DIR/timescaledb"

if wanted scale; then
  need_repo "$BABEL"; need_repo "$ES"
  for repo in "$BABEL" "$ES"; do
    say "analyze $repo (x2 determinism)..."
    "$BIN" analyze "$repo" --format agent-json --top 30 --sort-by cognitive > "$TMP/a1.json"
    assert_json "$TMP/a1.json"
    "$BIN" analyze "$repo" --format agent-json --top 30 --sort-by cognitive > "$TMP/a2.json"
    cmp "$TMP/a1.json" "$TMP/a2.json" || die "nondeterministic analyze: $repo"
  done
fi

if wanted gates; then
  need_repo "$BABEL"
  say "check gates (0 clean, 1 violations)..."
  allow_01 "$BIN" check "$BABEL" --cognitive 15 --cyclomatic 10 --max-nesting 4
  allow_01 "$BIN" check "$BABEL/packages/babel-parser" --cognitive 15 --cyclomatic 10 --max-nesting 4 --format agent-json
fi

if wanted history; then
  need_repo "$BABEL"
  say "hotspots..."
  "$BIN" hotspots "$BABEL" --since 90d --format agent-json > "$TMP/hot.json"
  assert_json "$TMP/hot.json"
  say "coupling/impact on a file with dependents..."
  TARGET=""
  # Intentional word-splitting below: newline-separated paths, no spaces in repos.
  CANDS="$(cd "$BABEL" && grep -rl --include='*.ts' --exclude-dir=node_modules -m1 -e '^import ' -e 'from "' packages/babel-parser/src | head -n 5)"
  for cand in $CANDS; do
    if "$BIN" impact "$BABEL/$cand" --path "$BABEL" --format agent-json > "$TMP/im.json" 2>/dev/null; then
      TARGET="$cand"; break
    fi
  done
  [ -n "$TARGET" ] || die "no impactable file found"
  assert_json "$TMP/im.json"
  "$BIN" coupling "$BABEL/$TARGET" --path "$BABEL" --format agent-json > "$TMP/co.json"
  assert_json "$TMP/co.json"
  say "debt/baseline..."
  allow_01 "$BIN" debt --base HEAD~20 --path "$BABEL" --format agent-json
  "$BIN" baseline "$BABEL/packages/babel-parser" --output "$TMP/base.json"
  allow_01 "$BIN" check "$BABEL/packages/babel-parser" --baseline "$TMP/base.json" --regressions
  say "duplication (0 clean, 1 findings, 3 ceiling)..."
  code=0; "$BIN" duplication "$BABEL/packages/babel-parser" --base HEAD~50 --json > "$TMP/du.json" || code=$?
  code=${code:-0}; case "$code" in 0|1|3) ;; *) die "duplication exit $code" ;; esac
  assert_json "$TMP/du.json"
fi

if wanted joins; then
  need_repo "$BABEL"
  say "project/snapshot/policy..."
  "$BIN" project "$BABEL/packages/babel-generator" --format agent-json > "$TMP/pr.json"
  assert_json "$TMP/pr.json"
  "$BIN" snapshot "$BABEL/packages/babel-generator" --output "$TMP/snap.json"
  allow_01 "$BIN" policy "$BABEL/packages/babel-generator" --base HEAD~10 --json
fi

if wanted sql; then
  need_repo "$TSDB"
  say "sql on timescaledb migrations..."
  MIG_ABS="$(find "$TSDB/sql" -name '*.sql' | head -n 1)"
  [ -n "$MIG_ABS" ] || die "no .sql files under timescaledb/sql"
  MIG_REL="${MIG_ABS#$TSDB/}"
  (cd "$TSDB" && allow_01 "$BIN" sql . --migration-root "$(dirname "$MIG_REL")" --format agent-json > "$TMP/sql.json")
  assert_json "$TMP/sql.json" findings
  say "negative path: empty dir exits 3..."
  mkdir -p "$TMP/empty"
  code=0; "$BIN" analyze "$TMP/empty" >/dev/null 2>&1 || code=$?
  code=${code:-0}; [ "$code" -eq 3 ] || die "empty-dir exit $code, wanted 3"
fi

if wanted plan; then
  say "sql-plan violation (exit 1) and clean (exit 0) controls..."
  mkdir -p "$TMP/plans/current" "$TMP/plans/baseline"
  printf '%s' '[{"Plan": {"Node Type": "Seq Scan", "Total Cost": 150.0, "Plan Rows": 10}}]' > "$TMP/plans/current/q.json"
  printf '%s' '[{"Plan": {"Node Type": "Seq Scan", "Total Cost": 100.0, "Plan Rows": 10}}]' > "$TMP/plans/baseline/q.json"
  code=0; "$BIN" sql-plan --current "$TMP/plans/current" --baseline "$TMP/plans/baseline" --max-cost-increase-percent 25 --format agent-json > "$TMP/plan.json" || code=$?
  code=${code:-0}; [ "$code" -eq 1 ] || die "plan exit $code, wanted 1"
  assert_json "$TMP/plan.json" violations
  "$BIN" sql-plan --current "$TMP/plans/current" --baseline "$TMP/plans/baseline" --max-cost-increase-percent 200 --format agent-json > "$TMP/plan2.json"
  assert_json "$TMP/plan2.json" violations
fi

if wanted coverage; then
  need_repo "$BABEL"
  if [ -f "$BABEL/coverage/lcov.info" ]; then
    say "test-targets with coverage..."
    "$BIN" test-targets "$BABEL/packages/babel-parser" --lcov "$BABEL/coverage/lcov.info" --format agent-json > "$TMP/tt.json"
    assert_json "$TMP/tt.json"
  else
    say "skip: no coverage (run: cd $BABEL && TEST_COVERAGE=true yarn jest packages/babel-parser)"
  fi
fi

if wanted scanners; then
  need_repo "$BABEL"
  if command -v osv-scanner >/dev/null; then
    say "osv-scanner..."
    allow_01 osv-scanner --format json --output "$TMP/osv.json" "$BABEL"
    allow_01 "$BIN" vulnerabilities "$BABEL" --osv "$TMP/osv.json" --fail-on-severity high
  else
    say "skip: osv-scanner not installed"
  fi
  if [ -n "${SMOKE_SECURITY_SARIF:-}" ]; then
    allow_01 "$BIN" security "$BABEL" --sarif "$SMOKE_SECURITY_SARIF" --fail-on-severity high
  else
    say "skip: set SMOKE_SECURITY_SARIF=<file.sarif> for the security gate (eslint @microsoft/eslint-formatter-sarif or semgrep --sarif)"
  fi
fi

if wanted mcp; then
  say "mcp stdio..."
  printf '%s' '{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}' | "$BIN" mcp | grep -q '"result":{}' || die "mcp stdio ping failed"
  say "mcp http..."
  "$BIN" mcp --port 0 >"$TMP/mcp.out" 2>"$TMP/mcp.log" &
  SRV=$!
  for ((i=0; i<100; i++)); do grep -q "listening on" "$TMP/mcp.log" && break; sleep 0.1; done
  grep -q "listening on" "$TMP/mcp.log" || die "mcp http never listened"
  PORT="$(sed -E 's|.*http://[^:]+:([0-9]+)/mcp.*|\1|' "$TMP/mcp.log")"
  curl -sf "http://127.0.0.1:$PORT/health" | grep -q '"status":"ok"' || die "health check failed"
  printf '%s' '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' | curl -sf -X POST "http://127.0.0.1:$PORT/mcp" -H 'Content-Type: application/json' --data @- | grep -q analyze_changed || die "tools/list failed"
  kill "$SRV"; wait "$SRV" 2>/dev/null || true; SRV=""
fi

if wanted self; then
  say "offline self-check (no clones)..."
  SELF="$ROOT/integrations/agent-adapter-ts"
  "$BIN" analyze "$SELF" --format agent-json --top 10 > "$TMP/self1.json"
  assert_json "$TMP/self1.json"
  "$BIN" analyze "$SELF" --format agent-json --top 10 > "$TMP/self2.json"
  cmp "$TMP/self1.json" "$TMP/self2.json" || die "nondeterministic analyze"
  allow_01 "$BIN" check "$SELF" --cognitive 15 --cyclomatic 10 --max-nesting 4
fi

say "OK"
