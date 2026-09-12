use crate::core::{AnalysisReport, FunctionAnalysis, FunctionMetrics};
use crate::diff::ChangedReport;
use serde_json::{Value, json};
use std::cmp::Ordering;

/// Sort key for budget-limited agent views.
///
/// Parses only exact lowercase names via [`SortKey::parse`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortKey {
    #[default]
    Crap,
    Cognitive,
    Cyclomatic,
}

impl SortKey {
    pub fn parse(text: &str) -> Option<SortKey> {
        match text {
            "crap" => Some(SortKey::Crap),
            "cognitive" => Some(SortKey::Cognitive),
            "cyclomatic" => Some(SortKey::Cyclomatic),
            _ => None,
        }
    }
}

/// Output budget for agent-oriented JSON views. Every field is opt-in;
/// a default budget keeps the full unfiltered output.
#[derive(Clone, Copy, Debug, Default)]
pub struct Budget {
    pub top: Option<usize>,
    pub sort_by: Option<SortKey>,
    pub min_crap: Option<f64>,
    pub min_delta: Option<f64>,
}

/// Compact agent-oriented view of a full analysis report.
///
/// Keeps only stable token-cheap fields; drops halstead/maintainability detail.
pub fn analyze_agent_json(report: &AnalysisReport) -> Value {
    analyze_agent_json_budgeted(report, &Budget::default())
}

/// Compact agent-oriented view of a full analysis report with output budget controls.
///
/// Filters entries below `min_crap`, sorts by `sort_by`, then truncates to `top`,
/// setting `truncated` when entries were dropped. A default budget keeps every entry.
pub fn analyze_agent_json_budgeted(report: &AnalysisReport, budget: &Budget) -> Value {
    let mut files: Vec<_> = report.files.iter().collect();
    files.sort_by(|left, right| left.path.cmp(&right.path));
    let functions: usize = report.files.iter().map(|file| file.functions.len()).sum();
    let mut flat: Vec<(&str, &FunctionAnalysis)> = Vec::new();
    for file in &files {
        let mut functions: Vec<&FunctionAnalysis> = file.functions.iter().collect();
        functions.sort_by(|left, right| {
            left.start_line
                .cmp(&right.start_line)
                .then(left.name.cmp(&right.name))
        });
        for function in functions {
            if budget
                .min_crap
                .is_some_and(|floor| !function.metrics.crap.is_some_and(|score| score >= floor))
            {
                continue;
            }
            flat.push((file.path.as_str(), function));
        }
    }
    if let Some(key) = budget.sort_by {
        flat.sort_by(|left, right| {
            sort_key_ord(key, &left.1.metrics, &right.1.metrics)
                .then(left.0.cmp(right.0))
                .then(left.1.name.cmp(&right.1.name))
        });
    }
    let truncated = budget.top.is_some_and(|top| flat.len() > top);
    if let Some(top) = budget.top {
        flat.truncate(top);
    }
    let files: Vec<Value> = files
        .into_iter()
        .map(|file| {
            let functions: Vec<Value> = flat
                .iter()
                .filter(|entry| entry.0 == file.path)
                .map(|(_, function)| {
                    json!({
                        "name": function.name,
                        "line": function.start_line,
                        "cognitive": function.metrics.cognitive,
                        "cyclomatic": function.metrics.cyclomatic,
                        "crap": function.metrics.crap,
                        "coverage": function.metrics.coverage,
                    })
                })
                .collect();
            json!({
                "path": file.path,
                "functions": functions,
            })
        })
        .collect();
    json!({
        "schema_version": report.schema_version,
        "metric_profile": report.metric_profile,
        "summary": {"files": report.files.len(), "functions": functions},
        "files": files,
        "truncated": truncated,
    })
}

/// Compact agent-oriented view of a changed (before/after) report.
pub fn changed_agent_json(report: &ChangedReport) -> Value {
    changed_agent_json_budgeted(report, &Budget::default())
}

