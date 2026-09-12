# leadline Pi extension

Native Pi extension for the `leadline` analyzer. All analysis runs in the
`leadline` binary; this package only registers tools and warn-mode
post-edit feedback. No metric logic lives here.

## Install

1. Put the `leadline` binary on your PATH (`cargo build --release` in the
   leadline repository, then copy `target/release/leadline` to a PATH
   directory such as `~/.cargo/bin`).
2. Register this extension with Pi (see your Pi version's extension
   install flow) pointing at `integrations/pi/index.ts`.
3. Confirm the tools are listed: `leadline_changed`, `leadline_function`,
   `leadline_check`.

If the binary is missing, every tool fails with a message telling you
where it looked (PATH plus well-known install locations).

## Uninstall

Remove the extension registration from your Pi configuration and delete
this directory reference. Clean removal: the extension writes no files
outside Pi's own config, so removing the registration leaves nothing
behind. Optionally remove the `leadline` binary from your PATH.

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
- `leadline check <path> --format agent-json`

## How to disable

Unregister the extension, or set its post-edit mode off (`gate` and
`advisory` modes produce no post-edit output; only `warn` emits
feedback, and it never blocks). Removing the `leadline` binary also
silently disables analysis.
