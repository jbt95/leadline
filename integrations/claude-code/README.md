# leadline for Claude Code

Thin plugin around the `leadline` binary. No metrics are reimplemented here.

## Requires

`leadline` on `PATH` (`leadline --version` must work).

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

Hooks shell out to `leadline changed` / `leadline check` only.
Both hooks default to non-blocking warn mode: they always exit 0
and print only on material regression, never on success.

## OS notes

- macOS / Linux: scripts run with `sh`; no dependencies beyond
  `leadline` (`python3` used opportunistically for JSON filtering).
- Windows: run under Git Bash (sh). Use `leadline.exe` on `PATH`.
