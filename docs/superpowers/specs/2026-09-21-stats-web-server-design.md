# Local metrics server (`leadline stats`) design

**Status:** Approved in chat on 2026-09-21; awaiting written-spec review.

**Scope:** Serve the canonical `Project` model over loopback with an embedded,
hand-written page, so a human can see every metric leadline produces without
reading terminal output or JSON.

## Goals

1. One command analyzes once, serves the page and the canonical JSON, and can
   re-analyze on demand from a button.
2. The page renders every section of `Project`, and names the missing input
   when a section cannot be computed.
3. Zero new dependencies and no build step: reuse the HTTP layer already
   hardened inside `src/mcp.rs`, embed hand-written HTML/CSS/JS with
   `include_str!`, draw charts as inline SVG.
4. Loopback by default, with the same `Host`/`Origin` validation MCP uses, and
   read-only with respect to the analyzed repository.
5. Keep the frontend a pure renderer: it sorts, filters, and formats; it never
   computes a metric, a score, or a ranking.

## Non-Goals

- File watching or automatic re-analysis.
- The static `code-health-report/` export, partitioned data files, and the
  single-file `report.html` mode (roadmap Milestone F, still open).
- Authentication, TLS, or serving beyond loopback; the page makes no network
  calls and loads no CDN asset.
- Any write action from the page: no threshold editing, no baseline writing,
  no snapshot capture.
- A JavaScript build step, a framework, a chart library, or an npm dependency.
- New metrics, new joins, or new fields on the `Project` contract: the page
  renders exactly what `Project` carries. Per-function Halstead values and the
  maintainability index are not part of it and stay available through
  `analyze --json`; adding them to `Project` is a schema change with its own
  review.

## Supersedes

The 2026-09-13 remaining-analytics-milestones spec listed "a report server,
network calls, telemetry, or a single-file HTML mode" as non-goals. That entry
is superseded **only** for a loopback-bound, read-only local server. Network
calls from the page, telemetry beyond the existing opt-in store, and the
single-file mode remain non-goals. The reason is deliberate: the local server
is the fastest way to get Milestone F's UI right, and a page that only consumes
canonical JSON can be exported statically later without changing any rendering
logic.

## Architecture

```text
leadline stats [PATH] ──▶ build_project(...)      same code path as `project`
                              │  (once, at launch)
                              ▼
                    Arc<RwLock<Snapshot>>          serialized Project bytes + meta
                              │
        ┌─────────────────────┼──────────────────────┐
        ▼                     ▼                      ▼
   GET / and assets    GET /api/project       POST /api/refresh
   embedded page       canonical JSON         re-analyzes on that connection,
        │                     ▲                swaps the snapshot, returns meta
        └──── page fetches and re-renders ───────────┘

src/http.rs  ── shared: bind with free-port fallback, request read, response
                write, deadlines, connection ceiling, Host/Origin guards
src/mcp.rs   ── uses src/http.rs (transport behavior unchanged)
src/stats.rs ── routes, snapshot state, options parsing, embedded assets
```

Module boundaries:

1. `src/http.rs` owns the transport. Both MCP and stats sit on it, so one
   audited code path serves HTTP. Nothing about MCP's observable behavior
   changes; its existing tests are the guard.
2. `src/stats.rs` owns routing, the snapshot, and option parsing. It never
   parses source code: it calls the same project builder the `project` command
   calls and keeps the serialized result.
3. `src/stats/` holds `index.html`, `stats.css`, and `stats.js`, embedded with
   `include_str!`. No template engine, no bundler.
4. The analysis engine, adapters, and joins are untouched.

## Command surface

```console
leadline stats [PATH] [--port [N]] [--host ADDR] [--open]
               [--lcov FILE | --jacoco FILE | --coverage FILE]
               [--since 30d|90d|365d] [--target REV]
               [--pit FILE | --stryker FILE | --test-map FILE]
               [--snapshots FILE]
               [--include-authors | --anonymize-authors | --exclude-git-identities]
```

- `PATH` defaults to `.`; every analysis flag is the one `project` already
  parses, forwarded to the same builder.
- `--port` defaults to 3000 (a bare `--port` means 3000, `0` asks the OS for a
  free port, and a taken port falls back to a free one with a note on stderr).
  Unlike `mcp`, serving is the point, so the flag is optional rather than
  opt-in.
- `--host` defaults to `127.0.0.1` and does not require `--port`.
- `--open` launches the default browser at the served URL, best effort. The
  browser is never opened without the flag, even on a terminal.
