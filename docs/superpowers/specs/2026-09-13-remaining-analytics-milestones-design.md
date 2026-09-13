# Remaining Analytics Milestones Design

**Status:** Approved in chat on 2026-09-13; awaiting written-spec review.

**Scope:** Complete roadmap Milestones E-I in one uninterrupted delivery while
preserving milestone-sized commits and review checkpoints.

## Goals

1. Compare complete repository states and explain new, existing, and resolved
   debt plus file-risk regressions.
2. Produce one deterministic canonical project model and a zero-network static
   report from the same analysis values used by CLI, CI, and agents.
3. Persist compact, HEAD-keyed trend snapshots explicitly and atomically.
4. Ingest PIT and Stryker mutation reports and explicit test-to-code maps.
5. Detect token-level duplication and enforce dependency architecture rules,
   including drift against a Git base.
6. Finish the roadmap without introducing a web framework, template engine,
   database, Git library, or plugin framework.

## Non-Goals

- Running mutation tools or tests.
- Guessing test relationships from names or directory layout.
- Runtime dependency discovery, package-manager resolution, call graphs, or
  reflection analysis.
- Named developer rankings or identities in reports by default.
- A report server, network calls, telemetry, or a single-file HTML mode.
- Automatic snapshot writes during analysis or report generation.
- Binary snapshot storage, incremental Git-history caching, or a plugin SDK.
- Additional OO metrics or function-level churn attribution.

## Architecture

The implementation adds three shared seams and keeps milestone logic in focused
modules:

```text
Git/worktree/index/revision -> GitAdapter (no lazy fetch, no optional locks)
          |
          v
    SnapshotContext + SourceSnapshot -----------+
          |                                     |
          +--> analysis                         +--> before/after comparison
          +--> dependency graph                 +--> debt/risk/policy/clone drift
          +--> normalized tokens                |
                                                |
filesystem + history + external reports         |
          |                                     |
          +-------------------> Project <--------+
                                  |
                 +----------------+----------------+
                 v                v                v
                CLI          agent/MCP JSON    static site/snapshots
```

`GitAdapter` is the single subprocess seam for all existing and new Git
operations. `SourceSnapshot` is a read-only adapter. `Project` is the only
broad join module. Static analyzers (`duplication`, `policy`), external adapters
(`mutation`, `test_relationships`), and comparison modules (`debt`, drift
functions) never render output or write files.

Existing reports and commands remain valid. New modules reuse normalized
analysis-root-relative `/` paths, versioned envelopes, deterministic arrays, and the
existing error/exit-code conventions.

## Source Snapshots

Add `src/git.rs` and migrate existing `diff`/`history` Git subprocesses to it.
Every invocation sets `GIT_NO_LAZY_FETCH=1` and `GIT_OPTIONAL_LOCKS=0`; no
analysis command may contact a promisor remote or acquire optional repository
locks. Missing partial-clone objects are incomplete input (`3`), not an excuse
to fetch. Commands use raw object reads and never invoke filters or hooks.

Add `src/source_snapshot.rs` with:

```rust
pub enum SnapshotTarget {
    Worktree,
    Index,
    Revision(String),
}

pub struct SourceEntry {
    pub path: String,
    pub bytes: Vec<u8>,
}

pub struct SnapshotContext {
    pub repo_root: Option<PathBuf>,
    pub analysis_root: PathBuf,
    pub scope_prefix: String,
    pub config: Config,
}

pub struct SourceSnapshot {
    pub target: SnapshotTarget,
    pub commit: Option<String>,
    pub commit_timestamp: Option<i64>,
    pub entries: Vec<SourceEntry>,
}
```

Public keys and policy globs are always analysis-root-relative, matching current
reports. Filesystem roots are process location only and are never serialized.
`scope_prefix` maps Git's repo-relative paths into the analysis root. One
validated target/current `Config` is passed explicitly to both comparison
sides. Explicit file paths retain today's exception to directory excludes.

