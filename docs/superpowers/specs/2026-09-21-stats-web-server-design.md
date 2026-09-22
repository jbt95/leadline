# Local metrics server (`leadline stats`) design

**Status:** Approved in chat on 2026-09-21. **Amended 2026-09-22** to describe
what shipped: the page is no longer hand-written HTML/CSS/JS, it is a bundled
React dashboard. See *Supersedes*.

**Scope:** Serve the canonical `Project` model over loopback with an embedded
page, so a human can see every metric leadline produces without reading
terminal output or JSON.

## Goals

1. One command analyzes once, serves the page and the canonical JSON, and can
   re-analyze on demand from a button.
2. The page renders the sections listed under *The page*, and names the missing
   input when a section cannot be computed.
3. Reuse the HTTP layer already hardened inside `src/mcp.rs`; embed the built
   dashboard with `include_str!` so the binary serves a complete UI with nothing
   beside it and the runtime stays offline.
4. Loopback by default, with the same `Host`/`Origin` validation MCP uses, and
   read-only with respect to the analyzed repository.
5. Keep the frontend a pure renderer: it sorts, filters, and formats; it never
   computes a metric, a score, or a ranking over `Project`.

## Non-Goals

- File watching or automatic re-analysis.
- The static `code-health-report/` export, partitioned data files, and the
  single-file `report.html` mode (roadmap Milestone F, still open).
- Authentication, TLS, or serving beyond loopback; the page makes no network
  calls and loads no CDN asset.
- Any write action from the page: no threshold editing, no baseline writing,
  no snapshot capture.
- Server-side rendering, a dev server at runtime, or installing anything at
  runtime. `web/node_modules` exists only to build `web/dist`.
- New metrics, new joins, or new fields on the `Project` contract: the page
  renders exactly what `Project` carries. Per-function Halstead values and the
  maintainability index are not part of it and stay available through
  `analyze --json`; adding them to `Project` is a schema change with its own
  review.

## Supersedes

**Superseded 2026-09-22 — "Zero new dependencies and no build step" (Goal 3),
and the Non-Goal "A JavaScript build step, a framework, a chart library, or an
npm dependency."** The page ships as a React 18 + TypeScript app built with
Vite and Tailwind, charting with Recharts and icons from lucide-react. The
bundle is produced by `npm run build` in `web/`, checked in under `web/dist`,
and embedded at compile time. What survives from the original intent is the
part that mattered: **the runtime is offline and dependency-free.** The binary
serves three embedded assets and the page fetches only its own server. The
build-time dependency tree is a development cost, not a runtime one. The
reason for the change is that hand-written SVG charting did not scale to the
treemap, sankey, and scatter views the sections needed.

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
web/         ── the dashboard source; web/dist is what gets embedded
```

Module boundaries:

1. `src/http.rs` owns the transport. Both MCP and stats sit on it, so one
   audited code path serves HTTP. Nothing about MCP's observable behavior
   changes; its existing tests are the guard.
2. `src/stats.rs` owns routing, the snapshot, and option parsing. It never
   parses source code: it calls the same project builder the `project` command
   calls and keeps the serialized result.
3. `web/` holds the dashboard. `web/dist/index.html`, `web/dist/assets/app.js`,
   and `web/dist/assets/index.css` are embedded with `include_str!`; the logo is
   served from `assets/logo.svg`. `web/dist` is checked in and hash-free by
   Vite config, so the embedded paths stay stable across rebuilds.
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
| GET | `/` | the embedded page (`web/dist/index.html`) |
| GET | `/assets/app.js`, `/assets/index.css` | the embedded bundle and stylesheet |
| GET | `/logo.svg` | the shipped `assets/logo.svg`, included at build time |
| GET | `/api/project` | canonical `Project` JSON, byte-identical to `project --json`; `schema_version` is at `meta.schema_version` |
| GET | `/api/telemetry` | the opt-in store: `ok`, `disabled`, or `error` over 1 MiB, plus a `series` array and per-operation p90 summaries |
| POST | `/api/refresh` | `200` with fresh `meta` once the re-analysis finishes on that connection; `409` when one is already running |
| GET | `/health` | `{"status":"ok","analyzed_at":…,"duration_ms":…,"head_commit":…,"analyzer_version":…}` |

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

One route renders at a time behind a hash URL (`#/overview` … `#/policy`), so
every view is deep-linkable and the browser's back and forward work.

