# Releasing leadline

Releases are reproducible and traceable: every binary maps to exactly one tag commit.

## Process

1. Prepare: bump the version in `Cargo.toml`, `Cargo.lock`, `package.json`, `integrations/agent-adapter-ts/package.json`, `integrations/agent-adapter-ts/package-lock.json`, `integrations/pi/package.json`, and `integrations/omp/package.json`; update `CHANGELOG.md`; verify CI is green on `main`.
2. Tag: `git tag vX.Y.Z && git push origin vX.Y.Z`. A `v*` tag starts `.github/workflows/release.yml`.
3. Build: each matrix entry runs `cargo build --release --locked --target <target>` on its runner, then archives the `leadline` binary as `leadline-<target>.tar.gz` (Unix) or `leadline-<target>.zip` (Windows). `--locked` pins the build to the committed `Cargo.lock`.
4. SBOM: the `publish` job scans the tag source with pinned `anchore/sbom-action` (syft `v1.51.1`) and writes CycloneDX JSON to `dist/leadline.cyclonedx.json`.
5. Checksums: `sha256sum dist/* > dist/SHA256SUMS` covers every archive plus the SBOM.
6. Publish: `gh release create "$GITHUB_REF_NAME" dist/* --verify-tag --generate-notes` attaches everything to the tag release. `--verify-tag` aborts unless the tag resolves, so a release always names the commit it was built from.

## Commit-to-binary traceability

- `actions/checkout` fetches the tag commit; every build compiles that commit and nothing else.
- Each binary therefore maps to its tag commit: `git rev-parse vX.Y.Z` shows the exact source a release archive came from.
- The SBOM records the dependency closure (`Cargo.lock`) compiled into the binaries.

## Verify a download

```sh
sha256sum -c SHA256SUMS
```
