# Performance methodology

## Criterion benches

```console
cargo bench --bench analyzer
```

Deterministic generated TypeScript inputs at three sizes isolate parsing, AST traversal, metric calculation, and function aggregation:

- 10K lines, 100K lines, 1M lines (`typescript_analysis`, 10 samples).
- Repository case: 100 files / 10K lines (`repository_analysis`), covering discovery, file reading, Rayon parallelism, and result sorting. Reported in files/s.
- Serialization case: one 100-file result to JSON (`result_serialization`). Reported in bytes/s.

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
