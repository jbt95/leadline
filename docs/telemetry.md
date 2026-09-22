# Local metrics

`leadline` can record what it does on your machine — how often each command
runs, how it exits, how long it takes, what it costs in CPU and memory, and
how many findings its gates report — into a local directory that Grafana
Alloy (or any Prometheus text-format scraper) can read. Nothing is sent
anywhere: there is no server, no account, and no network call. Recording is
off unless you name a directory:

```console
export LEADLINE_METRICS_DIR="$HOME/.leadline-metrics"
```

With the variable set (and non-empty), every CLI invocation and every MCP
tool call records one event. Unset it to disable recording; nothing else
changes, no file is created, and no measurable work is performed.

A process records only when it serves an entry point — a CLI command or an MCP
transport. A library caller that runs the analyzer in process, such as a test
binary that calls the MCP handler directly, never records, whatever the
variable says.

## What is recorded

| Metric | Type | Labels | Meaning |
| --- | --- | --- | --- |
| `leadline_invocations_total` | counter | `surface` (`cli`/`mcp`), `operation`, `outcome` | One per invocation or tool call. `operation` is the CLI command name or the MCP tool name, so `leadline stats` records `operation="stats"` and an unknown first argument records `other`. `outcome` is `success`, `gate_failed`, `usage_error`, `incomplete`, `input_error`, or `internal_error` (`gate_failed` mirrors exit `1`; `other` exists as a defensive fallback and no current command produces it). |
| `leadline_invocation_duration_seconds` | histogram | `surface`, `operation`, `outcome` | Wall-clock duration: cumulative `_bucket` samples over fixed bounds (`0.005`–`30` seconds, plus `+Inf`), with `_count` and `_sum`. Percentiles: `histogram_quantile(0.9, sum by (le, operation) (rate(leadline_invocation_duration_seconds_bucket[1h])))`; the mean is still `_sum` / `_count`. |
| `leadline_invocation_cpu_seconds` | histogram | `surface`, `operation`, `outcome` | CPU seconds (user plus system) the invocation consumed, measured from just after start-up: the whole command on the CLI, the delta across one tool call on MCP. Same `_bucket`/`_count`/`_sum` shape as duration, with its own bounds (`0.005`–`300` seconds). |
| `leadline_invocation_max_rss_bytes` | histogram | `surface`, `operation`, `outcome` | Peak resident set size at finish, in bytes (`16` MiB–`16` GiB bounds). On MCP this is the server's high-water mark, not a per-call figure. |
| `leadline_invocation_cpu_ratio` | histogram | `surface`, `operation` | Sampled CPU utilization in cores, one observation per sample tick; values above `1` are honest parallel work. Bounds `0.05`–`16`. |
| `leadline_invocation_rss_bytes` | histogram | `surface`, `operation` | Sampled resident bytes over the run, same bounds as peak RSS. |
| `leadline_live_cpu_millicores` | gauge | `surface`, `operation` | Latest sampled utilization, `1000` = one core; written at most once per second. |
| `leadline_live_rss_bytes` | gauge | `surface`, `operation` | Latest sampled resident bytes; written at most once per second. |
| `leadline_mcp_sessions_total` | counter | `transport`, `outcome` | One per MCP server session that ends, `outcome` `clean` or `error`; today only `transport="stdio"` emits it, because an HTTP server runs until the process is killed. |
| `leadline_mcp_session_seconds` | histogram | `transport` | MCP session lifetime, bounds `0.1`–`14400` seconds; stdio sessions only. |
| `leadline_mcp_errors_total` | counter | `transport` (`http`/`stdio`), `reason` | Protocol and transport failures by `reason`: `parse_error`, `invalid_request`, `method_not_found`, `tool_error`, `batch_too_large`, `response_too_large`, `http_bad_request`, `http_busy`, `origin_rejected`, or `body_budget_exhausted`. |
| `leadline_mcp_requests_total` | counter | `method` | One per JSON-RPC request (a batch counts per request). `method` is `initialize`, `tools_list`, `tools_call`, `ping`, `notification`, or `unknown`. |
| `leadline_mcp_inflight_calls` | gauge | `transport` | Tool calls currently executing. |
| `leadline_mcp_request_bytes` | histogram | `method` | Request line/body bytes, bounds `256` bytes–`32` MiB. |
| `leadline_mcp_response_bytes` | histogram | `method` | Response bytes, same bounds as requests. |
| `leadline_findings_total` | counter | `surface`, `operation`, `kind`, `state` | What the gates report. `check`: one `kind` per family (`function`, `parse_error`, `security`, `vulnerability`, `sql`) with `state="violation"` (zero counts never create a row). `debt`: `kind="function"` with `state="new"`/`"resolved"`, and `kind="risk"` with `state="increased"`/`"added"`. Recorded on both surfaces. |
| `leadline_security_findings_total` | counter | `surface`, `operation`, `kind` (`security`/`vulnerability`/`sql`), `severity` (`unknown`/`low`/`medium`/`high`/`critical`) | Scanner violations by family and severity, from `check` on either surface. |
| `leadline_parse_errors_total` | counter | `surface`, `operation`, `language` (`c`/`cpp`/`go`/`java`/`javascript`/`python`/`rust`/`typescript`/`tsx`) | Parse errors by language, from `check` on either surface. |
| `leadline_debt_functions` | gauge | `surface`, `state` | Standing function debt (`state="existing"`) from the most recent `debt` run. |
| `leadline_build_info` | gauge | `version`, `metrics_schema` | Constant `1`; identifies the analyzer version that rendered the file. |

