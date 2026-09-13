# Benchmarks

Run the source-analysis benchmark:

```console
cargo bench --bench analyzer
```

The suite measures four distinct costs:

- TypeScript source scaling at 10K, 100K, and 1M physical lines.
- Equivalent Java, JavaScript, TypeScript, and TSX workloads to catch grammar-specific regressions.
- Recovery from a deterministic malformed TypeScript corpus.
- Repository discovery and analysis at a fixed 10K total lines split across 1, 100, and 1,000 files, plus JSON serialization.

Run the Git-history benchmark:

```console
cargo bench --bench history
```

Three groups measure different costs on synthetic repositories staged outside
every timed loop:

- `raw_git_log`: the `git log --relative --no-merges --numstat -z -M30%` walk alone (subprocess, history walk, output generation), so leadline's parsing cost can be bounded by comparing it with the next group.
- `history_analysis`: end-to-end `analyze_history` at 20 and 200 commits, including the HEAD lookup, streaming parse, rename resolution, and aggregation.
- `hotspot_scoring`: the source x history join and ranking over 2,000 pre-analyzed files with no Git access.

Criterion reports wall-clock distributions and line, byte, file, or commit throughput. Inputs and outputs are black-boxed, fixture creation is outside the timed loop, and temporary repository fixtures are removed even if a benchmark panics.

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

## Git history baseline

The 2026-09-13 history baseline used leadline 0.2.0 with the same machine, toolchain, and Criterion version as the source baseline above. Each repository was staged before the timed loop; the raw `git log` case runs the exact production flags so that `raw_git_log` and `history_analysis` bound parsing overhead from both sides.

```console
cargo bench --bench history -- --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
```

| Case | Median time | Throughput |
| --- | ---: | ---: |
| Raw `git log`, 200 commits / 1,000 records | 292.07 ms | 684.78 commits/s |
| History analysis, 20 commits / 100 records | 79.101 ms | 252.84 commits/s |
| History analysis, 200 commits / 1,000 records | 320.44 ms | 624.14 commits/s |
| Hotspot scoring, 2,000 analyzed files | 296.03 µs | 6.756 Mfiles/s |

These numbers are dominated by `git` process startup: history analysis of 200 commits (320 ms) is close to the raw log walk (292 ms), and the remaining difference includes the HEAD lookup subprocess. Parsing and aggregation are a small fraction of run time. Compare same-machine before/after; do not treat the absolute numbers as portable.
