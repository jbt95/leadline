//! Hotspot analysis tests: source metrics x Git history join.

mod common;

use leadline::core::{
    AnalysisReport, FileAnalysis, METRIC_PROFILE, MetricSpecs, OUTPUT_SCHEMA_VERSION,
};
use leadline::history::{FileHistory, HISTORY_SCHEMA_VERSION, HistoryReport, HistoryWindow};
use leadline::hotspots::{HOTSPOT_MODEL, build};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn analysis(files: Vec<FileAnalysis>) -> AnalysisReport {
    AnalysisReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: "test",
        metric_specs: MetricSpecs::default(),
        files,
    }
}

fn analyze(path: &str, source: &str) -> FileAnalysis {
    leadline::analyze_source(path, source.as_bytes()).unwrap()
}

fn history_file(path: &str, changes_30d: u64, changes_90d: u64, changes_365d: u64) -> FileHistory {
    FileHistory {
        path: path.to_owned(),
        commits: changes_365d,
        changes_30d,
        changes_90d,
        changes_365d,
        lines_added: 40,
        lines_deleted: 7,
        days_since_last_change: 3,
        age_days: 800,
        contributors: 4,
        recent_contributors: 2,
    }
}

fn history(files: Vec<FileHistory>, available: bool) -> HistoryReport {
    HistoryReport {
        schema_version: HISTORY_SCHEMA_VERSION,
        analyzer_version: "test",
        available,
        head_commit: available.then(|| "abc123".to_owned()),
        head_timestamp: available.then_some(1_775_001_600),
        reference: "head-commit-time",
        files,
    }
}

const LOW: &str = "function low(x: number) { if (x) { return 1; } return 0; }";
const HIGH: &str = "function high(x: number) { if (x) { if (x > 1) { return 1; } } return 0; }";

/// Cognitive complexity of the two fixtures, derived from the documented rule
/// that each `if`, loop, `catch`, `switch`, and ternary adds `1 + current
/// nesting` with no flat base: `LOW` has one `if` at nesting 0, `HIGH` adds a
/// second `if` at nesting 1. Pinned as literals so a test asserts the metric
/// instead of re-deriving it with a copy of the production fold.
const LOW_COGNITIVE: u32 = 1;
const HIGH_COGNITIVE: u32 = 3;

#[test]
fn build_joins_source_and_history_dimensions() {
    let report = analysis(vec![analyze("src/a.ts", HIGH)]);
    let history = history(vec![history_file("src/a.ts", 2, 10, 40)], true);
    let hotspots = build(&report, &history, HistoryWindow::Days90, 10);

    assert!(hotspots.git_available);
    assert_eq!(hotspots.window, "90d");
    assert_eq!(hotspots.model, HOTSPOT_MODEL);
    assert_eq!(hotspots.files_analyzed, 1);
    assert_eq!(hotspots.head_commit.as_deref(), Some("abc123"));
    assert!(!hotspots.truncated);
    let hotspot = &hotspots.hotspots[0];
    assert_eq!(hotspot.path, "src/a.ts");
    assert_eq!(hotspot.functions, 1);
    assert_eq!(hotspot.max_cognitive, HIGH_COGNITIVE);
    assert_eq!(hotspot.changes_30d, Some(2));
    assert_eq!(hotspot.changes_90d, Some(10));
    assert_eq!(hotspot.changes_365d, Some(40));
    assert_eq!(hotspot.commits, Some(40));
    assert_eq!(hotspot.lines_added, Some(40));
    assert_eq!(hotspot.lines_deleted, Some(7));
    assert_eq!(hotspot.days_since_last_change, Some(3));
    assert_eq!(hotspot.contributors, Some(4));
    assert_eq!(hotspot.recent_contributors, Some(2));
    assert_eq!(
        hotspot.score,
        Some(u64::from(HIGH_COGNITIVE) * 10),
        "score is max cognitive times changes in the selected window"
    );
}

