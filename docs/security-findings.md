# Security findings

`leadline security [PATH] --sarif FILE` ingests scanner SARIF 2.1.0 output and
enriches each finding with function, risk, and changed-code context for local
triage and gating. Leadline never runs scanners, executes code, or touches the
network: it only reads artifact files you pass explicitly.

## Accepted SARIF fields

From each run: `tool.driver.name`, `tool.driver.rules[]` (`id` plus
`properties.security-severity`), and `results[]`. From each result: `ruleId`
(or `rule.id`), `level`, `locations[0].physicalLocation` (`artifactLocation.uri`
plus `region.startLine`/`endLine`), `partialFingerprints`, `fingerprints`
(object property bag or array), `baselineState`, and
`properties.security-severity`. Everything else —
including `message.text`, snippets, and embedded source — is dropped on read
and never serializes. Scanner messages and source text are omitted from every
output shape by construction, not by redaction.

Artifact URIs must be analysis-root-relative UTF-8 paths with `/` separators
(`file:` URIs shed their scheme; `\` becomes `/` for Windows scanners).
Percent escapes are decoded before validation, so `src/a%20b.ts` names
`src/a b.ts` and an encoded traversal is still rejected. Absolute paths,
parent traversals, drive prefixes, non-`file` URI schemes, and `file://`
authorities are rejected, as are files over 64 MiB, aggregate input over
256 MiB, and JSON nesting past 128 levels. Malformed SARIF exits `4`.

## Severity mapping

`properties.security-severity` on the result wins, then the matching rule's
value, then the SARIF `level` (`note`/`warning`/`error` map to
`low`/`medium`/`high`). A gitleaks result carrying none of those defaults to
`high` — gitleaks emits no severity signal, and a detected secret is a
concrete credential exposure the shared secret gate must block. Labels map
case-insensitively (`moderate` is `medium`);
numeric 0–10 scores map `<4` low, `<7` medium, `<9` high, `>=9` critical.
Otherwise missing or unparseable severity stays `unknown`, which sits below
every gate threshold and never violates.

## New versus changed

These are independent axes. A finding is `new` when its semantic key (tool,
rule, normalized path, scanner fingerprint or span fallback) is absent from all
`--baseline-sarif` files — or, with no baseline files, when SARIF
`baselineState` is `new` (`unchanged`/`updated` map to `existing`). A finding is
`changed` when its path is changed under the selected Git comparison
(`--base`, `--staged`, or `--target`); `changed` is `null` when no comparison
was requested or the finding has no path. Changed paths use the same
analysis-root base as artifact URIs, so a run scoped to `src/` matches a
scanner's `app.py`. Changed paths are pure Git state:
non-source files (`.env`, YAML, JSON, shell scripts) count, because scanners
report secrets and misconfigurations there too.

## Sort order and gates

Rows sort by severity descending, state (`new`, `existing`, `unknown`),
path (pathless last), line, tool, rule, and fingerprint. Duplicates merge
provenance `report_ids` without duplicating rows.

The command is informational unless `--fail-on-severity LEVEL` is present.
`--new-only` and `--changed-only` narrow the gate, never the displayed report.
`--changed-only` requires a comparison (`--base`, `--staged`, or `--target`):
without one no finding can be changed, so the combination is a usage error
instead of a silent pass. Any gate violation exits `1` with the full report
retained on stdout.

## Output shapes and nullability

- `--json`: the complete `SecurityReport` (`findings` with enrichment).
- `--format agent-json --top N` (default 50): first `N` findings plus
  `truncated`.
- `--format sarif`: one `{tool}/{rule_id}` result per finding with the fixed
  message `"{tool} finding {rule_id}"`; pathless findings carry no locations.
- Terminal: one `path:line [severity] tool/rule (state[,changed])
  function-or-reason` line per finding.
- `leadline check --sarif FILE` adds `security_violations` to check JSON and
  fails when either family fails; its SARIF output merges gate violations
  only, so results never contradict the exit code. MCP tool
  `security_findings` takes the same inputs with root-relative artifact paths.

Unknown evidence serializes as `null`: missing paths, spans, fingerprints are
computed, enrichment (`function_id`, metrics, `risk_score`, `fan_in`,
`blast_radius`, `concentration_percent`) stays null without attribution, and
unattributed findings carry a machine-readable `reason` (`no-path`,
`path-not-in-project`, `no-function-at-location`).

## Non-goals

Leadline is not a scanner, advisory database, package registry client, or
secret detector. It performs no reachability analysis beyond innermost-function
attribution and direct changed-path marking, and its severities are triage
signals, not exploitability judgments.