/// Compact agent-oriented view of a changed report with output budget controls.
///
/// Filters entries below `min_crap` or `min_delta`, sorts by `sort_by`, then truncates
/// each list to `top`, setting `truncated` when entries were dropped.
/// A default budget keeps the legacy ordering and every entry.
pub fn changed_agent_json_budgeted(report: &ChangedReport, budget: &Budget) -> Value {
    let mut regressions: Vec<Entry> = Vec::new();
    let mut improvements: Vec<Entry> = Vec::new();
    for change in &report.functions {
        let entry = Entry::new(
            &change.path,
            &change.name,
            change.before.as_ref(),
            change.after.as_ref(),
        );
        if entry.regressed {
            regressions.push(entry);
        } else if entry.improved {
            improvements.push(entry);
        }
    }
    if let Some(floor) = budget.min_crap {
        regressions.retain(|entry| entry.crap.is_some_and(|score| score >= floor));
        improvements.retain(|entry| entry.crap.is_some_and(|score| score >= floor));
    }
    if let Some(floor) = budget.min_delta {
        regressions.retain(|entry| entry.max_delta >= floor);
        improvements.retain(|entry| entry.max_delta >= floor);
    }
    if let Some(key) = budget.sort_by {
        regressions.sort_by(|left, right| {
            sort_entry(key, left, right)
                .then(left.path.cmp(&right.path))
                .then(left.name.cmp(&right.name))
        });
        improvements.sort_by(|left, right| {
            sort_entry(key, left, right)
                .then(left.path.cmp(&right.path))
                .then(left.name.cmp(&right.name))
        });
    } else {
        regressions.sort_by(|left, right| {
            right
                .crap_delta
                .total_cmp(&left.crap_delta)
                .then(left.path.cmp(&right.path))
                .then(left.name.cmp(&right.name))
        });
        improvements
            .sort_by(|left, right| left.path.cmp(&right.path).then(left.name.cmp(&right.name)));
    }
    let mut truncated = false;
    if let Some(top) = budget.top {
        if regressions.len() > top || improvements.len() > top {
            truncated = true;
        }
        regressions.truncate(top);
        improvements.truncate(top);
    }
    json!({
        "schema_version": report.schema_version,
        "base": report.base,
        "summary": {
            "changed_functions": report.functions.len(),
            "regressions": regressions.len(),
            "improvements": improvements.len(),
        },
        "regressions": regressions.into_iter().map(|entry| entry.value).collect::<Vec<_>>(),
        "improvements": improvements.into_iter().map(|entry| entry.value).collect::<Vec<_>>(),
        "truncated": truncated,
    })
}

struct Entry {
    value: Value,
    regressed: bool,
    improved: bool,
    crap_delta: f64,
    crap: Option<f64>,
    cognitive: u32,
    cyclomatic: u32,
    max_delta: f64,
    path: String,
    name: String,
}

impl Entry {
    fn new(
        path: &str,
        name: &str,
        before: Option<&FunctionAnalysis>,
        after: Option<&FunctionAnalysis>,
    ) -> Self {
        let (regressed, improved, crap_delta) = classify(before, after);
        let value = json!({
            "path": path,
            "line": after.or(before).map(|function| function.start_line),
            "function": name,
            "before": before.map(|f| metric_set(&f.metrics)),
            "after": after.map(|f| metric_set(&f.metrics)),
            "delta": delta(before, after),
        });
        let current = after.or(before);
        Self {
            value,
            regressed,
            improved,
            crap_delta,
            crap: current.and_then(|function| function.metrics.crap),
            cognitive: current
                .map(|function| function.metrics.cognitive)
                .unwrap_or(0),
            cyclomatic: current
                .map(|function| function.metrics.cyclomatic)
                .unwrap_or(0),
            max_delta: max_abs_delta(before, after),
            path: path.to_owned(),
            name: name.to_owned(),
        }
    }
}

fn metric_set(metrics: &FunctionMetrics) -> Value {
    json!({
        "cognitive": metrics.cognitive,
        "cyclomatic": metrics.cyclomatic,
        "crap": metrics.crap,
    })
}

fn delta(before: Option<&FunctionAnalysis>, after: Option<&FunctionAnalysis>) -> Value {
    match (before, after) {
        (Some(before), Some(after)) => json!({
            "cognitive": after.metrics.cognitive as i64 - before.metrics.cognitive as i64,
            "cyclomatic": after.metrics.cyclomatic as i64 - before.metrics.cyclomatic as i64,
            "crap": match (before.metrics.crap, after.metrics.crap) {
                (Some(before), Some(after)) => json!(after - before),
                _ => Value::Null,
            },
        }),
        _ => json!({"cognitive": null, "cyclomatic": null, "crap": null}),
    }
}

