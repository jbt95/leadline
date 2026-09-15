# Vulnerable dependency prioritization

`leadline vulnerabilities [PATH]` normalizes OSV-Scanner and Trivy reports
into one advisory list and prioritizes it with direct imports from changed
JavaScript/TypeScript code. Leadline consumes scanner reports only: it never
queries advisory services, registries, lockfiles, or package managers, and it
never touches the network.

## Accepted report fields

OSV-Scanner files keep package name/version/ecosystem, vulnerability ID,
`database_specific.severity` labels, plain numeric `severity[].score` values,
`affected[].ranges[].events[]` fixed versions, and the source path. Trivy
files keep `Target`, `Type` (lowercased as the ecosystem), `VulnerabilityID`,
`PkgName`, `InstalledVersion`, `FixedVersion`, and `Severity`. Descriptions,
titles, URLs, CVSS objects, and package-manager output are dropped on read and
never serialize; scanner versions are producer metadata only.

Inputs are bounded: 64 MiB per file, 256 MiB aggregate, JSON depth 128, one
million normalized rows, and at most 32 paths per input kind. Manifest paths
must be analysis-root-relative; absolute or escaping paths are rejected.
Malformed reports exit `4`.

## Severity mapping

The shared scanner mapping applies: case-insensitive labels (`moderate` is
`medium`) and plain numeric 0–10 scores (`<4` low, `<7` medium, `<9` high,
`>=9` critical). CVSS vectors are not scored: a vector without a literal
numeric score stays `unknown`, which sits below every gate threshold.

## New state and ordering

Advisory identity is lowercase ecosystem, package, installed version, and
advisory ID. Findings absent from all `--baseline-osv`/`--baseline-trivy`
files are `new` (everything is new with no baselines); present keys are
`existing`. Duplicates merge provenance `report_ids`. Rows sort by severity
descending, state, ecosystem, package, version, and advisory.

## Reachability model: changed direct imports

`reachable_from_changed` is direct-import evidence, not transitive runtime
reachability. With a Git comparison (`--base`, `--staged`, or `--target`),
changed files contribute bare npm imports (`import`, `require`, dynamic
`import`; `@scope/name/subpath` normalizes to `@scope/name`, relative imports
and Node builtins such as `node:fs` are ignored). An npm finding importing a
changed package gains `true` plus the matching `changed_imports`; npm findings
with no match gain `false`. Non-npm ecosystems have no package-to-import
mapping and stay `null`, as does everything without a comparison.

## Gates and configuration

The command is informational unless a threshold is set: `--fail-on-severity
LEVEL` for one invocation, or `[vulnerabilities] minimum_severity` in
`leadline.toml` (the flag wins). Only `new` findings at or above the minimum
fail, except `reachable_from_changed: false` suppresses failure; `true` and
`null` remain gate candidates. `leadline check --osv/--trivy` adds
`vulnerability_violations` to check JSON and fails when any family fails; its
SARIF output merges gate violations only. The read-only `vulnerabilities` MCP
tool takes the same inputs with root-relative artifact paths and applies
`[vulnerabilities] minimum_severity` when the argument is absent.
