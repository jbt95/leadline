# Cursor integration

MCP plus a project rule. The MCP server does the analysis; the rule teaches
the agent when to use it.

Requires a leadline build that provides the `mcp` subcommand.

## Install

Project-level rule:

```bash
mkdir -p .cursor/rules
cp integrations/cursor/rules/leadline.mdc .cursor/rules/leadline.mdc
```

Project-level MCP server: merge `mcp.example.json` into `.cursor/mcp.json`.
User-level MCP server: merge it into `~/.cursor/mcp.json` instead.

## Permissions

Prefer the MCP server: it needs no shell access beyond running `leadline mcp`.
If the agent calls the CLI directly instead, grant only the `leadline`
command, not a general shell.

## Uninstall

```bash
rm .cursor/rules/leadline.mdc
```

Remove the `leadline` entry from `.cursor/mcp.json` (or `~/.cursor/mcp.json`).
No other files are created, so nothing else needs cleanup.
