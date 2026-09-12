use leadline::agent::{
    Budget, analyze_agent_json, changed_agent_json, changed_agent_json_budgeted,
};
use leadline::core::{
    AnalysisReport, FileAnalysis, FunctionAnalysis, FunctionKind, FunctionMetrics, Language,
    MetricContribution, MetricSpecs,
};
use leadline::diff::{ChangedReport, FunctionChange};
use serde_json::json;

fn metrics(
    cognitive: u32,
    cyclomatic: u32,
    crap: Option<f64>,
    coverage: Option<f64>,
) -> FunctionMetrics {
    FunctionMetrics {
        loc: 10,
        logical_loc: 5,
        function_length: 10,
        parameters: 1,
        max_nesting: 1,
        cyclomatic,
        cognitive,
        halstead_n1: 1,
        halstead_n2: 1,
        halstead_total_operators: 1,
        halstead_total_operands: 1,
        halstead_vocabulary: 2,
        halstead_length: 2,
        halstead_volume: 2.0,
        halstead_difficulty: 1.0,
        halstead_effort: 2.0,
        maintainability_index: 80.0,
        coverage,
        crap,
    }
}

fn function(name: &str, start_line: u32, cognitive: u32, cyclomatic: u32) -> FunctionAnalysis {
    function_full(
        name,
        start_line,
        metrics(cognitive, cyclomatic, Some(1.0), Some(0.5)),
    )
}

fn function_full(name: &str, start_line: u32, metrics: FunctionMetrics) -> FunctionAnalysis {
    FunctionAnalysis {
        name: name.to_owned(),
        id: format!("{name}:function:{start_line}:{}", start_line + 5),
        kind: FunctionKind::Function,
        start_line,
        end_line: start_line + 5,
        start_byte: u64::from(start_line),
        end_byte: u64::from(start_line + 5),
        metrics,
        contributions: Vec::new(),
        source_fingerprint: 0,
    }
}
fn contribution(
    rule: &str,
    line: u32,
    nesting: u32,
    cognitive: u32,
    cyclomatic: u32,
) -> MetricContribution {
    MetricContribution {
        rule: rule.to_owned(),
        line,
        nesting,
        cognitive,
        cyclomatic,
    }
}

fn file(path: &str, functions: Vec<FunctionAnalysis>) -> FileAnalysis {
    FileAnalysis {
        path: path.to_owned(),
        language: Language::TypeScript,
        functions,
        parse_errors: Vec::new(),
    }
}

fn change(
    path: &str,
    name: &str,
    before: Option<FunctionAnalysis>,
    after: Option<FunctionAnalysis>,
) -> FunctionChange {
    FunctionChange {
        path: path.to_owned(),
        name: name.to_owned(),
        before,
        after,
    }
}

#[test]
fn analyze_agent_json_has_compact_sorted_shape() {
    let report = AnalysisReport {
        schema_version: 1,
        metric_profile: "default-v1",
        analyzer_version: "0.1.0",
        metric_specs: MetricSpecs::default(),
        files: vec![
            file(
                "b.ts",
                vec![function("beta", 20, 3, 2), function("alpha", 5, 1, 1)],
            ),
            file("a.ts", vec![function("solo", 1, 0, 1)]),
        ],
    };

    assert_eq!(
        analyze_agent_json(&report),
        json!({
            "schema_version": 1,
            "metric_profile": "default-v1",
            "summary": {"files": 2, "functions": 3},
            "truncated": false,
            "files": [
                {"path": "a.ts", "functions": [
                    {"name": "solo", "line": 1, "cognitive": 0, "cyclomatic": 1, "crap": 1.0, "coverage": 0.5},
                ]},
                {"path": "b.ts", "functions": [
                    {"name": "alpha", "line": 5, "cognitive": 1, "cyclomatic": 1, "crap": 1.0, "coverage": 0.5},
                    {"name": "beta", "line": 20, "cognitive": 3, "cyclomatic": 2, "crap": 1.0, "coverage": 0.5},
                ]},
            ],
        })
    );
}

