use leadline::config::{ArchitectureRule, Severity};
use leadline::graph::{DependencyEdge, DependencyFile, DependencyReport};
use leadline::policy::{PolicyStatus, compare, evaluate};
use std::collections::BTreeMap;

fn graph(edges: &[(&str, &str, &'static str)]) -> DependencyReport {
    let mut files = std::collections::BTreeSet::new();
    let mut rows = Vec::new();
    for (source, target, confidence) in edges {
        files.insert((*source).to_owned());
        files.insert((*target).to_owned());
        rows.push(DependencyEdge {
            source: (*source).to_owned(),
            target: (*target).to_owned(),
            kind: "import",
            confidence,
        });
    }
    DependencyReport {
        schema_version: 1,
        metric_profile: "default",
        analyzer_version: "0.2.0",
        files: files
            .into_iter()
            .map(|path| DependencyFile {
                path,
                fan_in: 0,
                fan_out: 0,
            })
            .collect(),
        edges: rows,
        unresolved: vec![],
        cycles: vec![],
    }
}

fn rule(name: &str, source: &str, deny: &[&str], severity: Severity) -> ArchitectureRule {
    ArchitectureRule {
        name: name.to_owned(),
        source: source.to_owned(),
        deny: deny.iter().map(|value| (*value).to_owned()).collect(),
        severity,
    }
}

#[test]
fn rules_match_parent_globs_in_declaration_order() {
    let rules = [
        rule(
            "domain-no-ui",
            "src/domain/**",
            &["src/ui/**"],
            Severity::Error,
        ),
        rule(
            "domain-no-widgets",
            "src/domain/**",
            &["src/widgets/**"],
            Severity::Warning,
        ),
    ];
    let report = evaluate(
        &graph(&[
            ("src/domain/a.ts", "src/ui/b.ts", "high"),
            ("src/domain/a.ts", "src/widgets/c.ts", "high"),
            ("src/ui/d.ts", "src/domain/e.ts", "high"),
        ]),
        &rules,
    );
    let rows: Vec<(&str, &str, &str)> = report
        .violations
        .iter()
        .map(|violation| {
            (
                violation.rule.as_str(),
                violation.source.as_str(),
                violation.target.as_str(),
            )
        })
        .collect();
    assert_eq!(
        rows,
        [
            ("domain-no-ui", "src/domain/a.ts", "src/ui/b.ts"),
            ("domain-no-widgets", "src/domain/a.ts", "src/widgets/c.ts"),
        ]
    );
    assert_eq!(report.error, 1);
    assert_eq!(report.warning, 1);
}

#[test]
fn unresolved_and_low_confidence_edges_never_match() {
    let rules = [rule(
        "domain-no-ui",
        "src/domain/**",
        &["src/ui/**"],
        Severity::Error,
    )];
    let report = evaluate(
        &graph(&[
            ("src/domain/a.ts", "src/ui/b.ts", "medium"),
            ("src/domain/a.ts", "src/ui/b.ts", "low"),
        ]),
        &rules,
    );
    assert!(report.violations.is_empty());
}

#[test]
fn drift_classifies_new_existing_and_resolved_with_renames() {
    let rules = [rule(
        "domain-no-ui",
        "src/domain/**",
        &["src/ui/**"],
        Severity::Error,
    )];
    let before = graph(&[
        ("src/domain/old.ts", "src/ui/old.ts", "high"),
        ("src/domain/removed.ts", "src/ui/removed.ts", "high"),
    ]);
    let after = graph(&[
        ("src/domain/keep.ts", "src/ui/keep.ts", "high"),
        ("src/domain/new.ts", "src/ui/new.ts", "high"),
    ]);
    let mut renames = BTreeMap::new();
    renames.insert(
        "src/domain/old.ts".to_owned(),
        "src/domain/keep.ts".to_owned(),
    );
    renames.insert("src/ui/old.ts".to_owned(), "src/ui/keep.ts".to_owned());
    let report = compare(&before, &after, &rules, &renames);
    let statuses: Vec<(&str, &str, PolicyStatus)> = report
        .violations
        .iter()
        .map(|violation| {
            (
                violation.source.as_str(),
                violation.target.as_str(),
                violation.status.unwrap_or(PolicyStatus::Existing),
            )
        })
        .collect();
    assert!(statuses.contains(&(
        "src/domain/keep.ts",
        "src/ui/keep.ts",
        PolicyStatus::Existing
    )));
    assert!(statuses.contains(&("src/domain/new.ts", "src/ui/new.ts", PolicyStatus::New)));
    assert!(
        statuses.contains(&(
            "src/domain/removed.ts",
            "src/ui/removed.ts",
            PolicyStatus::Resolved
        )),
        "a dropped violation is resolved drift: {statuses:?}"
    );
    assert!(
        report
            .violations
            .iter()
            .all(|violation| !violation.source.contains("old")),
        "renamed endpoints never appear as raw base paths: {statuses:?}"
    );
}
