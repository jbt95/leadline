#!/usr/bin/env bash
# Cut a release from the conventional commits since the last vX.Y.Z tag.
#
# The bump comes from the commits since the last tag: `feat` is minor,
# `fix`/`perf`/`revert` are patch, `!` or `BREAKING CHANGE` is major, and
# anything else releases nothing on its own. A release updates the version in
# every file listed in docs/releasing.md, closes the CHANGELOG's `Unreleased`
# section, verifies the bumped crate, and tags vX.Y.Z. The commit and tag are
# pushed atomically, and the script refuses to run when main has moved past
# the checkout.
set -euo pipefail

die() { echo "release: $1" >&2; exit 1; }
say() { echo "release: $1"; }

usage() {
  cat <<'EOF'
usage: bash scripts/release.sh [X.Y.Z] [--dry-run] [--no-verify] [--no-push]

  X.Y.Z        release this version instead of inferring one
  --dry-run    print the decision and change nothing
  --no-verify  skip cargo fmt/clippy/test
  --no-push    commit and tag locally, do not push
EOF
}

DRY_RUN=0
VERIFY=1
PUSH=1
VERSION=""
for arg in "$@"; do
  case "$arg" in
    --dry-run) DRY_RUN=1 ;;
    --no-verify) VERIFY=0 ;;
    --no-push) PUSH=0 ;;
    -h | --help) usage; exit 0 ;;
    -*) die "unknown flag: $arg" ;;
    *)
      [ -z "$VERSION" ] || die "unexpected argument: $arg"
      VERSION="$arg"
      ;;
  esac
done

command -v perl >/dev/null || die "perl is required"

ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || die "not inside a git repository"
cd "$ROOT"

# The files a release writes, rolled back if anything fails before the commit.
VERSIONED_FILES=(
  CHANGELOG.md
  Cargo.toml
  Cargo.lock
  package.json
  integrations/agent-adapter-ts/package.json
  integrations/agent-adapter-ts/package-lock.json
  integrations/typesafe-triage/package.json
  integrations/typesafe-triage/package-lock.json
  integrations/claude-code/.claude-plugin/plugin.json
  integrations/pi/package.json
  integrations/omp/package.json
)

semver() { [[ "$1" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; }

version_gt() { # true when $1 is strictly greater than $2
  if [ "$1" = "$2" ]; then
    return 1
  fi
  [ "$(printf '%s\n%s\n' "$1" "$2" | sort -t. -k1,1n -k2,2n -k3,3n | tail -n 1)" = "$1" ]
}

# The highest release tag reachable from HEAD, so a hotfix tag on a side
# branch merged later cannot pull the next version backwards.
CURRENT="$(git tag --merged HEAD --list 'v[0-9]*' | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sed 's/^v//' | sort -t. -k1,1n -k2,2n -k3,3n | tail -n 1 || true)"
[ -n "$CURRENT" ] || die "no vX.Y.Z tag in history; pass the version explicitly"
LAST_TAG="v$CURRENT"

LEVEL="$(git log -z --format=%B "$LAST_TAG..HEAD" | perl -0ne '
  $current = 0 unless defined $current;
  my ($subject, $body) = split /\n/, $_, 2;
  my $level = 0;
  if ($subject =~ /^[a-zA-Z]+(?:\([^)]*\))?!:/ || ($body && $body =~ /^BREAKING[ -]CHANGE:/m)) {
    $level = 3;
  } elsif ($subject =~ /^feat(?:\([^)]*\))?:/) {
    $level = 2;
  } elsif ($subject =~ /^(?:fix|perf|revert)(?:\([^)]*\))?:/) {
    $level = 1;
  }
  $current = $level if $level > $current;
  END { print +(qw(none patch minor major))[$current] }
')"

if [ -n "$VERSION" ]; then
  LEVEL=explicit
  semver "$VERSION" || die "version must be X.Y.Z: $VERSION"
  version_gt "$VERSION" "$CURRENT" || die "version $VERSION is not greater than $CURRENT"
else
  IFS=. read -r major minor patch <<<"$CURRENT"
  case "$LEVEL" in
    major) VERSION="$((major + 1)).0.0" ;;
    minor) VERSION="$major.$((minor + 1)).0" ;;
    patch) VERSION="$major.$minor.$((patch + 1))" ;;
    none)
      say "current=$CURRENT level=none next=none"
      say "nothing to do: no feat/fix/perf/revert or breaking commits since $LAST_TAG"
      exit 0
      ;;
    *) die "unexpected level: $LEVEL" ;;
  esac
fi

say "current=$CURRENT level=$LEVEL next=$VERSION"
if [ "$DRY_RUN" = 1 ]; then
  exit 0
fi

# Every check that can fail runs before the first file changes, so a refused
# release leaves the tree exactly as it was.
if [ "$PUSH" = 1 ]; then
  branch="$(git rev-parse --abbrev-ref HEAD)"
  [ "$branch" = main ] || die "refusing to push from branch '$branch': releases go out from main"
  git fetch -q origin main || die "cannot fetch origin main"
  if ! git merge-base --is-ancestor origin/main HEAD; then
    die "origin/main has commits this checkout does not; pull or rebase, then release again"
  fi
fi
if git rev-parse -q --verify "refs/tags/v$VERSION" >/dev/null; then
  die "tag v$VERSION already exists"
fi
git diff --quiet || die "working tree has unstaged changes"
git diff --cached --quiet || die "index has staged changes"

