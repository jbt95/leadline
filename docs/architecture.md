# Architecture

`leadline` is one Rust package with small internal modules. A multi-crate workspace would add release and dependency work without improving the initial interface.

```text
paths -> discovery -> source bytes -> TreeSitterAdapter -> metric events
      -> core metric engine -> function results -> coverage/report/diff

git repository -> history walk -> commit stream (resolved paths, deltas)
                                      │
                     ┌────────────────┴────────────────┐
                     ▼                                 ▼
             file history facts                 co-change index
                     │                                 │
   function results ─┴─> hotspots -> ranked    coupling -> related files
```

The parser seam is `ParserBackend::analyze`. The Tree-sitter adapter owns grammar selection, syntax names, and tree walking. The core engine receives only normalized events and source byte spans. It does not import Tree-sitter. A future Oxc adapter can implement the same interface without changing metrics.

Git history intelligence is a second, independent adapter. `history` never imports the parser, the metric engine, or coverage; it turns one streamed `git log` walk into per-file facts keyed by normalized scope-relative paths. `coupling` indexes co-changes from the same walk (the `history::walk_commits` seam) and answers "what changes with this file?". `hotspots` is a join layer: it imports both `core` results and `history` facts and owns the path join and ranking. Later dependency, ownership, and risk layers follow the same pattern. See `analytics-roadmap.md` for the target architecture and normalized analytics model.

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
- `hotspots` ranks by `max cognitive x changes in window` (`complexity-x-churn-v1`) and keeps every dimension in the JSON. A missing repository degrades to complexity ranking with `git_available: false`; it is never an error.
- `coupling` indexes co-changes from the same walk and reports directional and Jaccard values. Commits wider than 50 files contribute to file totals but never to pairs; co-change is process evidence, not dependency.
- `default-v1` is the only rule profile. Output records its name.

## Risks

Correctness risks are grammar node changes, language-specific switch shapes, ambiguous anonymous functions, logical-expression grouping, nested function ownership, and coverage path mismatch. Tiny cross-language fixtures guard these seams. Git-history risks are rename-heuristic thresholds, clock-skewed commit dates, contributor-identity collisions (shared emails, bots, mailmap drift), and the scope boundary for cross-directory renames. Synthetic repositories with fixed commit dates guard those seams.

Performance risks are parsing, file I/O, repeated subtree walks, operand identity allocation, and JSON aggregation. Analysis parses once, skips nested function bodies in parents, hashes borrowed source spans, and keeps no repository-wide trees. Benchmarks isolate source analysis at 10K, 100K, and 1M lines. End-to-end runs cover discovery, reading, aggregation, and serialization. Git history adds subprocess startup and stream parsing as separate bench groups; on the reference machine, history analysis of 200 commits (320 ms) is close to a raw `git log` walk (292 ms), so process startup, not parsing, dominates small runs.

## Upstream research and license

Mozilla `rust-code-analysis` separates language checks from metric implementations and supports Tree-sitter grammars. It is MPL-2.0. This project uses that design only as background research. No upstream source was copied. Grammar crates retain their own licenses.

Direct dependency licenses were checked from Cargo metadata:

| Dependency | License |
| --- | --- |
| Tree-sitter and all four grammar packages | MIT |
| `ignore` | Unlicense OR MIT |
| `quick-xml` | MIT |
| `rayon`, `serde`, `serde_json`, `criterion` | MIT OR Apache-2.0 |
