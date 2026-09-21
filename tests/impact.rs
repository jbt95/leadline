mod common;

use leadline::graph::{DependencyCycle, DependencyEdge, DependencyFile, DependencyReport};
use leadline::impact::{analyze_impact, impact_counts};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

fn temporary_directory() -> PathBuf {
    static NEXT_ID: AtomicU64 = AtomicU64::new(0);
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("leadline-impact-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn write_file(root: &std::path::Path, path: &str, source: &str) {
    let path = root.join(path);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, source).unwrap();
}

fn cli_chain_fixture() -> PathBuf {
    let root = temporary_directory();
    write_file(&root, "src/b.ts", "export const b = 1;\n");
    write_file(&root, "src/a.ts", "import './b';\nexport const a = 1;\n");
    write_file(&root, "src/c.ts", "import './a';\nexport const c = 1;\n");
    write_file(&root, "src/d.ts", "import './c';\nexport const d = 1;\n");
    root
}

fn fixture_chain() -> DependencyReport {
    DependencyReport {
        schema_version: 1,
        metric_profile: "default",
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
        metric_profile: "default",
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
        metric_profile: "default",
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

#[test]
fn cli_impact_terminal_reports_target_and_dependents() {
    let root = cli_chain_fixture();
    let output = common::leadline()
        .current_dir(&root)
        .args(["impact", "src/b.ts"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("src/b.ts"), "missing target in:\n{stdout}");
    assert!(
        stdout.contains("src/a.ts"),
        "missing dependent in:\n{stdout}"
    );
    assert!(
        stdout.contains("distance 1"),
        "missing distance in:\n{stdout}"
    );
    assert!(
        stdout.to_lowercase().contains("blast radius"),
        "missing blast radius in:\n{stdout}"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_impact_json_reports_dependents_in_order() {
    let root = cli_chain_fixture();
    let output = common::leadline()
        .current_dir(&root)
        .args(["impact", "src/b.ts", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["target"], "src/b.ts");
    assert_eq!(value["blast_radius"], 3);
    assert_eq!(value["direct_dependents"], 1);
    let dependents: Vec<(&str, u64)> = value["dependents"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            (
                row["path"].as_str().unwrap(),
                row["distance"].as_u64().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        dependents,
        [("src/a.ts", 1), ("src/c.ts", 2), ("src/d.ts", 3)]
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_impact_agent_json_is_compact() {
    let root = cli_chain_fixture();
    let output = common::leadline()
        .current_dir(&root)
        .args(["impact", "src/b.ts", "--format", "agent-json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["target"], "src/b.ts");
    assert_eq!(value["blast_radius"], 3);
    assert_eq!(value["truncated"], false);
    assert_eq!(value["dependents"].as_array().unwrap().len(), 3);
    assert!(
        value.get("analyzer_version").is_none(),
        "agent view must stay compact"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_impact_json_is_deterministic() {
    let root = cli_chain_fixture();
    let args = ["impact", "src/b.ts", "--json"];
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
    assert!(second.status.success());
    assert_eq!(first.stdout, second.stdout);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_impact_top_truncates_dependents_but_keeps_blast_radius() {
    let root = cli_chain_fixture();
    let output = common::leadline()
        .current_dir(&root)
        .args(["impact", "src/b.ts", "--top", "1", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["blast_radius"], 3);
    assert_eq!(value["truncated"], true);
    assert_eq!(value["dependents"].as_array().unwrap().len(), 1);
    assert_eq!(value["dependents"][0]["path"], "src/a.ts");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_impact_requires_an_existing_target() {
    let root = cli_chain_fixture();
    let output = common::leadline()
        .current_dir(&root)
        .args(["impact", "src/missing.ts"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing.ts"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_impact_rejects_targets_outside_the_scope() {
    let root = cli_chain_fixture();
    write_file(&root, "outside.ts", "export const outside = 1;\n");
    let output = common::leadline()
        .current_dir(&root)
        .args(["impact", "../outside.ts", "--path", "src"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("outside the scope"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_impact_rejects_unknown_format() {
    let root = cli_chain_fixture();
    let output = common::leadline()
        .current_dir(&root)
        .args(["impact", "src/b.ts", "--format", "xml"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unknown --format"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_impact_missing_graph_target_is_usage_error() {
    let root = cli_chain_fixture();
    write_file(&root, "src/notes.txt", "plain text, not in the graph\n");
    let output = common::leadline()
        .current_dir(&root)
        .args(["impact", "src/notes.txt"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("notes.txt"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn cli_impact_help_lists_the_command() {
    let output = common::leadline()
        .args(["impact", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("leadline impact"));
}

fn fixture_cyclic() -> DependencyReport {
    let file = |path: &str, fan_in: usize, fan_out: usize| DependencyFile {
        path: path.to_owned(),
        fan_in,
        fan_out,
    };
    let edge = |source: &str, target: &str| DependencyEdge {
        source: source.to_owned(),
        target: target.to_owned(),
        kind: "import",
        confidence: "high",
    };
    DependencyReport {
        schema_version: 1,
        metric_profile: "default",
        analyzer_version: "0.2.0",
        files: vec![
            file("a.ts", 2, 1),
            file("b.ts", 1, 1),
            file("c.ts", 1, 1),
            file("d.ts", 0, 1),
        ],
        edges: vec![
            edge("a.ts", "b.ts"),
            edge("b.ts", "c.ts"),
            edge("c.ts", "a.ts"),
            edge("d.ts", "c.ts"),
        ],
        unresolved: vec![],
        cycles: vec![],
    }
}

#[test]
fn impact_counts_matches_per_target_reports() {
    for graph in [fixture_chain(), fixture_cyclic()] {
        let counts = impact_counts(&graph);
        assert_eq!(counts.len(), graph.files.len());
        for file in &graph.files {
            let report = analyze_impact(&graph, &file.path, usize::MAX).unwrap();
            let count = &counts[file.path.as_str()];
            assert_eq!(count.blast_radius, report.blast_radius, "{}", file.path);
            assert_eq!(
                count.direct_dependents, report.direct_dependents,
                "{}",
                file.path
            );
            assert!(
                (count.blast_radius_percent - report.blast_radius_percent).abs() < 1e-9,
                "{}",
                file.path
            );
        }
    }
}