Entries are supported regular source files only, sorted by path, scoped before
blob loading, and filtered uniformly by the supplied analysis excludes.
One `SourceFilter` combines supported-language detection, the existing fixed
ignored directory components (`.git`, `node_modules`, `target`, `dist`, `build`,
`coverage`, `.next`, `.gradle`, `vendor`, `generated`), and configured excludes.
It is applied identically to Git-backed
worktree/index/revision and non-Git enumeration, with the existing explicit-file
exception.
Inside Git, worktree enumeration is the union of `git ls-files --cached -z` and
`git ls-files --others --exclude-standard -z`, so tracked-but-ignored files are
not accidentally lost. It uses `symlink_metadata`, never follows symlinks, and
skips symlinks and non-regular files. Outside Git, worktree enumeration falls
back to existing ignore-aware discovery with the same no-follow and regular-file
rules; Git analytics are unavailable. Index enumeration parses mode, OID, and
stage from `ls-files -s -z`: unmerged stages are incomplete (`3`), intent-to-add
zero OIDs are omitted, and only regular blob modes `100644`/`100755` are read.
Revision enumeration parses mode/type/OID/path from `ls-tree -rz`; symlinks,
submodules, and non-blobs are skipped. Any non-UTF-8, NUL-containing, or control
path is input data error (`4`). Revisions are validated using current `diff`
rules.

Revision and index blobs are streamed with `git cat-file --batch`, after one
enumeration, through `GitAdapter`. The implementation never checks out, resets,
stashes, fetches, or writes repository state. Worktree bytes use normal
filesystem reads after the metadata check.

Add in-memory entry points:

```rust
pub fn analyze_sources(entries: &[SourceEntry], coverage: Option<&CoverageMap>)
    -> Result<AnalysisReport>;

pub fn analyze_dependencies_from_sources(entries: &[SourceEntry])
    -> Result<DependencyReport>;

pub fn analyze_git_at(context: &SnapshotContext, revision: &str)
    -> Result<GitAnalyticsSnapshot>;
```

Filesystem functions delegate to the same implementations where practical.
`GitAnalyticsSnapshot` contains backward-compatible `HistoryReport`, internal
touch counts by file/identity, and a deterministic whole-project coupling edge
index, all produced by one revision-bounded history walk. History for
worktree/index uses resolved `HEAD`; history for a revision stops at that
resolved commit. Missing Git still degrades gracefully for current-state
project analysis, but comparisons and trend snapshots require requested
commits. Existing `analyze_history` and target coupling reports derive from this
shared implementation rather than adding per-file history walks.

The walk requests raw `%an`/`%ae`, never Git's ambient mailmap-expanded fields.
A deterministic mailmap adapter applies only the selected target/current
`.mailmap` blob; configured `mailmap.file` and dirty worktree mailmap state are
ignored. Comparison metadata and snapshot fingerprints include the selected
mailmap content hash.

The whole-project coupling index retains at most 1,000,000 unique pairs. If a
new pair would cross the ceiling, project coupling becomes unavailable with a
stable `pair_limit_exceeded` reason; history and ownership remain valid. The
existing target-specific coupling command keeps its current behavior.

Comparison settings use the target/current configuration on both sides. A
threshold, duplication setting, or policy edit therefore does not masquerade
as source drift; it changes how both repository states are judged.

## Milestone E: Debt And Risk Diff

Add `src/debt.rs` and command:

```text
leadline debt [--base REV] [--staged | --target REV] [--renames]
              [--path PATH] [--since 30d|90d|365d]
              [--fail-on-regression] [--json | --format agent-json]
```

Defaults match `changed`: base `HEAD~1`, target worktree, path `.`, no rename
detection, and risk window `90d`.

### Function Debt

Each configured threshold is evaluated independently on paired function
identities using the existing path/name/same-name-source-order rule:

| Before | After | Status |
| --- | --- | --- |
| clean or entity absent | violating | `new` |
| violating | violating | `existing` |
| violating | clean or entity absent | `resolved` |
| clean | clean | omitted |

The dimensions are `cognitive`, `cyclomatic`, `crap`, and `max_nesting`.
Entity absence is a known non-violation boundary. A present function whose
metric is missing or non-finite is unknown and cannot be classified as clean or
violating. `summary.unknown` counts each configured dimension/function identity
whose classification is prevented. This deliberately differs from the absolute
`check --crap` gate, where unavailable coverage fails closed. Debt pairs the
complete function sets directly; it does not consume `ChangedReport`, which
omits unchanged fingerprints. A finding
contains path, function identity, dimension, threshold, before/after values,
and status. Mixed changes are separate findings rather than collapsed to one
label.

### Risk Regression