| Route | Reads | Unavailable when |
| --- | --- | --- |
| Overview: quality gate verdict, KPI measures, coverage and mutation mix, top risk, language mix | `summary`, `coverage`, `mutation`, `architecture_violations`, `risk`, `files` | individual tiles show `—` |
| Measures: files, functions, coverage, mutation score, duplication, dependency edges and cycles, policy count, risk model, plus a files table | `summary`, `files` | never |
| Complexity: cyclomatic/cognitive distributions and a complexity-vs-coverage scatter | `functions[]` | CRAP needs `coverage` |
| Hotspots: risk treemap, churn-vs-complexity scatter, risk-component bars | `risk` | `git_activity` absent → "no Git history: churn, ownership and impact weighting unavailable" |
| Coupling: related-file pairs with directional and Jaccard values | `temporal_coupling` | absent section → "no Git history" |
| Trends: snapshot points when supplied | `snapshots` | absent → current-position baseline card |
| Telemetry: invocation counts, latency percentiles, live CPU/RSS, per-operation p90 | the `LEADLINE_METRICS_DIR` store | telemetry off → stated when `LEADLINE_METRICS_DIR` is unset |
| Policy: violations grouped by rule with worst severity and count | `architecture_violations` | never (empty means clean) |

Not rendered as sections of their own: `dependencies`, `ownership`, and
`duplication` reach the page only through the summary measures. Adding them is
future work with its own scope.

Rendering rules:

- A missing section states its reason in one line; it never renders zero, and
  never hides silently.
- The two full-length tables — the files table on Measures and the
  risk-component list on Hotspots — render a windowed slice of the sorted rows,
  **200 rows per page with paging controls**, so a large repository does not
  build a million DOM nodes. Sorting and filtering happen client-side over the
  fetched array. The summary cards that say "top N" are deliberately windowed
  rankings and are labelled as such.
- Charts are drawn from Recharts: histograms for distributions, proportional
  bars for rankings, a treemap for risk concentration, a sankey for co-change
  pairs. No canvas physics, no dependency-graph simulation.
- Palette: light ground `#f3f3f3`, card `#ffffff`, ink `#333333`, muted
  `#777777`, accent `#4b9fd5` with `#1d75b3` deep, `#00a94f` positive,
  `#d4333f` negative, `#ed7d20` warning. Status is conveyed by text as well as
  color. Rows are keyboard-focusable; the page is usable at 1280 px and above,
  and remains readable when narrowed.
- The page works with no network beyond the server: no external fonts, no CDN.
- Page chrome beyond the routes: a command palette for jumping between routes,
  a collapsible sidebar whose state persists in `localStorage`, and a mobile
  drawer under narrow widths. These are navigation aids only; they read no
  data and change no numbers.

## Honesty and security rules

- The page never computes a metric, a score, or a ranking **over `Project`**:
  it formats, sorts, and filters canonical values only, and every `Project`
  number shown is traceable to a `Project` field. It may derive chart geometry
  from canonical values — binning a histogram, a donut segment remainder, an
  interpolated percentile over telemetry buckets — because those are
  presentation shapes, not new measurements. Numbers sourced from the
  telemetry store are traceable to a store field.
- `stats` never writes to the analyzed repository, and it never shells out to
  anything but the existing Git adapter inside the analysis path.
- Loopback by default; `--host` is the only way to widen the bind, and the
  `Host`/`Origin` guards still apply.
- `/api/project` doubles as the no-JavaScript path: the same data any consumer
  would get from `project --json`.
- No telemetry beyond the existing opt-in metrics store.

## Testing

- `tests/stats.rs`, starting a real server on port 0 exactly as `tests/mcp.rs`
  does for HTTP: the endpoint matrix above, the `Origin`/`Host` guards, and
  build-time asset integrity (`include_str!` of the three embedded assets,
  non-empty with their markers, so a build that loses an asset fails loudly).
