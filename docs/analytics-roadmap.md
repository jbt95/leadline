# Analytics roadmap

This document is the architecture proposal for evolving `leadline` from a
function-level metric analyzer into a local engineering-intelligence engine.
It records the module boundaries, the normalized analytics model, the static
report data contract, and the planned milestones. Milestone A (Git history and
hotspots) is implemented; later sections describe designed-but-unbuilt work.

The product question is not "how many metrics do we have" but "where is
engineering risk concentrated, why does it matter, and what does changed code
affect". Every number below exists to answer a question, not to be minimized.

## Architecture

```text
                     Source
                       │
           ┌───────────┼────────────┐
           ▼           ▼            ▼
        Static        Git        Coverage
        analysis     history       / tests
           │           │            │
           ▼           ▼            ▼
        Quality      Churn      Test strength
           │           │            │
           └──────┬────┴─────┬──────┘
                  │          │
                  ▼          ▼
             Dependencies  Ownership
                  │          │
                  └────┬─────┘
                       ▼
                  Risk models
                       │
         ┌─────────────┼──────────────┐
         ▼             ▼              ▼
        CLI           CI         AI agents
                                       │
                                       ▼
                               Static web analytics
```

Module boundary rules:

1. **Adapters never import each other.** `parser`/`core` produce source
   metrics; `history` produces Git facts; `coverage` produces line hits.
   Adapters speak only in normalized paths and versioned value types.
2. **Join layers own the keys.** `hotspots` joins source and history facts.
   Later join modules (`coupling`, `impact`, `risk`) join their own inputs.
   The join key is always the normalized repo-relative `/` path from
   `normalize_path`, and later function joins use path plus function identity.
3. **Reports are versioned artifacts.** Every public JSON shape carries a
   `schema_version`, a `metric_profile`, and an `analyzer_version`. Composite
   models carry an explicit model name.
4. **Interfaces are thin.** CLI, MCP, CI, and the static report consume the
   same canonical model. The frontend never reimplements metric logic.

Current modules:

| Module | Role | Status |
| --- | --- | --- |
| `core`, `parser`, `discovery`, `coverage`, `diff`, `report`, `sarif` | existing source-metric engine and changed-code analysis | released |
| `history` | Git history to normalized per-file facts | Milestone A |
| `hotspots` | source metrics x Git facts, ranked with exposed dimensions | Milestone A |
| `coupling` | temporal co-change (planned) | Milestone B |
| `graph`, `impact` | static dependencies, blast radius, cycles (planned) | Milestone C |
| `risk` | explainable change-risk model (planned) | Milestone D |
| `duplication`, `tests`, `policy` | clone detection, test relationships, architecture rules (planned) | Milestones H/I |
| `report` (extended) | canonical report model plus static site generator (planned) | Milestone F |

## Normalized analytics schema

The future `report-data.json` is one `Project` object. Sections are optional:
a snapshot without Git produces no `git_activity`; a run without coverage
produces no `coverage`. `null` always means unknown, never zero.

```text
Project
├── meta                 schema_version, analyzer_version, metric_profile, generated_from
├── summary              headline counts and KPI values
├── snapshots            historical trend points (Milestone G)
├── modules              aggregated hierarchy nodes
├── files                file rows
├── functions            function rows
├── dependencies         edges with kind (import/call), resolved path, confidence
├── cycles               strongly connected components
├── git_activity         file churn, recency, contributors
├── temporal_coupling    co-change edges with directional and Jaccard values
├── ownership            concentration per file/module (no individual rankings)
├── coverage             per function/file line coverage
├── mutation             per function/file mutation score and mutant counts
├── duplication          clone groups and newly introduced duplication
├── architecture_violations  rule name, source, target, severity
└── risk                 model name, score, explainable components
```

Cross-cutting rules:

- Paths use `/` separators and are relative to the analysis root.
- Every row has a stable identity: file path, or path plus function name and
  span for functions. Historical joins additionally tolerate renames through
  the Git rename map.
- Deterministic ordering: files by path, functions by source order, edges by
  `(source, target)`. Arrays are never in hash order.
- Each composite score exposes `model` and per-component contributions; raw
  metrics remain present next to any composite.
- Contributors are identities, not people: mailmap-applied email, lowercased,
  falling back to name. Reports document this.

Relation to today's shapes: `AnalysisReport`, `ChangedReport`, `HistoryReport`,
and `HotspotReport` are the current canonical artifacts. The `Project` model
aggregates them; it does not replace them until the report generator lands.

