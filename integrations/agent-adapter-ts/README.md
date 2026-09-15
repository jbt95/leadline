# leadline-agent-adapter

Shared TypeScript adapter for the `leadline` binary. No runtime
dependencies; Node built-ins only.

- `core/` — binary discovery (PATH plus `~/.cargo/bin/leadline`,
  `/usr/local/bin/leadline`, `/opt/homebrew/bin/leadline`), process
  spawn with fixed argument lists, agent-json decoding (including per-file
  `parse_errors`, which formatting surfaces instead of an empty "no
  functions" result), compact formatting (capped at 50 lines), the shared
  secret-gate runner and status formatting (a missing scanner is
  `unavailable`, never `clean`), and warn-mode post-edit feedback. No
  metric logic lives here; all metrics come from the `leadline` Rust
  binary, and the core imports no harness APIs. OpenCode V1 and V2 both
  delegate to it, so identically named tools behave identically.
- `pi/` — Pi registration shim (four tools plus post-edit feedback).
  OMP vendors the same extension API, so `integrations/omp/` re-exports
  this registration instead of duplicating it.

Tools: `leadline_changed`, `leadline_function`, `leadline_check`, and
`leadline_secret_check`. Analyzer failures return the error as tool text
in every harness; only `leadline_secret_check` fails the call itself when
the gate reports findings or an unavailable scanner. Post-edit feedback
runs in warn mode only and never gates the agent.

Typecheck with `npm install && npm run check` in this directory.