#[test]
fn changed_agent_json_classifies_and_handles_added_removed() {
    let report = ChangedReport {
        schema_version: 1,
        metric_profile: "default-v1",
        analyzer_version: "0.1.0",
        metric_specs: MetricSpecs::default(),
        base: "HEAD".to_owned(),
        functions: vec![
            change(
                "app.ts",
                "worse",
                Some(function_full("worse", 1, metrics(1, 2, Some(2.0), None))),
                Some(function_full("worse", 1, metrics(4, 3, Some(5.0), None))),
            ),
            change(
                "app.ts",
                "better",
                Some(function_full("better", 10, metrics(4, 3, Some(5.0), None))),
                Some(function_full("better", 10, metrics(1, 2, Some(2.0), None))),
            ),
            change(
                "app.ts",
                "same",
                Some(function_full("same", 20, metrics(1, 1, Some(1.0), None))),
                Some(function_full("same", 20, metrics(1, 1, Some(1.0), None))),
            ),
            change("new.ts", "added", None, Some(function("added", 3, 2, 2))),
            change(
                "old.ts",
                "removed",
                Some(function("removed", 7, 2, 2)),
                None,
            ),
        ],
        parse_errors: Vec::new(),
    };

    assert_eq!(
        changed_agent_json(&report),
        json!({
            "schema_version": 1,
            "base": "HEAD",
            "summary": {"changed_functions": 5, "regressions": 1, "improvements": 1},
            "truncated": false,
            "regressions": [
                {"path": "app.ts", "line": 1, "function": "worse",
                 "before": {"cognitive": 1, "cyclomatic": 2, "crap": 2.0},
                 "after": {"cognitive": 4, "cyclomatic": 3, "crap": 5.0},
                 "delta": {"cognitive": 3, "cyclomatic": 1, "crap": 3.0}},
            ],
            "improvements": [
                {"path": "app.ts", "line": 10, "function": "better",
                 "before": {"cognitive": 4, "cyclomatic": 3, "crap": 5.0},
                 "after": {"cognitive": 1, "cyclomatic": 2, "crap": 2.0},
                 "delta": {"cognitive": -3, "cyclomatic": -1, "crap": -3.0}},
            ],
        })
    );

    // Added/removed functions carry nulls on the missing side and classify as neither.
    let added_only = ChangedReport {
        schema_version: 1,
        metric_profile: "default-v1",
        analyzer_version: "0.1.0",
        metric_specs: MetricSpecs::default(),
        base: "HEAD".to_owned(),
        functions: vec![
            change("new.ts", "added", None, Some(function("added", 3, 2, 2))),
            change(
                "old.ts",
                "removed",
                Some(function("removed", 7, 2, 2)),
                None,
            ),
        ],
        parse_errors: Vec::new(),
    };
    let value = changed_agent_json(&added_only);
    assert_eq!(
        value["summary"],
        json!({"changed_functions": 2, "regressions": 0, "improvements": 0})
    );
    assert_eq!(value["regressions"], json!([]));
    assert_eq!(value["improvements"], json!([]));
}

#[test]
fn changed_agent_json_explains_only_multiset_added_regression_causes() {
    let mut before = function_full("worse", 1, metrics(1, 1, Some(1.0), None));
    before.contributions = vec![contribution("if", 2, 0, 1, 1)];
    let mut after = function_full("worse", 9, metrics(2, 2, Some(2.0), None));
    after.contributions = vec![
        contribution("if", 20, 0, 1, 1),
        contribution("loop", 30, 1, 2, 1),
        contribution("loop", 31, 1, 2, 1),
    ];
    let mut improved = function_full("better", 40, metrics(2, 2, Some(2.0), None));
    improved.contributions = vec![contribution("if", 40, 0, 1, 1)];
    let report = ChangedReport {
        schema_version: 1,
        metric_profile: "default-v1",
        analyzer_version: "0.1.0",
        metric_specs: MetricSpecs::default(),
        base: "HEAD".to_owned(),
        functions: vec![
            change("app.ts", "worse", Some(before), Some(after)),
            change("app.ts", "better", Some(improved.clone()), Some(improved)),
        ],
        parse_errors: Vec::new(),
    };

    let default = changed_agent_json(&report);
    assert!(default["regressions"][0].get("causes").is_none());
    assert!(default["improvements"][0].get("causes").is_none());

    let explained = changed_agent_json_budgeted(
        &report,
        &Budget {
            explain: true,
            ..Budget::default()
        },
    );
    assert_eq!(
        explained["regressions"][0]["causes"],
        json!([
            {"rule": "loop", "line": 30, "nesting": 1, "cognitive": 2, "cyclomatic": 1},
            {"rule": "loop", "line": 31, "nesting": 1, "cognitive": 2, "cyclomatic": 1},
        ])
    );
    assert!(explained["improvements"][0].get("causes").is_none());
}

