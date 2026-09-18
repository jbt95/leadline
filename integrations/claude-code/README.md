# leadline for Claude Code

Thin plugin around the `leadline` binary. No metrics are reimplemented here.

## Requires

`leadline` on `PATH` (`leadline --version` must work). Secret gating (the
pre-commit staged hook) also needs `gitleaks`; without it the hook warns
visibly and non-blocking. The plugin vendors the shared gate runner, so
`LEADLINE_SECRET_RUNNER` is only needed when you point the hook at a
different runner.

## Install

```console
claude plugin marketplace add jbt95/leadline
claude plugin install leadline@leadline
```

Restart Claude Code, then confirm the plugin and its MCP server:

```console
claude plugin list
claude mcp list
```

The plugin ships three components: the `leadline` MCP server
(`leadline mcp`), warn-mode `PostToolUse` / `Stop` hooks, and the
`leadline` skill.

## Uninstall

```console
claude plugin uninstall leadline@leadline
claude plugin marketplace remove leadline
```

## Permissions

Hooks shell out to `leadline changed` / `leadline check` (warn mode, always
exit 0, print only on material regression). The `Stop` hook runs the check
only: the per-turn secret scan was removed (a whole-tree scan on every turn
while the gate only evaluates changed paths). Secret gating stays in the
pre-commit staged hook and the on-demand check (see
`docs/agent-integration-guide.md`).

## OS notes

- macOS / Linux: scripts run with `sh`; no dependencies beyond
  `leadline` (`python3` used opportunistically for JSON filtering).
- Windows: run under Git Bash (sh). Use `leadline.exe` on `PATH`.
