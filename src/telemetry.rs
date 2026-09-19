//! Opt-in local metrics for measuring leadline's own usage and outcomes.
//!
//! Disabled unless `LEADLINE_METRICS_DIR` points at a directory. When enabled,
//! every CLI invocation and MCP tool call updates a bounded, schema-versioned
//! store (`state.json` plus a lock file) and renders a Prometheus text file
//! (`leadline.prom`) for Grafana Alloy's textfile collector or any other
//! text-format scraper. Nothing is ever sent over the network, and nothing in
//! the store identifies a repository, path, argument, function, finding, or
//! person: only fixed label sets, counts, and durations.
//!
//! Recording is best-effort. Any I/O, lock, or serialization failure is
//! ignored, so metrics can never change command output, exit codes, or MCP
//! responses. Concurrent invocations block briefly on the store lock so no
//! sample is lost.
//!
//! The store is per-directory and cumulative. Deleting it resets every
//! counter, which Prometheus treats as a normal counter reset.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Environment variable naming the metrics directory; unset or empty disables
/// all recording.
pub const METRICS_DIR_VAR: &str = "LEADLINE_METRICS_DIR";

/// Version of the on-disk store; a mismatch starts a fresh store.
pub const STATE_SCHEMA_VERSION: u32 = 3;

const STATE_FILE_NAME: &str = "state.json";
const PROM_FILE_NAME: &str = "leadline.prom";
const LOCK_FILE_NAME: &str = "state.lock";

mod counters;
mod sampler;

pub use counters::ProcessSample;
pub use sampler::{ProcessCost, Sampler};

/// Reads this process's CPU time and resident memory, or `None` when the
/// platform cannot report them. Exposed for measurement code and tests.
pub fn process_sample() -> Option<ProcessSample> {
    counters::read()
}

/// Starts the sampler regardless of configuration, for measuring the
/// sampler's own overhead. Normal callers use [`Sampler::start`].
pub fn sampler_for_measurement(surface: &str, operation: &str) -> Sampler {
    Sampler::spawn(surface, operation)
}

/// True when telemetry is configured; the sampler checks this before it
/// spawns anything.
fn enabled() -> bool {
    metrics_directory().is_some()
}

/// Writes the live gauges of one running invocation.
fn record_live(surface: &str, operation: &str, millicores: Option<u64>, rss_bytes: Option<u64>) {
    mutate(|state| {
        if let Some(millicores) = millicores {
            state.set(
                "leadline_live_cpu_millicores",
                &[("surface", surface), ("operation", operation)],
                millicores,
            );
        }
        if let Some(rss_bytes) = rss_bytes {
            state.set(
                "leadline_live_rss_bytes",
                &[("surface", surface), ("operation", operation)],
                rss_bytes,
            );
        }
    });
}

/// Records one invocation's CPU time, peak memory, and sampled distributions.
/// Empty histograms record nothing, so a sampler that never ticked leaves only
/// the end-of-run readings.
pub fn record_process_cost(surface: &str, operation: &str, outcome: &str, cost: &ProcessCost) {
    mutate(|state| {
        if cost.cpu_seconds > 0.0 {
            state.observe(
                "leadline_invocation_cpu_seconds",
                &[
                    ("surface", surface),
                    ("operation", operation),
                    ("outcome", outcome),
                ],
                cost.cpu_seconds,
            );
        }
        if let Some(peak) = cost.peak_rss_bytes {
            state.observe(
                "leadline_invocation_max_rss_bytes",
                &[
                    ("surface", surface),
                    ("operation", operation),
                    ("outcome", outcome),
                ],
                peak as f64,
            );
        }
        let ratio = cost.cpu_ratio();
        state.observe_histogram(
            "leadline_invocation_cpu_ratio",
            &[("surface", surface), ("operation", operation)],
            &ratio.counts,
            ratio.count,
            ratio.sum,
        );
        let rss = cost.rss_histogram();
        state.observe_histogram(
            "leadline_invocation_rss_bytes",
            &[("surface", surface), ("operation", operation)],
            &rss.counts,
            rss.count,
            rss.sum,
        );
    });
}

/// One MCP server session ended.
pub fn record_mcp_session(transport: &str, outcome: &str, duration: Duration) {
    mutate(|state| {
        state.increment(
            "leadline_mcp_sessions_total",
            &[("transport", transport), ("outcome", outcome)],
            1,
        );
        state.observe(
            "leadline_mcp_session_seconds",
            &[("transport", transport)],
            duration.as_secs_f64(),
        );
    });
}

/// One MCP protocol or transport failure.
pub fn record_mcp_error(transport: &str, reason: &str) {
    mutate(|state| {
        state.increment(
            "leadline_mcp_errors_total",
            &[("transport", transport), ("reason", reason)],
            1,
        );
    });
}

/// One MCP JSON-RPC request, by resolved method label.
pub fn record_mcp_method(method: &str, notification: bool) {
    mutate(|state| {
        state.increment(
            "leadline_mcp_requests_total",
            &[("method", method_label(method, notification))],
            1,
        );
    });
}

/// Current number of tool calls executing.
pub fn set_mcp_inflight(transport: &str, inflight: u64) {
    mutate(|state| {
        state.set(
            "leadline_mcp_inflight_calls",
            &[("transport", transport)],
            inflight,
        );
    });
}

