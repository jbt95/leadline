# leadline for OpenCode

Thin wrappers around the `leadline` binary. No metrics are reimplemented.
OpenCode V1 still uses the plugin in `plugin/`; the OpenCode V2 beta must
use `plugin-v2/` because the plugin APIs are incompatible.

## Requires

`leadline` on `PATH` (`leadline --version` must work).

## Option A — MCP (portable)

Merge `mcp.example.json` into your OpenCode config (`opencode.json`):

```json
{ "mcp": { "leadline": { "type": "local", "command": ["leadline", "mcp"] } } }
```

Best when you move between machines or share config with other
MCP-capable harnesses.

## Option B — Native plugin for OpenCode V1

Load `plugin/leadline.ts` as an OpenCode V1 plugin. It registers four
stable tools that delegate to the shared adapter core and shell out to the
same binary (identical behavior to the V2 plugin and Pi):

- `leadline_changed` — changed functions vs git base
- `leadline_function` — one function by file and name
- `leadline_gate` — quality-gate check (warn mode)
- `leadline_secret_check` — shared secret gate (warn mode, on demand)

MCP on V2 uses a nested shape (`mcp.servers`); the local-stdio
equivalent of Option A is:

```jsonc
{ "mcp": { "servers": { "leadline": { "type": "local", "command": ["leadline", "mcp"] } } } }
```

## Option C — Native plugin for OpenCode V2 (experimental)

> The V2 plugin API is unstable (see `plugin-v2/README.md`). The V1
> plugin above is the stable path; MCP is the portable path.

Requires `bun install` in `plugin-v2/` once, then link the directory into
the global plugins directory and reference the **entry file** (not the
directory) from `opencode.json`:

```console
ln -sfn /path/to/leadline/integrations/opencode/plugin-v2 \
  ~/.config/opencode/plugins/leadline
```

```jsonc
{ "plugins": ["./plugins/leadline/index.ts"] }
```

```console
opencode2 service restart
```

A directory entry (`./plugins/leadline`) loads the directory and its
`index.ts` and fails the reload with `Duplicate plugin ID: leadline`.
Confirm with a real `opencode2 run` tool call; `opencode2 plugin list` does
not list directory-loaded plugins.

## Skill

`skill/SKILL.md` teaches the agent when to run analysis and how to
interpret it. Reference it from your OpenCode instructions.

## Permissions

Grant `leadline` subprocess execution only. The plugin never edits
source, never touches the network, and caps tool output to protect
context (50 report lines in the shared core).

Recommended: allow `leadline *`, deny shell for everything else
in this integration's scope.

## Uninstall

Remove the `plugins/leadline` link or the MCP entry. No other project
files are touched, so removal leaves config clean.
