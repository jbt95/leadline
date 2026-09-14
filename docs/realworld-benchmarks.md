# Real-world benchmarks

Local-only suite proving leadline on current, large open-source code.
Fixtures float on latest `main`/`master` (shallow clones); nothing here
runs in CI and `cargo test` never touches the network.

## Run

```console
bash scripts/realworld.sh          # clone/update into target/realworld/
cargo bench --bench realworld      # smoke + throughput + floors
```

`LEADLINE_REALWORLD_DIR` overrides the fixture directory. A missing
fixture dir skips gracefully with a pointer to the fetch script.

## What runs per fixture

- `analyze_path` over the fixture subpath with smoke asserts: success,
  `files > 0`, `functions > 0`, byte-identical rerun (determinism).
- Join smoke on `tanstack-query` only: `risk` and `project` builds with
  non-empty risk rows (full joins on spring-boot stay manual).
- Criterion throughput (`realworld/<name>`) plus a post-group floor check
  printing `measured vs floor` in files/sec.

## Baseline (Apple M2 Pro, 32 GiB, macOS 26.6.2, Rust 1.90.0, 2026-09-14, floating main)

| Fixture | Files | Functions | Mean analyze | Throughput | Floor |
| --- | --- | --- | --- | --- | --- |
| tanstack-query (`packages`) | 692 | 20,748 | 146 ms | ~4,800/s | 2,400 |
| nest (`packages`) | 902 | 9,706 | 79 ms | ~11,500/s | 5,400 |
| react (`packages`) | 1,838 | 39,299 | 484 ms | ~3,800/s | 1,900 |
| spring-boot (root) | 8,392 | 65,832 | 702 ms | ~12,000/s | 4,600 |

Your numbers will differ; floors carry ~50% headroom so they catch real
regressions (algorithmic blowups), not noise.

## Re-baseline policy

Floors live in `benches/realworld.toml`. When a floor trips because
upstream grew or refactored (not because leadline regressed), update the
floor to the new measured value minus ~50% headroom — never tune code to
a number. A trip caused by a leadline change is a regression: fix the
code, not the floor.
