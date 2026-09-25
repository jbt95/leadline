# Benchmarks

Run the source-analysis benchmark:

```console
cargo bench --bench analyzer
```

The suite measures four distinct costs:

- TypeScript source scaling at 10K, 100K, and 1M physical lines.
- Equivalent C, C++, Java, JavaScript, Python, Rust, TypeScript, TSX, and Zig workloads to catch grammar-specific regressions (Go is not in this group).
- Recovery from a deterministic malformed TypeScript corpus.
- Repository discovery and analysis at a fixed 10K total lines split across 1, 100, and 1,000 files, plus JSON serialization.

Run the Git-history benchmark:

```console
cargo bench --bench history
```

Four groups measure different costs on synthetic repositories staged outside
every timed loop:

- `raw_git_log`: the `git log --relative --no-merges --numstat -z -M30%` walk alone (subprocess, history walk, output generation), so leadline's parsing cost can be bounded by comparing it with the next group.
- `history_analysis`: end-to-end `analyze_history` at 20 and 200 commits, including the HEAD lookup, streaming parse, rename resolution, and aggregation.
- `hotspot_scoring`: the source x history join and ranking over 2,000 pre-analyzed files with no Git access.
- `coupling_analysis`: co-change indexing for one target over the 200-commit repository, sharing the same staged walk and rename resolution.

Criterion reports wall-clock distributions and line, byte, file, or commit throughput. Inputs and outputs are black-boxed, fixture creation is outside the timed loop, and temporary repository fixtures are removed even if a benchmark panics.

Run the in-memory duplication benchmark:

```console
cargo bench --bench duplication
```

`duplication_detection` measures clone detection over 500 and 2,000
in-memory files with `min_tokens = 12` and `min_lines = 5`, for a unique
corpus and for a corpus where every second file repeats one shared body, so
both the indexing and the exact-comparison paths are covered.

Run the snapshot-pipeline benchmarks:

```console
cargo bench --bench source_snapshot
```

`snapshot_analysis` measures `analyze_sources` over 1,000 and 10,000
in-memory entries, and `snapshot_dependencies` measures
`analyze_dependencies_from_sources` over the same entries — the two stages
a source snapshot runs without touching Git or the filesystem.

Run an end-to-end repository measurement with platform tools:

```console
cargo build --release
/usr/bin/time -l ./target/release/leadline analyze PATH --json > /dev/null
```

On macOS, `time -l` reports elapsed time and peak resident memory. Record these details with results:

```console
rustc -Vv
uname -a
sysctl -n machdep.cpu.brand_string
```

Do not compare results from different machines as one series. The source case covers parsing, AST traversal, metric calculation, and function aggregation. The repository case adds discovery, file reading, parallel work, and result sorting. The serialization case isolates JSON. The platform command measures total runtime and peak memory. No benchmark result belongs in the repository without its machine details.

For a same-machine before/after comparison, save a named baseline and compare against it:

```console
cargo bench --bench analyzer -- --save-baseline before
# Make the change.
cargo bench --bench analyzer -- --baseline before
```

Treat Criterion's statistical comparison as the regression signal. Do not add fixed wall-clock thresholds to shared CI runners.

## Baseline

The 2026-09-12 baseline used leadline 0.1.0, Criterion 0.8.2, and Rust 1.98.1. The machine was a Mac14,9 with an Apple M2 Pro, 10 CPU cores, 32 GiB memory, and macOS 26.6.2.

The command used 10 samples, a 100 ms warm-up, and a 200 ms requested measurement time. Criterion increased measurement time when necessary.

| Case | Median time | Throughput |
| --- | ---: | ---: |
| TypeScript, 10K generated lines | 27.132 ms | 368.57 KLOC/s |
| TypeScript, 100K generated lines | 282.96 ms | 353.41 KLOC/s |
| TypeScript, 1M generated lines | 2.9203 s | 342.43 KLOC/s |
| Repository, 100 files and 10K lines | 6.7269 ms | 14,866 files/s |
| JSON, 100-file result | 0.75825 ms | 1.06 GiB/s |

Run the same command on the same machine before comparing a later result:

```console
cargo bench --bench analyzer -- --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
```

## Real-world baseline

The 2026-09-13 real-world baseline used leadline 0.3.2, Criterion 0.8.2,
and Rust 1.90.0 on the same Apple M2 Pro machine as the source baseline
above. Fixtures are shallow clones of the floating upstream branches,
fetched with `scripts/realworld.sh`; SHAs at measurement time: TanStack Query
`b8fdc28`, Nest `a3a31b9`, React `ccea5fd`, Spring Boot `58b8ce6c`.
Criterion defaults (not the 10-sample synthetic config); times are group
means for `analyze_path` over the fixture subpath.

```console
bash scripts/realworld.sh
cargo bench --bench realworld
```

| Fixture (subpath) | Files | Functions | Mean time | Throughput |
| --- | ---: | ---: | ---: | ---: |
| TanStack Query (`packages`) | 692 | 20,748 | 140.81 ms | 4,914 files/s |
| Nest (`packages`) | 902 | 9,706 | 83.18 ms | 10,843 files/s |
| React (`packages`) | 1,838 | 39,299 | 480.33 ms | 3,827 files/s |
| Spring Boot (root) | 8,392 | 65,832 | 907.33 ms | 9,248 files/s |

The enforced floors in `benches/realworld.toml` sit ~50% below these
(2,400 / 5,400 / 1,900 / 4,600 files/s) so they catch real regressions,
not noise or upstream growth. Floating upstream branches mean later runs
measure different code: compare same-machine before/after, re-baseline the
manifest when upstream moves (per `docs/realworld-benchmarks.md`), and
never treat these absolutes as portable.

## Git history baseline

The 2026-09-13 history baseline used leadline 0.2.0 with the same machine, toolchain, and Criterion version as the source baseline above. Each repository was staged before the timed loop; the raw `git log` case runs the production walk flags so that `raw_git_log` and `history_analysis` bound parsing overhead from both sides.

```console
cargo bench --bench history -- --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
```

| Case | Median time | Throughput |
| --- | ---: | ---: |
| Raw `git log`, 200 commits / 1,000 records | 307.20 ms | 651.05 commits/s |
| History analysis, 20 commits / 100 records | 81.006 ms | 246.90 commits/s |
| History analysis, 200 commits / 1,000 records | 324.32 ms | 616.67 commits/s |
| Hotspot scoring, 2,000 analyzed files | 293.27 µs | 6.820 Mfiles/s |
| Coupling analysis, 200 commits / 1,000 records | 366.77 ms | 545.30 commits/s |

These numbers are dominated by `git` process startup: history analysis of 200 commits (320 ms) is close to the raw log walk (292 ms), and the remaining difference includes the HEAD lookup subprocess. Parsing and aggregation are a small fraction of run time. Compare same-machine before/after; do not treat the absolute numbers as portable.
