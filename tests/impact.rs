use leadline::graph::{DependencyCycle, DependencyEdge, DependencyFile, DependencyReport};
use leadline::impact::analyze_impact;

fn fixture_chain() -> DependencyReport {
    DependencyReport {
        schema_version: 1,
        metric_profile: "default-v1",
        analyzer_version: "0.2.0",
        files: vec![
            DependencyFile {
                path: "a.ts".to_owned(),
                fan_in: 1,
                fan_out: 1,
            },
            DependencyFile {
                path: "b.ts".to_owned(),
                fan_in: 2,
                fan_out: 0,
            },
            DependencyFile {
                path: "c.ts".to_owned(),
                fan_in: 1,
                fan_out: 1,
            },
            DependencyFile {
                path: "d.ts".to_owned(),
                fan_in: 0,
                fan_out: 1,
            },
            DependencyFile {
                path: "x.ts".to_owned(),
                fan_in: 0,
                fan_out: 1,
            },
        ],
        edges: vec![
            DependencyEdge {
                source: "a.ts".to_owned(),
                target: "b.ts".to_owned(),
                kind: "import",
                confidence: "high",
            },
            DependencyEdge {
                source: "c.ts".to_owned(),
                target: "a.ts".to_owned(),
                kind: "import",
                confidence: "high",
            },
            DependencyEdge {
                source: "d.ts".to_owned(),
                target: "c.ts".to_owned(),
                kind: "import",
                confidence: "high",
            },
            DependencyEdge {
                source: "x.ts".to_owned(),
                target: "b.ts".to_owned(),
                kind: "import",
                confidence: "high",
            },
        ],
        unresolved: vec![],
        cycles: vec![],
    }
}

#[test]
fn transitive_dependents_report_shortest_distances_in_order() {
    let graph = fixture_chain();
    let impact = analyze_impact(&graph, "b.ts", 20).unwrap();
    assert_eq!(impact.direct_dependents, 2);
    assert_eq!(impact.blast_radius, 4);
    assert_eq!(
        impact
            .dependents
            .iter()
            .map(|row| (row.path.as_str(), row.distance))
            .collect::<Vec<_>>(),
        [("a.ts", 1), ("x.ts", 1), ("c.ts", 2), ("d.ts", 3)]
    );
    assert_eq!(impact.target, "b.ts");
    assert_eq!(impact.files_analyzed, 5);
    assert_eq!((impact.fan_in, impact.fan_out), (2, 0));
    assert_eq!(impact.blast_radius_percent, 100.0);
    assert!(!impact.truncated);
    assert!(impact.cycles.is_empty());
}

#[test]
fn cycle_members_do_not_re_add_target() {
    let graph = DependencyReport {
        schema_version: 1,
        metric_profile: "default-v1",
        analyzer_version: "0.2.0",
        files: vec![
            DependencyFile {
                path: "a.ts".to_owned(),
                fan_in: 2,
                fan_out: 1,
            },
            DependencyFile {
                path: "b.ts".to_owned(),
                fan_in: 1,
                fan_out: 1,
            },
            DependencyFile {
                path: "c.ts".to_owned(),
                fan_in: 0,
                fan_out: 1,
            },
        ],
        edges: vec![
            DependencyEdge {
                source: "a.ts".to_owned(),
                target: "b.ts".to_owned(),
                kind: "import",
                confidence: "high",
            },
            DependencyEdge {
                source: "b.ts".to_owned(),
                target: "a.ts".to_owned(),
                kind: "import",
                confidence: "high",
            },
            DependencyEdge {
                source: "c.ts".to_owned(),
                target: "a.ts".to_owned(),
                kind: "import",
                confidence: "high",
            },
        ],
        unresolved: vec![],
        cycles: vec![DependencyCycle {
            files: vec!["a.ts".to_owned(), "b.ts".to_owned()],
        }],
    };
    let impact = analyze_impact(&graph, "b.ts", 20).unwrap();
    assert_eq!(
        impact
            .dependents
            .iter()
            .map(|row| (row.path.as_str(), row.distance))
            .collect::<Vec<_>>(),
        [("a.ts", 1), ("c.ts", 2)]
    );
    assert_eq!(impact.blast_radius, 2);
    assert_eq!(
        impact.cycles,
        vec![vec!["a.ts".to_owned(), "b.ts".to_owned()]]
    );
    assert!(
        !impact.dependents.iter().any(|row| row.path == "b.ts"),
        "target must not re-appear through its cycle"
    );
}

#[test]
fn cycles_exclude_components_without_target() {
    let graph = DependencyReport {
        schema_version: 1,
        metric_profile: "default-v1",
        analyzer_version: "0.2.0",
        files: vec![
            DependencyFile {
                path: "a.ts".to_owned(),
                fan_in: 1,
                fan_out: 1,
            },
            DependencyFile {
                path: "b.ts".to_owned(),
                fan_in: 1,
                fan_out: 1,
            },
            DependencyFile {
                path: "lonely.ts".to_owned(),
                fan_in: 0,
                fan_out: 0,
            },
        ],
        edges: vec![
            DependencyEdge {
                source: "a.ts".to_owned(),
                target: "b.ts".to_owned(),
                kind: "import",
                confidence: "high",
            },
            DependencyEdge {
                source: "b.ts".to_owned(),
                target: "a.ts".to_owned(),
                kind: "import",
                confidence: "high",
            },
        ],
        unresolved: vec![],
        cycles: vec![DependencyCycle {
            files: vec!["a.ts".to_owned(), "b.ts".to_owned()],
        }],
    };
    let impact = analyze_impact(&graph, "lonely.ts", 20).unwrap();
    assert!(impact.dependents.is_empty());
    assert!(impact.cycles.is_empty());
}

#[test]
fn zero_impact_target_reports_empty_dependents() {
    let graph = fixture_chain();
    let impact = analyze_impact(&graph, "d.ts", 20).unwrap();
    assert_eq!(impact.direct_dependents, 0);
    assert_eq!(impact.blast_radius, 0);
    assert!(impact.dependents.is_empty());
    assert_eq!(impact.blast_radius_percent, 0.0);
    assert_eq!((impact.fan_in, impact.fan_out), (0, 1));
    assert!(!impact.truncated);
}

#[test]
fn missing_target_returns_none() {
    let graph = fixture_chain();
    assert!(analyze_impact(&graph, "missing.ts", 20).is_none());
}

#[test]
fn truncation_keeps_full_blast_radius() {
    let graph = fixture_chain();
    let impact = analyze_impact(&graph, "b.ts", 2).unwrap();
    assert_eq!(impact.blast_radius, 4);
    assert_eq!(impact.blast_radius_percent, 100.0);
    assert_eq!(impact.direct_dependents, 2);
    assert_eq!(
        impact
            .dependents
            .iter()
            .map(|row| row.path.as_str())
            .collect::<Vec<_>>(),
        ["a.ts", "x.ts"]
    );
    assert!(impact.truncated);
}

#[test]
fn impact_report_serialization_is_byte_stable() {
    let graph = fixture_chain();
    let first = analyze_impact(&graph, "b.ts", 20).unwrap();
    let second = analyze_impact(&graph, "b.ts", 20).unwrap();
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
}