#[test]
fn ranking_uses_complexity_times_churn() {
    let report = analysis(vec![
        analyze("src/busy.ts", LOW),
        analyze("src/complex.ts", HIGH),
    ]);
    let history = history(
        vec![
            history_file("src/busy.ts", 20, 20, 20),
            history_file("src/complex.ts", 3, 3, 3),
        ],
        true,
    );
    let hotspots = build(&report, &history, HistoryWindow::Days90, 10);
    let ranked: Vec<&str> = hotspots
        .hotspots
        .iter()
        .map(|hotspot| hotspot.path.as_str())
        .collect();
    let busy_score = u64::from(LOW_COGNITIVE) * 20;
    let complex_score = u64::from(HIGH_COGNITIVE) * 3;
    assert!(
        busy_score > complex_score,
        "fixture must exercise a churn-led ranking"
    );
    assert_eq!(ranked, ["src/busy.ts", "src/complex.ts"]);
}

#[test]
fn window_selection_changes_the_score() {
    let report = analysis(vec![analyze("src/a.ts", HIGH)]);
    let history = history(vec![history_file("src/a.ts", 1, 10, 100)], true);
    let cognitive = u64::from(HIGH_COGNITIVE);

    let recent = build(&report, &history, HistoryWindow::Days30, 10);
    assert_eq!(recent.window, "30d");
    assert_eq!(recent.hotspots[0].score, Some(cognitive));

    let yearly = build(&report, &history, HistoryWindow::Days365, 10);
    assert_eq!(yearly.window, "365d");
    assert_eq!(yearly.hotspots[0].score, Some(cognitive * 100));
}

#[test]
fn missing_git_history_falls_back_to_complexity_ranking() {
    let report = analysis(vec![
        analyze("src/busy.ts", LOW),
        analyze("src/complex.ts", HIGH),
    ]);
    let hotspots = build(
        &report,
        &history(Vec::new(), false),
        HistoryWindow::Days90,
        10,
    );
    assert!(!hotspots.git_available);
    assert!(
        hotspots
            .hotspots
            .iter()
            .all(|hotspot| hotspot.score.is_none())
    );
    assert!(
        hotspots
            .hotspots
            .iter()
            .all(|hotspot| hotspot.changes_90d.is_none())
    );
    let ranked: Vec<&str> = hotspots
        .hotspots
        .iter()
        .map(|hotspot| hotspot.path.as_str())
        .collect();
    assert_eq!(ranked, ["src/complex.ts", "src/busy.ts"]);
}

#[test]
fn coverage_and_crap_are_carried_into_hotspots() {
    let mut file = analyze("src/a.ts", HIGH);
    for function in &mut file.functions {
        leadline::core::apply_coverage(function, Some(0.5));
    }
    let report = analysis(vec![file]);
    let history = history(vec![history_file("src/a.ts", 1, 5, 5)], true);
    let hotspots = build(&report, &history, HistoryWindow::Days90, 10);
    let hotspot = &hotspots.hotspots[0];
    assert_eq!(hotspot.coverage, Some(0.5));
    assert_eq!(hotspot.functions_with_coverage, 1);
    let expected_crap = hotspot.max_cyclomatic.pow(2) as f64 * (1.0 - 0.5_f64).powi(3)
        + f64::from(hotspot.max_cyclomatic);
    assert_eq!(hotspot.max_crap, Some(expected_crap));
}

#[test]
fn limit_truncates_ranked_output() {
    let report = analysis(vec![
        analyze("src/a.ts", LOW),
        analyze("src/b.ts", LOW),
        analyze("src/c.ts", LOW),
    ]);
    let history = history(
        vec![
            history_file("src/a.ts", 1, 1, 1),
            history_file("src/b.ts", 2, 2, 2),
            history_file("src/c.ts", 3, 3, 3),
        ],
        true,
    );
    let truncated = build(&report, &history, HistoryWindow::Days90, 2);
    assert_eq!(truncated.hotspots.len(), 2);
    assert!(truncated.truncated);
    let ranked: Vec<&str> = truncated
        .hotspots
        .iter()
        .map(|hotspot| hotspot.path.as_str())
        .collect();
    assert_eq!(ranked, ["src/c.ts", "src/b.ts"]);

    let full = build(&report, &history, HistoryWindow::Days90, 5);
    assert_eq!(full.hotspots.len(), 3);
    assert!(!full.truncated);
}

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn temporary_directory(prefix: &str) -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("{prefix}-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn git_repo_with_source(prefix: &str) -> PathBuf {
    let root = temporary_directory(prefix);
    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "a@example.invalid"]);
    git(&root, &["config", "user.name", "Author A"]);
    std::fs::write(root.join("a.ts"), HIGH).unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "one"]);
    root
}

