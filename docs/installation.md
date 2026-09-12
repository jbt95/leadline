# Installation

## Requirements

- Rust 1.90 or later (edition 2024, MSRV 1.90).
- `git` on `PATH` for `changed` / `diff` only. Other commands do not need git.
- Supported languages: Java (`.java`), JavaScript (`.js`, `.jsx`, `.mjs`, `.cjs`), TypeScript (`.ts`, `.mts`, `.cts`), TSX (`.tsx`).

## From source

```console
cargo install --path .
leadline --version
```

## From releases

Download the binary for your platform from GitHub Releases:

- macOS ARM64 / x86-64
- Linux ARM64 / x86-64
- Windows x86-64

Place it on `PATH`, then verify:

```console
leadline --version
```

## Verify

```console
leadline analyze . --json | head -c 400
```

Exit `0` means analysis succeeded. See `cli-reference.md` for the full exit-code table.
