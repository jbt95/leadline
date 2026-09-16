# leadline for Gemini CLI

Extension packaging the `leadline` MCP server, skill, `/leadline`
command, post-edit feedback hooks, and a blocking secret gate. No metrics
are reimplemented.

## Requires

`leadline` on `PATH` (`leadline --version` must work). The secret gate also
needs `gitleaks`, and resolves its shared runner from the checkout
(`integrations/common/leadline-secret-check.sh`), from the host project
directory for a copied extension, or from `LEADLINE_SECRET_RUNNER` when set.

## Install

Project-local extension install:

```bash
gemini extensions install ./integrations/gemini
```

This registers the `leadline` MCP server (`leadline mcp`), the
skill, the `/leadline` command, and the AfterTool / AfterAgent hooks.

## Uninstall

```bash
gemini extensions uninstall leadline
```

Manual installs: delete the copied directory. No other project
files are touched, so removal leaves config clean.

## Permissions

- MCP server is read-only: no edits, no shell, no network.
- AfterTool hooks are non-blocking: they always exit 0, return
  `{"decision":"continue"}`, and attach regressions as context only. They
  drain the hook event without logging it.
- The AfterAgent secret gate exits `2` on findings with the runner's
  redacted diagnostics on stderr; an unavailable scanner or runner exits `1`
  (visible, non-blocking). Gemini treats exit `2` as a blocked turn (see
  `docs/agent-integration-guide.md`).
- Grant `leadline` subprocess execution; nothing else needed.

## Hook protocol

The hook reads Gemini hook JSON on stdin and writes hook JSON on
stdout. Diagnostic logs go to stderr only — never stdout, which
would corrupt the protocol. The manifests resolve both scripts from
`${extensionPath}`, and hook timeouts are milliseconds (60000).
