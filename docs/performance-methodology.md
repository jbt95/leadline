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

- `raw_git_log`: the `git log --relative --no-merges --numstat -z -M30%` walk alone, so parsing overhead is bounded by comparing it with `history_analysis`.
- `history_analysis`: end-to-end history ingestion at 20 and 200 commits (HEAD lookup, streaming parse, rename resolution, aggregation). Reported in commits/s.
- `hotspot_scoring`: source x history join and ranking over 2,000 pre-analyzed files, no Git access. Reported in files/s.
- `coupling_analysis`: co-change indexing for one target over the 200-commit repository. Reported in commits/s.

Synthetic repositories for the history benches are staged once, outside the timed loops, with deterministic content.

Same-machine comparison only:

```console
cargo bench --bench analyzer -- --warm-up-time 0.1 --measurement-time 0.2 --sample-size 10
```

## Memory (RSS)

Criterion reports CPU/wall time only, never peak RSS. Every RSS claim
must come from `/usr/bin/time -l` on a release binary on the same
machine as its paired Criterion run. The existing Criterion CPU benches
stay unchanged: the current dependency result (~39 ms for 10,000
entries) is already below the stored 105 ms baseline, so no graph
rewrite or benchmark-only abstraction is warranted.

End-to-end time plus peak resident memory via platform tools:

```console
cargo build --release
/usr/bin/time -l ./target/release/leadline analyze PATH --json > /dev/null
```

On macOS, `time -l` reports elapsed time and maximum resident set size.

### Large-file capacity procedure

Generate function-count fixtures with the same Python loop, substituting
only the function count (10k/50k/100k/200k). Keep every generated
fixture under `${TMPDIR:-/tmp}/leadline-perf-results`; never commit a large
corpus:

```console
results="${TMPDIR:-/tmp}/leadline-perf-results"
mkdir -p "$results"
/usr/bin/python3 - "$results" <<'PY'
import sys
from pathlib import Path

results = Path(sys.argv[1])
results.mkdir(parents=True, exist_ok=True)
for count in (10_000, 50_000, 100_000, 200_000):
    with (results / f"functions-{count}.ts").open("w") as output:
        for index in range(count):
            output.write(f"function f{index}(x: number): number {{\n")
            output.write("  if (x > 1) {\n    return x + 1;\n  }\n")
            output.write("  return x;\n}\n")
PY
```

Source-byte control: a single 17.7 MB one-line TypeScript comment proves
scaling is driven by function cardinality, not source bytes:

```console
results="${TMPDIR:-/tmp}/leadline-perf-results"
/usr/bin/python3 -c "from pathlib import Path; Path('$results/comment-17mb.ts').write_text('// ' + 'x' * (17_700_000 - 3) + '\n')"
```

Measure each fixture with and without JSON serialization, recording wall
time, user time, peak RSS, and exit code:

```console
cargo build --offline --release
results="${TMPDIR:-/tmp}/leadline-perf-results"
/usr/bin/time -l ./target/release/leadline analyze "$results/functions-200000.ts" > /dev/null
/usr/bin/time -l ./target/release/leadline analyze "$results/functions-200000.ts" --json > /dev/null
```

Reference shape (2026-09-18, Mac14,9/M2 Pro, same-session release
binary): the 17.7 MB comment takes ~0.11 s at ~22 MB RSS, while the
same-size 200,000-function file takes ~3.98 s at ~1,078 MB RSS. RSS
scales approximately linearly in function count (10k: ~60 MB;
50k: ~273 MB; 100k: ~542 MB; 200k: ~1,078 MB).

### Clone-ceiling procedure

On identical clone-heavy files the existing 10M comparison ceiling is
reached at 500 files: wall time then stays near 3.3 s while token/file
storage keeps growing (100 files: complete; 500–2,000 files: ceiling
report with rising RSS). Measure with the release binary and keep the
JSON output for the identity comparison below:

```console
cargo build --offline --release
results="${TMPDIR:-/tmp}/leadline-perf-results"
mkdir -p "$results"
/usr/bin/time -l ./target/release/leadline duplication CLONE_CORPUS --json > "$results/dup-clone.json"
```

Record wall time, user time, peak RSS, exit code, and whether the result
is `complete` or a comparison-ceiling report. A run that terminates
abnormally without producing JSON (as seen for a single 200,000-function
file at ~4.7 GB RSS) is a capacity finding, not a timed result: report
the RSS high-water mark and the absence of output.

Output identity: a performance change must leave reports byte-identical.
Compare before/after JSON bytes and exit codes on the same corpora:

```console
cmp "$results/dup-before.json" "$results/dup-after.json"
```

Any byte difference or exit-code change disqualifies the run as a pure
performance comparison.

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

2026-09-19, leadline 0.14.0, Apple M2 Pro, 32 GiB, macOS 26.6.0 (Darwin 25.6.0 arm64), Rust 1.90.0, Criterion 0.8.2, same machine and the same frozen baseline (`--baseline before-duplication`, `--warm-up-time 0.1 --measurement-time 0.2 --sample-size 10`). The change interns duplication token text once instead of per occurrence; every paired JSON stayed byte-identical with identical exit codes.

| Case | Change [low, median, high] |
| --- | --- |
| `duplication_detection/unique/500` | [−1.13%, −0.65%, −0.20%] |
| `duplication_detection/unique/2000` | [+0.22%, +0.55%, +0.91%] |
| `duplication_detection/repeated/500` | [+0.48%, +1.35%, +2.35%] |
| `duplication_detection/repeated/2000` | [−1.28%, −1.08%, −0.89%] |

Peak RSS (`/usr/bin/time -l`, same session): `duplication dup-scale/500` 18.19 → 16.28 MiB (−10.5%), `dup-scale/100` 9.08 → 8.38 MiB (−7.7%), `bench-repo-diverse/src` 28.41 → 22.42 MiB (−21.1%), and self-duplication 22.64 → 21.50 MiB (−5.0%). The single-file 200,000-function shape still fails: both the before and after binaries are killed at a 3 GiB watchdog inside ~3.6 s with no JSON, which is the documented capacity limit rather than a regression.
