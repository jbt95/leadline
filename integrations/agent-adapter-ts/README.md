# leadline-agent-adapter

Shared TypeScript adapter for the `leadline` binary. No runtime
dependencies; Node built-ins only.

- `core/` — binary discovery (PATH plus `~/.cargo/bin/leadline`,
  `/usr/local/bin/leadline`, `/opt/homebrew/bin/leadline`), process
  spawn with fixed argument lists, JSON decoding, and compact formatting
  (capped at 50 functions with a truncation note). No metric logic lives
  here; all metrics come from the `leadline` Rust binary.
- `pi/` — thin Pi registration shim (lifecycle/registration only).
- `omp/` — thin OMP registration shim (lifecycle/registration only).

Tools: `leadline_changed`, `leadline_function`, `leadline_check`.
Post-edit feedback runs in warn mode only and never gates the agent.