/// (regressed, improved, crap_delta). Null crap counts as no-change on that axis.
fn classify(
    before: Option<&FunctionAnalysis>,
    after: Option<&FunctionAnalysis>,
) -> (bool, bool, f64) {
    let (Some(before), Some(after)) = (before, after) else {
        return (false, false, 0.0);
    };
    let up = after.metrics.cognitive > before.metrics.cognitive
        || after.metrics.cyclomatic > before.metrics.cyclomatic
        || crap_delta(before, after).is_some_and(|delta| delta > 0.0);
    let down = after.metrics.cognitive < before.metrics.cognitive
        || after.metrics.cyclomatic < before.metrics.cyclomatic
        || crap_delta(before, after).is_some_and(|delta| delta < 0.0);
    (up, down && !up, crap_delta(before, after).unwrap_or(0.0))
}

fn crap_delta(before: &FunctionAnalysis, after: &FunctionAnalysis) -> Option<f64> {
    match (before.metrics.crap, after.metrics.crap) {
        (Some(before), Some(after)) => Some(after - before),
        _ => None,
    }
}

/// Descending order by sort key; null crap sorts last.
fn sort_key_ord(key: SortKey, left: &FunctionMetrics, right: &FunctionMetrics) -> Ordering {
    match key {
        SortKey::Crap => cmp_crap(left.crap, right.crap),
        SortKey::Cognitive => right.cognitive.cmp(&left.cognitive),
        SortKey::Cyclomatic => right.cyclomatic.cmp(&left.cyclomatic),
    }
}

fn sort_entry(key: SortKey, left: &Entry, right: &Entry) -> Ordering {
    match key {
        SortKey::Crap => cmp_crap(left.crap, right.crap),
        SortKey::Cognitive => right.cognitive.cmp(&left.cognitive),
        SortKey::Cyclomatic => right.cyclomatic.cmp(&left.cyclomatic),
    }
}

