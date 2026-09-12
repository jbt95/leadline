# Benchmarks

Run the source-analysis benchmark:

```console
cargo bench --bench analyzer
```

It generates deterministic TypeScript inputs near 10K, 100K, and 1M physical lines. Criterion reports wall-clock distributions and line throughput. A 100-file, 10K-line repository case reports files per second. A separate case measures JSON serialization throughput.

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
