use leadline::config::{ArchitectureRule, Severity};
use leadline::core::{
    AnalysisReport, FileAnalysis, FunctionAnalysis, FunctionKind, FunctionMetrics, METRIC_PROFILE,
    MetricSpecs, OUTPUT_SCHEMA_VERSION,
};
use leadline::graph::{DependencyEdge, DependencyFile, DependencyReport};
use leadline::history::{FileHistory, HISTORY_SCHEMA_VERSION, HistoryReport, HistoryWindow};
use leadline::ownership::{OwnershipMode, build as build_ownership};
use leadline::policy::{PolicyReport, PolicyViolation, evaluate};
use leadline::risk_v2::{RISK_V2_MODEL, build};

fn metrics(crap: Option<f64>) -> FunctionMetrics {
    FunctionMetrics {
        loc: 10,
        logical_loc: 5,
        function_length: 10,
        parameters: 0,
        max_nesting: 1,
        cyclomatic: 10,
        cognitive: 30,
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

fn analysis(path: &str, crap: Option<f64>) -> AnalysisReport {
    AnalysisReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: "test",
        metric_specs: MetricSpecs::default(),
        files: vec![FileAnalysis {
            path: path.to_owned(),
            language: leadline::core::Language::TypeScript,
            functions: vec![FunctionAnalysis {
                name: "f".to_owned(),
                id: "f@1".to_owned(),
                kind: FunctionKind::Function,
                start_line: 1,
                end_line: 10,
                start_byte: 0,
                end_byte: 100,
                metrics: metrics(crap),
                contributions: vec![],
                source_fingerprint: 0,
            }],
            parse_errors: vec![],
        }],
    }
}

fn history(path: &str, changes: u64, contributors: u64) -> HistoryReport {
    HistoryReport {
        schema_version: HISTORY_SCHEMA_VERSION,
        analyzer_version: "test",
        available: true,
        reference: "head-commit-time",
        head_commit: Some("abc".to_owned()),
        head_timestamp: Some(0),
        files: vec![FileHistory {
            path: path.to_owned(),
            commits: changes,
            changes_30d: changes,
            changes_90d: changes,
            changes_365d: changes,
            lines_added: 1,
            lines_deleted: 1,
            days_since_last_change: 0,
            age_days: 0,
            contributors,
            recent_contributors: contributors,
        }],
    }
}

fn graph(path: &str, fan_in: usize, fan_out: usize) -> DependencyReport {
    DependencyReport {
        schema_version: 1,
        metric_profile: "default-v1",
        analyzer_version: "0.2.0",
        files: vec![
            DependencyFile {
                path: path.to_owned(),
                fan_in,
                fan_out,
            },
            DependencyFile {
                path: "src/other.ts".to_owned(),
                fan_in: 1,
                fan_out: 0,
            },
        ],
        edges: vec![DependencyEdge {
            source: "src/other.ts".to_owned(),
            target: path.to_owned(),
            kind: "import",
            confidence: "high",
        }],
        unresolved: vec![],
        cycles: vec![],
    }
}

fn ownership(path: &str, identities: &[(&str, u64)]) -> leadline::ownership::OwnershipReport {
    let mut analysis_paths = vec![path.to_owned()];
    let touches = leadline::history::FileTouches {
        path: path.to_owned(),
        identities: identities
            .iter()
            .map(|(identity, count)| ((*identity).to_owned(), *count))
            .collect(),
    };
    analysis_paths.sort();
    build_ownership(&[touches], &analysis_paths, OwnershipMode::AggregateOnly)
}

#[test]
fn v2_score_uses_concentration_and_highest_source_policy() {
    let path = "src/a.ts";
    let analysis = analysis(path, Some(30.0));
    let history = history(path, 20, 2);
    let graph = graph(path, 1, 0);
    let ownership = ownership(path, &[("a", 8), ("b", 2)]);
    let rules = [ArchitectureRule {
        name: "domain-no-ui".to_owned(),
        source: "src/**".to_owned(),
        deny: vec!["src/ui/**".to_owned()],
        severity: Severity::Warning,
    }];
    let policy = PolicyReport {
        schema_version: 1,
        analyzer_version: "test",
        rules: 1,
        info: 0,
        warning: 1,
        error: 0,
        violations: vec![PolicyViolation {
            rule: "domain-no-ui".to_owned(),
            severity: Severity::Warning,
            source: path.to_owned(),
            target: "src/ui/b.ts".to_owned(),
            status: None,
        }],
    };
    assert_eq!(policy.violations[0].severity, Severity::Warning);
    assert_eq!(rules.len(), 1);

    let report = build(
        &analysis,
        &history,
        &graph,
        &ownership,
        &policy,
        HistoryWindow::Days90,
    );
    assert_eq!(report.model, RISK_V2_MODEL);
    assert_eq!(report.schema_version, 1);
    let row = &report.risks[0];
    assert_eq!(row.path, path);
    assert_eq!(row.components.complexity, Some(100.0));
    assert_eq!(row.components.crap, Some(100.0));
    assert_eq!(row.components.churn, Some(100.0));
    assert_eq!(row.components.impact, Some(100.0));
    assert_eq!(row.components.ownership, Some(80.0));
    assert_eq!(row.components.policy, Some(60.0));
    assert_eq!(row.raw.policy_severity, Some("warning"));
    assert_eq!(row.raw.concentration_percent, Some(80.0));

    let expected =
        (20.0 * 100.0 + 15.0 * 100.0 + 20.0 * 100.0 + 20.0 * 100.0 + 10.0 * 80.0 + 15.0 * 60.0)
            / 100.0;
    assert!((row.score - expected).abs() < 1e-9, "score identity");
}

#[test]
fn v2_treats_missing_ownership_and_history_as_unknown() {
    let path = "src/a.ts";
    let graph = graph(path, 0, 0);
    let analysis = analysis(path, None);
    let history = HistoryReport {
        schema_version: HISTORY_SCHEMA_VERSION,
        analyzer_version: "test",
        available: false,
        reference: "head-commit-time",
        head_commit: None,
        head_timestamp: None,
        files: vec![],
    };
    let ownership = ownership(path, &[]);
    let policy = evaluate(&graph, &[]);
    assert!(policy.violations.is_empty());

    let report = build(
        &analysis,
        &history,
        &graph,
        &ownership,
        &policy,
        HistoryWindow::Days90,
    );
    let row = &report.risks[0];
    assert_eq!(row.components.crap, None);
    assert_eq!(row.components.churn, None);
    assert_eq!(row.components.ownership, None);
    assert_eq!(row.components.policy, Some(0.0));
    assert_eq!(report.git_available, false);
    assert_eq!(report.head_commit, None);
    // Known weights: complexity 20 + impact 20 + policy 15 = 55.
    let expected = (20.0 * 100.0 + 20.0 * 100.0 + 15.0 * 0.0) / 55.0;
    assert!((row.score - expected).abs() < 1e-9);
}

#[test]
fn v1_model_constants_are_unchanged() {
    assert_eq!(leadline::risk::RISK_MODEL, "change-risk-v1");
    assert_eq!(leadline::risk::RISK_SCHEMA_VERSION, 1);
    assert_eq!(
        leadline::risk::COMPONENT_WEIGHTS
            .iter()
            .map(|(name, weight)| (*name, *weight))
            .collect::<Vec<_>>(),
        [
            ("complexity", 25.0),
            ("crap", 20.0),
            ("churn", 20.0),
            ("impact", 20.0),
            ("ownership", 15.0),
            ("policy", 0.0),
        ]
    );
    assert_ne!(RISK_V2_MODEL, "change-risk-v1");
}
