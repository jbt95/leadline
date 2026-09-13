use leadline::config::DuplicationConfig;
use leadline::core::{
    AnalysisReport, FileAnalysis, FunctionAnalysis, FunctionKind, FunctionMetrics, METRIC_PROFILE,
    MetricSpecs, OUTPUT_SCHEMA_VERSION,
};
use leadline::duplication::detect;
use leadline::graph::{DependencyEdge, DependencyFile, DependencyReport};
use leadline::history::{FileHistory, HISTORY_SCHEMA_VERSION, HistoryReport};
use leadline::project::{PROJECT_SCHEMA_VERSION, ProjectInputs, build};
use leadline::source_snapshot::SourceEntry;

fn function(name: &str, cognitive: u32, loc: u32) -> FunctionAnalysis {
    FunctionAnalysis {
        name: name.to_owned(),
        id: format!("{name}@1"),
        kind: FunctionKind::Function,
        start_line: 1,
        end_line: 10,
        start_byte: 0,
        end_byte: 100,
        metrics: FunctionMetrics {
            loc,
            logical_loc: loc,
            function_length: loc,
            parameters: 0,
            max_nesting: 1,
            cyclomatic: 3,
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
            coverage: None,
            crap: None,
        },
        contributions: vec![],
        source_fingerprint: 0,
    }
}

fn analysis() -> AnalysisReport {
    AnalysisReport {
        schema_version: OUTPUT_SCHEMA_VERSION,
        metric_profile: METRIC_PROFILE,
        analyzer_version: "test",
        metric_specs: MetricSpecs::default(),
        files: vec![
            FileAnalysis {
                path: "src/a.ts".to_owned(),
                language: leadline::core::Language::TypeScript,
                functions: vec![function("f", 10, 5), function("g", 30, 7)],
                parse_errors: vec![],
            },
            FileAnalysis {
                path: "src/deep/b.ts".to_owned(),
                language: leadline::core::Language::TypeScript,
                functions: vec![function("h", 5, 3)],
                parse_errors: vec![],
            },
        ],
    }
}

fn graph() -> DependencyReport {
    DependencyReport {
        schema_version: 1,
        metric_profile: "default-v1",
        analyzer_version: "test",
        files: vec![
            DependencyFile {
                path: "src/a.ts".to_owned(),
                fan_in: 0,
                fan_out: 1,
            },
            DependencyFile {
                path: "src/deep/b.ts".to_owned(),
                fan_in: 1,
                fan_out: 0,
            },
        ],
        edges: vec![DependencyEdge {
            source: "src/a.ts".to_owned(),
            target: "src/deep/b.ts".to_owned(),
            kind: "import",
            confidence: "high",
        }],
        unresolved: vec![],
        cycles: vec![],
    }
}

fn duplication() -> leadline::duplication::DuplicationReport {
    detect(
        &[
            SourceEntry {
                path: "src/a.ts".to_owned(),
                bytes: b"export const a = 1;\n".to_vec(),
            },
            SourceEntry {
                path: "src/deep/b.ts".to_owned(),
                bytes: b"export const b = 2;\n".to_vec(),
            },
        ],
        &DuplicationConfig {
            min_tokens: 2,
            min_lines: 1,
            excludes: vec![],
        },
    )
}

fn history() -> HistoryReport {
    HistoryReport {
        schema_version: HISTORY_SCHEMA_VERSION,
        analyzer_version: "test",
        available: true,
        reference: "head-commit-time",
        head_commit: Some("abc".to_owned()),
        head_timestamp: Some(100),
        files: vec![FileHistory {
            path: "src/a.ts".to_owned(),
            commits: 3,
            changes_30d: 1,
            changes_90d: 2,
            changes_365d: 3,
            lines_added: 5,
            lines_deleted: 1,
            days_since_last_change: 1,
            age_days: 30,
            contributors: 2,
            recent_contributors: 2,
        }],
    }
}

#[test]
fn project_joins_every_section_and_recomputes_aggregates() {
    let analysis = analysis();
    let graph = graph();
    let duplication = duplication();
    let history = history();
    let git = leadline::history::GitAnalyticsSnapshot {
        history: history.clone(),
        touches: vec![],
        coupling: leadline::coupling::ProjectCoupling {
            available: true,
            reason: None,
            edges: vec![],
        },
    };
    let project = build(ProjectInputs {
        analysis: &analysis,
        generated_from: "HEAD".to_owned(),
        git: Some(&git),
        graph: &graph,
        ownership: None,
        mutation: None,
        test_relationships: None,
        duplication: &duplication,
        policy: None,
        risk_v1: None,
        risk_v2: None,
        snapshots: None,
    });

    assert_eq!(project.meta.schema_version, PROJECT_SCHEMA_VERSION);
    assert_eq!(project.meta.head_commit.as_deref(), Some("abc"));
    assert_eq!(project.meta.head_timestamp, Some(100));
    assert_eq!(project.summary.files, 2);
    assert_eq!(project.summary.functions, 3);
    assert_eq!(project.summary.dependency_edges, 1);
    assert_eq!(
        project.summary.duplication_groups as usize,
        duplication.groups.len()
    );
    assert_eq!(project.files[0].max_cognitive, 30);
    assert_eq!(project.files[0].loc, 12);
    assert_eq!(project.functions.len(), 3);
    assert_eq!(project.git_activity.as_ref().unwrap().len(), 1);
    assert!(project.temporal_coupling.is_some());
    assert!(project.ownership.is_none());
    assert!(project.coverage.is_none());
    assert!(project.risk.rows.is_empty());

    let root = project
        .modules
        .iter()
        .find(|module| module.path == ".")
        .unwrap();
    assert_eq!(root.files, 2);
    assert_eq!(root.functions, 3);
    assert_eq!(root.loc, 15);
    assert_eq!(root.max_cognitive, 30);
    let deep = project
        .modules
        .iter()
        .find(|module| module.path == "src/deep")
        .unwrap();
    assert_eq!(deep.files, 1);
    assert_eq!(deep.functions, 1);
    assert_eq!(deep.loc, 3);

    let first = serde_json::to_string_pretty(&project).unwrap();
    let second = serde_json::to_string_pretty(&project).unwrap();
    assert_eq!(first, second, "serialization is deterministic");
}
