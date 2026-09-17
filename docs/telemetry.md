# Local metrics

`leadline` can record what it does on your machine — how often each command
runs, how it exits, how long it takes, and how many findings its gates report
— into a local directory that Grafana Alloy (or any Prometheus text-format
scraper) can read. Nothing is sent anywhere: there is no server, no account,
and no network call. Recording is off unless you name a directory:

```console
export LEADLINE_METRICS_DIR="$HOME/.leadline-metrics"
```

With the variable set (and non-empty), every CLI invocation and every MCP
tool call records one event. Unset it to disable recording; nothing else
changes, no file is created, and no measurable work is performed.

## What is recorded

| Metric | Type | Labels | Meaning |
| --- | --- | --- | --- |
| `leadline_invocations_total` | counter | `surface` (`cli`/`mcp`), `operation`, `outcome` | One per invocation or tool call. `outcome` is `success`, `gate_failed`, `usage_error`, `incomplete`, `input_error`, or `internal_error` (`gate_failed` mirrors exit `1`; `other` exists as a defensive fallback and no current command produces it). |
| `leadline_invocation_duration_seconds` | summary | `surface`, `operation` | Wall-clock duration (`_count` and `_sum` samples). |
| `leadline_findings_total` | counter | `surface`, `operation`, `kind`, `state` | What the gates report. `check`: one `kind` per family (`function`, `parse_error`, `security`, `vulnerability`, `sql`) with `state="violation"` (zero counts never create a row). `debt`: `kind="function"` with `state="new"`/`"resolved"`, and `kind="risk"` with `state="increased"`/`"added"`. |
| `leadline_debt_functions` | gauge | `surface`, `state` | Standing function debt (`state="existing"`) from the most recent `debt` run. |
| `leadline_build_info` | gauge | `version`, `metrics_schema` | Constant `1`; identifies the analyzer version that rendered the file. |

Labels are closed sets, and the store enforces them: rows whose keys or
values do not match the tables above are dropped on every write and never
rendered. It therefore never contains a repository name, path, file or
function name, command argument, finding, commit, machine identifier, or
person — only counters, durations, and the fixed labels above.

## Files

The configured directory holds three files:

- `state.json` — the bounded, schema-versioned store (`schema_version` `1`).
  Missing, corrupt, or version-mismatched files start a fresh store instead
  of failing.
- `leadline.prom` — Prometheus text exposition rendered from the store after
  every event. This is what a scraper reads.
- `state.lock` — the advisory lock file. It is never deleted and holds no
  data.

Both data files are rewritten atomically (sibling temporary file, then
rename), so a reader never sees a partial store. Concurrent invocations
serialize on the lock; an invocation that cannot take it within a short
budget drops its sample rather than delaying the command. A killed process
cannot leave a stale lock.

The store is cumulative per directory: counters only grow (Prometheus treats
a deleted store as a counter reset), and the `existing` gauge only updates
when `debt` runs.

## Reading the numbers

These metrics are **evidence, not proof**. They show that leadline ran, how
it exited, and what it reported; they cannot show that leadline caused a
later improvement. The strongest valid claims are associational:

- Adoption and usage mix: `sum by (operation) (increase(leadline_invocations_total[1d]))`
- Gate pressure: `sum(increase(leadline_invocations_total{outcome="gate_failed"}[7d]))`
- Debt flow — the closest thing to impact: `sum by (state) (increase(leadline_findings_total{operation="debt",kind="function",state=~"new|resolved"}[7d]))`
- Mean invocation time: `sum(rate(leadline_invocation_duration_seconds_sum[1h])) / sum(rate(leadline_invocation_duration_seconds_count[1h]))`
- Standing debt: `max(leadline_debt_functions{state="existing"})`

A falling `new` count beside a steady `resolved` count is the shape of
regressions being caught before they land. A rising `existing` gauge is the
shape of accepted debt growing.

## Grafana Alloy and Prometheus

Alloy (or the OpenTelemetry Collector) moves telemetry; it does not store it.
Visualizing in Grafana still needs a Prometheus-compatible store — for a
local setup, Prometheus is enough. The chain is:

```text
leadline -> LEADLINE_METRICS_DIR -> Alloy textfile collector -> Prometheus -> Grafana
```

Start Prometheus with remote-write receipt enabled:

```console
prometheus --web.enable-remote-write-receiver --config.file=prometheus.yml
```

Then have Alloy read the directory and forward to it:

```alloy
prometheus.exporter.unix "leadline" {
  textfile {
    directory = "/Users/you/.leadline-metrics"
  }
}

prometheus.scrape "leadline" {
  targets         = prometheus.exporter.unix.leadline.targets
  forward_to      = [prometheus.remote_write.local.receiver]
  scrape_interval = "30s"
}

prometheus.remote_write "local" {
  endpoint {
    url = "http://localhost:9090/api/v1/write"
  }
}
```

The `textfile` collector is enabled by default on the Unix exporter and
reads every `*.prom` file in the directory; on Windows use
`prometheus.exporter.windows` with a `textfile` block.

Point the pipeline at a managed store when you already run one (Grafana
Cloud, Grafana Mimir, or any Prometheus remote-write endpoint) instead of a
local Prometheus:

```alloy
prometheus.remote_write "cloud" {
  endpoint {
    url = "https://prometheus-xxx.grafana.net/api/prom/push"
    basic_auth {
      username = sys.env("PROMETHEUS_USERNAME")
      password = sys.env("GRAFANA_CLOUD_API_KEY")
    }
  }
}
```

The URL and credentials come from your stack's Prometheus details; a
self-hosted Mimir instance uses its own `/api/v1/push` URL and an
`X-Scope-OrgID` header when multi-tenant. Whether local or remote, the
collector only forwards — retention and queries live in the
Prometheus-compatible store. Add that endpoint as a Grafana data source and
build panels from the queries above.

## MCP

MCP tool calls record `surface="mcp"` invocations, and the `check` tool
records `gate_failed` when its result reports `passed: false`. Finding counts
(`leadline_findings_total`) are CLI-only in this version. The server keeps
its read-only guarantee for the analyzed repository: when
`LEADLINE_METRICS_DIR` is set it writes only inside that directory, so keep
the directory outside the analyzed repository to keep the repository itself
write-free. Hosts that treat `readOnlyHint` strictly should note that opt-in
metrics are the one exception.

## Limitations

- Anything that inherits the variable records, including `cargo test` runs
  and scripts. Unset it for measurement runs you want kept clean, or point it
  at a separate directory.
- `leadline mcp` records its tool calls, not the server session itself, so
  server lifetime never appears as a duration.
- Under heavy parallel load a sample can be dropped when the store lock
  stays busy; the store itself is never corrupted.
- Counts are cumulative since the directory was created. Deleting the
  directory resets them, which Prometheus handles as a counter reset.
