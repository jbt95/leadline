use leadline::config::Thresholds;
use leadline::core::{FileAnalysis, FunctionAnalysis, FunctionKind, FunctionMetrics, Language};
use leadline::debt::{
    DebtStatus, RiskChangeStatus, RiskSideEntry, classify, compare_risks, risk_entries, summarize,
};
use std::collections::BTreeMap;

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

fn function(name: &str, cognitive: u32, cyclomatic: u32, crap: Option<f64>) -> FunctionAnalysis {
    FunctionAnalysis {
        name: name.to_owned(),
        id: format!("{name}@1"),
        kind: FunctionKind::Function,
        start_line: 1,
        end_line: 10,
        start_byte: 0,
        end_byte: 100,
        metrics: metrics(cognitive, cyclomatic, crap),
        contributions: vec![],
        source_fingerprint: 0,
    }
}

fn file(path: &str, functions: Vec<FunctionAnalysis>) -> FileAnalysis {
    FileAnalysis {
        path: path.to_owned(),
        language: Language::TypeScript,
        functions,
        parse_errors: vec![],
    }
}

fn thresholds() -> Thresholds {
    Thresholds {
        cognitive: Some(15),
        cyclomatic: Some(10),
        crap: Some(30.0),
        max_nesting: Some(4),
    }
}

fn dimension<'a>(
    findings: &'a [leadline::debt::DebtFinding],
    name: &str,
    dimension: &str,
) -> &'a leadline::debt::DebtFinding {
    findings
        .iter()
        .find(|finding| finding.name == name && finding.dimension == dimension)
        .unwrap()
}

#[test]
fn classifies_new_existing_resolved_and_unknown_per_dimension() {
    let before = file(
        "src/a.ts",
        vec![
            function("clean", 5, 5, Some(10.0)),
            function("existing", 20, 5, None),
            function("resolved", 20, 5, Some(10.0)),
            function("unknown", 20, 5, None),
        ],
    );
    let after = file(
        "src/a.ts",
        vec![
            function("clean", 5, 5, Some(10.0)),
            function("existing", 20, 5, None),
            function("resolved", 5, 5, Some(10.0)),
            function("unknown", 20, 5, None),
            function("brand-new", 20, 5, Some(10.0)),
        ],
    );
    let (findings, unknown) = classify(&[&before], &[&after], &thresholds());

    assert_eq!(
        dimension(&findings, "brand-new", "cognitive").status,
        DebtStatus::New
    );
    assert_eq!(
        dimension(&findings, "existing", "cognitive").status,
        DebtStatus::Existing
    );
    assert_eq!(
        dimension(&findings, "resolved", "cognitive").status,
        DebtStatus::Resolved
    );
    assert!(
        findings.iter().all(|finding| finding.name != "clean"),
        "clean-to-clean pairs never appear"
    );
    // unknown has cognitive = 20 (non-CRAP unaffected) and crap = None on
    // both sides: only the CRAP dimension is unknown for it.
    assert_eq!(
        unknown, 2,
        "one unknown per blocked dimension/function pair"
    );
    assert!(
        findings
            .iter()
            .all(|finding| finding.dimension != "crap" || finding.name != "unknown"),
        "unknown dimensions never produce findings"
    );
}

#[test]
fn entity_absence_is_not_unknown() {
    let before = file("src/a.ts", vec![]);
    let after = file("src/a.ts", vec![function("fresh", 20, 5, None)]);
    let (findings, unknown) = classify(&[&before], &[&after], &thresholds());
    assert_eq!(
        dimension(&findings, "fresh", "cognitive").status,
        DebtStatus::New
    );
    // cognitive/cyclomatic/max_nesting for the absent side are known clean;
    // only CRAP is unknown because the entity is absent before classification.
    assert_eq!(unknown, 1, "absent entity's missing CRAP stays unknown");
}

#[test]
fn mixed_dimensions_are_separate_findings() {
    let before = file("src/a.ts", vec![function("mixed", 20, 5, Some(10.0))]);
    let after = file("src/a.ts", vec![function("mixed", 20, 20, Some(10.0))]);
    let (findings, _) = classify(&[&before], &[&after], &thresholds());
    assert_eq!(findings.len(), 2, "existing cognitive + new cyclomatic");
    assert_eq!(
        dimension(&findings, "mixed", "cognitive").status,
        DebtStatus::Existing
    );
    assert_eq!(
        dimension(&findings, "mixed", "cyclomatic").status,
        DebtStatus::New
    );
}