Labels are closed sets, and the store enforces them: rows whose keys or
values do not match the tables above are dropped on every write and never
rendered. It therefore never contains a repository name, path, file or
function name, command argument, finding, commit, machine identifier, or
person — only counters, durations, process cost readings, and the fixed
labels above.

## CPU and memory

One sampler thread, started only when telemetry is enabled, reads the process
counters every 100 ms until the invocation ends. Each tick computes
`Δcpu / Δwall` (utilization in cores) and the current resident bytes, and
appends both to fixed-size histograms, so the memory it holds does not grow
with run length; the `leadline_live_*` gauges are written at most once per
second. At the end of the invocation the thread stops and its aggregates
merge into the store.
Short commands record their single end-of-run reading, so `cpu_seconds` and
`max_rss_bytes` are always present, and the utilization and resident-size
histograms carry that one observation instead of nothing.

The two surfaces report cost differently:

- On the CLI, `leadline_invocation_cpu_seconds` and
  `leadline_invocation_max_rss_bytes` describe the whole command, measured
  from just after start-up.
- On MCP, `cpu_seconds` is the delta across one tool call, while
  `max_rss_bytes` is the server's high-water mark for the process, not a
  per-call figure — a cheap tool call inside a large server still reads as
  the server's peak.

The counters come from one platform seam:

| Platform | CPU (user + system) | Current RSS | Peak RSS |
| --- | --- | --- | --- |
| macOS | `getrusage(RUSAGE_SELF)` | `proc_pid_rusage` (`RUSAGE_INFO_V2`) | `getrusage` `ru_maxrss` (already bytes) |
| Linux | `getrusage(RUSAGE_SELF)` | `/proc/self/statm` resident pages × page size | `getrusage` `ru_maxrss` (KiB) |
| Windows | `GetProcessTimes` | `GetProcessMemoryInfo` `WorkingSetSize` | `GetProcessMemoryInfo` `PeakWorkingSetSize` |
| other | nothing | nothing | nothing |

The peak in each sample is floored at that sample's current RSS: the two
counters are read at different instants (`getrusage` first), and macOS
`ru_maxrss` can trail `ri_resident_size` by a page, so a sample never reports
a peak below the resident size it just observed.

On targets outside the matrix the invocation is still recorded, but no
sampled or peak cost series appear: nothing is fabricated, and a failed or
unavailable counter read can never fail a command.

Sampling is measured, not assumed. On a release build for an Apple M2 Pro:

- 1.8 µs per counter read;
- 11.7 ms of CPU per 5 seconds of sampling, 0.23% of one core;
- -0.11% end-to-end on a 3.9 second analysis, i.e. inside noise.

## Files

The configured directory holds three files:

