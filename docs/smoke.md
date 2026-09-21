# Smoke tests

End-to-end CLI proof on large third-party repos, disjoint from the
benchmark fixtures (`babel`, `elasticsearch`, `timescaledb` instead of
TanStack Query, Nest, React, Spring Boot). Local-only: never runs in CI
and `cargo test` never touches the network.

## Run

```console
bash scripts/smoke.sh                   # everything (network + GBs of full clones)
SMOKE_ONLY=self bash scripts/smoke.sh   # offline self-check, no clones
SMOKE_ONLY=sql bash scripts/smoke.sh    # one section (after fetching)
```

`LEADLINE_SMOKE_DIR` overrides the fixture directory
(default `<repo>/target/smoke/`), `LEADLINE_BIN` pins the binary
(default `target/debug/leadline`, built with `cargo build --offline`
when missing). Full (not shallow) clones: the history surfaces degrade
to `git_available: false` without git history.

## What runs per section

| Section | Commands |
| --- | --- |
| `fetch` | clone or update the three full fixtures (network); every repo-backed section needs them (`plan`, `mcp`, and `self` do not) |
| `scale` | `analyze` x2 with byte-identical determinism diff on babel + elasticsearch |
| `gates` | `check` thresholds (exit 0 clean, 1 violations) |
| `history` | `hotspots`, `coupling`, `impact` (first file with dependents), `debt`, `baseline` + `check --regressions`, `duplication` (exits 0/1/3; ceiling is 3) |
| `joins` | `project`, `snapshot`, `policy` on one package |
| `sql` | `sql` on timescaledb migrations with `--migration-root`; empty-dir `analyze` exits 3 |
| `plan` | `sql-plan` violation (exit 1) and clean (exit 0) controls |
| `coverage` | `test-targets` when `babel/coverage/lcov.info` exists, else skip with the generation command |
| `scanners` | `osv-scanner` → `vulnerabilities` when installed, else skip; `security` needs `SMOKE_SECURITY_SARIF` (eslint `@microsoft/eslint-formatter-sarif` or `semgrep --sarif`) |
| `mcp` | stdio ping plus HTTP `--port 0`: `/health` and `tools/list` against the OS-assigned port |
| `self` | offline analyze/check of the repo’s own TypeScript (`integrations/agent-adapter-ts`) |

## Notes

- Fixtures float on latest `main`; a section that trips because upstream
  restructured (moved files, renamed branches) needs a script tweak, not
  a code fix — keep paths discovered (`git ls-files`, `find`), not hardcoded.
- Java coverage (elasticsearch JaCoCo) and mutation reports (PIT/Stryker)
  stay manual: generating them costs hours, while the CRAP-with-coverage
  and adapter paths are covered by `coverage` and the unit suite.
