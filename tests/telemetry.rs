//! Local metrics integration tests: opt-in recording through the CLI and MCP,
//! outcome labels, Prometheus text rendering, concurrency, and recovery from a
//! corrupt store. Every test that expects writes sets `LEADLINE_METRICS_DIR`
//! explicitly; one test proves the unset variable keeps the store untouched.

mod common;
use common::temporary_directory;
use std::path::Path;
use std::process::Command;

const VAR: &str = "LEADLINE_METRICS_DIR";

fn run_with(directory: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_leadline"))
        .env(VAR, directory)
        .args(args)
        .output()
        .unwrap()
}

fn run_without(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_leadline"))
        .env_remove(VAR)
        .args(args)
        .output()
        .unwrap()
}

fn state(directory: &Path) -> serde_json::Value {
    let bytes = std::fs::read(directory.join("state.json")).expect("state.json must exist");
    serde_json::from_slice(&bytes).expect("state.json must parse")
}

fn prom(directory: &Path) -> String {
    std::fs::read_to_string(directory.join("leadline.prom")).expect("leadline.prom must exist")
}

/// Value of one counter row, or `0` when no row matches exactly.
fn counter(directory: &Path, metric: &str, labels: &[(&str, &str)]) -> u64 {
    let state = state(directory);
    for row in state["counters"].as_array().unwrap() {
        if row["metric"] == metric && labels_match(&row["labels"], labels) {
            return row["value"].as_u64().unwrap();
        }
    }
    0
}

/// Value of one gauge row, or `None` when no row matches exactly.
fn gauge(directory: &Path, metric: &str, labels: &[(&str, &str)]) -> Option<u64> {
    let state = state(directory);
    for row in state["gauges"].as_array().unwrap() {
        if row["metric"] == metric && labels_match(&row["labels"], labels) {
            return Some(row["value"].as_u64().unwrap());
        }
    }
    None
}

fn labels_match(value: &serde_json::Value, labels: &[(&str, &str)]) -> bool {
    let row = value.as_object().unwrap();
    row.len() == labels.len()
        && labels
            .iter()
            .all(|(key, value)| row.get(*key).and_then(serde_json::Value::as_str) == Some(*value))
}

