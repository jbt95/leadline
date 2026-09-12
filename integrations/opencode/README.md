# leadline for OpenCode

Thin wrappers around the `leadline` binary. No metrics are reimplemented.

## Requires

`leadline` on `PATH` (`leadline --version` must work).

## Option A — MCP (portable)

Merge `mcp.example.json` into your OpenCode config (`opencode.json`):

```json
{ "mcp": { "leadline": { "type": "local", "command": ["leadline", "mcp"] } } }
```

Best when you move between machines or share config with other
MCP-capable harnesses.

## Option B — Native plugin (low overhead)

Load `plugin/leadline.ts` as an OpenCode plugin. It registers three
stable tools that shell out to the same binary:

- `leadline_changed` — changed functions vs git base
- `leadline_function` — one function by file and name
- `leadline_check` — quality-gate check (warn mode)

Prefer the native plugin for daily local use; keep MCP supported
for portability. Both return semantically identical results.

## Skill

`skill/SKILL.md` teaches the agent when to run analysis and how to
interpret it. Reference it from your OpenCode instructions.

## Permissions

Grant `leadline` subprocess execution only. The plugin never edits
source, never touches the network, and truncates output to 4000
chars to protect context.

Recommended: allow `leadline *`, deny shell for everything else
in this integration's scope.

## Uninstall

Remove the MCP entry or plugin file. No other project files are
touched, so removal leaves config clean.
