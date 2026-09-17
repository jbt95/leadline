# Vendored: anti-slop

- Upstream: <https://github.com/dmmulroy/anti-slop>
- Revision: `c44ef22ca116d0ba62a3ff663a0bd13a3f3fa40b`
- Cloned: 2026-09-17
- License: MIT — see `LICENSE` in this directory. The vendored
  `vendor/eslint-stylistic/` subtree keeps its own license and provenance files
  (copied with the source).

Vendored from upstream `src/`, with two exclusions:

- upstream's `*.test.ts` rule tests, because this project does not run
  upstream's rule-test harness;
- the `effect/` rule group, because this project does not use Effect.

The package pins `oxlint` and `@oxlint/plugins` to `1.78.0`, the versions
upstream developed this revision against (see `package.json`
`devDependencies`). Keep the two versions exact and equal.

## Updating

Retrieve a newer upstream revision, re-copy `src/` with the same exclusions,
update the revision above, and run `npm run check`. Local rule changes are
allowed; record any deviation here so the next update can reconcile it.
