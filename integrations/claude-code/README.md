# leadline for Claude Code

Thin plugin around the `leadline` binary. No metrics are reimplemented here.

## Requires

`leadline` on `PATH` (`leadline --version` must work).

## Install

One command (project-local):

```bash
claude plugin add ./integrations/claude-code
```

Or copy `integrations/claude-code` into your project and register
`hooks/hooks.json` in Claude Code settings.

## Uninstall

```bash
claude plugin remove leadline
```

Manual installs: delete the copied directory and remove the
`hooks/hooks.json` entry. No other project files are touched, so
removal leaves config clean.

## Permissions

Hooks shell out to `leadline changed` / `leadline check` only.
Both hooks default to non-blocking warn mode: they always exit 0
and print only on material regression, never on success.

## OS notes

- macOS / Linux: scripts run with `sh`; no dependencies beyond
  `leadline` (`python3` used opportunistically for JSON filtering).
- Windows: run under Git Bash (sh). Use `leadline.exe` on `PATH`.