#[test]
fn records_cli_invocations_outcomes_and_findings() {
    let metrics = temporary_directory();
    let root = temporary_directory();
    let file = root.join("branch.ts");
    std::fs::write(
        &file,
        "function branch(x: number) { if (x > 1) { if (x > 2) { return 2; } } return 1; }\n",
    )
    .unwrap();

    let analyze = run_with(&metrics, &["analyze", file.to_str().unwrap(), "--json"]);
    assert!(analyze.status.success());
    let failed = run_with(
        &metrics,
        &[
            "check",
            file.to_str().unwrap(),
            "--cognitive",
            "1",
            "--json",
        ],
    );
    assert_eq!(failed.status.code(), Some(1));

    assert_eq!(
        counter(
            &metrics,
            "leadline_invocations_total",
            &[
                ("surface", "cli"),
                ("operation", "analyze"),
                ("outcome", "success")
            ]
        ),
        1
    );
    assert_eq!(
        counter(
            &metrics,
            "leadline_invocations_total",
            &[
                ("surface", "cli"),
                ("operation", "check"),
                ("outcome", "gate_failed")
            ]
        ),
        1
    );
    assert_eq!(
        counter(
            &metrics,
            "leadline_findings_total",
            &[
                ("surface", "cli"),
                ("operation", "check"),
                ("kind", "function"),
                ("state", "violation"),
            ]
        ),
        1
    );

    // Rendered text keeps one metadata pair per family and ends with a newline.
    let text = prom(&metrics);
    assert!(text.contains(
        "leadline_invocations_total{operation=\"analyze\",outcome=\"success\",surface=\"cli\"} 1"
    ));
    assert_eq!(
        text.matches("# TYPE leadline_invocations_total counter")
            .count(),
        1
    );
    assert!(text.contains(&format!(
        "leadline_build_info{{metrics_schema=\"3\",version=\"{}\"}} 1",
        env!("CARGO_PKG_VERSION")
    )));
    assert!(text.ends_with('\n'));

    // Durations are summarized; no fixture path, file name, or source text
    // appears anywhere in the store.
    let stored = state(&metrics);
    assert!(
        stored["summaries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["metric"] == "leadline_invocation_duration_seconds")
    );
    let raw = format!("{stored}{text}");
    assert!(!raw.contains(root.to_str().unwrap()));
    assert!(!raw.contains("branch.ts"));
    assert!(!raw.contains("function branch"));

    std::fs::remove_dir_all(metrics).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn records_debt_flow_and_standing_counts() {
    if !common::tool_available("git") {
        return;
    }
    let metrics = temporary_directory();
    let root = temporary_directory();
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: boolean) { return x; }\n",
    )
    .unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "clean", "-q"]);
    std::fs::write(
        root.join("leadline.toml"),
        "[thresholds.function]\ncognitive = 1\n",
    )
    .unwrap();
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { if (x > 1) { if (x > 2) { return 2; } } return 1; }\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .env(VAR, &metrics)
        .args(["debt", "--base", "HEAD"])
        .output()
        .unwrap();
    assert!(output.status.success());

    assert!(
        counter(
            &metrics,
            "leadline_findings_total",
            &[
                ("surface", "cli"),
                ("operation", "debt"),
                ("kind", "function"),
                ("state", "new"),
            ]
        ) >= 1
    );
    // The standing count is a gauge: the latest run's `existing` number.
    assert_eq!(
        gauge(
            &metrics,
            "leadline_debt_functions",
            &[("surface", "cli"), ("state", "existing")]
        ),
        Some(0)
    );

    std::fs::remove_dir_all(metrics).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_base_gate_records_findings_once() {
    if !common::tool_available("git") {
        return;
    }
    let metrics = temporary_directory();
    let root = temporary_directory();
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: boolean) { return x; }\n",
    )
    .unwrap();
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.invalid"]);
    git(&root, &["config", "user.name", "Test"]);
    git(&root, &["add", "."]);
    git(&root, &["commit", "-m", "clean", "-q"]);
    std::fs::write(
        root.join("calc.ts"),
        "function calc(x: number) { if (x > 1) { if (x > 2) { return 2; } } return 1; }\n",
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .env(VAR, &metrics)
        .args(["check", "--base", "HEAD", "--cognitive", "1"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        counter(
            &metrics,
            "leadline_invocations_total",
            &[
                ("surface", "cli"),
                ("operation", "check"),
                ("outcome", "gate_failed")
            ]
        ),
        1
    );
    assert_eq!(
        counter(
            &metrics,
            "leadline_findings_total",
            &[
                ("surface", "cli"),
                ("operation", "check"),
                ("kind", "function"),
                ("state", "violation"),
            ]
        ),
        1
    );

    std::fs::remove_dir_all(metrics).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn check_parse_errors_are_recorded_as_findings() {
    let metrics = temporary_directory();
    let root = temporary_directory();
    let file = root.join("broken.ts");
    std::fs::write(&file, "function broken( { return 1; }\n").unwrap();

    let output = run_with(
        &metrics,
        &[
            "check",
            file.to_str().unwrap(),
            "--cyclomatic",
            "100",
            "--json",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        counter(
            &metrics,
            "leadline_findings_total",
            &[
                ("surface", "cli"),
                ("operation", "check"),
                ("kind", "parse_error"),
                ("state", "violation"),
            ]
        ) >= 1,
        "a parse-error-only gate failure must record a finding"
    );
    // The broken function never reaches the threshold, so the gate fails only
    // because of the parse error.
    assert_eq!(
        counter(
            &metrics,
            "leadline_findings_total",
            &[
                ("surface", "cli"),
                ("operation", "check"),
                ("kind", "function"),
                ("state", "violation"),
            ]
        ),
        0
    );

    std::fs::remove_dir_all(metrics).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn metrics_stay_disabled_without_the_variable() {
    let metrics = temporary_directory();
    let first = run_with(&metrics, &["version"]);
    assert!(first.status.success());
    let state_before = std::fs::read(metrics.join("state.json")).unwrap();
    let prom_before = std::fs::read(metrics.join("leadline.prom")).unwrap();

    let second = run_without(&["version"]);
    assert!(second.status.success());
    assert_eq!(
        std::fs::read(metrics.join("state.json")).unwrap(),
        state_before
    );
    assert_eq!(
        std::fs::read(metrics.join("leadline.prom")).unwrap(),
        prom_before
    );

    std::fs::remove_dir_all(metrics).unwrap();
}

#[test]
fn corrupt_state_recovers_without_changing_the_command() {
    let metrics = temporary_directory();
    std::fs::write(metrics.join("state.json"), b"{not json").unwrap();

    let output = run_with(&metrics, &["version"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        concat!("leadline ", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!(state(&metrics)["schema_version"], 3);
    assert_eq!(
        counter(
            &metrics,
            "leadline_invocations_total",
            &[
                ("surface", "cli"),
                ("operation", "version"),
                ("outcome", "success")
            ]
        ),
        1
    );

    // A schema mismatch also resets instead of failing, and counts restart.
    std::fs::write(
        metrics.join("state.json"),
        b"{\"schema_version\":999,\"counters\":[],\"summaries\":[],\"gauges\":[]}",
    )
    .unwrap();
    let output = run_with(&metrics, &["version"]);
    assert!(output.status.success());
    assert_eq!(state(&metrics)["schema_version"], 3);
    assert_eq!(
        counter(
            &metrics,
            "leadline_invocations_total",
            &[
                ("surface", "cli"),
                ("operation", "version"),
                ("outcome", "success")
            ]
        ),
        1
    );

    std::fs::remove_dir_all(metrics).unwrap();
}

#[test]
fn usage_errors_use_bounded_labels() {
    let metrics = temporary_directory();
    let output = run_with(&metrics, &["definitely-not-a-command"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        counter(
            &metrics,
            "leadline_invocations_total",
            &[
                ("surface", "cli"),
                ("operation", "other"),
                ("outcome", "usage_error")
            ]
        ),
        1
    );

    std::fs::remove_dir_all(metrics).unwrap();
}

#[test]
fn concurrent_invocations_are_not_lost() {
    let metrics = temporary_directory();
    let handles: Vec<_> = (0..6)
        .map(|_| {
            let directory = metrics.clone();
            std::thread::spawn(move || {
                let output = run_with(&directory, &["version"]);
                assert!(output.status.success());
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }
    assert_eq!(
        counter(
            &metrics,
            "leadline_invocations_total",
            &[
                ("surface", "cli"),
                ("operation", "version"),
                ("outcome", "success")
            ]
        ),
        6
    );

    std::fs::remove_dir_all(metrics).unwrap();
}

#[test]
fn mcp_tool_calls_and_gate_failures_are_recorded() {
    use std::io::Write as _;
    use std::process::Stdio;

    let metrics = temporary_directory();
    let root = temporary_directory();
    std::fs::write(
        root.join("branch.ts"),
        "function branch(x: number) { if (x > 1) { if (x > 2) { return 2; } } return 1; }\n",
    )
    .unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_leadline"))
        .arg("mcp")
        .env(VAR, &metrics)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let requests = [
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "explain_metric", "arguments": { "metric": "cognitive" } }
        }),
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "check",
                "arguments": { "path": root, "thresholds": { "cognitive": 1 } }
            }
        }),
    ];
    let mut stdin = child.stdin.take().unwrap();
    for request in &requests {
        writeln!(stdin, "{request}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let responses: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(responses.len(), 2);

    assert_eq!(
        counter(
            &metrics,
            "leadline_invocations_total",
            &[
                ("surface", "mcp"),
                ("operation", "explain_metric"),
                ("outcome", "success")
            ]
        ),
        1
    );
    assert_eq!(
        counter(
            &metrics,
            "leadline_invocations_total",
            &[
                ("surface", "mcp"),
                ("operation", "check"),
                ("outcome", "gate_failed")
            ]
        ),
        1
    );
    // MCP check calls record findings exactly like CLI ones.
    assert_eq!(
        counter(
            &metrics,
            "leadline_findings_total",
            &[
                ("surface", "mcp"),
                ("operation", "check"),
                ("kind", "function"),
                ("state", "violation"),
            ]
        ),
        1
    );

    std::fs::remove_dir_all(metrics).unwrap();
    std::fs::remove_dir_all(root).unwrap();
}

fn git(directory: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(directory)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?} failed: {output:?}");
}