/// Bytes in and out for one request line or body.
pub fn record_mcp_payload(method: &str, notification: bool, request: u64, response: u64) {
    mutate(|state| {
        let labels = [("method", method_label(method, notification))];
        state.observe("leadline_mcp_request_bytes", &labels, request as f64);
        state.observe("leadline_mcp_response_bytes", &labels, response as f64);
    });
}

/// Maps one JSON-RPC method onto the closed `method` label set.
pub fn method_label(method: &str, notification: bool) -> &'static str {
    if notification {
        return "notification";
    }
    match method {
        "initialize" => "initialize",
        "tools/list" => "tools_list",
        "tools/call" => "tools_call",
        "ping" => "ping",
        _ => "unknown",
    }
}

/// One metric family the store knows how to render. Unknown families read
/// from a tampered store are dropped rather than rendered.
struct Family {
    name: &'static str,
    help: &'static str,
    kind: Kind,
    /// Exact label key set; a row with any other key is dropped.
    label_keys: &'static [&'static str],
    /// Cumulative upper bounds for `Kind::Summary`; empty for counters and gauges.
    buckets: &'static [f64],
}

#[derive(Clone, Copy)]
enum Kind {
    Counter,
    Gauge,
    Summary,
}

/// Resident-memory histogram bounds in bytes: 16 MiB … 16 GiB.
const RSS_BUCKETS: &[f64] = &[
    16_777_216.0,
    33_554_432.0,
    67_108_864.0,
    134_217_728.0,
    268_435_456.0,
    536_870_912.0,
    1_073_741_824.0,
    2_147_483_648.0,
    4_294_967_296.0,
    8_589_934_592.0,
    17_179_869_184.0,
];

/// MCP payload histogram bounds in bytes: 256 B … 32 MiB.
const PAYLOAD_BUCKETS: &[f64] = &[
    256.0,
    1024.0,
    4096.0,
    16384.0,
    65536.0,
    262_144.0,
    1_048_576.0,
    4_194_304.0,
    16_777_216.0,
    33_554_432.0,
];

const FAMILIES: &[Family] = &[
    Family {
        name: "leadline_invocations_total",
        help: "Leadline invocations by surface, operation, and outcome.",
        kind: Kind::Counter,
        label_keys: &["surface", "operation", "outcome"],
        buckets: &[],
    },
    Family {
        name: "leadline_invocation_duration_seconds",
        help: "Wall-clock duration of leadline invocations in seconds.",
        kind: Kind::Summary,
        label_keys: &["surface", "operation", "outcome"],
        buckets: DURATION_BUCKETS,
    },
    Family {
        name: "leadline_findings_total",
        help: "Findings reported by gating commands, by operation, kind, and state.",
        kind: Kind::Counter,
        label_keys: &["surface", "operation", "kind", "state"],
        buckets: &[],
    },
    Family {
        name: "leadline_security_findings_total",
        help: "Scanner finding violations by operation, family, and severity.",
        kind: Kind::Counter,
        label_keys: &["surface", "operation", "kind", "severity"],
        buckets: &[],
    },
    Family {
        name: "leadline_parse_errors_total",
        help: "Parse errors by operation and language.",
        kind: Kind::Counter,
        label_keys: &["surface", "operation", "language"],
        buckets: &[],
    },
    Family {
        name: "leadline_debt_functions",
        help: "Standing function debt in the most recent debt run, by state.",
        kind: Kind::Gauge,
        label_keys: &["surface", "state"],
        buckets: &[],
    },
    Family {
        name: "leadline_invocation_cpu_seconds",
        help: "CPU seconds (user plus system) consumed by leadline invocations.",
        kind: Kind::Summary,
        label_keys: &["surface", "operation", "outcome"],
        buckets: &[
            0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0, 300.0,
        ],
    },
    Family {
        name: "leadline_invocation_max_rss_bytes",
        help: "Peak resident set size of leadline invocations in bytes.",
        kind: Kind::Summary,
        label_keys: &["surface", "operation", "outcome"],
        buckets: RSS_BUCKETS,
    },
    Family {
        name: "leadline_invocation_cpu_ratio",
        help: "CPU cores in use, sampled during leadline invocations.",
        kind: Kind::Summary,
        label_keys: &["surface", "operation"],
        buckets: &[0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 2.0, 4.0, 8.0, 16.0],
    },
    Family {
        name: "leadline_invocation_rss_bytes",
        help: "Resident set size in bytes, sampled during leadline invocations.",
        kind: Kind::Summary,
        label_keys: &["surface", "operation"],
        buckets: RSS_BUCKETS,
    },
    Family {
        name: "leadline_live_cpu_millicores",
        help: "CPU cores in use now, in millicores (1000 = one core).",
        kind: Kind::Gauge,
        label_keys: &["surface", "operation"],
        buckets: &[],
    },
    Family {
        name: "leadline_live_rss_bytes",
        help: "Resident set size of the running invocation in bytes.",
        kind: Kind::Gauge,
        label_keys: &["surface", "operation"],
        buckets: &[],
    },
    Family {
        name: "leadline_mcp_sessions_total",
        help: "MCP server sessions by transport and outcome.",
        kind: Kind::Counter,
        label_keys: &["transport", "outcome"],
        buckets: &[],
    },
    Family {
        name: "leadline_mcp_session_seconds",
        help: "MCP server session lifetime in seconds.",
        kind: Kind::Summary,
        label_keys: &["transport"],
        buckets: &[0.1, 1.0, 10.0, 60.0, 300.0, 1800.0, 3600.0, 14400.0],
    },
    Family {
        name: "leadline_mcp_errors_total",
        help: "MCP protocol and transport failures by transport and reason.",
        kind: Kind::Counter,
        label_keys: &["transport", "reason"],
        buckets: &[],
    },
    Family {
        name: "leadline_mcp_requests_total",
        help: "MCP JSON-RPC requests by method.",
        kind: Kind::Counter,
        label_keys: &["method"],
        buckets: &[],
    },
    Family {
        name: "leadline_mcp_inflight_calls",
        help: "MCP tool calls currently executing.",
        kind: Kind::Gauge,
        label_keys: &["transport"],
        buckets: &[],
    },
    Family {
        name: "leadline_mcp_request_bytes",
        help: "MCP request bytes by method.",
        kind: Kind::Summary,
        label_keys: &["method"],
        buckets: PAYLOAD_BUCKETS,
    },
    Family {
        name: "leadline_mcp_response_bytes",
        help: "MCP response bytes by method.",
        kind: Kind::Summary,
        label_keys: &["method"],
        buckets: PAYLOAD_BUCKETS,
    },
];

