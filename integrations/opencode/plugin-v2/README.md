# leadline native plugin for OpenCode V2 — EXPERIMENTAL

> The V2 plugin API is unstable. This was written against `@opencode/plugin`
> 2.0.2 (pinned in `package.json`) with opencode2 `v0.0.0-beta-18269` as the
> reference host. Expect breakage across V2 betas; check the
> [V2 plugins guide](https://opencode.ai/v2/docs/build/plugins) and the
> [V1 migration guide](https://opencode.ai/v2/docs/build/plugins/migrate-v1)
> when it stops loading. The V1 plugin in `../plugin/` is the stable path;
> MCP (`leadline mcp`) is the portable path.

Thin wrapper over the shared adapter core: shells out to the `leadline`
binary and returns compact text. No metrics are reimplemented. Effective tool
names are unchanged:

- `leadline_changed` — changed functions vs git base
- `leadline_function` — one function by file and name
- `leadline_check` — quality-gate check (warn mode)

## Install

Requires `leadline` on `PATH` (`leadline --version` must work).

1. Install the plugin dependency (network required once):
   ```console
   cd integrations/opencode/plugin-v2
   bun install
   ```
2. Link the plugin directory into the global plugins directory:
   ```console
   ln -sfn /path/to/leadline/integrations/opencode/plugin-v2 \
     ~/.config/opencode/plugins/leadline
   ```
3. Add a **file-level** entry to `opencode.json(c)`:
   ```jsonc
   {
     "$schema": "https://opencode.ai/config.json",
     "plugins": ["./plugins/leadline/index.ts"]
   }
   ```
   Point the entry at `index.ts`, not at the directory: a directory entry
   loads both the directory and its `index.ts` and fails the whole plugin
   reload with `Duplicate plugin ID: leadline`.
4. Restart the service and confirm the tools register:
   ```console
   opencode2 service restart
   ```
   A real `opencode2 run` tool call is the only reliable confirmation;
   `opencode2 plugin list` does not show directory-loaded plugins.

## Notes

- Tool results declare a string `output` schema plus a text `content` block:
  the V2 code-mode runtime reads `output`, so a result without it shows up as
  "no output" even when the tool ran.
- Each call resolves the session's project directory (`ctx.session.get`) and
  runs `leadline` there, so relative paths match the session.

## Permissions

Same as V1: grant `leadline` subprocess execution only. The plugin never
edits source, never touches the network, and caps tool output to protect
context.

## Uninstall

Remove the `plugins` entry and the `plugins/leadline` link. No other project
files are touched.