Both sides receive complete analysis, dependency graph, and history reports.
The debt command has no dual-snapshot coverage input, so CRAP is unknown on
both risk sides rather than comparing unlike coverage. Each side uses its own
complete graph denominator and its own revision-bounded history horizon. File
addition can therefore change impact percentages and newer commits can change
history components for otherwise unchanged files; these are genuine complete
state changes, not source-only deltas. Comparison metadata records both scope
sizes, resolved commits, and commit timestamps. Rows pair by normalized path,
honoring explicit rename detection.
Statuses are `added`, `increased`, `decreased`, `removed`, and `unchanged`.
Each row contains before/after score, score delta, component deltas, and raw
dimensions. Positive finite score delta is a regression; no epsilon is hidden.

`DebtReport` carries schema/profile/analyzer/model identifiers, comparison
metadata, debt findings, risk changes, parse diagnostics, summary counts, and
deterministic path/function/dimension ordering.

The command is informational by default and exits `0`. With
`--fail-on-regression`, any new debt finding or positive risk regression exits
`1`. Usage/config errors exit `2`, unavailable comparison data exits `3`, and
input I/O errors exit `4`.

## Ownership

Extend internal Git-history aggregation with contributor touch counts while
keeping current public history output backward compatible. A touch is one
commit by one normalized identity affecting one file, regardless of line
count. Identity follows the existing mailmap-applied lowercase email fallback
to author name.

Add `src/ownership.rs`:

- `contributors`: distinct identity count.
- `concentration_percent`: largest contributor touch count / all touches * 100.
- `bus_factor_50`: minimum contributors whose descending cumulative touches
  reach at least 50%.
- Module ownership sums file touches by identity before calculating the same
  values; it never averages percentages.

With zero touches, `contributors` is `0` while `concentration_percent` and
`bus_factor_50` are `null`; risk and trend ownership are unknown, never zero or
NaN.

Default reports expose aggregate counts/concentration only. `--include-authors`
adds per-file contributor rows, sorted by touches desc then identity.
`--anonymize-authors` assigns deterministic artifact-local opaque labels
(`author-001`, `author-002`, ...) from identity-sorted rows. Labels do not
correlate across repositories and are not described as cryptographic anonymity.
`--exclude-git-identities` explicitly forces aggregate-only output. These
three modes are mutually exclusive. No output ranks contributors across files
or modules.

## Milestone F: Canonical Project And Static Report

Add `src/project.rs` with `PROJECT_SCHEMA_VERSION = 1`. `Project` contains:

```text
meta                    schema/profile/analyzer/generated_from/HEAD facts
summary                 headline counts and bounded KPI values
modules                 recursive directory-prefix aggregates
files                   one normalized row per source file
functions               path + stable function identity + metrics
dependencies            resolved edges and unresolved references
cycles                   strongly connected components
git_activity             optional per-file history facts
temporal_coupling        optional whole-project co-change edges
ownership                optional file/module aggregate ownership
coverage                 optional function/file coverage summaries
mutation                 optional normalized mutants and summaries
test_relationships       optional explicit test-to-code edges
duplication              clone groups and occurrences
architecture_violations  rule violations and optional drift status
risk                     model name, rows, components, and raw facts
snapshots                optional trend points
```

The top-level DTO fixes every parent shape before schema v1 ships:

```rust
pub struct Project {
    pub meta: ProjectMeta,
    pub summary: ProjectSummary,
    pub modules: Vec<ProjectModule>,
    pub files: Vec<ProjectFile>,
    pub functions: Vec<ProjectFunction>,
    pub dependencies: ProjectDependencies, // { edges, unresolved }
    pub cycles: Vec<DependencyCycle>,
    pub git_activity: Option<GitActivitySection>,
    pub temporal_coupling: Option<CouplingSection>, // { available, reason, edges }
    pub ownership: Option<OwnershipSection>,
    pub coverage: Option<CoverageSection>,
    pub mutation: Option<MutationReport>, // { summary, mutants, unresolved }
    pub test_relationships: Option<TestRelationshipReport>, // { relationships, unresolved }
    pub duplication: DuplicationReport, // { complete, reason, groups, diagnostics }
    pub architecture_violations: Vec<ArchitectureViolation>,
    pub risk: ProjectRisk,
    pub snapshots: Option<Vec<TrendPoint>>,
}
```

The focused module specifications define leaf row fields. Parent objects above
are never changed piecemeal under schema version 1.