/// Closed value sets for every label except `operation`, so a tampered store
/// cannot export arbitrary label values.
const LABEL_VALUES: &[(&str, &[&str])] = &[
    ("surface", &["cli", "mcp"]),
    (
        "outcome",
        &[
            "success",
            "gate_failed",
            "usage_error",
            "incomplete",
            "input_error",
            "internal_error",
            "other",
            "clean",
            "error",
        ],
    ),
    ("transport", &["http", "stdio"]),
    (
        "method",
        &[
            "initialize",
            "tools_list",
            "tools_call",
            "ping",
            "notification",
            "unknown",
        ],
    ),
    (
        "reason",
        &[
            "parse_error",
            "invalid_request",
            "method_not_found",
            "tool_error",
            "batch_too_large",
            "response_too_large",
            "http_bad_request",
            "http_busy",
            "origin_rejected",
            "body_budget_exhausted",
        ],
    ),
    (
        "kind",
        &[
            "function",
            "parse_error",
            "security",
            "vulnerability",
            "sql",
            "risk",
        ],
    ),
    (
        "state",
        &[
            "violation",
            "new",
            "resolved",
            "increased",
            "added",
            "existing",
        ],
    ),
    (
        "severity",
        &["unknown", "low", "medium", "high", "critical"],
    ),
    ("language", &["java", "javascript", "typescript", "tsx"]),
];

/// Hard ceiling per series list; real usage stays in the low hundreds.
const MAX_SERIES: usize = 4096;

/// Fixed duration histogram buckets (seconds). Chosen for sub-millisecond
/// version probes up to multi-second whole-repo checks.
const DURATION_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
];

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Row {
    metric: String,
    labels: BTreeMap<String, String>,
    value: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SummaryRow {
    metric: String,
    labels: BTreeMap<String, String>,
    count: u64,
    sum: f64,
    buckets: Vec<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MetricState {
    schema_version: u32,
    #[serde(default)]
    counters: Vec<Row>,
    #[serde(default)]
    summaries: Vec<SummaryRow>,
    #[serde(default)]
    gauges: Vec<Row>,
}

impl Default for MetricState {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            counters: Vec::new(),
            summaries: Vec::new(),
            gauges: Vec::new(),
        }
    }
}

impl MetricState {
    /// Adds `by` to one counter row, creating it on first use. Zero values
    /// never create a row, so unused label combinations stay absent.
    fn increment(&mut self, metric: &str, labels: &[(&str, &str)], by: u64) {
        if by == 0 {
            return;
        }
        let labels = labels_map(labels);
        if let Some(row) = self
            .counters
            .iter_mut()
            .find(|row| row.metric == metric && row.labels == labels)
        {
            row.value = row.value.saturating_add(by);
        } else {
            self.counters.push(Row {
                metric: metric.to_owned(),
                labels,
                value: by,
            });
        }
    }

    /// Records one observation for a histogram family (count, sum, and
    /// cumulative buckets) using that family's own bounds.
    fn observe(&mut self, metric: &str, labels: &[(&str, &str)], value: f64) {
        let Some(buckets) = family_for(metric).map(|family| family.buckets) else {
            return;
        };
        let mut counts = vec![0u64; buckets.len()];
        for (slot, bound) in counts.iter_mut().zip(buckets.iter()) {
            if value <= *bound {
                *slot = 1;
            }
        }
        self.observe_histogram(metric, labels, &counts, 1, value);
    }

