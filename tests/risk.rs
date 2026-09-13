use leadline::config::Severity;
use leadline::core::{
    AnalysisReport, FileAnalysis, FunctionAnalysis, FunctionKind, FunctionMetrics, METRIC_PROFILE,
    MetricSpecs, OUTPUT_SCHEMA_VERSION,
};
use leadline::graph::{
    DEPENDENCY_SCHEMA_VERSION, DependencyEdge, DependencyFile, DependencyReport,
};
use leadline::history::{FileHistory, HISTORY_SCHEMA_VERSION, HistoryReport, HistoryWindow};
use leadline::ownership::{OwnershipMode, OwnershipReport, build as build_ownership};
use leadline::policy::{PolicyReport, PolicyStatus, PolicyViolation, evaluate};
use leadline::risk::{COMPONENT_WEIGHTS, RISK_MODEL, RiskReport, build};

fn metrics(cognitive: u32, cyclomatic: u32, crap: Option<f64>) -> FunctionMetrics {
    FunctionMetrics {
        loc: 10,
        logical_loc: 5,
        function_length: 10,
        parameters: 0,
        max_nesting: 1,
        cyclomatic,
        cognitive,
        halstead_n1: 0,
        halstead_n2: 0,
        halstead_total_operators: 0,
        halstead_total_operands: 0,
        halstead_vocabulary: 0,
        halstead_length: 0,
        halstead_volume: 0.0,
        halstead_difficulty: 0.0,
        halstead_effort: 0.0,
        maintainability_index: 50.0,
        coverage: crap.map(|_| 0.0),
        crap,
    }
}

fn file_with_metrics(
    path: &str,
    cognitive: u32,
    cyclomatic: u32,
    crap: Option<f64>,
) -> FileAnalysis {
    FileAnalysis {
        path: path.to_owned(),
        language: leadline::core::Language::TypeScript,
        functions: vec![FunctionAnalysis {
            name: "f".to_owned(),
            id: "f".to_owned(),
            kind: FunctionKind::Function,
            start_line: 1,
            end_line: 10,
            start_byte: 0,
            end_byte: 100,
            metrics: metrics(cognitive, cyclomatic, crap),
            contributions: vec![],
            source_fingerprint: 0,
        }],
        parse_errors: vec![],
    }
}

fn analysis(files: Vec<FileAnalysis>) -> AnalysisReport {
    AnalysisReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: "test",
        metric_specs: MetricSpecs::default(),
        files,
    }
}

fn history_file(
    path: &str,
    changes_30d: u64,
    changes_90d: u64,
    changes_365d: u64,
    contributors: u64,
) -> FileHistory {
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
        contributors,
        recent_contributors: 1,
    }
}

fn history(files: Vec<FileHistory>) -> HistoryReport {
    HistoryReport {
        schema_version: HISTORY_SCHEMA_VERSION,
        analyzer_version: "test",
        available: true,
        reference: "head-commit-time",
        head_commit: Some("abc123".to_owned()),
        head_timestamp: Some(1_775_001_600),
        files,
    }
}

fn ownership(path: &str, identities: &[(&str, u64)]) -> OwnershipReport {
    let touches = leadline::history::FileTouches {
        path: path.to_owned(),
        identities: identities
            .iter()
            .map(|(identity, count)| ((*identity).to_owned(), *count))
            .collect(),
    };
    build_ownership(&[touches], &[path.to_owned()], OwnershipMode::AggregateOnly)
}

fn empty_ownership() -> OwnershipReport {
    OwnershipReport {
        files: Vec::new(),
        modules: Vec::new(),
    }
}

fn empty_policy(graph: &DependencyReport) -> PolicyReport {
    evaluate(graph, &[])
}

/// Three-file graph where `src/a.ts` has exactly one dependent out of two
/// other files, so its blast radius is 1/2 = 50%.
fn graph_for_a() -> DependencyReport {
    DependencyReport {
        schema_version: DEPENDENCY_SCHEMA_VERSION,
        metric_profile: "default",
        analyzer_version: "test",
        files: vec![
            DependencyFile {
                path: "src/a.ts".to_owned(),
                fan_in: 1,
                fan_out: 0,
            },
            DependencyFile {
                path: "src/b.ts".to_owned(),
                fan_in: 0,
                fan_out: 1,
            },
            DependencyFile {
                path: "src/c.ts".to_owned(),
                fan_in: 0,
                fan_out: 0,
            },
        ],
        edges: vec![DependencyEdge {
            source: "src/b.ts".to_owned(),
            target: "src/a.ts".to_owned(),
            kind: "import",
            confidence: "high",
        }],
        unresolved: vec![],
        cycles: vec![],
    }
}

