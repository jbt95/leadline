# leadline native plugin for OpenCode V2 — EXPERIMENTAL

> The V2 plugin API is unstable. This was written against `@opencode/plugin`
> 2.0.2 (pinned in `package.json`) with opencode2 `v0.0.0-beta-18269` as the
> reference host. Expect breakage across V2 betas; check the
> [V2 plugins guide](https://opencode.ai/v2/docs/build/plugins) and the
> [V1 migration guide](https://opencode.ai/v2/docs/build/plugins/migrate-v1)
> when it stops loading. The V1 plugin in `../plugin/` is the stable path.

Thin wrapper with the same contract as the V1 plugin: shells out to the
`leadline` binary and returns compact JSON. No metrics are reimplemented.
Effective tool names are unchanged:

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
2. Reference the directory from your `opencode.json(c)` (path relative to
   the config file):
   ```jsonc
   {
     "$schema": "https://opencode.ai/config.json",
     "plugins": ["./integrations/opencode/plugin-v2"],
   }
   ```
3. Restart the OpenCode service (`opencode2 service restart`) and confirm
   the `leadline_*` tools are registered.

## Permissions

Same as V1: grant `leadline` subprocess execution only. The plugin never
edits source, never touches the network, and truncates output to 4000
chars to protect context.

## Uninstall

Remove the `plugins` entry. No other project files are touched.
