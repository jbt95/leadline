# Task 3 Report: Zig SQL host-call analysis

## Status
Closed out on `b750084`. The SQL parser behavior remains unchanged in this closeout. Documentation now records the direct Zig host-call heuristic and its boundaries.

## TDD evidence
- **Red:** Before `b750084`, `cargo test --offline --test sql host_sql_matrix_across_languages -- --exact` failed at the Zig matrix case with `src/db.zig: []` (exit 101), because the Zig match arm did not yet exist.
- **Green:** After the implementation, the same focused command passed with 1 test passed and 0 failed.

## Documentation change
- Added Zig to the host-language list in `docs/postgresql-risks.md`.
- Documented direct `call_expression` matching for bare identifiers and `field_expression.member` terminal names in the existing query/execute set.
- Documented first substantive named-child argument selection with extras/comments skipped, the published grammar's missing `arguments` field, `++` dynamic concatenation, arithmetic `+` staying quiet, loop attribution through `for_statement`/expression loops until the function boundary, and no SQL-text retention.
- Documented that package, build-script, and runtime wrappers that hide direct calls remain out of scope and that package/build/runtime code is not executed.

## Checks
- `cargo fmt --check` — **passed**. An initial check exposed only rustfmt line wrapping in the prior Task 3 test; the formatting-only adjustment was applied before the final check.
- `cargo clippy --offline --all-targets` — **passed** with no warnings.
- `cargo test --offline --test sql host_sql_matrix_across_languages -- --exact` — **passed**: 1 passed, 0 failed.
- `git diff --check` — **passed**.
- Full test suite — **not run**, per task instructions.

## Changed files
- `docs/postgresql-risks.md` — Zig SQL heuristic and limitations.
- `tests/sql.rs` — rustfmt-only line wrapping; no behavior change.
- `.superpowers/sdd/2026-09-25-zig-language-support/task-3-report.md` — this report.

## Self-review
- No SQL parser, fixture, graph, unused, integration, or CLI behavior was changed in this closeout.
- The documentation distinguishes direct syntax from package/build/runtime wrapper indirection and does not claim runtime proof.
- SQL literals remain excluded from serialized findings; only rule, span, path, and optional function metadata are documented as retained.
- The focused regression remains green after documentation and formatting changes.