#[test]
fn changed_agent_json_orders_regressions_by_crap_delta_then_path() {
    let report = ChangedReport {
        schema_version: 1,
        metric_profile: "default-v1",
        analyzer_version: "0.1.0",
        metric_specs: MetricSpecs::default(),
        base: "HEAD".to_owned(),
        // Deliberately unsorted input: output order must not follow it.
        functions: vec![
            change(
                "z.ts",
                "big",
                Some(function_full("big", 1, metrics(1, 1, Some(1.0), None))),
                Some(function_full("big", 1, metrics(9, 9, Some(11.0), None))),
            ),
            change(
                "a.ts",
                "tie_b",
                Some(function_full("tie_b", 1, metrics(1, 1, Some(1.0), None))),
                Some(function_full("tie_b", 2, metrics(2, 2, Some(3.0), None))),
            ),
            change(
                "a.ts",
                "tie_a",
                Some(function_full("tie_a", 1, metrics(1, 1, Some(1.0), None))),
                Some(function_full("tie_a", 1, metrics(2, 2, Some(3.0), None))),
            ),
        ],
        parse_errors: Vec::new(),
    };

    let value = changed_agent_json(&report);
    let order: Vec<String> = value["regressions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            format!(
                "{}:{}",
                entry["path"].as_str().unwrap(),
                entry["function"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(order, vec!["z.ts:big", "a.ts:tie_a", "a.ts:tie_b"]);
    // Regression line prefers the after position.
    assert_eq!(value["regressions"][1]["line"], json!(1));
    assert_eq!(value["regressions"][2]["line"], json!(2));
}

#[test]
fn changed_agent_json_treats_null_crap_as_no_change() {
    let report = ChangedReport {
        schema_version: 1,
        metric_profile: "default-v1",
        analyzer_version: "0.1.0",
        metric_specs: MetricSpecs::default(),
        base: "HEAD".to_owned(),
        functions: vec![change(
            "app.ts",
            "nocov",
            Some(function_full("nocov", 1, metrics(1, 1, None, None))),
            Some(function_full("nocov", 1, metrics(1, 1, None, None))),
        )],
        parse_errors: Vec::new(),
    };

    let value = changed_agent_json(&report);
    assert_eq!(
        value["summary"],
        json!({"changed_functions": 1, "regressions": 0, "improvements": 0})
    );
}

#[test]
fn agent_json_is_deterministic_across_repeated_serialization() {
    let analysis = AnalysisReport {
        schema_version: 1,
        metric_profile: "default-v1",
        analyzer_version: "0.1.0",
        metric_specs: MetricSpecs::default(),
        files: vec![file(
            "b.ts",
            vec![function("b", 2, 1, 1), function("a", 1, 2, 2)],
        )],
    };
    let first = serde_json::to_string(&analyze_agent_json(&analysis)).unwrap();
    let second = serde_json::to_string(&analyze_agent_json(&analysis)).unwrap();
    assert_eq!(first, second);

    let changed = ChangedReport {
        schema_version: 1,
        metric_profile: "default-v1",
        analyzer_version: "0.1.0",
        metric_specs: MetricSpecs::default(),
        base: "HEAD".to_owned(),
        functions: vec![
            change(
                "b.ts",
                "f",
                Some(function("f", 1, 1, 1)),
                Some(function("f", 1, 5, 5)),
            ),
            change(
                "a.ts",
                "g",
                Some(function("g", 1, 1, 1)),
                Some(function("g", 1, 5, 5)),
            ),
        ],
        parse_errors: Vec::new(),
    };
    let reversed = ChangedReport {
        functions: changed.functions.iter().rev().cloned().collect(),
        ..changed.clone()
    };
    assert_eq!(
        serde_json::to_string(&changed_agent_json(&changed)).unwrap(),
        serde_json::to_string(&changed_agent_json(&reversed)).unwrap()
    );
}
