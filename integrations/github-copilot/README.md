# GitHub Copilot integration

Repository instructions plus a review agent, backed by the MCP server for
analysis. The analyzer stays read-only on every surface.

Requires a leadline build that provides the `mcp` subcommand.

## Install

Local / IDE (VS Code with agent mode):

- Merge `mcp.example.json` into `.vscode/mcp.json` (workspace) or the user
  MCP config (all workspaces). Verify the config key against your VS Code
  version.
- Copy `copilot-instructions.snippet.md` into `.github/copilot-instructions.md`.
- Copy `agents/leadline.agent.md` into `.github/agents/leadline.agent.md`.

## Cloud agents

Cloud agents cannot reach a local stdio MCP server, so MCP tools are
unavailable there. On cloud surfaces, rely on the checked-in instructions
and agent file plus direct CLI calls (`leadline changed`, `leadline check`)
where the runner provides a shell, e.g. in CI.

## Uninstall

Delete the copied `.github` assets and remove the `leadline` entry from the
MCP config. No other files are created, so nothing else needs cleanup.