    /// Merges `count` observations into a cumulative histogram row. A counts
    /// slice that does not match the family's bucket count is dropped, which
    /// is also how an empty sampler result records nothing.
    fn observe_histogram(
        &mut self,
        metric: &str,
        labels: &[(&str, &str)],
        counts: &[u64],
        count: u64,
        sum: f64,
    ) {
        let Some(family) = family_for(metric) else {
            return;
        };
        if count == 0 || counts.len() != family.buckets.len() {
            return;
        }
        let labels = labels_map(labels);
        if let Some(row) = self
            .summaries
            .iter_mut()
            .find(|row| row.metric == metric && row.labels == labels)
        {
            row.count = row.count.saturating_add(count);
            row.sum += sum;
            for (slot, add) in row.buckets.iter_mut().zip(counts.iter()) {
                *slot = slot.saturating_add(*add);
            }
        } else {
            self.summaries.push(SummaryRow {
                metric: metric.to_owned(),
                labels,
                count,
                sum,
                buckets: counts.to_vec(),
            });
        }
    }

    /// Sets one gauge row to the latest value.
    fn set(&mut self, metric: &str, labels: &[(&str, &str)], value: u64) {
        let labels = labels_map(labels);
        if let Some(row) = self
            .gauges
            .iter_mut()
            .find(|row| row.metric == metric && row.labels == labels)
        {
            row.value = value;
        } else {
            self.gauges.push(Row {
                metric: metric.to_owned(),
                labels,
                value,
            });
        }
    }

    /// Drops rows that do not match a known family's exact label schema, so a
    /// tampered store stays bounded and cannot export arbitrary text.
    fn prune(&mut self) {
        self.counters.retain(valid_row);
        self.gauges.retain(valid_row);
        self.summaries.retain(valid_summary_row);
        self.counters.truncate(MAX_SERIES);
        self.gauges.truncate(MAX_SERIES);
        self.summaries.truncate(MAX_SERIES);
    }
}

fn family_for(metric: &str) -> Option<&'static Family> {
    FAMILIES.iter().find(|family| family.name == metric)
}

fn valid_row(row: &Row) -> bool {
    family_for(&row.metric).is_some_and(|family| valid_labels(family, &row.labels))
}

fn valid_summary_row(row: &SummaryRow) -> bool {
    family_for(&row.metric).is_some_and(|family| {
        valid_labels(family, &row.labels) && row.buckets.len() == family.buckets.len()
    })
}

/// A label map fits a family when its key set matches exactly, every
/// closed-set label carries an allowed value, and `operation` names a command
/// or tool.
fn valid_labels(family: &Family, labels: &BTreeMap<String, String>) -> bool {
    labels.len() == family.label_keys.len()
        && family
            .label_keys
            .iter()
            .all(|key| labels.contains_key(*key))
        && labels.iter().all(|(key, value)| match closed_set(key) {
            Some(allowed) => allowed.contains(&value.as_str()),
            None => key == "operation" && valid_operation(value),
        })
}

fn closed_set(key: &str) -> Option<&'static [&'static str]> {
    LABEL_VALUES
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, allowed)| *allowed)
}

/// Command and tool names are lowercase words joined by `-` or `_`.
fn valid_operation(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || character == '-'
                || character == '_'
        })
}

fn labels_map(labels: &[(&str, &str)]) -> BTreeMap<String, String> {
    labels
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

/// Records one CLI invocation or MCP tool call: a counter by outcome and one
/// duration observation.
pub fn record_invocation(surface: &str, operation: &str, outcome: &str, duration: Duration) {
    mutate(|state| {
        state.increment(
            "leadline_invocations_total",
            &[
                ("surface", surface),
                ("operation", operation),
                ("outcome", outcome),
            ],
            1,
        );
        state.observe(
            "leadline_invocation_duration_seconds",
            &[
                ("surface", surface),
                ("operation", operation),
                ("outcome", outcome),
            ],
            duration.as_secs_f64(),
        );
    });
}

/// Totals plus breakdowns for one `check` gate run. Counts only; paths
/// never reach this struct. `severity` holds `(kind, severity, count)`
/// triples for the scanner families; `languages` holds `(language, count)`
/// pairs for parse errors.
pub struct CheckFindings<'a> {
    pub functions: u64,
    pub parse_errors: u64,
    pub security: u64,
    pub vulnerabilities: u64,
    pub sql: u64,
    pub severity: &'a [(&'a str, &'a str, u64)],
    pub languages: &'a [(&'a str, u64)],
}

/// Records the `check` gate's function, parse-error, and scanner finding
/// counts. Families with no input contribute zero and never create a row.
/// Unknown severity or language strings are dropped by pruning.
pub fn record_check_findings(surface: &str, findings: &CheckFindings<'_>) {
    mutate(|state| {
        for (kind, count) in [
            ("function", findings.functions),
            ("parse_error", findings.parse_errors),
            ("security", findings.security),
            ("vulnerability", findings.vulnerabilities),
            ("sql", findings.sql),
        ] {
            state.increment(
                "leadline_findings_total",
                &[
                    ("surface", surface),
                    ("operation", "check"),
                    ("kind", kind),
                    ("state", "violation"),
                ],
                count,
            );
        }
        for (kind, level, count) in findings.severity {
            state.increment(
                "leadline_security_findings_total",
                &[
                    ("surface", surface),
                    ("operation", "check"),
                    ("kind", kind),
                    ("severity", level),
                ],
                *count,
            );
        }
        for (language, count) in findings.languages {
            state.increment(
                "leadline_parse_errors_total",
                &[
                    ("surface", surface),
                    ("operation", "check"),
                    ("language", language),
                ],
                *count,
            );
        }
    });
}