fn build_full(
    analysis: &AnalysisReport,
    history: &HistoryReport,
    graph: &DependencyReport,
    ownership: &OwnershipReport,
    policy: &PolicyReport,
    window: HistoryWindow,
) -> RiskReport {
    build(analysis, history, graph, ownership, policy, window)
}

#[test]
fn score_identity_sums_known_weighted_components() {
    // Fixture: max_cognitive 30, max_cyclomatic 10, max_crap Some(30.0),
    // changes_90d Some(20), concentration 80.0 (8/2 touches), blast 50%,
    // no policy violations (policy Some(0.0)).
    // complexity = 100.0, crap = 100.0, churn = 100.0, impact = 50.0,
    // ownership = 80.0, policy = 0.0.
    // score == (20*100 + 15*100 + 20*100 + 20*50 + 10*80 + 15*0) / 100 == 73.0
    let analysis = analysis(vec![file_with_metrics("src/a.ts", 30, 10, Some(30.0))]);
    let history = history(vec![history_file("src/a.ts", 5, 20, 40, 2)]);
    let graph = graph_for_a();
    let ownership = ownership("src/a.ts", &[("a", 8), ("b", 2)]);
    let policy = empty_policy(&graph);
    let report = build_full(
        &analysis,
        &history,
        &graph,
        &ownership,
        &policy,
        HistoryWindow::Days90,
    );
    assert_eq!(report.model, RISK_MODEL);
    assert_eq!(report.risks.len(), 1);
    let row = &report.risks[0];
    assert_eq!(row.components.complexity, Some(100.0));
    assert_eq!(row.components.crap, Some(100.0));
    assert_eq!(row.components.churn, Some(100.0));
    assert_eq!(row.components.impact, Some(50.0));
    assert_eq!(row.components.ownership, Some(80.0));
    assert_eq!(row.components.policy, Some(0.0));
    assert_eq!(row.raw.concentration_percent, Some(80.0));
    assert!((row.score - 73.0).abs() < 1e-9);
}

#[test]
fn null_components_renormalize_instead_of_zeroing() {
    // Same fixture but max_crap None, no history row, and no touches:
    // known = complexity (20) + impact (20) + policy (15);
    // score == (20*100 + 20*50 + 15*0) / 55.
    let analysis = analysis(vec![file_with_metrics("src/a.ts", 30, 10, None)]);
    let history = history(vec![]);
    let graph = graph_for_a();
    let ownership = empty_ownership();
    let policy = empty_policy(&graph);
    let report = build_full(
        &analysis,
        &history,
        &graph,
        &ownership,
        &policy,
        HistoryWindow::Days90,
    );
    assert_eq!(report.risks.len(), 1);
    let row = &report.risks[0];
    assert_eq!(row.components.complexity, Some(100.0));
    assert_eq!(row.components.crap, None);
    assert_eq!(row.components.churn, None);
    assert_eq!(row.components.ownership, None);
    assert_eq!(row.components.impact, Some(50.0));
    assert_eq!(row.components.policy, Some(0.0));
    assert!((row.score - 3000.0 / 55.0).abs() < 1e-9);
}

