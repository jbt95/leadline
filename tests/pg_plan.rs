use leadline::pg_plan::{PlanLimits, PlanRegressionKind, QueryPlan, compare_plan_directories};
use leadline::pg_plan::{ScanKind, extract_plan_facts, load_plan_directory};
mod common;
use common::{temporary_directory, write_temp};

const BASIC: &str = r#"[{"Plan": {"Node Type": "Index Scan", "Relation Name": "users", "Index Name": "users_email_idx", "Total Cost": 8.17, "Plan Rows": 1}}]"#;

#[test]
fn loads_raw_explain_json_by_filename() {
    let root = temporary_directory();
    let dir = root.join("current");
    std::fs::create_dir_all(&dir).unwrap();
    write_temp(&dir, "users-by-email.json", BASIC.as_bytes());
    let plans = load_plan_directory(&dir).unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].query_id, "users-by-email");
    assert_eq!(plans[0].plan.node_type, "Index Scan");
    assert_eq!(plans[0].plan.total_cost, Some(8.17));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn qualifies_relations_with_their_schema() {
    let root = temporary_directory();
    let dir = root.join("current");
    std::fs::create_dir_all(&dir).unwrap();
    let plan = r#"[{"Plan": {"Node Type": "Seq Scan", "Schema Name": "audit", "Relation Name": "users", "Total Cost": 1.0, "Plan Rows": 1}}]"#;
    write_temp(&dir, "q.json", plan.as_bytes());
    let plans = load_plan_directory(&dir).unwrap();
    assert_eq!(plans[0].plan.relation_name.as_deref(), Some("audit.users"));
    let facts = extract_plan_facts(&plans[0]);
    assert!(facts.relation_scans.contains_key("audit.users"));
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn same_table_in_different_schemas_stays_distinct() {
    let root = temporary_directory();
    let current = root.join("current");
    let baseline = root.join("baseline");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::create_dir_all(&baseline).unwrap();
    // Only audit.users regresses; public.users keeps its sequential scan, so
    // grouping by bare relation name would hide the audit regression.
    let baseline_plan = r#"[{"Plan": {"Node Type": "Append", "Total Cost": 10.0, "Plan Rows": 10, "Plans": [
        {"Node Type": "Seq Scan", "Schema Name": "public", "Relation Name": "users", "Total Cost": 5.0, "Plan Rows": 5},
        {"Node Type": "Index Scan", "Schema Name": "audit", "Relation Name": "users", "Total Cost": 5.0, "Plan Rows": 5}]}}]"#;
    let current_plan = r#"[{"Plan": {"Node Type": "Append", "Total Cost": 10.0, "Plan Rows": 10, "Plans": [
        {"Node Type": "Seq Scan", "Schema Name": "public", "Relation Name": "users", "Total Cost": 5.0, "Plan Rows": 5},
        {"Node Type": "Seq Scan", "Schema Name": "audit", "Relation Name": "users", "Total Cost": 5.0, "Plan Rows": 5}]}}]"#;
    write_temp(&current, "q.json", current_plan.as_bytes());
    write_temp(&baseline, "q.json", baseline_plan.as_bytes());
    let report = compare_plan_directories(&current, &baseline, &PlanLimits::default()).unwrap();
    assert_eq!(report.violations.len(), 1, "{:?}", report.violations);
    assert_eq!(
        report.violations[0].kind,
        PlanRegressionKind::IndexToSequentialScan
    );
    assert_eq!(
        report.violations[0].relation.as_deref(),
        Some("audit.users")
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn sorts_query_ids() {
    let root = temporary_directory();
    let dir = root.join("current");
    std::fs::create_dir_all(&dir).unwrap();
    write_temp(&dir, "b.json", BASIC.as_bytes());
    write_temp(&dir, "a.json", BASIC.as_bytes());
    let plans = load_plan_directory(&dir).unwrap();
    assert_eq!(plans.len(), 2);
    assert_eq!(plans[0].query_id, "a");
    assert_eq!(plans[1].query_id, "b");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn accepts_explain_analyze_fields() {
    let root = temporary_directory();
    let dir = root.join("current");
    std::fs::create_dir_all(&dir).unwrap();
    let analyze = r#"[{"Plan": {"Node Type": "Seq Scan", "Relation Name": "users", "Total Cost": 10.0, "Plan Rows": 100, "Actual Rows": 95, "Actual Loops": 2, "Plans": [{"Node Type": "Sort", "Total Cost": 1.0}]}, "Planning Time": 0.1, "Execution Time": 1.2}]"#;
    write_temp(&dir, "q.json", analyze.as_bytes());
    let plans = load_plan_directory(&dir).unwrap();
    assert_eq!(plans.len(), 1);
    assert_eq!(plans[0].plan.actual_rows, Some(95));
    assert_eq!(plans[0].plan.actual_loops, Some(2));
    assert_eq!(plans[0].plan.children.len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_symlink_nested_non_json_and_non_utf8_entries() {
    let root = temporary_directory();
    let dir = root.join("current");
    std::fs::create_dir_all(&dir).unwrap();
    write_temp(&dir, "ok.json", BASIC.as_bytes());
    write_temp(&dir, "notes.txt", b"ignored");
    std::fs::create_dir_all(dir.join("nested")).unwrap();
    write_temp(&dir.join("nested"), "inner.json", BASIC.as_bytes());
    assert!(
        load_plan_directory(&dir).is_err(),
        "nested directories must be rejected"
    );
    std::fs::remove_dir_all(dir.join("nested")).unwrap();
    let plans = load_plan_directory(&dir).unwrap();
    assert_eq!(plans.len(), 1, "non-json files are ignored");
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::ffi::OsStringExt;
        let bad = dir.join(std::ffi::OsString::from_vec(vec![0xff, 0xfe]));
        std::fs::write(&bad, BASIC.as_bytes()).unwrap();
        assert!(
            load_plan_directory(&dir).is_err(),
            "non-utf8 names must be rejected"
        );
        std::fs::remove_file(&bad).unwrap();
    }
    #[cfg(unix)]
    {
        let target = dir.join("ok.json");
        std::os::unix::fs::symlink(&target, dir.join("link.json")).unwrap();
        assert!(
            load_plan_directory(&dir).is_err(),
            "symlinks must be rejected"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_missing_plan_and_bad_shapes() {
    let root = temporary_directory();
    let dir = root.join("current");
    std::fs::create_dir_all(&dir).unwrap();
    write_temp(&dir, "empty.json", b"[]");
    assert!(load_plan_directory(&dir).is_err());
    std::fs::remove_file(dir.join("empty.json")).unwrap();
    write_temp(
        &dir,
        "two.json",
        b"[{\"Plan\": {\"Node Type\": \"Seq Scan\"}}, {\"Plan\": {\"Node Type\": \"Seq Scan\"}}]",
    );
    assert!(load_plan_directory(&dir).is_err());
    std::fs::remove_file(dir.join("two.json")).unwrap();
    write_temp(&dir, "noplan.json", b"[{\"Planning Time\": 1.0}]");
    assert!(load_plan_directory(&dir).is_err());
    std::fs::remove_file(dir.join("noplan.json")).unwrap();
    write_temp(&dir, "nonode.json", b"[{\"Plan\": {\"Total Cost\": 1.0}}]");
    assert!(load_plan_directory(&dir).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_invalid_numeric_fields() {
    let root = temporary_directory();
    let dir = root.join("current");
    std::fs::create_dir_all(&dir).unwrap();
    for (name, body) in [
        (
            "negcost.json",
            r#"[{"Plan": {"Node Type": "Seq Scan", "Total Cost": -1.0}}]"#,
        ),
        (
            "negrows.json",
            r#"[{"Plan": {"Node Type": "Seq Scan", "Plan Rows": -5}}]"#,
        ),
        (
            "negactual.json",
            r#"[{"Plan": {"Node Type": "Seq Scan", "Actual Rows": -1}}]"#,
        ),
        (
            "negloops.json",
            r#"[{"Plan": {"Node Type": "Seq Scan", "Actual Loops": -2}}]"#,
        ),
    ] {
        write_temp(&dir, name, body.as_bytes());
        assert!(
            load_plan_directory(&dir).is_err(),
            "{name} must be rejected"
        );
        std::fs::remove_file(dir.join(name)).unwrap();
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn rejects_too_many_files_and_oversized_and_deep_inputs() {
    let root = temporary_directory();
    let dir = root.join("current");
    std::fs::create_dir_all(&dir).unwrap();
    for i in 0..201 {
        write_temp(&dir, &format!("q{i:03}.json"), BASIC.as_bytes());
    }
    assert!(
        load_plan_directory(&dir).is_err(),
        "201 files must exceed the 200 limit"
    );
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    let overdeep = format!("{}0{}", "[".repeat(129), "]".repeat(129));
    write_temp(&dir, "deep.json", overdeep.as_bytes());
    assert!(
        load_plan_directory(&dir).is_err(),
        "over-deep JSON must be rejected"
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn rejects_control_character_stems() {
    let root = temporary_directory();
    let dir = root.join("current");
    std::fs::create_dir_all(&dir).unwrap();
    write_temp(&dir, "bad\nname.json", BASIC.as_bytes());
    assert!(load_plan_directory(&dir).is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn extracts_sort_join_scans_and_estimate_error() {
    let query = QueryPlan {
        query_id: "q".to_owned(),
        plan: leadline::pg_plan::PgPlanNode {
            node_type: "Hash Join".to_owned(),
            relation_name: None,
            index_name: None,
            total_cost: Some(100.0),
            plan_rows: Some(1000),
            actual_rows: None,
            actual_loops: None,
            children: vec![
                leadline::pg_plan::PgPlanNode {
                    node_type: "Index Scan".to_owned(),
                    relation_name: Some("users".to_owned()),
                    index_name: Some("users_email_idx".to_owned()),
                    total_cost: Some(8.17),
                    plan_rows: Some(1),
                    actual_rows: Some(100),
                    actual_loops: Some(1),
                    children: vec![],
                },
                leadline::pg_plan::PgPlanNode {
                    node_type: "Sort".to_owned(),
                    relation_name: None,
                    index_name: None,
                    total_cost: Some(10.0),
                    plan_rows: Some(50),
                    actual_rows: Some(50),
                    actual_loops: Some(2),
                    children: vec![leadline::pg_plan::PgPlanNode {
                        node_type: "Seq Scan".to_owned(),
                        relation_name: Some("orders".to_owned()),
                        index_name: None,
                        total_cost: Some(5.0),
                        plan_rows: Some(50),
                        actual_rows: Some(50),
                        actual_loops: None,
                        children: vec![],
                    }],
                },
            ],
        },
    };
    let facts = extract_plan_facts(&query);
    assert_eq!(facts.total_cost, Some(100.0));
    assert_eq!(facts.plan_rows, Some(1000));
    assert_eq!(facts.sort_nodes, 1);
    assert_eq!(facts.join_counts["Hash Join"], 1);
    assert_eq!(
        facts.relation_scans["users"],
        std::collections::BTreeSet::from([ScanKind::Index])
    );
    assert_eq!(
        facts.relation_scans["orders"],
        std::collections::BTreeSet::from([ScanKind::Sequential])
    );
    assert_eq!(facts.max_estimate_error_ratio, Some(100.0));
    assert!(!facts.estimate_error_unbounded);
}

#[test]
fn estimate_error_compares_per_loop_rows() {
    // PostgreSQL reports `Actual Rows` averaged over `Actual Loops`, so a
    // nested-loop inner node with accurate per-loop rows must stay at 1.0
    // no matter how often it ran.
    let query = QueryPlan {
        query_id: "q".to_owned(),
        plan: leadline::pg_plan::PgPlanNode {
            node_type: "Index Scan".to_owned(),
            relation_name: Some("users".to_owned()),
            index_name: Some("users_pkey".to_owned()),
            total_cost: Some(1.0),
            plan_rows: Some(1),
            actual_rows: Some(1),
            actual_loops: Some(500),
            children: vec![],
        },
    };
    let facts = extract_plan_facts(&query);
    assert_eq!(facts.max_estimate_error_ratio, Some(1.0));
    assert!(!facts.estimate_error_unbounded);
}

#[test]
fn extracts_scan_variants_join_kinds_and_repeated_relations() {
    let scan = |node_type: &str, relation: &str| leadline::pg_plan::PgPlanNode {
        node_type: node_type.to_owned(),
        relation_name: Some(relation.to_owned()),
        index_name: None,
        total_cost: None,
        plan_rows: None,
        actual_rows: None,
        actual_loops: None,
        children: vec![],
    };
    let query = QueryPlan {
        query_id: "q".to_owned(),
        plan: leadline::pg_plan::PgPlanNode {
            node_type: "Merge Join".to_owned(),
            relation_name: None,
            index_name: None,
            total_cost: None,
            plan_rows: None,
            actual_rows: None,
            actual_loops: None,
            children: vec![
                leadline::pg_plan::PgPlanNode {
                    node_type: "Nested Loop".to_owned(),
                    relation_name: None,
                    index_name: None,
                    total_cost: None,
                    plan_rows: None,
                    actual_rows: None,
                    actual_loops: None,
                    children: vec![
                        scan("Index Only Scan", "users"),
                        scan("Bitmap Heap Scan", "orders"),
                    ],
                },
                leadline::pg_plan::PgPlanNode {
                    node_type: "Incremental Sort".to_owned(),
                    relation_name: None,
                    index_name: None,
                    total_cost: None,
                    plan_rows: None,
                    actual_rows: None,
                    actual_loops: None,
                    children: vec![scan("Index Scan", "users")],
                },
            ],
        },
    };
    let facts = extract_plan_facts(&query);
    assert_eq!(facts.sort_nodes, 1);
    assert_eq!(facts.join_counts["Merge Join"], 1);
    assert_eq!(facts.join_counts["Nested Loop"], 1);
    assert_eq!(
        facts.relation_scans["users"],
        std::collections::BTreeSet::from([ScanKind::Index, ScanKind::IndexOnly])
    );
    assert_eq!(
        facts.relation_scans["orders"],
        std::collections::BTreeSet::from([ScanKind::Bitmap])
    );
}

#[test]
fn extracts_unbounded_and_missing_estimate_error() {
    let node = |plan_rows: Option<u64>, actual_rows: Option<u64>| QueryPlan {
        query_id: "q".to_owned(),
        plan: leadline::pg_plan::PgPlanNode {
            node_type: "Seq Scan".to_owned(),
            relation_name: Some("t".to_owned()),
            index_name: None,
            total_cost: None,
            plan_rows,
            actual_rows,
            actual_loops: None,
            children: vec![],
        },
    };
    let facts = extract_plan_facts(&node(Some(0), Some(10)));
    assert_eq!(facts.max_estimate_error_ratio, None);
    assert!(facts.estimate_error_unbounded);
    let facts = extract_plan_facts(&node(None, Some(10)));
    assert_eq!(facts.max_estimate_error_ratio, None);
    assert!(!facts.estimate_error_unbounded);
    let facts = extract_plan_facts(&node(Some(10), None));
    assert_eq!(facts.max_estimate_error_ratio, None);
    assert!(!facts.estimate_error_unbounded);
}

#[test]
fn flags_index_to_sequential_scan() {
    let root = temporary_directory();
    let current = root.join("current");
    let baseline = root.join("baseline");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::create_dir_all(&baseline).unwrap();
    write_temp(&baseline, "q.json", r#"[{"Plan": {"Node Type": "Index Scan", "Relation Name": "users", "Total Cost": 10.0, "Plan Rows": 10}}]"#.as_bytes());
    write_temp(&current, "q.json", r#"[{"Plan": {"Node Type": "Seq Scan", "Relation Name": "users", "Total Cost": 10.0, "Plan Rows": 10}}]"#.as_bytes());
    let report = compare_plan_directories(&current, &baseline, &PlanLimits::default()).unwrap();
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.kind == PlanRegressionKind::IndexToSequentialScan)
    );
    assert!(
        !report
            .violations
            .iter()
            .any(|v| v.kind == PlanRegressionKind::CostIncrease)
    );
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn compares_cost_rows_sorts_joins_and_thresholds() {
    let root = temporary_directory();
    let current = root.join("current");
    let baseline = root.join("baseline");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::create_dir_all(&baseline).unwrap();
    // exact no-change
    write_temp(&baseline, "same.json", BASIC.as_bytes());
    write_temp(&current, "same.json", BASIC.as_bytes());
    // cost +50% over a 25% limit, plus a newly added sort that rides along
    write_temp(&baseline, "costly.json", r#"[{"Plan": {"Node Type": "Seq Scan", "Relation Name": "t", "Total Cost": 100.0, "Plan Rows": 10}}]"#.as_bytes());
    write_temp(&current, "costly.json", r#"[{"Plan": {"Node Type": "Sort", "Total Cost": 150.0, "Plan Rows": 10, "Plans": [{"Node Type": "Seq Scan", "Relation Name": "t", "Total Cost": 90.0, "Plan Rows": 10}]}}]"#.as_bytes());
    // cost decrease is a change but never a violation
    write_temp(
        &baseline,
        "cheaper.json",
        r#"[{"Plan": {"Node Type": "Seq Scan", "Total Cost": 100.0, "Plan Rows": 10}}]"#.as_bytes(),
    );
    write_temp(
        &current,
        "cheaper.json",
        r#"[{"Plan": {"Node Type": "Seq Scan", "Total Cost": 50.0, "Plan Rows": 10}}]"#.as_bytes(),
    );
    // row growth 3x over a 2x limit
    write_temp(
        &baseline,
        "rows.json",
        r#"[{"Plan": {"Node Type": "Seq Scan", "Total Cost": 10.0, "Plan Rows": 100}}]"#.as_bytes(),
    );
    write_temp(
        &current,
        "rows.json",
        r#"[{"Plan": {"Node Type": "Seq Scan", "Total Cost": 10.0, "Plan Rows": 300}}]"#.as_bytes(),
    );
    // join strategy change without any numeric failure is informational only
    write_temp(
        &baseline,
        "join.json",
        r#"[{"Plan": {"Node Type": "Hash Join", "Total Cost": 10.0, "Plan Rows": 10}}]"#.as_bytes(),
    );
    write_temp(
        &current,
        "join.json",
        r#"[{"Plan": {"Node Type": "Merge Join", "Total Cost": 10.0, "Plan Rows": 10}}]"#
            .as_bytes(),
    );
    // new and removed queries are informational
    write_temp(&current, "added.json", BASIC.as_bytes());
    write_temp(&baseline, "dropped.json", BASIC.as_bytes());
    // missing metrics produce no changes
    write_temp(
        &baseline,
        "nometrics.json",
        r#"[{"Plan": {"Node Type": "Seq Scan"}}]"#.as_bytes(),
    );
    write_temp(
        &current,
        "nometrics.json",
        r#"[{"Plan": {"Node Type": "Seq Scan"}}]"#.as_bytes(),
    );
    let limits = PlanLimits {
        max_cost_increase_percent: Some(25.0),
        max_plan_rows_ratio: Some(2.0),
        max_estimate_error_ratio: None,
    };
    let report = compare_plan_directories(&current, &baseline, &limits).unwrap();
    let kinds: Vec<PlanRegressionKind> = report.violations.iter().map(|v| v.kind).collect();
    assert!(kinds.contains(&PlanRegressionKind::CostIncrease));
    assert!(kinds.contains(&PlanRegressionKind::RowGrowth));
    // added sort rides the cost violation in the same query
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.query_id == "costly" && v.kind == PlanRegressionKind::AddedSort)
    );
    // join-only change does not violate on its own
    assert!(!report.violations.iter().any(|v| v.query_id == "join"));
    // cheaper is changed but clean
    let cheaper = report
        .queries
        .iter()
        .find(|q| q.query_id == "cheaper")
        .unwrap();
    assert!(!cheaper.changes.is_empty());
    assert!(!report.violations.iter().any(|v| v.query_id == "cheaper"));
    // same/nometrics/new/removed carry no violations
    for id in ["same", "nometrics", "added", "dropped"] {
        assert!(
            !report.violations.iter().any(|v| v.query_id == id),
            "{id} must be clean"
        );
    }
    // stable ordering: query id, then kind rank
    let keys: Vec<(&str, PlanRegressionKind)> = report
        .violations
        .iter()
        .map(|v| (v.query_id.as_str(), v.kind))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn compares_estimate_error_and_threshold_equality() {
    let root = temporary_directory();
    let current = root.join("current");
    let baseline = root.join("baseline");
    std::fs::create_dir_all(&current).unwrap();
    std::fs::create_dir_all(&baseline).unwrap();
    // estimate error 10x over an 8x absolute limit fails from current alone
    write_temp(
        &baseline,
        "est.json",
        r#"[{"Plan": {"Node Type": "Seq Scan", "Plan Rows": 10, "Actual Rows": 10}}]"#.as_bytes(),
    );
    write_temp(
        &current,
        "est.json",
        r#"[{"Plan": {"Node Type": "Seq Scan", "Plan Rows": 10, "Actual Rows": 100}}]"#.as_bytes(),
    );
    // exactly at the cost threshold passes
    write_temp(
        &baseline,
        "edge.json",
        r#"[{"Plan": {"Node Type": "Seq Scan", "Total Cost": 100.0, "Plan Rows": 10}}]"#.as_bytes(),
    );
    write_temp(
        &current,
        "edge.json",
        r#"[{"Plan": {"Node Type": "Seq Scan", "Total Cost": 125.0, "Plan Rows": 10}}]"#.as_bytes(),
    );
    let limits = PlanLimits {
        max_cost_increase_percent: Some(25.0),
        max_plan_rows_ratio: None,
        max_estimate_error_ratio: Some(8.0),
    };
    let report = compare_plan_directories(&current, &baseline, &limits).unwrap();
    assert!(
        report
            .violations
            .iter()
            .any(|v| v.query_id == "est" && v.kind == PlanRegressionKind::EstimateError)
    );
    assert!(!report.violations.iter().any(|v| v.query_id == "edge"));
    std::fs::remove_dir_all(root).unwrap();
}