# The list above is the only thing rolled back, so a refused release leaves
# the tree exactly as it was.
ROLLBACK=0
rollback() {
  local status=$?
  if [ "$ROLLBACK" = 1 ]; then
    git checkout -- "${VERSIONED_FILES[@]}" 2>/dev/null || true
    echo "release: failed; the version bump was rolled back" >&2
  fi
  return "$status"
}
trap rollback EXIT
ROLLBACK=1

# One literal line per file, so a version that moved or changed shape stops
# the release instead of quietly bumping the wrong string.
replace_line() { # FILE OLD_LINE NEW_LINE
  local file="$1" old="$2" new="$3" found
  [ -f "$file" ] || die "missing $file"
  found="$(grep -Fxc -- "$old" "$file" || true)"
  [ "$found" = 1 ] || die "$file: expected one '$old' line, found $found"
  OLD="$old" NEW="$new" perl -pi -e 's/^\Q$ENV{OLD}\E$/$ENV{NEW}/' -- "$file"
}

replace_text() { # FILE OLD_TEXT NEW_TEXT EXPECTED
  local file="$1" old="$2" new="$3" expected="$4" found
  [ -f "$file" ] || die "missing $file"
  found="$(grep -Fo -- "$old" "$file" | wc -l | tr -d ' ' || true)"
  [ "$found" = "$expected" ] || die "$file: expected $expected occurrences of '$old', found $found"
  OLD="$old" NEW="$new" perl -pi -e 's/\Q$ENV{OLD}\E/$ENV{NEW}/g' -- "$file"
}

replace_line Cargo.toml "version = \"$CURRENT\"" "version = \"$VERSION\""

lock_line="$(grep -n '^name = "leadline"$' Cargo.lock | cut -d: -f1 || true)"
[ -n "$lock_line" ] || die "Cargo.lock: no leadline entry"
lock_next="$(sed -n "$((lock_line + 1))p" Cargo.lock)"
[ "$lock_next" = "version = \"$CURRENT\"" ] || die "Cargo.lock: leadline version is '$lock_next', expected $CURRENT"
LINE="$((lock_line + 1))" NEW="$VERSION" perl -pi -e 'if ($. == $ENV{LINE}) { s/^version = ".*"$/version = "$ENV{NEW}"/ }' Cargo.lock

replace_line package.json "  \"version\": \"$CURRENT\"," "  \"version\": \"$VERSION\","
replace_line integrations/agent-adapter-ts/package.json "  \"version\": \"$CURRENT\"," "  \"version\": \"$VERSION\","
replace_line integrations/agent-adapter-ts/package-lock.json "  \"version\": \"$CURRENT\"," "  \"version\": \"$VERSION\","
replace_line integrations/agent-adapter-ts/package-lock.json "      \"version\": \"$CURRENT\"," "      \"version\": \"$VERSION\","
replace_line integrations/typesafe-triage/package.json "  \"version\": \"$CURRENT\"," "  \"version\": \"$VERSION\","
replace_line integrations/typesafe-triage/package-lock.json "  \"version\": \"$CURRENT\"," "  \"version\": \"$VERSION\","
replace_line integrations/typesafe-triage/package-lock.json "      \"version\": \"$CURRENT\"," "      \"version\": \"$VERSION\","
replace_text integrations/claude-code/.claude-plugin/plugin.json "\"version\":\"$CURRENT\"" "\"version\":\"$VERSION\"" 1
replace_line integrations/pi/package.json "  \"version\": \"$CURRENT\"," "  \"version\": \"$VERSION\","
replace_line integrations/omp/package.json "  \"version\": \"$CURRENT\"," "  \"version\": \"$VERSION\","

changelog_heading="$(grep -n '^## Unreleased$' CHANGELOG.md | cut -d: -f1 || true)"
[ -n "$changelog_heading" ] || die "CHANGELOG.md: no '## Unreleased' heading"
[ "$(printf '%s\n' "$changelog_heading" | wc -l | tr -d ' ')" = 1 ] || die "CHANGELOG.md: more than one '## Unreleased' heading"
entries="$(awk '/^## Unreleased$/{inside=1; next} /^#/{inside=0} inside && /^- /{count++} END{print count+0}' CHANGELOG.md)"
[ "$entries" -gt 0 ] || say "warning: the Unreleased section has no entries"
CHANGELOG_HEADING="$changelog_heading" NEW="$(printf '## Unreleased\n\n## %s - %s' "$VERSION" "$(date +%Y-%m-%d)")" \
  perl -pi -e 'if ($. == $ENV{CHANGELOG_HEADING}) { s/^\Q## Unreleased\E$/$ENV{NEW}/ }' \
  CHANGELOG.md

if [ "$VERIFY" = 1 ]; then
  say "verifying the bumped tree: cargo fmt --check, clippy, test"
  cargo fmt --check
  cargo clippy --all-targets --locked -- -D warnings
  cargo test --locked
fi

git add -- "${VERSIONED_FILES[@]}"
git commit -q -m "chore(release): bump to $VERSION"
ROLLBACK=0
git tag "v$VERSION"

if [ "$PUSH" = 1 ]; then
  # Atomic: the commit and the tag land together or not at all.
  git push --atomic origin "HEAD:refs/heads/main" "refs/tags/v$VERSION"
fi

say "released v$VERSION ($LEVEL)"