- The URL line goes to **stderr** (`leadline: stats on http://127.0.0.1:3000`),
  so stdout stays clean for pipelines.
- Exit codes follow the CLI contract: `2` usage, `3` incomplete analysis,
  `4` input error. An interrupt ends the process the way it does for every
  other command; there is no handler and no shutdown write.
- `stats` joins `CLI_OPERATIONS` (31 → 32 entries), the dispatch match, the
  usage text, and its `cli_operation` telemetry label.

## HTTP surface

| Method | Path | Response |
| --- | --- | --- |
| GET | `/` | the embedded page |
| GET | `/stats.css`, `/stats.js`, `/logo.svg` | embedded assets (`/logo.svg` is the shipped `assets/logo.svg`, included at build time) |
| GET | `/api/project` | canonical `Project` JSON, byte-identical to `project --json` |
| POST | `/api/refresh` | `200` with fresh `meta` once the re-analysis finishes on that connection; `409` when one is already running |
| GET | `/health` | `{"status":"ok","analyzed_at":…,"head_commit":…,"analyzer_version":…}` |

Rules:

- The transport reuses MCP's limits unchanged: 8 KiB request line, 64 KiB
  header block, 32 MiB body, 5 s read and 10 s write deadlines, 64 concurrent
  connections with 503 beyond, and the same loopback `Host`/`Origin` rules
  (a loopback-bound listener answers only loopback authorities; a browser
  origin that is not loopback is 403).
- `/api/project` serves the bytes serialized once per analysis, so repeated
  fetches never re-serialize and never re-analyze.
- `POST /api/refresh` holds a single-flight guard: a second request while one is
  running answers `409 refresh already running` instead of stacking work.
  Refresh runs synchronously on its own connection — the listener serves the
  current snapshot on every other connection meanwhile — and the swap happens
  only after a complete, successful analysis, so a failed refresh leaves the
  previous snapshot served.
- Unknown paths are `404`, unsupported methods on a known path are `405`,
  and both are plain JSON error bodies like MCP's.

## The page

Sections, in order, with the `Project` field each one reads:

| Section | Source | Unavailable when |
| --- | --- | --- |
| Header: path, HEAD commit and timestamp, analyzer version, analyzed-at, duration, Refresh, Download JSON | `meta` | never |
| KPI strip: files, functions, parse errors, dependency edges, cycles, duplication groups and lines, coverage percent, mutation score, policy violations, risk model | `summary` | individual tiles show `—` |
| Complexity distribution: cyclomatic and cognitive histograms, plus a CRAP distribution when coverage exists | `files`, `functions` | CRAP needs `coverage` |
| Hotspots: ranked rows with the `change-risk` score and its components | `risk` | `git_activity` absent → "no Git history: churn, ownership and impact weighting unavailable" |
| Explorer: files table (language, functions, LOC, max cognitive, max cyclomatic, max CRAP, coverage, parse errors) with sort and filter, drilling into a file's functions | `files`, `functions` | never |
| Dependencies: fan-in and fan-out leaders, the edge list, unresolved references, cycles | `dependencies`, `cycles` | never |
| Coupling: related-file pairs with directional and Jaccard values | `temporal_coupling` | absent section → "no Git history" |
| Ownership: concentration per module, contributor counts (never named rankings) | `ownership` | absent, or all-anonymous |
| Coverage: per-file coverage and CRAP | `coverage` | no coverage input → "no coverage input: pass --lcov, --jacoco, or --coverage" |
| Mutation: per-file mutation score and mutant counts, plus test relationships | `mutation`, `test_relationships` | no reports → "no mutation report: pass --pit or --stryker" |
| Duplication: clone groups with their occurrences | `duplication` | never |
| Architecture violations: rule, source, target, severity | `architecture_violations` | never (empty means clean) |
| Trends: snapshot points when supplied | `snapshots` | absent → section hidden |

Rendering rules:

- A missing section states its reason in one line; it never renders zero, and
  never hides silently.
- Tables render a windowed slice of the sorted rows — 200 rows per page with
  paging controls — so a large repository does not build a million DOM nodes.
  Sorting and filtering happen client-side over the fetched array.
- Charts are inline SVG drawn by hand: histograms for distributions, and
  proportional bars for rankings. No canvas physics, no dependency graph
  simulation; dependencies render as ranked lists plus cycle groups.
