# Test selection

`leadline select-tests` turns a git diff into the test files a change can
affect. It reads the diff, the static dependency graph, and per-language test
conventions; it never runs a test. What runs the selection is your business.

```console
leadline select-tests [PATH] [--base REV] [--staged] [--format json|agent-json|list] [--explain] [--max-ratio N]
```

| Flag | Meaning |
| --- | --- |
| `PATH`, `--path PATH` | Analysis root (default `.`). The diff, the graph, and discovery are all scoped here. |
| `--base REV` | Base revision, default `HEAD~1`. The comparison target is the working tree, untracked files included. |
| `--staged` | Compare `HEAD` against the index instead. Exclusive with `--base` (exit `2`). |
| `--format json` | The selection as contract JSON (below). |
| `--format agent-json` | The same value in the compact single-line envelope. |
| `--format list` | Bare paths, one per line, nothing else: the form to pipe into a runner. |
| `--explain` | Add the tier and the `via` changed files to each terminal row. |
| `--max-ratio N` | Warn on stderr when the selection exceeds the fraction `N` of the test files. A diagnostic: it never changes the exit code. |

`[tia]` in `leadline.toml` supplies extra test patterns, `always_run`
patterns, and exclusions; see [configuration](configuration.md#tia) and the
section below.

## The escalation ladder

The point of the command is that under-selection is never silent. Every changed
file is placed on a tier, and the strongest tier any file needed is reported on
the selection.

| Tier | Condition | Action |
| --- | --- | --- |
| T2 `graph` | the file is a node in the dependency graph | the test files in its reverse closure, and itself when it is one |
| T3 `directory` | unsupported extension, a parse error, absent from the graph, or no selected test reached it | every test file under its top-level directory |
| T4 `all` | the file matches an `always_run` pattern | every test file |

T1 — recorded coverage — arrives with the recorders and does not exist yet.

**T2, the graph.** Reverse reachability over the import graph, walked once for
the whole change set and never truncated. A changed file that is itself a test
file is selected as well.

```console
$ leadline select-tests --base HEAD~1 --explain
Test selection (1 changed, 1 of 55 test files, ratio 0.018) against HEAD~1
Escalation: none
  crates/tia/src/conventions.rs [graph] via crates/tia/src/conventions.rs
```

That file's own `#[cfg(test)] mod tests` is what selected it: the ladder found
it in the reverse closure and the content check saw the inline tests.

The T2 transcript above is this repository. The three below are a TypeScript
repository of 45 test files outside it (`react-lucene-query`), so the numbers
are a different suite.

**T3, the directory.** A file the ladder cannot reason about escalates to the
test files under its top-level directory rather than being skipped. A root-level
file has the whole repository as its top-level directory, so a root manifest
change selects everything:

```console
$ leadline select-tests --base HEAD~1 --explain
Test selection (1 changed, 45 of 45 test files, ratio 1.000) against HEAD~1
Escalation: directory (unsupported extension: package.json)
  browser-tests/playground.spec.ts [directory] via package.json
  browser-tests/worker.spec.ts [directory] via package.json
  tests/core-helpers.test.ts [directory] via package.json
```

The run prints all 45 rows; the list is elided here.

A file one directory down escalates to that directory only — here an untracked
`tests/notes.txt`, which also trips `--max-ratio`:

```console
$ leadline select-tests --base HEAD --explain --max-ratio 0.25
leadline: warning: selected 19 of 45 test files (ratio 0.422) exceeds --max-ratio 0.25
Test selection (1 changed, 19 of 45 test files, ratio 0.422) against HEAD
Escalation: directory (unsupported extension: tests/notes.txt)
  tests/core-helpers.test.ts [directory] via tests/notes.txt
  tests/core.test.ts [directory] via tests/notes.txt
```

All 19 rows are printed.

The fourth reason, `no test found`, is what makes "a changed file always
produces something" true by construction rather than dependent on every
convention row being complete: after the conventions, the content check, and
the closure have run, a changed file that contributed no selected test escalates
the same way.

**T4, everything.** A changed file matching `always_run` selects every test
file. A *root* manifest is already caught: its top-level directory is the
repository, so T3 reaches everything by accident. A nested one is not. A
migration under `db/migrations/` escalates only to the tests under `db/`, which
is usually none, and a lockfile in a workspace package is the same story.
`always_run` is what turns "this change is not about one subtree" into every
test file, and it reports tier `all` so the reason stays visible. With
`always_run = ["**/*.md"]` in `leadline.toml`, a documentation edit runs the
whole suite:

```console
$ leadline select-tests --base HEAD~1 --explain
Test selection (1 changed, 45 of 45 test files, ratio 1.000) against HEAD~1
Escalation: all (always run: docs/effect.md)
  browser-tests/playground.spec.ts [all] via docs/effect.md
  browser-tests/worker.spec.ts [all] via docs/effect.md
```

All 45 rows are printed.

An empty `always_run` leaves a nested documentation edit at T3, which selects
whatever tests sit in `docs/` — usually none. That is the gap the list fills.

## Test-file conventions

A candidate must be a file `parse::discovery` accepts — a supported extension,
not in an ignored directory such as `node_modules`, `target`, or `vendor` — and
not matched by `[analysis] exclude` or `[tia] exclude`.

| Language | Name form | Directory layout |
| --- | --- | --- |
| Rust | `*_test.rs` | a top-level file under `tests/` (`tests/foo.rs`) or a directory target (`tests/<name>/main.rs`) |
| Rust | any file declaring `#[test]` (content row) | — |
| TypeScript, TSX, JavaScript | a name containing `.test.` or `.spec.` | any `__tests__/` component |
| Go | `*_test.go` | — |
| Python | `test_*.py`, `*_test.py` | — |
| Java | `*Test.java`, `*Tests.java`, `*IT.java` | `src/test/` at any depth |
| C, C++ | `test_*.c*`, `*_test.c*` (an extension starting with `c`, so headers are not tests) | — |
| Zig | `test_*.zig` | — |
| Zig | any file declaring `test "..."` (content row) | — |

Directory layouts are per language, never a blanket "any `tests/` component"
rule: only Rust compiles `tests/`, only the JavaScript ecosystem reads
`__tests__/`, and only Java has `src/test/`. A `tests/` directory of fixtures in
a C repository is data a test reads, not a suite: claiming it was one of the
two defects the demo exposed.

Two shared exclusions sit above the rows, split by how strong the row's signal
is:

- A **data** tree — `fixtures`, `testdata`, `snapshots`, `__snapshots__`,
  `__mocks__` — outranks every row, including a name that says "test". Nothing
  there is executed by a runner, whatever it is called.
- A **helper** tree — `common`, `helpers`, `support` — outranks only the
  layout rows, where the directory is the entire signal. A test's own name
  outranks where it sits: `src/common/app.test.ts` is a test, while an unnamed
  Java class in `src/test/java/.../helpers/` is not.

### What the conventions cost

The path rows need no I/O: a path is a test file or it is not, decided by
string shape alone. The two content rows (`#[test]`, `test "..."`) need the
file's bytes, so they are applied only to files the reverse closure of a
changed file already reaches. The reading therefore scales with the blast
radius of the diff, not with the size of the repository.

That matters because inline `#[cfg(test)] mod tests` inside `src/**` is the
dominant Rust test location in many projects. A path-only Rust table would
select nothing for a changed file whose only tests are inline.

## Configuration

```toml
[tia]
test_globs = ["**/*.check.ts"]   # added to the conventions, never replacing them
always_run = ["**/migrations/**", "tsconfig*.json", "*.lock"]
exclude = ["vendor/**"]
```

`test_globs` is **additive**: a candidate still has to clear discovery and the
exclusions, and the built-in rows keep working. `always_run` is the T4 trigger
described above. Patterns are analysis-root-relative with `/` separators;
`exclude` removes candidates the same way it removes analysis sources.

## Output contract

`--format json` prints one object; `--format agent-json` prints the same one on
a single line.

```json
{
  "schema_version": 1,
  "model": "test-selection",
  "base": "HEAD~1",
  "changed": ["tests/notes.txt"],
  "selected": [
    { "path": "tests/core.test.ts", "tier": "directory", "via": ["tests/notes.txt"] }
  ],
  "escalation": { "tier": "directory", "reasons": ["unsupported extension: tests/notes.txt"] },
  "totals": { "test_files": 45, "selected": 19, "ratio": 0.4222222222222222 },
  "confidence": { "graph_files": 122, "graph_edges": 220, "unresolved": 6, "parse_errors": 0 }
}
```

- `selected` is sorted by path. `tier` is the strongest tier that reached that
  test, and `via` is every changed file that reached it, so `--explain`
  re-derives nothing.
- `escalation` is **absent** when no file escalated. `tier` is the strongest
  tier the run needed and `reasons` names one per affected file.
- `totals.test_files` is the denominator: the path-convention test files plus
  any selected file the paths did not name, so `ratio` is always ≤ 1. It is a
  lower bound, not a total — see the limitations.
- `confidence` is the graph's own report on itself: how many files and edges it
  saw, how many import forms no resolver could turn into an edge, and how many
  changed files failed to parse. It exists so a small selection is never
  mistaken for a confident one.
- `--format list` prints bare paths and nothing else. Terminal output prints the
  totals line, the escalation, and one row per test.

Exit codes: `0` with a selection, `2` for a usage error, `3` when the diff or
the graph cannot be read.

## What the demo found and fixed

The first two defects below were found by running the command on this
repository's own history, not by reasoning about it — they are the most useful
thing Stage 0 produced. The third was found reviewing that fix.

- **Fixtures and helper modules were selected as tests.** The first version
  matched any path with a `tests/`, `test/`, or `__tests__/` component
  regardless of language, so `crates/leadline/tests/fixtures/c/decisions.c` — a
  C fixture a test reads — and `crates/leadline/tests/common/mod.rs` — a shared
  Rust module the harness never runs — were selected as targets. Each is now
  refused structurally rather than by a name blacklist: layouts are per
  language, the Rust layout rejects a nested module outright, and a data tree
  outranks any test-shaped name.
- **`ratio` could exceed 1.** `totals.test_files` counted path-convention test
  files only, so `crates/leadline/src/main.rs` — which carries an inline
  `#[cfg(test)] mod tests`, is selected at tier `graph`, and was not in the
  denominator — produced `156 of 155 test files, ratio 1.006`. The denominator
  is now the union of the path-convention files and the selected set, so it can
  never be smaller than the numerator.
- **The tightening then over-reached, and was scoped back.** Excluding helper
  directories from *name-form* matches made `src/common/app.test.ts` invisible,
  and since no test was then found for such a change, the ladder escalated its
  top-level directory — every test under `src/` for one changed file. Name-form
  matches are now suppressed only by data trees; layout matches keep the helper
  exclusion.

## Limitations

Read these before trusting a selection. Stage 0 proves the mechanism; it does
not prove the safety.

- **File-level selection only.** Test-case granularity needs the recorders, so
  the "test case level, file fallback" decision is realized as its fallback half
  for now. A single changed function selects every test in its file.
- **Weaker graph coverage in Go and Python.** Package-qualified imports produce
  no edges today, so T2 has less to reach and T3 escalates more often there.
  `unresolved` is the signal: a Go or Python repository with a large
  `confidence.unresolved` and a small `selected` is the failure mode those
  counters exist to expose.
- **A syntax-broken dependant is invisible.** Tree-sitter error recovery still
  returns a tree, so a file that fails to parse simply loses its outgoing edges
  and can drop a test from the closure. `unresolved` does not cover it —
  `unresolved` counts import forms that failed to resolve, not files that failed
  to parse. `confidence.parse_errors` is exact, and deliberately counts
  **changed** files only: those are parsed anyway to decide the tier, while a
  repo-wide count would cost a second full parse. A repo-wide parse-error count
  falls out of the index when the index lands.
- **`ratio` is a lower-bound denominator**, so it over-estimates the fraction of
  the suite being run. Content-detected inline test files are discovered only
  when a change reaches them, so a repository with heavy inline testing has more
  test files than `totals.test_files` knows about. Erring high here is the safe
  direction; the number should still not be quoted as the true size of the
  suite.
- **Anonymous functions are excluded** from recorded maps, because the parser
  names them by position (`<anonymous@LINE:COLUMN>`), which is unstable across
  any edit above them. A changed anonymous function escalates to its enclosing
  named function. The stable symbol key those maps will use is built and
  persisted, but selection does not read it in Stage 0.
- **Recall against actually-failing tests is not measured.** The number that
  would earn trust — whether a selection contains every test that actually
  failed — needs a shadow run and a runnable suite, and is a later stage.
  Nothing here should be described as safe, and a selection is evidence to
  review, not a proof of coverage.

## Follow-ups

An MCP tool mirroring `select-tests` does not exist yet; the MCP surface is a
separate concern and lands on its own. Beyond it: coverage recorders and the T1
tier, a persisted index (which is also what makes a repo-wide parse-error count
free), and monorepo package routing. `selected` entries carry a path, so
grouping them by workspace and runner later is additive.
