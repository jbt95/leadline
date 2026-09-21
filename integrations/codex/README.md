# leadline for Codex

MCP server plus skill and project instructions. No metrics are
reimplemented; the MCP server is the `leadline` binary itself.

## Requires

`leadline` on `PATH` (`leadline --version` must work).

## Install (project-local)

1. Append `mcp.example.toml` to the project's Codex MCP config.
2. Copy `AGENTS.snippet.md` into the project's `AGENTS.md`.
3. Reference `SKILL.md` from the project skills directory if used.

No global configuration is required.

## Context-size discipline

- Default to `leadline changed --format agent-json` (changed
  functions only, capped output).
- Never return full repository reports into agent context.
- Prefer per-function queries for follow-ups.

## Permissions

The MCP server is read-only: no source edits, no shell execution,
no network. Grant only MCP tool invocation.

## Uninstall

Remove the `mcp_servers.leadline` entry and the pasted `AGENTS.md`
section. No other project files are touched.