- Unit tests in `src/stats.rs` for option parsing (port default, bare `--port`,
  `--host` without `--port`, `--open` accepted, unknown flag rejected) and for
  the refresh guard, including the 409 and the 500-that-keeps-the-snapshot.
- `web/test/pure.test.mjs` via `node --test` (zero dependencies) for the pure
  helpers the page shares with its tests: cumulative-bucket quantiles with
  explicit expected values, histogram merging, and hash-slug resolution.
- Regression: `analyze --json` and `project --json` byte-identical before and
  after for a fixed fixture, proving the transport extraction and the new
  command change nothing, and the existing `tests/mcp.rs` HTTP cases pass
  unmodified apart from the `mcp::DEFAULT_HTTP_PORT` → `http::DEFAULT_PORT` and
  `bind_http` → `bind` renames, proving MCP's transport behavior is unchanged
  by the extraction.
- Payload measurement on this repository and a large corpus, recorded in the
  pull request discussion; `?section=` filtering is added only if the numbers
  demand it.
- The visual result is reviewed in a real browser before the work is called
  done.

## Files touched

`src/http.rs` (new, extracted from `src/mcp.rs`), `src/mcp.rs` (uses it),
`src/stats.rs` (new), `src/telemetry.rs` (store summaries and live gauges for
`/api/telemetry`), `web/**` (the dashboard and its checked-in `web/dist`),
`assets/stats-*.png` (README screenshots), `src/main.rs` (operation list,
dispatch, usage), `tests/stats.rs` (new), `docs/cli-reference.md`,
`docs/architecture.md`, `docs/security-model.md`, `docs/analytics-roadmap.md`,
`docs/telemetry.md` (operation label and the stats page), `README.md`,
`CHANGELOG.md`.

## Acceptance criteria

1. `leadline stats --port 0` prints a loopback URL and serves page, assets,
   JSON, telemetry, refresh, and health on it.
2. The page shows each of its routes for a repository with Git, coverage, and
   duplication data, and states the exact reason for each section it cannot
   compute on a repository without them.
3. Refresh re-analyzes and the page shows the new `analyzed_at`; a second
   concurrent refresh is rejected with `409`.
4. `cargo fmt --check`, `cargo clippy --offline --all-targets --locked --
   -D warnings`, and `cargo test --offline --locked` pass, as do
   `npm test` and `npm run build` in `web/`.
5. `analyze --json` and `project --json` are byte-identical to the previous
   release for the same inputs.
6. Documentation lists the command, the endpoints, the security posture, and
   the roadmap status.

## Risks

- **Transport extraction.** Moving the HTTP layer out of `src/mcp.rs` touches
  security-sensitive code; MCP's existing HTTP tests, plus a byte-identical
  MCP behavior check, guard it.
- **Checked-in bundle drift.** `web/dist` is committed and embedded. A source
  fingerprint (`web/dist/.src-hash`, written by `npm run build` and checked by
  `tests/stats.rs` with no node involved) fails the offline suite when `web/src`
  changes without a rebuild; `npm run build && git diff --exit-code web/dist`
  remains the escape hatch for build nondeterminism. It needs registry access,
  so it does not live in the offline test path.
- **Build-time dependency tree.** Six direct npm dependencies and a full lock
  file for one dashboard. The runtime is unaffected, but a fresh checkout needs
  `npm install` before `web/dist` can be regenerated.
- **Payload size.** A very large `Project` on a huge repository may be slow to
  fetch or render. Measured first; `?section=` filtering is the escape hatch.
- **DOM cost.** Even windowed, large repositories cost render time.
- **Scope creep into Milestone F.** Static export, partitioning, and the
  single-file mode stay out; they are cheap later because the page already
  depends only on canonical JSON.

## Open questions

- Whether to extend `Project` with Halstead and maintainability-index fields so
  the explorer can show them, or to leave that detail to `analyze --json`. A
  schema change with its own review; deferred.
- Whether `dependencies`, `ownership`, and `duplication` deserve routes of
  their own. Deferred: the summary measures cover the triage case.
- Whether the page should offer per-section JSON downloads in addition to the
  full document. Deferred: the full download plus browser dev tools covers it.
- Whether `/health` should expose the refresh state (`idle`/`running`) or stay
  minimal. Leaning minimal, with the state visible in the page.