`null` or an omitted optional section means unknown/unavailable, never zero.
Files sort by path; functions by path/source order; edges by source/target;
groups by stable hash; findings by documented identity; modules by path.

Module hierarchy uses every directory prefix (`src`, `src/payment`, etc.). A
root module `.` contains project totals. Aggregates are calculated in Rust;
the browser never derives scores, ranks, or KPI values.

Add `src/site.rs` and command:

```text
leadline report [PATH] [--output DIR] [--force]
                [--since 30d|90d|365d]
                [--lcov FILE | --jacoco FILE | --coverage FILE]...
                [--pit FILE]... [--stryker FILE]... [--test-map FILE]...
                [--snapshots FILE]
                [--include-authors | --anonymize-authors |
                 --exclude-git-identities]
```

Default output is `code-health-report` when `--output` is omitted. Existing
non-empty output is rejected unless `--force`. Generation stages all files in
a sibling directory. Forced replacement is a same-filesystem backup swap:
destination to fixed sibling backup, stage to destination, rollback backup on
the second rename failing, then remove backup. This is transactional under
ordinary errors but not called crash-atomic; a startup check detects a leftover
backup and refuses to overwrite it until the user resolves it. Failure cleans
the staging directory and preserves or restores the prior report.

Output:

```text
code-health-report/
  index.html
  assets/app.css
  assets/app.js
  assets/project-data.js
  data/project.json
```

`project.json` is canonical pretty JSON. For payloads up to 10 MiB measured as
UTF-8 bytes, `project-data.js` assigns the same serialized value to
`globalThis.__LEADLINE_PROJECT__` and then dispatches a fixed
`leadline:data-ready` event. Repository strings are never interpolated into
HTML. The JavaScript serializer additionally escapes `<`, U+2028, and U+2029
after JSON encoding and is tested with `</script>`, quotes, and backslashes.
Generation streams canonical JSON and JavaScript chunks to staged files and
retains at most the Project model plus one transport chunk; source bytes are
dropped after analysis/graph/token extraction.

### Static UI

The zero-network vanilla UI uses local system fonts and an engineering-dossier
visual language: warm paper, dark ink/navy, safety-orange emphasis, dense but
legible data tables, visible keyboard focus, accessible contrast, responsive
layouts, and reduced-motion support.

Views:

1. Overview KPI strip, distributions, top risk/hotspot lists, data-availability
   notices.
2. Searchable/sortable file and function explorers.
3. Dependency edges, impact summaries, SCC cycles, and coupling relationships.
4. Historical trend charts and module treemap.
5. Mutation/test relationships, duplication groups, and policy violations.

Charts and treemap use deterministic SVG/CSS generated from canonical values.
The UI filters and renders only; it does not recompute metrics or scores.
Large lists render in bounded pages of 100 rows. Search is debounced and scans
in scheduled chunks, returns at most 1,000 matches until narrowed, and never
blocks initial rendering on a full sort. Browser acceptance includes a 100,000
synthetic-row interaction budget of 200 ms per main-thread task on the reference
machine. `data/project.json` always
remains the complete canonical artifact. Above 10 MiB, the JavaScript transport
uses `project-data.js` as a bootstrap containing all scalar/object fields,
partition metadata, and empty arrays for the partitionable `files`,
`functions`, `dependencies.edges`, `dependencies.unresolved`,
`temporal_coupling.edges`, `mutation.mutants`,
`test_relationships.relationships`, `duplication.groups`, and
`architecture_violations` leaf arrays. Parent metadata, null state, and
unpartitioned unresolved/diagnostic arrays remain in the bootstrap. Fixed
5,000-row chunks named
`assets/project-data-<section>-0001.js` call one fixed append function with
`{section: "dependencies.edges", rows}` in canonical section/chunk order; the
append function resolves only this closed dotted-path allowlist. Oversized individual rows
form a one-row chunk. A final `assets/project-data-ready.js` validates expected
counts and dispatches `leadline:data-ready`. `index.html` contains only these
generated fixed-path script tags; no repository data. No runtime fetch is
required, so partitioned reports still work under `file://`.

## Milestone G: Snapshots And Trends

Add `src/snapshots.rs`, `SNAPSHOT_SCHEMA_VERSION = 1`, and command:

```text
leadline snapshot [PATH] --output FILE
                  [report input flags] [--replace]
```

