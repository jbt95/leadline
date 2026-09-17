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
//! responses, and a concurrent invocation that cannot take the store lock
//! within a short budget drops its sample instead of waiting.
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
pub const STATE_SCHEMA_VERSION: u32 = 1;

const STATE_FILE_NAME: &str = "state.json";
const PROM_FILE_NAME: &str = "leadline.prom";
const LOCK_FILE_NAME: &str = "state.lock";
const LOCK_ATTEMPTS: u32 = 8;
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(10);

/// One metric family the store knows how to render. Unknown families read
/// from a tampered store are dropped rather than rendered.
struct Family {
    name: &'static str,
    help: &'static str,
    kind: Kind,
    /// Exact label key set; a row with any other key is dropped.
    label_keys: &'static [&'static str],
}

#[derive(Clone, Copy)]
enum Kind {
    Counter,
    Gauge,
    Summary,
}

const FAMILIES: &[Family] = &[
    Family {
        name: "leadline_invocations_total",
        help: "Leadline invocations by surface, operation, and outcome.",
        kind: Kind::Counter,
        label_keys: &["surface", "operation", "outcome"],
    },
    Family {
        name: "leadline_invocation_duration_seconds",
        help: "Wall-clock duration of leadline invocations in seconds.",
        kind: Kind::Summary,
        label_keys: &["surface", "operation"],
    },
    Family {
        name: "leadline_findings_total",
        help: "Findings reported by gating commands, by operation, kind, and state.",
        kind: Kind::Counter,
        label_keys: &["surface", "operation", "kind", "state"],
    },
    Family {
        name: "leadline_debt_functions",
        help: "Standing function debt in the most recent debt run, by state.",
        kind: Kind::Gauge,
        label_keys: &["surface", "state"],
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
];

/// Hard ceiling per series list; real usage stays in the low hundreds.
const MAX_SERIES: usize = 4096;

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

    /// Records one observation for a summary family (count plus sum).
    fn observe(&mut self, metric: &str, labels: &[(&str, &str)], value: f64) {
        let labels = labels_map(labels);
        if let Some(row) = self
            .summaries
            .iter_mut()
            .find(|row| row.metric == metric && row.labels == labels)
        {
            row.count = row.count.saturating_add(1);
            row.sum += value;
        } else {
            self.summaries.push(SummaryRow {
                metric: metric.to_owned(),
                labels,
                count: 1,
                sum: value,
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
    family_for(&row.metric).is_some_and(|family| valid_labels(family, &row.labels))
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
            &[("surface", surface), ("operation", operation)],
            duration.as_secs_f64(),
        );
    });
}

/// Records the `check` gate's function, parse-error, and scanner finding
/// counts. Families with no input contribute zero and never create a row.
pub fn record_check_findings(
    surface: &str,
    functions: u64,
    parse_errors: u64,
    security: u64,
    vulnerabilities: u64,
    sql: u64,
) {
    mutate(|state| {
        for (kind, count) in [
            ("function", functions),
            ("parse_error", parse_errors),
            ("security", security),
            ("vulnerability", vulnerabilities),
            ("sql", sql),
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
    });
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
    let mut attempts = 0;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(_) if attempts < LOCK_ATTEMPTS => {
                attempts += 1;
                std::thread::sleep(LOCK_RETRY_DELAY);
            }
            Err(_) => return Err(std::io::Error::other("metrics store lock is busy")),
        }
    }
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
                    .filter(|row| row.metric == family.name && valid_labels(family, &row.labels))
                    .collect();
                if rows.is_empty() {
                    continue;
                }
                rows.sort_by(|left, right| left.labels.cmp(&right.labels));
                output.push_str(&format!(
                    "# HELP {} {}\n# TYPE {} summary\n",
                    family.name, family.help, family.name
                ));
                for row in rows {
                    let labels = labels_text(&row.labels);
                    output.push_str(&format!(
                        "{}_sum{} {}\n{}_count{} {}\n",
                        family.name, labels, row.sum, family.name, labels, row.count
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
            "leadline_build_info{{metrics_schema=\"1\",version=\"{}\"}} 1",
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
                &[("surface", "cli"), ("operation", "check")],
                value,
            );
        }

        let text = render(&state);
        assert!(text.contains(
            "leadline_invocation_duration_seconds_sum{operation=\"check\",surface=\"cli\"} 0.75"
        ));
        assert!(text.contains(
            "leadline_invocation_duration_seconds_count{operation=\"check\",surface=\"cli\"} 2"
        ));
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
}