/// Maps a file path to its metrics language label, if supported.
pub fn language_label(path: &str) -> Option<&'static str> {
    crate::parser::detect_language(path).map(|language| language.as_str())
}

/// Records one `debt` run: new and resolved function debt, increased and added
/// risk rows, and the standing `existing` function count.
pub fn record_debt(surface: &str, summary: &crate::debt::DebtSummary) {
    mutate(|state| {
        for (kind, state_name, count) in [
            ("function", "new", summary.new),
            ("function", "resolved", summary.resolved),
            ("risk", "increased", summary.risk_increased),
            ("risk", "added", summary.risk_added),
        ] {
            state.increment(
                "leadline_findings_total",
                &[
                    ("surface", surface),
                    ("operation", "debt"),
                    ("kind", kind),
                    ("state", state_name),
                ],
                count,
            );
        }
        state.set(
            "leadline_debt_functions",
            &[("surface", surface), ("state", "existing")],
            summary.existing,
        );
    });
}

/// Applies one update under the store lock when metrics are enabled.
fn mutate(update: impl FnOnce(&mut MetricState)) {
    let Some(directory) = metrics_directory() else {
        return;
    };
    let _ = update_store(&directory, update);
}

fn metrics_directory() -> Option<PathBuf> {
    let value = std::env::var_os(METRICS_DIR_VAR)?;
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

/// Locks, reads, updates, and atomically rewrites the store. The lock lives on
/// a stable sibling file (never renamed, never deleted) and is released when
/// the file handle drops, so a killed process cannot leave a stale lock.
fn update_store(directory: &Path, update: impl FnOnce(&mut MetricState)) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let lock_path = directory.join(LOCK_FILE_NAME);
    // A pre-planted symlink would otherwise be followed; unlink one before
    // opening so every file the store touches stays inside the directory.
    if std::fs::symlink_metadata(&lock_path).is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        std::fs::remove_file(&lock_path)?;
    }
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    // Block so concurrent invocations queue instead of losing samples. The
    // lock releases when `lock` drops, so a killed holder cannot wedge us.
    lock.lock()?;
    let mut state = read_state(directory);
    update(&mut state);
    state.prune();
    let encoded = serde_json::to_vec(&state).map_err(std::io::Error::other)?;
    write_atomic(&directory.join(STATE_FILE_NAME), &encoded)?;
    write_atomic(&directory.join(PROM_FILE_NAME), render(&state).as_bytes())?;
    Ok(())
}

/// Reads the store, tolerating missing, unreadable, corrupt, and
/// schema-mismatched files by starting fresh.
fn read_state(directory: &Path) -> MetricState {
    std::fs::read(directory.join(STATE_FILE_NAME))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<MetricState>(&bytes).ok())
        .filter(|state| state.schema_version == STATE_SCHEMA_VERSION)
        .unwrap_or_default()
}

/// Writes through a per-process sibling temporary file and rename, so a
/// reader never sees a partial store and a pre-created symlink or stale
/// temporary file cannot redirect the write.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let file_name = path
        .file_name()
        .ok_or_else(|| std::io::Error::other("metrics path has no file name"))?;
    let mut sibling = path.to_path_buf();
    sibling.set_file_name(format!(
        ".{}.tmp.{}",
        file_name.to_string_lossy(),
        std::process::id()
    ));
    // `create_new` refuses to follow a symlink; the only legitimate occupant
    // is a leftover from a crashed process with the same pid.
    let mut file = match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&sibling)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            std::fs::remove_file(&sibling)?;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&sibling)?
        }
        Err(error) => return Err(error),
    };
    if let Err(error) = file.write_all(bytes) {
        drop(file);
        let _ = std::fs::remove_file(&sibling);
        return Err(error);
    }
    drop(file);
    if let Err(error) = std::fs::rename(&sibling, path) {
        let _ = std::fs::remove_file(&sibling);
        return Err(error);
    }
    Ok(())
}

