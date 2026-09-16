# Installation

One static binary, no runtime. Pick one block, then verify.

## macOS / Linux (install script)

```console
curl -fsSL https://raw.githubusercontent.com/jbt95/leadline/main/install.sh | sh
```

Options: `LEADLINE_VERSION=v0.7.0` pins a release, `LEADLINE_INSTALL_DIR` changes the destination (default `~/.local/bin`), `LEADLINE_SKIP_CHECKSUM=1` skips SHA256 verification (not recommended). `install.sh --help` prints the same.

If `~/.local/bin` is not on `PATH`, the script says so; enable it for your shell:

```console
export PATH="$HOME/.local/bin:$PATH"
```

## Windows (PowerShell)

```powershell
Invoke-WebRequest -Uri "https://github.com/jbt95/leadline/releases/latest/download/leadline-x86_64-pc-windows-msvc.zip" -OutFile "$env:TEMP\leadline.zip"
Expand-Archive -Path "$env:TEMP\leadline.zip" -DestinationPath "$env:USERPROFILE\.local\bin" -Force
& "$env:USERPROFILE\.local\bin\leadline.exe" --version
```

Then add `%USERPROFILE%\.local\bin` to `Path` (System Properties → Environment Variables) and reopen the terminal. Pin a version by replacing `latest/download` with `download/v0.7.0`.

## From source

Requires Rust 1.90 or later:

```console
cargo install --path .
```

## Verify

```console
leadline --version
leadline doctor
leadline analyze . --json | head -c 400
```

`doctor` self-checks parsers, coverage readers, `git`, and `leadline.toml`; exit `0` from `analyze` means analysis succeeded. Exit codes: see `cli-reference.md`.

## Updating

```console
leadline update
```

Downloads the latest release, verifies the archive against the release `SHA256SUMS`, and replaces the running binary in place. `LEADLINE_BASE_URL` points at a mirror. Harness integrations (Pi, OpenCode, Claude Code, and so on) update separately through their own installers.

## Uninstall

```console
rm ~/.local/bin/leadline   # or wherever LEADLINE_INSTALL_DIR pointed
```

## Next: connect your harness

`leadline` works as plain CLI everywhere. For agent harnesses, see the [agent integration guide](agent-integration-guide.md), the [compatibility matrix](../integrations/COMPATIBILITY.md), and [per-harness READMEs](../integrations/).
