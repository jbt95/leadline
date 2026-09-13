# leadline OMP extension

Native OMP extension for the `leadline` analyzer. OMP vendors the Pi
extension API, so this package re-exports the same registration as the Pi
extension (`../pi/index.ts`); only the manifests are harness-specific.
All analysis runs in the `leadline` binary. No metric logic lives here.

## Install

1. Install the release binary (puts `leadline` on `~/.local/bin`):

   ```console
   curl -fsSL https://raw.githubusercontent.com/jbt95/leadline/main/install.sh | sh
   ```

2. Install the extension from this repository:

   ```console
   omp plugin install git:github.com/jbt95/leadline
   ```

3. Restart OMP and confirm the plugin is healthy:

   ```console
   omp plugin doctor
   ```

   It must report a `pi` manifest and no load errors.

## MCP fallback

Where this native extension is not installed, use the MCP server as the
fallback for the same analysis:

```bash
leadline mcp
```

Configure it as a local MCP server in OMP. Do not register both the
native extension and the MCP server for the same project; that would
expose duplicate `leadline` tools.

## Uninstall

```console
omp plugin uninstall leadline
```

## Permissions

The extension needs permission to spawn the `leadline` subprocess and to
read the repository files you ask it to analyze.

## Commands executed

- `leadline changed --base <rev> --format agent-json [--path <path>]`
- `leadline function <file> <name> --format agent-json`
- `leadline check <path> --format agent-json [--cognitive N --cyclomatic N --max-nesting N]`
- Post-edit (warn mode, after successful edit/write tool results):
  `leadline changed --base HEAD~1 --format agent-json`
