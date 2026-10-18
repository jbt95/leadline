use leadline::analytics::{ProjectRequest, build};
use leadline::coverage::CoverageMap;
use leadline::history::HistoryWindow;
use leadline::ownership::OwnershipMode;
use leadline::source_snapshot::SnapshotTarget;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn fixture(relative: &str) -> PathBuf {
    PathBuf::from("tests/fixtures/complex-project").join(relative)
}

fn temporary_repo() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "leadline-complex-fixture-{}-{id}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn git(root: &Path, args: &[&str], date: Option<&str>) -> String {
    let mut command = Command::new("git");
    command.current_dir(root).args(args);
    if let Some(date) = date {
        command
            .env("GIT_AUTHOR_DATE", date)
            .env("GIT_COMMITTER_DATE", date);
    }
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn copy_tree(source: &Path, target: &Path) {
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let destination = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            std::fs::create_dir_all(&destination).unwrap();
            copy_tree(&entry.path(), &destination);
        } else {
            std::fs::copy(entry.path(), destination).unwrap();
        }
    }
}

fn apply_history_snapshot(root: &Path, name: &str) {
    let _ = std::fs::remove_dir_all(root.join("src"));
    copy_tree(&fixture(&format!("history/{name}")), root);
}

fn commit(root: &Path, message: &str, date: &str) {
    git(root, &["add", "-A"], None);
    git(root, &["commit", "-q", "-m", message], Some(date));
}

fn complex_project() -> leadline::project::Project {
    let root = fixture("repo/current");
    let mut coverage =
        CoverageMap::from_lcov(&std::fs::read_to_string(fixture("coverage/lcov.info")).unwrap())
            .unwrap();
    coverage.merge(
        CoverageMap::from_jacoco_xml(
            &std::fs::read_to_string(fixture("coverage/jacoco.xml")).unwrap(),
        )
        .unwrap(),
    );
    let request = ProjectRequest {
        path: root,
        target: SnapshotTarget::Worktree,
        window: HistoryWindow::Days90,
        mutation_inputs: Vec::new(),
        test_maps: Vec::new(),
        ownership_mode: OwnershipMode::AggregateOnly,
        snapshots_path: None,
        coverage: Some(coverage),
    };

    build(&request).unwrap()
}

#[test]
fn complex_project_combines_multilanguage_graph_coverage_and_risk() {
    let project = complex_project();

    assert_eq!(project.summary.files, 7);
    assert!(project.summary.functions >= 8);
    assert_eq!(project.summary.dependency_cycles, 1);
    assert_eq!(project.dependencies.unresolved.len(), 1);
    assert!(project.coverage.as_ref().unwrap().covered_functions >= 7);
    let service = project
        .risk
        .rows
        .iter()
        .find(|row| row.path == "src/service.ts")
        .unwrap();
    assert!(service.fan_in >= 4);
    assert!(service.blast_radius >= 4);
    assert_eq!(
        serde_json::to_vec(&project).unwrap(),
        serde_json::to_vec(&complex_project()).unwrap(),
    );
}