Snapshot capture analyzes the resolved HEAD tree and the `leadline.toml` blob at
HEAD, never dirty worktree source/config. The store is one versioned JSON
document:

```rust
pub struct TrendStore {
    pub schema_version: u32,
    pub points: Vec<TrendPoint>,
}

pub struct TrendPoint {
    pub commit: String,
    pub commit_timestamp: i64,
    pub scope: String,
    pub analyzer_version: String,
    pub metric_profile: String,
    pub risk_model: String,
    pub duplication_profile: String,
    pub mutation_model: String,
    pub config_hash: String,
    pub mailmap_hash: Option<String>,
    pub source_hash: String,
    pub input_fingerprint: String,
    pub files: u64,
    pub functions: u64,
    pub parse_errors: u64,
    pub mean_cognitive: Option<f64>,
    pub max_cognitive: Option<u32>,
    pub coverage_percent: Option<f64>,
    pub hotspot_files: Option<u64>,
    pub max_hotspot_score: Option<u64>,
    pub risk_files_ge_70: u64,
    pub max_risk_score: Option<f64>,
    pub dependency_edges: u64,
    pub cycles: u64,
    pub coupling_edges: Option<u64>,
    pub max_ownership_concentration_percent: Option<f64>,
    pub mutation_score: Option<f64>,
    pub scored_mutants: Option<u64>,
    pub duplication_complete: bool,
    pub duplicate_groups: Option<u64>,
    pub duplicated_lines: Option<u64>,
    pub policy_info: u64,
    pub policy_warning: u64,
    pub policy_error: u64,
}
```

Every hash uses a domain tag plus a versioned canonical JSON structure with
sorted arrays; counts, roles, paths, hashes, and byte payloads are distinct JSON
values, never ambiguous concatenation. `source_hash` covers sorted path/byte
pairs. `config_hash` covers the complete canonical target config.
`input_fingerprint` covers source target, scope, config/mailmap hashes,
adapter/model versions, and sorted `(input role, content hash)` external inputs;
machine paths are excluded.

Rows are keyed by `(commit, scope, config_hash, metric_profile, risk_model,
duplication_profile, mutation_model)`, use commit timestamp, and sort by
timestamp then that key. An identical key and fingerprint is idempotent. A
changed fingerprint for an existing key exits `3` unless `--replace` is
explicit. The UI connects trend lines only across matching profile/model keys
and `config_hash`, and marks analyzer-version changes; it never compares
incompatible points.

Counts use complete untruncated Project sections. Means are arithmetic over all
functions; coverage is function-LOC-weighted over known function coverage;
empty denominators are `null`. `hotspot_files` counts positive hotspot scores
and is `null` without Git; coupling counts edges with at least two co-changes
and is `null` without Git. `risk_files_ge_70` uses the report's named risk
model. Ownership records the maximum known file concentration. Mutation fields
follow the normalized score denominator. `duplicated_lines` is the union of
clone line intervals per file, so overlaps count once. Floating values retain
their computed `f64` value without display rounding in canonical JSON.
If duplication hits a safety ceiling, `duplication_complete` is false and both
duplication counts are `null`, so partial values never enter a trend.

Append rejects an unknown store schema. Profile/model compatibility is carried
per point and enforced when building a trend series, so snapshots remain useful
across analyzer upgrades. Before read/append/write, the command acquires a
fixed sibling lock file with `create_new`; contention or a stale crash lock is
an output error (`4`) and the message names the lock for manual removal. While
holding it, writes follow the baseline sibling-temp/rename pattern, preventing
lost concurrent appends. An RAII guard removes the lock after the store rename
and on every normal error path. Lock cleanup failure exits `4` and names the
lock; only process crashes leave manual-recovery locks. Tests cover successful
and error-path release, active contention, stale-lock refusal, and concurrent
lost-append prevention. No Git HEAD is an incomplete input error (`3`).

`report --snapshots FILE` validates the store and includes trend points in the
Project. Snapshot files never contain source text, author identities, function
details, or complete Project payloads.

## Milestone H: Mutation And Test Relationships

Add `src/mutation.rs` with adapters for:

- PIT `mutations.xml`: mutation status/detected flag, source file, mutated
  class/method, line, mutator, description.
- Stryker mutation-testing-report JSON: schema major `1` is accepted and any
  other or missing `schemaVersion` is rejected; file-keyed mutants provide
  native ID, status, mutator, replacement, and start/end locations.