- Palette comes from the shipped logo: ground `#0e1626`, text `#e8eef6`,
  accent `#38bdf8`, muted `#64748b`. Status is conveyed by text as well as
  color. Rows are keyboard-focusable; the page is usable at 1280 px and above,
  and remains readable when narrowed.
- The page works with no network beyond the server: no external fonts, no CDN.

## Honesty and security rules

- The page never computes a metric, score, or ranking; it formats, sorts, and
  filters canonical values only. Every number shown is traceable to a `Project`
  field.
- `stats` never writes to the analyzed repository, and it never shells out to
  anything but the existing Git adapter inside the analysis path.
- Loopback by default; `--host` is the only way to widen the bind, and the
  `Host`/`Origin` guards still apply.
- `/api/project` doubles as the no-JavaScript path: the same data any consumer
  would get from `project --json`.
- No telemetry beyond the existing opt-in metrics store.

## Testing

- `tests/stats.rs`, starting a real server on port 0 exactly as `tests/mcp.rs`
  does for HTTP: `GET /` returns the page with the expected marker;
  `/api/project` parses and carries `schema_version` 1; `POST /api/refresh`
  advances `analyzed_at` and records a duration; a concurrent refresh gets
  `409`; an analysis failure during refresh keeps the previous snapshot and
  answers `500`; unknown paths `404`; wrong methods `405`; a foreign `Origin`
  `403`; a non-loopback `Host` `400`.
- Unit tests in `src/stats.rs` for option parsing (port default, bare `--port`,
  `--host` without `--port`, `--open` accepted, unknown flag rejected) and for
  the refresh guard.
- Asset tests: the embedded page, CSS, and JS are non-empty and contain their
  expected markers, so a build that loses an asset fails loudly.
- Regression: `analyze --json` and `project --json` byte-identical before and
  after, proving the transport extraction and the new command change nothing,
  and the existing `tests/mcp.rs` HTTP cases pass unmodified, proving MCP's
  transport behavior is unchanged by the extraction.
- Payload measurement on this repository and a large corpus, recorded in the
  pull request discussion; `?section=` filtering is added only if the numbers
  demand it.
- The visual result is reviewed in a real browser before the work is called
  done.

## Files touched

`src/http.rs` (new, extracted from `src/mcp.rs`), `src/mcp.rs` (uses it),
`src/stats.rs` (new), `src/stats/index.html`, `src/stats/stats.css`,
`src/stats/stats.js` (new assets; the logo is served from the existing
`assets/logo.svg` through `include_str!`), `src/main.rs`
(operation list, dispatch, usage), `tests/stats.rs` (new),
`docs/cli-reference.md`, `docs/architecture.md`, `docs/security-model.md`,
`docs/analytics-roadmap.md`, `docs/telemetry.md` (operation label),
`README.md`, `CHANGELOG.md`.

## Acceptance criteria

1. `leadline stats --port 0` prints a loopback URL and serves page, assets,
   JSON, refresh, and health on it.
2. The page shows every section of `Project` for a repository with Git,
   coverage, and duplication data, and states the exact reason for each section
   it cannot compute on a repository without them.
3. Refresh re-analyzes and the page shows the new `analyzed_at`; a second
   concurrent refresh is rejected with `409`.
4. `cargo fmt --check`, `cargo clippy --offline --all-targets --locked --
   -D warnings`, and `cargo test --offline --locked` pass.
5. `analyze --json` and `project --json` are byte-identical to the previous
   release for the same inputs.
6. Documentation lists the command, the endpoints, the security posture, and
   the roadmap status.

## Risks

- **Transport extraction.** Moving the HTTP layer out of `src/mcp.rs` touches
  security-sensitive code; MCP's existing HTTP tests, plus a byte-identical
  MCP behavior check, guard it.
- **Payload size.** A very large `Project` on a huge repository may be slow to
  fetch or render. Measured first; `?section=` filtering is the escape hatch.
- **DOM cost.** A framework-free page must stay windowed or large repositories
  will lock the browser.
- **Scope creep into Milestone F.** Static export, partitioning, and the
  single-file mode stay out; they are cheap later because the page already
  depends only on canonical JSON.

## Open questions

- Whether to extend `Project` with Halstead and maintainability-index fields so
  the explorer can show them, or to leave that detail to `analyze --json`. A
  schema change with its own review; deferred.
- Whether the page should offer per-section JSON downloads in addition to the
  full document. Deferred: the full download plus browser dev tools covers it.
- Whether `/health` should expose the refresh state (`idle`/`running`) or stay
  minimal. Leaning minimal, with the state visible in the page.