- `state.json` — the bounded, schema-versioned store (`schema_version` `3`).
  Missing, corrupt, or version-mismatched files start a fresh store instead
  of failing. Upgrading from an older schema resets existing counters once,
  which Prometheus handles as counter resets.
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
- Slow invocations (p90 by operation): `histogram_quantile(0.9, sum by (le, operation) (rate(leadline_invocation_duration_seconds_bucket[1h])))`
- CPU seconds (p90 by operation): `histogram_quantile(0.9, sum by (le, operation) (rate(leadline_invocation_cpu_seconds_bucket[1h])))`
- Peak resident memory (p90 by operation): `histogram_quantile(0.9, sum by (le, operation) (rate(leadline_invocation_max_rss_bytes_bucket[1h])))`
- Cores in use right now: `leadline_live_cpu_millicores / 1000`
- MCP failures by reason: `sum by (reason) (increase(leadline_mcp_errors_total[1d]))`
- Standing debt: `max(leadline_debt_functions{state="existing"})`

A falling `new` count beside a steady `resolved` count is the shape of
regressions being caught before they land. A rising `existing` gauge is the
shape of accepted debt growing.

## Stats page

`leadline stats` serves this same store to its loopback dashboard at
`GET /api/telemetry`: invocation counts, latency percentiles, live CPU and
memory gauges, and gate finding counts. Histogram rows also carry their
bucket bounds so the page can interpolate p50/p90/p99 latency, p90 CPU, and
p90 peak memory per operation. Every five seconds the server samples the
sampler's live CPU/RSS gauges plus store-wide totals into a capped ring of
720 points (one hour), served as the `series` array behind the CPU,
memory, and invocation-rate timeseries: those curves show what real
invocations consumed, never the idling server process. The page shows a
Telemetry section with those figures, or states that telemetry is off when
`LEADLINE_METRICS_DIR` is unset on the server. If the persisted store is
larger than 1 MiB, the endpoint reports an `error` status instead of serving
it. Otherwise it reads the bounded store,
adds histogram bucket bounds, and appends the in-memory `series` ring
(including per-operation p90 summaries); it never mutates the persisted store.

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
records `gate_failed` when its result reports `passed: false`. The `check`
and `debt` tools record finding counts exactly like their CLI counterparts
(`leadline_findings_total`, severity and language breakdowns included). The
server also records its own traffic: `leadline_mcp_sessions_total` and
`leadline_mcp_session_seconds` cover stdio session lifetime and how each one
ended (`transport` is `stdio` here; an HTTP server runs until its host kills
it, so it records no session row),
`leadline_mcp_requests_total` counts every JSON-RPC request by method,
`leadline_mcp_inflight_calls` shows concurrency, and
`leadline_mcp_request_bytes`/`leadline_mcp_response_bytes` size the payloads.
The server keeps its read-only guarantee for the analyzed repository: when
`LEADLINE_METRICS_DIR` is set it writes only inside that directory, so keep
the directory outside the analyzed repository to keep the repository itself
write-free. Hosts that treat `readOnlyHint` strictly should note that opt-in
metrics are the one exception.

## Limitations

- Anything that inherits the variable records, including scripts and tools
  that inherit your shell. Two things never record: the repository's own test
  suite, which removes `LEADLINE_METRICS_DIR` from every child it spawns, and
  any process that only calls the library in process. Unset the variable for
  other measurement runs you want kept clean, or point it at a separate
  directory.
- `leadline mcp` records stdio server sessions
  (`leadline_mcp_sessions_total`, `leadline_mcp_session_seconds`) alongside
  its tool calls; an HTTP server killed by its host records no session row,
  only its requests, errors, payloads, and in-flight gauge.
- On MCP, `leadline_invocation_max_rss_bytes` is the server's high-water
  mark, not what one tool call allocated; `cpu_seconds` is the per-call
  delta.
- Sampling needs macOS, Linux, or Windows. On any other target the
  invocation, duration, and finding series are still recorded, but no CPU or
  memory series appear, and a counter read that fails records nothing rather
  than failing the command.
- The `leadline_live_*` gauges are per invocation, not a process list: they
  keep the most recent invocation's last value until the next invocation
  overwrites them, so they must be read as "last writer", not "currently
  running".
- Under heavy parallel load a sample can be dropped when the store lock
  stays busy; the store itself is never corrupted.
- Counts are cumulative since the directory was created. Deleting the
  directory resets them, which Prometheus handles as a counter reset.