Each input gets `report_id = BLAKE3(input bytes)` as provenance, not semantic
identity. Stryker identity is `(provider, normalized path, native mutant ID,
complete normalized semantic fields)`. PIT identity is `(provider, normalized
path, mutated class, mutated method, method description, line, mutator, index,
block, description, complete normalized semantic fields)`. Equal complete rows
deduplicate and union their sorted provenance report IDs; distinct mutants
sharing a line/mutator are never collapsed.

Provider statuses map as follows (matching is case-insensitive after removing
`_`/`-`): `KILLED` -> `killed`; `TIMED_OUT`/`TIMEOUT` -> `timed_out`;
`SURVIVED` -> `survived`; `NO_COVERAGE` -> `no_coverage`; `IGNORED`/`PENDING` ->
`ignored`; `NON_VIABLE`/`COMPILE_ERROR` -> `compile_error`; `RUN_ERROR`,
`RUNTIME_ERROR`, and `MEMORY_ERROR` -> `error`; anything else -> `unknown`.
The raw provider status is retained.

Normalized locations are 1-based half-open line/column spans. PIT's line-only
location becomes `[line:1, line+1:1)`. Stryker coordinates are converted from
the report schema's base before storage. Function attribution chooses the
unique innermost containing function span. Equal innermost spans or no
container leave `function_id: null` with a reason; method names never break a
tie.

Paths pass through a strict external-path parser before normalization. It
rejects absolute paths, drive/UNC prefixes, NUL/control characters, and parent
traversal that escapes the logical analysis root. Rejected rows are retained
as unresolved and are never used for filesystem reads or attribution.
Resolved paths normalize relative to the analysis root.
Ambiguous PIT source paths remain unresolved with a reason; suffix matching is
accepted only when exactly one discovered source file matches.

Normalized statuses are `killed`, `timed_out`, `survived`, `no_coverage`,
`ignored`, `compile_error`, `error`, and `unknown`. Cross-provider mutation
score is:

```text
100 * (killed + timed_out)
      / (killed + timed_out + survived + no_coverage)
```

An empty denominator yields `null`. Ignored, compile-error, error, and unknown
rows remain visible but do not enter the score.

Function attribution uses normalized path plus source span. No matching
function leaves `function_id: null`; it never guesses by method name alone.

Add `src/test_relationships.rs`. Input schema:

```json
{
  "schema_version": 1,
  "relationships": [
    {
      "test_path": "tests/payment.test.ts",
      "target_path": "src/payment.ts",
      "target_function": "charge"
    }
  ]
}
```

`target_function` is optional. Both paths use the same strict external-path
parser; invalid, unknown, and ambiguous paths or function names are retained as
unresolved with a stable reason and never read from disk. Exact duplicates
collapse. Rows sort by target path/function then test path.

Add inspection command:

```text
leadline mutation [PATH] [--pit FILE]... [--stryker FILE]...
                  [--test-map FILE]...
                  [--json | --format agent-json]
```

At least one external input is required. Unsupported schemas and unreadable
inputs exit `4`; unresolved individual rows do not fail the whole report.

## Milestone I: Duplication And Architecture Policy

### Duplication

Add configuration:

```toml
[duplication]
min_tokens = 50
min_lines = 5
exclude = ["generated/**"]
```

Unknown keys fail. Minimums must be positive. Analysis excludes always apply;
duplication excludes add to them.

Add `src/duplication.rs` with normalization profile `tokens-v1`. Files whose
Tree-sitter root contains `ERROR` or `MISSING` nodes produce a diagnostic and
are excluded from clone candidates. Leaf tokens omit comments and normalize
identifiers, string/number literals, and template content by token category;
keywords and punctuation remain exact. Languages are separate partitions, so
Java and TypeScript never share a clone group.

The deterministic algorithm is:

1. Hash every `min_tokens` window by `(language, tokens-v1, normalized tokens)`.
2. Compare candidate windows exactly within each hash bucket.
3. Extend equal pairs left and right to their maximal equal token run without
   allowing two occurrences in the same file to overlap.
4. Canonicalize by `(language, complete normalized sequence)`, collect sorted
   occurrences, and discard a group when every occurrence is contained by a
   longer group's corresponding occurrence.
5. Within a group, sort by path/start/end and greedily retain non-overlapping
   occurrences; drop groups with fewer than two remaining occurrences or whose
   occurrence spans fail `min_lines`.

