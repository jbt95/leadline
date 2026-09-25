# Architecture

`leadline` is one Rust package with small internal modules. A multi-crate workspace would add release and dependency work without improving the initial interface.

```text
paths -> discovery -> source bytes -> TreeSitterBackend -> metric events
      -> core metric engine -> function results -> coverage/report/diff

git repository -> history walk -> commit stream (resolved paths, deltas)
                                      │
                     ┌────────────────┴────────────────┐
                     ▼                                 ▼
             file history facts                 co-change index
                     │                                 │
   function results ─┴─> hotspots -> ranked    coupling -> related files

source files -> graph -> dependency edges -> impact -> dependents
                                                 │
   function results + history + graph/impact ────┴─> risk -> ranked
```

The parser seam is `ParserBackend::analyze`, currently implemented by `TreeSitterBackend`. It selects a Tree-sitter grammar from the source extension, walks the syntax tree, and returns `FileAnalysis`. The bundle path derives dependency and token products from that same tree; the core metric engine consumes normalized events and source byte spans, not Tree-sitter nodes.

Git history intelligence is a second, independent adapter. `history` never imports the parser, the metric engine, or coverage; it turns one streamed `git log` walk into per-file facts keyed by normalized scope-relative paths. `coupling` indexes co-changes from the same walk (the `history::walk_commits` seam) and answers "what changes with this file?". `hotspots` is a join layer: it imports both `core` results and `history` facts and owns the path join and ranking. `graph` extracts the static file-level dependency graph (relative JS/TS imports, exact Java type imports, file-relative Rust `mod` declarations, quoted C/C++ includes, Python relative imports, and local Zig `@import` strings, resolved against discovery) and `impact` answers "what depends on this file?" by reverse-BFS over those edges; `risk` is a join layer over `core`, `history`, `ownership`, `graph`/`impact`, and `policy` facts, scoring each file with the versioned `change-risk` model and weight-renormalized components. Ownership and policy layers follow the same pattern. See `analytics-roadmap.md` for the target architecture and normalized analytics model.

Zig dependency edges are deliberately local-only: string `@import` arguments
resolve only when they name an exact relative discovered `.zig` file. Bare
package imports do not produce edges, and the graph does not model a Zig
package graph or execute `build.zig`.

The HTTP transport is a third seam. `http` owns binding (with a free-port fallback), request reading, response writing, deadlines, the connection ceiling, and the `Host`/`Origin` guards; `mcp` and `stats` route on top of it, and MCP's observable behavior is unchanged by the move. `stats` owns routing, the analyze-once snapshot, and option parsing: it never parses source code, it calls the same project builder the `project` command calls, keeps the serialized bytes, and serves the embedded page, its assets, and the canonical JSON.

Each repository file is read, parsed, and walked by one Rayon worker. The worker returns only function results. Trees and source bytes then drop. Results use sorted paths and source order for stable output.

## Decisions

- Anonymous functions use `<anonymous@LINE:COLUMN>`. A variable assignment supplies the variable name when available.
- Nested functions get independent results. Their nodes do not add metrics to an outer function.
- Generated and vendor directories are detected by fixed path-component names. Explicit file paths remain analyzable.
- Coverage uses known executable lines inside the inclusive function range. Missing lines do not count as uncovered. No known lines means unavailable coverage.
- Partial coverage overlap computes from the known overlapping lines only.
- LCOV and JaCoCo paths are normalized lexically. Source maps are not applied.
- Changed analysis pairs functions by name and same-name source order. A deterministic source fingerprint detects edits that preserve metrics.
- Git history uses two subprocesses per run: one HEAD lookup and one `git log --relative --no-merges --numstat -z -M30%` stream parsed record by record. `--relative` scopes and rekeys paths in Git, so subdirectory analyses never walk the whole repository.
- Recency windows are relative to the HEAD commit time, never the wall clock, so the same snapshot produces the same report. Merge commits are excluded. Renames resolve newest to oldest; the 30% threshold keeps small moves but can pair unrelated boilerplate-heavy files.
- `hotspots` ranks by `max cognitive x changes in window` (`complexity-x-churn`) and keeps every dimension in the JSON. A missing repository degrades to complexity ranking with `git_available: false`; it is never an error.
- `coupling` indexes co-changes from the same walk and reports directional and Jaccard values. Commits wider than 50 files contribute to file totals but never to pairs; co-change is process evidence, not dependency.
- `graph` resolves only relative JS/TS imports, exact Java type imports, file-relative Rust `mod` declarations, quoted C/C++ includes, Python relative imports, and local Zig `@import` strings against discovered files; bare package imports are ignored, ambiguity stays unresolved, and every edge carries `confidence: "high"`. Fan-in counts direct importers, fan-out counts resolved targets, and cycles are strongly connected components of at least two files (a self-import is an edge, not a cycle).
- `unused` adds Zig convention entries only for `build.zig` at any directory
  depth and exact `src/main.zig`, `src/lib.zig`, and `src/root.zig` paths at
  the analysis root or under an exact `/src/...` suffix. Arbitrary `.zig` files
  are not convention entries; they require a resolved local `@import` edge.
- `impact` follows reverse graph edges from the target with shortest-distance BFS; `blast_radius` counts unique dependents, `blast_radius_percent` is on a 0-100 scale, and `--top` truncates only the shown list. Resolved imports are static evidence, incomplete wherever aliases, package graphs, reflection, or runtime-built specifiers decide the real target.
- `risk` ranks files by the `change-risk` score: weight-renormalized components for complexity (20), CRAP (15), churn (20), impact (20), ownership concentration (10), and policy (15). Unknown components stay `null` and renormalize instead of zeroing; rows sort by score desc, path asc, and the build returns the full ranking (`--limit` caps terminal and agent-JSON rows only). The command is informational (exit `0`); score regressions are compared by `debt --fail-on-regression`.
- `default` is the only rule profile. Output records its name.

## Risks

Correctness risks are grammar node changes, language-specific switch shapes, ambiguous anonymous functions, logical-expression grouping, nested function ownership, and coverage path mismatch. Tiny cross-language fixtures guard these seams. Git-history risks are rename-heuristic thresholds, clock-skewed commit dates, contributor-identity collisions (shared emails, bots, mailmap drift), and the scope boundary for cross-directory renames. Synthetic repositories with fixed commit dates guard those seams.

Performance risks are parsing, file I/O, repeated subtree walks, operand identity allocation, and JSON aggregation. Analysis parses once, skips nested function bodies in parents, hashes borrowed source spans, and keeps no repository-wide trees. Benchmarks isolate source analysis at 10K, 100K, and 1M lines. End-to-end runs cover discovery, reading, aggregation, and serialization. Git history adds subprocess startup and stream parsing as separate bench groups; on the reference machine, history analysis of 200 commits (320 ms) is close to a raw `git log` walk (292 ms), so process startup, not parsing, dominates small runs.

## Upstream research and license

Mozilla `rust-code-analysis` separates language checks from metric implementations and supports Tree-sitter grammars. It is MPL-2.0. This project uses that design only as background research. No upstream source was copied. Grammar crates retain their own licenses.

Direct dependency licenses were checked from Cargo metadata:

| Dependency | License |
| --- | --- |
| Tree-sitter and all nine grammar packages | MIT |
| `ignore` | Unlicense OR MIT |
| `quick-xml` | MIT |
| `rayon`, `serde`, `serde_json`, `criterion` | MIT OR Apache-2.0 |
