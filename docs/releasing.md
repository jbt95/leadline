# Releasing leadline

Releases are reproducible and traceable: every binary maps to exactly one tag commit.

## Automatic releases

Merging to `main` runs the `Auto release` workflow, which cuts the release
from the conventional commits since the last tag:

| Commit | Bump |
| --- | --- |
| `feat` | minor |
| `fix`, `perf`, `revert` | patch |
| `!` after the type or scope, or `BREAKING CHANGE:` in the body | major |
| anything else (`docs`, `chore`, `ci`, `test`, `refactor`, merges) | none |

`scripts/release.sh` does the work: it bumps the version in `Cargo.toml`,
`Cargo.lock`, `package.json`, `integrations/agent-adapter-ts/package.json`,
`integrations/agent-adapter-ts/package-lock.json`,
`integrations/typesafe-triage/package.json`,
`integrations/typesafe-triage/package-lock.json`,
`integrations/claude-code/.claude-plugin/plugin.json`,
`integrations/pi/package.json`, and `integrations/omp/package.json`; closes
the `Unreleased` section of `CHANGELOG.md` as `## X.Y.Z - date`; verifies the
bumped tree with `cargo fmt --check`, `cargo clippy --all-targets --locked --
-D warnings`, and `cargo test --locked`; then commits `chore(release): bump to
X.Y.Z`, creates the tag `vX.Y.Z`, and pushes the two atomically. A refusal —
`main` moved, a dirty tree, a version file that drifted — leaves the tree
untouched. A push with no bump-worthy commit releases nothing.

A tag pushed with the workflow's own token does not start another workflow, so
the release job then dispatches the `Release` workflow explicitly
(`workflow_dispatch` is exempt from that rule) with the tag it just created.

## Manual releases

`bash scripts/release.sh` runs the same script on any checkout:

| Argument | Effect |
| --- | --- |
| `X.Y.Z` | release this version instead of the inferred one |
| `--dry-run` | print the decision and change nothing |
| `--no-verify` | skip `cargo fmt`/`clippy`/`test` |
| `--no-push` | commit and tag locally, do not push |

Tagging by hand works too: prepare the bump commit (same file list plus
`CHANGELOG.md`), verify CI is green on `main`, then
`git tag vX.Y.Z && git push origin vX.Y.Z`. A `v*` tag starts the `Release`
workflow; `workflow_dispatch` can release an existing tag by name.

## What the release workflow does

1. Build: each matrix entry runs `cargo build --release --locked --target <target>` on its runner, then archives the `leadline` binary as `leadline-<target>.tar.gz` (Unix) or `leadline-<target>.zip` (Windows). `--locked` pins the build to the committed `Cargo.lock`.
2. SBOM: the `publish` job scans the tag source with pinned `anchore/sbom-action` (syft `v1.51.1`) and writes CycloneDX JSON to `dist/leadline.cyclonedx.json`.
3. VERSION and checksums: the publish job writes the tag name to `dist/VERSION`, then `cd dist && sha256sum * > SHA256SUMS` covers every archive, the SBOM, and `VERSION`.
4. Publish: `gh release create "$TAG" dist/* --verify-tag --generate-notes` attaches everything to the tag release. `--verify-tag` aborts unless the tag resolves, so a release always names the commit it was built from.

## Commit-to-binary traceability

- `actions/checkout` fetches the tag commit; every build compiles that commit and nothing else.
- Each binary therefore maps to its tag commit: `git rev-parse vX.Y.Z` shows the exact source a release archive came from.
- The SBOM records the dependency closure (`Cargo.lock`) compiled into the binaries.

## Verify a download

```sh
sha256sum -c SHA256SUMS
```