Group ID is BLAKE3 over `(tokens-v1, language, complete normalized sequence)`.
Rows expose language, path, start/end line, token count, and duplicated lines.
Project-level duplicated lines are the union of accepted line intervals per
file, preventing double counting across overlapping groups.

The v1 safety ceiling is 10,000,000 normalized tokens and 10,000,000 exact
candidate comparisons per run. Exceeding either produces an incomplete
duplication report with the observed count and exits `3` for the focused
command; Project keeps the section marked incomplete rather than silently
dropping candidates. The documented upgrade path is a suffix-array/indexed
maximal-repeat implementation, only when real profiles justify it.

Command:

```text
leadline duplication [PATH] [--base REV]
                     [--json | --format agent-json]
```

Without `--base`, status is absent. With it, current settings analyze both
sides. Base paths first pass through the `-M30%` rename map. An occurrence
identity is `(mapped path, group content ID, left eight-token context hash,
right eight-token context hash, ordinal among equal contexts in the file)`;
line numbers are not identity, so unrelated line shifts do not create drift.
Occurrences classify `added`, `existing`, or `removed`. A group is `new` when
it has target occurrences and no base occurrences, `resolved` for the reverse,
and `existing` otherwise; added/removed occurrences remain visible inside an
existing group.

### Architecture Policy

Add configuration:

```toml
[[architecture.rules]]
name = "domain-does-not-import-ui"
source = "src/domain/**"
deny = ["src/ui/**"]
severity = "error"
```

Rules require unique non-empty names, one valid source glob, at least one deny
glob, and severity `info`, `warning`, or `error`. Globs are
analysis-root-relative gitignore-style patterns compiled once with the existing
`ignore` dependency. Negation (`!`), leading `/`, directory-only trailing `/`,
absolute/drive/UNC paths, controls, and escaping `..` are rejected. Patterns
match a file path and its parent directories via one shared
`matched_path_or_any_parents` helper on both snapshots. Rules are evaluated in
declaration order over high-confidence resolved graph edges only. One edge can
violate multiple rules; violation identity is rule name/source/target.

Add `src/policy.rs` and command:

```text
leadline policy [PATH] [--base REV] [--fail-on-violation]
                [--json | --format agent-json]
```

Without `--base`, current violations have no drift status. With it, the current
rule set evaluates both graphs; the same `-M30%` diff rename map rewrites both
base edge endpoints into target paths before violation identities classify
`new`, `existing`, or `resolved`. The threshold is recorded in comparison
metadata. `--fail-on-violation` exits `1` for current error-severity violations;
default is informational.

### Change Risk v2

Keep the existing v1 DTO, builder, ordering, truncation, serialized bytes, and
`leadline risk` behavior untouched. Add a separate v2 DTO/builder for Project
and debt comparisons when at least one architecture rule is configured:

| Component | Weight |
| --- | ---: |
| complexity | 20 |
| CRAP | 15 |
| churn | 20 |
| impact | 20 |
| ownership | 10 |
| policy | 15 |

Policy affects only the violating edge's source endpoint. It is known: no
violation `0`, info `30`, warning `60`, error `100`; the highest violation
affecting a source file wins. V2 raw facts add ownership concentration and
highest policy severity without changing `RiskRaw` v1. The v2 ownership component is
the file's `concentration_percent` (high concentration means high risk), not
v1's contributor-count proxy. Unknown non-policy components renormalize over
known weights. With no architecture rules, Project and debt retain v1. Every
report exposes model, components, raw facts, and exact before/after deltas.

## Agent And MCP Interfaces

Add compact agent JSON for debt, Project summary, mutation, duplication, and
policy. Nulls remain null; truncation is explicit. Agent rows carry enough raw
identity and dimensions to request a focused follow-up without copying the
full Project.

Add read-only MCP tools:

- `analyze_debt`
- `analyze_project`
- `analyze_mutation`
- `analyze_duplication`
- `check_policy`

MCP accepts local input paths but never writes reports or snapshots. The
filesystem-writing `report` and `snapshot` commands remain CLI-only.

## Determinism, Security, And Privacy

- Every emitted array has an explicit stable sort key.
- No current time, machine path, username, hostname, or random identifier is
  present in canonical data.