/// Renders every known family in Prometheus text exposition format with
/// sorted rows, so identical state always produces identical bytes.
fn render(state: &MetricState) -> String {
    let mut output = String::new();
    for family in FAMILIES {
        match family.kind {
            Kind::Counter | Kind::Gauge => {
                let source = match family.kind {
                    Kind::Counter => &state.counters,
                    _ => &state.gauges,
                };
                let mut rows: Vec<&Row> = source
                    .iter()
                    .filter(|row| row.metric == family.name && valid_labels(family, &row.labels))
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                rows.sort_by(|left, right| left.labels.cmp(&right.labels));
                let kind = match family.kind {
                    Kind::Counter => "counter",
                    _ => "gauge",
                };
                output.push_str(&format!(
                    "# HELP {} {}\n# TYPE {} {}\n",
                    family.name, family.help, family.name, kind
                ));
                for row in rows {
                    output.push_str(&format!(
                        "{}{} {}\n",
                        family.name,
                        labels_text(&row.labels),
                        row.value
                    ));
                }
            }
            Kind::Summary => {
                let mut rows: Vec<&SummaryRow> = state
                    .summaries
                    .iter()
                    .filter(|row| row.metric == family.name && valid_summary_row(row))
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                rows.sort_by(|left, right| left.labels.cmp(&right.labels));
                output.push_str(&format!(
                    "# HELP {} {}\n# TYPE {} histogram\n",
                    family.name, family.help, family.name
                ));
                for row in rows {
                    let labels = labels_text(&row.labels);
                    for (bound, count) in family.buckets.iter().zip(row.buckets.iter()) {
                        output.push_str(&format!(
                            "{}_bucket{} {}\n",
                            family.name,
                            with_le(&labels, &bound.to_string()),
                            count
                        ));
                    }
                    output.push_str(&format!(
                        "{}_bucket{} {}\n{}_sum{} {}\n{}_count{} {}\n",
                        family.name,
                        with_le(&labels, "+Inf"),
                        row.count,
                        family.name,
                        labels,
                        row.sum,
                        family.name,
                        labels,
                        row.count
                    ));
                }
            }
        }
    }
    output.push_str(
        "# HELP leadline_build_info Leadline version and local metrics store schema.\n\
         # TYPE leadline_build_info gauge\n",
    );
    output.push_str(&format!(
        "leadline_build_info{{metrics_schema=\"{}\",version=\"{}\"}} 1\n",
        STATE_SCHEMA_VERSION,
        env!("CARGO_PKG_VERSION")
    ));
    output
}

fn labels_text(labels: &BTreeMap<String, String>) -> String {
    if labels.is_empty() {
        return String::new();
    }
    let mut text = String::from("{");
    for (index, (key, value)) in labels.iter().enumerate() {
        if index > 0 {
            text.push(',');
        }
        text.push_str(key);
        text.push_str("=\"");
        text.push_str(&escape_label_value(value));
        text.push('"');
    }
    text.push('}');
    text
}

fn with_le(base: &str, le: &str) -> String {
    if base.is_empty() {
        return format!("{{le=\"{le}\"}}");
    }
    let mut text = base[..base.len() - 1].to_owned();
    text.push_str(&format!(",le=\"{le}\"}}"));
    text
}