fn cmp_crap(left: Option<f64>, right: Option<f64>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => right.total_cmp(&left),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

/// Max absolute before/after delta across cognitive, cyclomatic, crap, and loc.
/// Missing sides or null crap contribute nothing on their axis.
fn max_abs_delta(before: Option<&FunctionAnalysis>, after: Option<&FunctionAnalysis>) -> f64 {
    match (before, after) {
        (Some(before), Some(after)) => {
            let cognitive =
                (after.metrics.cognitive as f64 - before.metrics.cognitive as f64).abs();
            let cyclomatic =
                (after.metrics.cyclomatic as f64 - before.metrics.cyclomatic as f64).abs();
            let loc = (after.metrics.loc as f64 - before.metrics.loc as f64).abs();
            let crap = match (before.metrics.crap, after.metrics.crap) {
                (Some(before), Some(after)) => (after - before).abs(),
                _ => 0.0,
            };
            cognitive.max(cyclomatic).max(loc).max(crap)
        }
        _ => 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{FileAnalysis, FunctionKind, Language, MetricSpecs};
    use crate::diff::FunctionChange;

    fn metrics(cognitive: u32, cyclomatic: u32, crap: Option<f64>) -> FunctionMetrics {
        FunctionMetrics {
            loc: 10,
            logical_loc: 8,
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
            halstead_volume: 1.0,
            halstead_difficulty: 1.0,
            halstead_effort: 1.0,
            maintainability_index: 100.0,
            coverage: Some(1.0),
            crap,
        }
    }

    fn function(
        name: &str,
        line: u32,
        cognitive: u32,
        cyclomatic: u32,
        crap: Option<f64>,
    ) -> FunctionAnalysis {
        FunctionAnalysis {
            name: name.to_owned(),
            id: format!("{name}:{line}"),
            kind: FunctionKind::Function,
            start_line: line,
            end_line: line + 5,
            start_byte: 0,
            end_byte: 10,
            metrics: metrics(cognitive, cyclomatic, crap),
            source_fingerprint: 0,
            contributions: Vec::new(),
        }
    }

    fn analysis_report() -> AnalysisReport {
        AnalysisReport {
            schema_version: 1,
            metric_profile: "test",
            analyzer_version: "test",
            metric_specs: MetricSpecs::default(),
            files: vec![FileAnalysis {
                path: "src/lib.rs".to_owned(),
                language: Language::Java,
                functions: vec![
                    function("low", 1, 1, 1, Some(1.0)),
                    function("high", 10, 9, 5, Some(30.0)),
                    function("nocrap", 20, 2, 2, None),
                ],
                parse_errors: Vec::new(),
            }],
        }
    }

    fn change(
        path: &str,
        name: &str,
        before: FunctionMetrics,
        after: FunctionMetrics,
    ) -> FunctionChange {
        let mut before_fn = function(name, 1, 0, 0, None);
        before_fn.metrics = before;
        let mut after_fn = function(name, 1, 0, 0, None);
        after_fn.metrics = after;
        FunctionChange {
            path: path.to_owned(),
            name: name.to_owned(),
            before: Some(before_fn),
            after: Some(after_fn),
        }
    }

    fn changed_report() -> ChangedReport {
        ChangedReport {
            schema_version: 1,
            metric_profile: "test",
            analyzer_version: "test",
            metric_specs: MetricSpecs::default(),
            base: "HEAD".to_owned(),
            functions: vec![
                change(
                    "a.rs",
                    "big",
                    metrics(1, 1, Some(2.0)),
                    metrics(10, 1, Some(20.0)),
                ),
                change(
                    "a.rs",
                    "tiny",
                    metrics(5, 2, Some(4.0)),
                    metrics(6, 2, Some(4.5)),
                ),
            ],
            parse_errors: Vec::new(),
        }
    }

    fn function_names(out: &Value) -> Vec<&str> {
        out["files"][0]["functions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|function| function["name"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn sort_key_parses_exact_lowercase_names() {
        assert_eq!(SortKey::parse("crap"), Some(SortKey::Crap));
        assert_eq!(SortKey::parse("cognitive"), Some(SortKey::Cognitive));
        assert_eq!(SortKey::parse("cyclomatic"), Some(SortKey::Cyclomatic));
        assert_eq!(SortKey::parse("CRAP"), None);
        assert_eq!(SortKey::parse(""), None);
    }

    #[test]
    fn default_budget_matches_legacy_output() {
        let report = analysis_report();
        let budgeted = analyze_agent_json_budgeted(&report, &Budget::default());
        assert_eq!(analyze_agent_json(&report), budgeted);
        assert_eq!(budgeted["schema_version"], 1);
        assert_eq!(budgeted["summary"]["files"], 1);
        assert_eq!(budgeted["summary"]["functions"], 3);
        assert_eq!(budgeted["truncated"], false);
        assert_eq!(function_names(&budgeted), vec!["low", "high", "nocrap"]);

        let changed = changed_report();
        let budgeted = changed_agent_json_budgeted(&changed, &Budget::default());
        assert_eq!(changed_agent_json(&changed), budgeted);
        assert_eq!(budgeted["base"], "HEAD");
        assert_eq!(budgeted["summary"]["regressions"], 2);
        assert_eq!(budgeted["summary"]["improvements"], 0);
        assert_eq!(budgeted["truncated"], false);
    }

    #[test]
    fn top_truncates_with_flag() {
        let report = analysis_report();
        let out = analyze_agent_json_budgeted(
            &report,
            &Budget {
                top: Some(2),
                ..Default::default()
            },
        );
        assert_eq!(out["truncated"], true);
        assert_eq!(function_names(&out), vec!["low", "high"]);

        let out = analyze_agent_json_budgeted(
            &report,
            &Budget {
                top: Some(10),
                ..Default::default()
            },
        );
        assert_eq!(out["truncated"], false);
        assert_eq!(function_names(&out), vec!["low", "high", "nocrap"]);
    }

    #[test]
    fn sort_orders_by_crap_descending() {
        let report = analysis_report();
        let out = analyze_agent_json_budgeted(
            &report,
            &Budget {
                sort_by: Some(SortKey::Crap),
                ..Default::default()
            },
        );
        assert_eq!(out["truncated"], false);
        assert_eq!(function_names(&out), vec!["high", "low", "nocrap"]);
    }

    #[test]
    fn min_crap_drops_null_and_low_entries() {
        let report = analysis_report();
        let out = analyze_agent_json_budgeted(
            &report,
            &Budget {
                min_crap: Some(5.0),
                ..Default::default()
            },
        );
        assert_eq!(out["truncated"], false);
        assert_eq!(function_names(&out), vec!["high"]);
    }

    #[test]
    fn min_delta_drops_tiny_changes() {
        let report = changed_report();
        let out = changed_agent_json_budgeted(
            &report,
            &Budget {
                min_delta: Some(2.0),
                ..Default::default()
            },
        );
        assert_eq!(out["truncated"], false);
        let regressions = out["regressions"].as_array().unwrap();
        assert_eq!(regressions.len(), 1);
        assert_eq!(regressions[0]["function"].as_str().unwrap(), "big");
    }
}