#[test]
fn complex_scanner_reports_deduplicate_enrich_and_gate() {
    let current_security = [
        fixture("scanners/security/current-a.sarif"),
        fixture("scanners/security/current-b.sarif"),
    ];
    let baseline_security = [fixture("scanners/security/baseline.sarif")];
    let mut security =
        leadline::security::read_security_reports(leadline::security::SecurityInputs {
            current: &current_security,
            baseline: &baseline_security,
        })
        .unwrap();
    let changed =
        std::collections::BTreeSet::from(["src/service.ts".to_owned(), "src/store.ts".to_owned()]);
    leadline::security::enrich(&mut security, &complex_project(), Some(&changed));

    assert_eq!(security.findings.len(), 3);
    let injection = security
        .findings
        .iter()
        .find(|finding| finding.rule_id == "sql-injection")
        .unwrap();
    assert_eq!(injection.report_ids, ["current-a.sarif", "current-b.sarif"]);
    assert!(injection.function_id.is_some());
    assert_eq!(injection.changed, Some(true));
    let security_gate = leadline::security::SecurityGate {
        minimum: leadline::security::SecuritySeverity::High,
        new_only: true,
        changed_only: true,
    };
    assert_eq!(
        leadline::security::gate_violations(&security, &security_gate).len(),
        2
    );
    assert!(
        !serde_json::to_string(&security)
            .unwrap()
            .contains("SECRET_SENTINEL")
    );

    let current_osv = [fixture("scanners/vulnerabilities/current-osv.json")];
    let current_trivy = [fixture("scanners/vulnerabilities/current-trivy.json")];
    let baseline_osv = [fixture("scanners/vulnerabilities/baseline-osv.json")];
    let mut vulnerabilities = leadline::vulnerabilities::read_vulnerability_reports(
        leadline::vulnerabilities::VulnerabilityInputs {
            osv: &current_osv,
            trivy: &current_trivy,
            baseline_osv: &baseline_osv,
            baseline_trivy: &[],
        },
    )
    .unwrap();
    let imports = leadline::graph::external_packages_from_sources(&[
        leadline::source_snapshot::SourceEntry {
            path: "src/changed.ts".to_owned(),
            bytes: std::fs::read(fixture("scanners/vulnerabilities/changed.ts")).unwrap(),
        },
    ])
    .unwrap();
    leadline::vulnerabilities::add_changed_import_evidence(&mut vulnerabilities, &imports, true);

    assert_eq!(vulnerabilities.findings.len(), 3);
    let minimist = vulnerabilities
        .findings
        .iter()
        .find(|finding| finding.package == "minimist")
        .unwrap();
    assert_eq!(
        minimist.report_ids,
        ["current-osv.json", "current-trivy.json"]
    );
    assert_eq!(minimist.reachable_from_changed, Some(true));
    assert_eq!(
        leadline::vulnerabilities::vulnerability_gate_violations(
            &vulnerabilities,
            leadline::security::SecuritySeverity::High,
        )
        .len(),
        1
    );
}

#[test]
fn complex_sql_fixture_covers_static_host_and_schema_rules() {
    let root = fixture("repo/current");
    let config = leadline::config::SqlConfig {
        large_offset: 1000,
        migration_roots: vec!["migrations".to_owned()],
    };

    let report = leadline::sql::analyze_sql_path(&root, &config, &[]).unwrap();
    let rules: std::collections::BTreeSet<&str> = report
        .findings
        .iter()
        .map(|finding| finding.rule_id.as_str())
        .collect();

    assert!(report.schema_evidence_available);
    assert_eq!(
        rules,
        std::collections::BTreeSet::from([
            "sql/dynamic-concatenation",
            "sql/large-offset",
            "sql/leading-wildcard",
            "sql/nonsargable-predicate",
            "sql/query-in-loop",
            "sql/unknown-table",
            "sql/update-delete-without-where",
        ])
    );
    assert_eq!(report.findings.len(), 7);
    assert!(
        report
            .findings
            .iter()
            .filter(|finding| {
                matches!(
                    finding.rule_id.as_str(),
                    "sql/dynamic-concatenation" | "sql/query-in-loop"
                )
            })
            .all(|finding| finding.function_id.is_some())
    );
    assert_eq!(
        report,
        leadline::sql::analyze_sql_path(&root, &config, &[]).unwrap()
    );
}

#[test]
fn complex_plan_fixture_reports_every_regression_and_query_status() {
    let limits = leadline::pg_plan::PlanLimits {
        max_cost_increase_percent: Some(50.0),
        max_plan_rows_ratio: Some(2.0),
        max_estimate_error_ratio: Some(5.0),
    };

    let report = leadline::pg_plan::compare_plan_directories(
        &fixture("plans/current"),
        &fixture("plans/baseline"),
        &limits,
    )
    .unwrap();

    assert_eq!(
        report
            .queries
            .iter()
            .map(|query| (query.query_id.as_str(), query.status))
            .collect::<Vec<_>>(),
        [
            ("checkout", leadline::pg_plan::QueryPlanStatus::Changed),
            ("new-query", leadline::pg_plan::QueryPlanStatus::New),
            ("removed-query", leadline::pg_plan::QueryPlanStatus::Removed),
            ("stable", leadline::pg_plan::QueryPlanStatus::Unchanged),
        ]
    );
    assert_eq!(
        report
            .violations
            .iter()
            .map(|change| change.kind)
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from([
            leadline::pg_plan::PlanRegressionKind::IndexToSequentialScan,
            leadline::pg_plan::PlanRegressionKind::CostIncrease,
            leadline::pg_plan::PlanRegressionKind::RowGrowth,
            leadline::pg_plan::PlanRegressionKind::EstimateError,
            leadline::pg_plan::PlanRegressionKind::AddedSort,
            leadline::pg_plan::PlanRegressionKind::JoinStrategyChange,
        ])
    );
    assert_eq!(report.violations.len(), 7);
}

