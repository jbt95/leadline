# leadline Pi extension

Native Pi extension for the `leadline` analyzer. All analysis runs in the
`leadline` binary; this package only registers tools and warn-mode
post-edit feedback. No metric logic lives here.

## Install

1. Install the release binary (puts `leadline` on `~/.local/bin`):

   ```console
   curl -fsSL https://raw.githubusercontent.com/jbt95/leadline/main/install.sh | sh
   ```

2. Install the extension from this repository:

   ```console
   pi install git:github.com/jbt95/leadline
   ```

3. Restart Pi and confirm the tools are listed: `leadline_changed`,
   `leadline_function`, `leadline_gate`, `leadline_secret_check`.

## Uninstall

```console
pi remove git:github.com/jbt95/leadline
```

Removing the `leadline` binary also silently disables analysis.

## Permissions

The extension needs permission to spawn the `leadline` subprocess and to
read the repository files you ask it to analyze.

## What data is read

Only the source files under the analyzed path, plus git metadata for the
base revision in `leadline_changed`. Nothing leaves the machine; the
binary runs locally.

## Commands executed

- `leadline changed --base <rev> --format agent-json [--path <path>]`
- `leadline function <file> <name> --format agent-json`
- `leadline check <path> --format agent-json [--cognitive N --cyclomatic N --max-nesting N]`
- `leadline_secret_check` (shared secret gate; fails the tool call on
  findings or an unavailable scanner; see `docs/agent-integration-guide.md`)
- Post-edit (warn mode, after successful edit/write tool results):
  `leadline changed --base HEAD~1 --format agent-json` plus the secret-gate
  worktree warning

Analyzer failures (missing binary, git errors, unreadable files) surface as
tool text, the same as the OpenCode wrappers; parse errors appear in the
formatted output instead of an empty function list.

## How to disable

Unregister the extension. Removing the `leadline` binary also
silently disables analysis.
