# Performance methodology

## Criterion benches

```console
cargo bench --bench analyzer
```

Deterministic generated TypeScript inputs at three sizes isolate parsing, AST traversal, metric calculation, and function aggregation:

- 10K lines, 100K lines, 1M lines (`typescript_analysis`, 10 samples).
- Repository case: 100 files / 10K lines (`repository_analysis`), covering discovery, file reading, Rayon parallelism, and result sorting. Reported in files/s.
- Serialization case: one 100-file result to JSON (`result_serialization`). Reported in bytes/s.

```console
cargo bench --bench history
```

- `raw_git_log`: the production `git log --relative --no-merges --numstat -z -M30%` walk alone, so parsing overhead is bounded by comparing it with `history_analysis`.
- `history_analysis`: end-to-end history ingestion at 20 and 200 commits (HEAD lookup, streaming parse, rename resolution, aggregation). Reported in commits/s.
- `hotspot_scoring`: source x history join and ranking over 2,000 pre-analyzed files, no Git access. Reported in files/s.
- `coupling_analysis`: co-change indexing for one target over the 200-commit repository. Reported in commits/s.

Synthetic repositories for the history benches are staged once, outside the timed loops, with deterministic content.

Same-machine comparison only:

```console
cargo bench --bench analyzer -- --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
```

## Memory (RSS)

End-to-end time plus peak resident memory via platform tools:

```console
cargo build --release
/usr/bin/time -l ./target/release/leadline analyze PATH --json > /dev/null
```

On macOS, `time -l` reports elapsed time and maximum resident set size.

## Recording results

Every result must record machine and toolchain details:

```console
rustc -Vv
uname -a
sysctl -n machdep.cpu.brand_string
```

Never compare runs across machines as one series. Never commit a benchmark result without its machine details.

## Baseline

2026-09-12, leadline 0.1.0, Criterion 0.8.2, Rust 1.98.1, Mac14,9, Apple M2 Pro (10 cores), 32 GiB, macOS 26.6.2:

| Case | Median | Throughput |
| --- | ---: | ---: |
| 10K lines | 27.132 ms | 368.57 KLOC/s |
| 100K lines | 282.96 ms | 353.41 KLOC/s |
| 1M lines | 2.9203 s | 342.43 KLOC/s |
| Repo 100 files / 10K lines | 6.7269 ms | 14,866 files/s |
| JSON 100-file result | 0.75825 ms | 1.06 GiB/s |

2026-09-13, leadline 0.2.0, same machine and toolchain:

| Case | Median | Throughput |
| --- | ---: | ---: |
| Raw `git log`, 200 commits / 1,000 records | 307.20 ms | 651.05 commits/s |
| History analysis, 20 commits / 100 records | 81.006 ms | 246.90 commits/s |
| History analysis, 200 commits / 1,000 records | 324.32 ms | 616.67 commits/s |
| Hotspot scoring, 2,000 files | 293.27 µs | 6.820 Mfiles/s |
| Coupling analysis, 200 commits / 1,000 records | 366.77 ms | 545.30 commits/s |

2026-09-16, leadline 0.6.1, Mac14,9, Apple M2 Pro (10 cores), 32 GiB, macOS 26.6.2, same toolchain as the baselines above:

Repository: 120 synthetic TypeScript files x 12 functions each (`cargo bench --offline --bench index`).

| Case | Median | 95% CI |
| --- | ---: | ---: |
| `index_refresh/cold_analyze` | 33.146 ms | 33.052–33.246 ms |
| `index_refresh/warm_refresh` | 605.20 µs | 576.90–642.45 µs |

Warm refresh is ~55x faster than cold analysis. Warm refresh of a repository this size must stay under 100 ms: a regression past that budget is a bug, not a trade-off.

2026-09-18, leadline 0.11.0 → 0.12.0, Apple M2 Pro, 32 GiB, macOS 26.6.2 (Darwin 25.6.0 arm64), Rust 1.90.0, Criterion 0.8.2, same machine before and after (`cargo bench --offline --bench duplication`, `sample_size(10)`):

| Case | Before (median) | After (median) | Delta |
| --- | ---: | ---: | ---: |
| `duplication_detection/unique/500` | 1.0988 s | 174.10 ms | −84% |
| `duplication_detection/unique/2000` | 9.4678 s | 668.16 ms | −93% |
| `duplication_detection/repeated/500` | 1.0486 s | 236.01 ms | −77% |
| `duplication_detection/repeated/2000` | 6.7576 s | 910.26 ms | −87% |

End-to-end self-debt (`./target/release/leadline debt --base HEAD~1 --json`, `/usr/bin/time -l`, median of 3, same machine and binary flags): 1.37 s / ~30.5 MB RSS before → 1.13 s / ~30 MB RSS after (−17%, RSS unchanged). The after column includes the rolling-hash correctness fix, which intentionally reports previously invisible clone groups (probe corpus: 8 → 17 groups at identical `duplicated_lines`).
