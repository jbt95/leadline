# leadline-agent-adapter

Shared TypeScript adapter for the `leadline` binary. No runtime
dependencies; Node built-ins only.

- `core/` — binary discovery (PATH plus `~/.cargo/bin/leadline`,
  `/usr/local/bin/leadline`, `/opt/homebrew/bin/leadline`), process
  spawn with fixed argument lists, agent-json decoding, compact
  formatting (capped at 50 lines), and warn-mode post-edit feedback.
  No metric logic lives here; all metrics come from the `leadline`
  Rust binary, and the core imports no harness APIs.
- `pi/` — Pi registration shim (three tools plus post-edit feedback).
  OMP vendors the same extension API, so `integrations/omp/` re-exports
  this registration instead of duplicating it.

Tools: `leadline_changed`, `leadline_function`, `leadline_check`.
Post-edit feedback runs in warn mode only and never gates the agent.

Typecheck with `npm install && npm run check` in this directory.