- `leadline.toml` is limited to 1 MiB and 16 nested TOML levels. Each external
  XML/JSON input is limited to 64 MiB; repeated inputs are limited to 256 MiB
  total and 1,000,000 aggregate mutation/relationship rows. A streaming
  pre-scan limits JSON nesting to 128 and XML nesting to 64. XML rejects
  DTD/DOCTYPE/entity declarations and encodings other than absent/UTF-8 before
  `quick-xml` parsing. Limit/schema violations are input data errors (`4`).
  Inputs never execute commands or load network resources.
- Git revisions reject leading `-`, `:`, controls, and empty values.
- Output paths are lexical destinations supplied by the user; generated file
  names are fixed and cannot traverse directories.
- Site generation never inserts repository content into HTML and does not use
  `innerHTML` for untrusted strings.
- Author identities are opt-in, never globally ranked, and can be anonymized.
- Same inputs, configuration, analyzer version, Git state, and external reports
  produce byte-identical canonical JSON and static assets.

## Error Model

Existing commands retain their documented mappings, including internal error
exit `5`. New commands use:

| Exit | Meaning for E-I commands |
| ---: | --- |
| 0 | success/informational findings |
| 1 | requested CI gate found regression or error violation |
| 2 | usage or configuration error |
| 3 | incomplete analysis, comparison, missing Git object/HEAD, or incompatible snapshot append |
| 4 | external report, source, Git command, or output I/O/data error |
| 5 | unexpected internal invariant failure |

Unresolved imports, mutation paths, and test relationships are report rows,
not process failures. Unsupported report schemas and malformed config fail
explicitly. MCP maps usage/config to JSON-RPC invalid params, incomplete/input
data to stable tool application errors, and invariant failures to internal
error; it does not flatten every failure into invalid params.

## Delivery Order

1. GitAdapter, SnapshotContext/SourceSnapshot, and in-memory
   analysis/graph/history seams.
2. Normalized ownership, mutation/test, duplication, policy, and risk-v2 DTOs
   plus their independent adapters; no Project schema is published yet.
3. Milestone E debt/risk comparison and gates.
4. Whole-project ownership/coupling aggregation and final canonical Project
   schema containing every E-I section from version 1.
5. Milestone G snapshot store and trend model.
6. Milestone F static report with all E-I views and browser acceptance.
7. CLI/agent/MCP integration for Milestones H/I and drift gates.
8. Documentation, benchmarks, cross-milestone review,
   acceptance verification, and roadmap completion.

Each step lands in a reviewable commit. Implementation is one continuous
effort; there are no milestone approval pauses unless a destructive,
security-sensitive, or externally publishing action is required.

## Verification

TDD is mandatory for behavioral code:

- SourceSnapshot fixtures cover worktree, untracked, index, revision, scope,
  excludes, dirty-tree isolation, malformed revisions, and byte stability.
- Debt fixtures cover every transition, mixed dimensions, rename behavior,
  complete-graph risk changes, and gate exits.
- Project tests recompute every aggregate from canonical rows and compare
  byte-identical serializations.
- Site tests open `file://` output in desktop and mobile Playwright sizes,
  exercise navigation/search/pagination, and check malicious strings,
  keyboard focus, overflow, console errors, and reduced motion.
- Snapshot tests cover append, same-HEAD idempotence/replacement protection,
  ordering, unknown-schema rejection, profile/model series separation,
  no-HEAD failure, atomic-write cleanup, and deterministic bytes.
- PIT/Stryker fixtures cover each status, path resolution, schema rejection,
  function attribution, merge/deduplication, and score identity.
- Test-map fixtures cover resolved, unresolved, ambiguous, duplicate, and
  traversal-like paths.
- Duplication fixtures cover exact/type-2 clones, comments/formatting,
  near-misses, overlap suppression, stable IDs, excludes, minimums, drift,
  and cross-language separation.
- Policy fixtures cover globs, multiple matching rules, severity, unresolved
  edges, invalid config, drift, and gate exits.
- MCP schema/dispatch tests cover every new read-only tool.
- Benchmarks cover 1K/10K source snapshots, duplication over repeated and
  unique corpora, and Project/site generation.

Final checks, run sequentially:

```console
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo bench --no-run --locked
```

Independent review must cover correctness, regressions, security, privacy,
determinism, browser behavior, and missing tests. The roadmap is marked fully
implemented only after all checks and acceptance scenarios pass.
