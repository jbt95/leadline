# Static PostgreSQL risk analysis

`leadline sql [PATH]` flags high-signal PostgreSQL query risks in `.sql`
files and obvious query call sites in Go, Java, JavaScript, TypeScript, TSX, and Rust —
without executing SQL, connecting to a database, or adding a parser
dependency. Findings are review prompts, not proof of runtime behavior or
index usage: estimates reflect the planner's view, and every rule is a
syntactic heuristic with documented boundaries.

C and C++ are analyzed for complexity and dependencies, but their query call
sites are not recognized yet: libpq (`PQexec`) and SQLite (`sqlite3_exec`)
calls stay invisible to this command, so a C or C++ repository reports only
the `.sql` file rules.

## Rules

| Rule | Severity | Contract | Remediation |
|---|---|---|---|
| `sql/update-delete-without-where` | high | statement is a top-level `UPDATE`/`DELETE` with no top-level `WHERE` (the verb must be the statement's first keyword, so foreign-key `ON DELETE` clauses do not match) | Add a `WHERE` clause to bound the rows this statement touches. |
| `sql/leading-wildcard` | medium | `LIKE`/`ILIKE` pattern is a literal beginning with unescaped `%` | Avoid a leading wildcard in `LIKE` patterns or back it with a trigram index. |
| `sql/nonsargable-predicate` | medium | `lower`/`upper`/`trim`/`date`/`date_trunc`/`cast` or an explicit `::` cast wraps the filtered column side of a `WHERE` comparison | Compare the bare column so indexes stay usable; move functions and casts to the literal side. |
| `sql/large-offset` | medium | numeric top-level `OFFSET` above `large_offset` (default 1000) | Replace large `OFFSET` pagination with keyset pagination. |
| `sql/unknown-table` | medium | referenced table absent from `CREATE TABLE` declarations under configured migration roots | Declare the table in a migration under a configured root or fix the reference. |
| `sql/dynamic-concatenation` | high | recognized `query`/`execute`-family call receives concatenation or an interpolated template | Pass query text as a literal with bound parameters instead of concatenation. |
| `sql/query-in-loop` | medium | recognized call has a loop ancestor before the containing function boundary | Move the query out of the loop or batch it into one round trip. |

Severities are fixed; only `large_offset` and migration roots are
configurable via `[sql]` (`minimum_severity` gates, it never rescores).

## Syntactic contracts and boundaries

- The tokenizer keeps shapes only: keywords, normalized identifiers, string
  wildcard prefixes, integer values, and operators. Literal text, comments,
  and string bodies never reach findings or debug output.
- `WHERE` scans stay inside the depth-zero clause ending at `GROUP`, `ORDER`,
  `LIMIT`, `OFFSET`, `RETURNING`, `UNION`, `INTERSECT`, or `EXCEPT`.
  Unknown or dynamic patterns (parameters, columns, non-listed functions)
  do not trigger statement rules.
- `UPDATE`/`DELETE` count only when the verb is the statement's first
  depth-zero keyword (after an optional leading `WITH`, whose CTE bodies are
  parenthesized). `ALTER TABLE ... ON DELETE CASCADE` and other foreign-key
  clauses are not unbounded DML.
- Table references come from `FROM`/`JOIN`/`UPDATE`/`INSERT INTO` and
  `DELETE ... USING` at any depth (subqueries included); table functions, CTE
  aliases, derived-table aliases, and scalar-function `FROM` argument markers
  (`extract(epoch FROM ts)`, `trim(both from name)`) are skipped. Every
  comma-separated `FROM` item and `DELETE ... USING` item is checked; unquoted
  names fold to lowercase and default to the `public` schema, while quoted
  names keep their case. Without configured migration roots no unknown-table
  findings emit (`schema_evidence_available: false`).
- Host calls match terminal `query`, `execute`, `executeQuery`,
  `executeUpdate`, and `raw` names only — declarations and bare references
  never match. Dynamic means a `+` concatenation or template substitution in
  the first argument; parameterized literals stay quiet, and nested functions
  are not entered, so a callback computing `a + b` is not the call's text.
  Loop state stops at the nearest function boundary, so nested functions are
  never blamed for an outer loop. Call receivers and argument text never
  cross over.
- Rust call sites are recognized on `sqlx::query(...)`, `sqlx::query_as(...)`,
  `sqlx::query_scalar(...)`, method calls such as `conn.execute(...)` or
  `client.query(...)`, and any call whose terminal name matches the
  `query`/`execute` family above. A Rust call site is dynamic when its first
  argument holds a `format!` or `concat!` macro invocation or a `+`
  concatenation. Loop attribution is unchanged: a call inside a loop, before
  any enclosing function boundary, is flagged `inside_loop`.
- One finding per statement per rule (host sites: one per rule per site).

## Limits and inputs

Invalid UTF-8, unclosed quotes/comments/dollar quotes (nested block comments
are supported), parenthesis nesting past 128, files past 64 MiB, and inputs
past 1,000,000 tokens are input errors (exit `4`). E-string backslash escapes
(`E'it\'s'`) stay inside the literal. Explicit `.sql` files are analyzed
directly. `.sql` discovery honors fixed ignored directories plus configured
excludes; symlinks are skipped. Every file is read once per analysis.
Declared-table evidence covers `CREATE [OR REPLACE]
[GLOBAL|LOCAL] [TEMP|TEMPORARY|UNLOGGED] TABLE [IF NOT EXISTS]` and
`ALTER TABLE [IF EXISTS] [ONLY] old RENAME TO new` targets, and every CTE
alias in a statement counts as known.

## Output shapes

- `--json`: the complete report (`findings` plus `schema_evidence_available`).
- `--format agent-json --top N` (default 50): first `N` findings plus
  `truncated`.
- `--format sarif`: rule IDs equal the SQL rule IDs, messages carry the
  fixed remediation strings.
- Terminal: one `path:line [severity] rule` line per finding.
- `leadline check --sql --sql-fail-on-severity LEVEL` adds
  `sql_violations` to check JSON and fails when any family fails; its SARIF
  output merges gate violations only. The read-only `sql_risks` MCP tool takes
  `path`, `large_offset`, `migration_roots`, `minimum_severity`, and `top`
  with root-relative paths, applies `[analysis].exclude` like the CLI, and
  falls back to `[sql]` for unset `large_offset`/`migration_roots`.

Findings carry rule ID, severity, path, span, optional containing function
ID, and remediation; unknown evidence serializes as `null`.
