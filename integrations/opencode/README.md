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

## Option C — Native plugin for OpenCode V2 (experimental)

> The V2 plugin API is unstable (see `plugin-v2/README.md`). The V1
> plugin above is the stable path.

`plugin-v2/` is a V2 port of Option B with the same three tools
(`leadline_changed`, `leadline_function`, `leadline_check`) and the same
shell-out contract. Setup and caveats are in `plugin-v2/README.md`.

MCP on V2 uses a nested shape (`mcp.servers`); the local-stdio equivalent
of Option A is:

```jsonc
{ "mcp": { "servers": { "leadline": { "type": "local", "command": ["leadline", "mcp"] } } } }
```

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
