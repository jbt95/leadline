# Versioning

`leadline` follows semver (`major.minor.patch`). Four identifiers evolve independently:

| Schema | Current | Bumped when |
| --- | --- | --- |
| CLI (flags, commands, exit codes) | 1.5 | A flag, command, or exit code is added, changed, or removed. |
| JSON schema (`schema_version`) | `2` analysis + `test-targets` / `1` other reports / none in scanner reports | An envelope or function field is added, changed, or removed. |
| Metric spec (`metric_profile`, `metric_specs`) | `default` | Any metric rule changes. The profile name does not version; metric changes ride the release version plus a changelog entry describing the delta. |
| MCP tools | 1.7 | A tool, input, or output shape changes. |

## Rules

- Patch releases fix bugs only. They never silently redefine a metric: identical source produces identical numbers across patches.
- A metric rule change requires a minor or major release plus a changelog entry describing the delta. The `default` profile name never versions — there are no `default-v2` tags.
- Additive JSON fields are minor; removed or retyped fields are major.
- The analyze/function/check and changed/diff envelopes share the top-level
  `schema_version` (`2`), and `test-targets` matches them. Every other report
  family (hotspots, coupling, dependencies, impact, risk, project, debt,
  duplication, and snapshots) versions independently and currently emits `1`;
  the scanner reports (`security`, `vulnerabilities`, `sql`, `sql-plan`) carry
  findings only, with no `schema_version`.
- The analyzer reports its own `analyzer_version` in the analysis and analytics
  envelopes so results stay attributable; the scanner reports carry findings
  only.
- The local metrics store ([telemetry.md](telemetry.md)) carries its own `schema_version` (`3`); it is not a report schema and never appears in report output.