#[test]
fn caps_saturate_at_100() {
    let analysis = analysis(vec![file_with_metrics("src/a.ts", 300, 200, Some(300.0))]);
    let history = history(vec![history_file("src/a.ts", 200, 200, 200, 1)]);
    let graph = DependencyReport {
        schema_version: DEPENDENCY_SCHEMA_VERSION,
        metric_profile: "default",
        analyzer_version: "test",
        files: vec![
            DependencyFile {
                path: "src/a.ts".to_owned(),
                fan_in: 1,
                fan_out: 0,
            },
            DependencyFile {
                path: "src/b.ts".to_owned(),
                fan_in: 0,
                fan_out: 1,
            },
        ],
        edges: vec![DependencyEdge {
            source: "src/b.ts".to_owned(),
            target: "src/a.ts".to_owned(),
            kind: "import",
            confidence: "high",
        }],
        unresolved: vec![],
        cycles: vec![],
    };
    let ownership = ownership("src/a.ts", &[("a", 10)]);
    let policy = PolicyReport {
        schema_version: 1,
        analyzer_version: "test",
        rules: 1,
        info: 0,
        warning: 0,
        error: 1,
        violations: vec![PolicyViolation {
            rule: "domain-no-ui".to_owned(),
            severity: Severity::Error,
            source: "src/a.ts".to_owned(),
            target: "src/ui/b.ts".to_owned(),
            status: None,
        }],
    };
    let report = build_full(
        &analysis,
        &history,
        &graph,
        &ownership,
        &policy,
        HistoryWindow::Days90,
    );
    let row = &report.risks[0];
    assert_eq!(row.components.complexity, Some(100.0));
    assert_eq!(row.components.crap, Some(100.0));
    assert_eq!(row.components.churn, Some(100.0));
    assert_eq!(row.components.impact, Some(100.0));
    assert_eq!(row.components.ownership, Some(100.0));
    assert_eq!(row.components.policy, Some(100.0));
    assert_eq!(row.raw.policy_severity, Some("error"));
    assert!((row.score - 100.0).abs() < 1e-9);
    assert_eq!(row.raw.blast_radius_percent, 100.0);
}

#[test]
fn policy_uses_highest_unresolved_source_severity() {
    let path = "src/a.ts";
    let analysis = analysis(vec![file_with_metrics(path, 10, 5, None)]);
    let history = history(vec![]);
    let graph = graph_for_a();
    let ownership = empty_ownership();
    let policy = PolicyReport {
        schema_version: 1,
        analyzer_version: "test",
        rules: 1,
        info: 0,
        warning: 1,
        error: 1,
        violations: vec![
            PolicyViolation {
                rule: "r".to_owned(),
                severity: Severity::Error,
                source: path.to_owned(),
                target: "src/ui/fixed.ts".to_owned(),
                status: Some(PolicyStatus::Resolved),
            },
            PolicyViolation {
                rule: "r".to_owned(),
                severity: Severity::Warning,
                source: path.to_owned(),
                target: "src/ui/live.ts".to_owned(),
                status: None,
            },
        ],
    };
    let report = build_full(
        &analysis,
        &history,
        &graph,
        &ownership,
        &policy,
        HistoryWindow::Days90,
    );
    let row = &report.risks[0];
    assert_eq!(row.components.policy, Some(60.0));
    assert_eq!(row.raw.policy_severity, Some("warning"));
}

#[test]
fn ranking_orders_by_score_then_path() {
    // src/high.ts outranks the tied pair; the tie breaks by path ascending.
    let analysis = analysis(vec![
        file_with_metrics("src/b.ts", 30, 10, None),
        file_with_metrics("src/a.ts", 30, 10, None),
        file_with_metrics("src/high.ts", 30, 10, None),
    ]);
    let graph = DependencyReport {
        schema_version: DEPENDENCY_SCHEMA_VERSION,
        metric_profile: "default",
        analyzer_version: "test",
        files: vec![
            DependencyFile {
                path: "src/a.ts".to_owned(),
                fan_in: 0,
                fan_out: 0,
            },
            DependencyFile {
                path: "src/b.ts".to_owned(),
                fan_in: 0,
                fan_out: 0,
            },
            DependencyFile {
                path: "src/high.ts".to_owned(),
                fan_in: 2,
                fan_out: 0,
            },
            DependencyFile {
                path: "src/dep1.ts".to_owned(),
                fan_in: 0,
                fan_out: 1,
            },
            DependencyFile {
                path: "src/dep2.ts".to_owned(),
                fan_in: 0,
                fan_out: 1,
            },
        ],
        edges: vec![
            DependencyEdge {
                source: "src/dep1.ts".to_owned(),
                target: "src/high.ts".to_owned(),
                kind: "import",
                confidence: "high",
            },
            DependencyEdge {
                source: "src/dep2.ts".to_owned(),
                target: "src/high.ts".to_owned(),
                kind: "import",
                confidence: "high",
            },
        ],
        unresolved: vec![],
        cycles: vec![],
    };
    let history = history(vec![]);
    let ownership = empty_ownership();
    let policy = empty_policy(&graph);
    let report = build_full(
        &analysis,
        &history,
        &graph,
        &ownership,
        &policy,
        HistoryWindow::Days90,
    );
    let order: Vec<&str> = report.risks.iter().map(|row| row.path.as_str()).collect();
    assert_eq!(order, ["src/high.ts", "src/a.ts", "src/b.ts"]);
}

