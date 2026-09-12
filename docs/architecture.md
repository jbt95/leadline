# Architecture

`leadline` is one Rust package with small internal modules. A multi-crate workspace would add release and dependency work without improving the initial interface.

```text
paths -> discovery -> source bytes -> TreeSitterAdapter -> metric events
      -> core metric engine -> function results -> coverage/report/diff
```

The parser seam is `ParserBackend::analyze`. The Tree-sitter adapter owns grammar selection, syntax names, and tree walking. The core engine receives only normalized events and source byte spans. It does not import Tree-sitter. A future Oxc adapter can implement the same interface without changing metrics.

Each repository file is read, parsed, and walked by one Rayon worker. The worker returns only function results. Trees and source bytes then drop. Results use sorted paths and source order for stable output.

## Decisions

- Anonymous functions use `<anonymous@LINE:COLUMN>`. A variable assignment supplies the variable name when available.
- Nested functions get independent results. Their nodes do not add metrics to an outer function.
- Generated and vendor directories are detected by fixed path-component names. Explicit file paths remain analyzable.
- Coverage uses known executable lines inside the inclusive function range. Missing lines do not count as uncovered. No known lines means unavailable coverage.
- Partial coverage overlap computes from the known overlapping lines only.
- LCOV and JaCoCo paths are normalized lexically. Source maps are not applied.
- Changed analysis pairs functions by name and same-name source order. A deterministic source fingerprint detects edits that preserve metrics.
- `default-v1` is the only rule profile. Output records its name.

## Risks

Correctness risks are grammar node changes, language-specific switch shapes, ambiguous anonymous functions, logical-expression grouping, nested function ownership, and coverage path mismatch. Tiny cross-language fixtures guard these seams.

Performance risks are parsing, file I/O, repeated subtree walks, operand identity allocation, and JSON aggregation. Analysis parses once, skips nested function bodies in parents, hashes borrowed source spans, and keeps no repository-wide trees. Benchmarks isolate source analysis at 10K, 100K, and 1M lines. End-to-end runs cover discovery, reading, aggregation, and serialization.

## Upstream research and license

Mozilla `rust-code-analysis` separates language checks from metric implementations and supports Tree-sitter grammars. It is MPL-2.0. This project uses that design only as background research. No upstream source was copied. Grammar crates retain their own licenses.

Direct dependency licenses were checked from Cargo metadata:

| Dependency | License |
| --- | --- |
| Tree-sitter and all four grammar packages | MIT |
| `ignore` | Unlicense OR MIT |
| `quick-xml` | MIT |
| `rayon`, `serde`, `serde_json`, `criterion` | MIT OR Apache-2.0 |