fn escape_label_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rendering_escapes_label_values() {
        assert_eq!(escape_label_value("plain"), "plain");
        assert_eq!(escape_label_value("a\"b"), "a\\\"b");
        assert_eq!(escape_label_value("a\\b"), "a\\\\b");
        assert_eq!(escape_label_value("a\nb"), "a\\nb");
    }

    #[test]
    fn rendering_orders_rows_deterministically() {
        let mut state = MetricState::default();
        state.increment(
            "leadline_findings_total",
            &[
                ("surface", "cli"),
                ("operation", "debt"),
                ("kind", "function"),
                ("state", "resolved"),
            ],
            1,
        );
        state.increment(
            "leadline_findings_total",
            &[
                ("surface", "cli"),
                ("operation", "debt"),
                ("kind", "function"),
                ("state", "new"),
            ],
            9,
        );

        let text = render(&state);
        assert_eq!(
            text.matches("# TYPE leadline_findings_total counter")
                .count(),
            1
        );
        let new = text.find("state=\"new\"").unwrap();
        let resolved = text.find("state=\"resolved\"").unwrap();
        assert!(
            new < resolved,
            "rows must be sorted for deterministic output"
        );
    }

    #[test]
    fn render_skips_unknown_metrics_and_always_includes_build_info() {
        let mut state = MetricState::default();
        state.increment("attacker_metric", &[("a", "b")], 1);

        let text = render(&state);
        assert!(!text.contains("attacker_metric"));
        assert!(text.contains(&format!(
            "leadline_build_info{{metrics_schema=\"{}\",version=\"{}\"}} 1",
            STATE_SCHEMA_VERSION,
            env!("CARGO_PKG_VERSION")
        )));
        assert!(text.ends_with('\n'));
    }

    #[test]
    fn tampered_rows_are_pruned_and_never_rendered() {
        let mut state = MetricState::default();
        // Unknown family.
        state.counters.push(Row {
            metric: "attacker_metric".to_owned(),
            labels: BTreeMap::from([("a".to_owned(), "b".to_owned())]),
            value: 1,
        });
        // Known family, unknown label key.
        state.counters.push(Row {
            metric: "leadline_findings_total".to_owned(),
            labels: BTreeMap::from([("bad key".to_owned(), "x".to_owned())]),
            value: 1,
        });
        // Known family and keys, value outside the closed set.
        state.counters.push(Row {
            metric: "leadline_invocations_total".to_owned(),
            labels: BTreeMap::from([
                ("operation".to_owned(), "analyze".to_owned()),
                ("outcome".to_owned(), "success".to_owned()),
                ("surface".to_owned(), "/home/user/private".to_owned()),
            ]),
            value: 1,
        });
        // Known family and keys, operation outside the bounded shape.
        state.increment(
            "leadline_invocations_total",
            &[
                ("surface", "cli"),
                ("operation", "weird\"\\\n"),
                ("outcome", "success"),
            ],
            1,
        );

        let text = render(&state);
        assert!(!text.contains("attacker_metric"));
        assert!(!text.contains("bad key"));
        assert!(!text.contains("/home/user/private"));

        state.prune();
        assert!(state.counters.is_empty());
    }

    #[test]
    fn prune_caps_stored_series() {
        let mut state = MetricState::default();
        for index in 0..(MAX_SERIES + 10) {
            let operation = format!("op{index}");
            state.increment(
                "leadline_invocations_total",
                &[
                    ("surface", "cli"),
                    ("operation", &operation),
                    ("outcome", "success"),
                ],
                1,
            );
        }

        state.prune();
        assert_eq!(state.counters.len(), MAX_SERIES);
    }

    #[test]
    fn summaries_track_count_and_sum() {
        let mut state = MetricState::default();
        for value in [0.5, 0.25] {
            state.observe(
                "leadline_invocation_duration_seconds",
                &[
                    ("surface", "cli"),
                    ("operation", "check"),
                    ("outcome", "success"),
                ],
                value,
            );
        }

        let text = render(&state);
        assert!(text.contains(
            "leadline_invocation_duration_seconds_sum{operation=\"check\",outcome=\"success\",surface=\"cli\"} 0.75"
        ));
        assert!(text.contains(
            "leadline_invocation_duration_seconds_count{operation=\"check\",outcome=\"success\",surface=\"cli\"} 2"
        ));
    }

    #[test]
    fn duration_buckets_are_cumulative() {
        let mut state = MetricState::default();
        state.observe(
            "leadline_invocation_duration_seconds",
            &[
                ("surface", "cli"),
                ("operation", "check"),
                ("outcome", "success"),
            ],
            0.02,
        );

        let row = &state.summaries[0];
        // 0.02 lands in the first bound >= value (0.025 at index 2) and all higher.
        assert_eq!(row.buckets.len(), DURATION_BUCKETS.len());
        assert_eq!(row.buckets[0], 0);
        assert_eq!(row.buckets[1], 0);
        for slot in row.buckets.iter().skip(2) {
            assert_eq!(*slot, 1);
        }

        let text = render(&state);
        let base = "{operation=\"check\",outcome=\"success\",surface=\"cli\"}";
        assert!(text.contains(&format!(
            "leadline_invocation_duration_seconds_bucket{base_without} 0\n",
            base_without = with_le(base, "0.01")
        )));
        assert!(text.contains(&format!(
            "leadline_invocation_duration_seconds_bucket{base_without} 1\n",
            base_without = with_le(base, "0.025")
        )));
    }

    #[test]
    fn duration_plus_inf_equals_count() {
        let mut state = MetricState::default();
        for value in [0.001, 0.5, 60.0] {
            state.observe(
                "leadline_invocation_duration_seconds",
                &[
                    ("surface", "cli"),
                    ("operation", "check"),
                    ("outcome", "success"),
                ],
                value,
            );
        }

        // 60.0 exceeds every finite bucket, so only +Inf covers it.
        let row = &state.summaries[0];
        assert_eq!(row.count, 3);
        assert_eq!(*row.buckets.last().unwrap(), 2);

        let text = render(&state);
        let base = "{operation=\"check\",outcome=\"success\",surface=\"cli\"}";
        assert!(text.contains(&format!(
            "leadline_invocation_duration_seconds_bucket{} 3\n",
            with_le(base, "+Inf")
        )));
        assert!(text.contains(&format!(
            "leadline_invocation_duration_seconds_count{base} 3"
        )));
    }

    #[test]
    fn duration_outcome_splits_rows() {
        let mut state = MetricState::default();
        for outcome in ["success", "gate_failed"] {
            state.observe(
                "leadline_invocation_duration_seconds",
                &[
                    ("surface", "cli"),
                    ("operation", "check"),
                    ("outcome", outcome),
                ],
                0.1,
            );
        }

        assert_eq!(state.summaries.len(), 2);
        let text = render(&state);
        assert!(text.contains("outcome=\"success\""));
        assert!(text.contains("outcome=\"gate_failed\""));
        assert_eq!(
            text.matches("leadline_invocation_duration_seconds_count{")
                .count(),
            2
        );
    }

    #[test]
    fn duration_renders_histogram_shape() {
        let mut state = MetricState::default();
        state.observe(
            "leadline_invocation_duration_seconds",
            &[
                ("surface", "cli"),
                ("operation", "check"),
                ("outcome", "success"),
            ],
            0.1,
        );

        let text = render(&state);
        assert!(text.contains("# TYPE leadline_invocation_duration_seconds histogram"));
        assert!(!text.contains("# TYPE leadline_invocation_duration_seconds summary"));
        assert!(text.contains("leadline_invocation_duration_seconds_bucket{"));
        assert!(text.contains("leadline_invocation_duration_seconds_sum{"));
        assert!(text.contains("leadline_invocation_duration_seconds_count{"));
        assert!(text.contains("le=\"+Inf\""));
    }

    #[test]
    fn v1_state_is_dropped_by_read_state() {
        let directory =
            std::env::temp_dir().join(format!("leadline-telemetry-v1-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(
            directory.join("state.json"),
            br#"{"schema_version":1,"counters":[],"summaries":[{"metric":"leadline_invocation_duration_seconds","labels":{"surface":"cli","operation":"check"},"count":2,"sum":0.75}],"gauges":[]}"#,
        )
        .unwrap();

        let state = read_state(&directory);
        assert_eq!(state.schema_version, STATE_SCHEMA_VERSION);
        assert!(state.summaries.is_empty());
        assert!(state.counters.is_empty());

        std::fs::remove_dir_all(&directory).unwrap();
    }

    /// A pre-created symlink at the temporary path must be unlinked, not
    /// followed, so a hostile entry cannot redirect the write.
    #[cfg(unix)]
    #[test]
    fn write_atomic_does_not_follow_symlinks() {
        let directory =
            std::env::temp_dir().join(format!("leadline-telemetry-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let target = directory.join("target");
        std::fs::write(&target, "safe").unwrap();
        let sibling = directory.join(format!(".state.json.tmp.{}", std::process::id()));
        std::os::unix::fs::symlink(&target, &sibling).unwrap();

        write_atomic(&directory.join("state.json"), b"new").unwrap();

        assert_eq!(std::fs::read_to_string(&target).unwrap(), "safe");
        assert_eq!(
            std::fs::read_to_string(directory.join("state.json")).unwrap(),
            "new"
        );
        assert!(!sibling.exists());
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn security_and_language_breakdowns_render() {
        let mut state = MetricState::default();
        state.increment(
            "leadline_security_findings_total",
            &[
                ("surface", "cli"),
                ("operation", "check"),
                ("kind", "security"),
                ("severity", "high"),
            ],
            3,
        );
        state.increment(
            "leadline_parse_errors_total",
            &[
                ("surface", "mcp"),
                ("operation", "check"),
                ("language", "typescript"),
            ],
            2,
        );

        let text = render(&state);
        assert!(text.contains(
            "leadline_security_findings_total{kind=\"security\",operation=\"check\",severity=\"high\",surface=\"cli\"} 3"
        ));
        assert!(text.contains(
            "leadline_parse_errors_total{language=\"typescript\",operation=\"check\",surface=\"mcp\"} 2"
        ));
    }

    #[test]
    fn unknown_severity_and_language_are_pruned() {
        let mut state = MetricState::default();
        state.increment(
            "leadline_security_findings_total",
            &[
                ("surface", "cli"),
                ("operation", "check"),
                ("kind", "security"),
                ("severity", "pwned"),
            ],
            1,
        );
        state.increment(
            "leadline_parse_errors_total",
            &[
                ("surface", "cli"),
                ("operation", "check"),
                ("language", "/etc/passwd"),
            ],
            1,
        );

        let text = render(&state);
        assert!(!text.contains("pwned"));
        assert!(!text.contains("/etc/passwd"));

        state.prune();
        assert!(state.counters.is_empty());
    }

    #[test]
    fn language_label_maps_extensions_only() {
        assert_eq!(language_label("src/Main.java"), Some("java"));
        assert_eq!(language_label("app.min.MJS"), Some("javascript"));
        assert_eq!(language_label("src/index.TS"), Some("typescript"));
        assert_eq!(language_label("src/view.tsx"), Some("tsx"));
        assert_eq!(language_label("notes.md"), None);
        assert_eq!(language_label("Makefile"), None);
    }

    #[test]
    fn histogram_families_use_their_own_bucket_counts() {
        let mut state = MetricState::default();
        state.observe(
            "leadline_mcp_session_seconds",
            &[("transport", "stdio")],
            2.0,
        );
        state.observe(
            "leadline_invocation_duration_seconds",
            &[
                ("surface", "cli"),
                ("operation", "check"),
                ("outcome", "success"),
            ],
            0.4,
        );
        let text = render(&state);
        let session_buckets = text
            .lines()
            .filter(|line| line.starts_with("leadline_mcp_session_seconds_bucket"))
            .count();
        let duration_buckets = text
            .lines()
            .filter(|line| line.starts_with("leadline_invocation_duration_seconds_bucket"))
            .count();
        assert_eq!(session_buckets, 9, "{text}");
        assert_eq!(duration_buckets, 13, "{text}");
    }

    #[test]
    fn summary_rows_with_the_wrong_bucket_count_are_dropped() {
        let mut state = MetricState::default();
        state.summaries.push(SummaryRow {
            metric: "leadline_mcp_session_seconds".to_owned(),
            labels: labels_map(&[("transport", "stdio")]),
            count: 1,
            sum: 1.0,
            buckets: vec![0u64; 3],
        });
        state.prune();
        assert!(state.summaries.is_empty(), "{:?}", state.summaries);
    }

    #[test]
    fn observed_histograms_merge_into_cumulative_buckets() {
        let mut state = MetricState::default();
        state.observe(
            "leadline_mcp_session_seconds",
            &[("transport", "http")],
            2.0,
        );
        // Cumulative counts, as the sampler produces them: every bound at or
        // above the sample is incremented.
        state.observe_histogram(
            "leadline_mcp_session_seconds",
            &[("transport", "http")],
            &[1, 3, 0, 0, 0, 0, 0, 0],
            4,
            5.0,
        );
        let row = state
            .summaries
            .iter()
            .find(|row| row.metric == "leadline_mcp_session_seconds")
            .expect("row");
        assert_eq!(row.count, 5);
        assert_eq!(row.sum, 7.0);
        assert_eq!(row.buckets, vec![1, 3, 1, 1, 1, 1, 1, 1]);
    }
}