#[test]
fn build_returns_the_full_ranking_without_truncation() {
    let analysis = analysis(vec![
        file_with_metrics("src/a.ts", 30, 10, None),
        file_with_metrics("src/b.ts", 30, 10, None),
        file_with_metrics("src/c.ts", 30, 10, None),
    ]);
    let history = history(vec![]);
    let graph = DependencyReport {
        schema_version: DEPENDENCY_SCHEMA_VERSION,
        metric_profile: "default",
        analyzer_version: "test",
        files: vec![
            DependencyFile {
                path: "src/a.ts".to_owned(),
                fan_in: 0,
                fan_out: 0,
            },
            DependencyFile {
                path: "src/b.ts".to_owned(),
                fan_in: 0,
                fan_out: 0,
            },
            DependencyFile {
                path: "src/c.ts".to_owned(),
                fan_in: 0,
                fan_out: 0,
            },
        ],
        edges: vec![],
        unresolved: vec![],
        cycles: vec![],
    };
    let ownership = empty_ownership();
    let policy = empty_policy(&graph);
    let report = build_full(
        &analysis,
        &history,
        &graph,
        &ownership,
        &policy,
        HistoryWindow::Days90,
    );
    assert_eq!(report.risks.len(), 3);
}

#[test]
fn window_selects_matching_changes_field() {
    let analysis = analysis(vec![file_with_metrics("src/a.ts", 10, 5, None)]);
    let history = history(vec![history_file("src/a.ts", 2, 10, 40, 4)]);
    let graph = DependencyReport {
        schema_version: DEPENDENCY_SCHEMA_VERSION,
        metric_profile: "default",
        analyzer_version: "test",
        files: vec![DependencyFile {
            path: "src/a.ts".to_owned(),
            fan_in: 0,
            fan_out: 0,
        }],
        edges: vec![],
        unresolved: vec![],
        cycles: vec![],
    };
    let ownership = empty_ownership();
    let policy = empty_policy(&graph);
    let recent = build_full(
        &analysis,
        &history,
        &graph,
        &ownership,
        &policy,
        HistoryWindow::Days30,
    );
    assert_eq!(recent.window, "30d");
    assert_eq!(recent.risks[0].raw.changes, Some(2));
    assert_eq!(
        recent.risks[0].components.churn,
        Some(100.0 * (2.0_f64 / 20.0).min(1.0))
    );
    let quarter = build_full(
        &analysis,
        &history,
        &graph,
        &ownership,
        &policy,
        HistoryWindow::Days90,
    );
    assert_eq!(quarter.window, "90d");
    assert_eq!(quarter.risks[0].raw.changes, Some(10));
    let yearly = build_full(
        &analysis,
        &history,
        &graph,
        &ownership,
        &policy,
        HistoryWindow::Days365,
    );
    assert_eq!(yearly.window, "365d");
    assert_eq!(yearly.risks[0].raw.changes, Some(40));
    assert_eq!(yearly.risks[0].components.churn, Some(100.0));
}

#[test]
fn serialization_is_deterministic() {
    let analysis = analysis(vec![file_with_metrics("src/a.ts", 30, 10, Some(30.0))]);
    let history = history(vec![history_file("src/a.ts", 5, 20, 40, 2)]);
    let graph = graph_for_a();
    let ownership = ownership("src/a.ts", &[("a", 8), ("b", 2)]);
    let policy = empty_policy(&graph);
    let first = build_full(
        &analysis,
        &history,
        &graph,
        &ownership,
        &policy,
        HistoryWindow::Days90,
    );
    let second = build_full(
        &analysis,
        &history,
        &graph,
        &ownership,
        &policy,
        HistoryWindow::Days90,
    );
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
}

// --- CLI tests ---

