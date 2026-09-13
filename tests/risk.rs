use leadline::core::{
    AnalysisReport, FileAnalysis, FunctionAnalysis, FunctionKind, FunctionMetrics, METRIC_PROFILE,
    MetricSpecs, OUTPUT_SCHEMA_VERSION,
};
use leadline::graph::{
    DEPENDENCY_SCHEMA_VERSION, DependencyEdge, DependencyFile, DependencyReport,
};
use leadline::history::{FileHistory, HISTORY_SCHEMA_VERSION, HistoryReport, HistoryWindow};
use leadline::risk::build;

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

/// Three-file graph where `src/a.ts` has exactly one dependent out of two
/// other files, so its blast radius is 1/2 = 50%.
fn graph_for_a() -> DependencyReport {
    DependencyReport {
        schema_version: DEPENDENCY_SCHEMA_VERSION,
        metric_profile: "default-v1",
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

#[test]
fn score_identity_sums_known_weighted_components() {
    // Fixture: max_cognitive 30, max_cyclomatic 10, max_crap Some(30.0),
    // changes_90d Some(20), contributors Some(2), blast_radius_percent 50.0.
    // complexity = 100.0, crap = 100.0, churn = 100.0, impact = 50.0,
    // ownership = 50.0, policy = None.
    // score == (25*100 + 20*100 + 20*100 + 20*50 + 15*50) / 100 == 82.5
    let analysis = analysis(vec![file_with_metrics("src/a.ts", 30, 10, Some(30.0))]);
    let history = history(vec![history_file("src/a.ts", 5, 20, 40, 2)]);
    let graph = graph_for_a();
    let report = build(&analysis, &history, &graph, HistoryWindow::Days90, 10);
    assert_eq!(report.risks.len(), 1);
    let row = &report.risks[0];
    assert_eq!(row.components.complexity, Some(100.0));
    assert_eq!(row.components.crap, Some(100.0));
    assert_eq!(row.components.churn, Some(100.0));
    assert_eq!(row.components.impact, Some(50.0));
    assert_eq!(row.components.ownership, Some(50.0));
    assert!((report.risks[0].score - 82.5).abs() < 1e-9);
    assert_eq!(report.risks[0].components.policy, None);
}

#[test]
fn null_components_renormalize_instead_of_zeroing() {
    // Same fixture but max_crap None and no history row for the file:
    // known = complexity (25) + impact (20); score == (25*100 + 20*50) / 45 == 77.777...
    let analysis = analysis(vec![file_with_metrics("src/a.ts", 30, 10, None)]);
    let history = history(vec![]);
    let graph = graph_for_a();
    let report = build(&analysis, &history, &graph, HistoryWindow::Days90, 10);
    assert_eq!(report.risks.len(), 1);
    let row = &report.risks[0];
    assert_eq!(row.components.complexity, Some(100.0));
    assert_eq!(row.components.crap, None);
    assert_eq!(row.components.churn, None);
    assert_eq!(row.components.ownership, None);
    assert_eq!(row.components.impact, Some(50.0));
    assert!((report.risks[0].score - 77.77777777777777).abs() < 1e-9);
}

#[test]
fn caps_saturate_at_100() {
    let analysis = analysis(vec![file_with_metrics("src/a.ts", 300, 200, Some(300.0))]);
    let history = history(vec![history_file("src/a.ts", 200, 200, 200, 1)]);
    let graph = DependencyReport {
        schema_version: DEPENDENCY_SCHEMA_VERSION,
        metric_profile: "default-v1",
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
    let report = build(&analysis, &history, &graph, HistoryWindow::Days90, 10);
    let row = &report.risks[0];
    assert_eq!(row.components.complexity, Some(100.0));
    assert_eq!(row.components.crap, Some(100.0));
    assert_eq!(row.components.churn, Some(100.0));
    assert_eq!(row.components.impact, Some(100.0));
    assert_eq!(row.components.ownership, Some(100.0));
    assert!((row.score - 100.0).abs() < 1e-9);
    assert_eq!(row.raw.blast_radius_percent, 100.0);
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
        metric_profile: "default-v1",
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
    let report = build(&analysis, &history, &graph, HistoryWindow::Days90, 10);
    let order: Vec<&str> = report.risks.iter().map(|row| row.path.as_str()).collect();
    assert_eq!(order, ["src/high.ts", "src/a.ts", "src/b.ts"]);
}

#[test]
fn limit_truncates_with_flag() {
    let analysis = analysis(vec![
        file_with_metrics("src/a.ts", 30, 10, None),
        file_with_metrics("src/b.ts", 30, 10, None),
        file_with_metrics("src/c.ts", 30, 10, None),
    ]);
    let history = history(vec![]);
    let graph = DependencyReport {
        schema_version: DEPENDENCY_SCHEMA_VERSION,
        metric_profile: "default-v1",
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
    let truncated = build(&analysis, &history, &graph, HistoryWindow::Days90, 2);
    assert_eq!(truncated.risks.len(), 2);
    assert!(truncated.truncated);
    let full = build(&analysis, &history, &graph, HistoryWindow::Days90, 5);
    assert_eq!(full.risks.len(), 3);
    assert!(!full.truncated);
}

#[test]
fn window_selects_matching_changes_field() {
    let analysis = analysis(vec![file_with_metrics("src/a.ts", 10, 5, None)]);
    let history = history(vec![history_file("src/a.ts", 2, 10, 40, 4)]);
    let graph = DependencyReport {
        schema_version: DEPENDENCY_SCHEMA_VERSION,
        metric_profile: "default-v1",
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
    let recent = build(&analysis, &history, &graph, HistoryWindow::Days30, 10);
    assert_eq!(recent.window, "30d");
    assert_eq!(recent.risks[0].raw.changes, Some(2));
    assert_eq!(
        recent.risks[0].components.churn,
        Some(100.0 * (2.0_f64 / 20.0).min(1.0))
    );
    let quarter = build(&analysis, &history, &graph, HistoryWindow::Days90, 10);
    assert_eq!(quarter.window, "90d");
    assert_eq!(quarter.risks[0].raw.changes, Some(10));
    let yearly = build(&analysis, &history, &graph, HistoryWindow::Days365, 10);
    assert_eq!(yearly.window, "365d");
    assert_eq!(yearly.risks[0].raw.changes, Some(40));
    assert_eq!(yearly.risks[0].components.churn, Some(100.0));
}

#[test]
fn serialization_is_deterministic() {
    let analysis = analysis(vec![file_with_metrics("src/a.ts", 30, 10, Some(30.0))]);
    let history = history(vec![history_file("src/a.ts", 5, 20, 40, 2)]);
    let graph = graph_for_a();
    let first = build(&analysis, &history, &graph, HistoryWindow::Days90, 10);
    let second = build(&analysis, &history, &graph, HistoryWindow::Days90, 10);
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
}