## Static report data contract (Milestone F)

```text
analysis engine (Rust, single source of truth)
     ↓
canonical report model
     ↓
report-data.json + static HTML/CSS/JS assets
```

Output layout:

```text
code-health-report/
  index.html
  assets/
  data/
    project.json
    modules.json
    files-0001.json   (only when partitioning is required)
```

Contract rules:

- The frontend renders and filters; it never computes metrics, scores, or
  rankings. Ordering and aggregation rules live in Rust.
- `index.html` opens from `file://` where the browser allows it, and the
  directory publishes unchanged to CI artifacts, GitHub/GitLab Pages, S3, or
  internal static hosting. No backend, no network calls at runtime.
- Canonical data is reproducible: same repository, configuration, analyzer
  version, Git snapshot, and coverage input produce byte-identical
  `report-data.json`. Generation time and machine details, if collected, live
  outside the canonical data (`meta.generated_at` is opt-in and excluded by
  default).
- Privacy controls: `--include-authors`, `--anonymize-authors`, and
  `--exclude-git-identities`. Author identities are excluded by default; the
  report shows contributor counts and concentration, never named rankings.
- Scale: aggregation plus paginated/virtualized tables and partitioned data
  files. Never emit millions of DOM nodes; start with aggregated views and
  load detail on drill-down.
- A single-file `--single-file report.html` embed is a later, optional mode.

## Git analytics implementation proposal

Options considered for history ingestion:

| Option | Pros | Cons |
| --- | --- | --- |
| `git` CLI streaming (chosen) | no new dependency or license review; matches existing `diff`; one audit-friendly code path | subprocess startup dominates small runs; output format is our parsing contract |
| `git2` / libgit2 | in-process history walk | C dependency, license and supply-chain review, thread-safety friction, larger binary |
| `gix` (pure Rust) | in-process, fast revision walking | large API surface, still maturing for rename/log parity; would need careful benchmarking |

Measured on the 2026-09-13 machine (see `docs/benchmark.md`): process startup
costs tens of milliseconds per spawn in that environment, while a development
probe over a synthetic 100,000-record stream parsed in about 20 ms
(≈0.2 µs per record; the committed bench groups measure the end-to-end
pipeline instead). History analysis therefore uses exactly two subprocesses
per run: one HEAD lookup and one streamed
`git log --relative --no-merges --numstat -z -M30%` walk. Git performs scope
filtering and path rekeying via `--relative`, so subdirectory analyses do not
walk or filter the whole repository. A `gix` spike is justified later, when
incremental snapshotting (Milestone G) needs random access to history rather
than one linear walk; it is not justified for Milestone A.

Caching plan (not yet implemented): cache history facts keyed by repository
identity plus `HEAD` commit hash, independently of source-metric caches. A
source edit must not invalidate Git aggregates, and a new commit must not
invalidate parsed source metrics.

## Milestones

| Milestone | Scope | Status |
| --- | --- | --- |
| A | Git history model, churn, recency, age, contributors, hotspots, CLI + JSON, fixtures, benchmarks | **implemented** |
| B | temporal coupling: co-change counts, directional coupling, Jaccard, `coupling` command | next |
| C | dependency extraction, fan-in/out, transitive dependents, blast radius, cycles | planned |
| D | explainable `change-risk-v1` model (complexity, CRAP, churn, impact, ownership, policy) | planned |
| E | diff intelligence: new vs existing vs resolved debt, risk regressions | planned |
| F | static web report MVP: overview, distributions, hotspots, explorers, dependencies | planned |
| G | historical snapshots and trends, treemap, coupling and cycle views | planned |
| H | mutation ingestion (PIT, Stryker) and test-to-code relationships | planned |
| I | duplication detection and architecture policies with drift detection | planned |

Deferred on purpose: additional OO metrics (NPath, LCOM, RFC, DIT, CBO, WMC)
are only added when a concrete question needs them.

## Open questions

- Function-level churn attribution: per-function `git blame` is too slow for
  whole repositories; candidate strategies are blame-on-demand for hotspots
  and per-function change counts from line-range tracking between snapshots.
- Temporal coupling thresholds: minimum support before a co-change edge is
  reported, and how to suppress mega-commits (formatting sweeps, dependency
  bumps) without hiding real coupling.
- Snapshot cadence and storage format for Milestone G; JSON first, compact
  binary only if scale requires it.
- Whether architecture policies evaluate the existing `-M30%` rename map or a
  stricter threshold, because policy violations must not appear or disappear
  based on rename heuristics.