static CLI_RISK_NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn cli_risk_git(root: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn cli_risk_temp_dir(prefix: &str) -> std::path::PathBuf {
    let id = CLI_RISK_NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("{prefix}-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn cli_risk_risky_source() -> String {
    let mut source = String::from("export function risky(x: number): number {\n  let y = 0;\n");
    for index in 0..30 {
        source.push_str(&format!("  if (x > {index}) {{ y += 1; }}\n"));
    }
    source.push_str("  return y;\n}\n");
    source
}

const CLI_RISK_CALM_SOURCE: &str = "export function calm(x: number): number {\n  return x;\n}\n";

fn risk_fixture() -> std::path::PathBuf {
    let root = cli_risk_temp_dir("leadline-risk");
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("risky.ts"), cli_risk_risky_source()).unwrap();
    std::fs::write(src.join("calm.ts"), CLI_RISK_CALM_SOURCE).unwrap();
    cli_risk_git(&root, &["init", "-q"]);
    cli_risk_git(&root, &["config", "user.email", "a@example.invalid"]);
    cli_risk_git(&root, &["config", "user.name", "Author A"]);
    cli_risk_git(&root, &["add", "-A"]);
    cli_risk_git(&root, &["commit", "-q", "-m", "one"]);
    for index in 2..=5 {
        let path = src.join("risky.ts");
        let mut contents = std::fs::read_to_string(&path).unwrap();
        contents.push_str(&format!("// churn {index}\n"));
        std::fs::write(&path, contents).unwrap();
        cli_risk_git(&root, &["add", "-A"]);
        cli_risk_git(&root, &["commit", "-q", "-m", &format!("churn {index}")]);
    }
    root
}

fn cli_risk_expected_score(components: &serde_json::Value) -> f64 {
    let weights = [
        ("complexity", 20.0),
        ("crap", 15.0),
        ("churn", 20.0),
        ("impact", 20.0),
        ("ownership", 10.0),
        ("policy", 15.0),
    ];
    let mut weighted = 0.0_f64;
    let mut total = 0.0_f64;
    for (name, weight) in weights {
        if let Some(value) = components[name].as_f64() {
            weighted += weight * value;
            total += weight;
        }
    }
    weighted / total
}

#[test]
fn cli_risk_terminal_ranks_riskiest_first() {
    let root = risk_fixture(); // risky.ts committed 5x with a cognitive-30 function; calm.ts once
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["risk"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("Top change risks"));
    let risky = stdout.find("src/risky.ts").unwrap();
    let calm = stdout.find("src/calm.ts").unwrap();
    assert!(risky < calm);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_risk_json_shape_and_score_identity() {
    let root = risk_fixture();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["risk", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["model"], "change-risk");
    assert_eq!(value["window"], "90d");
    assert_eq!(value["git_available"], true);
    assert!(value.get("truncated").is_none());
    let risks = value["risks"].as_array().unwrap();
    assert_eq!(risks.len(), 2);
    assert_eq!(risks[0]["path"], "src/risky.ts");
    for row in risks {
        let components = &row["components"];
        assert!(components.get("complexity").is_some());
        assert!(components.get("crap").is_some());
        assert!(components.get("churn").is_some());
        assert!(components.get("impact").is_some());
        assert!(components.get("ownership").is_some());
        // No architecture rules in the fixture: policy contributes zero.
        assert_eq!(components["policy"], serde_json::json!(0.0));
        let expected = cli_risk_expected_score(components);
        let score = row["score"].as_f64().unwrap();
        assert!(
            (score - expected).abs() < 1e-9,
            "score {score} != recomputed {expected}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_risk_agent_json_is_compact() {
    let root = risk_fixture();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["risk", "--format", "agent-json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["model"], "change-risk");
    assert_eq!(value["window"], "90d");
    assert_eq!(value["summary"]["files_analyzed"], 2);
    assert_eq!(value["summary"]["risks"], 2);
    let risks = value["risks"].as_array().unwrap();
    assert_eq!(risks.len(), 2);
    assert_eq!(risks[0]["path"], "src/risky.ts");
    assert!(risks[0].get("score").is_some());
    assert!(risks[0].get("components").is_some());
    assert!(risks[0].get("raw").is_none());
    assert!(value.get("analyzer_version").is_none());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_risk_since_window_accepted() {
    let root = risk_fixture();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["risk", "--since", "30d", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["window"], "30d");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_risk_rejects_unknown_window() {
    let root = risk_fixture();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["risk", "--since", "9d"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("30d"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_risk_json_and_agent_json_are_exclusive() {
    let root = risk_fixture();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["risk", "--json", "--format", "agent-json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_risk_json_is_deterministic() {
    let root = risk_fixture();
    let args = ["risk", "--json"];
    let first = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(args)
        .output()
        .unwrap();
    let second = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(args)
        .output()
        .unwrap();
    assert!(first.status.success());
    assert!(second.status.success());
    assert_eq!(first.stdout, second.stdout);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_risk_without_git_still_ranks_with_null_churn() {
    let root = cli_risk_temp_dir("leadline-risk-nogit");
    let src = root.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("risky.ts"), cli_risk_risky_source()).unwrap();
    std::fs::write(src.join("calm.ts"), CLI_RISK_CALM_SOURCE).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["risk", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["git_available"], false);
    let risks = value["risks"].as_array().unwrap();
    assert_eq!(risks.len(), 2);
    assert_eq!(risks[0]["path"], "src/risky.ts");
    assert!(risks[0]["components"]["churn"].is_null());
    assert!(risks[0]["components"]["ownership"].is_null());
    assert_eq!(risks[0]["components"]["policy"], serde_json::json!(0.0));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_risk_empty_directory_is_incomplete() {
    let root = cli_risk_temp_dir("leadline-risk-empty");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["risk"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_risk_limit_caps_presentation_not_json() {
    let root = risk_fixture();
    // --json is always the full ranking.
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["risk", "--limit", "1", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value.get("truncated").is_none());
    assert_eq!(value["risks"].as_array().unwrap().len(), 2);
    assert_eq!(value["risks"][0]["path"], "src/risky.ts");
    // Terminal and agent-json cap rows at --limit instead.
    let terminal = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["risk", "--limit", "1"])
        .output()
        .unwrap();
    assert!(terminal.status.success());
    let stdout = String::from_utf8(terminal.stdout).unwrap();
    assert!(stdout.contains("raise --limit"));
    let agent = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["risk", "--limit", "1", "--format", "agent-json"])
        .output()
        .unwrap();
    assert!(agent.status.success());
    let agent_value: serde_json::Value = serde_json::from_slice(&agent.stdout).unwrap();
    assert_eq!(agent_value["risks"].as_array().unwrap().len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_risk_rejects_unknown_flags() {
    let root = risk_fixture();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_leadline"))
        .current_dir(&root)
        .args(["risk", "--bogus"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn scope_files_names_the_impact_universe() {
    // Analysis covers one file of a three-file graph: one analyzed row,
    // while blast percents range over the three-file scope.
    let graph = graph_for_a();
    let report = build_full(
        &analysis(vec![file_with_metrics("src/a.ts", 30, 10, Some(30.0))]),
        &history(vec![history_file("src/a.ts", 20, 20, 20, 2)]),
        &graph,
        &ownership("src/a.ts", &[("a", 8), ("b", 2)]),
        &empty_policy(&graph),
        HistoryWindow::Days90,
    );
    assert_eq!(report.files_analyzed, 1);
    assert_eq!(report.scope_files, 3);
}

#[test]
fn head_commit_pins_the_window() {
    let graph = graph_for_a();
    let report = build_full(
        &analysis(vec![file_with_metrics("src/a.ts", 30, 10, Some(30.0))]),
        &history(vec![history_file("src/a.ts", 20, 20, 20, 2)]),
        &graph,
        &ownership("src/a.ts", &[("a", 8), ("b", 2)]),
        &empty_policy(&graph),
        HistoryWindow::Days90,
    );
    assert_eq!(report.head_commit.as_deref(), Some("abc123"));
}

#[test]
fn weight_table_lists_exactly_the_six_components() {
    let names: Vec<&str> = COMPONENT_WEIGHTS.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        names,
        [
            "complexity",
            "crap",
            "churn",
            "impact",
            "ownership",
            "policy",
        ]
    );
    let weights: Vec<f64> = COMPONENT_WEIGHTS
        .iter()
        .map(|(_, weight)| *weight)
        .collect();
    assert_eq!(weights, [20.0, 15.0, 20.0, 20.0, 10.0, 15.0]);
    assert_eq!(RISK_MODEL, "change-risk");
}
