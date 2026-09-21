# Cline integration

MCP as the portable baseline, plus an optional lifecycle hook for post-edit
feedback. The hook and the MCP server are both backed by the same `leadline`
binary; no analysis logic is duplicated.

Requires a leadline build that provides the `mcp` subcommand.

## Install

MCP server: add `mcp.example.json` to Cline's MCP settings (Cline settings
-> MCP Servers -> configure `cline_mcp_settings.json`), merging with any
existing `mcpServers` entries.

Lifecycle hook (optional):

```bash
chmod +x integrations/cline/plugin/leadline-post-edit.sh integrations/cline/plugin/leadline-secret-check.sh
```

Register the script as a post-edit hook following `plugin/cline-hooks.example.json`
(adapting keys to your Cline version), optionally setting `LEADLINE_CHECK_ARGS`
to enable quality-gate status, e.g. `LEADLINE_CHECK_ARGS="--cognitive 15"`.

The hook obeys a stdin/stdout contract: it reads a hook event JSON object on
stdin, writes a compact report on stdout, and always exits 0. It is advisory
only and never blocks the agent.

Secret gate (optional): register
`integrations/cline/plugin/leadline-secret-check.sh` as the `secretGate`
hook from `plugin/cline-hooks.example.json`. It runs the shared worktree
gate wrapper, which exits `2` on findings and exits `1` (visible,
non-blocking) when the scanner or runner is unavailable; whether Cline
treats exit `2` as blocking is unverified, so
treat it as a visible check until you confirm it in your Cline version
(see `docs/agent-integration-guide.md`).

## Uninstall

Remove the `leadline` entry from `cline_mcp_settings.json`, remove the hook
registration, and delete the script copy if you installed it outside the repo.
Hook removal leaves no state behind.