#[test]
fn risk_changes_classify_and_apply_renames() {
    let entry = |path: &str, score: f64| RiskSideEntry {
        path: path.to_owned(),
        score: Some(score),
        components: [
            ("complexity", Some(score)),
            ("crap", Some(score)),
            ("churn", None),
            ("impact", Some(0.0)),
            ("ownership", None),
            ("policy", Some(0.0)),
        ]
        .into_iter()
        .collect(),
    };
    let before = vec![entry("src/old.ts", 40.0), entry("src/shrunk.ts", 60.0)];
    let after = vec![
        entry("src/new.ts", 50.0),
        entry("src/shrunk.ts", 30.0),
        entry("src/fresh.ts", 20.0),
    ];
    let mut renames = BTreeMap::new();
    renames.insert("src/old.ts".to_owned(), "src/new.ts".to_owned());
    let changes = compare_risks(&before, &after, &renames);
    let status = |path: &str| {
        changes
            .iter()
            .find(|change| change.path == path)
            .unwrap()
            .status
    };
    assert_eq!(status("src/new.ts"), RiskChangeStatus::Increased);
    assert_eq!(status("src/shrunk.ts"), RiskChangeStatus::Decreased);
    assert_eq!(status("src/fresh.ts"), RiskChangeStatus::Added);
    let increased = changes
        .iter()
        .find(|change| change.path == "src/new.ts")
        .unwrap();
    assert_eq!(increased.delta, Some(10.0));
    let complexity = increased
        .components
        .iter()
        .find(|delta| delta.component == "complexity")
        .unwrap();
    assert_eq!(complexity.delta, Some(10.0));
    let churn = increased
        .components
        .iter()
        .find(|delta| delta.component == "churn")
        .unwrap();
    assert_eq!(churn.delta, None, "unknown components have no delta");
    assert!(
        changes.iter().all(|change| change.path != "src/old.ts"),
        "renamed base paths never appear as removed rows"
    );
}

#[test]
fn removes_and_summarizes() {
    let entry = |path: &str, score: f64| RiskSideEntry {
        path: path.to_owned(),
        score: Some(score),
        components: BTreeMap::new(),
    };
    let changes = compare_risks(
        &[entry("src/gone.ts", 50.0)],
        &[entry("src/keep.ts", 10.0)],
        &BTreeMap::new(),
    );
    assert_eq!(
        changes
            .iter()
            .find(|change| change.path == "src/gone.ts")
            .unwrap()
            .status,
        RiskChangeStatus::Removed
    );
    let summary = summarize(&[], 3, &changes);
    assert_eq!(summary.risk_removed, 1);
    assert_eq!(summary.risk_added, 1);
    assert_eq!(summary.unknown, 3);
}

#[test]
fn projection_matches_score_and_components() {
    let graph = leadline::graph::DependencyReport {
        schema_version: 1,
        metric_profile: "default",
        analyzer_version: "test",
        files: vec![],
        edges: vec![],
        unresolved: vec![],
        cycles: vec![],
    };
    let ownership = leadline::ownership::OwnershipReport {
        files: vec![],
        modules: vec![],
    };
    let policy = leadline::policy::evaluate(&graph, &[]);
    let report = leadline::risk::build(
        &leadline::core::AnalysisReport {
            schema_version: 1,
            metric_profile: "default",
            analyzer_version: "test",
            metric_specs: Default::default(),
            files: vec![file("src/a.ts", vec![function("f", 30, 10, Some(30.0))])],
        },
        &leadline::history::HistoryReport {
            schema_version: 1,
            analyzer_version: "test",
            available: false,
            reference: "head-commit-time",
            head_commit: None,
            head_timestamp: None,
            files: vec![],
        },
        &graph,
        &ownership,
        &policy,
        leadline::history::HistoryWindow::Days90,
    );
    let entries = risk_entries(&report);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].score, Some(report.risks[0].score));
    assert_eq!(
        entries[0].components.get("complexity").copied().flatten(),
        Some(100.0)
    );
    assert_eq!(
        entries[0].components.get("policy").copied().flatten(),
        Some(0.0)
    );
}
