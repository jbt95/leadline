# leadline for Gemini CLI

Extension packaging the `leadline` MCP server, skill, `/leadline`
command, and non-blocking hooks. No metrics are reimplemented.

## Requires

`leadline` on `PATH` (`leadline --version` must work).

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
- Hooks are non-blocking: they always exit 0, return
  `{"decision":"continue"}`, and attach regressions as context only.
- Grant `leadline` subprocess execution; nothing else needed.

## Hook protocol

The hook reads Gemini hook JSON on stdin and writes hook JSON on
stdout. Diagnostic logs go to stderr only — never stdout, which
would corrupt the protocol.