#[test]
fn cli_hotspots_json_reports_ranked_dimensions() {
    let root = git_repo_with_source("leadline-hotspots");
    let output = common::leadline()
        .current_dir(&root)
        .args(["hotspots", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["model"], HOTSPOT_MODEL);
    assert_eq!(value["window"], "90d");
    assert_eq!(value["git_available"], true);
    let hotspot = &value["hotspots"][0];
    assert_eq!(hotspot["path"], "a.ts");
    assert_eq!(hotspot["changes_90d"], 1);
    assert_eq!(
        hotspot["score"].as_u64().unwrap(),
        hotspot["max_cognitive"].as_u64().unwrap()
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_hotspots_without_git_still_answers() {
    let root = temporary_directory("leadline-hotspots-nogit");
    std::fs::write(root.join("a.ts"), HIGH).unwrap();
    let output = common::leadline()
        .current_dir(&root)
        .args(["hotspots", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["git_available"], false);
    assert!(value["hotspots"][0]["score"].is_null());
    assert!(value["hotspots"][0]["changes_90d"].is_null());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_hotspots_rejects_unknown_windows() {
    let root = git_repo_with_source("leadline-hotspots-window");
    let output = common::leadline()
        .current_dir(&root)
        .args(["hotspots", "--since", "7d"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("30d"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_hotspots_agent_json_is_compact() {
    let root = git_repo_with_source("leadline-hotspots-agent");
    let output = common::leadline()
        .current_dir(&root)
        .args(["hotspots", "--format", "agent-json", "--limit", "1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["window"], "90d");
    assert_eq!(value["summary"]["hotspots"], 1);
    let hotspot = &value["hotspots"][0];
    assert_eq!(hotspot["path"], "a.ts");
    assert!(hotspot.get("score").is_some());
    assert!(hotspot.get("changes").is_some());
    assert!(hotspot.get("language").is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_hotspots_single_file_invocation() {
    let root = git_repo_with_source("leadline-hotspots-file");
    let output = common::leadline()
        .current_dir(&root)
        .args(["hotspots", "a.ts", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["hotspots"][0]["path"], "a.ts");
    assert_eq!(value["hotspots"][0]["changes_90d"], 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_hotspots_merges_coverage() {
    let root = git_repo_with_source("leadline-hotspots-coverage");
    std::fs::write(
        root.join("coverage.info"),
        "TN:\nSF:a.ts\nDA:1,1\nend_of_record\n",
    )
    .unwrap();
    let output = common::leadline()
        .current_dir(&root)
        .args(["hotspots", "a.ts", "--lcov", "coverage.info", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let hotspot = &value["hotspots"][0];
    assert_eq!(hotspot["coverage"], 1.0);
    assert!(hotspot["max_crap"].is_number());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_hotspots_terminal_lists_dimensions() {
    let root = git_repo_with_source("leadline-hotspots-terminal");
    let output = common::leadline()
        .current_dir(&root)
        .args(["hotspots", "--limit", "1"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Top engineering hotspots (window: 90d)"));
    assert!(stdout.contains("a.ts"));
    assert!(stdout.contains("Hotspot score"));
    assert!(stdout.contains("Changes / 90 days"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_hotspots_json_is_deterministic() {
    let root = git_repo_with_source("leadline-hotspots-determinism");
    let args = ["hotspots", "--json"];
    let first = common::leadline()
        .current_dir(&root)
        .args(args)
        .output()
        .unwrap();
    let second = common::leadline()
        .current_dir(&root)
        .args(args)
        .output()
        .unwrap();
    assert!(first.status.success());
    assert_eq!(first.stdout, second.stdout);
    std::fs::remove_dir_all(root).unwrap();
}
