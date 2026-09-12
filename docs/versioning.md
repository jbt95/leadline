# Versioning

`leadline` follows semver (`major.minor.patch`). Four identifiers evolve independently:

| Schema | Current | Bumped when |
| --- | --- | --- |
| CLI (flags, commands, exit codes) | 1.1 | A flag, command, or exit code is added, changed, or removed. |
| JSON schema (`schema_version`) | `1` | An envelope or function field is added, changed, or removed. |
| Metric spec (`metric_profile`, `metric_specs`) | `default-v1` | Any metric rule changes. Per-family versions allow one family to move (e.g. `cognitive: default-v2`) without renaming the rest. |
| MCP tools | 1.4 | A tool, input, or output shape changes. |

## Rules

- Patch releases fix bugs only. They never silently redefine a metric: identical source produces identical numbers across patches.
- A metric rule change requires a new spec tag (`default-v2`) and a minor or major release, plus a changelog entry describing the delta.
- Additive JSON fields are minor; removed or retyped fields are major.
- The analyzer reports its own `analyzer_version` in every JSON envelope so results stay attributable.