#[test]
fn complex_history_fixture_joins_changes_coupling_hotspots_and_debt() {
    let root = temporary_repo();
    git(&root, &["init", "-q"], None);
    git(&root, &["config", "user.name", "Author A"], None);
    git(&root, &["config", "user.email", "a@example.invalid"], None);

    apply_history_snapshot(&root, "01-base");
    commit(&root, "base", "2026-01-01T12:00:00Z");
    let base = git(&root, &["rev-parse", "HEAD"], None);

    git(&root, &["config", "user.name", "Author B"], None);
    git(&root, &["config", "user.email", "b@example.invalid"], None);
    apply_history_snapshot(&root, "02-feature");
    commit(&root, "feature", "2026-01-10T12:00:00Z");

    git(&root, &["config", "user.name", "Author A"], None);
    git(&root, &["config", "user.email", "a@example.invalid"], None);
    apply_history_snapshot(&root, "03-current");
    commit(&root, "current", "2026-02-01T12:00:00Z");

    let history = leadline::history::analyze_history(&root).unwrap();
    let service_history = history
        .files
        .iter()
        .find(|file| file.path == "src/service.ts")
        .unwrap();
    assert_eq!(service_history.commits, 3);
    assert_eq!(service_history.contributors, 2);

    let coupling = leadline::coupling::analyze_coupling(
        &root,
        "src/service.ts",
        &leadline::coupling::CouplingOptions::default(),
    )
    .unwrap();
    let store = coupling
        .related
        .iter()
        .find(|file| file.path == "src/store.ts")
        .unwrap();
    assert_eq!(store.co_changes, 3);

    let project = build(&ProjectRequest {
        path: root.clone(),
        target: SnapshotTarget::Worktree,
        window: HistoryWindow::Days90,
        mutation_inputs: Vec::new(),
        test_maps: Vec::new(),
        ownership_mode: OwnershipMode::AggregateOnly,
        snapshots_path: None,
        coverage: None,
    })
    .unwrap();
    assert!(project.meta.git_available);
    let project_coupling = project.temporal_coupling.as_ref().unwrap();
    assert!(project_coupling.available);
    assert!(project_coupling.edges.iter().any(|edge| {
        edge.source == "src/service.ts" && edge.target == "src/store.ts" && edge.co_changes == 3
    }));

    let analysis = leadline::analyze_path(&root, None).unwrap();
    let hotspots = leadline::hotspots::build(
        &analysis,
        &history,
        HistoryWindow::Days90,
        analysis.files.len(),
    );
    assert_eq!(hotspots.hotspots[0].path, "src/service.ts");
    assert_eq!(hotspots.hotspots[0].changes, Some(3));

    let changed = leadline::diff::analyze_changes(
        &root,
        &leadline::diff::ChangeOptions {
            base: base.clone(),
            target: leadline::diff::ComparisonTarget::Revision("HEAD".to_owned()),
            detect_renames: true,
        },
    )
    .unwrap();
    assert!(changed.functions.iter().any(|change| {
        change.path == "src/policy.ts" && change.before.is_some() && change.after.is_some()
    }));
    assert!(
        changed
            .functions
            .iter()
            .any(|change| { change.path == "src/service.ts" && change.name == "process" })
    );

    let debt = leadline::analytics::analyze_debt(&leadline::analytics::DebtRequest {
        path: root.clone(),
        base,
        target: SnapshotTarget::Worktree,
        detect_renames: true,
        window: HistoryWindow::Days90,
        fail_on_regression: true,
    })
    .unwrap();
    assert!(debt.summary.new > 0);
    assert!(debt.risk_changes.iter().any(|change| {
        change.path == "src/policy.ts" && change.status != leadline::debt::RiskChangeStatus::Added
    }));

    std::fs::remove_dir_all(root).unwrap();
}
