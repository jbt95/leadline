# Windsurf integration

MCP only. There is no dedicated leadline adapter for Windsurf; the MCP
server does the analysis and `instructions.md` teaches Cascade when to call it.

Requires a leadline build that provides the `mcp` subcommand.

## Install

Project-level: merge `mcp.example.json` into the project's MCP config and
copy `instructions.md` into `.windsurfrules` (or the project's Cascade rules)
so every checkout gets the same behavior.

User-level: merge `mcp.example.json` into the user MCP config (Windsurf
Settings -> Cascade -> MCP servers) so leadline is available in every project
without touching project files.

Verify config file locations against your Windsurf version; Windsurf
moves these paths between releases.

## Uninstall

Remove the `leadline` entry from the MCP config you edited and delete the
pasted rules from `.windsurfrules` (or Cascade rules). No other files are
created, so nothing else needs cleanup.
